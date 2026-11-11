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

use mpfree_param::ParamAddr;
use mpfree_seq::{Lock, Note};
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
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    taken: Taken,
    /// Song tick from which the note may be handed over.
    release: Tick,
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
        }
    }

    /// A note for the coming [`Recorder::process`] call, at `offset` in the
    /// period: the armed track's pad notes, or its arp's output. They may
    /// come in any order; they are kept in offset order, and notes on the
    /// same offset keep the order they came in.
    pub fn input(&mut self, offset: u32, event: Event) {
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
        if view.armed != self.armed {
            self.held = [NOT_HELD; 128];
            self.armed = view.armed;
            self.n_recent = 0;
        }
        self.switches.follow(view);
        let markers = block.markers();
        let mut m = 0;
        let mut next = 0;
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
            while next < self.n_input && self.input[next].0 < end {
                let (offset, event) = self.input[next];
                next += 1;
                // Nothing is held outside recording: punch out, stop and
                // jumps end every note first.
                if span.state != State::Recording {
                    continue;
                }
                let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
                let tick = span.tick_at(frame).round() as Tick;
                if let Some(at) = switch.filter(|&at| tick >= at) {
                    self.switch(view, at);
                    switch = None;
                }
                self.release(tick);
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
                        self.held[n] = Held {
                            on: true,
                            tick,
                            velocity,
                        };
                    }
                    Event::AllOff => self.finish_all(view, span.swing, tick),
                    Event::Lock { addr, value } => self.lock(view, span.swing, addr, value, tick),
                    // Moves are recorded as gestures, not as locks.
                    Event::Lane { .. } | Event::Unlock { .. } => {}
                }
            }
            if let Some(at) = switch {
                self.switch(view, at);
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
        self.n_input = 0;
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
            }
            _ => {}
        }
    }

    /// The armed track's cue starts on song tick `at`: the held notes end
    /// there, in the clip that was playing.
    fn switch(&mut self, view: &PlayView, at: Tick) {
        self.finish_all(view, self.swing, at);
        // As in the player, a switch ends the replace pass.
        self.replacing = None;
        if let Some(track) = self.armed {
            self.switches.start(view, track, at);
        }
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
        let Some(track) = self.armed else { return };
        let Some((clip, origin)) = self.switches.playing(view, track) else {
            return;
        };
        let Some((tick, pass)) = clip_tick(clip, origin, h.tick) else {
            return;
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
            return;
        }
        let note = Note {
            tick,
            len,
            note: n as u8,
            velocity: h.velocity.min(127),
            chance: 100,
            cond: mpfree_seq::Cond::Always,
            ratchet: 1,
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
            if self.n_taken == MAX_PENDING {
                self.dropped += 1;
            } else {
                self.taken[self.n_taken] = self.pending[i].taken;
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
