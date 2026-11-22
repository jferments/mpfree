//! Clean pitching: a band-limited read for voices that play a sound faster
//! than its own rate, and octave copies of a sound for reads more than an
//! octave up.
//!
//! A voice reading its sound `r` times faster than the output's rate
//! (`r` > 1: tuned up, played above a keygroup's root, or a file recorded
//! at a higher rate) moves everything up by `r`, and whatever lands above
//! the output's Nyquist frequency folds back down as aliasing. The
//! engine's Hermite read does nothing about that: a 15 kHz tone an octave
//! up comes out as a 14.1 kHz tone at full level.
//!
//! [`read2`] low-passes as it reads. Each output frame is a sum of the
//! frames around the read position weighted by a Kaiser-windowed sinc
//! stretched by `r`, so its cutoff sits at the output's Nyquist frequency
//! whatever the speed. The window is short ([`HALF`] zero crossings each
//! side), so the cut is gentle: what folds back lands above about 17 kHz,
//! and everything that would fold lower is at least 50 dB down. Its
//! length grows with `r`, so past an octave a voice reads an octave copy
//! instead ([`Octaves`]): the sound low-passed and halved in length, read
//! at `r / 2`, and the same again for each octave. The read never stretches
//! the sinc more than twice, so it costs 16 to 32 taps a frame.
//!
//! Copies are made off the audio thread ([`prepare`]): at once for short
//! sounds, on the sampler's job service ([`crate::jobs`]) for longer ones.
//! A voice more than an octave up with no copy to read (it started before
//! they were made, or a live tune took a pad there that the view didn't
//! expect) reads the sound itself with the sinc stretched twice: what folds stays above about
//! 16 kHz up to two and a half times the speed, and comes lower past that.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use crate::dsp::Sample;
use crate::jobs::{Job, Jobs, Key, Lane, Order};
use crate::model::Pcm;

/// Zero crossings of the read's sinc on each side of the read position,
/// counted in output frames.
pub const HALF: usize = 8;
/// Table steps per zero crossing, read with linear interpolation (its
/// error is under 1e-4 of the kernel's peak).
const STEPS: usize = 128;
/// The table's length: a power of two past the farthest a tap reaches
/// (under `HALF + 1` crossings), so an index masked to it needs no bounds
/// check. Past [`HALF`] crossings it holds zeros.
const TABLE: usize = 2048;
const MASK: usize = TABLE - 1;
/// A tap's place in the table is a fixed-point number with this many
/// fraction bits.
const FRAC_BITS: u32 = 16;
const FRAC_MASK: u32 = (1 << FRAC_BITS) - 1;
/// The read's Kaiser window (about 54 dB of stop band).
const BETA: f64 = 5.0;
/// The most the read stretches its sinc: past this a voice reads an
/// octave copy, or (with none yet) under-filters a little.
pub const MAX_STRETCH: f32 = 2.0;

/// Octave copies made at most: enough for 5 octaves up from the sound's
/// own rate.
pub const LEVELS: usize = 5;
/// Sounds this long or shorter (in frames) get their copies at once when
/// asked ([`prepare`]): a fraction of a millisecond each on the
/// workstation. Longer ones are made on the copies' own thread.
pub const SYNC_FRAMES: usize = 1 << 15;
/// Sounds longer than this (about 3 minutes at 44.1 kHz) get no copies.
pub const MAX_FRAMES: usize = 1 << 23;

/// The halving filter's taps on each side of its centre (a half-band
/// low-pass: every other tap is zero).
const HB_HALF: usize = 23;
/// The halving filter's Kaiser window (about 70 dB of stop band): it
/// passes up to 0.2 of the rate it filters and stops from 0.3, so what
/// folds while halving lands in the top fifth of the copy, which the read
/// filters out again.
const HB_BETA: f64 = 7.0;

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// The read's kernel table. Each entry is the kernel's value and its slope
/// to the next entry, per fixed-point step, so a tap's weight is one
/// multiply and add.
pub struct Kernel([[f32; 2]; TABLE]);

static KERNEL: OnceLock<Kernel> = OnceLock::new();

/// The read's windowed sinc, [`STEPS`] entries per zero crossing from the
/// centre out, zero from [`HALF`] on. Made once (on the stack, without
/// allocating); the engine asks for it when it is made so the audio
/// thread never has to wait for it, and a render asks once, not every
/// frame.
pub fn kernel() -> &'static Kernel {
    KERNEL.get_or_init(|| {
        let i0b = bessel_i0(BETA);
        let at = |i: usize| -> f32 {
            if i.is_multiple_of(STEPS) && i > 0 {
                // The sinc's zero crossings, exactly.
                return 0.0;
            }
            let u = i as f64 / STEPS as f64;
            if u >= HALF as f64 {
                return 0.0;
            }
            let r = u / HALF as f64;
            let w = bessel_i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / i0b;
            (sinc(u) * w) as f32
        };
        let mut t = [[0.0f32; 2]; TABLE];
        let mut next = at(0);
        for (i, e) in t.iter_mut().enumerate() {
            let a = next;
            next = at(i + 1);
            *e = [a, (next - a) / (1u32 << FRAC_BITS) as f32];
        }
        Kernel(t)
    })
}

/// The kernel at fixed-point table place `u`.
#[inline(always)]
fn weight(k: &Kernel, u: u32) -> f32 {
    let [a, d] = k.0[(u >> FRAC_BITS) as usize & MASK];
    a + d * (u & FRAC_MASK) as i32 as f32
}

/// How a block of reads stretches the sinc: `r` frames of the data read
/// per output frame (at least 1, at most [`MAX_STRETCH`]), as the step
/// between two taps' places in the kernel table, and the taps on each
/// side that reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reach {
    step: u32,
    taps: i64,
}

impl Default for Reach {
    /// No stretch.
    fn default() -> Reach {
        Reach::new(1.0)
    }
}

impl Reach {
    /// Its two numbers, to keep elsewhere ([`Reach::from_parts`] makes it
    /// again): a grain keeps them in its `mpfree_grain::Aim`.
    pub fn parts(self) -> (u32, u32) {
        (self.step, self.taps as u32)
    }

    /// The reach whose [`Reach::parts`] these were.
    pub fn from_parts(step: u32, taps: u32) -> Reach {
        Reach {
            step,
            taps: i64::from(taps),
        }
    }

    pub fn new(r: f32) -> Reach {
        let r = r.clamp(1.0, MAX_STRETCH);
        let one = (STEPS << FRAC_BITS) as f64;
        Reach {
            step: (one / f64::from(r)).round() as u32,
            taps: (HALF as f32 * r).ceil() as i64,
        }
    }
}

/// The output's two channels at 32.32 fixed-point position `pos` in
/// `data` (interleaved when `stereo`), read through `k` ([`kernel`]) as
/// [`Reach`] says, low-passed
/// so that nothing above the output's Nyquist frequency folds back.
/// Frames outside `lo..=hi` read as the nearest of the two (as the Hermite
/// read clamps to the region). `picks` are what each output channel takes:
/// 0 the left, 1 the right, 2 the two summed and halved (a mono sound
/// gives its one channel to both). The weights are normalised, so a
/// steady level reads back exactly; at a whole frame with no stretch it
/// gives that frame.
///
/// It walks out from the read position both ways at once, frame `i - m`
/// and frame `i + 1 + m` together, each into sums of its own, so the two
/// halves' arithmetic overlaps.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub fn read2<S: Sample>(
    k: &Kernel,
    data: &[S],
    stereo: bool,
    lo: i64,
    hi: i64,
    pos: i64,
    reach: Reach,
    picks: (u8, u8),
) -> (f32, f32) {
    let i = pos >> 32;
    let du = reach.step;
    // Places of the first tap back (frame i) and forward (frame i + 1):
    // t and 1 - t frames from the position, t its fraction.
    let mut ub = ((u64::from(pos as u32) * u64::from(du)) >> 32) as u32;
    let mut uf = du - ub;
    let taps = reach.taps;
    let n0 = i - taps + 1;
    let n1 = i + taps;
    // Sums back and forward: weights, left, right.
    let (mut wb, mut lb, mut rb) = (0.0f32, 0.0f32, 0.0f32);
    let (mut wf, mut lf, mut rf) = (0.0f32, 0.0f32, 0.0f32);
    if stereo {
        if n0 >= lo && n1 <= hi {
            let back = &data[2 * n0 as usize..2 * (i as usize + 1)];
            let fwd = &data[2 * (i as usize + 1)..2 * (n1 as usize + 1)];
            for (b, f) in back.chunks_exact(2).rev().zip(fwd.chunks_exact(2)) {
                let (x, y) = (weight(k, ub), weight(k, uf));
                wb += x;
                lb += x * b[0].raw();
                rb += x * b[1].raw();
                wf += y;
                lf += y * f[0].raw();
                rf += y * f[1].raw();
                ub += du;
                uf += du;
            }
        } else {
            for m in 0..taps {
                let jb = (i - m).clamp(lo, hi) as usize;
                let jf = (i + 1 + m).clamp(lo, hi) as usize;
                let (x, y) = (weight(k, ub), weight(k, uf));
                wb += x;
                lb += x * data[2 * jb].raw();
                rb += x * data[2 * jb + 1].raw();
                wf += y;
                lf += y * data[2 * jf].raw();
                rf += y * data[2 * jf + 1].raw();
                ub += du;
                uf += du;
            }
        }
    } else if n0 >= lo && n1 <= hi {
        let back = &data[n0 as usize..=i as usize];
        let fwd = &data[i as usize + 1..=n1 as usize];
        for (b, f) in back.iter().rev().zip(fwd) {
            let (x, y) = (weight(k, ub), weight(k, uf));
            wb += x;
            lb += x * b.raw();
            wf += y;
            lf += y * f.raw();
            ub += du;
            uf += du;
        }
    } else {
        for m in 0..taps {
            let jb = (i - m).clamp(lo, hi) as usize;
            let jf = (i + 1 + m).clamp(lo, hi) as usize;
            let (x, y) = (weight(k, ub), weight(k, uf));
            wb += x;
            lb += x * data[jb].raw();
            wf += y;
            lf += y * data[jf].raw();
            ub += du;
            uf += du;
        }
    }
    // The sums are of raw samples: scaled once here, which gives exactly
    // what summing the scaled samples did.
    let g = 1.0 / (wb + wf);
    let l = (lb + lf) * g * S::SCALE;
    if !stereo {
        return (l, l);
    }
    let r = (rb + rf) * g * S::SCALE;
    let side = |p: u8| match p {
        0 => l,
        1 => r,
        _ => 0.5 * (l + r),
    };
    (side(picks.0), side(picks.1))
}

/// A sound's octave copies: copy `k` (from 1) is the sound low-passed to
/// a little under a `2^k`th of its rate's Nyquist frequency and holding
/// every `2^k`th frame, its frame `j` lined up with the sound's frame
/// `j * 2^k` (the filter is symmetric, so nothing is delayed). Interleaved
/// as the sound is.
pub struct Octaves {
    levels: Vec<Arc<[f32]>>,
}

impl std::fmt::Debug for Octaves {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Octaves")
            .field("levels", &self.levels.len())
            .finish()
    }
}

impl Octaves {
    /// The copies of `data` (`channels` interleaved): up to [`LEVELS`],
    /// fewer for a sound too short to halve again.
    pub fn make<S: Sample>(data: &[S], channels: usize) -> Octaves {
        let taps = halving_taps();
        let mut levels: Vec<Arc<[f32]>> = Vec::new();
        for _ in 0..LEVELS {
            let next: Vec<f32> = match levels.last() {
                None if data.len() / channels >= 2 * HB_HALF => halve(data, channels, &taps),
                Some(from) if from.len() / channels >= 2 * HB_HALF => {
                    halve(&from[..], channels, &taps)
                }
                _ => break,
            };
            levels.push(next.into());
        }
        Octaves { levels }
    }

    /// [`Octaves::make`] for a sound's samples as it holds them.
    pub fn make_from(data: &Pcm, channels: usize) -> Octaves {
        match data {
            Pcm::F32(d) => Octaves::make(d, channels),
            Pcm::I16(d) => Octaves::make(d, channels),
        }
    }

    /// How many copies there are.
    pub fn count(&self) -> usize {
        self.levels.len()
    }

    /// Copy `k`, from 1.
    pub fn level(&self, k: usize) -> Option<&Arc<[f32]>> {
        k.checked_sub(1).and_then(|i| self.levels.get(i))
    }

    /// The bytes the copies take.
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(|l| l.len() * 4).sum()
    }
}

/// The halving filter: a Kaiser-windowed half-band low-pass, `2 *
/// HB_HALF + 1` taps (centre first, then each side's), summing to 1.
fn halving_taps() -> Vec<f64> {
    let i0b = bessel_i0(HB_BETA);
    let n = HB_HALF as f64 + 1.0;
    let mut h: Vec<f64> = (0..=HB_HALF)
        .map(|m| {
            let r = m as f64 / n;
            0.5 * sinc(0.5 * m as f64) * bessel_i0(HB_BETA * (1.0 - r * r).max(0.0).sqrt()) / i0b
        })
        .collect();
    let sum: f64 = h[0] + 2.0 * h[1..].iter().sum::<f64>();
    for x in &mut h {
        *x /= sum;
    }
    h
}

/// `data` low-passed and halved: frame `j` is the filter centred on frame
/// `2j`, with silence beyond the ends.
fn halve<S: Sample>(data: &[S], channels: usize, h: &[f64]) -> Vec<f32> {
    let frames = data.len() / channels;
    let out_frames = frames.div_ceil(2);
    let mut out = vec![0.0f32; out_frames * channels];
    let at = |n: i64, c: usize| -> f64 {
        if n < 0 || n >= frames as i64 {
            0.0
        } else {
            f64::from(data[n as usize * channels + c].f())
        }
    };
    for j in 0..out_frames {
        let centre = 2 * j as i64;
        for c in 0..channels {
            let mut acc = h[0] * at(centre, c);
            // Only the odd taps are not zero.
            let mut m = 1;
            while m <= HB_HALF {
                acc += h[m] * (at(centre - m as i64, c) + at(centre + m as i64, c));
                m += 2;
            }
            out[j * channels + c] = acc as f32;
        }
    }
    out
}

/// Where a sound's octave copies are kept once made, shared by every
/// clone of the sound, and whether they have been asked for.
#[derive(Default)]
pub struct OctaveCell {
    built: OnceLock<Octaves>,
    asked: AtomicBool,
}

impl std::fmt::Debug for OctaveCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OctaveCell")
            .field("built", &self.built.get())
            .field("asked", &self.asked.load(Ordering::Relaxed))
            .finish()
    }
}

impl OctaveCell {
    /// The copies, once made. Never blocks or allocates: safe on the
    /// audio thread.
    #[inline]
    pub fn get(&self) -> Option<&Octaves> {
        self.built.get()
    }
}

/// What making the octave copies of a sound of `frames` frames and
/// `channels` channels holds: the copies, as floats, half the sound each
/// time, under the sound's length as floats in all.
pub fn need(frames: usize, channels: usize) -> u64 {
    (frames as u64)
        .saturating_mul(4 * channels as u64)
        .saturating_add(1_000_000)
}

/// The service's key for a sound's copies: its cell, which the job holds.
fn key(cell: &Arc<OctaveCell>) -> Key {
    Key::new("octaves", Arc::as_ptr(cell) as usize as u128)
}

/// Makes sure a sound's octave copies are made or on their way: at once
/// for a sound of up to [`SYNC_FRAMES`] frames, on the process's job
/// service ([`Jobs::shared`]) for a longer one, never for one longer than
/// [`MAX_FRAMES`]. Does nothing if they are made or asked for already.
/// Copies the memory floor can't spare are refused (and counted by the
/// service), and a later call asks again. Off the audio thread only (it may
/// allocate, and briefly wait for the service's lock).
pub fn prepare(cell: &Arc<OctaveCell>, data: &Pcm, channels: u8) {
    prepare_on(&Jobs::shared(), cell, data, channels);
}

/// [`prepare`] on `jobs`.
fn prepare_on(jobs: &Jobs, cell: &Arc<OctaveCell>, data: &Pcm, channels: u8) {
    if cell.built.get().is_some() {
        return;
    }
    let channels = usize::from(channels.max(1));
    let frames = data.len() / channels;
    if frames > MAX_FRAMES {
        return;
    }
    if frames <= SYNC_FRAMES {
        let _ = cell.built.set(Octaves::make_from(data, channels));
        return;
    }
    if cell.asked.swap(true, Ordering::AcqRel) {
        return;
    }
    let (built, refused) = (cell.clone(), cell.clone());
    let data = data.clone();
    jobs.submit(
        Job::new(Lane::Sound, key(cell), move |_| {
            let _ = built.built.set(Octaves::make_from(&data, channels));
        })
        .order(Order::First)
        .needs(need(frames, channels), move |_| {
            // Left without copies for now: the next view built asks again.
            refused.asked.store(false, Ordering::Release);
        }),
    );
}

/// Makes a sound's octave copies here and now (any length up to
/// [`MAX_FRAMES`]), if they aren't made already: for offline renders and
/// tests, which mustn't depend on a thread's timing.
pub fn prepare_now(cell: &OctaveCell, data: &Pcm, channels: u8) {
    let channels = usize::from(channels.max(1));
    if cell.built.get().is_none() && data.len() / channels <= MAX_FRAMES {
        let _ = cell.built.set(Octaves::make_from(data, channels));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(frame: f64) -> i64 {
        (frame * 4_294_967_296.0).round() as i64
    }

    #[test]
    fn a_whole_frame_unstretched_reads_that_frame() {
        let data: Vec<f32> = (0..64).map(|i| ((i * 37) % 11) as f32 - 5.0).collect();
        for i in 10..50 {
            let (l, r) = read2(
                kernel(),
                &data,
                false,
                0,
                63,
                (i as i64) << 32,
                Reach::new(1.0),
                (0, 0),
            );
            assert!((l - data[i]).abs() < 1e-5, "{i}: {l} vs {}", data[i]);
            assert_eq!(l, r);
        }
    }

    #[test]
    fn a_steady_level_reads_back_exactly_at_any_stretch_and_position() {
        let data = vec![0.25f32; 400];
        for r in [1.0, 1.3, 1.99, 2.0] {
            for p in [3.0, 100.25, 199.5, 396.75] {
                let (l, _) = read2(
                    kernel(),
                    &data,
                    false,
                    0,
                    399,
                    pos(p),
                    Reach::new(r),
                    (0, 0),
                );
                assert!((l - 0.25).abs() < 1e-6, "r {r} at {p}: {l}");
            }
        }
    }

    #[test]
    fn stereo_picks_take_each_side_or_their_sum() {
        let data: Vec<f32> = (0..200).flat_map(|_| [1.0f32, -0.5]).collect();
        let at = pos(77.3);
        let reach = Reach::new(1.4);
        let (l, r) = read2(kernel(), &data, true, 0, 199, at, reach, (0, 1));
        assert!((l - 1.0).abs() < 1e-6 && (r + 0.5).abs() < 1e-6);
        let (a, b) = read2(kernel(), &data, true, 0, 199, at, reach, (2, 1));
        assert!((a - 0.25).abs() < 1e-6 && (b + 0.5).abs() < 1e-6);
        let (a, b) = read2(kernel(), &data, true, 0, 199, at, reach, (1, 0));
        assert!((a + 0.5).abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
    }

    #[test]
    fn near_an_edge_the_read_clamps_as_far_from_it_it_reads_direct() {
        // The clamped loop and the direct one agree where both apply: a
        // region inside the data, read away from its edges, gives what
        // the whole data gives.
        let data: Vec<f32> = (0..300).map(|i| (i as f32 * 0.37).sin()).collect();
        let reach = Reach::new(1.7);
        for p in [50.0, 100.4, 149.9] {
            let a = read2(kernel(), &data, false, 0, 299, pos(p), reach, (0, 0));
            let b = read2(kernel(), &data, false, 20, 250, pos(p), reach, (0, 0));
            assert_eq!(a, b, "at {p}");
        }
        // At the region's first frame, the frames before it read as it.
        let (x, _) = read2(
            kernel(),
            &data,
            false,
            40,
            250,
            40 << 32,
            Reach::new(1.0),
            (0, 0),
        );
        assert!(x.is_finite());
    }

    /// The level of `freq` (cycles a frame) in `x`, by correlation.
    fn level(x: &[f32], freq: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (n, &v) in x.iter().enumerate() {
            let ph = 2.0 * std::f64::consts::PI * freq * n as f64;
            re += f64::from(v) * ph.cos();
            im += f64::from(v) * ph.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

    #[test]
    fn halving_keeps_the_low_band_and_drops_the_top() {
        let n = 8192;
        let low: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * 0.1 * i as f64).sin() as f32)
            .collect();
        let high: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * 0.35 * i as f64).sin() as f32)
            .collect();
        let o = Octaves::make(&low, 1);
        assert_eq!(o.count(), LEVELS);
        let l1 = o.level(1).unwrap();
        assert_eq!(l1.len(), n / 2);
        // 0.1 of the rate is 0.2 of the copy's.
        let kept = level(&l1[100..l1.len() - 100], 0.2);
        assert!((kept - 1.0).abs() < 0.01, "kept {kept}");
        let o = Octaves::make(&high, 1);
        let l1 = o.level(1).unwrap();
        // 0.35 would fold to 0.3 of the copy's rate.
        let folded = level(&l1[100..l1.len() - 100], 0.3);
        assert!(folded < 10f64.powf(-65.0 / 20.0), "folded {folded}");
    }

    #[test]
    fn a_copys_frames_line_up_with_the_sounds() {
        // A slow sine halved: copy frame j is near sound frame 2j.
        let n = 4096;
        let x: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * 0.01 * i as f64).sin() as f32)
            .collect();
        let o = Octaves::make(&x, 1);
        let l1 = o.level(1).unwrap();
        for j in 100..1900 {
            assert!(
                (l1[j] - x[2 * j]).abs() < 2e-3,
                "{j}: {} vs {}",
                l1[j],
                x[2 * j]
            );
        }
        let l2 = o.level(2).unwrap();
        for j in 100..900 {
            assert!((l2[j] - x[4 * j]).abs() < 4e-3, "{j}");
        }
    }

    #[test]
    fn a_long_sounds_copies_are_made_on_the_job_service_unless_memory_is_short() {
        let long = Pcm::F32(vec![0.1f32; SYNC_FRAMES + 10].into());
        // 1 MB more than the floor free: the copies' 1.1 MB is refused.
        let short = Jobs::new(crate::jobs::Config {
            free: || Some(257_000_000),
            room: |free, need| match free {
                Some(f) if f.saturating_sub(need) >= 256_000_000 => Ok(()),
                _ => Err("not enough memory".into()),
            },
            ..crate::jobs::Config::default()
        });
        let cell = Arc::new(OctaveCell::default());
        prepare_on(&short, &cell, &long, 1);
        let t = std::time::Instant::now();
        while short.counts().refused == 0 {
            assert!(t.elapsed().as_secs() < 20, "never refused");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert!(cell.get().is_none());
        assert!(
            !cell.asked.load(Ordering::Acquire),
            "a later call asks again"
        );
        let jobs = Jobs::new(crate::jobs::Config::default());
        prepare_on(&jobs, &cell, &long, 1);
        let t = std::time::Instant::now();
        while cell.get().is_none() {
            assert!(t.elapsed().as_secs() < 20, "the copies never came");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(jobs.counts().done, 1);
        assert!(!jobs.has(key(&cell)));
    }

    #[test]
    fn short_sounds_get_their_copies_at_once_and_long_ones_soon() {
        let short = Pcm::F32(vec![0.1f32; 2 * 1000].into());
        let cell = Arc::new(OctaveCell::default());
        prepare(&cell, &short, 2);
        assert!(cell.get().is_some());
        assert_eq!(cell.get().unwrap().level(1).unwrap().len(), 1000);

        let long = Pcm::F32(vec![0.1f32; SYNC_FRAMES + 10].into());
        let cell = Arc::new(OctaveCell::default());
        prepare(&cell, &long, 1);
        let t = std::time::Instant::now();
        while cell.get().is_none() {
            assert!(t.elapsed().as_secs() < 20, "the copies never came");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        assert_eq!(cell.get().unwrap().count(), LEVELS);
    }
}
