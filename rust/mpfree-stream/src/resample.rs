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

    /// Crossings a side.
    pub fn zeros(&self) -> usize {
        self.zeros
    }

    /// Table steps a crossing.
    pub fn steps(&self) -> usize {
        self.steps
    }

    /// The table: its length is a power of two, and past `zeros`
    /// crossings it holds zeros.
    pub fn table(&self) -> &[f32] {
        &self.table
    }
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
