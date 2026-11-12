//! Takes stretched to another tempo, keeping their pitch.
//!
//! A take plays at the rate it was recorded at, so after a tempo change it
//! no longer fills the ticks it was recorded over. Fitting it makes a
//! stretched copy with `mpfree-stretch` (the sampler's phase vocoder,
//! which keeps hits sharp) and plays the copy in its place:
//!
//! 1. Off the audio thread, read the take ([`crate::AudioTracks::read_take`])
//!    and make a [`StretchJob`] with [`fit_ratio`] for the tempo now.
//! 2. Run it in steps on a worker ([`StretchJob::run`]).
//! 3. Load the result under a free id ([`crate::AudioTracks::free_take_id`],
//!    [`crate::AudioTracks::load_take`]). Its anchor is the old take's, so
//!    frame 0 stays on the same tick.
//! 4. Once it has copied in, [`crate::Command::Swap`] plays it wherever the
//!    old take played, as one undo step. The old take stays (Undo goes
//!    back to it) until it is forgotten.
//!
//! The stretch is one ratio over the whole take: right for a take recorded
//! at one tempo and played at one tempo, which is the usual case.

use std::sync::Arc;

use mpfree_stretch::{Job, Progress, Settings, StretchError, TimeMap};

use crate::save::TakeFile;
use crate::TakeInfo;

/// How many times longer a take recorded at `recorded` BPM must be to fit
/// `now` BPM; None if either is not a tempo.
pub fn fit_ratio(recorded: f64, now: f64) -> Option<f64> {
    (recorded.is_finite() && now.is_finite() && recorded > 0.0 && now > 0.0).then(|| recorded / now)
}

/// One take being stretched; see the module notes. Allocates: run it on a
/// worker.
pub struct StretchJob {
    job: Job,
    take: TakeInfo,
    new_id: u16,
    ratio: f64,
    now_bpm: f64,
}

impl StretchJob {
    /// Stretch `audio` (the take's, interleaved, as `read_take` gives it)
    /// `ratio` times as long, for a take to load as `new_id`. `now_bpm` is
    /// the tempo it is fitted to (saved as the new take's tempo).
    pub fn new(
        take: TakeInfo,
        audio: Vec<f32>,
        rate: u32,
        ratio: f64,
        now_bpm: f64,
        new_id: u16,
    ) -> Result<StretchJob, StretchError> {
        let map = TimeMap::ratio(take.frames as u64, ratio)?;
        let job = Job::new(
            Arc::from(audio),
            take.channels as u8,
            rate,
            map,
            Settings::default(),
        )?;
        Ok(StretchJob {
            job,
            take,
            new_id,
            ratio,
            now_bpm,
        })
    }

    /// Run about `frames` frames of input; how far it has got.
    pub fn run(&mut self, frames: usize) -> Progress {
        self.job.run(frames)
    }

    pub fn ratio(&self) -> f64 {
        self.ratio
    }

    /// The take that was stretched.
    pub fn take(&self) -> &TakeInfo {
        &self.take
    }

    /// The stretched take, ready for [`crate::AudioTracks::load_take`],
    /// and the [`crate::Command::Swap`] that plays it in the old one's
    /// place once loaded; None until the job is finished.
    pub fn finish(self) -> Option<(TakeFile, crate::Command)> {
        let ch = self.take.channels;
        let audio = self.job.into_output()?;
        let frames = audio.len() / ch;
        Some((
            TakeFile {
                id: self.new_id,
                track: self.take.track,
                group: self.take.group,
                pass: self.take.pass,
                bpm: self.now_bpm,
                anchor: self.take.anchor,
                frames,
                channels: ch,
                file: String::new(),
                audio,
            },
            crate::Command::Swap {
                old: self.take.id,
                new: self.new_id,
                ratio: self.ratio,
            },
        ))
    }
}
