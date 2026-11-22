//! What streaming and recording tell the musician: the layer's states
//! turned into System status reports (`mpfree-status`, its Storage kinds).
//!
//! The kinds and their words live in the catalogue
//! ([`mpfree_status::kinds`]); a [`Watch`] decides when each begins, ends or
//! happens, so whoever reads the layer says the same thing about the same
//! state. Feed it what the layer shows, as often as it is read (every
//! round, or every 250 ms):
//!
//! | Fed | Reports |
//! |---|---|
//! | [`Watch::take`]: a take's [`TakeView`] | Recording at risk (its state is At risk), Not saved yet (unsaved past twice the interval, not at risk), Recording may stop (capture memory nine tenths full while recording), Recording stopped (once, when it stops by itself) |
//! | [`Watch::take_gone`] | ends what the take had going |
//! | [`Watch::admission`]: [`Admission`] | Streaming at limit (not even the least safe read-ahead fits) |
//! | [`Watch::player`]: [`player::Stats`] | Stick fell behind (frames played as silence since last time) |
//! | [`Watch::card_lost`]: sources [`Coordinator::lost`] gave | Card removed |
//! | [`Watch::drive`]: the stick's [`Drive`] | Stick missing, Stick nearly full |
//! | [`Watch::found`]: recovery's [`Found`] | Take recovered |
//! | [`Watch::checked`]: a whole-take check's answer | Take damaged |
//!
//! Each report goes to a callback (in the app, `ToUi::Problem`), so a watch
//! allocates nothing once its list has grown to the problems going at
//! once. It does no I/O: the caller reads the stick's room (`statvfs`) off
//! the screen thread, because a failing stick can make that wait.
//!
//! [`Coordinator::lost`]: crate::coord::Coordinator::lost

use mpfree_status::kinds::{
    CARD_REMOVED, NOT_SAVED_YET, RECORDING_AT_RISK, RECORDING_MAY_STOP, RECORDING_STOPPED,
    STICK_FELL_BEHIND, STICK_MISSING, STICK_NEARLY_FULL, STREAMING_AT_LIMIT, TAKE_DAMAGED,
    TAKE_RECOVERED,
};
use mpfree_status::{Kind, Report, Subject};

use crate::coord::Admission;
use crate::player;
use crate::rec::TakeState;
use crate::recover::Found;
use crate::writer::TakeView;

/// Under this many bytes free the stick is nearly full.
pub const NEARLY_FULL_BYTES: u64 = 1_000_000_000;
/// Or under this many seconds of what is armed.
pub const NEARLY_FULL_S: f64 = 600.0;
/// Nearly full ends once the room is this much over the line, so room
/// hovering at the line doesn't come and go.
const NEARLY_FULL_ENDS: f64 = 1.1;

/// The stick's data partition as the caller found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drive {
    /// There, and not read-only (`statvfs` answered without `ST_RDONLY`).
    pub writable: bool,
    /// Bytes free for MPFree (`f_bavail` x `f_frsize`).
    pub free: u64,
    /// Bytes a second the armed takes would record (0 with none armed).
    pub armed_bytes_per_s: u64,
}

impl Drive {
    /// The room under which the stick is nearly full.
    pub fn line(&self) -> u64 {
        let armed = (self.armed_bytes_per_s as f64 * NEARLY_FULL_S) as u64;
        NEARLY_FULL_BYTES.max(armed)
    }
}

/// Turns the layer's states into reports, each change once.
#[derive(Default)]
pub struct Watch {
    /// Lasting problems going now, by kind and subject.
    going: Vec<(&'static Kind, Subject)>,
    /// Takes whose stop by itself was reported.
    stopped: Vec<Subject>,
    /// The player's missed frames when last fed.
    missed: Option<u64>,
}

impl Watch {
    pub fn new() -> Watch {
        Watch::default()
    }

    /// Whether `kind` is going about `subject`.
    pub fn is_going(&self, kind: &Kind, subject: Subject) -> bool {
        self.going.iter().any(|&(k, s)| k == kind && s == subject)
    }

    fn set(
        &mut self,
        kind: &'static Kind,
        subject: Subject,
        on: bool,
        out: &mut dyn FnMut(Report),
    ) {
        let at = self
            .going
            .iter()
            .position(|&(k, s)| k == kind && s == subject);
        match (on, at) {
            (true, None) => {
                self.going.push((kind, subject));
                out(Report::began(kind).about(subject));
            }
            (false, Some(i)) => {
                self.going.swap_remove(i);
                out(Report::ended(kind).about(subject));
            }
            _ => {}
        }
    }

    /// One take as its lane shows it, about `subject` (the caller's name
    /// for the take or its track).
    pub fn take(&mut self, subject: Subject, v: &TakeView, out: &mut dyn FnMut(Report)) {
        let at_risk = matches!(v.state, TakeState::AtRisk(_));
        // Still taking audio: a take at risk records on into memory.
        let recording = at_risk || v.state == TakeState::Recording;
        self.set(&RECORDING_AT_RISK, subject, at_risk, out);
        self.set(&NOT_SAVED_YET, subject, v.show_unsaved && !at_risk, out);
        self.set(
            &RECORDING_MAY_STOP,
            subject,
            v.nearly_full && recording,
            out,
        );
        let told = self.stopped.iter().position(|&s| s == subject);
        match (v.state, told) {
            (TakeState::Stopped(_), None) => {
                self.stopped.push(subject);
                out(Report::happened(&RECORDING_STOPPED, 1).about(subject));
            }
            // A new take under the same subject.
            (TakeState::Recording, Some(i)) => {
                self.stopped.swap_remove(i);
            }
            _ => {}
        }
    }

    /// The take is no longer shown: what it had going ends.
    pub fn take_gone(&mut self, subject: Subject, out: &mut dyn FnMut(Report)) {
        for kind in [&RECORDING_AT_RISK, &NOT_SAVED_YET, &RECORDING_MAY_STOP] {
            self.set(kind, subject, false, out);
        }
        self.stopped.retain(|&s| s != subject);
    }

    /// The coordinator's admission: not even the least safe read-ahead
    /// fits what plays.
    pub fn admission(&mut self, a: &Admission, out: &mut dyn FnMut(Report)) {
        self.set(&STREAMING_AT_LIMIT, Subject::NONE, !a.safe, out);
    }

    /// The player's counters: frames played as silence since last fed.
    /// The first feed only sets where counting starts.
    pub fn player(&mut self, s: &player::Stats, out: &mut dyn FnMut(Report)) {
        if let Some(was) = self.missed {
            if s.missed > was {
                out(Report::happened(&STICK_FELL_BEHIND, 1));
            }
        }
        self.missed = Some(s.missed);
    }

    /// Sources whose card went, as [`crate::coord::Coordinator::lost`]
    /// gave them since last fed: one report for the lot.
    pub fn card_lost(&mut self, sources: usize, out: &mut dyn FnMut(Report)) {
        if sources > 0 {
            out(Report::happened(&CARD_REMOVED, 1));
        }
    }

    /// The stick's data partition.
    pub fn drive(&mut self, d: &Drive, out: &mut dyn FnMut(Report)) {
        self.set(&STICK_MISSING, Subject::NONE, !d.writable, out);
        let line = d.line();
        let full = if self.is_going(&STICK_NEARLY_FULL, Subject::NONE) {
            (d.free as f64) < line as f64 * NEARLY_FULL_ENDS
        } else {
            d.free < line
        };
        self.set(&STICK_NEARLY_FULL, Subject::NONE, d.writable && full, out);
    }

    /// What recovery found for one take: a take it finished after a cut.
    pub fn found(&mut self, f: &Found, out: &mut dyn FnMut(Report)) {
        if matches!(f, Found::Finished { .. } | Found::CutOff { .. }) {
            out(Report::happened(&TAKE_RECOVERED, 1));
        }
    }

    /// A whole-take check's answer for the take `subject`: a mismatch
    /// stays until the musician has seen it.
    pub fn checked(&mut self, subject: Subject, matched: bool, out: &mut dyn FnMut(Report)) {
        if !matched {
            out(Report::began(&TAKE_DAMAGED).about(subject));
        }
    }
}
