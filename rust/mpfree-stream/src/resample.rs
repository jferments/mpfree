//! Playing a file at the engine's rate (disk streaming's P3, step 3):
//! stems and clips recorded at another rate are resampled ahead, off the
//! sound thread, into engine-rate cache units, so the sound thread reads
//! them at speed 1 like any other.
//!
//! Each engine frame is made from the file's frames around its exact
//! place with a Kaiser-windowed sinc. Its cutoff sits a little under the
//! lower of the two rates' Nyquist frequencies, so nothing above what the
//! engine can carry folds back down. The kernel is symmetric, so there is
//! no delay to make up: engine frame 0 is file frame 0, and a 48 kHz stem
//! stays sample-aligned with a 44.1 kHz one. Places are worked out in
//! whole numbers (frame × file rate ÷ engine rate), so they never drift,
//! however long the file.
//!
//! A unit needs only the file frames within the kernel's reach either
//! side of it, read with it ([`Rate::span`]). So each unit comes out
//! exactly as one pass over the whole file would make it: the
//! resampler's state carries across units by construction, and a seek, a
//! loop or a unit that went missing changes nothing.

use std::sync::OnceLock;

/// Zero crossings of the sinc on each side, at the cutoff.
pub const ZEROS: usize = 24;
/// The cutoff, as a share of the lower Nyquist frequency: at 44.1 kHz
/// the pass band is flat to about 18 kHz and what would fold back is cut
/// by about 60 dB.
pub const CUTOFF: f64 = 0.92;
/// The Kaiser window's shape (about 60 dB of stop band).
const BETA: f64 = 5.65;
/// Table steps per zero crossing, read with linear interpolation.
const STEPS: usize = 512;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// One side of a Kaiser-windowed sinc, tabled: shared by the conversion
/// here and the cursors' band-limited read ([`crate::cursor::Read`]).
pub struct Kernel {
    zeros: usize,
    steps: usize,
    /// From 0 to `zeros` crossings, then zeros up to a power of two, so
    /// interpolation never reads past the end.
    table: Box<[f32]>,
}

impl Kernel {
    /// `zeros` crossings a side, a Kaiser window of shape `beta`, `steps`
    /// table steps a crossing. Exactly 0 at every crossing, so a read at a
    /// whole frame, unstretched, is that frame's own sample.
    pub fn new(zeros: usize, beta: f64, steps: usize) -> Kernel {
        let n = zeros * steps;
        // Zeros past the edge up to a power of two, so a read can mask its
        // index instead of checking it.
        let mut table = vec![0.0f32; (n + 2).next_power_of_two()].into_boxed_slice();
        let i0 = bessel_i0(beta);
        for (i, v) in table.iter_mut().enumerate().take(n + 1) {
            if i % steps == 0 && i != 0 {
                continue;
            }
            let x = i as f64 / steps as f64;
            let sinc = if i == 0 {
                1.0
            } else {
                let px = std::f64::consts::PI * x;
                px.sin() / px
            };
            let r = x / zeros as f64;
            let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0;
            *v = (sinc * w) as f32;
        }
        Kernel {
            zeros,
            steps,
            table,
        }
    }

    /// The 8-point read's weights ([`near_weight`]) as one side of a
    /// table, `steps` a frame, for a read that stretches them with its
    /// speed (the cursors' [`crate::cursor::Read::Near`] faster than the
    /// file's own speed). Its [`Kernel::zeros`] is then its reach,
    /// [`NEAR_HALF`] frames; past that it holds zeros.
    pub fn near(steps: usize) -> Kernel {
        let n = NEAR_HALF * steps;
        let mut table = vec![0.0f32; (n + 2).next_power_of_two()].into_boxed_slice();
        for (i, v) in table.iter_mut().enumerate().take(n + 1) {
            *v = near_weight(i as f64 / steps as f64) as f32;
        }
        Kernel {
            zeros: NEAR_HALF,
            steps,
            table,
        }
    }

    /// Crossings a side (frames, for [`Kernel::near`]).
    pub fn zeros(&self) -> usize {
        self.zeros
    }

    /// Table steps a crossing (a frame, for [`Kernel::near`]).
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// The table: its length is a power of two, and past `zeros`
    /// crossings it holds zeros.
    pub fn table(&self) -> &[f32] {
        &self.table
    }
}

/// Frames the 8-point read weighs on each side of its position: it reads
/// 2 × `NEAR_HALF` frames, from the position's frame less `NEAR_HALF - 1`
/// on.
pub const NEAR_HALF: usize = 4;
/// Frames the 8-point read weighs.
pub const NEAR_TAPS: usize = 2 * NEAR_HALF;
/// The 8-point read's rows per frame of fraction, read with a straight
/// line between them.
const NEAR_ROWS: usize = 256;
/// Its cutoff, a fraction of the sound's rate: 20.1 kHz for a 44.1 kHz
/// sound.
const NEAR_FC: f64 = 0.455;
/// Its Kaiser window.
const NEAR_BETA: f64 = 5.0;

/// The 8-point read's weight for a frame `d` frames from the read
/// position (`d` from −[`NEAR_HALF`] to [`NEAR_HALF`]), before a read's
/// weights are summed to one: a sinc cut at [`NEAR_FC`] under a Kaiser
/// window [`NEAR_HALF`] frames a side.
pub fn near_weight(d: f64) -> f64 {
    let r = d / NEAR_HALF as f64;
    let w = bessel_i0(NEAR_BETA * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(NEAR_BETA);
    let x = 2.0 * NEAR_FC * d;
    let sinc = if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    };
    sinc * w
}

/// The 8-point read's exact weights for a read `t` (0 to 1) past a frame,
/// summing to one: the table holds these at each row and steps linearly
/// between them.
fn near_row(t: f64) -> [f64; NEAR_TAPS] {
    let mut h = [0.0; NEAR_TAPS];
    for (j, e) in h.iter_mut().enumerate() {
        // Frame i - (NEAR_HALF - 1) + j, from position i + t.
        *e = near_weight(j as f64 - (NEAR_HALF as f64 - 1.0) - t);
    }
    let sum: f64 = h.iter().sum();
    h.map(|x| x / sum)
}

/// The 8-point read's table (DSP quality step 1.4): for each of
/// [`NEAR_ROWS`] fractions of a frame, the [`NEAR_TAPS`] weights and their
/// steps to the next row's. 16 KB.
///
/// The read the sampler's pads use at their sound's own speed or slower
/// and up to two semitones up, and the cursors' cheapest read
/// ([`crate::cursor::Read::Near`]): a Kaiser-windowed sinc over 8 frames,
/// cut at 0.455 of the sound's rate, each row's weights summing to one.
/// Within two semitones either way every image of the sound stays 50 dB
/// down and a tone keeps its level within 1 dB to 13.5 kHz out; a fifth
/// down, images stay 50 dB down to 9 kHz out. Tones at the very top of a
/// sound pitched well down still fold (18 kHz an octave down at −22 dB, 20
/// kHz a fifth down at −9 dB); passing those would take about 48 frames.
///
/// Unlike Hermite it is not the frame itself on a whole frame: its window
/// dulls the top there (−0.9 dB at 15 kHz, −2.5 dB at 18 kHz, −3.7 dB at
/// 20 kHz), so a read at the sound's own speed copies the frames instead.
/// The measures are DSP quality's (`docs/dev/design/dsp-quality.md`, step
/// 1.4, and `pitched-reads.md`).
pub struct NearKernel([[[f32; NEAR_TAPS]; 2]; NEAR_ROWS]);

/// The 8-point read's table, made the first time it is asked for: ask
/// once off the sound thread (as the sampler's engine and a new cursor
/// do).
pub fn near_kernel() -> &'static NearKernel {
    static NEAR: OnceLock<NearKernel> = OnceLock::new();
    NEAR.get_or_init(|| {
        let mut k = NearKernel([[[0.0; NEAR_TAPS]; 2]; NEAR_ROWS]);
        for (p, e) in k.0.iter_mut().enumerate() {
            let a = near_row(p as f64 / NEAR_ROWS as f64);
            let b = near_row((p + 1) as f64 / NEAR_ROWS as f64);
            for j in 0..NEAR_TAPS {
                e[0][j] = a[j] as f32;
                e[1][j] = (b[j] - a[j]) as f32;
            }
        }
        k
    })
}

impl NearKernel {
    /// The weights for a read `frac` (a 32.32 position's low 32 bits) past
    /// a frame: the top 8 bits pick the row, the other 24 move toward the
    /// next.
    #[inline(always)]
    pub fn weights(&self, frac: u32) -> [f32; NEAR_TAPS] {
        let [a, d] = &self.0[(frac >> 24) as usize];
        let u = (frac & 0x00ff_ffff) as f32 * (1.0 / (1u32 << 24) as f32);
        let mut c = [0.0; NEAR_TAPS];
        for j in 0..NEAR_TAPS {
            c[j] = a[j] + u * d[j];
        }
        c
    }
}

/// The 8-point read's sum over one channel's [`NEAR_TAPS`] frames, in four
/// running sums (one vector register), the same order every time.
#[inline(always)]
pub fn near_sum(c: &[f32; NEAR_TAPS], x: &[f32; NEAR_TAPS]) -> f32 {
    let mut acc = [0.0f32; 4];
    for q in 0..NEAR_TAPS / 4 {
        for m in 0..4 {
            acc[m] += c[4 * q + m] * x[4 * q + m];
        }
    }
    (acc[0] + acc[1]) + (acc[2] + acc[3])
}

/// [`near_sum`] for two channels at once, `x` interleaved left and right:
/// (left, right), each summed in the same order as the other.
#[inline(always)]
pub fn near_sum2(c: &[f32; NEAR_TAPS], x: &[f32; 2 * NEAR_TAPS]) -> (f32, f32) {
    let mut acc = [0.0f32; 4];
    for q in 0..NEAR_TAPS / 2 {
        for m in 0..4 {
            acc[m] += c[2 * q + m / 2] * x[4 * q + m];
        }
    }
    (acc[0] + acc[2], acc[1] + acc[3])
}

fn table() -> &'static Kernel {
    static T: OnceLock<Kernel> = OnceLock::new();
    T.get_or_init(|| Kernel::new(ZEROS, BETA, STEPS))
}

/// One file's conversion to the engine's rate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rate {
    /// The file's frames a second.
    from: u32,
    /// The engine's.
    to: u32,
    /// File frames.
    native: u64,
    /// File frames per crossing of the kernel: the kernel is stretched to
    /// cut at the lower rate.
    stretch: f64,
    /// File frames the kernel reaches on each side.
    reach: u64,
}

impl Rate {
    /// The conversion of a file of `native` frames at `from` frames a
    /// second to the engine's `to`; none if the rates are the same (44.1
    /// kHz audio isn't converted) or either is 0.
    pub fn new(from: u32, to: u32, native: u64) -> Option<Rate> {
        if from == to || from == 0 || to == 0 {
            return None;
        }
        let ratio = f64::from(from) / f64::from(to);
        let stretch = ratio.max(1.0) / CUTOFF;
        Some(Rate {
            from,
            to,
            native,
            stretch,
            reach: (ZEROS as f64 * stretch).ceil() as u64,
        })
    }

    /// File frames per engine frame.
    pub fn ratio(&self) -> f64 {
        f64::from(self.from) / f64::from(self.to)
    }

    /// Engine frames: every one whose place falls inside the file.
    pub fn frames(&self) -> u64 {
        let n = u128::from(self.native) * u128::from(self.to);
        u64::try_from(n.div_ceil(u128::from(self.from))).unwrap_or(u64::MAX)
    }

    /// File frames the kernel reaches on each side of a place.
    pub fn reach(&self) -> u64 {
        self.reach
    }

    /// Engine frame `e`'s place in the file: a whole frame and the
    /// fraction past it.
    fn place(&self, e: u64) -> (u64, f64) {
        let n = u128::from(e) * u128::from(self.from);
        let to = u128::from(self.to);
        ((n / to) as u64, (n % to) as f64 / f64::from(self.to))
    }

    /// The file frames (`start..end`) engine frames `first..first +
    /// frames` are made from.
    pub fn span(&self, first: u64, frames: u64) -> (u64, u64) {
        if frames == 0 {
            return (0, 0);
        }
        let (a, _) = self.place(first);
        let (b, _) = self.place(first + frames - 1);
        let start = (a + 1).saturating_sub(self.reach);
        let end = (b + self.reach + 1).min(self.native);
        (start.min(end), end)
    }

    /// Make engine frames from `first` into `out` (interleaved,
    /// `channels` a frame, as many frames as fit), from `input`: the file's
    /// frames from `at` on, decoded to floats. Frames outside the file
    /// count as silence. `input` must hold [`Rate::span`] of them.
    pub fn run(&self, channels: usize, input: &[f32], at: u64, first: u64, out: &mut [f32]) {
        const ONE: f64 = (1u64 << 16) as f64;
        const INV: f32 = 1.0 / (1u32 << 16) as f32;
        let tab = table().table();
        let mask = tab.len() - 1;
        let ch = channels.max(1);
        // The frames in hand: inside the file and inside `input`.
        let from = at;
        let to = (at + (input.len() / ch) as u64).min(self.native);
        // Table steps between neighbouring frames, in fixed point.
        let per = STEPS as f64 / self.stretch;
        let step = (per * ONE) as u32;
        let r = self.reach;
        let tap = |pos: u32| {
            let i = (pos >> 16) as usize;
            let a = tab[i & mask];
            a + (pos & 0xFFFF) as f32 * INV * (tab[(i + 1) & mask] - a)
        };
        for (j, o) in out.chunks_exact_mut(ch).enumerate() {
            let (whole, frac) = self.place(first + j as u64);
            o.fill(0.0);
            let mut total = 0.0f32;
            // Frame whole, whole - 1, ...: frac, frac + 1, ... frames from
            // the place; then whole + 1, whole + 2, ...: 1 - frac, 2 - frac.
            let left = (frac * per * ONE) as u32;
            let right = ((1.0 - frac) * per * ONE) as u32;
            if whole + 1 >= from + r && whole + r < to {
                // Every frame the kernel reaches is in hand.
                let at = (whole + 1 - r - from) as usize * ch;
                let (before, after) = input[at..at + 2 * r as usize * ch].split_at(r as usize * ch);
                let mut pos = left;
                for f in before.chunks_exact(ch).rev() {
                    let w = tap(pos);
                    for (o, &x) in o.iter_mut().zip(f) {
                        *o += w * x;
                    }
                    total += w;
                    pos += step;
                }
                let mut pos = right;
                for f in after.chunks_exact(ch) {
                    let w = tap(pos);
                    for (o, &x) in o.iter_mut().zip(f) {
                        *o += w * x;
                    }
                    total += w;
                    pos += step;
                }
            } else {
                // Near the file's edges: frames outside it are silence,
                // and their weights still count.
                let mut add = |k: Option<u64>, w: f32| {
                    total += w;
                    if let Some(k) = k.filter(|&k| k >= from && k < to) {
                        let x = &input[(k - from) as usize * ch..][..ch];
                        for (o, &x) in o.iter_mut().zip(x) {
                            *o += w * x;
                        }
                    }
                };
                let mut pos = left;
                for m in 0..r {
                    add(whole.checked_sub(m), tap(pos));
                    pos += step;
                }
                let mut pos = right;
                for m in 1..=r {
                    add(Some(whole + m), tap(pos));
                    pos += step;
                }
            }
            // Weights summed to 1 at every place, so the level doesn't
            // ripple with the fraction.
            if total != 0.0 {
                for o in o.iter_mut() {
                    *o /= total;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_near_table_matches_the_exact_kernel() {
        // Every row, halfway between rows and a spread of other fractions:
        // the table's weights against the exact ones, and their sum.
        let k = near_kernel();
        let mut fracs: Vec<u32> = (0..256u32)
            .flat_map(|r| [r << 24, (r << 24) | 0x0080_0000])
            .collect();
        let mut x = 0x9e37_79b9u32;
        for _ in 0..4096 {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            fracs.push(x);
        }
        let (mut worst_tap, mut worst_row, mut worst_sum) = (0.0f64, 0.0f64, 0.0f64);
        for &f in &fracs {
            let got = k.weights(f);
            let want = near_row(f64::from(f) / 4_294_967_296.0);
            let mut row = 0.0;
            for j in 0..NEAR_TAPS {
                let e = (f64::from(got[j]) - want[j]).abs();
                worst_tap = worst_tap.max(e);
                row += e;
            }
            worst_row = worst_row.max(row);
            worst_sum = worst_sum.max((got.iter().map(|&w| f64::from(w)).sum::<f64>() - 1.0).abs());
        }
        eprintln!("near table: tap {worst_tap:.2e}, row {worst_row:.2e}, sum {worst_sum:.2e}");
        // The table rows themselves are the exact weights, to f32.
        for r in 0..256u32 {
            let got = k.weights(r << 24);
            let want = near_row(f64::from(r) / 256.0);
            for j in 0..NEAR_TAPS {
                assert!(
                    (f64::from(got[j]) - want[j]).abs() < 1e-7,
                    "row {r} tap {j}"
                );
            }
        }
        assert!(worst_tap < 1e-5, "worst tap {worst_tap:e}");
        assert!(worst_row < 2e-5, "worst row {worst_row:e}");
        assert!(worst_sum < 1e-6, "worst sum {worst_sum:e}");
    }

    #[test]
    fn the_stretched_near_table_holds_the_same_weights() {
        // The one-sided table a faster read stretches is the same kernel:
        // at each of its steps, the exact weight before the sum to one.
        let k = Kernel::near(128);
        assert_eq!(k.zeros(), NEAR_HALF);
        for i in 0..=NEAR_HALF * 128 {
            let d = i as f64 / 128.0;
            let want = near_weight(d);
            assert!((f64::from(k.table()[i]) - want).abs() < 1e-7, "{d}");
        }
        assert!(k.table()[NEAR_HALF * 128 + 1..].iter().all(|&w| w == 0.0));
    }
}
