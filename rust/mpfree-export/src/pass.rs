//! The sound thread's half of an export: it sets the capture points, feeds
//! their blocks to the bounce, logs the notes, and says when the tail is
//! done.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_fx::{BounceIn, CapturePoint, Mixer, StripId};
use mpfree_midi::msg::Msg;
use mpfree_midi::smf::NoteLog;

use crate::mode::Mode;

/// After the range ends, the export stops once every stream has been
/// silent (below about -90 dBFS) this long: long enough for a slow delay's
/// next echo. The silence isn't kept: the files end where the sound does.
pub const TAIL_QUIET_S: f64 = 2.0;
/// And at most this long after the range, whatever still sounds.
pub const TAIL_MAX_S: f64 = 10.0;

/// Not yet: the range hasn't ended.
pub(crate) const NOT_ENDED: u64 = u64::MAX;

/// What the pass and the job share.
pub(crate) struct Shared {
    pub cancel: AtomicBool,
    /// Frames the pass has played.
    pub played: AtomicU64,
    /// The frame the range ended on, or [`NOT_ENDED`].
    pub ended_at: AtomicU64,
    /// Frames every file has so far.
    pub written: AtomicU64,
}

impl Shared {
    pub fn new() -> Shared {
        Shared {
            cancel: AtomicBool::new(false),
            played: AtomicU64::new(0),
            ended_at: AtomicU64::new(NOT_ENDED),
            written: AtomicU64::new(0),
        }
    }
}

/// Why a pass failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The files couldn't keep up and blocks were lost (this many).
    Overrun(u32),
    /// The mixer refused a capture point (too few free, or a strip it
    /// doesn't have), or the pass wasn't attached.
    Capture,
    /// A capture point's latency changed during the pass (an effect with
    /// look-ahead was put in or taken out): the files would no longer
    /// line up.
    Latency,
}

/// What [`Pass::period`] says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Play the next block.
    Go,
    /// The range and its tail are recorded: stop the transport.
    Done,
    /// Cancel was pressed: stop the transport.
    Cancelled,
    Failed(Failure),
}

/// The sound thread's half (see the module docs). Made by
/// [`crate::start`] off the sound thread; nothing in it allocates.
pub struct Pass {
    pub(crate) bin: BounceIn,
    points: Vec<(StripId, CapturePoint)>,
    first_slot: usize,
    lat: Vec<u32>,
    /// The longest latency: the files lag the song this much.
    most: u64,
    attached: bool,
    played: u64,
    ended_at: Option<u64>,
    quiet_frames: usize,
    max_tail: u64,
    pub(crate) notes: Option<NoteLog>,
    pub(crate) shared: Arc<Shared>,
    mode: Mode,
    pub(crate) over: Option<Step>,
}

impl Pass {
    pub(crate) fn new(
        bin: BounceIn,
        points: Vec<(StripId, CapturePoint)>,
        first_slot: usize,
        notes: Option<NoteLog>,
        shared: Arc<Shared>,
        mode: Mode,
        rate: u32,
    ) -> Pass {
        let n = points.len();
        Pass {
            bin,
            points,
            first_slot,
            lat: vec![0; n],
            most: 0,
            attached: false,
            played: 0,
            ended_at: None,
            quiet_frames: (TAIL_QUIET_S * f64::from(rate)) as usize,
            max_tail: (TAIL_MAX_S * f64::from(rate)) as u64,
            notes,
            shared,
            mode,
            over: None,
        }
    }

    /// How this export plays: Fast (the sound loop runs it as fast as it
    /// can, quiet) or Listen (in real time, heard).
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Sets the export's capture points on the mixer and takes their
    /// latencies. Call it on the sound thread when the pass arrives,
    /// before its first period. False if the mixer refused one: the pass
    /// has then failed, and holds no capture point.
    pub fn attach(&mut self, mixer: &mut Mixer) -> bool {
        if self.over.is_some() {
            return false;
        }
        for (i, &(strip, point)) in self.points.iter().enumerate() {
            if !mixer.set_capture(self.first_slot + i, Some((strip, point))) {
                for j in 0..i {
                    mixer.set_capture(self.first_slot + j, None);
                }
                self.end(Step::Failed(Failure::Capture));
                return false;
            }
        }
        for (i, l) in self.lat.iter_mut().enumerate() {
            *l = mixer.capture_latency(self.first_slot + i).unwrap_or(0);
        }
        self.most = u64::from(self.lat.iter().copied().max().unwrap_or(0));
        self.bin.begin(&self.lat);
        self.attached = true;
        true
    }

    /// After the mixer, each block of `frames`: copies every capture
    /// point's block into the files' rings. `ended` says the range is over
    /// (the arrangement's end has passed, or the bars asked for have
    /// played); from then on, the tail plays until everything is silent.
    /// Never waits or allocates.
    pub fn period(&mut self, mixer: &Mixer, frames: usize, ended: bool) -> Step {
        if let Some(s) = self.over {
            return s;
        }
        if !self.attached {
            return self.end(Step::Failed(Failure::Capture));
        }
        if self.shared.cancel.load(Ordering::Relaxed) {
            return self.end(Step::Cancelled);
        }
        for i in 0..self.points.len() {
            let slot = self.first_slot + i;
            if mixer.capture_latency(slot).unwrap_or(0) != self.lat[i] {
                return self.end(Step::Failed(Failure::Latency));
            }
            if let Some((l, r)) = mixer.capture(slot) {
                self.bin.push(i, l, r);
            }
        }
        self.played += frames as u64;
        self.shared.played.store(self.played, Ordering::Relaxed);
        let lost = self.bin.overruns();
        if lost > 0 {
            return self.end(Step::Failed(Failure::Overrun(lost)));
        }
        if ended && self.ended_at.is_none() {
            self.ended_at = Some(self.played);
            self.shared.ended_at.store(self.played, Ordering::Relaxed);
        }
        if let Some(at) = self.ended_at {
            let quiet = (0..self.points.len())
                .map(|s| self.bin.quiet(s))
                .min()
                .unwrap_or(usize::MAX);
            // The streams hear the range's last frames `most` late.
            let tail = self.played - at;
            if tail >= self.most && (quiet >= self.quiet_frames || tail >= self.max_tail) {
                return self.end(Step::Done);
            }
        }
        Step::Go
    }

    fn end(&mut self, s: Step) -> Step {
        self.bin.finish();
        self.over = Some(s);
        s
    }

    /// Stops the pass from the sound thread (STOP pressed while it plays,
    /// say): its next period says Cancelled, and the files are removed.
    pub fn cancel(&self) {
        self.shared.cancel.store(true, Ordering::Relaxed);
    }

    /// Frames every file's ring has room for now. A fast pass waits for a
    /// block's worth before each block ([`Pass::wait_for_room`]); a pass in
    /// real time never waits.
    pub fn room(&self) -> usize {
        self.bin.room()
    }

    /// Fast mode only, before a block: sleeps (a quarter of a millisecond
    /// at a time) until the rings have room for `frames`, or cancel is
    /// pressed. False if `most` passed first: the files have stalled.
    pub fn wait_for_room(&self, frames: usize, most: Duration) -> bool {
        let until = Instant::now() + most;
        loop {
            if self.room() >= frames || self.shared.cancel.load(Ordering::Relaxed) {
                return true;
            }
            if Instant::now() >= until {
                return false;
            }
            std::thread::sleep(Duration::from_micros(250));
        }
    }

    /// A message the player sent to an instrument, `tick` ticks (at
    /// `mpfree_midi::smf::PPQ`) after the range's start, on track `track`.
    /// Never allocates; past the room it is counted.
    pub fn note(&mut self, tick: u64, track: u16, msg: Msg) {
        if self.over.is_none() {
            if let Some(n) = self.notes.as_mut() {
                n.push(tick, track, msg);
            }
        }
    }

    /// Frees the export's capture points. Call it once [`Pass::period`]
    /// says the pass is over, then send the pass back to
    /// [`crate::Job::finish`].
    pub fn detach(&mut self, mixer: &mut Mixer) {
        if self.attached {
            for i in 0..self.points.len() {
                mixer.set_capture(self.first_slot + i, None);
            }
            self.attached = false;
        }
    }

    /// Frames played so far.
    pub fn played(&self) -> u64 {
        self.played
    }

    /// How the pass ended, once it has.
    pub fn over(&self) -> Option<Step> {
        self.over
    }
}
