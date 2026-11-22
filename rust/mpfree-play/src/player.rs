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
//! records with overdub off, each armed track's notes are erased as the
//! playhead reaches them, for one pass of its own clip: until that clip
//! wraps or switches, the transport jumps, recording stops or overdub turns
//! on. Then the take goes on as overdub on that track. The armed tracks are
//! the view's armed one and its others ([`PlayView::set_also`], several
//! tracks recording one take), each with a pass of its own, so a track
//! whose clip is half a bar long goes on as overdub while a two-bar clip's
//! pass still erases. Erased notes are not played; [`Player::erased`]
//! names them for the control thread to remove (`Clip::remove_note`).
//! The pass's parameter locks are erased the same way
//! ([`Player::erased_locks`]), trigless ones too, so the new take does not
//! play the old take's locks.
//!
//! **Live erase** (ERASE held on a pad while the transport plays, the
//! MPC's way; sequencer roadmap S1, and while only playing since the
//! button combos of 19 Nov). While the transport records or plays, the
//! armed track's notes of the sounds [`Player::set_erasing`] names are
//! erased as the playhead reaches them, on every pass, with that sound's
//! locks; they go out through [`Player::erased`] and
//! [`Player::erased_locks`] like a replace pass's. While recording they
//! are in the take being recorded; while only playing, each hold of
//! ERASE is a take of its own, numbered from [`ERASE_TAKES`], so one undo
//! brings back what it erased.
//!
//! **Chance.** A note with a chance below 100 % rolls each time it comes
//! round. The roll is a hash of the note and the absolute frame it would
//! play on, so it is the same for any period size, yet differs on every
//! pass, transport loops included. While the dice are anchored
//! ([`Player::anchor_dice`], an export's replays) the frame counts from
//! the one the transport last started on, so playing the same range again
//! rolls the same.
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
//! the ones from its bar line on; a muted track drops its own. A ratchet's
//! velocity ramp (`Note::ramp`) steps each later hit's velocity evenly
//! to the last hit's (`mpfree_seq::ramped`).
//!
//! **Steps that are off, ties and slides** (sequencer S6). A step that is
//! off (`Note::on` false) is left out like a note its chance left out,
//! with its locks, but it sets no last result for PRE and NEI. A tie or a
//! slide (`mpfree_seq::Join`) that plays looks, before anything on its
//! tick ends, at its track's notes started before its tick that are still
//! sounding or end on it: a note of its own pitch goes on to the tie's end
//! with no new attack, and the locks it holds stay (a slide onto the same
//! pitch does the same); with a slide, the other notes end one tick after
//! the slide's note-on, so the two overlap and a synth in mono or legato
//! mode glides. A tie or a slide with nothing to join plays as any note.
//!
//! **Cues** (see [`crate::view`]). A track's waiting cue starts on the
//! first bar line the playhead plays: the clip so far plays the ticks
//! before it, its notes still sounding end on it, and the cued clip plays
//! from it. [`Player::started`] names each switch for the control thread.
//! A switch on an armed track ends its replace pass (the take goes on as
//! overdub there).
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
//! **Pressure** (`mpfree_seq::Touch`). A note that carries recorded pad
//! pressure sends its values as [`Event::Pressure`] while it plays: each
//! value on the song tick its time from the note's start lands on (the
//! note's start as played, after quantise and swing, plus the value's
//! clip ticks at the clip's speed, rounded to a tick), so the pressure
//! moves with the note. Values stop at the note's end (the whole note, a
//! ratchet's too) or wherever it is cut; a new note on the same sound and
//! pitch ends the old one's. A stop, a locate or a loop jump ends them all
//! with the notes; a switch ends the track's on its bar line. On a tick,
//! a value comes before the note-offs there. The raw value goes out:
//! each instrument shapes it with its own pressure curve.
//!
//! **No allocation, locks or panics** in `process`: fixed tables only. Full
//! tables are counted in [`Stats`], never silently dropped.

use std::ops::Range;

use mpfree_param::ParamAddr;
use mpfree_seq::mpfree_moves::Out as MoveOut;
use mpfree_seq::{
    ClipProgram, Cond, DestId, Direction, Join, Lock, Note, Speed, Touch, MAX_TRACKS,
};
use mpfree_transport::time::{Swing, Tick};
use mpfree_transport::{Block, Mark, Marker, Span, State};

use crate::steps::{first_pass, Steps, HALF, STEP};
use crate::view::{ClipView, MuteLanded, NowPlaying, PlayView, Started, Switches, TrackView};

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
/// Notes sending their recorded pressure at once, all tracks.
pub const MAX_TOUCHING: usize = 64;
/// FILL changes in one period.
const MAX_FILL_CHANGES: usize = 8;
/// `dest` and `track` of an event meant for every destination.
pub const ALL: u16 = u16::MAX;
/// The track number of the song's own lanes' events (the morph's, in song
/// mode): past every track.
pub const SONG_TRACK: u16 = MAX_TRACKS as u16;

/// The first take number of live erase while only playing: its takes are
/// numbered on their own from here (the top bit set), so they never join
/// a recorded take (numbered from 1 by record-ons).
pub const ERASE_TAKES: u32 = 1 << 31;

/// A parameter lock replace recording erased: remove it from clip `slot`
/// of `track`, as for [`Erased`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErasedLock {
    pub track: usize,
    pub slot: usize,
    pub lock: Lock,
    pub take: u32,
}

/// A note replace recording or live erase erased: remove it from clip
/// `slot` of `track`. `take` numbers the take that erased it, as in
/// [`crate::Taken`], or from [`ERASE_TAKES`] for live erase while only
/// playing.
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
    /// Pad pressure (polyphonic aftertouch) on a sounding note: the pad's
    /// raw value, 0 to 127, which each instrument's pressure curve shapes.
    /// Played from a note's recorded pressure ([`mpfree_seq::Touch`]), and
    /// what the recorder takes in from the pads.
    Pressure {
        note: u8,
        value: u8,
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
    /// A clip's program change as it starts (sequencer S5,
    /// [`mpfree_seq::Clip::program`]): after the clip before it ends its
    /// notes on that tick, before the clip's first note. A scene's
    /// ([`crate::SceneProgram`]) goes out the same way on its line, unless
    /// the clip playing then sends its own. For outside gear (MIDI Out
    /// sends bank select, then the program change); the pads take nothing
    /// from it.
    Program {
        program: u8,
        bank: Option<u16>,
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
    /// The event's note is a pitch, from a clip of notes
    /// ([`mpfree_seq::Holds::Notes`], the instrument redesign's step 5c),
    /// not a pad. False for a clip of pad hits, the live pads and every
    /// event that isn't a note's.
    pub notes: bool,
}

// Out fills the room tables' fixed lists (`room`): it took the kind of
// note at 40 bytes, and grows no further unnoticed.
const _: () = assert!(std::mem::size_of::<Out>() <= 40);

/// Where the player's events go: the host maps each destination to the
/// sampler, the arp, MIDI out ... Events arrive in frame order.
pub trait Sink {
    fn send(&mut self, out: Out);

    /// Whether the player may start holding a new parameter (a step's
    /// lock or a move's first value), whose unlock the sink must then
    /// take: asked just before that lock is sent. A sink with a fixed
    /// table answers by its [`crate::room::Room`], so no unlock is ever
    /// lost; refused, the player leaves the lock out and counts it in
    /// [`Stats::busy`]. Every other sink takes them all.
    fn hold(&mut self) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Notes started.
    pub notes: u64,
    /// Notes (or a note's pressure) not started because a table was full.
    /// Should stay 0.
    pub dropped: u64,
    /// Note-offs that came after their tick (should stay 0).
    pub late_offs: u64,
    /// Notes and locks replace recording erased.
    pub erased: u64,
    /// Notes their chance left out.
    pub skipped: u64,
    /// Notes their trig condition left out (chance did not roll).
    pub unmet: u64,
    /// Steps that are off ([`Note::on`]), left out.
    pub off: u64,
    /// Ties (and slides onto the same pitch) that went on as the note
    /// before them, with no attack ([`Join`]).
    pub tied: u64,
    /// Parameter locks sent (each lock, not each unlock). Locks a full
    /// table left out count in `dropped`.
    pub locks: u64,
    /// Move values sent (`Event::Lane`).
    pub lanes: u64,
    /// Pressure values sent (`Event::Pressure`).
    pub pressure: u64,
    /// Locks and moves left out because the sink had no room to hold
    /// another parameter this period ([`Sink::hold`]): a moment too busy.
    pub busy: u64,
}

#[derive(Clone, Copy, Debug)]
struct Sounding {
    track: u16,
    dest: DestId,
    note: u8,
    /// Its note is a pitch (a clip of notes, [`Out::notes`]).
    notes: bool,
    off: Tick,
    /// Start order, to break ties.
    seq: u64,
    /// Its start's song tick: a tie or slide joins only notes from before
    /// its own tick.
    from: Tick,
}

#[derive(Clone, Copy, Debug)]
struct Start {
    tick: Tick,
    track: u16,
    dest: DestId,
    note: u8,
    /// Its note is a pitch (a clip of notes, [`Out::notes`]).
    notes: bool,
    velocity: u8,
    off: Tick,
    /// Percent; below 100 the note rolls with `key`.
    chance: u8,
    key: u64,
    cond: Cond,
    /// The clip's pass, counted from its start (pass 1).
    pass: i32,
    /// Hits, spread over `len` song ticks, their velocity ramping by
    /// `ramp` to the last.
    ratchet: u8,
    len: f32,
    ramp: i8,
    /// With its own attack, or tied or sliding into the note before it.
    join: Join,
    /// Not a note: the track's clip starts here, so its last result
    /// (for PRE and NEI) starts again at "not played".
    reset: bool,
    /// With `reset`, the program change the clip starting here sends;
    /// with `scene`, the scene's.
    program: Option<ClipProgram>,
    /// Not a note and no clip start: a scene's program change on its own
    /// (sequencer S5, [`crate::SceneProgram`]).
    scene: bool,
    /// Decided before the span plays: the note plays. A step that is off
    /// ([`Note::on`]) starts out false.
    plays: bool,
    /// Its recorded pressure in the clip in `slot` (at `speed`), sent
    /// until song tick `until` (the whole note's end).
    touch: Touch,
    slot: usize,
    speed: Speed,
    until: Tick,
}

/// A playing note's pressure: values `next..end` of the pressure of clip
/// `slot` of `track`, the next one on song tick `at`, until `until`.
#[derive(Clone, Copy, Debug)]
struct Touching {
    track: u16,
    dest: DestId,
    note: u8,
    notes: bool,
    slot: usize,
    /// The note's start as played, and its clip's speed.
    base: Tick,
    speed: Speed,
    next: u32,
    end: u32,
    at: Tick,
    until: Tick,
}

const NO_TOUCHING: Touching = Touching {
    track: 0,
    dest: 0,
    note: 0,
    notes: false,
    slot: 0,
    base: 0,
    speed: Speed::NORMAL,
    next: 0,
    end: 0,
    at: 0,
    until: 0,
};

/// A ratchet's later hit, waiting for its tick.
#[derive(Clone, Copy, Debug)]
struct Hit {
    tick: Tick,
    track: u16,
    dest: DestId,
    note: u8,
    notes: bool,
    velocity: u8,
    off: Tick,
}

const NO_HIT: Hit = Hit {
    tick: 0,
    track: 0,
    dest: 0,
    note: 0,
    notes: false,
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
    notes: false,
    off: 0,
    seq: 0,
    from: 0,
};
const NO_START: Start = Start {
    tick: 0,
    track: 0,
    dest: 0,
    note: 0,
    notes: false,
    velocity: 0,
    off: 0,
    chance: 100,
    key: 0,
    cond: Cond::Always,
    pass: 1,
    ratchet: 1,
    len: 0.0,
    ramp: 0,
    join: Join::Attack,
    reset: false,
    program: None,
    scene: false,
    plays: true,
    touch: Touch::NONE,
    slot: 0,
    speed: Speed::NORMAL,
    until: 0,
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
    /// Replace recording: the armed tracks in their pass being replaced
    /// (a bit each), and each one's pass in [`Player::passes`].
    replacing: u128,
    /// The pass of each armed track's clip being replaced, while its bit
    /// in `replacing` is set.
    passes: [i64; MAX_TRACKS],
    /// Live erase: the sounds (a bit each, by note) whose notes on the
    /// armed track are erased while the transport records or plays.
    erasing: u128,
    /// The armed tracks that have had their replace pass in this take
    /// (since the last record-on), a bit each.
    replaced: u128,
    /// Record-ons so far: the number of the latest take.
    take: u32,
    /// Live erase while only playing: the number of its latest take, from
    /// [`ERASE_TAKES`] (each hold of ERASE starts the next).
    erase_take: u32,
    /// Cues started that the view does not show yet.
    switches: Switches,
    started: [Started; MAX_TRACKS],
    n_started: usize,
    /// The mute cue each track carried out last (the mute queue, S4):
    /// what it is from then on, while the view still shows that cue.
    mute_landed: [Option<MuteLanded>; MAX_TRACKS],
    landed: [MuteLanded; MAX_TRACKS],
    n_landed: usize,
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
    /// The clip start each track sent its program change for (or found
    /// none to send): its slot and start, as `seen`. A stop or a jump
    /// forgets them, so the gear is set again when the music starts.
    programmed: [Option<(usize, Tick)>; MAX_TRACKS],
    /// The scene program change ([`crate::SceneProgram`]) each track
    /// landed: its launch's id. Its line passed, so after a stop it is
    /// sent where the music plays from.
    scene_landed: [Option<u32>; MAX_TRACKS],
    /// The scene program change each track sent (or left to the clip's
    /// own) since the music last started or jumped: its launch's id.
    scene_sent: [Option<u32>; MAX_TRACKS],
    fill: bool,
    /// [`Player::anchor_dice`]: chance rolls count frames from `dice_from`,
    /// the frame the transport last started on.
    anchored: bool,
    dice_from: u64,
    /// FILL changes due in the coming period: (offset, on), in offset order.
    fill_changes: [(u32, bool); MAX_FILL_CHANGES],
    n_fill_changes: usize,
    /// This span has a note left out (so some locks may be too).
    left_out: bool,
    /// Where each track's moves played up to, to tell a clip that goes on
    /// from one that starts, jumps or comes back (which first chases its
    /// moves' state).
    moves_pos: [Option<MovesPos>; MAX_TRACKS + 1],
    /// The song's own lanes played last span: when they stop (song mode
    /// off), their parameters are let go.
    song_lanes: bool,
    /// Notes sending their pressure.
    touching: [Touching; MAX_TOUCHING],
    n_touching: usize,
    /// The tracks it plays, a bit each (all, but in the look-ahead's
    /// copy: [`Player::only`]).
    only: u128,
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

/// What a clip's span erases: replace recording's pass, and live erase's
/// sounds (a bit each, by note).
#[derive(Clone, Copy, Debug)]
struct Erase {
    pass: Option<i64>,
    notes: u128,
    /// The take what it erases joins: the take recording, or live erase's
    /// own while only playing.
    take: u32,
}

/// What [`Player::collect`] plays, for each of its runs.
#[derive(Clone, Copy)]
struct Collect<'a> {
    clip: &'a ClipView,
    track: u16,
    dest: DestId,
    swing: Swing,
    ticks: &'a Range<Tick>,
    erase: Erase,
    sound: bool,
    /// The track's pads left out ([`crate::view`]'s `TrackView::pads`):
    /// a hit on one isn't played (a clip of notes plays every note).
    pads: u128,
    cut: Option<Tick>,
}

/// A run of a clip's ticks that one pass plays in a span: from `from` to
/// `to` (both in) and short of `end`, each tick played `shift` clip ticks
/// later in the pass. Going forward, the whole pass; with a play
/// direction (S6), a step.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// The pass, counted from the clip's origin.
    pass: i64,
    /// The song tick the pass starts on.
    start: f64,
    from: f64,
    to: f64,
    end: f64,
    shift: f64,
}

/// Pass numbers for the trig conditions: from 1 at the pass `start` is
/// in, worked out when first asked.
struct Passes {
    origin: Tick,
    start: Tick,
    first: Option<i64>,
}

impl Passes {
    /// Pass `pass` (counted from the origin), counted from 1 at `start`'s.
    fn number(&mut self, clip: &ClipView, pass: i64) -> i32 {
        let (origin, start) = (self.origin, self.start);
        let first = *self
            .first
            .get_or_insert_with(|| first_pass(clip, origin, start));
        (pass - first + 1).clamp(-(1 << 30), 1 << 30) as i32
    }
}

const NO_ERASE: Erase = Erase {
    pass: None,
    notes: 0,
    take: 0,
};

impl Erase {
    fn is_none(self) -> bool {
        self.pass.is_none() && self.notes == 0
    }

    /// Whether live erase erases sound `note`.
    fn sound(self, note: u8) -> bool {
        1u128
            .checked_shl(u32::from(note))
            .is_some_and(|b| self.notes & b != 0)
    }
}

const NO_ERASED: Erased = Erased {
    track: 0,
    slot: 0,
    note: Note::new(0, 1, 0, 1),
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
            replacing: 0,
            passes: [0; MAX_TRACKS],
            replaced: 0,
            erasing: 0,
            take: 0,
            erase_take: ERASE_TAKES,
            switches: Switches::new(),
            started: [Started {
                mute: None,
                track: 0,
                slot: None,
                at: 0,
                origin: 0,
                from: 0,
            }; MAX_TRACKS],
            n_started: 0,
            mute_landed: [None; MAX_TRACKS],
            landed: [MuteLanded {
                track: 0,
                silent: false,
                at: 0,
                id: 0,
            }; MAX_TRACKS],
            n_landed: 0,
            lock_starts: [NO_LOCK_START; MAX_LOCK_STARTS],
            n_lock_starts: 0,
            held: [NO_HELD; MAX_LOCKED],
            n_held: 0,
            hits: [NO_HIT; MAX_HITS],
            n_hits: 0,
            last: [false; MAX_TRACKS],
            seen: [None; MAX_TRACKS],
            programmed: [None; MAX_TRACKS],
            scene_landed: [None; MAX_TRACKS],
            scene_sent: [None; MAX_TRACKS],
            fill: false,
            anchored: false,
            dice_from: 0,
            fill_changes: [(0, false); MAX_FILL_CHANGES],
            n_fill_changes: 0,
            left_out: false,
            moves_pos: [None; MAX_TRACKS + 1],
            song_lanes: false,
            touching: [NO_TOUCHING; MAX_TOUCHING],
            n_touching: 0,
            only: u128::MAX,
        }
    }

    /// Anchor the chance rolls (on) to the frame the transport next starts
    /// on, and each start after it, or let them follow the absolute frame
    /// again (off, as it starts). Anchored, every play of the same range
    /// from the same start rolls the same: an export's replays play each
    /// track alone with the notes the first pass played. Real-time safe.
    pub fn anchor_dice(&mut self, on: bool) {
        self.anchored = on;
        self.dice_from = 0;
    }

    /// Live erase: from the coming period, while the transport records or
    /// plays, erase the armed track's notes of these sounds (a bit each, by
    /// note: bit 3 for note 3) as the playhead reaches them, with their
    /// sound's locks; 0 erases nothing. Going from nothing to something
    /// starts the next live erase take, for what it erases while only
    /// playing. Real-time safe.
    pub fn set_erasing(&mut self, notes: u128) {
        if self.erasing == 0 && notes != 0 {
            self.erase_take = self.erase_take.wrapping_add(1) | ERASE_TAKES;
        }
        self.erasing = notes;
    }

    /// The sounds live erase erases (see [`Player::set_erasing`]).
    pub fn erasing(&self) -> u128 {
        self.erasing
    }

    /// Becomes what `src` is, for [`Player::process`]: the look-ahead's
    /// copy ([`crate::Ahead`]) each period. Its tables are fixed (no heap),
    /// and only their used part is copied (about 15 kB of the 140 kB),
    /// in place. Real-time safe.
    pub fn copy_from(&mut self, src: &Player) {
        // Every field by name, so a new one can't be left out.
        let Player {
            sounding,
            n_sounding,
            starts,
            n_starts,
            seq,
            stats,
            erased,
            n_erased,
            erased_locks,
            n_erased_locks,
            replacing,
            passes,
            erasing,
            replaced,
            take,
            erase_take,
            switches,
            started,
            n_started,
            mute_landed,
            landed,
            n_landed,
            lock_starts,
            n_lock_starts,
            held,
            n_held,
            hits,
            n_hits,
            last,
            seen,
            programmed,
            scene_landed,
            scene_sent,
            fill,
            anchored,
            dice_from,
            fill_changes,
            n_fill_changes,
            left_out,
            moves_pos,
            song_lanes,
            touching,
            n_touching,
            only,
        } = src;
        fn used<T: Copy>(to: &mut [T], from: &[T], n: usize) -> usize {
            let n = n.min(from.len());
            to[..n].copy_from_slice(&from[..n]);
            n
        }
        self.n_sounding = used(&mut self.sounding, sounding, *n_sounding);
        self.n_starts = used(&mut self.starts, starts, *n_starts);
        self.n_erased = used(&mut self.erased, erased, *n_erased);
        self.n_erased_locks = used(&mut self.erased_locks, erased_locks, *n_erased_locks);
        self.n_started = used(&mut self.started, started, *n_started);
        self.n_landed = used(&mut self.landed, landed, *n_landed);
        self.n_lock_starts = used(&mut self.lock_starts, lock_starts, *n_lock_starts);
        self.n_held = used(&mut self.held, held, *n_held);
        self.n_hits = used(&mut self.hits, hits, *n_hits);
        self.n_fill_changes = used(&mut self.fill_changes, fill_changes, *n_fill_changes);
        self.n_touching = used(&mut self.touching, touching, *n_touching);
        self.seq = *seq;
        self.stats = *stats;
        self.replacing = *replacing;
        // Only the passes in use.
        let mut bits = *replacing;
        while bits != 0 {
            let i = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            self.passes[i] = passes[i];
        }
        self.erasing = *erasing;
        self.replaced = *replaced;
        self.take = *take;
        self.erase_take = *erase_take;
        self.switches = *switches;
        self.mute_landed = *mute_landed;
        self.last = *last;
        self.seen = *seen;
        self.programmed = *programmed;
        self.scene_landed = *scene_landed;
        self.scene_sent = *scene_sent;
        self.fill = *fill;
        self.anchored = *anchored;
        self.dice_from = *dice_from;
        self.left_out = *left_out;
        self.moves_pos = *moves_pos;
        self.song_lanes = *song_lanes;
        self.only = *only;
    }

    /// Plays only the tracks whose bit is set in `tracks` (bit `i`:
    /// track `i`; all until this is called), for a copy that only some
    /// tracks' events matter from: the look-ahead's ([`crate::Ahead`]),
    /// which plays on ahead for the tracks on slow gear alone. Never for
    /// the real player: a track left out misses its notes' starts and
    /// ends.
    pub fn only(&mut self, tracks: u128) {
        self.only = tracks;
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

    /// The mute cues that landed in the last [`Player::process`] call, in
    /// the order they did. The host applies the changes queued with each
    /// `id` to the project (one queue's land together) and compiles a new
    /// view without them.
    pub fn mutes_landed(&self) -> &[MuteLanded] {
        &self.landed[..self.n_landed]
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

    /// What `track` plays at song tick `tick` as this player sees it: its
    /// clip (launched, or cued and started), else in song mode the
    /// placement at `tick`. None when it plays nothing. For a host that
    /// follows a track's loop, such as a frozen mixer strip. Never
    /// allocates.
    pub fn now_playing(&self, view: &PlayView, track: usize, tick: Tick) -> Option<NowPlaying> {
        let (clip, origin, start) = match self.switches.playing_from(view, track) {
            Some(p) => p,
            None if view.song => {
                let r = view.tracks.get(track)?.region_at(tick)?;
                (&r.clip, r.clip.origin, r.clip.start)
            }
            None => return None,
        };
        // A play direction (S6): ping-pong comes round the same every
        // 2n - 2 steps, a whole number of passes counted from the one its
        // launch started in; random never does.
        let (origin, song_len, repeats) = match clip.direction {
            Direction::PingPong if clip.steps >= 2 => {
                let n = i64::from(clip.steps);
                let turn = 2 * n - 2;
                let passes = turn / gcd(n, turn);
                let first = first_pass(clip, origin, start) as f64;
                let origin = origin + (first * clip.song_len).round() as Tick;
                (origin, passes as f64 * clip.song_len, true)
            }
            Direction::Random if clip.steps >= 2 => (origin, clip.song_len, false),
            _ => (origin, clip.song_len, true),
        };
        Some(NowPlaying {
            slot: clip.slot,
            origin,
            song_len,
            stamp: clip.stamp,
            muted: self.muted(view, track),
            repeats,
            holds_notes: clip.holds_notes,
        })
    }

    /// Play one period: the transport's `block` for it, the clips in `view`.
    pub fn process(&mut self, view: &PlayView, block: &Block, out: &mut impl Sink) {
        self.n_erased = 0;
        self.n_erased_locks = 0;
        self.n_started = 0;
        self.n_landed = 0;
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
        if let (Mark::Started { .. }, true) = (m.mark, self.anchored) {
            self.dice_from = m.frame;
        }
        match m.mark {
            Mark::RecordOn { .. } => {
                self.replacing = 0;
                self.replaced = 0;
                self.take = self.take.wrapping_add(1);
            }
            Mark::RecordOff { .. } => self.replacing = 0,
            _ => {}
        }
        if matches!(
            m.mark,
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Stopped { .. }
        ) {
            self.replacing = 0;
        }
        match m.mark {
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Stopped { .. } => {
                self.end_all(m.offset, m.frame, out);
                self.unlock_where(m.offset, m.frame, out, false, |_| true);
                self.moves_pos = [None; MAX_TRACKS + 1];
                // A loop jump keeps the last results, so a PRE chain
                // carries round the loop, and the programs sent: the clips
                // starting on its far side send their own.
                if !matches!(m.mark, Mark::Looped { .. }) {
                    self.last = [false; MAX_TRACKS];
                    self.programmed = [None; MAX_TRACKS];
                    self.scene_sent = [None; MAX_TRACKS];
                }
            }
            Mark::Panic => {
                self.end_all(m.offset, m.frame, out);
                self.unlock_where(m.offset, m.frame, out, false, |_| true);
                self.moves_pos = [None; MAX_TRACKS + 1];
                out.send(Out {
                    offset: m.offset,
                    frame: m.frame,
                    track: ALL,
                    dest: ALL,
                    event: Event::AllOff,
                    notes: false,
                });
            }
            _ => {}
        }
    }

    /// End every sounding note, in the order they started, and drop the
    /// ratchet hits to come.
    fn end_all(&mut self, offset: u32, frame: u64, out: &mut impl Sink) {
        self.n_hits = 0;
        self.n_touching = 0;
        for s in &self.sounding[..self.n_sounding] {
            out.send(Out {
                offset,
                frame,
                track: s.track,
                dest: s.dest,
                event: Event::NoteOff { note: s.note },
                notes: s.notes,
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
        for i in 0..view.tracks.len() {
            if (self.only >> i) & 1 == 0 {
                continue;
            }
            // A mute cue (the mute queue) splits the track's part of the
            // span on its line: the track as it was up to it, then as the
            // cue has it.
            match self.mute_due(view, i, &ticks, bar) {
                Some(m) => {
                    if m > ticks.start {
                        self.play_track(view, span, i, ticks.start..m);
                    }
                    self.land_mute(view, i, m);
                    self.play_track(view, span, i, m..ticks.end);
                }
                None => self.play_track(view, span, i, ticks.clone()),
            }
        }
        // The song's own lanes, after every track's.
        let song = SONG_TRACK as usize;
        match view.song_lanes.as_ref().filter(|_| view.song) {
            Some(lanes) => {
                self.collect_moves(lanes, 0, song, ALL, &ticks);
                self.song_lanes = true;
            }
            None if self.song_lanes => {
                self.song_lanes = false;
                if self.held[..self.n_held]
                    .iter()
                    .any(|h| h.track == SONG_TRACK)
                {
                    self.end_locks(ticks.start, SONG_TRACK, ALL);
                }
                self.forget_moves(song);
            }
            None => {}
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
            // A tie or a slide holds the note before it, before anything
            // on this tick can end it.
            let tied = !s.reset && !s.scene && s.plays && s.join != Join::Attack && self.join(&s);
            // Locks on or before this tick go first (after the note-offs
            // before them), so a note starts with its step's locks. A note
            // left out leaves out its own locks.
            while k < self.n_lock_starts && self.lock_starts[k].tick <= s.tick {
                let l = self.lock_starts[k];
                self.advance(view, l.tick, false, first, span, out);
                if self.lock_plays(&l, i) {
                    self.lock(l, span, out);
                }
                k += 1;
            }
            // Notes ending on or before this tick end first, and ratchet
            // hits on or before it play.
            self.advance(view, s.tick, true, first, span, out);
            // A clip starting here sends its program change (or a scene
            // its own), after the notes ending here and before its own.
            if let Some(p) = s.program {
                let (offset, frame) = place(span, s.tick);
                out.send(Out {
                    offset,
                    frame,
                    track: s.track,
                    dest: s.dest,
                    event: Event::Program {
                        program: p.program,
                        bank: p.bank,
                    },
                    notes: false,
                });
            }
            if s.reset || s.scene || !s.plays {
                continue;
            }
            // A tie goes on as the note it continues: no attack, and the
            // locks it holds stay.
            if tied {
                self.stats.tied += 1;
                continue;
            }
            if s.ratchet > 1 {
                self.ratchet(i);
            }
            let (offset, frame) = place(span, s.tick);
            if let Some(j) = self.find(s.track, s.dest, s.note, s.notes) {
                out.send(Out {
                    offset,
                    frame,
                    track: s.track,
                    dest: s.dest,
                    event: Event::NoteOff { note: s.note },
                    notes: s.notes,
                });
                self.remove(j);
            }
            // The old note's pressure ends with it.
            self.stop_touch(s.track, s.dest, s.note, s.notes);
            if self.n_sounding == MAX_ACTIVE {
                self.stats.dropped += 1;
                continue;
            }
            if !s.touch.is_none() {
                self.start_touch(view, &s);
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
                notes: s.notes,
            });
            self.sounding[self.n_sounding] = Sounding {
                track: s.track,
                dest: s.dest,
                note: s.note,
                notes: s.notes,
                off: s.off,
                seq: self.seq,
                from: s.tick,
            };
            self.n_sounding += 1;
            self.seq += 1;
            self.stats.notes += 1;
        }
        while k < self.n_lock_starts {
            let l = self.lock_starts[k];
            self.advance(view, l.tick, false, first, span, out);
            self.lock(l, span, out);
            k += 1;
        }
        self.advance(view, ticks.end - 1, true, first, span, out);
    }

    /// What track `i` erases in `span`: an armed track's replace pass (the
    /// armed one's or another's, [`PlayView::set_also`]), and the armed
    /// one's live erase while recording or playing; nothing for a track not
    /// armed.
    #[inline(never)]
    fn erase_of(&self, view: &PlayView, span: &Span, i: usize) -> Erase {
        let bit = 1u128.checked_shl(i as u32).unwrap_or(0);
        if view.armed_set() & bit == 0 {
            return NO_ERASE;
        }
        Erase {
            pass: (self.replacing & bit != 0).then(|| self.passes[i]),
            // Live erase is the armed track's alone.
            notes: if matches!(span.state, State::Recording | State::Playing)
                && view.armed == Some(i)
            {
                self.erasing
            } else {
                0
            },
            take: if span.state == State::Recording {
                self.take
            } else {
                self.erase_take
            },
        }
    }

    /// Track `i`'s part of a span, playing `ticks` (the whole span, or
    /// the part before or after a mute cue's line).
    fn play_track(&mut self, view: &PlayView, span: &Span, i: usize, ticks: Range<Tick>) {
        let track = &view.tracks[i];
        let bar = span.meter.bar_ticks();
        // A scene's mute switches with its cue, before the view shows it.
        let muted = self.muted(view, i);
        // What it erases, if armed (worked out apart: most tracks aren't).
        let erase = if view.armed == Some(i) || view.also != 0 {
            self.erase_of(view, span, i)
        } else {
            NO_ERASE
        };
        // A cue starting in this span: the clip so far plays up to it.
        let switch = self.switches.due(view, i, &ticks, bar);
        let before = switch.map_or(ticks.clone(), |at| ticks.start..at);
        let playing = self.switches.playing_from(view, i);
        // Song mode: a track with no clip launched plays its
        // placements.
        let song = view.song && playing.is_none();
        let placed = if song {
            track.region_at(ticks.start)
        } else {
            None
        };
        let region = placed.map(|r| (r.clip.slot, r.clip.start));
        // A clip started since the last span (a launch, not a cue):
        // the track's last result starts again.
        let now = playing.map(|(c, _, start)| (c.slot, start)).or(region);
        if let (Some(seen), Some(last)) = (self.seen.get_mut(i), self.last.get_mut(i)) {
            if *seen != now {
                *seen = now;
                *last = false;
            }
        }
        // The scene's program change (S5): on its line, or here once it
        // landed. The clip playing then (from a switch on or before it,
        // the cued one) sends its own instead, if it has one.
        if let Some(at) = self.scene_due(view, i, &ticks, bar) {
            let own = match switch {
                Some(s) if s <= at => track.cue.as_ref().and_then(|c| c.clip.as_ref()),
                _ if song => track.region_at(at).map(|r| &r.clip),
                _ => playing.map(|(c, _, _)| c),
            }
            .and_then(|c| c.program);
            self.land_scene(view, i, at, own.is_none(), track.dest);
        }
        // Its program change, once: on its start, or here when it
        // started before (the music starting, a jump into it).
        let clip = playing.map(|(c, _, _)| c).or(placed.map(|r| &r.clip));
        if let (Some(key), Some(c)) = (now, clip) {
            let at = key.1.clamp(ticks.start, ticks.end - 1);
            self.program_start(i, key, c.program, at, track.dest, false);
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
            self.cut_touch(i as u16, ticks.start);
            self.forget_moves(i);
        }
        if let Some((clip, origin, start)) = playing {
            if !(muted && erase.is_none()) {
                self.collect(
                    clip, origin, start, i as u16, track.dest, span.swing, &before, erase, !muted,
                    track.pads, switch,
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
        let Some(at) = switch else { return };
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
        self.cut_touch(i as u16, at);
        // The old clip's locks end on the switch, before the new one's.
        self.end_locks(at, i as u16, track.dest);
        // The new clip's last result starts at "not played", on the
        // switch, after the old clip's notes before it; its program change
        // goes there too.
        let playing = self.switches.playing_from(view, i);
        let key = playing.map(|(c, _, start)| (c.slot, start));
        match (key, playing) {
            (Some(key), Some((c, _, _))) => {
                self.program_start(i, key, c.program, at, track.dest, true);
            }
            _ => {
                self.push_start(Start {
                    tick: at,
                    track: i as u16,
                    reset: true,
                    ..NO_START
                });
                // Stopped: the next clip sends its program again.
                if let Some(p) = self.programmed.get_mut(i) {
                    *p = None;
                }
            }
        }
        if let Some(seen) = self.seen.get_mut(i) {
            *seen = key;
        }
        // A switch ends that track's replace pass.
        if let Some(bit) = 1u128.checked_shl(i as u32) {
            self.replacing &= !bit;
        }
        if let (Some((clip, origin, start)), false) = (playing, self.muted(view, i)) {
            self.collect(
                clip,
                origin,
                start,
                i as u16,
                track.dest,
                span.swing,
                &(at..ticks.end),
                NO_ERASE,
                true,
                track.pads,
                None,
            );
            if clip.moves.is_empty() {
                self.forget_moves(i);
            } else {
                self.collect_moves(clip, origin, i, track.dest, &(at..ticks.end));
            }
        } else if view.song && playing.is_none() && !self.muted(view, i) {
            // Stopped: the track goes back to the song.
            self.collect_song(track, i, &(at..ticks.end), None, span.swing);
        }
    }

    /// Whether track `i` is silent now: the latest of a cue's mute (a
    /// scene's) and a mute cue that landed, while the view doesn't show
    /// them yet; else as the view has it.
    fn muted(&self, view: &PlayView, i: usize) -> bool {
        let landed = self
            .mute_landed
            .get(i)
            .copied()
            .flatten()
            .filter(|l| view.mute_cue(i).is_some_and(|c| c.id == l.id));
        match (landed, self.switches.mute_switch(i)) {
            (Some(l), Some((m, at))) if at > l.at => m,
            (Some(l), _) => l.silent,
            (None, _) => self.switches.muted(view, i),
        }
    }

    /// Where the mute cue on track `i` lands in a span playing `ticks`, if
    /// it does: the first line of its launch quantise among them. A cue
    /// that landed already waits for the view to show it applied.
    fn mute_due(&self, view: &PlayView, i: usize, ticks: &Range<Tick>, bar: Tick) -> Option<Tick> {
        let c = view.mute_cue(i)?;
        if self
            .mute_landed
            .get(i)
            .copied()
            .flatten()
            .is_some_and(|l| l.id >= c.id)
        {
            return None;
        }
        let at = c.line.next(ticks.start, bar);
        (at < ticks.end).then_some(at)
    }

    /// Carry out the mute cue on track `i` at song tick `at`: going
    /// silent, its notes end there and its locks, ratchets and pressure go,
    /// as for a cue's switch; heard again, it plays from there on.
    fn land_mute(&mut self, view: &PlayView, i: usize, at: Tick) {
        let Some(c) = view.mute_cue(i) else {
            return;
        };
        let was = self.muted(view, i);
        let l = MuteLanded {
            track: i,
            silent: c.silent,
            at,
            id: c.id,
        };
        if let Some(m) = self.mute_landed.get_mut(i) {
            *m = Some(l);
        }
        if let Some(slot) = self.landed.get_mut(self.n_landed) {
            *slot = l;
            self.n_landed += 1;
        }
        if c.silent && !was {
            let t = i as u16;
            for s in &mut self.sounding[..self.n_sounding] {
                if s.track == t {
                    s.off = s.off.min(at);
                }
            }
            self.cut_hits(t, at);
            self.cut_touch(t, at);
            self.end_locks(at, t, view.tracks[i].dest);
            self.forget_moves(i);
        }
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
            if s.scene {
                continue;
            }
            if s.reset {
                if let Some(last) = self.last.get_mut(t) {
                    *last = false;
                }
                continue;
            }
            // A step that is off is left out with its locks, and decides
            // nothing for PRE and NEI.
            if !s.plays {
                self.stats.off += 1;
                self.left_out = true;
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
                    || mix(s.key ^ place(span, s.tick).1.wrapping_sub(self.dice_from)) % 100
                        < u64::from(s.chance));
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
            .filter(|s| {
                !s.reset && !s.scene && s.track == l.track && l.row.is_none_or(|r| r == s.note)
            })
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

    /// Where track `i`'s scene program change goes in a span playing
    /// `ticks`, if it does: once a launch, on the first line of its line
    /// among them (bars of `bar` ticks); once it landed (or with no line),
    /// on the span's first tick after a stop or a jump.
    fn scene_due(&self, view: &PlayView, i: usize, ticks: &Range<Tick>, bar: Tick) -> Option<Tick> {
        let p = view.scene_program(i)?;
        if self.scene_sent.get(i).copied().flatten() == Some(p.id) {
            return None;
        }
        let landed = self.scene_landed.get(i).copied().flatten() == Some(p.id);
        let at = match p.line {
            Some(line) if !landed => line.next(ticks.start, bar),
            _ => ticks.start,
        };
        (at < ticks.end).then_some(at)
    }

    /// Track `i`'s scene program change lands on song tick `at`: sent
    /// there with `send`, else left to the clip's own.
    fn land_scene(&mut self, view: &PlayView, i: usize, at: Tick, send: bool, dest: DestId) {
        let Some(p) = view.scene_program(i) else {
            return;
        };
        if let (Some(landed), Some(sent)) =
            (self.scene_landed.get_mut(i), self.scene_sent.get_mut(i))
        {
            *landed = Some(p.id);
            *sent = Some(p.id);
        }
        if send {
            self.push_start(Start {
                tick: at,
                track: i as u16,
                dest,
                program: Some(p.program),
                scene: true,
                plays: false,
                ..NO_START
            });
        }
    }

    /// Track `i`'s clip `key` (its slot and the tick it started on) starts
    /// on `tick`: its reset there (with `reset`, always; else only to carry
    /// a program change), and its `program` change the first time this
    /// start is seen.
    fn program_start(
        &mut self,
        i: usize,
        key: (usize, Tick),
        program: Option<ClipProgram>,
        tick: Tick,
        dest: DestId,
        reset: bool,
    ) {
        let program = match self.programmed.get_mut(i) {
            Some(p) if *p != Some(key) => {
                *p = Some(key);
                program
            }
            _ => None,
        };
        if reset || program.is_some() {
            self.push_start(Start {
                tick,
                track: i as u16,
                dest,
                reset: true,
                program,
                ..NO_START
            });
        }
    }

    /// Tie or slide `s` (a note that plays, [`Join`]) to the notes of its
    /// track sounding as it starts or ending on its tick, started before
    /// it (a ratchet's hit before its tick that is still waiting too): a
    /// note of its own pitch goes on to `s`'s end (true: `s` plays no
    /// attack of its own); with a slide, the rest end one tick after `s`
    /// starts, so the two overlap and a legato synth glides.
    fn join(&mut self, s: &Start) -> bool {
        let mut tied = false;
        // Notes of the other kind (pads against pitches) never join.
        let notes = self.sounding[..self.n_sounding]
            .iter_mut()
            .filter(|x| x.notes == s.notes)
            .map(|x| (x.track, x.dest, x.note, x.from, &mut x.off));
        // A ratchet's hits from before the tie's tick that this span hasn't
        // played yet count as sounding, as they would at a shorter period.
        let hits = self.hits[..self.n_hits]
            .iter_mut()
            .filter(|h| h.notes == s.notes)
            .map(|h| (h.track, h.dest, h.note, h.tick, &mut h.off));
        for (track, dest, note, from, off) in notes.chain(hits) {
            if track != s.track || dest != s.dest || *off < s.tick || from >= s.tick {
                continue;
            }
            if note == s.note {
                *off = (*off).max(s.until);
                tied = true;
            } else if s.join == Join::Slide {
                *off = s.tick + 1;
            }
        }
        tied
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
                notes: s.notes,
                velocity: mpfree_seq::ramped(s.velocity, s.ramp, h as u8, s.ratchet),
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
    #[allow(clippy::too_many_arguments)]
    fn advance(
        &mut self,
        view: &PlayView,
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
            self.end_until(view, h.tick, first, span, out);
            let (offset, frame) = place(span, h.tick);
            if let Some(j) = self.find(h.track, h.dest, h.note, h.notes) {
                out.send(Out {
                    offset,
                    frame,
                    track: h.track,
                    dest: h.dest,
                    event: Event::NoteOff { note: h.note },
                    notes: h.notes,
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
                notes: h.notes,
            });
            self.sounding[self.n_sounding] = Sounding {
                track: h.track,
                dest: h.dest,
                note: h.note,
                notes: h.notes,
                off: h.off,
                seq: self.seq,
                from: h.tick,
            };
            self.n_sounding += 1;
            self.seq += 1;
            self.stats.notes += 1;
        }
        self.end_until(view, tick, first, span, out);
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
            notes: false,
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
                    None if !out.hold() => {
                        self.stats.busy += 1;
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
                    None if !out.hold() => {
                        self.stats.busy += 1;
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
                        notes: false,
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
            if !f(&self.held[j]) {
                // Kept: moved down only past ones taken out.
                if kept != j {
                    self.held[kept] = self.held[j];
                }
                kept += 1;
                continue;
            }
            let mut h = self.held[j];
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
                        notes: false,
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
                    notes: false,
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

    /// Start or end the replace pass for this span: each armed track's
    /// over one pass of its own playing clip.
    fn replace_state(&mut self, view: &PlayView, span: &Span, ticks: &Range<Tick>) {
        if span.state != State::Recording || span.overdub {
            self.replacing = 0;
            return;
        }
        let mut bits = view.armed_set() & !self.replaced;
        while bits != 0 {
            let t = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            let clip = self.switches.playing(view, t);
            if let Some((clip, origin)) = clip.filter(|(c, o)| c.song_len > 0.0 && ticks.end > *o) {
                let from = (ticks.start - origin).max(0) as f64;
                self.passes[t] = (from / clip.song_len).floor() as i64;
                self.replacing |= 1 << t;
                self.replaced |= 1 << t;
            }
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
                let key = (r.clip.slot, r.clip.start);
                self.program_start(i, key, r.clip.program, r.clip.start, track.dest, true);
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
                NO_ERASE,
                true,
                track.pads,
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
    /// `erase`, the notes of its pass, and those of its sounds on any pass,
    /// are erased instead; without
    /// `sound`, the rest are not played. With `cut`, notes end there at the
    /// latest. Passes count from 1 at `start`, for the trig conditions. A
    /// clip with a play direction (S6) plays its steps in the order it
    /// picks ([`crate::steps`]).
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
        erase: Erase,
        sound: bool,
        pads: u128,
        cut: Option<Tick>,
    ) {
        if (clip.notes.is_empty() && clip.locks.is_empty())
            || clip.song_len <= 0.0
            || ticks.is_empty()
        {
            return;
        }
        let swing = clip.swing.unwrap_or(transport_swing);
        let c = Collect {
            clip,
            track,
            dest,
            swing,
            ticks,
            erase,
            sound,
            pads: if clip.holds_notes { 0 } else { pads },
            cut,
        };
        // The pass `start` is in, counted from the origin, worked out at
        // the first note: most spans have none.
        let mut passes = Passes {
            origin,
            start,
            first: None,
        };
        // Swing only delays notes, by at most this much; rounding moves them
        // by half a tick. Look a little wider and keep the exact matches.
        let delay = f64::from(swing.grid) * f64::from(swing.amount.saturating_sub(50)) / 50.0;
        let lo = ticks.start as f64 - delay - 1.0;
        let hi = ticks.end as f64 + 1.0;
        if let Some(steps) = Steps::of(clip, usize::from(track), origin, start) {
            return self.collect_steps(&c, &steps, &mut passes, lo, hi);
        }
        let origin = origin as f64;
        let first_pass = ((lo - origin) / clip.song_len).floor().max(0.0);
        let last_pass = ((hi - origin) / clip.song_len).floor();
        if last_pass < first_pass {
            return;
        }
        let mut pass = first_pass;
        while pass <= last_pass {
            let start = origin + pass * clip.song_len;
            self.collect_run::<false>(
                &c,
                &mut passes,
                Run {
                    pass: pass as i64,
                    start,
                    from: clip.speed.to_clip(lo - start),
                    to: clip.speed.to_clip(hi - start),
                    end: f64::INFINITY,
                    shift: 0.0,
                },
            );
            pass += 1.0;
        }
    }

    /// [`Player::collect`] for a clip with a play direction: the play
    /// steps whose notes can land between song ticks `lo` and `hi`, each
    /// playing the clip step its direction picks.
    fn collect_steps(&mut self, c: &Collect, steps: &Steps, passes: &mut Passes, lo: f64, hi: f64) {
        let clip = c.clip;
        let origin = passes.origin as f64;
        let len = steps.len();
        // A play step's notes play within half a step of its line.
        let x_lo = clip.speed.to_clip(lo - origin);
        let x_hi = clip.speed.to_clip(hi - origin);
        let mut big = (((x_lo - HALF) / STEP).floor() as i64 + 1).max(0);
        let last = ((x_hi + HALF) / STEP).floor() as i64;
        while big <= last {
            let pass = steps.pass(big);
            let start = origin + pass as f64 * clip.song_len;
            let from = clip.speed.to_clip(lo - start);
            let to = clip.speed.to_clip(hi - start);
            // Nothing before the origin, as going forward.
            let from = from.max(-(pass as f64) * len);
            let k_line = f64::from(steps.pick(big)) * STEP;
            let shift = steps.line(big) - k_line;
            // The clip step's ticks around its line: for the first step,
            // from its line, and the clip's last half step before it.
            self.collect_run::<true>(
                c,
                passes,
                Run {
                    pass,
                    start,
                    from: (from - shift).max(k_line - HALF).max(0.0),
                    to: to - shift,
                    end: k_line + HALF,
                    shift,
                },
            );
            if k_line == 0.0 {
                let shift = shift - len;
                self.collect_run::<true>(
                    c,
                    passes,
                    Run {
                        pass,
                        start,
                        from: (from - shift).max(len - HALF),
                        to: to - shift,
                        end: len,
                        shift,
                    },
                );
            }
            big += 1;
        }
    }

    /// The notes and locks of one run of a clip's ticks ([`Run`]) whose
    /// tick is in the span. `STEPPED`: a run of a play direction's step,
    /// which ends short of `r.end` and plays `r.shift` later; forward, a
    /// whole pass, which needs neither (it is the hot path of every clip
    /// playing, so it keeps the cost it had before play directions:
    /// mpfree-bench's Time cases).
    #[inline(always)]
    fn collect_run<const STEPPED: bool>(&mut self, c: &Collect, passes: &mut Passes, r: Run) {
        let Collect {
            clip,
            track,
            dest,
            swing,
            ticks,
            erase,
            sound,
            pads,
            cut,
        } = *c;
        if r.to < r.from {
            return;
        }
        let first = clip.notes.partition_point(|n| n.local < r.from);
        for n in &clip.notes[first..] {
            if n.local > r.to || (STEPPED && n.local >= r.end) {
                break;
            }
            let local = if STEPPED { n.local + r.shift } else { n.local };
            let straight = r.start + clip.speed.to_song(local);
            let tick = swing.apply(straight).round() as Tick;
            if !ticks.contains(&tick) {
                continue;
            }
            if erase.pass == Some(r.pass) || erase.sound(n.note) {
                if self.n_erased == MAX_ERASED {
                    self.stats.dropped += 1;
                } else {
                    self.erased[self.n_erased] = Erased {
                        track: usize::from(track),
                        slot: clip.slot,
                        note: n.src,
                        take: erase.take,
                    };
                    self.n_erased += 1;
                    self.stats.erased += 1;
                }
                continue;
            }
            if !sound {
                continue;
            }
            // A pad muted on this track (step 10): its hits are left out.
            if pads != 0 && pads >> (n.note & 127) & 1 == 1 {
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
            // The whole note's end (a ratchet's last hit's too), for its
            // pressure.
            let whole = (swing.apply(straight + n.len).round() as Tick).max(tick + 1);
            let until = cut.map_or(whole, |c| whole.min(c));
            self.push_start(Start {
                tick,
                track,
                dest,
                note: n.note,
                notes: clip.holds_notes,
                velocity: n.velocity,
                off,
                chance: n.src.chance,
                key,
                cond: n.src.cond,
                pass: passes.number(clip, r.pass),
                ratchet,
                len: n.len as f32,
                ramp: n.src.ramp,
                join: n.src.join,
                reset: false,
                program: None,
                scene: false,
                plays: n.src.on,
                touch: n.src.touch,
                slot: clip.slot,
                speed: clip.speed,
                until,
            });
        }
        // The run's locks, placed as its notes are. A pass being replaced
        // erases them, and live erase its sounds' own; a muted track sends
        // none.
        if !(sound || erase.pass == Some(r.pass) || erase.notes != 0) {
            return;
        }
        let first = clip.locks.partition_point(|l| l.local < r.from);
        for l in &clip.locks[first..] {
            if l.local > r.to || (STEPPED && l.local >= r.end) {
                break;
            }
            let local = if STEPPED { l.local + r.shift } else { l.local };
            let straight = r.start + clip.speed.to_song(local);
            let tick = swing.apply(straight).round() as Tick;
            if !ticks.contains(&tick) {
                continue;
            }
            if erase.pass == Some(r.pass) || l.src.sound.is_some_and(|s| erase.sound(s)) {
                if self.n_erased_locks == MAX_ERASED {
                    self.stats.dropped += 1;
                } else {
                    self.erased_locks[self.n_erased_locks] = ErasedLock {
                        track: usize::from(track),
                        slot: clip.slot,
                        lock: l.src,
                        take: erase.take,
                    };
                    self.n_erased_locks += 1;
                    self.stats.erased += 1;
                }
            } else if sound {
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

    /// End the sounding notes whose end tick is at or before `tick`, in
    /// tick order (ties in start order).
    fn end_until(
        &mut self,
        view: &PlayView,
        tick: Tick,
        first: Tick,
        span: &Span,
        out: &mut impl Sink,
    ) {
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
            // A pressure value due first (on a tick, before the note-offs
            // there) goes first.
            let mut touch: Option<usize> = None;
            for (i, t) in self.touching[..self.n_touching].iter().enumerate() {
                if t.at <= tick
                    && best.is_none_or(|b| t.at <= self.sounding[b].off)
                    && touch.is_none_or(|k| t.at < self.touching[k].at)
                {
                    touch = Some(i);
                }
            }
            if let Some(k) = touch {
                self.send_touch(view, k, first, span, out);
                continue;
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
                notes: s.notes,
            });
            self.remove(i);
        }
    }

    /// Start sending note `s`'s pressure (its note-on just went out).
    fn start_touch(&mut self, view: &PlayView, s: &Start) {
        let points = view.touch_of(usize::from(s.track), s.slot);
        let Some(p) = points.get(s.touch.range()).and_then(|r| r.first()) else {
            return;
        };
        if self.n_touching == MAX_TOUCHING {
            self.stats.dropped += 1;
            return;
        }
        let t = Touching {
            track: s.track,
            dest: s.dest,
            note: s.note,
            notes: s.notes,
            slot: s.slot,
            base: s.tick,
            speed: s.speed,
            next: s.touch.first,
            end: s.touch.first + s.touch.len,
            at: s.tick + s.speed.to_song(f64::from(p.at)).round() as Tick,
            until: s.until,
        };
        if t.at < t.until {
            self.touching[self.n_touching] = t;
            self.n_touching += 1;
        }
    }

    /// Send pressure value `touching[k]` is due to send, and step it on (or
    /// drop it when it has no more before its note's end).
    fn send_touch(
        &mut self,
        view: &PlayView,
        k: usize,
        first: Tick,
        span: &Span,
        out: &mut impl Sink,
    ) {
        let t = self.touching[k];
        let points = view.touch_of(usize::from(t.track), t.slot);
        let Some(p) = points.get(t.next as usize) else {
            self.drop_touch(k);
            return;
        };
        let (offset, frame) = place(span, t.at.max(first));
        out.send(Out {
            offset,
            frame,
            track: t.track,
            dest: t.dest,
            event: Event::Pressure {
                note: t.note,
                value: p.value.min(127),
            },
            notes: t.notes,
        });
        self.stats.pressure += 1;
        let next = t.next + 1;
        let at = points
            .get(next as usize)
            .filter(|_| next < t.end)
            .map(|q| t.base + t.speed.to_song(f64::from(q.at)).round() as Tick);
        match at {
            // Values on one tick: the last of them is what counts, but each
            // goes out so the order is kept.
            Some(at) if at < t.until => {
                self.touching[k].next = next;
                self.touching[k].at = at.max(t.at);
            }
            _ => self.drop_touch(k),
        }
    }

    fn drop_touch(&mut self, k: usize) {
        self.touching.copy_within(k + 1..self.n_touching, k);
        self.n_touching -= 1;
    }

    /// A note on `track`, `dest` and `note` (a pitch with `notes`)
    /// starts: the pressure of the one before it ends.
    fn stop_touch(&mut self, track: u16, dest: DestId, note: u8, notes: bool) {
        if let Some(k) = self.touching[..self.n_touching]
            .iter()
            .position(|t| (t.track, t.dest, t.note, t.notes) == (track, dest, note, notes))
        {
            self.drop_touch(k);
        }
    }

    /// `track`'s notes send no pressure from `at` on.
    fn cut_touch(&mut self, track: u16, at: Tick) {
        for t in &mut self.touching[..self.n_touching] {
            if t.track == track {
                t.until = t.until.min(at);
            }
        }
        let mut kept = 0;
        for k in 0..self.n_touching {
            let t = self.touching[k];
            if t.at < t.until {
                self.touching[kept] = t;
                kept += 1;
            }
        }
        self.n_touching = kept;
    }

    /// Notes sending their recorded pressure now.
    pub fn touching(&self) -> usize {
        self.n_touching
    }

    /// The sounding note on `track`, `dest` and `note`, of its kind (a
    /// pitch with `notes`, else a pad).
    fn find(&self, track: u16, dest: DestId, note: u8, notes: bool) -> Option<usize> {
        self.sounding[..self.n_sounding]
            .iter()
            .position(|s| s.track == track && s.dest == dest && s.note == note && s.notes == notes)
    }

    /// Remove keeping the start order.
    fn remove(&mut self, i: usize) {
        self.sounding.copy_within(i + 1..self.n_sounding, i);
        self.n_sounding -= 1;
    }
}

/// The greatest common divisor of two positive numbers.
fn gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
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
