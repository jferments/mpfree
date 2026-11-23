//! A packed, blocked f32 GEMM on `mpfree_simd::gemm4x8`: C[m × n] = A · B.
//! B (a layer's weights) is packed once, when the model loads, and kept as
//! halves; A (the patches) is packed per tile, straight from the layer's
//! input (`net.rs`).

use mpfree_simd::{GEMM_MR as MR, GEMM_NR as NR};
use std::time::{Duration, Instant};

/// Steps of K a block: a block of A's panel (256 × 4 floats) and of B's
/// (256 × 8) stay in the L1 cache while the kernel reads them.
pub(crate) const KC: usize = 256;

/// B, K × N, in KC blocks of NR-column panels, zero padded to whole
/// panels, as halves.
pub(crate) struct PackedB {
    pub k: usize,
    pub panels: usize,
    halves: Vec<u16>,
}

/// The f32 panels of a [`PackedB`], unpacked.
#[derive(Clone, Copy)]
pub(crate) struct BView<'a> {
    pub k: usize,
    pub panels: usize,
    pub data: &'a [f32],
}

/// What a job's layers share: the unpacked weights of the layer that runs
/// (one layer at a time, so as big as the widest), and the time unpacking
/// and the multiply-adds counted.
pub(crate) struct Work {
    scratch: Vec<f32>,
    pub unpack: Duration,
    pub macs: u64,
}

impl Work {
    /// With room for `widest` floats from the start, so the buffer never
    /// grows past it.
    pub fn new(widest: usize) -> Work {
        Work {
            scratch: Vec::with_capacity(widest),
            unpack: Duration::ZERO,
            macs: 0,
        }
    }
}

impl PackedB {
    /// B from `get(row, column)`, each value rounded to a half.
    pub fn new(k: usize, n: usize, get: impl Fn(usize, usize) -> f32) -> PackedB {
        let panels = n.div_ceil(NR);
        let mut halves = Vec::with_capacity(k * panels * NR);
        let mut k0 = 0;
        while k0 < k {
            let kc = KC.min(k - k0);
            for p in 0..panels {
                for kk in 0..kc {
                    for j in 0..NR {
                        let col = p * NR + j;
                        let v = if col < n { get(k0 + kk, col) } else { 0.0 };
                        halves.push(crate::half::to_f16(v));
                    }
                }
            }
            k0 += kc;
        }
        PackedB { k, panels, halves }
    }

    /// Floats the panels take unpacked.
    pub fn len(&self) -> usize {
        self.halves.len()
    }

    /// Bytes they take at rest.
    pub fn bytes(&self) -> usize {
        self.halves.len() * 2
    }

    /// The panels as f32, unpacked into the job's scratch.
    pub fn view<'a>(&self, work: &'a mut Work) -> BView<'a> {
        let t = Instant::now();
        work.scratch.clear();
        work.scratch.resize(self.halves.len(), 0.0);
        mpfree_simd::f16_to_f32(&self.halves, &mut work.scratch);
        work.unpack += t.elapsed();
        BView {
            k: self.k,
            panels: self.panels,
            data: &work.scratch,
        }
    }

    #[cfg(test)]
    pub fn halves(&self) -> &[u16] {
        &self.halves
    }
}

/// Packs `m` rows of a row-major m × k matrix into MR-row panels per KC
/// block: the layout [`gemm`] reads A in (and `net::put` writes).
pub(crate) fn pack_a(a: &[f32], m: usize, k: usize, out: &mut Vec<f32>) {
    let rp = m.div_ceil(MR);
    out.clear();
    out.resize(rp * MR * k, 0.0);
    let mut at = 0;
    let mut k0 = 0;
    while k0 < k {
        let kc = KC.min(k - k0);
        for p in 0..rp {
            for kk in 0..kc {
                for r in 0..MR {
                    let row = p * MR + r;
                    out[at] = if row < m { a[row * k + k0 + kk] } else { 0.0 };
                    at += 1;
                }
            }
        }
        k0 += kc;
    }
}

/// Multiply-adds [`gemm`] does for `m` rows of `b` (padding included):
/// what a layer counts.
pub(crate) fn macs(m: usize, b: &PackedB) -> u64 {
    (m.div_ceil(MR) * MR * b.k * b.panels * NR) as u64
}

/// C (m × panels·NR, row-major) = packed A · packed B.
pub(crate) fn gemm(m: usize, ap: &[f32], b: BView, c: &mut [f32]) {
    let rp = m.div_ceil(MR);
    let ldc = b.panels * NR;
    let mut k0 = 0;
    let mut a_off = 0;
    let mut b_off = 0;
    while k0 < b.k {
        let kc = KC.min(b.k - k0);
        for p in 0..b.panels {
            let bp = &b.data[b_off + p * kc * NR..b_off + (p + 1) * kc * NR];
            for r in 0..rp {
                let a = &ap[a_off + r * kc * MR..a_off + (r + 1) * kc * MR];
                let mut acc = [0f32; MR * NR];
                if k0 > 0 {
                    for i in 0..MR {
                        let row = r * MR + i;
                        if row < m {
                            let at = row * ldc + p * NR;
                            acc[i * NR..(i + 1) * NR].copy_from_slice(&c[at..at + NR]);
                        }
                    }
                }
                mpfree_simd::gemm4x8(kc, a, bp, &mut acc);
                for i in 0..MR {
                    let row = r * MR + i;
                    if row < m {
                        let at = row * ldc + p * NR;
                        c[at..at + NR].copy_from_slice(&acc[i * NR..(i + 1) * NR]);
                    }
                }
            }
        }
        a_off += rp * kc * MR;
        b_off += b.panels * kc * NR;
        k0 += kc;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gemm_matches_a_plain_product_of_the_halves() {
        let mut seed = 1u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5
        };
        for &(m, k, n) in &[
            (1, 1, 1),
            (5, 7, 9),
            (13, 300, 17),
            (64, 600, 32),
            (3, 257, 8),
        ] {
            let a: Vec<f32> = (0..m * k).map(|_| rnd()).collect();
            let bm: Vec<f32> = (0..k * n).map(|_| rnd()).collect();
            let pb = PackedB::new(k, n, |r, c| bm[r * n + c]);
            let mut work = Work::new(pb.len());
            let mut ap = Vec::new();
            pack_a(&a, m, k, &mut ap);
            let mut c = vec![0f32; m * pb.panels * NR];
            assert_eq!(macs(m, &pb), (m.div_ceil(4) * 4 * k * pb.panels * 8) as u64);
            gemm(m, &ap, pb.view(&mut work), &mut c);
            for i in 0..m {
                for j in 0..n {
                    let want: f64 = (0..k)
                        .map(|t| {
                            let w = mpfree_simd::plain::half(crate::half::to_f16(bm[t * n + j]));
                            f64::from(a[i * k + t]) * f64::from(w)
                        })
                        .sum();
                    let got = f64::from(c[i * pb.panels * NR + j]);
                    assert!(
                        (want - got).abs() < 1e-4 * (1.0 + want.abs()),
                        "{m} {k} {n} [{i}][{j}]: {want} vs {got}"
                    );
                }
            }
        }
    }

    #[test]
    fn unpacking_never_grows_the_scratch_past_the_widest() {
        let big = PackedB::new(300, 20, |r, c| (r + c) as f32);
        let small = PackedB::new(7, 3, |r, c| (r * c) as f32);
        let mut work = Work::new(big.len());
        let cap = work.scratch.capacity();
        for b in [&small, &big, &small] {
            let v = b.view(&mut work);
            assert_eq!(v.data.len(), b.len());
        }
        assert_eq!(work.scratch.capacity(), cap);
    }
}
