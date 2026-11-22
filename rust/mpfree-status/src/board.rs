//! The board: the screen thread's record of every problem this session.
//!
//! It keeps one entry per kind and subject (up to [`OPEN`]) and the last
//! [`EVENTS`] reports, counting any older ones, in memory it takes when it
//! is made, so applying a report never allocates. Views ([`Board::chip`],
//! [`Board::systems`], [`Board::problems`]) are what the status chip and the
//! System status page draw; [`Board::raw`] and [`Board::events`] feed the
//! Dev > Status board page.
//!
//! The rules it keeps (the System status plan's chip rules):
//!
//! - **It latches until you look.** A problem that has ended keeps its
//!   colour, as a ring, until [`Board::seen`] (the page opened), so a 3 ms
//!   dropout mid-set is still there when you glance up.
//! - **It stays while the problem lasts,** seen or not.
//! - **No flicker.** A lasting problem shows once it has lasted
//!   [`SETTLE_ON`] and goes [`SETTLE_OFF`] after it stops; one that comes
//!   back inside that time is the same spell. A moment counts as happening
//!   now for [`NOW_FOR`] after it last happened.
//! - **A cause hides its effects** ([`Kind::hides`]): an effect reported
//!   while its cause is going is kept but not shown.
//! - **Worst first:** red before yellow, then one happening now before one
//!   that ended, then the most recent.

use crate::{Kind, Level, Op, Report, Shape, Subject, System};
use std::time::{Duration, Instant};

/// How many problems (kind and subject) the board holds at once.
pub const OPEN: usize = 64;
/// How many reports it keeps for the Dev page.
pub const EVENTS: usize = 256;
/// A lasting problem shows once it has lasted this long.
pub const SETTLE_ON: Duration = Duration::from_secs(1);
/// And goes this long after it stops.
pub const SETTLE_OFF: Duration = Duration::from_secs(3);
/// A moment counts as happening now this long after it last happened.
pub const NOW_FOR: Duration = Duration::from_secs(3);

/// Which problems a list shows: only those happening now, or everything
/// since MPFree started (the page's Now | This session switch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Window {
    Now,
    Session,
}

/// Where a problem is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemState {
    /// A lasting problem that hasn't lasted [`SETTLE_ON`] yet (this spell):
    /// not shown as happening.
    Settling,
    /// Happening now.
    Now,
    /// Over.
    Ended,
}

/// One problem as the screen shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProblemView {
    pub kind: &'static Kind,
    pub subject: Subject,
    pub state: ProblemState,
    /// How many times it happened (a moment) or how many spells were shown
    /// (lasting and staying).
    pub times: u64,
    /// When it was first reported this session.
    pub first: Instant,
    /// When it last happened, or was last going.
    pub last: Instant,
    /// When it ended, once over.
    pub ended: Option<Instant>,
    /// Shown at least once (a lasting problem only after [`SETTLE_ON`]).
    pub shown: bool,
    /// Looked at since it last happened.
    pub seen: bool,
    /// Its last report came while a cause that hides it was going.
    pub hidden: bool,
}

impl ProblemView {
    pub fn is_now(&self) -> bool {
        self.state == ProblemState::Now
    }

    /// Whether the chip and the page show it: shown, not hidden.
    pub fn listed(&self) -> bool {
        self.shown && !self.hidden
    }
}

/// The status chip's colour from the problems: `None` is all good. `ring`
/// when the worst comes only from problems that ended and haven't been
/// seen. The CPU and RAM readings join this on the screen thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChipView {
    pub level: Option<Level>,
    pub ring: bool,
    /// Problems happening now (yellow or red).
    pub now: usize,
    /// Problems that ended and haven't been seen.
    pub unseen: usize,
}

/// One system's line on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemView {
    pub system: System,
    /// Its worst problem now or not yet seen; `None` when it works.
    pub level: Option<Level>,
    pub ring: bool,
    /// Problems happening now.
    pub now: usize,
    /// Problems that ended this session.
    pub earlier: usize,
}

/// One report as it arrived, for the Dev page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventView {
    pub kind: &'static Kind,
    pub subject: Subject,
    pub op: Op,
    pub at: Instant,
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    kind: &'static Kind,
    subject: Subject,
    first: Instant,
    last: Instant,
    /// Lasting and staying: when the current spell began, while it goes.
    going_since: Option<Instant>,
    /// Lasting: when the current spell stopped (it shows for SETTLE_OFF).
    stopped_at: Option<Instant>,
    ended: Option<Instant>,
    times: u64,
    shown: bool,
    state: ProblemState,
    seen: bool,
    hidden: bool,
}

impl Entry {
    /// Going now: a moment within NOW_FOR, a state that began and hasn't
    /// been reported ended (a lasting one settling counts: it hides its
    /// effects from the start).
    fn going(&self) -> bool {
        match self.kind.shape {
            Shape::Happens => self.state == ProblemState::Now,
            Shape::Lasts => self.going_since.is_some() && self.stopped_at.is_none(),
            Shape::Stays | Shape::UntilSeen => self.state == ProblemState::Now,
        }
    }

    fn view(&self) -> ProblemView {
        ProblemView {
            kind: self.kind,
            subject: self.subject,
            state: self.state,
            times: self.times,
            first: self.first,
            last: self.last,
            ended: self.ended,
            shown: self.shown,
            seen: self.seen,
            hidden: self.hidden,
        }
    }
}

/// See the module's page.
pub struct Board {
    open: Vec<Entry>,
    /// A ring once full: `next` is the oldest.
    events: Vec<EventView>,
    next: usize,
    older: u64,
    lost: u64,
}

impl Default for Board {
    fn default() -> Board {
        Board::new()
    }
}

impl Board {
    /// An empty board, with all the memory it will use.
    pub fn new() -> Board {
        Board {
            open: Vec::with_capacity(OPEN),
            events: Vec::with_capacity(EVENTS),
            next: 0,
            older: 0,
            lost: 0,
        }
    }

    /// Takes one report at `now`.
    pub fn apply(&mut self, r: Report, now: Instant) {
        self.record(r, now);
        let i = match self.find(r.kind, r.subject) {
            Some(i) => i,
            None if r.op == Op::Ended => return,
            None => {
                if self.open.len() == OPEN && !self.evict() {
                    self.lost += 1;
                    return;
                }
                self.open.push(Entry {
                    kind: r.kind,
                    subject: r.subject,
                    first: now,
                    last: now,
                    going_since: None,
                    stopped_at: None,
                    ended: None,
                    times: 0,
                    shown: false,
                    state: ProblemState::Ended,
                    seen: true,
                    hidden: false,
                });
                self.open.len() - 1
            }
        };
        let hidden = self.hidden_now(r.kind, i);
        let e = &mut self.open[i];
        match r.op {
            Op::Happened { times } => {
                e.times += u64::from(times);
                e.last = now;
                e.ended = None;
                e.shown = true;
                e.state = ProblemState::Now;
                e.seen = false;
                e.hidden = hidden;
            }
            // Already going: nothing new.
            Op::Began if e.kind.shape == Shape::Lasts && e.going() => {}
            Op::Began if e.kind.shape == Shape::Lasts => {
                if e.state == ProblemState::Now {
                    // Back inside SETTLE_OFF: the same spell.
                    e.stopped_at = None;
                    e.last = now;
                } else {
                    e.going_since = Some(now);
                    e.stopped_at = None;
                    e.last = now;
                    e.state = ProblemState::Settling;
                    e.hidden = hidden;
                }
            }
            Op::Began => {
                if e.state != ProblemState::Now {
                    e.times += 1;
                    e.going_since = Some(now);
                    e.stopped_at = None;
                    e.ended = None;
                    e.shown = true;
                    e.state = ProblemState::Now;
                    e.seen = false;
                    e.hidden = hidden;
                }
                e.last = now;
            }
            Op::Ended => match e.kind.shape {
                Shape::Lasts if e.going() && e.state == ProblemState::Now => {
                    e.stopped_at = Some(now);
                    e.last = now;
                }
                Shape::Lasts if e.going() => {
                    // It stopped before it settled: this spell never
                    // showed, and a problem that never showed is no
                    // problem.
                    e.going_since = None;
                    e.state = ProblemState::Ended;
                    if !e.shown {
                        self.open.swap_remove(i);
                    }
                }
                Shape::Stays | Shape::UntilSeen if e.state == ProblemState::Now => {
                    e.going_since = None;
                    e.state = ProblemState::Ended;
                    e.ended = Some(now);
                    e.last = now;
                }
                Shape::Happens | Shape::Lasts | Shape::Stays | Shape::UntilSeen => {}
            },
        }
    }

    /// Moves time on to `now`: lasting problems that have settled show,
    /// those stopped for [`SETTLE_OFF`] end, moments older than
    /// [`NOW_FOR`] end. Call it every few hundred ms.
    pub fn tick(&mut self, now: Instant) {
        for i in 0..self.open.len() {
            let settled = {
                let e = &self.open[i];
                e.kind.shape == Shape::Lasts
                    && e.state == ProblemState::Settling
                    && e.going_since
                        .is_some_and(|t| now.duration_since(t) >= SETTLE_ON)
            };
            let hidden = settled && self.hidden_now(self.open[i].kind, i);
            let e = &mut self.open[i];
            if settled {
                if !e.shown {
                    e.first = e.going_since.unwrap_or(now);
                }
                e.shown = true;
                e.times += 1;
                e.ended = None;
                e.state = ProblemState::Now;
                e.seen = false;
                e.hidden = e.hidden || hidden;
            }
            if e.state != ProblemState::Now {
                continue;
            }
            match e.kind.shape {
                Shape::Happens => {
                    if now.duration_since(e.last) >= NOW_FOR {
                        e.state = ProblemState::Ended;
                        e.ended = Some(e.last);
                    }
                }
                Shape::Lasts => match e.stopped_at {
                    Some(t) if now.duration_since(t) >= SETTLE_OFF => {
                        e.state = ProblemState::Ended;
                        e.ended = Some(t);
                        e.going_since = None;
                        e.stopped_at = None;
                    }
                    Some(_) => {}
                    None => e.last = now,
                },
                Shape::Stays | Shape::UntilSeen => e.last = now,
            }
        }
    }

    /// The musician looked (the System status page opened): every problem
    /// is seen, so ended ones stop colouring the chip, and problems that
    /// stay until seen end.
    pub fn seen(&mut self, now: Instant) {
        for e in &mut self.open {
            e.seen = true;
            if e.kind.shape == Shape::UntilSeen && e.state == ProblemState::Now {
                e.state = ProblemState::Ended;
                e.ended = Some(now);
                e.going_since = None;
            }
        }
    }

    /// The status colour from the problems (see [`ChipView`]).
    pub fn chip(&self) -> ChipView {
        Board::colour(self.open.iter())
    }

    /// The nine systems, in page order.
    pub fn systems(&self) -> [SystemView; 9] {
        System::ALL.map(|s| {
            let c = Board::colour(self.open.iter().filter(|e| e.kind.system == s));
            let earlier = self
                .open
                .iter()
                .filter(|e| {
                    e.kind.system == s
                        && e.kind.level > Level::Note
                        && e.view().listed()
                        && e.state != ProblemState::Now
                })
                .count();
            SystemView {
                system: s,
                level: c.level,
                ring: c.ring,
                now: c.now,
                earlier,
            }
        })
    }

    /// A system's problems, worst first: those happening now, or all this
    /// session (notes included). Hidden effects and lasting problems that
    /// haven't settled are left out.
    pub fn problems(&self, system: System, window: Window) -> Vec<ProblemView> {
        let mut out: Vec<ProblemView> = self
            .open
            .iter()
            .map(Entry::view)
            .filter(|p| {
                p.kind.system == system && p.listed() && (window == Window::Session || p.is_now())
            })
            .collect();
        out.sort_by(|a, b| {
            b.kind
                .level
                .cmp(&a.kind.level)
                .then(b.is_now().cmp(&a.is_now()))
                .then(b.last.cmp(&a.last))
        });
        out
    }

    /// Every entry as it stands, hidden and settling ones too, by system
    /// and then worst first: the Dev page's raw list.
    pub fn raw(&self) -> Vec<ProblemView> {
        let mut out: Vec<ProblemView> = self.open.iter().map(Entry::view).collect();
        out.sort_by(|a, b| {
            a.kind
                .system
                .cmp(&b.kind.system)
                .then(b.kind.level.cmp(&a.kind.level))
                .then(b.is_now().cmp(&a.is_now()))
                .then(b.last.cmp(&a.last))
        });
        out
    }

    /// The reports kept, newest first.
    pub fn events(&self) -> impl Iterator<Item = &EventView> {
        let (new, old) = self.events.split_at(self.next);
        new.iter().rev().chain(old.iter().rev())
    }

    /// Reports older than the [`EVENTS`] kept.
    pub fn older(&self) -> u64 {
        self.older
    }

    /// Reports for a new problem that found the board full of problems
    /// still happening or not yet seen (so none could make room).
    pub fn lost(&self) -> u64 {
        self.lost
    }

    /// The kinds shown this session (yellow or red) that aren't in
    /// `allowed`: tests and the bench fail on any.
    pub fn unexpected(&self, allowed: &[&Kind]) -> Vec<&'static Kind> {
        let mut out: Vec<&'static Kind> = Vec::new();
        for e in &self.open {
            if e.shown
                && e.kind.level > Level::Note
                && !allowed.contains(&e.kind)
                && !out.contains(&e.kind)
            {
                out.push(e.kind);
            }
        }
        out
    }

    /// How many problems it holds.
    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    fn find(&self, kind: &Kind, subject: Subject) -> Option<usize> {
        self.open
            .iter()
            .position(|e| e.kind == kind && e.subject == subject)
    }

    /// Whether a cause of `kind` (other than entry `skip`) is going.
    fn hidden_now(&self, kind: &Kind, skip: usize) -> bool {
        self.open
            .iter()
            .enumerate()
            .any(|(j, e)| j != skip && e.going() && kind.hidden_by(e.kind))
    }

    fn record(&mut self, r: Report, now: Instant) {
        let ev = EventView {
            kind: r.kind,
            subject: r.subject,
            op: r.op,
            at: now,
        };
        if self.events.len() < EVENTS {
            self.events.push(ev);
            self.next = self.events.len() % EVENTS;
        } else {
            self.events[self.next] = ev;
            self.next = (self.next + 1) % EVENTS;
            self.older += 1;
        }
    }

    /// Makes room for a new problem: the oldest that ended and was seen,
    /// else the oldest that ended. False when every problem is happening
    /// now or settling.
    fn evict(&mut self) -> bool {
        let pick = |seen_only: bool| {
            self.open
                .iter()
                .enumerate()
                .filter(|(_, e)| e.state == ProblemState::Ended && (e.seen || !seen_only))
                .min_by_key(|(_, e)| e.last)
                .map(|(i, _)| i)
        };
        match pick(true).or_else(|| pick(false)) {
            Some(i) => {
                self.open.swap_remove(i);
                true
            }
            None => false,
        }
    }

    fn colour<'a>(entries: impl Iterator<Item = &'a Entry>) -> ChipView {
        let mut c = ChipView::default();
        let mut filled: Option<Level> = None;
        let mut ringed: Option<Level> = None;
        for e in entries {
            let p = e.view();
            if !p.listed() || p.kind.level == Level::Note {
                continue;
            }
            if p.is_now() {
                c.now += 1;
                filled = filled.max(Some(p.kind.level));
            } else if !p.seen {
                c.unseen += 1;
                ringed = ringed.max(Some(p.kind.level));
            }
        }
        c.level = filled.max(ringed);
        c.ring = c.level.is_some() && filled < c.level;
        c
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::*;
    use crate::{Action, Report};

    static TAKE_RECOVERED: Kind = Kind {
        id: "storage/take-recovered",
        system: System::Storage,
        level: Level::Note,
        shape: Shape::Happens,
        short: "Take recovered",
        title: "A take was recovered",
        what: "A take cut off by a power cut was finished.",
        to_try: "Nothing: it is in the project.",
        action: None,
        manual: "a-take-was-recovered",
        hides: &[],
    };

    static TAKE_DAMAGED: Kind = Kind {
        id: "storage/take-damaged",
        system: System::Storage,
        level: Level::Warn,
        shape: Shape::UntilSeen,
        short: "Take damaged",
        title: "A take is damaged",
        what: "The check of a take found damage; the audio is kept.",
        to_try: "Listen to it before you rely on it.",
        action: Some(Action::ShowMissing),
        manual: "a-take-is-damaged",
        hides: &[],
    };

    fn at(t0: Instant, s: f32) -> Instant {
        t0 + Duration::from_secs_f32(s)
    }

    fn go(b: &mut Board, t0: Instant, s: f32) {
        b.tick(at(t0, s));
    }

    #[test]
    fn a_moment_shows_at_once_then_rings_until_seen() {
        let t0 = Instant::now();
        let mut b = Board::new();
        assert_eq!(b.chip(), ChipView::default(), "all good");
        b.apply(Report::happened(&SOUND_DROPOUT, 1), t0);
        let c = b.chip();
        assert_eq!((c.level, c.ring, c.now), (Some(Level::Fail), false, 1));
        go(&mut b, t0, 2.9);
        assert!(b.chip().now == 1, "still now within NOW_FOR");
        go(&mut b, t0, 3.1);
        let c = b.chip();
        assert_eq!(
            (c.level, c.ring, c.now, c.unseen),
            (Some(Level::Fail), true, 0, 1),
            "ended, not seen: a red ring"
        );
        b.seen(at(t0, 10.0));
        assert_eq!(b.chip(), ChipView::default(), "seen: all good again");
        let p = b.problems(System::Sound, Window::Session);
        assert_eq!(p.len(), 1, "still listed for the session");
        assert_eq!(p[0].ended, Some(t0));
        assert!(b.problems(System::Sound, Window::Now).is_empty());
    }

    #[test]
    fn moments_count_into_one_card() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::happened(&SOUND_DROPOUT, 2), t0);
        b.apply(Report::happened(&SOUND_DROPOUT, 1), at(t0, 5.0));
        let p = b.problems(System::Sound, Window::Session);
        assert_eq!(p.len(), 1);
        assert_eq!((p[0].times, p[0].first, p[0].last), (3, t0, at(t0, 5.0)));
        assert!(p[0].is_now(), "it happened again: now again, unseen");
    }

    #[test]
    fn a_short_state_never_shows_and_leaves_nothing() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::began(&NEAR_LIMIT), t0);
        go(&mut b, t0, 0.5);
        assert_eq!(b.chip(), ChipView::default(), "settling: not shown");
        b.apply(Report::ended(&NEAR_LIMIT), at(t0, 0.9));
        go(&mut b, t0, 2.0);
        assert_eq!(b.chip(), ChipView::default());
        assert!(b.is_empty(), "a state under SETTLE_ON is no problem");
        assert_eq!(b.events().count(), 2, "but its reports are on the Dev page");
    }

    #[test]
    fn a_state_shows_after_a_second_and_stays_three_after_it_stops() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::began(&NEAR_LIMIT), t0);
        go(&mut b, t0, 1.0);
        assert_eq!(b.chip().level, Some(Level::Warn));
        assert!(!b.chip().ring);
        b.apply(Report::ended(&NEAR_LIMIT), at(t0, 4.0));
        go(&mut b, t0, 6.9);
        assert_eq!(b.chip().now, 1, "kept SETTLE_OFF after it stops");
        go(&mut b, t0, 7.0);
        let c = b.chip();
        assert_eq!((c.now, c.unseen, c.ring), (0, 1, true));
        let p = b.problems(System::Sound, Window::Session)[0];
        assert_eq!((p.times, p.first, p.ended), (1, t0, Some(at(t0, 4.0))));
    }

    #[test]
    fn a_state_back_within_three_seconds_is_the_same_spell() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::began(&GRAINS_THINNED), t0);
        go(&mut b, t0, 1.5);
        b.apply(Report::ended(&GRAINS_THINNED), at(t0, 2.0));
        b.apply(Report::began(&GRAINS_THINNED), at(t0, 3.0));
        go(&mut b, t0, 6.0);
        let p = b.problems(System::Sound, Window::Now);
        assert_eq!(p.len(), 1, "never ended");
        assert_eq!(p[0].times, 1, "one spell");
        // A new spell later settles again and counts.
        b.apply(Report::ended(&GRAINS_THINNED), at(t0, 7.0));
        go(&mut b, t0, 10.5);
        b.apply(Report::began(&GRAINS_THINNED), at(t0, 20.0));
        go(&mut b, t0, 20.5);
        assert!(
            b.problems(System::Sound, Window::Now).is_empty(),
            "settling"
        );
        go(&mut b, t0, 21.0);
        assert_eq!(b.problems(System::Sound, Window::Now)[0].times, 2);
    }

    #[test]
    fn something_still_wrong_stays_filled_after_it_is_seen() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::began(&PORT_MISSING).about(Subject(3)), t0);
        b.seen(at(t0, 1.0));
        let c = b.chip();
        assert_eq!((c.level, c.ring, c.now), (Some(Level::Warn), false, 1));
        b.apply(Report::ended(&PORT_MISSING).about(Subject(3)), at(t0, 5.0));
        assert_eq!(b.chip(), ChipView::default(), "ended after it was seen");
    }

    #[test]
    fn one_card_per_kind_and_subject() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::began(&PORT_MISSING).about(Subject(1)), t0);
        b.apply(Report::began(&PORT_MISSING).about(Subject(2)), t0);
        b.apply(Report::began(&PORT_MISSING).about(Subject(1)), at(t0, 1.0));
        let p = b.problems(System::Midi, Window::Now);
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|p| p.times == 1), "began twice is one spell");
        assert_eq!(b.systems()[System::Midi.index()].now, 2);
    }

    #[test]
    fn the_worst_comes_first_and_colours_the_chip() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::happened(&VOICES_GIVEN_UP, 6), t0);
        b.apply(Report::happened(&SOUND_DROPOUT, 1), at(t0, 0.5));
        go(&mut b, t0, 4.0);
        b.apply(Report::happened(&VOICES_GIVEN_UP, 1), at(t0, 4.0));
        b.apply(Report::happened(&SOUND_SKIPPED, 1), at(t0, 1.0));
        go(&mut b, t0, 4.5);
        let p = b.problems(System::Sound, Window::Session);
        let ids: Vec<_> = p.iter().map(|p| p.kind.id).collect();
        assert_eq!(
            ids,
            ["sound/skipped", "sound/dropout", "sound/voices-given-up"],
            "red first, the most recent red first; yellow now after them"
        );
        // A red that ended outranks a yellow now: a red ring.
        go(&mut b, t0, 4.6);
        let c = b.chip();
        assert_eq!((c.level, c.ring), (Some(Level::Fail), true));
        let s = b.systems()[System::Sound.index()];
        assert_eq!(
            (s.level, s.ring, s.now, s.earlier),
            (Some(Level::Fail), true, 1, 2)
        );
        assert_eq!(b.systems()[System::Memory.index()].level, None, "works");
    }

    #[test]
    fn a_cause_hides_the_effects_it_brings() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::happened(&SOUND_DROPOUT, 1), t0);
        b.seen(t0);
        b.apply(Report::began(&NO_SESSION), at(t0, 1.0));
        b.apply(Report::happened(&SOUND_DROPOUT, 4), at(t0, 1.2));
        b.apply(Report::happened(&SAVE_FAILED, 1), at(t0, 1.2));
        go(&mut b, t0, 2.5);
        let sound = b.problems(System::Sound, Window::Session);
        assert!(sound.is_empty(), "dropouts during it are its echoes");
        assert_eq!(
            b.problems(System::Storage, Window::Now).len(),
            1,
            "not hidden"
        );
        let raw = b.raw();
        assert!(raw
            .iter()
            .any(|p| p.kind == &SOUND_DROPOUT && p.hidden && p.times == 5));
        // Once the session is back, a new dropout is its own problem again.
        b.apply(Report::ended(&NO_SESSION), at(t0, 3.0));
        go(&mut b, t0, 6.5);
        b.apply(Report::happened(&SOUND_DROPOUT, 1), at(t0, 7.0));
        assert_eq!(b.problems(System::Sound, Window::Now).len(), 1);
    }

    #[test]
    fn a_note_is_listed_but_never_colours() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::happened(&TAKE_RECOVERED, 1), t0);
        assert_eq!(b.chip(), ChipView::default());
        assert_eq!(b.systems()[System::Storage.index()].level, None);
        assert_eq!(b.problems(System::Storage, Window::Session).len(), 1);
        assert!(b.unexpected(&[]).is_empty(), "a note is never unexpected");
    }

    #[test]
    fn a_problem_that_stays_until_seen_ends_when_seen() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::began(&TAKE_DAMAGED), t0);
        go(&mut b, t0, 60.0);
        assert_eq!(b.chip().now, 1, "it stays");
        b.seen(at(t0, 61.0));
        assert_eq!(b.chip(), ChipView::default());
        let p = b.problems(System::Storage, Window::Session)[0];
        assert_eq!(p.ended, Some(at(t0, 61.0)));
    }

    #[test]
    fn a_problem_that_stays_shows_at_once_and_ends_when_told() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::began(&NO_GPU), t0);
        assert_eq!(b.chip().level, Some(Level::Warn), "no settling");
        go(&mut b, t0, 600.0);
        assert_eq!(b.chip().now, 1);
        b.apply(Report::ended(&NO_GPU), at(t0, 601.0));
        assert!(b.chip().ring, "ended, not seen");
    }

    #[test]
    fn it_keeps_the_last_256_reports_and_counts_the_rest() {
        let t0 = Instant::now();
        let mut b = Board::new();
        for i in 0..300u32 {
            b.apply(Report::happened(&SOUND_DROPOUT, 1), at(t0, i as f32 / 10.0));
        }
        assert_eq!(b.events().count(), EVENTS);
        assert_eq!(b.older(), 300 - EVENTS as u64);
        let ats: Vec<Instant> = b.events().map(|e| e.at).collect();
        assert_eq!(ats[0], at(t0, 29.9), "newest first");
        assert!(ats.windows(2).all(|w| w[0] >= w[1]));
        assert_eq!(b.problems(System::Sound, Window::Session)[0].times, 300);
    }

    #[test]
    fn a_full_board_makes_room_from_the_oldest_seen() {
        let t0 = Instant::now();
        let mut b = Board::new();
        for s in 0..OPEN as u32 {
            b.apply(
                Report::happened(&SOUND_DROPOUT, 1).about(Subject(s)),
                at(t0, s as f32),
            );
        }
        go(&mut b, t0, 100.0);
        b.seen(at(t0, 100.0));
        b.apply(Report::happened(&SAVE_FAILED, 1), at(t0, 101.0));
        assert_eq!(b.len(), OPEN);
        assert!(
            !b.raw()
                .iter()
                .any(|p| p.kind == &SOUND_DROPOUT && p.subject == Subject(0)),
            "the oldest seen made room"
        );
        assert_eq!(b.lost(), 0);
        // Full of problems happening now: a new one can't make room, and
        // is counted.
        let mut b = Board::new();
        for s in 0..OPEN as u32 {
            b.apply(Report::began(&PORT_MISSING).about(Subject(s)), t0);
        }
        b.apply(Report::happened(&SAVE_FAILED, 1), t0);
        assert_eq!((b.len(), b.lost()), (OPEN, 1));
    }

    #[test]
    fn it_allocates_nothing_once_made() {
        let t0 = Instant::now();
        let mut b = Board::new();
        let (open, events) = (b.open.capacity(), b.events.capacity());
        for i in 0..2000u32 {
            let t = at(t0, i as f32 / 4.0);
            let k = crate::kinds::ALL[i as usize % crate::kinds::ALL.len()];
            let r = match (i % 3, k.shape) {
                (_, Shape::Happens) => Report::happened(k, 1),
                (0, _) => Report::ended(k),
                _ => Report::began(k),
            };
            b.apply(r.about(Subject(i % 90)), t);
            b.tick(t);
            if i % 50 == 0 {
                b.seen(t);
            }
        }
        assert_eq!((b.open.capacity(), b.events.capacity()), (open, events));
        assert!(b.len() <= OPEN);
    }

    #[test]
    fn unexpected_names_each_kind_not_allowed_once() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::happened(&VOICES_GIVEN_UP, 1), t0);
        b.apply(Report::happened(&SOUND_DROPOUT, 1).about(Subject(1)), t0);
        b.apply(Report::happened(&SOUND_DROPOUT, 1).about(Subject(2)), t0);
        b.apply(Report::began(&NEAR_LIMIT), t0);
        let u = b.unexpected(&[&VOICES_GIVEN_UP]);
        assert_eq!(
            u,
            vec![&SOUND_DROPOUT],
            "the settling state isn't shown yet"
        );
        assert!(b.unexpected(&[&VOICES_GIVEN_UP, &SOUND_DROPOUT]).is_empty());
    }

    #[test]
    fn an_end_for_a_problem_never_reported_is_ignored() {
        let t0 = Instant::now();
        let mut b = Board::new();
        b.apply(Report::ended(&PORT_MISSING), t0);
        assert!(b.is_empty());
        assert_eq!(b.events().count(), 1);
    }
}
