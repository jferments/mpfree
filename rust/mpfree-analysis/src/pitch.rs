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
//!
//! [`pitch_track`] follows the pitch every 10 ms instead, for chopping a
//! phrase where its notes change ([`PitchTrack::changes`]): YIN on 40 ms
//! windows of the sound brought down to about 8 kHz, then each cut found
//! to the millisecond by which note's period the sound repeats at.

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

/// A sound's pitch over time, for chopping where it changes
/// ([`pitch_track`], [`PitchTrack::changes`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PitchTrack {
    /// Frames of the sound between two readings.
    pub hop: u64,
    /// One reading a hop: the fractional MIDI note heard, or None where
    /// nothing is pitched (silence, noise, a drum hit).
    pub notes: Vec<Option<f32>>,
    /// The sound's length, frames.
    pub frames: u64,
    /// The mono mix at the rate the track listens at, one frame for each
    /// `down` of the sound's, to find exactly where a note starts.
    low: Vec<f32>,
    low_rate: u32,
    down: u64,
    /// A reading's hop and window, frames of `low`.
    low_hop: usize,
    window: usize,
}

/// The rate the pitch track listens at (about: the sound's rate over a
/// whole number): a voice's or an instrument's fundamental is far below
/// it, and YIN's work is a small part of what it would be at 44.1 kHz
/// (about 0.2 s a minute of sound on the workstation).
const TRACK_RATE: u32 = 8_000;
/// Each reading's window, and the time between readings, ms.
const TRACK_WINDOW_MS: f32 = 40.0;
const TRACK_HOP_MS: f32 = 10.0;
/// The pitches a reading looks for, Hz.
const TRACK_MIN_HZ: f32 = 60.0;
const TRACK_MAX_HZ: f32 = 1500.0;
/// Readings more than this far below the loudest (as RMS) count as
/// silence: 30 dB.
const TRACK_FLOOR: f32 = 0.0316;

/// The pitch of `data` (1 or 2 interleaved channels at `rate`) every
/// 10 ms: YIN on 40 ms windows of the mono mix, brought down to about
/// 8 kHz first (averaging, so its fundamental is kept and the cost is
/// low enough for the screen). A reading more than 30 dB below the
/// loudest, or not periodic enough, is None. Steady notes read within
/// a few cents; a reading alone between two others of one note is
/// taken as that note (an octave slip, or a reading missed).
pub fn pitch_track(data: &[f32], channels: usize, rate: u32) -> Result<PitchTrack, AnalysisError> {
    let x = mono(data, channels)?;
    if rate == 0 {
        return Err(AnalysisError::BadInput("rate"));
    }
    let frames = x.len() as u64;
    let down = (rate / TRACK_RATE).max(1) as usize;
    let low_rate = rate / down as u32;
    // Averaged in blocks of `down`: a low-pass with its zeros at the new
    // rate's multiples, then one frame a block.
    let y: Vec<f32> = x
        .chunks(down)
        .map(|c| c.iter().sum::<f32>() / c.len() as f32)
        .collect();
    let (tau_min, tau_max) = lags(
        low_rate,
        TRACK_MIN_HZ,
        TRACK_MAX_HZ.min(low_rate as f32 / 3.0),
    )
    .ok_or(AnalysisError::BadInput("rate"))?;
    let w = ((TRACK_WINDOW_MS / 1000.0 * low_rate as f32) as usize).max(2 * tau_max);
    let hop = ((TRACK_HOP_MS / 1000.0 * low_rate as f32) as usize).max(1);
    let mut yin = Yin::new(w, tau_min, tau_max, 0.2);
    let span = yin.span();
    let mut notes = Vec::new();
    if y.len() >= span {
        let starts: Vec<usize> = (0..=(y.len() - span) / hop).map(|i| i * hop).collect();
        let rms: Vec<f32> = starts.iter().map(|&a| rms_of(&y[a..a + w])).collect();
        let loudest = rms.iter().copied().fold(0.0, f32::max);
        notes = starts
            .iter()
            .zip(&rms)
            .map(|(&a, &r)| {
                if loudest < 1e-4 || r < loudest * TRACK_FLOOR {
                    return None;
                }
                let (hz, ap) = yin.run(&y[a..], low_rate)?;
                (ap < yin.threshold
                    && hz.is_finite()
                    && (TRACK_MIN_HZ..=TRACK_MAX_HZ).contains(&hz))
                .then(|| 69.0 + 12.0 * (hz / 440.0).log2())
            })
            .collect();
        // A reading an octave (or any note) off, or missing, between two
        // that agree is taken as theirs.
        for i in 1..notes.len().saturating_sub(1) {
            if let (Some(a), b, Some(c)) = (notes[i - 1], notes[i], notes[i + 1]) {
                if (a - c).abs() < 0.5 && b.is_none_or(|b| (b - a).abs() >= 0.5) {
                    notes[i] = Some((a + c) / 2.0);
                }
            }
        }
    }
    Ok(PitchTrack {
        hop: (hop * down) as u64,
        notes,
        frames,
        low: y,
        low_rate,
        down: down as u64,
        low_hop: hop,
        window: w,
    })
}

fn rms_of(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

impl PitchTrack {
    /// Where the notes start: the first reading of each run of readings on
    /// one note (within half a semitone of the run's first two) that lasts
    /// `min_frames` at least, when its note is `min_semitones` or more from
    /// the one before (a quarter of a semitone less, so a note sung a
    /// little flat still counts; vibrato, or a slide, which moves a little
    /// at a time, doesn't), or it comes `min_frames` or more after the
    /// last note (a rest). The sound's first note is a cut only after a lead-in as
    /// long. Each cut is then found to the millisecond ([`Self::start`]).
    /// Frames, in order, inside the sound.
    pub fn changes(&self, min_semitones: f32, min_frames: u64) -> Vec<u64> {
        let hop = self.hop.max(1);
        let need = min_frames.div_ceil(hop).max(1) as usize;
        let mut runs: Vec<(usize, usize, f32)> = Vec::new(); // (first, len, note)
        let mut i = 0;
        while i < self.notes.len() {
            let Some(first) = self.notes[i] else {
                i += 1;
                continue;
            };
            // The note the run keeps to: its first readings', once they
            // settle.
            let note = match self.notes.get(i + 1).copied().flatten() {
                Some(next) if (next - first).abs() < 0.5 => (first + next) / 2.0,
                _ => first,
            };
            let mut j = i + 1;
            while j < self.notes.len() && self.notes[j].is_some_and(|n| (n - note).abs() < 0.5) {
                j += 1;
            }
            // Its note, once formed: the middle of its readings, so a first
            // reading caught between two notes doesn't set it.
            let mut heard: Vec<f32> = self.notes[i..j].iter().flatten().copied().collect();
            heard.sort_by(f32::total_cmp);
            runs.push((i, j - i, heard[heard.len() / 2]));
            i = j;
        }
        let change = (min_semitones - 0.25).max(0.25);
        let mut cuts = Vec::new();
        // The last note kept: its end, and its note.
        let mut last: Option<(usize, f32)> = None;
        for (k, &(first, len, note)) in runs.iter().enumerate() {
            if len < need {
                continue;
            }
            let new = match last {
                None => first >= need,
                Some((end, _)) if first >= end + need => true,
                Some((_, before)) => (note - before).abs() >= change,
            };
            if new {
                // The note held just before, if one is (else a rest or
                // noise).
                let near = self.window.div_ceil(self.low_hop);
                let before = runs[..k]
                    .iter()
                    .rev()
                    .find(|r| r.1 >= need)
                    .filter(|&&(f, l, _)| f + l + near >= first)
                    .map(|r| r.2);
                let at = self.start(first, before, note);
                if at > 0 && at < self.frames && cuts.last().is_none_or(|&l| at > l) {
                    cuts.push(at);
                }
            }
            last = Some((first + len, note));
        }
        cuts
    }

    /// Where the note `note` first read at reading `first` starts, frames
    /// of the sound. A window that hears two notes reads one of them or
    /// neither, so the change is somewhere in the window before the first
    /// reading, or this one's. Each millisecond there votes: from a note
    /// `before`, for whichever note's period the next few periods repeat at
    /// better; from a rest or noise, for the new note where it is loud
    /// and repeats at its period. The change is where the votes split
    /// best, old before it and new from it.
    fn start(&self, first: usize, before: Option<f32>, note: f32) -> u64 {
        let y = &self.low;
        let a = first * self.low_hop;
        let lo = a.saturating_sub(self.low_hop);
        let hi = (a + self.window).min(y.len());
        let step = (self.low_rate as usize / 1000).max(1);
        let to_frames = |t: usize| (t as u64 * self.down).min(self.frames);
        if lo >= hi {
            return to_frames(a);
        }
        let period = |n: f32| {
            let hz = 440.0 * 2f32.powf((n - 69.0) / 12.0);
            ((self.low_rate as f32 / hz).round() as usize).max(1)
        };
        let p_new = period(note);
        let p_most = before.map_or(p_new, |o| period(o).max(p_new));
        let span = 2 * p_most;
        // How badly `span` frames from `t` repeat `p` later: 0 for a
        // perfect repeat, about 1 for noise; None for silence.
        let misfit = |t: usize, p: usize| -> Option<f32> {
            let x = y.get(t..t + span + p)?;
            let (mut d, mut e) = (0.0f32, 0.0f32);
            for j in 0..span {
                d += (x[j] - x[j + p]).powi(2);
                e += x[j] * x[j] + x[j + p] * x[j + p];
            }
            (e > 1e-12).then(|| d / e)
        };
        let ts: Vec<usize> = (lo..hi).step_by(step).collect();
        let (votes, reach): (Vec<i32>, usize) = match before {
            Some(old) => {
                let p_old = period(old);
                let votes = ts
                    .iter()
                    .map(|&t| match (misfit(t, p_old), misfit(t, p_new)) {
                        (Some(o), Some(n)) if n < o => 1,
                        (Some(o), Some(n)) if o < n => -1,
                        _ => 0,
                    })
                    .collect();
                // A vote hears the frames from it to the end of its periods:
                // the votes turn when the new note fills half of those.
                (votes, (span + p_most) / 2)
            }
            None => {
                let level: Vec<f32> = ts
                    .iter()
                    .map(|&t| rms_of(&y[t..(t + step).min(y.len())]))
                    .collect();
                let loudest = level.iter().copied().fold(0.0, f32::max);
                let votes = ts
                    .iter()
                    .zip(&level)
                    .map(|(&t, &l)| {
                        let pitched = misfit(t, p_new).is_some_and(|m| m < 0.5);
                        if l >= loudest * 0.1 && pitched {
                            1
                        } else {
                            -1
                        }
                    })
                    .collect();
                (votes, 0)
            }
        };
        // The split that agrees with the most votes (the earliest of
        // equals).
        let mut best = (i32::MIN, 0usize);
        let mut score: i32 = votes.iter().sum();
        for (k, v) in votes.iter().enumerate() {
            if score > best.0 {
                best = (score, k);
            }
            score -= 2 * v;
        }
        to_frames(ts[best.1] + reach)
    }
}
