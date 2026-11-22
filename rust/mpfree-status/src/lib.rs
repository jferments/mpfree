//! System status for MPFree: what a musician needs to know is wrong, in one
//! place (the System status plan, approved 22 Nov 2026; design page
//! `docs/dev/design/status.md`).
//!
//! Every engine and thread reports problems here instead of growing a chip
//! of its own. The pieces:
//!
//! - [`Kind`]: one kind of problem, written once in the catalogue
//!   ([`kinds`]): its [`System`], [`Level`] and [`Shape`], short words for
//!   its system's line, a title, what happened, what to try, an optional
//!   [`Action`] and the effects it hides when it is their cause. A new kind
//!   joins by adding an entry, never by changing the screen.
//! - [`Report`]: a kind, a [`Subject`] (which port, track, take or plugin)
//!   and an [`Op`] (it happened, it began, it ended). A report is `Copy` and
//!   holds no heap memory, so any thread can send one through the screen
//!   channel without allocating.
//! - [`Board`]: lives on the screen thread and turns reports into one
//!   status colour ([`ChipView`]), the nine systems ([`SystemView`]) and
//!   their problems ([`ProblemView`]). It latches a problem until it is seen,
//!   waits 1 s before showing a lasting problem and keeps it 3 s after it
//!   stops (no flicker), keeps one entry per kind and subject with counts,
//!   lets a cause hide its effects, and allocates nothing once made. It
//!   takes the time as an argument, so tests drive the clock.
//!
//! The audio thread never reports: watchers on the screen thread read its
//! counters (phase 1), so its rules (no allocation, no locks) stay as they
//! are. This crate is std only, with no I/O and no drawing.

mod board;
pub mod kinds;

pub use board::{
    Board, Change, ChipLook, ChipView, EventView, ProblemState, ProblemView, SystemView, Window,
    CHANGES, EVENTS, NOW_FOR, OPEN, RAM_BUSY_PCT, RAM_FULL_PCT, SETTLE_OFF, SETTLE_ON,
};

/// The nine parts of the instrument a problem belongs to, in the order the
/// System status page lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum System {
    Sound,
    Memory,
    Storage,
    /// The pads, buttons, knobs and touchscreen.
    Controls,
    Midi,
    Project,
    Screen,
    Plugins,
    /// The MPC itself: its cores, temperature and processor clock.
    Mpc,
}

impl System {
    pub const ALL: [System; 9] = [
        System::Sound,
        System::Memory,
        System::Storage,
        System::Controls,
        System::Midi,
        System::Project,
        System::Screen,
        System::Plugins,
        System::Mpc,
    ];

    /// The system's name on the page.
    pub fn name(self) -> &'static str {
        match self {
            System::Sound => "Sound",
            System::Memory => "Memory",
            System::Storage => "Storage",
            System::Controls => "Pads and buttons",
            System::Midi => "MIDI and sync",
            System::Project => "Project",
            System::Screen => "Screen",
            System::Plugins => "Plugins",
            System::Mpc => "The MPC",
        }
    }

    /// How a kind's id starts for this system ("sound/dropout").
    pub fn id_prefix(self) -> &'static str {
        match self {
            System::Sound => "sound/",
            System::Memory => "memory/",
            System::Storage => "storage/",
            System::Controls => "controls/",
            System::Midi => "midi/",
            System::Project => "project/",
            System::Screen => "screen/",
            System::Plugins => "plugins/",
            System::Mpc => "mpc/",
        }
    }

    /// Its place in [`System::ALL`].
    pub fn index(self) -> usize {
        self as usize
    }
}

/// How serious a problem is. "All good" is no problem at all.
///
/// - **Fail** (red): you heard a fault, work may be lost, or MPFree isn't
///   answering you.
/// - **Warn** (yellow): MPFree gave something up to keep playing, is near a
///   limit, or something you use isn't working, and nothing is lost.
/// - **Note**: worth listing for the session ("Take recovered"), but it
///   never colours anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Level {
    Note,
    Warn,
    Fail,
}

impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Level::Note => "note",
            Level::Warn => "warn",
            Level::Fail => "fail",
        }
    }
}

/// How a problem lives in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// A moment, counted, with when it last happened (the sound dropped
    /// out). It counts as happening now for [`NOW_FOR`] after the last time.
    Happens,
    /// A state with a start and an end (the sound near its limit). It shows
    /// once it has lasted [`SETTLE_ON`] and goes [`SETTLE_OFF`] after it
    /// stops, so a state that comes and goes doesn't flicker.
    Lasts,
    /// Lasts until its cause is fixed or the project changes (a MIDI port
    /// missing, the project opened with parts left out). It shows at once
    /// and ends when it is reported ended; a kind nobody ends stays until
    /// the next start.
    Stays,
    /// Stays until the musician has seen it (a take found damaged).
    UntilSeen,
}

/// What a problem's button does, where MPFree can do it for you. The
/// screen thread carries it out; a button only ever does what you tap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Silence every note that is playing (a note may be stuck on).
    SilenceNotes,
    /// Show what a project opened without.
    ShowMissing,
    /// Open the last project (after a safe start).
    OpenLastProject,
}

impl Action {
    /// The button's words.
    pub fn label(self) -> &'static str {
        match self {
            Action::SilenceNotes => "Silence all notes",
            Action::ShowMissing => "Show what's missing",
            Action::OpenLastProject => "Open the last project",
        }
    }
}

/// One kind of problem: an entry of the catalogue ([`kinds`]).
///
/// Kinds are `static`s and are compared by [`Kind::id`].
#[derive(Debug)]
pub struct Kind {
    /// "sound/dropout": the system's [`System::id_prefix`] and a slug. It
    /// names the kind in log lines and tests.
    pub id: &'static str,
    pub system: System,
    pub level: Level,
    pub shape: Shape,
    /// Short words for its system's line on the page ("Sound dropped
    /// out"), [`SHORT_MAX`] characters at most.
    pub short: &'static str,
    /// The card's title, a plain sentence ("The sound dropped out"). A red
    /// problem's title also goes on the hint line when it starts (phase 2).
    pub title: &'static str,
    /// What happened and what it means for the music.
    pub what: &'static str,
    /// What to try.
    pub to_try: &'static str,
    pub action: Option<Action>,
    /// The anchor of its entry in the manual's "When something goes wrong"
    /// (phase 2 writes the chapter).
    pub manual: &'static str,
    /// The effects this kind causes, hidden while it is going so the page
    /// names the cause, not its echoes (no session hides the dropouts).
    pub hides: &'static [&'static Kind],
}

/// The most characters a kind's [`Kind::short`] words may have.
pub const SHORT_MAX: usize = 18;

impl PartialEq for Kind {
    fn eq(&self, other: &Kind) -> bool {
        self.id == other.id
    }
}

impl Eq for Kind {}

impl Kind {
    /// Whether a problem of this kind is hidden while `cause` is going.
    pub fn hidden_by(&self, cause: &Kind) -> bool {
        cause.hides.contains(&self)
    }
}

/// Which port, track, take or plugin a problem is about; [`Subject::NONE`]
/// when it is about the whole system. Its name ("Juno-X", "Vocal take") is
/// looked up on the screen thread when the page draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Subject(pub u32);

impl Subject {
    pub const NONE: Subject = Subject(0);
}

/// What a report says happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    /// A moment, `times` of them since the last report (a watcher compares
    /// counters every 250 ms, so several can arrive at once).
    Happened { times: u32 },
    /// A lasting or staying problem began.
    Began,
    /// It ended.
    Ended,
}

/// One report: a kind, what it is about and what happened. `Copy`, with no
/// heap memory, so it can go through the screen channel from any thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub kind: &'static Kind,
    pub subject: Subject,
    pub op: Op,
}

impl Report {
    /// `kind` happened `times` times, about the whole system.
    pub fn happened(kind: &'static Kind, times: u32) -> Report {
        Report {
            kind,
            subject: Subject::NONE,
            op: Op::Happened { times },
        }
    }

    /// `kind` began, about the whole system.
    pub fn began(kind: &'static Kind) -> Report {
        Report {
            kind,
            subject: Subject::NONE,
            op: Op::Began,
        }
    }

    /// `kind` ended, about the whole system.
    pub fn ended(kind: &'static Kind) -> Report {
        Report {
            kind,
            subject: Subject::NONE,
            op: Op::Ended,
        }
    }

    /// The same report about `subject`.
    pub fn about(mut self, subject: Subject) -> Report {
        self.subject = subject;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_small_and_copy() {
        fn copy<T: Copy + Send + 'static>() {}
        copy::<Report>();
        assert!(
            std::mem::size_of::<Report>() <= 24,
            "fits the screen channel cheaply"
        );
    }

    #[test]
    fn systems_are_in_page_order_and_index_themselves() {
        for (i, s) in System::ALL.iter().enumerate() {
            assert_eq!(s.index(), i);
        }
        assert_eq!(System::Controls.name(), "Pads and buttons");
    }

    #[test]
    fn levels_order_by_how_serious() {
        assert!(Level::Fail > Level::Warn && Level::Warn > Level::Note);
    }

    #[test]
    fn a_cause_hides_only_its_listed_effects() {
        assert!(kinds::SOUND_DROPOUT.hidden_by(&kinds::NO_SESSION));
        assert!(!kinds::NO_SESSION.hidden_by(&kinds::SOUND_DROPOUT));
        assert!(!kinds::SAVE_FAILED.hidden_by(&kinds::NO_SESSION));
    }
}
