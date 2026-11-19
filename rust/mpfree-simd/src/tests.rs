use super::*;
use crate::workload::{saw_levels, Supersaw};

/// A small deterministic random source.
struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0
    }
    /// -1 to 1.
    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
    #[cfg_attr(not(target_arch = "arm"), allow(dead_code))]
    fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }
}

/// A table of `len` entries with sample values in -1 to 1 and steps in the
/// size a real table's are (a difference of up to 2 over the phase bits
/// below the index).
fn table(rng: &mut Rng, len: usize) -> Vec<[f32; 2]> {
    let per = len as f32 / 4_294_967_296.0;
    (0..len)
        .map(|_| [rng.unit(), rng.unit() * 2.0 * per])
        .collect()
}

fn shift_of(len: usize) -> u32 {
    32 - len.trailing_zeros()
}

fn bits(x: &[f32]) -> Vec<u32> {
    x.iter().map(|v| v.to_bits()).collect()
}

/// The twin against a plain f64 sum, one frame at a time.
#[test]
fn the_twin_reads_each_table_linearly_and_sums_in_order() {
    let mut rng = Rng(7);
    let len = 512;
    let t = table(&mut rng, len);
    let shift = shift_of(len);
    let mut ph = [3u32 << 30, 17, u32::MAX - 5];
    let inc = [1u32 << 22, 12_345_678, 3_000_000_001];
    let gain = [0.5f32, -0.25, 0.125];
    let start = ph;
    let mut buf = vec![9.0f32; 37];
    plain::table_sum::<false>(&t, shift, &mut buf, &mut ph, &inc, &gain);
    for (n, &y) in buf.iter().enumerate() {
        let mut want = 0.0f64;
        for k in 0..3 {
            let p = start[k].wrapping_add(inc[k].wrapping_mul(n as u32));
            let [a, s] = t[(p >> shift) as usize];
            let frac = f64::from(p & ((1 << shift) - 1));
            want += (f64::from(a) + f64::from(s) * frac) * f64::from(gain[k]);
        }
        assert!((f64::from(y) - want).abs() < 1e-5, "{n}: {y} vs {want}");
    }
    for k in 0..3 {
        assert_eq!(ph[k], start[k].wrapping_add(inc[k].wrapping_mul(37)));
    }
}

#[test]
fn writing_starts_from_zero_and_adding_keeps_what_is_there() {
    let mut rng = Rng(11);
    let t = table(&mut rng, 1024);
    let shift = shift_of(1024);
    let (inc, gain) = ([1u32 << 24, 3 << 22], [0.5f32, 0.5]);
    let mut a = vec![5.0f32; 20];
    let mut b = vec![5.0f32; 20];
    let (mut pa, mut pb) = ([0u32, 1 << 31], [0u32, 1 << 31]);
    plain::table_sum::<false>(&t, shift, &mut a, &mut pa, &inc, &gain);
    plain::table_sum::<true>(&t, shift, &mut b, &mut pb, &inc, &gain);
    for (x, y) in a.iter().zip(&b) {
        assert!((y - x - 5.0).abs() < 1e-5, "{x} {y}");
    }
    // A silent first oscillator writes +0, never -0.
    let mut z = vec![1.0f32; 8];
    plain::table_sum::<false>(&t, shift, &mut z, &mut [0], &[0], &[-0.0]);
    assert!(z.iter().all(|x| x.to_bits() == 0), "{z:?}");
}

#[test]
fn any_block_size_gives_the_same_samples_on_either_path() {
    let mut rng = Rng(3);
    let t = table(&mut rng, 2048);
    let shift = shift_of(2048);
    let inc: Vec<u32> = (0..7).map(|_| rng.next() >> 6).collect();
    let gain: Vec<f32> = (0..7).map(|_| rng.unit()).collect();
    let start: Vec<u32> = (0..7).map(|_| rng.next()).collect();
    let whole = {
        let mut out = vec![0.0f32; 300];
        let mut ph = start.clone();
        plain::table_sum::<false>(&t, shift, &mut out, &mut ph, &inc, &gain);
        out
    };
    for path in [Path::Plain, Path::Neon] {
        let mut cut = vec![0.0f32; 300];
        let mut ph = start.clone();
        let mut at = 0;
        for n in [1, 7, 16, 64, 100, 3, 109] {
            table_sum_on::<false>(path, &t, shift, &mut cut[at..at + n], &mut ph, &inc, &gain);
            at += n;
        }
        assert_eq!(bits(&whole), bits(&cut), "{path:?}");
    }
}

#[test]
fn no_oscillator_or_no_frame_changes_nothing() {
    let t = vec![[0.5f32, 0.0]; 512];
    let mut buf = vec![2.0f32; 9];
    for path in [Path::Plain, Path::Neon] {
        table_sum_on::<false>(path, &t, 23, &mut buf, &mut [], &[], &[]);
        assert!(buf.iter().all(|&x| x == 2.0));
        let mut ph = [5u32];
        table_sum_on::<false>(path, &t, 23, &mut [], &mut ph, &[9], &[1.0]);
        assert_eq!(ph, [5]);
    }
}

#[test]
fn the_shortest_slice_sets_how_many_oscillators_play() {
    let mut rng = Rng(5);
    let t = table(&mut rng, 512);
    let mut a = vec![0.0f32; 12];
    let mut b = vec![0.0f32; 12];
    let mut pa = [1u32, 2, 3];
    let mut pb = [1u32, 2];
    for path in [Path::Plain, Path::Neon] {
        table_sum_on::<false>(path, &t, 23, &mut a, &mut pa, &[7, 8, 9], &[0.1, 0.2]);
        table_sum_on::<false>(path, &t, 23, &mut b, &mut pb, &[7, 8], &[0.1, 0.2]);
        assert_eq!(bits(&a), bits(&b));
        assert_eq!(pa[2], 3, "the third phase stays");
    }
}

#[test]
fn the_status_says_which_code_runs() {
    let s = status();
    if neon_available() {
        assert_eq!(s, "NEON");
        assert_eq!(path(), Path::Neon);
    } else {
        assert!(s.starts_with("plain Rust"), "{s}");
        assert_eq!(path(), Path::Plain);
        // Asking for NEON where it can't run runs the twin, and says so.
        let t = vec![[0.0f32, 0.0]; 512];
        let took = table_sum_on::<false>(Path::Neon, &t, 23, &mut [0.0; 8], &mut [0], &[1], &[1.0]);
        assert_eq!(took, Path::Plain);
        assert_eq!(fallbacks(), 0);
    }
}

#[test]
fn the_workload_plays_a_saw_chord_at_a_sensible_level() {
    let levels = saw_levels();
    assert_eq!(levels.len(), 17);
    for lv in &levels {
        assert_eq!(lv.table.len(), 1 << (32 - lv.shift));
        // No denormal anywhere: the paths can't differ on flushing.
        assert!(lv
            .table
            .iter()
            .flatten()
            .all(|x| *x == 0.0 || x.is_normal()));
    }
    let mut s = Supersaw::new(8, 44_100);
    let (mut l, mut r) = (vec![0.0f32; 128], vec![0.0f32; 128]);
    let mut sum = 0.0f64;
    for _ in 0..50 {
        s.block(Path::Plain, &mut l, &mut r);
        sum += l
            .iter()
            .chain(&r)
            .map(|&x| f64::from(x).powi(2))
            .sum::<f64>();
    }
    let rms = (sum / (50.0 * 256.0)).sqrt();
    assert!(rms > 0.05 && rms < 1.0, "{rms}");
}

// The kernel against its twin: run on the ARMv7 build, under qemu-arm in
// CI (`cargo test --target armv7-unknown-linux-gnueabihf -p mpfree-simd`).
#[cfg(all(target_arch = "arm", target_endian = "little", target_os = "linux"))]
mod on_arm {
    use super::*;

    /// Under qemu-arm (and on the MPC) NEON is there, so these tests run the
    /// kernel and can't pass by running the twin twice.
    #[test]
    fn neon_is_there() {
        assert!(neon_available(), "no NEON reported by getauxval(AT_HWCAP)");
        assert_eq!(path(), Path::Neon);
        assert_eq!(status(), "NEON");
    }

    /// One random call on both paths; panics with the case if they differ.
    fn check(rng: &mut Rng, case: usize) {
        let len = 1usize << (3 + rng.below(10)); // 8 to 4096 entries
        let shift = shift_of(len);
        let t = table(rng, len);
        let count = 1 + rng.below(9) as usize;
        let frames = rng.below(70) as usize;
        let start: Vec<u32> = (0..count)
            .map(|_| match rng.below(4) {
                0 => u32::MAX - rng.below(64), // about to wrap
                _ => rng.next(),
            })
            .collect();
        let inc: Vec<u32> = (0..count)
            .map(|_| match rng.below(5) {
                0 => 0,
                1 => u32::MAX - rng.below(1 << 20), // a step backwards
                2 => rng.next(),
                _ => rng.next() >> rng.below(20),
            })
            .collect();
        let gain: Vec<f32> = (0..count)
            .map(|_| if rng.below(8) == 0 { 0.0 } else { rng.unit() })
            .collect();
        let fill: Vec<f32> = (0..frames).map(|_| rng.unit() * 3.0).collect();
        for add in [false, true] {
            let (mut a, mut b) = (fill.clone(), fill.clone());
            let (mut pa, mut pb) = (start.clone(), start.clone());
            let took = if add {
                plain::table_sum::<true>(&t, shift, &mut a, &mut pa, &inc, &gain);
                table_sum_on::<true>(Path::Neon, &t, shift, &mut b, &mut pb, &inc, &gain)
            } else {
                plain::table_sum::<false>(&t, shift, &mut a, &mut pa, &inc, &gain);
                table_sum_on::<false>(Path::Neon, &t, shift, &mut b, &mut pb, &inc, &gain)
            };
            assert_eq!(took == Path::Neon, frames >= 4, "case {case}");
            assert_eq!(
                bits(&a),
                bits(&b),
                "case {case}: len {len} oscillators {count} frames {frames} add {add}"
            );
            assert_eq!(pa, pb, "case {case}: phases");
        }
    }

    #[test]
    fn the_kernel_matches_its_twin_bit_for_bit() {
        let mut rng = Rng(0x5eed);
        for case in 0..3000 {
            check(&mut rng, case);
        }
    }

    /// Poly's own sizes: seven saws, 16-frame steps, every level of a real
    /// band-limited saw, both ways of starting.
    #[test]
    fn the_kernel_matches_its_twin_on_every_saw_level() {
        let mut rng = Rng(42);
        for lv in saw_levels() {
            for add in [false, true] {
                let ph: Vec<u32> = (0..7).map(|_| rng.next()).collect();
                let inc: Vec<u32> = (0..7).map(|_| rng.next() >> 4).collect();
                let gain: Vec<f32> = (0..7).map(|_| rng.unit() * 0.4).collect();
                let fill: Vec<f32> = (0..16).map(|_| rng.unit()).collect();
                let (mut a, mut b) = (fill.clone(), fill);
                let (mut pa, mut pb) = (ph.clone(), ph);
                if add {
                    plain::table_sum::<true>(&lv.table, lv.shift, &mut a, &mut pa, &inc, &gain);
                    table_sum_on::<true>(
                        Path::Neon,
                        &lv.table,
                        lv.shift,
                        &mut b,
                        &mut pb,
                        &inc,
                        &gain,
                    );
                } else {
                    plain::table_sum::<false>(&lv.table, lv.shift, &mut a, &mut pa, &inc, &gain);
                    table_sum_on::<false>(
                        Path::Neon,
                        &lv.table,
                        lv.shift,
                        &mut b,
                        &mut pb,
                        &inc,
                        &gain,
                    );
                }
                assert_eq!(bits(&a), bits(&b), "shift {} add {add}", lv.shift);
                assert_eq!(pa, pb);
            }
        }
    }

    /// The whole workload, many blocks, both paths: the same samples.
    #[test]
    fn the_supersaw_workload_sounds_the_same_on_both_paths() {
        let (mut a, mut b) = (Supersaw::new(8, 44_100), Supersaw::new(8, 44_100));
        let mut buf = [[0.0f32; 128]; 4];
        for _ in 0..200 {
            let [la, ra, lb, rb] = &mut buf;
            assert_eq!(a.block(Path::Plain, la, ra), Path::Plain);
            assert_eq!(b.block(Path::Neon, lb, rb), Path::Neon);
            assert_eq!(bits(la), bits(lb));
            assert_eq!(bits(ra), bits(rb));
        }
    }

    /// Values so small their products are denormal: NEON flushes them, and
    /// with the app's flush-to-zero on (the sound threads') the twin's VFP
    /// code flushes them too, so the two still agree. The lowest table
    /// levels (the largest shift, so the smallest steps) and gains near
    /// zero, as the synth library thread asked.
    #[test]
    fn with_flush_to_zero_on_the_kernel_matches_on_denormals_too() {
        assert!(mpfree_lanes::fpu::flush_to_zero());
        let mut rng = Rng(99);
        for case in 0..500 {
            let len = [8usize, 512, 4096][case % 3];
            let shift = shift_of(len);
            let tiny = |rng: &mut Rng| rng.unit() * 1e-36;
            let t: Vec<[f32; 2]> = (0..len)
                .map(|_| [tiny(&mut rng), tiny(&mut rng) * 1e-9])
                .collect();
            let count = 7;
            let ph: Vec<u32> = (0..count).map(|_| rng.next()).collect();
            let inc: Vec<u32> = (0..count).map(|_| rng.next() >> 3).collect();
            let gain: Vec<f32> = (0..count).map(|_| rng.unit() * 1e-3).collect();
            let fill: Vec<f32> = (0..16).map(|_| tiny(&mut rng)).collect();
            let (mut a, mut b) = (fill.clone(), fill);
            let (mut pa, mut pb) = (ph.clone(), ph);
            plain::table_sum::<true>(&t, shift, &mut a, &mut pa, &inc, &gain);
            table_sum_on::<true>(Path::Neon, &t, shift, &mut b, &mut pb, &inc, &gain);
            assert_eq!(bits(&a), bits(&b), "case {case}");
        }
    }

    #[test]
    fn a_table_that_does_not_fit_runs_the_twin_and_is_counted() {
        let t = vec![[0.25f32, 0.0]; 600]; // not a power of two
        let before = fallbacks();
        let mut buf = [0.0f32; 8];
        let took = table_sum_on::<false>(Path::Neon, &t, 23, &mut buf, &mut [0], &[1], &[1.0]);
        assert_eq!(took, Path::Plain);
        assert_eq!(buf, [0.25; 8]);
        assert_eq!(fallbacks(), before + 1);
    }
}
