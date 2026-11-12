//! The clip recorder (roadmap T3): notes played while the transport
//! records go into the armed track's playing clip, at the clip tick they
//! were played on.
//!
//! The control thread arms a track in the play view
//! ([`PlayView::set_armed`]). The audio thread feeds the [`Recorder`] each
//! period's pad notes for that track (or its arp's notes) with their
//! offsets, then calls [`Recorder::process`] with the transport's block
//! and the play view. A note is finished when its
//! pad is released, or when a punch out, a loop jump, a locate or a stop
//! ends it. It is handed over in [`Recorder::taken`] for the control thread
//! to add to the clip (`Clip::add_note`) and compile a new view. The notes
//! keep the exact tick they were played on; the clip's quantise applies on
//! playback, so it can change later.
//!
//! A note the playhead has not yet reached where it will play (quantise or
//! swing move it later) waits until the playhead passes that point, so the
//! player never plays it again straight after it was heard live. Every
//! tick comes from the span the note fell in, so the notes are the same,
//! in the same order, for any period size. No allocation: fixed tables.
//!
//! A cue that switches the armed track's clip (see [`crate::view`]) ends
//! the held notes on its bar line, into the clip that was playing; notes
//! played from that tick on go into the new clip. The recorder follows
//! cues the same way the player does, so the two agree on which clip
//! plays even before the control thread's new view arrives.
//!
//! **Knob moves become parameter locks.** While the transport records, the
//! host feeds the armed track's knob moves in as [`Event::Lock`] (the
//! parameter on the armed track, and its new value). Each lands on a step,
//! the Elektron way:
//!
//! - on the note being played: a pad held now, or else the note that
//!   started less than [`LOCK_REACH`] before the move (one the clip holds,
//!   or one just recorded), so the lock plays with that note (a sampler
//!   pad's parameter looks only at that pad's notes). Old notes a replace
//!   pass has erased don't count;
//! - otherwise on the nearest step line (the clip's quantise grid, or
//!   1/16), as a trigless lock.
//!
//! A recorded lock waits, like a note, until the playhead has passed where
//! it plays, so a replace pass never erases it. A turn sends many values:
//! a waiting lock takes each later value for its step and parameter. Then
//! [`Recorder::taken_locks`] hands it to the control thread to put in the
//! clip (`Takes::add_lock`), in place of a lock already there.
//!
//! **Perform FX gestures become moves.** The host feeds the same gestures
//! it gives `mpfree_moves::MoveLog` (the XY pad, the DJ knob, a Perform FX
//! pad's Hold) to [`Recorder::input_move`]. While the transport records,
//! each gesture is kept on the armed track's playing clip's own ticks, as
//! it goes on past the clip's end, and handed over when it ends
//! ([`Recorder::taken_moves`], points from [`Recorder::move_points`]) for
//! the control thread to write into the clip's lanes (`Takes::put_move`,
//! which replaces what the lane had over the ticks the gesture covers).
//! A parameter on the armed track gets a track-relative lane; any other
//! (the master's Perform FX) a fixed one. A punch out, a stop or a switch
//! ends the gesture there; a loop jump or a locate hands over what was
//! kept and goes on from the new place; a gesture longer than
//! [`MOVE_POINTS`] values is handed over in parts.
//!
//! **Pad pressure goes on its note.** The host feeds the armed track's pad
//! pressure in as [`Event::Pressure`] (the pad, its raw value) when the
//! track records pressure (`Track::record_touch`). While a pad's note is
//! held and the transport records, its values are thinned and kept on the
//! note, timed in clip ticks from the note's start (`TouchPoint::at`):
//!
//! - a value 2 or more steps ([`TOUCH_STEP`]) from the last one kept is
//!   kept at once;
//! - a value one step away is kept once it has lasted [`TOUCH_SETTLE_MS`],
//!   on the frame that time is up, so a slow drift is kept every 10 ms or
//!   so while a finger's one-step jitter is not.
//!
//! Every time comes from the frame the value came on, so what is kept is
//! the same for any period size. Each note's values wait in one of
//! [`TOUCH_BUFS`] buffers of [`TOUCH_POINTS`] values until the note is
//! handed over; [`Recorder::touch_points`] then gives them, named by the
//! taken note's `touch`, for the control thread to put in the clip
//! (`Takes::add_touched`). Values past a full buffer, or a note that finds
//! no free buffer, are counted ([`Recorder::touch_dropped`]), never
//! silently lost.

use mpfree_param::ParamAddr;
use mpfree_seq::mpfree_moves::{Input, Kind, Map, Target};
use mpfree_seq::{Lock, Note, Speed, Touch, TouchPoint};
use mpfree_transport::time::{Swing, Tick};
use mpfree_transport::{Block, Mark, Marker, Span, State};

use crate::player::Event;
use crate::view::{ClipView, PlayView, Switches};

/// Notes one period can bring to the recorder.
pub const MAX_INPUT: usize = 256;
/// Finished notes waiting for the playhead, and notes handed over in one
/// period.
pub const MAX_PENDING: usize = 256;
/// Recorded locks handed over in one period (after a turn's values are
/// joined).
pub const MAX_LOCKS_TAKEN: usize = 64;
/// How long after a note starts a knob move still locks it: a 1/16 note,
/// in the clip's ticks.
pub const LOCK_REACH: Tick = 240;
/// Notes the recorder remembers having recorded, for knob moves just
/// after them.
const RECENT: usize = 32;
/// Gestures one period can bring to the recorder.
pub const MAX_MOVE_INPUT: usize = 256;
/// Gestures being recorded at once.
pub const MAX_GESTURES: usize = 16;
/// Values a recorded gesture keeps before it is handed over in parts.
pub const MOVE_POINTS: usize = 1024;
/// Gestures handed over in one period.
pub const MAX_MOVES_TAKEN: usize = 32;
/// Room `PlayView::make_room` sets aside in each lane of the armed
/// track's clips: two parts of a gesture, and what a write adds.
pub const MOVE_ROOM: usize = 2 * MOVE_POINTS + 8;
/// Pressure values one period can bring (they have their own room, so
/// they never crowd out notes).
pub const MAX_TOUCH_INPUT: usize = 512;
/// Notes keeping pressure at once: held, or waiting for the playhead.
pub const TOUCH_BUFS: usize = 32;
/// Pressure values one note keeps; later ones are counted as dropped.
pub const TOUCH_POINTS: usize = 2048;
/// A pressure value this many steps from the last one kept is kept at once.
pub const TOUCH_STEP: u8 = 2;
/// A value one step from the last one kept is kept once it has lasted
/// this long.
pub const TOUCH_SETTLE_MS: u32 = 10;
/// Room `PlayView::make_room` and the host set aside for pressure in the
/// armed track's clips.
pub const TOUCH_ROOM: usize = 8192;
/// No buffer.
const NO_BUF: u8 = u8::MAX;

/// A recorded gesture for the control thread: write its points
/// ([`Recorder::move_points`]) into the lane for `target` of clip `slot`
/// of `track` (`Moves::put`, made with `map` if new), a clip `len` ticks
/// long. The points are clip ticks counted on from the pass the gesture
/// began in. `take` numbers the take, as in [`Taken`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TakenMove {
    pub track: usize,
    pub slot: usize,
    pub target: Target,
    pub map: Map,
    pub len: u32,
    pub take: u32,
    from: usize,
    n: usize,
}

/// A gesture being recorded.
#[derive(Clone, Copy, Debug)]
struct Gesture {
    addr: ParamAddr,
    map: Map,
    /// The clip it goes into, as the recorder found it playing.
    track: usize,
    slot: usize,
    origin: Tick,
    take: u32,
    /// Song tick of the first value, and its clip tick (in the first
    /// pass, 0 to the length).
    song0: f64,
    clip0: f64,
    /// The clip's length and speed.
    len: u32,
    speed: Speed,
    /// Values kept in this part, in `points[k * MOVE_POINTS..]`. 0 after a
    /// part was handed over: the next value starts the next part.
    n: usize,
}

/// A recorded parameter lock for the control thread: put it in clip `slot`
/// of `track`, in place of a lock of the same parameter on the same tick
/// and row. `take` numbers the take, as in [`Taken`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakenLock {
    pub track: usize,
    pub slot: usize,
    pub lock: Lock,
    pub take: u32,
}

/// A recorded note for the control thread: add it to clip `slot` of
/// `track`. Its tick and length are in the clip's own ticks. `take` numbers
/// the take it was played in (see [`Recorder::take`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Taken {
    pub track: usize,
    pub slot: usize,
    pub note: Note,
    pub take: u32,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    on: bool,
    /// Song tick of the press.
    tick: Tick,
    velocity: u8,
    /// The speed of the clip it went into, for its pressure's clip ticks.
    speed: Speed,
    /// Its pressure buffer, or [`NO_BUF`] before the first value kept.
    buf: u8,
    /// The last pressure value kept (0 when pressed).
    kept: u8,
    /// A value one step from `kept` waiting to last [`TOUCH_SETTLE_MS`],
    /// and the frame it came on.
    wait: Option<(u8, u64)>,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    taken: Taken,
    /// Song tick from which the note may be handed over.
    release: Tick,
    /// Its pressure buffer, or [`NO_BUF`].
    buf: u8,
}

/// A recorded lock waiting for the playhead, as a [`Pending`] note does.
#[derive(Clone, Copy, Debug)]
struct PendingLock {
    taken: TakenLock,
    release: Tick,
}

const NOT_HELD: Held = Held {
    on: false,
    tick: 0,
    velocity: 0,
    speed: Speed::NORMAL,
    buf: NO_BUF,
    kept: 0,
    wait: None,
};
const NO_TAKEN_LOCK: TakenLock = TakenLock {
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

const NO_TAKE: Taken = Taken {
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
        touch: mpfree_seq::Touch::NONE,
    },
    take: 0,
};

pub struct Recorder {
    /// The armed track in the last view, to notice a change.
    armed: Option<usize>,
    input: [(u32, Event); MAX_INPUT],
    n_input: usize,
    held: [Held; 128],
    pending: [Pending; MAX_PENDING],
    n_pending: usize,
    taken: [Taken; MAX_PENDING],
    n_taken: usize,
    /// The transport's swing in the last span, for notes a marker ends.
    swing: Swing,
    take: u32,
    dropped: u64,
    /// Cues started that the view does not show yet.
    switches: Switches,
    locks: [TakenLock; MAX_LOCKS_TAKEN],
    n_locks: usize,
    /// Recorded locks waiting for the playhead; a turn's later values join
    /// the waiting lock.
    pending_locks: [PendingLock; MAX_LOCKS_TAKEN],
    n_pending_locks: usize,
    /// Knob moves on another track, or on no track: not recorded.
    stray_locks: u64,
    /// The latest notes this take recorded, a ring: knob moves just after
    /// them lock them, whether or not the clip holds them yet.
    recent: [Taken; RECENT],
    n_recent: usize,
    /// The replace pass, as the player keeps it: the old notes it has
    /// reached are erased, so knob moves do not lock them.
    replacing: Option<i64>,
    replaced: bool,
    move_input: [(u32, Input); MAX_MOVE_INPUT],
    n_move_input: usize,
    gestures: [Option<Gesture>; MAX_GESTURES],
    /// Each gesture's values: (clip tick, knob position, kind).
    points: Vec<(f64, f32, Kind)>,
    taken_moves: [Option<TakenMove>; MAX_MOVES_TAKEN],
    n_taken_moves: usize,
    /// The points handed over this period.
    taken_points: Vec<(f64, f32, Kind)>,
    /// Pressure values waiting for the coming period, in offset order:
    /// (offset, pad, value).
    touch_input: [(u32, u8, u8); MAX_TOUCH_INPUT],
    n_touch_input: usize,
    /// The notes' pressure buffers: buffer `k` is
    /// `touch[k * TOUCH_POINTS..][..touch_len[k]]`.
    touch: Vec<TouchPoint>,
    touch_len: [u16; TOUCH_BUFS],
    /// Buffers in use (a bit each), and buffers handed over this period,
    /// free from the next (their values stay readable until then).
    bufs_used: u32,
    bufs_freeing: u32,
    touch_dropped: u64,
}

impl Default for Recorder {
    fn default() -> Self {
        Recorder::new()
    }
}

impl Recorder {
    pub fn new() -> Recorder {
        Recorder {
            armed: None,
            input: [(0, Event::AllOff); MAX_INPUT],
            n_input: 0,
            held: [NOT_HELD; 128],
            pending: [Pending {
                taken: NO_TAKE,
                release: 0,
                buf: NO_BUF,
            }; MAX_PENDING],
            n_pending: 0,
            taken: [NO_TAKE; MAX_PENDING],
            n_taken: 0,
            swing: Swing::default(),
            take: 0,
            dropped: 0,
            switches: Switches::new(),
            locks: [NO_TAKEN_LOCK; MAX_LOCKS_TAKEN],
            n_locks: 0,
            pending_locks: [PendingLock {
                taken: NO_TAKEN_LOCK,
                release: 0,
            }; MAX_LOCKS_TAKEN],
            n_pending_locks: 0,
            stray_locks: 0,
            recent: [NO_TAKE; RECENT],
            n_recent: 0,
            replacing: None,
            replaced: false,
            move_input: [(
                0,
                Input::End {
                    addr: ParamAddr::macro_value(0),
                },
            ); MAX_MOVE_INPUT],
            n_move_input: 0,
            gestures: [None; MAX_GESTURES],
            points: vec![(0.0, 0.0, Kind::End); MAX_GESTURES * MOVE_POINTS],
            taken_moves: [None; MAX_MOVES_TAKEN],
            n_taken_moves: 0,
            taken_points: Vec::with_capacity(MAX_MOVES_TAKEN * MOVE_POINTS),
            touch_input: [(0, 0, 0); MAX_TOUCH_INPUT],
            n_touch_input: 0,
            touch: vec![TouchPoint::default(); TOUCH_BUFS * TOUCH_POINTS],
            touch_len: [0; TOUCH_BUFS],
            bufs_used: 0,
            bufs_freeing: 0,
            touch_dropped: 0,
        }
    }

    /// A note for the coming [`Recorder::process`] call, at `offset` in the
    /// period: the armed track's pad notes, or its arp's output. They may
    /// come in any order; they are kept in offset order, and notes on the
    /// same offset keep the order they came in.
    ///
    /// [`Event::Pressure`] (a pad's raw pressure) goes in its own queue of
    /// [`MAX_TOUCH_INPUT`], so a flood of pressure never crowds out notes;
    /// a value on the same offset as a note comes after it.
    pub fn input(&mut self, offset: u32, event: Event) {
        if let Event::Pressure { note, value } = event {
            if self.n_touch_input == MAX_TOUCH_INPUT {
                self.touch_dropped += 1;
                return;
            }
            let mut i = self.n_touch_input;
            while i > 0 && self.touch_input[i - 1].0 > offset {
                self.touch_input[i] = self.touch_input[i - 1];
                i -= 1;
            }
            self.touch_input[i] = (offset, note, value);
            self.n_touch_input += 1;
            return;
        }
        if self.n_input == MAX_INPUT {
            self.dropped += 1;
            return;
        }
        let mut i = self.n_input;
        while i > 0 && self.input[i - 1].0 > offset {
            self.input[i] = self.input[i - 1];
            i -= 1;
        }
        self.input[i] = (offset, event);
        self.n_input += 1;
    }

    /// A Perform FX gesture for the coming [`Recorder::process`] call, at
    /// `offset` in the period: the same moves the host gives the move log.
    /// They may come in any order; they are kept in offset order.
    pub fn input_move(&mut self, offset: u32, input: Input) {
        if self.n_move_input == MAX_MOVE_INPUT {
            self.dropped += 1;
            return;
        }
        let mut i = self.n_move_input;
        while i > 0 && self.move_input[i - 1].0 > offset {
            self.move_input[i] = self.move_input[i - 1];
            i -= 1;
        }
        self.move_input[i] = (offset, input);
        self.n_move_input += 1;
    }

    /// The gestures (or parts of them) handed over in the last
    /// [`Recorder::process`] call, in order.
    pub fn taken_moves(&self) -> impl Iterator<Item = &TakenMove> + '_ {
        self.taken_moves[..self.n_taken_moves].iter().flatten()
    }

    /// A handed-over gesture's points, for `Moves::put`.
    pub fn move_points(&self, k: &TakenMove) -> &[(f64, f32, Kind)] {
        self.taken_points.get(k.from..k.from + k.n).unwrap_or(&[])
    }

    /// Gestures being recorded now.
    pub fn moving(&self) -> usize {
        self.gestures.iter().flatten().count()
    }

    /// The notes finished and handed over in the last [`Recorder::process`]
    /// call, in order.
    pub fn taken(&self) -> &[Taken] {
        &self.taken[..self.n_taken]
    }

    /// The parameter locks recorded in the last [`Recorder::process`]
    /// call, a turn's last value for each step and parameter.
    pub fn taken_locks(&self) -> &[TakenLock] {
        &self.locks[..self.n_locks]
    }

    /// Knob moves not recorded because they named a parameter that is not
    /// on the armed track.
    pub fn stray_locks(&self) -> u64 {
        self.stray_locks
    }

    /// Notes or locks lost because a table was full. It should stay 0.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Pressure values lost: past a full queue or a note's full buffer, or
    /// on a note that found no free buffer. It should stay 0.
    pub fn touch_dropped(&self) -> u64 {
        self.touch_dropped
    }

    /// The pressure kept on a note handed over in [`Recorder::taken`] this
    /// period, named by its `touch` (empty for none). The values stay
    /// readable until the next [`Recorder::process`] call.
    pub fn touch_points(&self, note: &Note) -> &[TouchPoint] {
        self.touch.get(note.touch.range()).unwrap_or(&[])
    }

    /// The number of the latest take: each record-on starts the next one,
    /// from 1. The player counts the same record-ons, so a take's recorded
    /// notes and the notes it erased carry the same number.
    pub fn take(&self) -> u32 {
        self.take
    }

    /// Notes pressed now and being recorded.
    pub fn holding(&self) -> usize {
        self.held.iter().filter(|h| h.on).count()
    }

    /// Record the period described by `block`, with `view` playing, into
    /// the view's armed track. Notes held while the arm changes are not
    /// recorded.
    pub fn process(&mut self, view: &PlayView, block: &Block) {
        self.n_taken = 0;
        self.n_locks = 0;
        self.n_taken_moves = 0;
        self.taken_points.clear();
        // Buffers handed over last period are read by now.
        self.bufs_used &= !std::mem::take(&mut self.bufs_freeing);
        if view.armed != self.armed {
            for n in 0..128 {
                let b = self.held[n].buf;
                self.free_buf(b);
            }
            self.held = [NOT_HELD; 128];
            // What the old arm's gestures kept goes in; they record no more.
            for k in 0..MAX_GESTURES {
                self.hand_over(k);
                self.gestures[k] = None;
            }
            self.armed = view.armed;
            self.n_recent = 0;
        }
        self.switches.follow(view);
        let markers = block.markers();
        let mut m = 0;
        let mut next = 0;
        let mut next_move = 0;
        let mut next_touch = 0;
        for span in block.spans() {
            // A marker on a span's first frame happened just before it.
            while m < markers.len() && markers[m].offset <= span.offset {
                self.marker(view, &markers[m]);
                m += 1;
            }
            self.swing = span.swing;
            self.replace_state(view, span);
            // Cues starting in this span, as the player starts them. The
            // armed track's waits for the notes played before it.
            let mut switch = None;
            if matches!(span.state, State::Playing | State::Recording) {
                let ticks = span.ticks();
                let bar = span.meter.bar_ticks();
                for i in 0..view.tracks.len() {
                    match self.switches.due(view, i, &ticks, bar) {
                        Some(at) if Some(i) == self.armed => switch = Some(at),
                        Some(at) => {
                            self.switches.start(view, i, at);
                        }
                        None => {}
                    }
                }
            }
            let end = span.offset + span.len;
            loop {
                let note = (next < self.n_input)
                    .then(|| self.input[next].0)
                    .filter(|&o| o < end);
                let gesture = (next_move < self.n_move_input)
                    .then(|| self.move_input[next_move].0)
                    .filter(|&o| o < end);
                // Pressure after the notes and moves on its offset.
                let touch = (next_touch < self.n_touch_input)
                    .then(|| self.touch_input[next_touch].0)
                    .filter(|&o| o < end);
                if let Some(t) = touch {
                    if note.is_none_or(|a| t < a) && gesture.is_none_or(|b| t < b) {
                        let (offset, pad, value) = self.touch_input[next_touch];
                        next_touch += 1;
                        self.touch_value(span, offset, pad, value);
                        continue;
                    }
                }
                match (note, gesture) {
                    (None, None) => break,
                    (Some(a), Some(b)) if b < a => {}
                    (None, Some(_)) => {}
                    _ => {
                        let (offset, event) = self.input[next];
                        next += 1;
                        self.note_input(view, span, offset, event, &mut switch);
                        continue;
                    }
                }
                let (offset, input) = self.move_input[next_move];
                next_move += 1;
                if span.state != State::Recording {
                    // A gesture ending while not recording is forgotten.
                    if let Input::End { addr } = input {
                        if let Some(k) = self.gesture_of(addr) {
                            self.gestures[k] = None;
                        }
                    }
                    continue;
                }
                let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
                let song = span.tick_at(frame);
                if let Some(at) = switch.filter(|&at| half_up(song) >= at) {
                    self.switch(view, at);
                    switch = None;
                }
                self.gesture(view, input, song);
            }
            if let Some(at) = switch {
                self.switch(view, at);
            }
            // One-step values whose time is up within this span.
            if span.state == State::Recording {
                self.settle_all(span, span.frame + u64::from(span.len));
            }
            // The last tick the next span can start a note on is
            // `end - 1`, so this releases nothing an input there would not.
            if span.state.rolling() {
                self.release(span.ticks().end - 1);
            }
        }
        for marker in &markers[m..] {
            self.marker(view, marker);
        }
        // Offsets past the period's end are the host's mistake.
        self.dropped += (self.n_input - next) as u64;
        self.dropped += (self.n_move_input - next_move) as u64;
        self.touch_dropped += (self.n_touch_input - next_touch) as u64;
        self.n_input = 0;
        self.n_move_input = 0;
        self.n_touch_input = 0;
    }

    /// Pad `pad`'s pressure `value` at `offset` in `span`: thinned onto its
    /// held note while recording (see the module notes).
    fn touch_value(&mut self, span: &Span, offset: u32, pad: u8, value: u8) {
        if span.state != State::Recording {
            return;
        }
        let n = usize::from(pad & 127);
        if !self.held[n].on {
            return;
        }
        let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
        // A one-step value whose time came before this one is kept first.
        self.settle(n, span, frame + 1);
        let v = value.min(127);
        let h = &mut self.held[n];
        if v.abs_diff(h.kept) >= TOUCH_STEP {
            h.wait = None;
            self.keep(n, span, frame, v);
        } else if v == h.kept {
            h.wait = None;
        } else if h.wait.is_none_or(|(w, _)| w != v) {
            h.wait = Some((v, frame));
        }
    }

    /// Frames a one-step value must last.
    fn settle_frames(span: &Span) -> u64 {
        u64::from(span.rate) * u64::from(TOUCH_SETTLE_MS) / 1000
    }

    /// Keep held note `n`'s waiting one-step value if its time is up before
    /// frame `before` (its time falls in `span`).
    fn settle(&mut self, n: usize, span: &Span, before: u64) {
        let Some((v, from)) = self.held[n].wait else {
            return;
        };
        let due = from + Self::settle_frames(span);
        if due < before {
            self.held[n].wait = None;
            self.keep(n, span, due.max(span.frame), v);
        }
    }

    fn settle_all(&mut self, span: &Span, before: u64) {
        for n in 0..128 {
            if self.held[n].on && self.held[n].wait.is_some() {
                self.settle(n, span, before);
            }
        }
    }

    /// Keep `value` on held note `n`, on `frame` of `span`.
    fn keep(&mut self, n: usize, span: &Span, frame: u64, value: u8) {
        let h = self.held[n];
        self.held[n].kept = value;
        let mut buf = h.buf;
        if buf == NO_BUF {
            match (0..TOUCH_BUFS).find(|&k| self.bufs_used & (1 << k) == 0) {
                Some(k) => {
                    self.bufs_used |= 1 << k;
                    self.touch_len[k] = 0;
                    buf = k as u8;
                    self.held[n].buf = buf;
                }
                None => {
                    self.touch_dropped += 1;
                    return;
                }
            }
        }
        let k = usize::from(buf);
        let len = usize::from(self.touch_len[k]);
        if len == TOUCH_POINTS {
            self.touch_dropped += 1;
            return;
        }
        let song = span.tick_at(frame).round() as Tick;
        let at = h.speed.to_clip((song - h.tick).max(0) as f64).round() as u32;
        // In time order, also across a tempo change's rounding.
        let at = if len > 0 {
            at.max(self.touch[k * TOUCH_POINTS + len - 1].at)
        } else {
            at
        };
        self.touch[k * TOUCH_POINTS + len] = TouchPoint { at, value };
        self.touch_len[k] += 1;
    }

    /// Buffer `b` is free again (at once: nothing reads it).
    fn free_buf(&mut self, b: u8) {
        if usize::from(b) < TOUCH_BUFS {
            self.bufs_used &= !(1 << b);
        }
    }

    /// One note or knob move at `offset` in `span`, after the armed
    /// track's `switch` when that is due at or before it.
    fn note_input(
        &mut self,
        view: &PlayView,
        span: &Span,
        offset: u32,
        event: Event,
        switch: &mut Option<Tick>,
    ) {
        // Nothing is held outside recording: punch out, stop and jumps end
        // every note first.
        if span.state != State::Recording {
            return;
        }
        let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
        let tick = span.tick_at(frame).round() as Tick;
        if let Some(at) = switch.filter(|&at| tick >= at) {
            self.switch(view, at);
            *switch = None;
        }
        self.release(tick);
        // Pressure whose time came before this note event is kept first.
        match event {
            Event::NoteOn { note, .. } | Event::NoteOff { note } => {
                self.settle(usize::from(note & 127), span, frame + 1);
            }
            Event::AllOff => self.settle_all(span, frame + 1),
            _ => {}
        }
        match event {
            // A MIDI-style note-on at velocity 0 is a release.
            Event::NoteOn { note, velocity: 0 } | Event::NoteOff { note } => {
                let n = usize::from(note & 127);
                if self.held[n].on {
                    self.finish(view, span.swing, n, tick);
                }
            }
            Event::NoteOn { note, velocity } => {
                let n = usize::from(note & 127);
                if self.held[n].on {
                    self.finish(view, span.swing, n, tick);
                }
                let speed = self
                    .armed
                    .and_then(|t| self.switches.playing(view, t))
                    .map_or(Speed::NORMAL, |(c, _)| c.speed);
                self.held[n] = Held {
                    on: true,
                    tick,
                    velocity,
                    speed,
                    ..NOT_HELD
                };
            }
            Event::AllOff => self.finish_all(view, span.swing, tick),
            Event::Lock { addr, value } => self.lock(view, span.swing, addr, value, tick),
            // Moves are recorded as gestures, not as locks; pressure has
            // its own queue.
            Event::Lane { .. } | Event::Unlock { .. } | Event::Pressure { .. } => {}
        }
    }

    fn marker(&mut self, view: &PlayView, m: &Marker) {
        match m.mark {
            Mark::RecordOn { .. } => {
                self.take = self.take.wrapping_add(1);
                self.replacing = None;
                self.replaced = false;
            }
            // The playhead goes on: notes wait for it as before.
            Mark::RecordOff { tick } => {
                self.finish_all(view, self.swing, tick);
                self.end_gestures(view, tick as f64, true);
                self.replacing = None;
            }
            // The playhead leaves: nothing recorded here will be reached
            // in this pass, so everything goes now.
            Mark::Looped { end: tick, .. }
            | Mark::Located { from: tick, .. }
            | Mark::Stopped { tick } => {
                self.finish_all(view, self.swing, tick);
                self.release(Tick::MAX);
                self.replacing = None;
                // A stop ends the gestures; after a jump they go on from
                // where the playhead lands.
                self.end_gestures(view, tick as f64, matches!(m.mark, Mark::Stopped { .. }));
            }
            _ => {}
        }
    }

    /// The armed track's cue starts on song tick `at`: the held notes end
    /// there, in the clip that was playing.
    fn switch(&mut self, view: &PlayView, at: Tick) {
        self.finish_all(view, self.swing, at);
        self.end_gestures(view, at as f64, true);
        // As in the player, a switch ends the replace pass.
        self.replacing = None;
        if let Some(track) = self.armed {
            self.switches.start(view, track, at);
        }
    }

    /// The gesture on `addr`, if one is being recorded.
    fn gesture_of(&self, addr: ParamAddr) -> Option<usize> {
        self.gestures
            .iter()
            .position(|g| g.is_some_and(|g| g.addr == addr))
    }

    /// A Perform FX move at song tick `song` while recording: kept on the
    /// armed track's playing clip's ticks.
    fn gesture(&mut self, view: &PlayView, input: Input, song: f64) {
        let Some(track) = self.armed else { return };
        let Some((clip, origin)) = self.switches.playing(view, track) else {
            return;
        };
        if clip.song_len <= 0.0 || song < origin as f64 {
            return;
        }
        let (addr, value) = match input {
            Input::Move { addr, value, map } => (addr, Some((value, map))),
            Input::End { addr } => (addr, None),
        };
        let found = self.gesture_of(addr);
        if found.is_none() && value.is_none() {
            return;
        }
        let Some(k) = found.or_else(|| self.gestures.iter().position(Option::is_none)) else {
            self.dropped += 1;
            return;
        };
        // A new part starts with a new gesture, after a jump or a handed
        // over part, or when the clip under it changed.
        let fresh =
            self.gestures[k].is_none_or(|g| g.n == 0 || g.slot != clip.slot || g.origin != origin);
        if fresh {
            self.hand_over(k);
            let map = value
                .map(|v| v.1)
                .or(self.gestures[k].map(|g| g.map))
                .unwrap_or(Map::linear(0.0, 1.0));
            let len = clip.length as f64;
            self.gestures[k] = Some(Gesture {
                addr,
                map,
                track,
                slot: clip.slot,
                origin,
                take: self.take,
                song0: song,
                clip0: clip
                    .speed
                    .to_clip((song - origin as f64).rem_euclid(clip.song_len))
                    .clamp(0.0, (len - 1.0).max(0.0)),
                len: clip.length as u32,
                speed: clip.speed,
                n: 0,
            });
        }
        let Some(g) = self.gestures[k].as_mut() else {
            return;
        };
        let at = g.clip0 + g.speed.to_clip(song - g.song0);
        let point = match value {
            Some((v, map)) => {
                g.map = map;
                let kind = if map.stepped() {
                    Kind::Step
                } else {
                    Kind::Ramp
                };
                (at, map.unit(v), kind)
            }
            None => (at, 0.0, Kind::End),
        };
        self.points[k * MOVE_POINTS + g.n] = point;
        g.n += 1;
        if value.is_none() {
            self.hand_over(k);
            self.gestures[k] = None;
        } else if g.n == MOVE_POINTS {
            self.hand_over(k);
        }
    }

    /// Every gesture's part so far goes to the control thread. With
    /// `end`, they end on song tick `song` (a punch out, a stop or a
    /// switch); otherwise each goes on in a new part (a jump).
    fn end_gestures(&mut self, view: &PlayView, song: f64, end: bool) {
        for k in 0..MAX_GESTURES {
            let Some(g) = self.gestures[k] else {
                continue;
            };
            if !end {
                self.hand_over(k);
                continue;
            }
            if g.n == MOVE_POINTS {
                self.hand_over(k);
            }
            let g = self.gestures[k].as_mut().expect("still there");
            if g.n == 0 {
                // Nothing since the last part: the end goes in on its own,
                // where the clip it was moving is now.
                let here = self
                    .switches
                    .playing(view, g.track)
                    .filter(|(c, o)| c.slot == g.slot && *o == g.origin && c.song_len > 0.0);
                if let Some((c, o)) = here {
                    g.song0 = song;
                    g.clip0 = c
                        .speed
                        .to_clip((song - o as f64).rem_euclid(c.song_len))
                        .clamp(0.0, (f64::from(g.len) - 1.0).max(0.0));
                } else {
                    self.gestures[k] = None;
                    continue;
                }
            }
            let at = g.clip0 + g.speed.to_clip(song - g.song0);
            let last = (g.n > 0).then(|| self.points[k * MOVE_POINTS + g.n - 1].0);
            self.points[k * MOVE_POINTS + g.n] = (at.max(last.unwrap_or(at)), 0.0, Kind::End);
            g.n += 1;
            self.hand_over(k);
            self.gestures[k] = None;
        }
    }

    /// Hand gesture `k`'s values so far to the control thread, leaving it
    /// to start a new part with its next value.
    fn hand_over(&mut self, k: usize) {
        let Some(g) = self.gestures[k].as_mut() else {
            return;
        };
        if g.n == 0 {
            return;
        }
        let n = std::mem::take(&mut g.n);
        let from = self.taken_points.len();
        if self.n_taken_moves == MAX_MOVES_TAKEN || from + n > self.taken_points.capacity() {
            self.dropped += 1;
            return;
        }
        self.taken_points
            .extend_from_slice(&self.points[k * MOVE_POINTS..k * MOVE_POINTS + n]);
        self.taken_moves[self.n_taken_moves] = Some(TakenMove {
            track: g.track,
            slot: g.slot,
            target: Target::for_track(g.addr, g.track as u8),
            map: g.map,
            len: g.len,
            take: g.take,
            from,
            n,
        });
        self.n_taken_moves += 1;
    }

    /// Start or end the replace pass for this span, as the player does.
    fn replace_state(&mut self, view: &PlayView, span: &Span) {
        let ticks = span.ticks();
        if ticks.is_empty() {
            return;
        }
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

    /// A knob move at song tick `song`: a lock on the step it belongs to,
    /// waiting like a note until the playhead has passed where it plays.
    fn lock(&mut self, view: &PlayView, swing: Swing, addr: ParamAddr, value: f32, song: Tick) {
        let Some(track) = self.armed else { return };
        let rel = match addr.owner.split() {
            Some((t, on)) if usize::from(t) == track => mpfree_param::RelAddr::new(on, addr.param),
            _ => {
                self.stray_locks += 1;
                return;
            }
        };
        if !value.is_finite() {
            return;
        }
        let Some((clip, origin)) = self.switches.playing(view, track) else {
            return;
        };
        let Some((at, pass)) = clip_tick(clip, origin, song) else {
            return;
        };
        let scope = Lock::scope(rel);
        let mine = |n: u8| scope.is_none_or(|p| p == n);
        let since = |from: Tick| (at - from).rem_euclid(clip.length);
        // The pass a clip tick at or just before the move falls in.
        let pass_of = |t: Tick| pass - i64::from(t > at);
        // A pad held now: the lock goes on its note.
        let held = (0..128u8)
            .filter(|&n| self.held[usize::from(n)].on && mine(n))
            .filter_map(|n| clip_tick(clip, origin, self.held[usize::from(n)].tick))
            .filter(|&(t, _)| since(t) < LOCK_REACH)
            .min_by_key(|&(t, _)| since(t));
        // Else the note that started last, not long ago, by the tick it
        // was played on: one in the clip (but not an old one the replace
        // pass has erased), or one this take recorded, which the clip may
        // not hold yet.
        let erased = |t: Tick| self.replacing == Some(pass_of(t));
        let played = || {
            let recent = self.recent[..self.n_recent.min(RECENT)]
                .iter()
                .filter(|k| (k.track, k.slot) == (track, clip.slot))
                .map(|k| k.note);
            clip.notes
                .iter()
                .map(|n| n.src)
                .filter(|n| !erased(n.tick))
                .chain(recent)
                .filter(|n| mine(n.note) && since(n.tick) < LOCK_REACH)
                .min_by_key(|n| since(n.tick))
                .map(|n| (n.tick, pass_of(n.tick)))
        };
        let (tick, tick_pass) = held.or_else(played).unwrap_or_else(|| {
            // Trigless, on the nearest step line.
            let grid = clip
                .quantise
                .map_or(LOCK_REACH, |q| Tick::from(q.grid))
                .max(1);
            let line = (at + grid / 2) / grid * grid;
            (
                line.rem_euclid(clip.length),
                pass + line.div_euclid(clip.length),
            )
        });
        // Where the player will play it in that pass, as for a note.
        let q = clip.quantise.map_or(tick as f64, |q| q.apply(tick as f64));
        let straight = origin as f64 + tick_pass as f64 * clip.song_len + clip.speed.to_song(q);
        let plays = clip.swing.unwrap_or(swing).apply(straight).round() as Tick;
        let same = |k: &PendingLock| {
            (
                k.taken.track,
                k.taken.slot,
                k.taken.lock.tick,
                k.taken.lock.sound,
            ) == (track, clip.slot, tick, scope)
                && k.taken.lock.addr == rel
        };
        let pending = &mut self.pending_locks[..self.n_pending_locks];
        if let Some(k) = pending.iter_mut().find(|k| same(k)) {
            k.taken.lock.value = value;
            return;
        }
        if self.n_pending_locks == MAX_LOCKS_TAKEN {
            self.dropped += 1;
            return;
        }
        self.pending_locks[self.n_pending_locks] = PendingLock {
            taken: TakenLock {
                track,
                slot: clip.slot,
                lock: Lock::new(tick, scope, rel, value),
                take: self.take,
            },
            release: plays + 2,
        };
        self.n_pending_locks += 1;
    }

    fn finish_all(&mut self, view: &PlayView, swing: Swing, tick: Tick) {
        for n in 0..128 {
            if self.held[n].on {
                self.finish(view, swing, n, tick);
            }
        }
    }

    /// End held note `n` at song tick `end` and queue it.
    fn finish(&mut self, view: &PlayView, transport_swing: Swing, n: usize, end: Tick) {
        let h = self.held[n];
        self.held[n].on = false;
        self.held[n].buf = NO_BUF;
        self.held[n].wait = None;
        // A note not queued lets its pressure go.
        let Some(track) = self.armed else {
            return self.free_buf(h.buf);
        };
        let Some((clip, origin)) = self.switches.playing(view, track) else {
            return self.free_buf(h.buf);
        };
        let Some((tick, pass)) = clip_tick(clip, origin, h.tick) else {
            return self.free_buf(h.buf);
        };
        let len = (clip.speed.to_clip((end - h.tick).max(1) as f64).round() as Tick)
            .clamp(1, clip.length);
        // Where the player will play it in this pass.
        let at = clip.quantise.map_or(tick as f64, |q| q.apply(tick as f64));
        let straight = origin as f64 + pass as f64 * clip.song_len + clip.speed.to_song(at);
        let plays = clip
            .swing
            .unwrap_or(transport_swing)
            .apply(straight)
            .round() as Tick;
        if self.n_pending == MAX_PENDING {
            self.dropped += 1;
            return self.free_buf(h.buf);
        }
        let note = Note {
            tick,
            len,
            note: n as u8,
            velocity: h.velocity.min(127),
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
            touch: mpfree_seq::Touch::NONE,
        };
        let taken = Taken {
            track,
            slot: clip.slot,
            note,
            take: self.take,
        };
        self.recent[self.n_recent % RECENT] = taken;
        self.n_recent += 1;
        // A tick of margin for rounding.
        self.pending[self.n_pending] = Pending {
            taken,
            release: plays + 2,
            buf: h.buf,
        };
        self.n_pending += 1;
    }

    /// Hand over the waiting notes and locks due at or before song tick
    /// `now`, by due tick, ties in the order they finished or were made.
    fn release(&mut self, now: Tick) {
        loop {
            let mut best: Option<usize> = None;
            for (i, p) in self.pending[..self.n_pending].iter().enumerate() {
                if p.release <= now && best.is_none_or(|b| p.release < self.pending[b].release) {
                    best = Some(i);
                }
            }
            let Some(i) = best else { break };
            let p = self.pending[i];
            if self.n_taken == MAX_PENDING {
                self.dropped += 1;
                self.free_buf(p.buf);
            } else {
                let mut taken = p.taken;
                if usize::from(p.buf) < TOUCH_BUFS {
                    let k = usize::from(p.buf);
                    // Named in the buffers; free from the next period.
                    taken.note.touch = Touch {
                        first: (k * TOUCH_POINTS) as u32,
                        len: u32::from(self.touch_len[k]),
                    };
                    self.bufs_freeing |= 1 << k;
                }
                self.taken[self.n_taken] = taken;
                self.n_taken += 1;
            }
            self.pending.copy_within(i + 1..self.n_pending, i);
            self.n_pending -= 1;
        }
        loop {
            let mut best: Option<usize> = None;
            for (i, p) in self.pending_locks[..self.n_pending_locks]
                .iter()
                .enumerate()
            {
                if p.release <= now
                    && best.is_none_or(|b| p.release < self.pending_locks[b].release)
                {
                    best = Some(i);
                }
            }
            let Some(i) = best else { return };
            if self.n_locks == MAX_LOCKS_TAKEN {
                self.dropped += 1;
            } else {
                self.locks[self.n_locks] = self.pending_locks[i].taken;
                self.n_locks += 1;
            }
            self.pending_locks
                .copy_within(i + 1..self.n_pending_locks, i);
            self.n_pending_locks -= 1;
        }
    }
}

/// The clip tick of song tick `song`, and the pass it falls in, for the
/// clip playing from `origin`. None before the origin.
/// A song tick rounded half up, as the player rounds moves.
fn half_up(x: f64) -> Tick {
    (x + 0.5).floor() as Tick
}

fn clip_tick(clip: &ClipView, origin: Tick, song: Tick) -> Option<(Tick, i64)> {
    if song < origin || clip.song_len <= 0.0 {
        return None;
    }
    let from = (song - origin) as f64;
    let mut pass = (from / clip.song_len).floor() as i64;
    let mut tick = clip
        .speed
        .to_clip(from - pass as f64 * clip.song_len)
        .round() as Tick;
    if tick >= clip.length {
        tick -= clip.length;
        pass += 1;
    }
    Some((tick.max(0), pass))
}
