//! Prints: a loop at another tempo with its pitch kept.
//!
//! A [`PrintJob`] stretches a loop's [`Bounce`] (from
//! [`crate::Looper::bounce_print`]) to the loop's length at another tempo,
//! with `mpfree-stretch` (the sampler's phase vocoder, which keeps hits
//! sharp), and gives the [`Print`] for [`crate::Looper::print`]. It is
//! heavy work for a worker thread: [`PrintJob::run`] does it in steps.
//!
//! A loop goes round, so its start follows its end. The job stretches the
//! loop with two stretch windows of it wrapped on before and after, and
//! keeps the middle. A held note rarely fits a whole number of its cycles
//! into the new length, so the print's start is crossfaded over
//! [`SEAM`] frames from what the stretch made after its end (which carries
//! on from the end) into its own start: the print joins up at its loop
//! point with no click.

use std::sync::Arc;

use mpfree_stretch::{auto_window, Job, Settings, StretchError, TimeMap};
use mpfree_transport::time::{Tick, PPQ};

use crate::{Bounce, Print};

/// The crossfade at a print's loop point, in frames (at most half the
/// print).
pub const SEAM: usize = 1024;

/// Frames a loop `ticks` long lasts at `bpm`, at `rate`.
pub fn frames_at(ticks: Tick, bpm: f64, rate: u32) -> usize {
    (ticks as f64 * 60.0 * f64::from(rate) / (bpm * PPQ as f64)).round() as usize
}

/// One print being made.
pub struct PrintJob {
    job: Job,
    /// Output frames before the loop (the wrapped-on window, stretched).
    before: usize,
    frames: usize,
    ticks: Tick,
    origin: Tick,
    version: u64,
}

impl PrintJob {
    /// A job making `bounce` (stereo, at `rate`) `frames` long. The
    /// stretch must stay within 10x either way.
    pub fn new(bounce: &Bounce, rate: u32, frames: usize) -> Result<PrintJob, StretchError> {
        let len = bounce.frames.min(bounce.audio.len() / 2);
        if len == 0 || frames == 0 {
            return Err(StretchError::Empty);
        }
        // Two windows of the loop on either side, taken round it.
        let wrap = 2 * auto_window(rate);
        let total = len + 2 * wrap;
        let mut data = Vec::with_capacity(2 * total);
        for i in 0..total {
            let q = (i + len * (wrap / len + 1) - wrap) % len;
            data.push(bounce.audio[2 * q]);
            data.push(bounce.audio[2 * q + 1]);
        }
        let r = frames as f64 / len as f64;
        let before = (wrap as f64 * r).round().max(1.0);
        let (w, l, f) = (wrap as f64, len as f64, frames as f64);
        let map = TimeMap::new(vec![
            (0.0, 0.0),
            (w, before),
            (w + l, before + f),
            (2.0 * w + l, 2.0 * before + f),
        ])?;
        let settings = Settings::default();
        let job = Job::new(Arc::from(data), 2, rate, map, settings)?;
        Ok(PrintJob {
            job,
            before: before as usize,
            frames,
            ticks: bounce.ticks,
            origin: bounce.origin,
            version: bounce.version,
        })
    }

    /// Does about `frames` output frames' worth of work; true once done.
    pub fn run(&mut self, frames: usize) -> bool {
        self.job.run(frames).finished
    }

    /// How far it has got, 0 to 1.
    pub fn progress(&self) -> f32 {
        self.job.progress().done
    }

    /// The print, once [`PrintJob::run`] is done.
    pub fn finish(self) -> Option<Print> {
        let mut audio = self.job.into_output()?;
        let (from, n) = (2 * self.before, 2 * self.frames);
        let seam = SEAM.min(self.frames / 2).min(self.before);
        if audio.len() < from + n + 2 * seam {
            return None;
        }
        for i in 0..seam {
            let w = (i as f32 + 0.5) / seam as f32;
            for c in 0..2 {
                let after = audio[from + n + 2 * i + c];
                let start = &mut audio[from + 2 * i + c];
                *start = after * (1.0 - w) + *start * w;
            }
        }
        audio.drain(..from);
        audio.truncate(n);
        audio.shrink_to_fit();
        Some(Print {
            audio,
            frames: self.frames,
            ticks: self.ticks,
            origin: self.origin,
            version: self.version,
        })
    }
}

/// A whole print at once: `bounce` made `frames` long (see [`PrintJob`]).
pub fn print(bounce: &Bounce, rate: u32, frames: usize) -> Result<Print, StretchError> {
    let mut job = PrintJob::new(bounce, rate, frames)?;
    while !job.run(1 << 16) {}
    job.finish().ok_or(StretchError::Empty)
}
