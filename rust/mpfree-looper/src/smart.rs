//! Smart loop end's findings, the snap it offers and lands, and the
//! first pass waiting for its decision.

use super::*;

/// Smart loop end ([`Command::SetSmartEnd`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SmartEnd {
    /// The loop closes on your tap.
    #[default]
    Off,
    /// It closes on your tap, then offers the end it found.
    Suggest,
    /// It listens on to the end of the reach, then closes on the end it
    /// found.
    Auto,
}

/// What smart loop end made of a free first pass's closing tap, once
/// ([`Looper::take_smart`]), for the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmartNote {
    pub n: usize,
    pub outcome: SmartOutcome,
    /// A smart start: frames from your press to your first hit, where
    /// Auto started the loop or Suggest offers to ([`Shift::start`]). 0
    /// when the start stays on your press.
    pub start: i64,
    /// The closing tap's frame (the song's frame clock).
    pub tap: u64,
    /// The best end and the runner-up (the tap, when it was), scored.
    pub best: Option<Candidate>,
    pub second: Option<Candidate>,
    /// How sure it was of the end, the bars, the tempo and the start
    /// ([`close`]), with the reading the loop has now. None when it
    /// decided nothing (an overdub started on the tap).
    pub sure: Option<Confidence>,
}

/// What became of the tap, in frames from it (negative: the end is
/// earlier, the tap was late).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmartOutcome {
    /// Auto moved the end.
    Moved(i64),
    /// Suggest offers moving it ([`Status::offer`]).
    Offered(i64),
    /// The end found is within [`OFFER_MS`] of the tap: nothing worth
    /// offering.
    Close(i64),
    /// The tap stood, and why.
    Kept(&'static str),
}

/// Why Suggest kept a tap when Then: Overdub started the overdub on it
/// ([`SmartOutcome::Kept`]): Suggest can't wait for you there.
pub const OVERDUB_AT_ONCE: &str = "the overdub started at once";

/// Why Suggest kept a tap, or a snap was refused, when the end it would
/// move to is later than the loop kept because loop memory was full as
/// the pass closed ([`Bounds::short`]).
pub const MEMORY_FULL: &str = "loop memory was full, so nothing after it was kept";

/// The smallest move Suggest offers, in milliseconds: anything nearer the
/// tap can't be heard.
pub const OFFER_MS: f64 = 5.0;

/// By ear, how long Auto waits past its reach for the finder's answer
/// (ms) before it keeps your tap; the loop and the song start that far
/// in, so nothing is heard late.
pub const FIND_WAIT_AUTO_MS: f64 = 30.0;

/// How long Suggest waits for it.
pub const FIND_WAIT_SUGGEST_MS: f64 = 250.0;

/// `ms` milliseconds in frames at `rate`.
pub(crate) fn ms_frames(rate: u32, ms: f64) -> u64 {
    (ms * f64::from(rate) / 1000.0).round() as u64
}

/// Smart loop end by ear: a decision asked of the finder for loop `k`'s
/// take `gen` ([`Looper::find_job`], [`Looper::found`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Finding {
    pub(crate) k: usize,
    pub(crate) gen: u64,
    pub(crate) ask: ends::Ask,
    pub(crate) bars: u32,
    /// The frame by which it must have answered, else the tap stands.
    pub(crate) due: u64,
    /// Handed to the host for the finder.
    pub(crate) sent: bool,
    pub(crate) answer: Option<ends::Decision>,
}

/// How far a free first loop's start and end are, or move, from your
/// press and your closing tap, in frames
/// ([`Status::offer`], [`Status::moved`], [`Step::Snap`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shift {
    /// The loop's start, later (above 0): a smart start puts it on your
    /// first hit.
    pub start: i64,
    /// Its end, later (above 0; negative: earlier, the tap was late).
    pub end: i64,
}

impl Shift {
    pub const NONE: Shift = Shift { start: 0, end: 0 };

    /// How much longer the loop is (the tempo changes with it).
    pub fn longer(&self) -> i64 {
        self.end - self.start
    }

    pub fn is_none(&self) -> bool {
        *self == Shift::NONE
    }
}

impl std::ops::Add for Shift {
    type Output = Shift;
    fn add(self, o: Shift) -> Shift {
        Shift {
            start: self.start + o.start,
            end: self.end + o.end,
        }
    }
}

impl std::ops::Sub for Shift {
    type Output = Shift;
    fn sub(self, o: Shift) -> Shift {
        Shift {
            start: self.start - o.start,
            end: self.end - o.end,
        }
    }
}

impl std::ops::Neg for Shift {
    type Output = Shift;
    fn neg(self) -> Shift {
        Shift {
            start: -self.start,
            end: -self.end,
        }
    }
}

/// A change of a free first loop's start or end made with the song's
/// tempo, landing on the loop's downbeat ([`Looper::snap_plan`],
/// [`Looper::snap_at`]): the loop's length and the tempo change together,
/// so its bars stay and it plays as recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// Take what smart loop end found ([`Status::offer`]): the end, and
    /// the start on your first hit (a smart start).
    Found,
    /// Put the end back on your closing tap ([`Status::moved`]): Use my
    /// tap.
    Tap,
    /// Put the start back on your press: Use my press.
    Press,
    /// Read as: read the loop as this many bars of the song's meter. Its
    /// length and its sound stay; the tempo changes so they fit it.
    Bars(u32),
    /// Move start / Move end: put the start and the end here, in frames
    /// from your press and your closing tap (as [`Status::offer`] and
    /// [`Status::moved`] count them), within what the first pass kept.
    /// It lands as a snap does: the bars stay and the tempo follows.
    To(Shift),
    /// Undo the last step, a snap ([`Step::Snap`], [`Step::Bars`]).
    Undo,
    /// Redo the next step, a snap.
    Redo,
}

/// A snap worked out ([`Looper::snap_plan`]): the host asks the time engine
/// for the tempo on `at`, and once that is committed, gives the plan back
/// ([`Looper::snap_at`]) so the loop changes on the same frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapPlan {
    pub n: usize,
    pub snap: Snap,
    /// How far the start and the end move.
    pub by: Shift,
    /// The loop's length in song ticks once it lands: Read as changes it.
    pub ticks: Tick,
    /// The tempo at which the loop's bars are its new length exactly.
    pub bpm: f64,
    /// The loop's next downbeat, as a song tick and a frame.
    pub tick: Tick,
    pub at: u64,
    /// The loop's version it was worked out at ([`Status::version`]): a
    /// loop changed since doesn't take it.
    pub version: u64,
    /// The loop's clock as it was worked out on ([`ClockStamp`]): a free
    /// loop's snap doesn't land once the free clock has started again or
    /// changed since, as its downbeat's frame is no downbeat of it then.
    pub stamp: ClockStamp,
}

/// What became of a snap the host gave ([`Looper::take_snap`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapDone {
    pub plan: SnapPlan,
    /// It landed on its frame; false: the loop had changed, and nothing
    /// of the loop moved.
    pub landed: bool,
}

/// A closed free first pass waiting for smart loop end's decision.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SmartWait {
    pub(crate) tap: u64,
    pub(crate) reach: u64,
    pub(crate) start: u64,
    pub(crate) bar: Tick,
    pub(crate) beat: Tick,
    /// How the tap's length read: the bars an end found keeps.
    pub(crate) reading: Reading,
}

/// `like`'s bars over `len` frames instead: the reading of an end smart
/// loop end found, a few milliseconds from the tap, keeps the tap's bar
/// count (None when that tempo is out of the app's range).
pub(crate) fn reread(len: usize, rate: u32, bar: Tick, like: &Reading) -> Option<Reading> {
    let bpm = reading::tempo_of(len, rate, bar, like.bars);
    (bpm.is_finite() && (LOOP_MIN_BPM..=LOOP_MAX_BPM).contains(&bpm))
        .then_some(Reading { bpm, ..*like })
}
