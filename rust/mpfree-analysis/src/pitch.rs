//! Root pitch: the note a sample plays, so it can be tuned to the project's
//! key or spread across a keygroup.
//!
//! [`yin`] is the YIN estimator (de Cheveigné and Kawahara, 2002): the
//! cumulative mean normalised difference function with an absolute threshold
//! and parabolic refinement. The difference function is computed with an
//! FFT, so a 100 ms window down to 40 Hz costs three 16k-point transforms.
//!
//! [`root_pitch`] looks at up to [`MAX_WINDOWS`] 100 ms windows within
//! 24 dB of the loudest, runs YIN on each, groups the voiced ones by note
//! and picks the note with the most weight (loudness × periodicity ×
//! steadiness). That ignores a noisy attack, a pitch glide (an 808's thump)
//! and the odd octave slip.

use crate::fft::Fft;
use crate::{mono, AnalysisError};

/// Windows examined per sample, at most. Keeps long samples cheap.
pub const MAX_WINDOWS: usize = 24;

/// What [`root_pitch`] looks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PitchSettings {
    /// Lowest pitch reported, Hz.
    pub min_hz: f32,
    /// Highest pitch reported, Hz.
    pub max_hz: f32,
    /// Length of each analysed window, ms.
    pub window_ms: f32,
    /// YIN's absolute threshold on the normalised difference (0.1 to 0.2 is
    /// usual). Windows whose best dip is above it count as unpitched.
    pub threshold: f32,
}

impl Default for PitchSettings {
    fn default() -> Self {
        PitchSettings {
            min_hz: 40.0,
            max_hz: 2000.0,
            window_ms: 100.0,
            threshold: 0.15,
        }
    }
}

/// A detected root pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pitch {
    pub hz: f32,
    /// Fractional MIDI note number (69 is A4 at 440 Hz).
    pub midi: f32,
    /// The nearest MIDI note.
    pub note: u8,
    /// How far from `note`, −50 to +50 cents.
    pub cents: f32,
    /// 0 to 1: how periodic the chosen window is, times how much of the
    /// voiced weight agrees on the note.
    pub confidence: f32,
    /// First frame of the window the pitch was read from.
    pub frame: u64,
}

impl Pitch {
    fn from_hz(hz: f32, confidence: f32, frame: u64) -> Pitch {
        let midi = 69.0 + 12.0 * (hz / 440.0).log2();
        let note = midi.round().clamp(0.0, 127.0);
        Pitch {
            hz,
            midi,
            note: note as u8,
            cents: (midi - note) * 100.0,
            confidence,
            frame,
        }
    }

    /// Semitones to add to play this sample at `target` (a MIDI note).
    pub fn semitones_to(&self, target: u8) -> f32 {
        f32::from(target) - self.midi
    }
}

/// A MIDI note's name with its octave, middle C (60) being "C4".
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!(
        "{}{}",
        NAMES[usize::from(note % 12)],
        i32::from(note / 12) - 1
    )
}

/// Reusable YIN state for one window length and lag range.
struct Yin {
    w: usize,
    threshold: f32,
    tau_min: usize,
    tau_max: usize,
    fft: Fft,
    re: Vec<f32>,
    im: Vec<f32>,
    re2: Vec<f32>,
    im2: Vec<f32>,
    d: Vec<f32>,
}

impl Yin {
    fn new(w: usize, tau_min: usize, tau_max: usize, threshold: f32) -> Yin {
        let n = (2 * w + tau_max).next_power_of_two();
        Yin {
            w,
            threshold,
            tau_min,
            tau_max,
            fft: Fft::new(n),
            re: vec![0.0; n],
            im: vec![0.0; n],
            re2: vec![0.0; n],
            im2: vec![0.0; n],
            d: vec![0.0; tau_max + 2],
        }
    }

    /// Frames each call needs.
    fn span(&self) -> usize {
        self.w + self.tau_max + 1
    }

    /// `(hz, aperiodicity)` for `x` (at least `span()` frames), or `None`
    /// for silence.
    fn run(&mut self, x: &[f32], rate: u32) -> Option<(f32, f32)> {
        let (w, tmax) = (self.w, self.tau_max);
        let x = &x[..self.span()];
        // r(τ) = Σ_{j<w} x[j]·x[j+τ], as the inverse transform of conj(A)·B.
        self.re.fill(0.0);
        self.im.fill(0.0);
        self.re2.fill(0.0);
        self.im2.fill(0.0);
        self.re[..w].copy_from_slice(&x[..w]);
        self.re2[..x.len()].copy_from_slice(x);
        self.fft.forward(&mut self.re, &mut self.im);
        self.fft.forward(&mut self.re2, &mut self.im2);
        for k in 0..self.re.len() {
            let (ar, ai, br, bi) = (self.re[k], self.im[k], self.re2[k], self.im2[k]);
            // conj(A)·B, then conjugated again for the inverse-by-forward trick.
            self.re[k] = ar * br + ai * bi;
            self.im[k] = -(ar * bi - ai * br);
        }
        self.fft.forward(&mut self.re, &mut self.im);
        let scale = 1.0 / self.re.len() as f64;
        // d(τ) = E(0) + E(τ) − 2 r(τ), energies from prefix sums (in f64).
        let mut prefix = Vec::with_capacity(x.len() + 1);
        prefix.push(0.0f64);
        for &v in x {
            let last = *prefix.last().expect("not empty");
            prefix.push(last + f64::from(v) * f64::from(v));
        }
        let e0 = prefix[w];
        if e0 < 1e-9 * w as f64 {
            return None;
        }
        for tau in 0..=tmax {
            let r = f64::from(self.re[tau]) * scale;
            let e = prefix[tau + w] - prefix[tau];
            self.d[tau] = (e0 + e - 2.0 * r).max(0.0) as f32;
        }
        // Cumulative mean normalised difference, in place.
        self.d[0] = 1.0;
        let mut sum = 0.0f64;
        for tau in 1..=tmax {
            sum += f64::from(self.d[tau]);
            self.d[tau] = if sum > 0.0 {
                (f64::from(self.d[tau]) * tau as f64 / sum) as f32
            } else {
                1.0
            };
        }
        let d = &self.d[..=tmax];
        // First dip under the threshold, walked down to its bottom; else the deepest dip.
        let tmin = self.tau_min.max(2);
        let mut best = None;
        let mut tau = tmin;
        while tau < tmax {
            if d[tau] < self.threshold {
                while tau + 1 < tmax && d[tau + 1] < d[tau] {
                    tau += 1;
                }
                best = Some(tau);
                break;
            }
            tau += 1;
        }
        let tau = best.unwrap_or_else(|| {
            (tmin..tmax)
                .min_by(|&a, &b| d[a].total_cmp(&d[b]))
                .unwrap_or(tmin)
        });
        // Parabolic refinement through the dip.
        let (a, b, c) = (d[tau - 1], d[tau], d[tau + 1]);
        let den = a - 2.0 * b + c;
        let shift = if den.abs() > 1e-12 {
            (0.5 * (a - c) / den).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        let period = tau as f32 + shift;
        Some((rate as f32 / period, b.clamp(0.0, 1.0)))
    }
}

/// YIN on one window: `(hz, aperiodicity)` for the first `window` frames
/// of `x`, searching `min_hz..max_hz`. `x` must hold `window` frames plus
/// one period of `min_hz` (and one more frame). Aperiodicity is 0 for a
/// perfectly periodic signal and near 1 for noise. `None` for silence or
/// when `x` is too short.
pub fn yin(
    x: &[f32],
    rate: u32,
    window: usize,
    min_hz: f32,
    max_hz: f32,
    threshold: f32,
) -> Option<(f32, f32)> {
    let (tau_min, tau_max) = lags(rate, min_hz, max_hz)?;
    if window < 2 {
        return None;
    }
    let mut y = Yin::new(window, tau_min, tau_max, threshold);
    if x.len() < y.span() {
        return None;
    }
    y.run(x, rate)
}

fn lags(rate: u32, min_hz: f32, max_hz: f32) -> Option<(usize, usize)> {
    if !(min_hz > 0.0 && max_hz > min_hz) || rate == 0 {
        return None;
    }
    let tau_min = ((rate as f32 / max_hz).floor() as usize).max(2);
    let tau_max = (rate as f32 / min_hz).ceil() as usize + 1;
    (tau_max > tau_min + 2).then_some((tau_min, tau_max))
}

/// The root pitch of a sample (1 or 2 interleaved channels), or `None` if
/// nothing in it is pitched. See the module notes for how windows are
/// chosen.
pub fn root_pitch(
    data: &[f32],
    channels: usize,
    rate: u32,
    s: PitchSettings,
) -> Result<Option<Pitch>, AnalysisError> {
    let x = mono(data, channels)?;
    if rate == 0 {
        return Err(AnalysisError::BadInput("rate"));
    }
    let (tau_min, tau_max) =
        lags(rate, s.min_hz, s.max_hz).ok_or(AnalysisError::BadInput("pitch range"))?;
    let w = ((s.window_ms.clamp(10.0, 1000.0) / 1000.0 * rate as f32) as usize).max(2 * tau_max);
    let mut y = Yin::new(w, tau_min, tau_max, s.threshold.clamp(0.01, 0.5));
    let span = y.span();
    if x.len() < span {
        return Ok(None);
    }
    // Candidate windows every half window, kept if within 24 dB of the
    // loudest: enough of a decaying tail to outweigh a pitched attack.
    let hop = (w / 2).max(1);
    let starts: Vec<usize> = (0..=(x.len() - span) / hop).map(|i| i * hop).collect();
    let rms: Vec<f32> = starts
        .iter()
        .map(|&a| (x[a..a + w].iter().map(|v| v * v).sum::<f32>() / w as f32).sqrt())
        .collect();
    let loudest = rms.iter().copied().fold(0.0, f32::max);
    if loudest < 1e-3 {
        return Ok(None);
    }
    let keep: Vec<usize> = (0..starts.len())
        .filter(|&i| rms[i] >= loudest * 0.063)
        .collect();
    let picked: Vec<usize> = if keep.len() <= MAX_WINDOWS {
        keep
    } else {
        (0..MAX_WINDOWS)
            .map(|k| keep[k * (keep.len() - 1) / (MAX_WINDOWS - 1)])
            .collect()
    };
    // (midi, aperiodicity, weight, frame) for each voiced window, in time order.
    let mut voiced = Vec::new();
    for &i in &picked {
        if let Some((hz, ap)) = y.run(&x[starts[i]..], rate) {
            if ap < y.threshold && hz.is_finite() && (s.min_hz..=s.max_hz).contains(&hz) {
                let midi = 69.0 + 12.0 * (hz / 440.0).log2();
                voiced.push((midi, ap, rms[i] * (1.0 - ap), starts[i]));
            }
        }
    }
    // Steady windows count for more than ones in a glide or a vibrato swing:
    // each weight is divided by how far its pitch is from its neighbours'.
    let pitches: Vec<f32> = voiced.iter().map(|v| v.0).collect();
    for (k, v) in voiced.iter_mut().enumerate() {
        let near: Vec<f32> = [k.checked_sub(1), Some(k + 1)]
            .into_iter()
            .flatten()
            .filter_map(|j| pitches.get(j))
            .map(|p| (p - v.0).abs())
            .collect();
        let jump = if near.is_empty() {
            0.0
        } else {
            near.iter().copied().fold(f32::INFINITY, f32::min)
        };
        v.2 /= 1.0 + 4.0 * jump;
    }
    if voiced.is_empty() {
        return Ok(None);
    }
    // The note with the most weight wins; ties go to the lower note.
    let total: f32 = voiced.iter().map(|v| v.2).sum();
    let note_of = |m: f32| m.round() as i32;
    let mut best_note = note_of(voiced[0].0);
    let mut best_weight = -1.0;
    for v in &voiced {
        let n = note_of(v.0);
        let wsum: f32 = voiced
            .iter()
            .filter(|u| note_of(u.0) == n)
            .map(|u| u.2)
            .sum();
        if wsum > best_weight || (wsum == best_weight && n < best_note) {
            best_weight = wsum;
            best_note = n;
        }
    }
    let mut group: Vec<_> = voiced
        .iter()
        .filter(|v| note_of(v.0) == best_note)
        .copied()
        .collect();
    // The most periodic window of the winning note gives the frame; the
    // median of the group gives the pitch.
    let steadiest = group
        .iter()
        .copied()
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("group is not empty");
    group.sort_by(|a, b| a.0.total_cmp(&b.0));
    let median = group[group.len() / 2].0;
    let hz = 440.0 * ((median - 69.0) / 12.0).exp2();
    let confidence = ((1.0 - steadiest.1) * best_weight / total).clamp(0.0, 1.0);
    Ok(Some(Pitch::from_hz(hz, confidence, steadiest.3 as u64)))
}
