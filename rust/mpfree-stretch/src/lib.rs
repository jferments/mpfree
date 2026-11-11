//! Offline time stretch and pitch shift for the MPFree sampler.
//!
//! A [`Job`] makes a new sound from an old one: longer or shorter along a
//! [`TimeMap`] (one ratio, a length, or warp markers), higher or lower by
//! up to two octaves without changing its timing, or both. It is heavy
//! work, so it runs on a worker thread in steps ([`Job::run`]) that report
//! progress and can be dropped at any time; the old sound keeps playing
//! until the new one is ready. Nothing here touches the audio thread.
//!
//! How it works:
//!
//! - **Stretch:** a phase vocoder with phase locking (`vocoder.rs`).
//!   Windows of about 46 ms are read where the map says and written at a
//!   steady hop of a quarter window.
//! - **Hits:** the input's hits are found first (the sampler's own onset
//!   detector, `mpfree-analysis`). Every window that holds an attack
//!   reads it at its own speed (from half a window before the hit to
//!   three quarters after), the stretch is taken up between hits, and the
//!   phases start again at the first of those windows, so drums come out
//!   with the attack they went in with.
//! - **Pitch:** the sound is stretched by the pitch ratio and then read
//!   that much faster through a windowed-sinc resampler (`resample.rs`),
//!   low-passed first when it speeds up. Formants move with the pitch, as
//!   on a sampler; keeping them is a later setting.
//! - **Stereo:** both channels turn their phases together, so the image
//!   stays where it was.
//!
//! Our own code, MIT, no C++ to cross-build for the MPC.

#![forbid(unsafe_code)]

mod map;
mod resample;
mod vocoder;

use std::sync::Arc;

pub use map::{TimeMap, MAX_SLOPE, MIN_SLOPE};

use resample::Resampler;
use vocoder::Vocoder;

/// The furthest the pitch moves, either way.
pub const MAX_SEMITONES: f32 = 24.0;

/// What a [`Job`] does besides following its map.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Semitones up (or down, negative), keeping the timing; ±24.
    pub semitones: f32,
    /// The analysis window in frames: a power of two from 256 to 16384, or
    /// 0 for about 46 ms at the sound's rate (2048 frames at 44.1 kHz).
    /// Shorter is tighter on drums, longer smoother on pads.
    pub window: usize,
    /// Keep hits sharp: play each attack at its own speed and start the
    /// phases again there.
    pub transients: bool,
    /// How keen the hit finder is, 0 to 1 (as the Chop page's).
    pub sensitivity: f32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            semitones: 0.0,
            window: 0,
            transients: true,
            sensitivity: 0.6,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StretchError {
    Channels,
    Rate,
    Empty,
    Window,
    Pitch,
    Map(&'static str),
}

impl std::fmt::Display for StretchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StretchError::Channels => write!(f, "only mono and stereo"),
            StretchError::Rate => write!(f, "rate out of range"),
            StretchError::Empty => write!(f, "no audio"),
            StretchError::Window => write!(f, "window must be a power of two, 256 to 16384"),
            StretchError::Pitch => write!(f, "pitch out of range (±24 semitones)"),
            StretchError::Map(why) => write!(f, "time map: {why}"),
        }
    }
}

impl std::error::Error for StretchError {}

/// How far a job has got.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progress {
    /// 0 to 1.
    pub done: f32,
    pub finished: bool,
}

/// The window for `rate`: about 46 ms, a power of two.
pub fn auto_window(rate: u32) -> usize {
    ((rate as f64 * 0.046).round() as usize)
        .next_power_of_two()
        .clamp(256, 16_384)
}

enum Stage {
    /// The vocoder, then (for a pitch shift) the resampler.
    Vocoder(Box<Vocoder>, Option<Resampler>),
    Resample(Vec<f32>, Resampler),
    Done,
}

/// One stretch or pitch shift, run in steps.
pub struct Job {
    ch: usize,
    out_frames: usize,
    stage: Stage,
    output: Vec<f32>,
    resets: usize,
    hits: usize,
}

impl Job {
    /// A job on `data` (interleaved, 1 or 2 channels, at `rate`): the
    /// output follows `map` and is shifted by `settings.semitones`. Finding
    /// the hits happens here, so it takes a moment on long sounds; call it
    /// on the worker too.
    pub fn new(
        data: Arc<[f32]>,
        channels: u8,
        rate: u32,
        map: TimeMap,
        settings: Settings,
    ) -> Result<Job, StretchError> {
        let ch = usize::from(channels);
        if ch != 1 && ch != 2 {
            return Err(StretchError::Channels);
        }
        if !(8000..=192_000).contains(&rate) {
            return Err(StretchError::Rate);
        }
        if data.is_empty() || !data.len().is_multiple_of(ch) {
            return Err(StretchError::Empty);
        }
        if !settings.semitones.is_finite() || settings.semitones.abs() > MAX_SEMITONES {
            return Err(StretchError::Pitch);
        }
        let n = match settings.window {
            0 => auto_window(rate),
            w if w.is_power_of_two() && (256..=16_384).contains(&w) => w,
            _ => return Err(StretchError::Window),
        };
        let frames_in = (data.len() / ch) as u64;
        if (map.input_frames() - frames_in as f64).abs() > 0.5 {
            return Err(StretchError::Map("must cover the whole input"));
        }
        let out_frames = map.output_frames() as usize;
        if out_frames == 0 {
            return Err(StretchError::Empty);
        }
        let speed = 2f64.powf(f64::from(settings.semitones) / 12.0);
        let shifting = settings.semitones != 0.0;
        let identity = !shifting && map.points().iter().all(|&(x, y)| (x - y).abs() < 1e-9);
        if identity {
            return Ok(Job {
                ch,
                out_frames,
                stage: Stage::Done,
                output: data.to_vec(),
                resets: 0,
                hits: 0,
            });
        }
        let hits = if settings.transients {
            let found = mpfree_analysis::OnsetAnalysis::analyse(&data, ch, rate)
                .map(|o| o.pick(settings.sensitivity.clamp(0.0, 1.0)))
                .unwrap_or_default();
            sharp(&data, ch, &found, n)
        } else {
            Vec::new()
        };
        let resampler = shifting.then(|| Resampler::new(speed));
        let mut vmap = if shifting { map.scaled(speed) } else { map };
        // Every window holding an attack reads it at its own speed, and the
        // phases start again at the first of them.
        let (before, after) = ((n / 2) as u64, (n * 3 / 4) as u64);
        if settings.transients {
            vmap = vmap.locked(&hits, before as f64, after as f64);
        }
        let restarts: Vec<u64> = hits.iter().map(|h| h.saturating_sub(before)).collect();
        let v_frames = match &resampler {
            Some(r) => (out_frames as f64 * speed).ceil() as usize + r.margin() + 1,
            None => out_frames,
        };
        let n_hits = hits.len();
        let vocoder = Vocoder::new(data, ch, vmap, restarts, n, v_frames);
        Ok(Job {
            ch,
            out_frames,
            stage: Stage::Vocoder(Box::new(vocoder), resampler),
            output: Vec::new(),
            resets: 0,
            hits: n_hits,
        })
    }

    /// How long the result is, in frames.
    pub fn output_frames(&self) -> u64 {
        self.out_frames as u64
    }

    pub fn channels(&self) -> u8 {
        self.ch as u8
    }

    /// Hits found in the input, and phase restarts made at them.
    pub fn hits(&self) -> (usize, usize) {
        (self.hits, self.resets)
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.stage, Stage::Done)
    }

    /// Does about `frames` output frames' worth of work (a few ms on the
    /// MPC for a few thousand frames) and says how far the job has got.
    pub fn run(&mut self, frames: usize) -> Progress {
        let frames = frames.max(1);
        let stage = std::mem::replace(&mut self.stage, Stage::Done);
        self.stage = match stage {
            Stage::Vocoder(mut v, r) => {
                let steps = frames.div_ceil(v.hop());
                v.run(steps);
                if !v.finished() {
                    Stage::Vocoder(v, r)
                } else {
                    self.resets = v.resets;
                    let out = std::mem::take(&mut v.out);
                    match r {
                        Some(r) => {
                            self.output = vec![0.0; self.out_frames * self.ch];
                            Stage::Resample(out, r)
                        }
                        None => {
                            self.output = out;
                            Stage::Done
                        }
                    }
                }
            }
            Stage::Resample(input, mut r) => {
                r.run(&input, self.ch, &mut self.output, frames);
                if r.done() >= self.out_frames {
                    Stage::Done
                } else {
                    Stage::Resample(input, r)
                }
            }
            Stage::Done => Stage::Done,
        };
        self.progress()
    }

    pub fn progress(&self) -> Progress {
        let done = match &self.stage {
            Stage::Vocoder(v, r) => {
                let share = if r.is_some() { 0.75 } else { 1.0 };
                v.progress() * share
            }
            Stage::Resample(_, r) => 0.75 + 0.25 * r.done() as f32 / self.out_frames.max(1) as f32,
            Stage::Done => 1.0,
        };
        Progress {
            done: done.min(1.0),
            finished: self.is_finished(),
        }
    }

    /// The result (interleaved, [`Job::output_frames`] long), once
    /// finished.
    pub fn into_output(self) -> Option<Vec<f32>> {
        self.is_finished().then_some(self.output)
    }
}

/// The hits in `found` that really jump: the brightness (the energy of
/// each frame's change from the last, both channels) in the eighth of a
/// window after the hit at least four times what it was over the half
/// window before. Beating partials or a swell can fool the hit finder, and
/// starting the phases again there would only break a steady sound.
fn sharp(data: &[f32], ch: usize, found: &[u64], n: usize) -> Vec<u64> {
    let frames = data.len() / ch;
    let bright = |from: usize, to: usize| -> f64 {
        let (from, to) = (from.max(1), to.min(frames));
        if to <= from {
            return 0.0;
        }
        let mut sum = 0.0f64;
        for i in from..to {
            for c in 0..ch {
                let d = f64::from(data[i * ch + c] - data[(i - 1) * ch + c]);
                sum += d * d;
            }
        }
        sum / (to - from) as f64
    };
    found
        .iter()
        .copied()
        .filter(|&h| {
            let h = h as usize;
            let after = bright(h, h + n / 8);
            let before = bright(h.saturating_sub(n / 2), h);
            after > 4.0 * before + 1e-12
        })
        .collect()
}

/// Runs a whole job at once: `data` stretched along `map` and shifted by
/// `settings.semitones`.
pub fn stretch(
    data: &[f32],
    channels: u8,
    rate: u32,
    map: TimeMap,
    settings: Settings,
) -> Result<Vec<f32>, StretchError> {
    let mut job = Job::new(data.into(), channels, rate, map, settings)?;
    while !job.run(1 << 16).finished {}
    Ok(job.into_output().unwrap_or_default())
}
