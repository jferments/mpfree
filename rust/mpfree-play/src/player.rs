//! The player: each period, plays the view's clips on the transport's spans.
//!
//! **Whole ticks.** A note's song position (after quantise, speed and
//! swing) is rounded to the nearest tick (1/960 of a beat, 0.26 ms at
//! 120 BPM, the MPC's own resolution). A note plays when its tick is one of
//! the ticks the transport plays in a span (`Span::ticks`), on that tick's
//! frame. The transport plays every tick exactly once between jumps, the
//! same for any period size, so notes inherit that: none is lost or doubled
//! at a period edge or a tempo change. Clip positions are worked out from
//! the tick each time, never counted, so a loop jump or a locate lands every
//! clip in the right place.
//!
//! **Note-offs** come on the tick where the note ends. A loop jump, a locate
//! or a stop ends every sounding note on the frame where it happens (sounds
//! keep their release tails); a panic (STOP while stopped) also sends
//! `AllOff`. A note that starts while the same note is still sounding on the
//! same track ends the old one first, on the same frame.
//!
//! **Replace recording.** The first time in a take that the transport
//! records with overdub off, the armed track's notes are erased as the
//! playhead reaches them, for one pass of its clip: until the clip wraps,
//! the transport jumps, recording stops or overdub turns on. Then the take
//! goes on as overdub. Erased notes are not played; [`Player::erased`]
//! names them for the control thread to remove (`Clip::remove_note`).
//! The pass's parameter locks are erased the same way
//! ([`Player::erased_locks`]), trigless ones too, so the new take does not
//! play the old take's locks.
//!
//! **Chance.** A note with a chance below 100 % rolls each time it comes
//! round. The roll is a hash of the note and the absolute frame it would
//! play on, so it is the same for any period size, yet differs on every
//! pass, transport loops included.
//!
//! **Trig conditions** (`mpfree_seq::Cond`, the Elektron way). A note's
//! condition is decided first, and its chance rolls only if it holds.
//! A:B and 1ST count passes from the tick the clip started on
//! ([`crate::Launch::start`]), by song position: pass 1 starts there, the
//! passes before it count 0, -1 and so on, and a locate or a loop jump
//! lands on the right pass. FILL is one switch for the whole project,
//! turned on and off on an exact frame ([`Player::set_fill`]). Each track
//! keeps its last result: whether its last conditional note played
//! (`Cond::sets_last`), which PRE reads, and NEI reads on the track to its
//! left (the first track has none). The player decides a span's notes in
//! the order they play (tick, then track, then the clip's order), so the
//! results are the same for any period size; on one tick, the track to the
//! left is decided first. A track's result starts again at "not played"
//! when its clip starts, and on a stop or a locate; a transport loop jump
//! keeps it.
//!
//! **Ratchets.** A note with a ratchet of N plays N hits spread evenly over
//! its length, each lasting 1/N of it (hits less than a tick apart merge).
//! Swing and quantise move the first hit; the rest keep their spacing from
//! it. The condition and chance are decided once, on the first hit, so a
//! ratchet plays whole or not at all: its later hits wait in a table until
//! their tick. A stop, a locate or a loop jump drops them; a switch drops
//! the ones from its bar line on; a muted track drops its own.
//!
//! **Cues** (see [`crate::view`]). A track's waiting cue starts on the
//! first bar line the playhead plays: the clip so far plays the ticks
//! before it, its notes still sounding end on it, and the cued clip plays
//! from it. [`Player::started`] names each switch for the control thread.
//! A switch on the armed track ends a replace pass (the take goes on as
//! overdub).
//!
//! **Parameter locks** (the Elektron way). A clip's lock plays on its
//! tick as [`Event::Lock`], before any note-on on the same frame (and
//! after the note-offs there), so the step's note starts with the locked
//! value. The lock holds until the track plays a note that has no lock on
//! that parameter on its own tick: [`Event::Unlock`] comes on that note's
//! frame, just before its note-on. A sampler pad's parameter is ended only
//! by that pad's notes; any other (the instrument, the strip, an insert)
//! by any note of the track. A note its condition or chance leaves out
//! ends nothing, and leaves out its own locks: those on its tick and row,
//! and a whole-step lock when every note on its tick is left out. A lock
//! with no note on its tick (trigless) always plays, and holds until the
//! next note in the same way. A stop, a locate or a transport loop jump unlocks
//! everything, after the note-offs; a track whose clip stops or switches
//! unlocks its locks on the switch, before the new clip's; a muted track,
//! or one with nothing playing, unlocks at once and sends no locks. The
//! player keeps what it has locked, so every Unlock follows a Lock.
//!
//! **No allocation, locks or panics** in `process`: fixed tables only. Full
//! tables are counted in [`Stats`], never silently dropped.

use std::ops::Range;

use mpfree_param::ParamAddr;
use mpfree_seq::mpfree_moves::Out as MoveOut;
use mpfree_seq::{Cond, DestId, Lock, Note, MAX_TRACKS};
use mpfree_transport::time::{Swing, Tick};
use mpfree_transport::{Block, Mark, Marker, Span, State};

use crate::view::{ClipView, PlayView, Started, Switches, TrackView};

/// Notes sounding at once, across all tracks.
pub const MAX_ACTIVE: usize = 256;
/// Notes starting in one span.
pub const MAX_STARTS: usize = 256;
/// Notes replace recording can erase in one period.
pub const MAX_ERASED: usize = 256;
/// Parameter locks and move values starting in one span, all tracks.
pub const MAX_LOCK_STARTS: usize = 1024;
/// Parameter locks held at once, all tracks.
pub const MAX_LOCKED: usize = 256;
/// Ratchet hits waiting for their tick, all tracks.
pub const MAX_HITS: usize = 256;
/// FILL changes in one period.
const MAX_FILL_CHANGES: usize = 8;
/// `dest` and `track` of an event meant for every destination.
pub const ALL: u16 = u16::MAX;

/// A parameter lock replace recording erased: remove it from clip `slot`
/// of `track`, as for [`Erased`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErasedLock {
    pub track: usize,
    pub slot: usize,
    pub lock: Lock,
    pub take: u32,
}

/// A note replace recording erased: remove it from clip `slot` of `track`.
/// `take` numbers the take that erased it, as in [`crate::Taken`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Erased {
    pub track: usize,
    pub slot: usize,
    pub note: Note,
    pub take: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    NoteOn {
        note: u8,
        velocity: u8,
    },
    NoteOff {
        note: u8,
    },
    /// Cut every sound (panic).
    AllOff,
    /// A parameter lock from a step: hold `addr` at `value` (in the
    /// parameter's own units) until [`Event::Unlock`]. For the resolver in
    /// `mpfree-mod` (`ModEvent::Lock`).
    Lock {
        addr: ParamAddr,
        value: f32,
    },
    /// A recorded move (an FX lane, `mpfree-moves`) sets `addr` to `value`
    /// (in the parameter's own units) until the next value, a step's
    /// `Lock` (which wins for its step) or [`Event::Unlock`]. For the
    /// resolver in `mpfree-mod` (`ModEvent::Lane`).
    Lane {
        addr: ParamAddr,
        value: f32,
    },
    /// The lock or move on `addr` ends: the parameter goes back to its own
    /// value.
    Unlock {
        addr: ParamAddr,
    },
}

// A lock's value comes from a clip, which holds only finite values.
impl Eq for Event {}

/// One event for a destination, on an exact frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Out {
    /// Frame offset in the period.
    pub offset: u32,
    /// Absolute frame.
    pub frame: u64,
    /// The track it came from ([`ALL`] for `AllOff`).
    pub track: u16,
    /// The destination ([`ALL`] for every one).
    pub dest: DestId,
    pub event: Event,
}

/// Where the player's events go: the host maps each destination to the
/// sampler, the arp, MIDI out ... Events arrive in frame order.
pub trait Sink {
    fn send(&mut self, out: Out);
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Notes started.
    pub notes: u64,
    /// Notes not started because a table was full. Should stay 0.
    pub dropped: u64,
    /// Note-offs that came after their tick (should stay 0).
    pub late_offs: u64,
    /// Notes and locks replace recording erased.
    pub erased: u64,
    /// Notes their chance left out.
    pub skipped: u64,
    /// Notes their trig condition left out (chance did not roll).
    pub unmet: u64,
    /// Parameter locks sent (each lock, not each unlock). Locks a full
    /// table left out count in `dropped`.
    pub locks: u64,
    /// Move values sent (`Event::Lane`).
    pub lanes: u64,
}

#[derive(Clone, Copy, Debug)]
struct Sounding {
    track: u16,
    dest: DestId,
    note: u8,
    off: Tick,
    /// Start order, to break ties.
    seq: u64,
}

#[derive(Clone, Copy, Debug)]
struct Start {
    tick: Tick,
    track: u16,
    dest: DestId,
    note: u8,
    velocity: u8,
    off: Tick,
    /// Percent; below 100 the note rolls with `key`.
    chance: u8,
    key: u64,
    cond: Cond,
    /// The clip's pass, counted from its start (pass 1).
    pass: i32,
    /// Hits, spread over `len` song ticks.
    ratchet: u8,
    len: f32,
    /// Not a note: the track's clip starts here, so its last result
    /// (for PRE and NEI) starts again at "not played".
    reset: bool,
    /// Decided before the span plays: the note plays.
    plays: bool,
}

/// A ratchet's later hit, waiting for its tick.
#[derive(Clone, Copy, Debug)]
struct Hit {
    tick: Tick,
    track: u16,
    dest: DestId,
    note: u8,
    velocity: u8,
    off: Tick,
}

const NO_HIT: Hit = Hit {
    tick: 0,
    track: 0,
    dest: 0,
    note: 0,
    velocity: 0,
    off: 0,
};

/// What a [`LockStart`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LockKind {
    /// A step's parameter lock.
    Step,
    /// A move's value (`value`).
    Lane,
    /// A move lets its parameter go.
    LaneEnd,
}

/// A parameter lock or move value to send in a span, or (`addr` None)
/// every lock and move of `track` to end: its clip stopped or switched, or
/// it went quiet.
#[derive(Clone, Copy, Debug)]
struct LockStart {
    kind: LockKind,
    tick: Tick,
    track: u16,
    dest: DestId,
    addr: Option<ParamAddr>,
    value: f32,
    /// The pad whose notes end it (see [`Lock::scope`]).
    scope: Option<u8>,
    /// The step row it sits on ([`Lock::sound`]): which notes on its tick
    /// it belongs to.
    row: Option<u8>,
}

/// A parameter held, one entry a parameter. A step's lock: since `at`,
/// until a note of `track` (of the pad `scope`, for a pad's parameter)
/// with no lock on its own tick. A move (`lane`): at `value`, until the
/// move lets go, a step's lock takes over, or everything ends.
#[derive(Clone, Copy, Debug)]
struct Held {
    track: u16,
    dest: DestId,
    addr: ParamAddr,
    scope: Option<u8>,
    at: Tick,
    /// Held by a move rather than a step's lock.
    lane: bool,
    /// The move's value, while `lane`.
    value: f32,
    /// A step's lock holding a parameter a move is also moving: the
    /// move's value now, to go back to when the lock ends.
    under: Option<f32>,
}

/// Where a track's moves played up to in the last span: on clip tick `to`
/// of pass `pass` of the clip in `slot` from `origin`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct MovesPos {
    slot: usize,
    origin: Tick,
    pass: i64,
    to: f64,
}

const NO_ADDR: ParamAddr = ParamAddr::new(mpfree_param::Owner::Macro(0), 0);
const NO_LOCK_START: LockStart = LockStart {
    kind: LockKind::Step,
    tick: 0,
    track: 0,
    dest: 0,
    addr: None,
    value: 0.0,
    scope: None,
    row: None,
};
const NO_HELD: Held = Held {
    track: 0,
    dest: 0,
    addr: NO_ADDR,
    scope: None,
    at: 0,
    lane: false,
    value: 0.0,
    under: None,
};

const NO_SOUND: Sounding = Sounding {
    track: 0,
    dest: 0,
    note: 0,
    off: 0,
    seq: 0,
};
const NO_START: Start = Start {
    tick: 0,
    track: 0,
    dest: 0,
    note: 0,
    velocity: 0,
    off: 0,
    chance: 100,
    key: 0,
    cond: Cond::Always,
    pass: 1,
    ratchet: 1,
    len: 0.0,
    reset: false,
    plays: true,
};

pub struct Player {
    sounding: [Sounding; MAX_ACTIVE],
    n_sounding: usize,
    starts: [Start; MAX_STARTS],
    n_starts: usize,
    seq: u64,
    stats: Stats,
    erased: [Erased; MAX_ERASED],
    n_erased: usize,
    erased_locks: [ErasedLock; MAX_ERASED],
    n_erased_locks: usize,
    /// Replace recording: the armed clip's pass being replaced.
    replacing: Option<i64>,
    /// This take (since the last record-on) has had its replace pass.
    replaced: bool,
    /// Record-ons so far: the number of the latest take.
    take: u32,
    /// Cues started that the view does not show yet.
    switches: Switches,
    started: [Started; MAX_TRACKS],
    n_started: usize,
    lock_starts: [LockStart; MAX_LOCK_STARTS],
    n_lock_starts: usize,
    held: [Held; MAX_LOCKED],
    n_held: usize,
    /// Ratchet hits to come, in tick order (equal ticks in the order
    /// their notes played).
    hits: [Hit; MAX_HITS],
    n_hits: usize,
    /// Each track's last result: whether its last conditional note played.
    last: [bool; MAX_TRACKS],
    /// The clip each track played in the last span: its slot and start.
    seen: [Option<(usize, Tick)>; MAX_TRACKS],
    fill: bool,
    /// FILL changes due in the coming period: (offset, on), in offset order.
    fill_changes: [(u32, bool); MAX_FILL_CHANGES],
    n_fill_changes: usize,
    /// This span has a note left out (so some locks may be too).
    left_out: bool,
    /// Where each track's moves played up to, to tell a clip that goes on
    /// from one that starts, jumps or comes back (which first chases its
    /// moves' state).
    moves_pos: [Option<MovesPos>; MAX_TRACKS],
}

const NO_ERASED_LOCK: ErasedLock = ErasedLock {
    track: 0,
    slot: 0,
    lock: Lock {
        tick: 0,
        sound: None,
        addr: mpfree_param::RelAddr::new(mpfree_param::OnTrack::Inst, 0),
        value: 0.0,
    },
    take: 0,
};

const NO_ERASED: Erased = Erased {
    track: 0,
    slot: 0,
    note: Note {
        tick: 0,
        len: 1,
        note: 0,
        velocity: 1,
        chance: 100,
        cond: mpfree_seq::Cond::Always,
        ratchet: 1,
    },
    take: 0,
};

impl Default for Player {
    fn default() -> Self {
        Player::new()
    }
}

impl Player {
    pub fn new() -> Player {
        Player {
            sounding: [NO_SOUND; MAX_ACTIVE],
            n_sounding: 0,
            starts: [NO_START; MAX_STARTS],
            n_starts: 0,
            seq: 0,
            stats: Stats::default(),
            erased: [NO_ERASED; MAX_ERASED],
            n_erased: 0,
            erased_locks: [NO_ERASED_LOCK; MAX_ERASED],
            n_erased_locks: 0,
            replacing: None,
            replaced: false,
            take: 0,
            switches: Switches::new(),
            started: [Started {
                mute: None,
                track: 0,
                slot: None,
                at: 0,
                origin: 0,
            }; MAX_TRACKS],
            n_started: 0,
            lock_starts: [NO_LOCK_START; MAX_LOCK_STARTS],
            n_lock_starts: 0,
            held: [NO_HELD; MAX_LOCKED],
            n_held: 0,
            hits: [NO_HIT; MAX_HITS],
            n_hits: 0,
            last: [false; MAX_TRACKS],
            seen: [None; MAX_TRACKS],
            fill: false,
            fill_changes: [(0, false); MAX_FILL_CHANGES],
            n_fill_changes: 0,
            left_out: false,
            moves_pos: [None; MAX_TRACKS],
        }
    }

    /// Turn FILL on or off from frame `offset` of the coming period (the
    /// next [`Player::process`] call). Notes with the FILL and !FILL
    /// conditions follow it from that frame on. Real-time safe. A period
    /// takes up to 8 changes; past that, a change replaces the last one.
    pub fn set_fill(&mut self, on: bool, offset: u32) {
        let n = self.n_fill_changes;
        if n == MAX_FILL_CHANGES {
            self.fill_changes[n - 1] = (offset, on);
            return;
        }
        let mut j = n;
        while j > 0 && self.fill_changes[j - 1].0 > offset {
            self.fill_changes[j] = self.fill_changes[j - 1];
            j -= 1;
        }
        self.fill_changes[j] = (offset, on);
        self.n_fill_changes += 1;
    }

    /// FILL, as of the end of the last period.
    pub fn fill(&self) -> bool {
        self.fill
    }

    /// FILL on frame `offset` of this period.
    fn fill_at(&self, offset: u32) -> bool {
        self.fill_changes[..self.n_fill_changes]
            .iter()
            .rev()
            .find(|c| c.0 <= offset)
            .map_or(self.fill, |c| c.1)
    }

    /// The cues that started in the last [`Player::process`] call, in the
    /// order they did. Apply each to the launches ([`Started::apply`]) and
    /// compile a new view.
    pub fn started(&self) -> &[Started] {
        &self.started[..self.n_started]
    }

    /// The notes replace recording erased in the last
    /// [`Player::process`] call, in the order the playhead reached them.
    pub fn erased(&self) -> &[Erased] {
        &self.erased[..self.n_erased]
    }

    /// The parameter locks replace recording erased in the last
    /// [`Player::process`] call, in the order the playhead reached them:
    /// the old notes' locks and trigless locks of the pass being replaced.
    pub fn erased_locks(&self) -> &[ErasedLock] {
        &self.erased_locks[..self.n_erased_locks]
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Notes sounding now (started, not yet ended).
    pub fn sounding(&self) -> usize {
        self.n_sounding
    }

    /// Parameter locks held now (sent, not yet unlocked).
    pub fn locked(&self) -> usize {
        self.n_held
    }

    /// Play one period: the transport's `block` for it, the clips in `view`.
    pub fn process(&mut self, view: &PlayView, block: &Block, out: &mut impl Sink) {
        self.n_erased = 0;
        self.n_erased_locks = 0;
        self.n_started = 0;
        self.switches.follow(view);
        let markers = block.markers();
        let mut m = 0;
        for span in block.spans() {
            // A marker on a span's first frame happened just before it.
            while m < markers.len() && markers[m].offset <= span.offset {
                self.marker(&markers[m], out);
                m += 1;
            }
            if matches!(span.state, State::Playing | State::Recording) {
                self.play_span(view, span, out);
            }
        }
        for marker in &markers[m..] {
            self.marker(marker, out);
        }
        if self.n_fill_changes > 0 {
            self.fill = self.fill_changes[self.n_fill_changes - 1].1;
            self.n_fill_changes = 0;
        }
    }

    fn marker(&mut self, m: &Marker, out: &mut impl Sink) {
        match m.mark {
            Mark::RecordOn { .. } => {
                self.replacing = None;
                self.replaced = false;
                self.take = self.take.wrapping_add(1);
            }
            Mark::RecordOff { .. } => self.replacing = None,
            _ => {}
        }
        if matches!(
            m.mark,
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Stopped { .. }
        ) {
            self.replacing = None;
        }
        match m.mark {
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Stopped { .. } => {
                self.end_all(m.offset, m.frame, out);
                self.unlock_where(m.offset, m.frame, out, false, |_| true);
                self.moves_pos = [None; MAX_TRACKS];
                // A loop jump keeps the last results, so a PRE chain
                // carries round the loop.
                if !matches!(m.mark, Mark::Looped { .. }) {
                    self.last = [false; MAX_TRACKS];
                }
            }
            Mark::Panic => {
                self.end_all(m.offset, m.frame, out);
                self.unlock_where(m.offset, m.frame, out, false, |_| true);
                self.moves_pos = [None; MAX_TRACKS];
                out.send(Out {
                    offset: m.offset,
                    frame: m.frame,
                    track: ALL,
                    dest: ALL,
                    event: Event::AllOff,
                });
            }
            _ => {}
        }
    }

    /// End every sounding note, in the order they started, and drop the
    /// ratchet hits to come.
    fn end_all(&mut self, offset: u32, frame: u64, out: &mut impl Sink) {
        self.n_hits = 0;
        for s in &self.sounding[..self.n_sounding] {
            out.send(Out {
                offset,
                frame,
                track: s.track,
                dest: s.dest,
                event: Event::NoteOff { note: s.note },
            });
        }
        self.n_sounding = 0;
    }

    fn play_span(&mut self, view: &PlayView, span: &Span, out: &mut impl Sink) {
        let ticks = span.ticks();
        if ticks.is_empty() {
            return;
        }
        self.replace_state(view, span, &ticks);
        self.n_starts = 0;
        self.n_lock_starts = 0;
        let bar = span.meter.bar_ticks();
        for (i, track) in view.tracks.iter().enumerate() {
            // A scene's mute switches with its cue, before the view shows it.
            let muted = self.switches.muted(view, i);
            let replace = if view.armed == Some(i) {
                self.replacing
            } else {
                None
            };
            // A cue starting in this span: the clip so far plays up to it.
            let switch = self.switches.due(view, i, &ticks, bar);
            let before = switch.map_or(ticks.clone(), |at| ticks.start..at);
            let playing = self.switches.playing_from(view, i);
            // Song mode: a track with no clip launched plays its
            // placements.
            let song = view.song && playing.is_none();
            let region = if song {
                track
                    .region_at(ticks.start)
                    .map(|r| (r.clip.slot, r.clip.start))
            } else {
                None
            };
            // A clip started since the last span (a launch, not a cue):
            // the track's last result starts again.
            let now = playing.map(|(c, _, start)| (c.slot, start)).or(region);
            if let (Some(seen), Some(last)) = (self.seen.get_mut(i), self.last.get_mut(i)) {
                if *seen != now {
                    *seen = now;
                    *last = false;
                }
            }
            // A track gone quiet lets its locks and ratchets go at once.
            if muted || (playing.is_none() && region.is_none()) {
                if self.held[..self.n_held]
                    .iter()
                    .any(|h| usize::from(h.track) == i)
                {
                    self.end_locks(ticks.start, i as u16, track.dest);
                }
                self.cut_hits(i as u16, ticks.start);
                self.forget_moves(i);
            }
            if let Some((clip, origin, start)) = playing {
                if !(muted && replace.is_none()) {
                    self.collect(
                        clip, origin, start, i as u16, track.dest, span.swing, &before, replace,
                        !muted, switch,
                    );
                }
                // The clip's moves after its locks: a step's lock on the
                // same tick takes the parameter first, and the move's value
                // waits under it.
                if muted || clip.moves.is_empty() {
                    self.forget_moves(i);
                } else {
                    self.collect_moves(clip, origin, i, track.dest, &before);
                }
            } else if song && !muted {
                self.collect_song(track, i, &before, switch, span.swing);
            }
            let Some(at) = switch else { continue };
            // A track starts one cue a call at most, and there are at most
            // MAX_TRACKS tracks, so this always fits.
            if let (Some(s), Some(slot)) = (
                self.switches.start(view, i, at),
                self.started.get_mut(self.n_started),
            ) {
                *slot = s;
                self.n_started += 1;
            }
            for s in &mut self.sounding[..self.n_sounding] {
                if usize::from(s.track) == i {
                    s.off = s.off.min(at);
                }
            }
            self.cut_hits(i as u16, at);
            // The old clip's locks end on the switch, before the new one's.
            self.end_locks(at, i as u16, track.dest);
            // The new clip's last result starts at "not played", on the
            // switch, after the old clip's notes before it.
            self.push_start(Start {
                tick: at,
                track: i as u16,
                reset: true,
                ..NO_START
            });
            let playing = self.switches.playing_from(view, i);
            if let Some(seen) = self.seen.get_mut(i) {
                *seen = playing.map(|(c, _, start)| (c.slot, start));
            }
            if view.armed == Some(i) {
                self.replacing = None;
            }
            if let (Some((clip, origin, start)), false) = (playing, self.switches.muted(view, i)) {
                self.collect(
                    clip,
                    origin,
                    start,
                    i as u16,
                    track.dest,
                    span.swing,
                    &(at..ticks.end),
                    None,
                    true,
                    None,
                );
                if clip.moves.is_empty() {
                    self.forget_moves(i);
                } else {
                    self.collect_moves(clip, origin, i, track.dest, &(at..ticks.end));
                }
            } else if view.song && playing.is_none() && !self.switches.muted(view, i) {
                // Stopped: the track goes back to the song.
                self.collect_song(track, i, &(at..ticks.end), None, span.swing);
            }
        }
        // In tick order; equal ticks keep track order, then note order. An
        // insertion sort: stable, in place (no allocation), and fast here
        // because each track's notes already come in order.
        for i in 1..self.n_starts {
            let s = self.starts[i];
            let mut j = i;
            while j > 0 && self.starts[j - 1].tick > s.tick {
                self.starts[j] = self.starts[j - 1];
                j -= 1;
            }
            self.starts[j] = s;
        }
        for i in 1..self.n_lock_starts {
            let s = self.lock_starts[i];
            let mut j = i;
            while j > 0 && self.lock_starts[j - 1].tick > s.tick {
                self.lock_starts[j] = self.lock_starts[j - 1];
                j -= 1;
            }
            self.lock_starts[j] = s;
        }
        self.decide(span);
        let first = ticks.start;
        let mut k = 0;
        for i in 0..self.n_starts {
            let s = self.starts[i];
            // Locks on or before this tick go first (after the note-offs
            // before them), so a note starts with its step's locks. A note
            // left out leaves out its own locks.
            while k < self.n_lock_starts && self.lock_starts[k].tick <= s.tick {
                let l = self.lock_starts[k];
                self.advance(l.tick, false, first, span, out);
                if self.lock_plays(&l, i) {
                    self.lock(l, span, out);
                }
                k += 1;
            }
            // Notes ending on or before this tick end first, and ratchet
            // hits on or before it play.
            self.advance(s.tick, true, first, span, out);
            if s.reset || !s.plays {
                continue;
            }
            if s.ratchet > 1 {
                self.ratchet(i);
            }
            let (offset, frame) = place(span, s.tick);
            if let Some(j) = self.find(s.track, s.dest, s.note) {
                out.send(Out {
                    offset,
                    frame,
                    track: s.track,
                    dest: s.dest,
                    event: Event::NoteOff { note: s.note },
                });
                self.remove(j);
            }
            if self.n_sounding == MAX_ACTIVE {
                self.stats.dropped += 1;
                continue;
            }
            // The track's locks without a lock on this note's tick end (a
            // move under one takes its parameter back). Moves go on.
            let (track, note, tick) = (s.track, s.note, s.tick);
            self.unlock_where(offset, frame, out, true, |h| {
                !h.lane && h.track == track && h.at != tick && h.scope.is_none_or(|p| p == note)
            });
            out.send(Out {
                offset,
                frame,
                track: s.track,
                dest: s.dest,
                event: Event::NoteOn {
                    note: s.note,
                    velocity: s.velocity,
                },
            });
            self.sounding[self.n_sounding] = Sounding {
                track: s.track,
                dest: s.dest,
                note: s.note,
                off: s.off,
                seq: self.seq,
            };
            self.n_sounding += 1;
            self.seq += 1;
            self.stats.notes += 1;
        }
        while k < self.n_lock_starts {
            let l = self.lock_starts[k];
            self.advance(l.tick, false, first, span, out);
            self.lock(l, span, out);
            k += 1;
        }
        self.advance(ticks.end - 1, true, first, span, out);
    }

    /// Decide this span's notes, in the order they play: each note's trig
    /// condition, then its chance. The order (tick, then track, then the
    /// clip's order) is the same for any period size, so the last results
    /// that PRE and NEI read are too.
    fn decide(&mut self, span: &Span) {
        self.left_out = false;
        for j in 0..self.n_starts {
            let s = self.starts[j];
            let t = usize::from(s.track);
            if s.reset {
                if let Some(last) = self.last.get_mut(t) {
                    *last = false;
                }
                continue;
            }
            if s.cond == Cond::Always && s.chance >= 100 {
                continue;
            }
            let fill = match s.cond {
                Cond::Fill | Cond::NotFill => self.fill_at(place(span, s.tick).0),
                _ => false,
            };
            let pre = self.last.get(t).copied().unwrap_or(false);
            let nei = t.checked_sub(1).and_then(|n| self.last.get(n).copied());
            let met = s.cond.holds(i64::from(s.pass), fill, pre, nei);
            let plays = met
                && (s.chance >= 100
                    || mix(s.key ^ place(span, s.tick).1) % 100 < u64::from(s.chance));
            if !met {
                self.stats.unmet += 1;
            } else if !plays {
                self.stats.skipped += 1;
            }
            self.left_out |= !plays;
            if s.cond.sets_last(s.chance) {
                if let Some(last) = self.last.get_mut(t) {
                    *last = plays;
                }
            }
            self.starts[j].plays = plays;
        }
    }

    /// Whether lock `l` plays, with `starts[from..]` this span's notes not
    /// yet played: a lock on a step's note (on its tick, and its row, or
    /// any row for a whole-step lock) plays if one of those notes does. A
    /// trigless lock, with no such note, always plays.
    fn lock_plays(&self, l: &LockStart, from: usize) -> bool {
        if l.kind != LockKind::Step || l.addr.is_none() || !self.left_out {
            return true;
        }
        let mut notes = self.starts[from..self.n_starts]
            .iter()
            .take_while(|s| s.tick == l.tick)
            .filter(|s| !s.reset && s.track == l.track && l.row.is_none_or(|r| r == s.note))
            .peekable();
        notes.peek().is_none() || notes.any(|s| s.plays)
    }

    fn push_start(&mut self, s: Start) {
        if self.n_starts == MAX_STARTS {
            self.stats.dropped += 1;
            return;
        }
        self.starts[self.n_starts] = s;
        self.n_starts += 1;
    }

    /// Queue the later hits of `starts[i]`, a ratchet whose first hit
    /// plays now: spread evenly over the note's length (hits less than a
    /// tick apart merge), none from a switch of its track on (the switch's
    /// reset comes after it in the starts).
    fn ratchet(&mut self, i: usize) {
        let s = self.starts[i];
        let n = i64::from(s.ratchet.max(1));
        let cut = self.starts[i + 1..self.n_starts]
            .iter()
            .find(|x| x.reset && x.track == s.track)
            .map_or(Tick::MAX, |x| x.tick);
        let at = |h: i64| s.tick + (f64::from(s.len) * h as f64 / n as f64).round() as Tick;
        let mut prev = s.tick;
        for h in 1..n {
            let tick = at(h);
            if tick >= cut {
                break;
            }
            if tick <= prev {
                continue;
            }
            prev = tick;
            self.push_hit(Hit {
                tick,
                track: s.track,
                dest: s.dest,
                note: s.note,
                velocity: s.velocity,
                off: at(h + 1).max(tick + 1).min(cut),
            });
        }
    }

    /// Queue a hit after those on or before its tick.
    fn push_hit(&mut self, h: Hit) {
        if self.n_hits == MAX_HITS {
            self.stats.dropped += 1;
            return;
        }
        let mut j = self.n_hits;
        while j > 0 && self.hits[j - 1].tick > h.tick {
            self.hits[j] = self.hits[j - 1];
            j -= 1;
        }
        self.hits[j] = h;
        self.n_hits += 1;
    }

    /// Drop `track`'s hits from `at` on; the rest end by `at`.
    fn cut_hits(&mut self, track: u16, at: Tick) {
        let mut kept = 0;
        for j in 0..self.n_hits {
            let mut h = self.hits[j];
            if h.track == track {
                if h.tick >= at {
                    continue;
                }
                h.off = h.off.min(at);
            }
            self.hits[kept] = h;
            kept += 1;
        }
        self.n_hits = kept;
    }

    /// Play what is due by `tick`: the note-offs on or before it and the
    /// ratchet hits before it (on it too, with `on_tick`), in time order,
    /// note-offs first on a tick.
    fn advance(
        &mut self,
        tick: Tick,
        on_tick: bool,
        first: Tick,
        span: &Span,
        out: &mut impl Sink,
    ) {
        while self.n_hits > 0
            && (self.hits[0].tick < tick || (on_tick && self.hits[0].tick == tick))
        {
            let h = self.hits[0];
            self.hits.copy_within(1..self.n_hits, 0);
            self.n_hits -= 1;
            self.end_until(h.tick, first, span, out);
            let (offset, frame) = place(span, h.tick);
            if let Some(j) = self.find(h.track, h.dest, h.note) {
                out.send(Out {
                    offset,
                    frame,
                    track: h.track,
                    dest: h.dest,
                    event: Event::NoteOff { note: h.note },
                });
                self.remove(j);
            }
            if self.n_sounding == MAX_ACTIVE {
                self.stats.dropped += 1;
                continue;
            }
            out.send(Out {
                offset,
                frame,
                track: h.track,
                dest: h.dest,
                event: Event::NoteOn {
                    note: h.note,
                    velocity: h.velocity,
                },
            });
            self.sounding[self.n_sounding] = Sounding {
                track: h.track,
                dest: h.dest,
                note: h.note,
                off: h.off,
                seq: self.seq,
            };
            self.n_sounding += 1;
            self.seq += 1;
            self.stats.notes += 1;
        }
        self.end_until(tick, first, span, out);
    }

    /// Queue the end of every lock `track` holds, on `tick`.
    fn end_locks(&mut self, tick: Tick, track: u16, dest: DestId) {
        self.push_lock(LockStart {
            kind: LockKind::Step,
            tick,
            track,
            dest,
            addr: None,
            value: 0.0,
            scope: None,
            row: None,
        });
    }

    fn push_lock(&mut self, l: LockStart) {
        if self.n_lock_starts == MAX_LOCK_STARTS {
            self.stats.dropped += 1;
            return;
        }
        self.lock_starts[self.n_lock_starts] = l;
        self.n_lock_starts += 1;
    }

    /// Send a lock or a move's value and hold its parameter, let a move
    /// go, or end a track's locks and moves. One entry a parameter: a
    /// step's lock beats a move on its step (the move's value waits under
    /// it), and a move takes the parameter again when the lock ends.
    fn lock(&mut self, l: LockStart, span: &Span, out: &mut impl Sink) {
        let (offset, frame) = place(span, l.tick);
        let Some(addr) = l.addr else {
            self.unlock_where(offset, frame, out, false, |h| h.track == l.track);
            return;
        };
        let held = self.held[..self.n_held].iter().position(|h| h.addr == addr);
        let at = |event| Out {
            offset,
            frame,
            track: l.track,
            dest: l.dest,
            event,
        };
        match l.kind {
            LockKind::Step => {
                match held {
                    Some(j) => {
                        let h = &mut self.held[j];
                        if h.lane {
                            h.under = Some(h.value);
                            h.lane = false;
                        }
                        (h.track, h.dest, h.scope, h.at) = (l.track, l.dest, l.scope, l.tick);
                    }
                    None if self.n_held == MAX_LOCKED => {
                        self.stats.dropped += 1;
                        return;
                    }
                    None => {
                        self.held[self.n_held] = Held {
                            track: l.track,
                            dest: l.dest,
                            addr,
                            scope: l.scope,
                            at: l.tick,
                            ..NO_HELD
                        };
                        self.n_held += 1;
                    }
                }
                out.send(at(Event::Lock {
                    addr,
                    value: l.value,
                }));
                self.stats.locks += 1;
            }
            LockKind::Lane => {
                match held {
                    // A step's lock holds it: the move waits under it.
                    Some(j) if !self.held[j].lane => {
                        self.held[j].under = Some(l.value);
                        return;
                    }
                    Some(j) => {
                        let h = &mut self.held[j];
                        // Already there (a chase and its first point, or a
                        // flat stretch): nothing to send.
                        if h.value.to_bits() == l.value.to_bits() {
                            return;
                        }
                        (h.track, h.dest, h.value) = (l.track, l.dest, l.value);
                    }
                    None if self.n_held == MAX_LOCKED => {
                        self.stats.dropped += 1;
                        return;
                    }
                    None => {
                        self.held[self.n_held] = Held {
                            track: l.track,
                            dest: l.dest,
                            addr,
                            at: l.tick,
                            lane: true,
                            value: l.value,
                            ..NO_HELD
                        };
                        self.n_held += 1;
                    }
                }
                out.send(at(Event::Lane {
                    addr,
                    value: l.value,
                }));
                self.stats.lanes += 1;
            }
            LockKind::LaneEnd => match held {
                Some(j) if self.held[j].lane => {
                    let h = self.held[j];
                    self.held.copy_within(j + 1..self.n_held, j);
                    self.n_held -= 1;
                    out.send(Out {
                        offset,
                        frame,
                        track: h.track,
                        dest: h.dest,
                        event: Event::Unlock { addr },
                    });
                }
                // Under a step's lock: when it ends, the parameter is free.
                Some(j) => self.held[j].under = None,
                None => {}
            },
        }
    }

    /// End the held locks and moves `f` picks, in the order they were
    /// held. With `hand_back`, a lock with a move under it gives the
    /// parameter back to the move (its value, not an unlock).
    fn unlock_where(
        &mut self,
        offset: u32,
        frame: u64,
        out: &mut impl Sink,
        hand_back: bool,
        f: impl Fn(&Held) -> bool,
    ) {
        let mut kept = 0;
        for j in 0..self.n_held {
            let mut h = self.held[j];
            if !f(&h) {
                self.held[kept] = h;
                kept += 1;
                continue;
            }
            match h.under.filter(|_| hand_back) {
                Some(value) => {
                    out.send(Out {
                        offset,
                        frame,
                        track: h.track,
                        dest: h.dest,
                        event: Event::Lane {
                            addr: h.addr,
                            value,
                        },
                    });
                    self.stats.lanes += 1;
                    (h.lane, h.value, h.under) = (true, value, None);
                    self.held[kept] = h;
                    kept += 1;
                }
                None => out.send(Out {
                    offset,
                    frame,
                    track: h.track,
                    dest: h.dest,
                    event: Event::Unlock { addr: h.addr },
                }),
            }
        }
        self.n_held = kept;
    }

    /// The moves of `clip` (from `origin`) due on `ticks`: each lane's
    /// values on the clip's own ticks of each pass, on the song tick each
    /// rounds to. When the clip did not just play up to the first of them
    /// (it starts, comes back, or the song jumped), every lane's state
    /// there comes first.
    fn collect_moves(
        &mut self,
        clip: &ClipView,
        origin: Tick,
        track: usize,
        dest: DestId,
        ticks: &Range<Tick>,
    ) {
        if clip.moves.is_empty() || clip.song_len <= 0.0 || ticks.is_empty() {
            if let Some(p) = self.moves_pos.get_mut(track) {
                *p = None;
            }
            return;
        }
        let len = clip.length as f64;
        // A value on song tick x sounds on x rounded half up: these bounds
        // put each in exactly one span, however the periods fall.
        let lo = ticks.start as f64 - 0.5;
        let hi = ticks.end as f64 - 0.5;
        let o = origin as f64;
        let first = ((lo - o) / clip.song_len).floor().max(0.0);
        let last = ((hi - o) / clip.song_len).floor();
        let (tr, t8) = (track as u16, track as u8);
        let mut checked = false;
        let mut pass = first;
        while pass <= last {
            let start = o + pass * clip.song_len;
            let from = clip.speed.to_clip(lo - start).max(0.0);
            let to = clip.speed.to_clip(hi - start).min(len);
            let p = pass as i64;
            if from < to {
                if !checked {
                    checked = true;
                    let goes_on = self
                        .moves_pos
                        .get(track)
                        .copied()
                        .flatten()
                        .is_some_and(|m| {
                            m.slot == clip.slot
                                && m.origin == origin
                                && ((m.pass == p && m.to == from)
                                    || (m.pass + 1 == p && m.to >= len && from == 0.0))
                        });
                    if !goes_on {
                        // The state on the first tick this span plays.
                        let tick = ticks.start.max(half_up(start));
                        let at = clip.speed.to_clip(tick as f64 - start).clamp(0.0, len);
                        clip.moves.chase(at, |target, o| {
                            self.push_lane(tick, tr, dest, target.on(t8), o);
                        });
                    }
                }
                clip.moves.play(from, to, |t, target, o| {
                    let tick = half_up(start + clip.speed.to_song(t));
                    self.push_lane(tick.max(ticks.start), tr, dest, target.on(t8), o);
                });
                if let Some(m) = self.moves_pos.get_mut(track) {
                    *m = Some(MovesPos {
                        slot: clip.slot,
                        origin,
                        pass: p,
                        to,
                    });
                }
            }
            pass += 1.0;
        }
    }

    /// Track `i`'s moves did not play on: the next that do chase first.
    fn forget_moves(&mut self, i: usize) {
        if let Some(p) = self.moves_pos.get_mut(i) {
            *p = None;
        }
    }

    fn push_lane(&mut self, tick: Tick, track: u16, dest: DestId, addr: ParamAddr, o: MoveOut) {
        let (kind, value) = match o {
            MoveOut::Value(v) => (LockKind::Lane, v),
            MoveOut::End => (LockKind::LaneEnd, 0.0),
        };
        self.push_lock(LockStart {
            kind,
            tick,
            track,
            dest,
            addr: Some(addr),
            value,
            scope: None,
            row: None,
        });
    }

    /// Start or end the replace pass for this span.
    fn replace_state(&mut self, view: &PlayView, span: &Span, ticks: &Range<Tick>) {
        if span.state != State::Recording || span.overdub {
            self.replacing = None;
            return;
        }
        if self.replaced {
            return;
        }
        let clip = view.armed.and_then(|t| self.switches.playing(view, t));
        if let Some((clip, origin)) = clip.filter(|(c, o)| c.song_len > 0.0 && ticks.end > *o) {
            let from = (ticks.start - origin).max(0) as f64;
            self.replacing = Some((from / clip.song_len).floor() as i64);
            self.replaced = true;
        }
    }

    /// The track's placements over `ticks` (song mode): each one's notes,
    /// locks and moves in its own stretch, ending at its end (or `cut`,
    /// whichever is first). Where one starts the track's last result
    /// starts again; where one ends, its ratchets and locks end, as at a
    /// cue's switch.
    fn collect_song(
        &mut self,
        track: &TrackView,
        i: usize,
        ticks: &Range<Tick>,
        cut: Option<Tick>,
        swing: Swing,
    ) {
        let t = i as u16;
        let first = track.regions.partition_point(|r| r.end <= ticks.start);
        for r in &track.regions[first..] {
            if r.clip.start >= ticks.end {
                break;
            }
            let part = ticks.start.max(r.clip.start)..ticks.end.min(r.end);
            if ticks.contains(&r.clip.start) {
                self.push_start(Start {
                    tick: r.clip.start,
                    track: t,
                    reset: true,
                    ..NO_START
                });
            }
            let end = cut.map_or(r.end, |c| c.min(r.end));
            let clip = &r.clip;
            self.collect(
                clip,
                clip.origin,
                clip.start,
                t,
                track.dest,
                swing,
                &part,
                None,
                true,
                Some(end),
            );
            if clip.moves.is_empty() {
                self.forget_moves(i);
            } else {
                self.collect_moves(clip, clip.origin, i, track.dest, &part);
            }
            if ticks.contains(&r.end) {
                self.cut_hits(t, r.end);
                self.end_locks(r.end, t, track.dest);
                self.push_start(Start {
                    tick: r.end,
                    track: t,
                    reset: true,
                    ..NO_START
                });
            }
        }
    }

    /// The notes of `clip`, from `origin`, whose tick is in `ticks`. With
    /// `replace`, the notes of that pass are erased instead; without
    /// `sound`, the rest are not played. With `cut`, notes end there at the
    /// latest. Passes count from 1 at `start`, for the trig conditions.
    #[allow(clippy::too_many_arguments)]
    fn collect(
        &mut self,
        clip: &ClipView,
        origin: Tick,
        start: Tick,
        track: u16,
        dest: DestId,
        transport_swing: Swing,
        ticks: &Range<Tick>,
        replace: Option<i64>,
        sound: bool,
        cut: Option<Tick>,
    ) {
        if (clip.notes.is_empty() && clip.locks.is_empty())
            || clip.song_len <= 0.0
            || ticks.is_empty()
        {
            return;
        }
        // The pass `start` is in, counted from the origin (`num` passes
        // are exactly `length * den` song ticks), worked out at the first
        // note: most spans have none.
        let mut first_number = None;
        let number_of = |pass: f64, first: &mut Option<i64>| {
            let first = *first.get_or_insert_with(|| {
                ((start - origin) * Tick::from(clip.speed.num.max(1)))
                    .div_euclid(clip.length.max(1) * Tick::from(clip.speed.den.max(1)))
            });
            (pass as i64 - first + 1).clamp(-(1 << 30), 1 << 30) as i32
        };
        let swing = clip.swing.unwrap_or(transport_swing);
        // Swing only delays notes, by at most this much; rounding moves them
        // by half a tick. Look a little wider and keep the exact matches.
        let delay = f64::from(swing.grid) * f64::from(swing.amount.saturating_sub(50)) / 50.0;
        let lo = ticks.start as f64 - delay - 1.0;
        let hi = ticks.end as f64 + 1.0;
        let origin = origin as f64;
        let first_pass = ((lo - origin) / clip.song_len).floor().max(0.0);
        let last_pass = ((hi - origin) / clip.song_len).floor();
        if last_pass < first_pass {
            return;
        }
        let mut pass = first_pass;
        while pass <= last_pass {
            let start = origin + pass * clip.song_len;
            let from = clip.speed.to_clip(lo - start);
            let to = clip.speed.to_clip(hi - start);
            let first = clip.notes.partition_point(|n| n.local < from);
            for n in &clip.notes[first..] {
                if n.local > to {
                    break;
                }
                let straight = start + clip.speed.to_song(n.local);
                let tick = swing.apply(straight).round() as Tick;
                if !ticks.contains(&tick) {
                    continue;
                }
                if replace == Some(pass as i64) {
                    if self.n_erased == MAX_ERASED {
                        self.stats.dropped += 1;
                    } else {
                        self.erased[self.n_erased] = Erased {
                            track: usize::from(track),
                            slot: clip.slot,
                            note: n.src,
                            take: self.take,
                        };
                        self.n_erased += 1;
                        self.stats.erased += 1;
                    }
                    continue;
                }
                if !sound {
                    continue;
                }
                // A ratchet's first hit lasts its share of the note.
                let ratchet = n.src.ratchet.max(1);
                let off = if ratchet > 1 {
                    tick + (n.len / f64::from(ratchet)).round() as Tick
                } else {
                    swing.apply(straight + n.len).round() as Tick
                };
                let off = off.max(tick + 1);
                let off = cut.map_or(off, |c| off.min(c));
                let key = mix(mix(mix(u64::from(track)) ^ clip.slot as u64) ^ n.src.tick as u64)
                    ^ u64::from(n.note);
                self.push_start(Start {
                    tick,
                    track,
                    dest,
                    note: n.note,
                    velocity: n.velocity,
                    off,
                    chance: n.src.chance,
                    key,
                    cond: n.src.cond,
                    pass: number_of(pass, &mut first_number),
                    ratchet,
                    len: n.len as f32,
                    reset: false,
                    plays: true,
                });
            }
            // The pass's locks, placed as its notes are. A pass being
            // replaced erases them; a muted track sends none.
            if sound || replace == Some(pass as i64) {
                let first = clip.locks.partition_point(|l| l.local < from);
                for l in &clip.locks[first..] {
                    if l.local > to {
                        break;
                    }
                    let straight = start + clip.speed.to_song(l.local);
                    let tick = swing.apply(straight).round() as Tick;
                    if !ticks.contains(&tick) {
                        continue;
                    }
                    if replace == Some(pass as i64) {
                        if self.n_erased_locks == MAX_ERASED {
                            self.stats.dropped += 1;
                        } else {
                            self.erased_locks[self.n_erased_locks] = ErasedLock {
                                track: usize::from(track),
                                slot: clip.slot,
                                lock: l.src,
                                take: self.take,
                            };
                            self.n_erased_locks += 1;
                            self.stats.erased += 1;
                        }
                    } else {
                        self.push_lock(LockStart {
                            kind: LockKind::Step,
                            tick,
                            track,
                            dest,
                            addr: Some(l.addr.on(track as u8)),
                            value: l.value,
                            scope: Lock::scope(l.addr),
                            row: l.src.sound,
                        });
                    }
                }
            }
            pass += 1.0;
        }
    }

    /// End the sounding notes whose end tick is at or before `tick`, in
    /// tick order (ties in start order).
    fn end_until(&mut self, tick: Tick, first: Tick, span: &Span, out: &mut impl Sink) {
        loop {
            let mut best: Option<usize> = None;
            for (i, s) in self.sounding[..self.n_sounding].iter().enumerate() {
                if s.off <= tick
                    && best.is_none_or(|b| {
                        (s.off, s.seq) < (self.sounding[b].off, self.sounding[b].seq)
                    })
                {
                    best = Some(i);
                }
            }
            let Some(i) = best else { return };
            let s = self.sounding[i];
            let at = if s.off < first {
                self.stats.late_offs += 1;
                first
            } else {
                s.off
            };
            let (offset, frame) = place(span, at);
            out.send(Out {
                offset,
                frame,
                track: s.track,
                dest: s.dest,
                event: Event::NoteOff { note: s.note },
            });
            self.remove(i);
        }
    }

    fn find(&self, track: u16, dest: DestId, note: u8) -> Option<usize> {
        self.sounding[..self.n_sounding]
            .iter()
            .position(|s| s.track == track && s.dest == dest && s.note == note)
    }

    /// Remove keeping the start order.
    fn remove(&mut self, i: usize) {
        self.sounding.copy_within(i + 1..self.n_sounding, i);
        self.n_sounding -= 1;
    }
}

/// A 64-bit mix (splitmix64's finaliser), for the chance rolls.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Offset in the period and absolute frame of a tick in a span.
/// The song tick `x` sounds on: rounded half up, also below zero (where
/// `f64::round` would round half down), so it matches the half-open
/// bounds a span is cut on.
fn half_up(x: f64) -> Tick {
    (x + 0.5).floor() as Tick
}

fn place(span: &Span, tick: Tick) -> (u32, u64) {
    let frame = span
        .frame_of(tick as f64)
        .clamp(span.frame, span.frame + u64::from(span.len) - 1);
    (span.offset + (frame - span.frame) as u32, frame)
}
