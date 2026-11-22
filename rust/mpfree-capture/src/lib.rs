//! CAPTURE (roadmap T3): keep what was just played, even though nothing
//! was recording.
//!
//! [`Capture`] keeps a history of every note the pads (or the arp in front
//! of them) play, whether the transport rolls or not. The audio thread
//! hands it each period's notes with their offsets ([`Capture::input`]),
//! then the transport's block ([`Capture::process`]). That never
//! allocates: the history is a fixed ring of [`MAX_HITS`] note starts and
//! ends, minutes of busy playing.
//!
//! Keys a MIDI route plays on a track that isn't armed go into the same
//! history, named by their track ([`Capture::input_on`]); the pads' notes
//! are the armed track's ([`ARMED`]). The phrase is everything played on
//! them all, and [`Phrase::split`] parts it by track, so each track's
//! notes go into a clip of its own, one take for them all
//! (`Takes::joining`).
//!
//! Every note is placed on a clock that never goes back. While the
//! transport rolls it counts song ticks, and it keeps the bar lines where
//! the song has them, so a loop of whole bars or a jump from bar line to
//! bar line goes on without a seam. While the transport is stopped it
//! counts ticks at the current tempo.
//!
//! The **phrase** is what was played since the last silence of at least a
//! bar ([`GAP_BARS`]): either all of it while the transport rolled (in
//! time) or all of it while it was stopped (free). [`Capture::pending`]
//! says, without allocating, how many bars CAPTURE would keep now.
//! [`Capture::phrase`] turns the phrase into clip notes (allocating, so
//! never on the audio thread):
//!
//! - In time, the clip starts on the bar line at or before the first note,
//!   and each note keeps its place in the bar and the song tick it was
//!   played on.
//! - Free, the clip starts on the first note.
//! - The clip ends on the bar line after the last note starts, so it is
//!   whole bars long, at most [`MAX_BARS`]: a longer phrase keeps its last
//!   bars.
//! - A note ends where its pad was released; a note still held ends now.
//! - A note keeps the pad pressure played on it ([`Phrase::touch`]),
//!   thinned as recording thins it (`mpfree_play::Recorder`): a value 2
//!   or more steps from the last one kept at once, a one-step value once
//!   it has lasted 10 ms. The history keeps [`MAX_TOUCH_HITS`] values
//!   apart from the notes, in their own ring, so pressure never pushes
//!   notes out; a phrase whose first notes' pressure is older than that
//!   says so ([`Phrase::touch_cut`]).
//!
//! [`keep`] puts a phrase into the project as a take of its own, so UNDO
//! takes it back:
//!
//! - Played in time over the clip that is playing on the track, its notes
//!   go into that clip, each on the clip tick it was played on, as if it
//!   had been recorded.
//! - Otherwise it becomes a new clip in the track's first empty slot. The
//!   host launches it with [`Kept::launch`]. A phrase played in time keeps
//!   its place against the song, so it goes on playing in time at once.
//!   A free phrase starts on the next bar line.
//! - The notes keep their pressure when the track records pressure
//!   (`Track::record_touch`). Pressure a playing clip has no room for is
//!   left off its notes and counted ([`Kept::touch_lost`]); the notes
//!   still go in.
//!
//! **Pads and notes** (step sequencer refactor E4). A clip holds one kind
//! ([`Holds`]): pads, or notes (after the pad layout, scale, chord and
//! octave, before the track's transpose). The history keeps each touch as
//! both where it has both: the pad, and the notes it played, each marked
//! [`Took::paired`]. A pad and a note of the same number are different
//! notes. [`keep`] takes one kind, as a take there would record: over the
//! playing clip, that clip's kind (as an overdub does); else the kind a
//! new clip on the track holds, which the host says. A touch kept only as
//! the other kind (a kit pad with no note, the arp's notes, which come
//! with no pad) is left out and counted ([`Kept::left_out`]), so a clip
//! of notes never gets a pad's number, nor a clip of pads a pitch.
//!
//! The Perform FX moves made over a phrase played in time go with it: the
//! host takes them from its `mpfree_moves::MoveLog` ([`MoveLog::phrase`])
//! and [`moves_for_clip`] puts them on the kept clip's ticks, for
//! `Takes::capture_moves` to write into its lanes as part of the same
//! capture.
//!
//! [`MoveLog::phrase`]: mpfree_seq::mpfree_moves::MoveLog::phrase
#![forbid(unsafe_code)]

use std::collections::HashMap;

use mpfree_play::record::{TOUCH_SETTLE_MS, TOUCH_STEP};
use mpfree_play::{Event, Launch, MoveWrite, Takes};
use mpfree_seq::mpfree_moves::{Gesture, Target};
use mpfree_seq::{
    Clip, Holds, ModelError, Note, Project, Touch, TouchPoint, MAX_CLIP_TICKS, MAX_NOTES,
    MAX_TOUCH_POINTS, SLOTS,
};
use mpfree_transport::time::{ceil_to, ticks_in, Meter, Tick};
use mpfree_transport::{Block, Span};

/// Note starts and ends kept in the history: 8192 notes, about 8 minutes
/// of 1/16 notes on two pads at 120 BPM.
pub const MAX_HITS: usize = 16384;
/// Notes one period can bring.
pub const MAX_INPUT: usize = 256;
/// Pad pressure values the history keeps, after thinning: about 30 s of
/// all 16 pads pressed and moving at once, far more of ordinary playing.
pub const MAX_TOUCH_HITS: usize = 32768;
/// Pressure values one period can bring (their own room, so they never
/// crowd out notes).
pub const MAX_TOUCH_INPUT: usize = 512;
// A whole phrase's pressure always fits in a new clip.
const _: () = assert!(MAX_TOUCH_HITS <= MAX_TOUCH_POINTS);
/// A silence of this many bars ends a phrase.
pub const GAP_BARS: i64 = 1;
/// The longest phrase CAPTURE keeps, in bars; a longer one keeps its last
/// bars.
pub const MAX_BARS: i64 = 32;
/// The armed track's notes (the pads, and the arp in front of them): they
/// go into whichever track is armed when CAPTURE is pressed.
pub const ARMED: u8 = u8::MAX;
/// Tracks whose notes can be held at once: the armed track's and 31 a
/// route plays. A note on another track while they all hold notes is
/// left out ([`Capture::dropped`]).
pub const MAX_VOICES: usize = 32;

/// What a note given to the history is (refactor E4): a pad or a note,
/// and whether the same touch is given as the other kind too (a pad and
/// the notes it played), so [`keep`] can take either kind without losing
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Took {
    pub holds: Holds,
    /// The same touch is in the history as the other kind too.
    pub paired: bool,
}

impl Took {
    /// A pad alone.
    pub const PAD: Took = Took {
        holds: Holds::Hits,
        paired: false,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    On,
    Off,
    AllOff,
}

/// One note start or end in the history.
#[derive(Clone, Copy, Debug)]
struct Hit {
    /// On the capture clock, in ticks.
    at: f64,
    /// The song tick, while the transport rolled.
    song: f64,
    kind: Kind,
    /// [`ARMED`], or the track a route played it on.
    track: u8,
    /// A pad or a note.
    holds: Holds,
    /// Its touch is in the history as the other kind too.
    paired: bool,
    note: u8,
    velocity: u8,
}

const NO_HIT: Hit = Hit {
    at: 0.0,
    song: 0.0,
    kind: Kind::AllOff,
    track: ARMED,
    holds: Holds::Hits,
    paired: false,
    note: 0,
    velocity: 0,
};

/// Where a held pad or note is kept in a [`Voice`]: a pad and a note of
/// the same number are different notes.
fn key(holds: Holds, note: u8) -> usize {
    usize::from(note & 127)
        + match holds {
            Holds::Hits => 0,
            Holds::Notes => 128,
        }
}

/// [`key`] read back.
fn unkey(k: usize) -> (Holds, u8) {
    let holds = if k >= 128 { Holds::Notes } else { Holds::Hits };
    (holds, (k & 127) as u8)
}

/// Pads and notes a voice can hold.
const KEYS: usize = 256;

/// One pad pressure value kept in the history.
#[derive(Clone, Copy, Debug)]
struct TouchHit {
    /// On the capture clock, in ticks.
    at: f64,
    /// Note starts and ends pushed before it: it came after hit
    /// `after - 1` and before hit `after`.
    after: u64,
    track: u8,
    holds: Holds,
    note: u8,
    value: u8,
}

const NO_TOUCH: TouchHit = TouchHit {
    at: 0.0,
    after: 0,
    track: ARMED,
    holds: Holds::Hits,
    note: 0,
    value: 0,
};

/// A held note's pressure thinning, as the recorder's.
#[derive(Clone, Copy, Debug, Default)]
struct Thin {
    /// The last value kept (0 when the note starts).
    kept: u8,
    /// A one-step value and the frame it came on, kept once it lasts.
    wait: Option<(u8, u64)>,
}

const THIN: Thin = Thin {
    kept: 0,
    wait: None,
};

/// One track's held notes: the armed track's (voice 0), or a routed
/// track's while it holds notes.
#[derive(Clone, Copy, Debug)]
struct Voice {
    track: u8,
    /// By [`key`].
    held: [bool; KEYS],
    n_held: usize,
    thin: [Thin; KEYS],
}

const VOICE: Voice = Voice {
    track: ARMED,
    held: [false; KEYS],
    n_held: 0,
    thin: [THIN; KEYS],
};

impl Voice {
    /// Holding nothing, for `track`; in place, so nothing its size goes
    /// on the stack.
    fn reset(&mut self, track: u8) {
        self.track = track;
        self.held.fill(false);
        self.n_held = 0;
        self.thin.fill(THIN);
    }
}

/// The phrase being played, kept up to date as notes come.
#[derive(Clone, Copy, Debug)]
struct Head {
    /// Number of its first note start in the history.
    first: u64,
    timed: bool,
    bar: u32,
    first_at: f64,
    /// Number and clock time of its latest note start.
    last: u64,
    last_at: f64,
}

/// What CAPTURE would keep now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pending {
    pub bars: i64,
    /// Played while the transport rolled.
    pub timed: bool,
}

/// One note of a phrase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Played {
    /// Its tick and length from the phrase's start.
    pub note: Note,
    /// The song tick it was played on, in time; None when free.
    pub song: Option<f64>,
    /// [`ARMED`], or the track a route played it on.
    pub track: u8,
    /// Its number is a pad's or a note's.
    pub holds: Holds,
    /// Its touch is in the phrase as the other kind too.
    pub paired: bool,
}

/// The last phrase, as clip notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Phrase {
    /// In the order they were played. A note's `touch` names its pressure
    /// in [`Phrase::touch`].
    pub played: Vec<Played>,
    /// Every note's pad pressure, one run a note, each timed in ticks from
    /// its note's start, as a clip keeps it (`Clip::touch`).
    pub touch: Vec<TouchPoint>,
    /// Some of the first notes' pressure was older than the history keeps
    /// ([`MAX_TOUCH_HITS`]), so it is cut short.
    pub touch_cut: bool,
    /// Whole bars.
    pub length: Tick,
    /// Ticks in a bar.
    pub bar: Tick,
    /// In time: the song tick of the phrase's first tick, a bar line. None
    /// when free.
    pub song_start: Option<Tick>,
    /// Names the phrase as it stands: it changes when another note starts.
    pub id: u64,
}

impl Phrase {
    pub fn notes(&self) -> Vec<Note> {
        self.played.iter().map(|p| p.note).collect()
    }

    pub fn bars(&self) -> i64 {
        self.length / self.bar.max(1)
    }

    /// Note `n`'s pressure (a note of this phrase), empty for none.
    pub fn touch_of(&self, n: &Note) -> &[TouchPoint] {
        self.touch.get(n.touch.range()).unwrap_or(&[])
    }

    /// The phrase parted by track, with [`ARMED`]'s notes on track
    /// `armed`: the armed track's part first, then the others in track
    /// order. Every part keeps the phrase's length, bar, song start and
    /// id, so their clips play together as they were played; each note
    /// keeps its pressure.
    pub fn split(&self, armed: usize) -> Vec<(usize, Phrase)> {
        let track_of = |p: &Played| {
            if p.track == ARMED {
                armed
            } else {
                usize::from(p.track)
            }
        };
        let mut tracks: Vec<usize> = self.played.iter().map(track_of).collect();
        tracks.sort_unstable_by_key(|&t| (t != armed, t));
        tracks.dedup();
        tracks
            .into_iter()
            .map(|t| (t, self.part(|p| track_of(p) == t)))
            .collect()
    }

    /// The notes `which` picks, each with its pressure, as a phrase of the
    /// same length, bar, song start and id.
    fn part(&self, which: impl Fn(&Played) -> bool) -> Phrase {
        let mut touch = Vec::new();
        let played = self
            .played
            .iter()
            .filter(|p| which(p))
            .map(|p| {
                let run = self.touch_of(&p.note);
                let mut p = *p;
                p.note.touch = if run.is_empty() {
                    Touch::NONE
                } else {
                    let first = touch.len() as u32;
                    touch.extend_from_slice(run);
                    Touch {
                        first,
                        len: run.len() as u32,
                    }
                };
                p
            })
            .collect();
        Phrase {
            played,
            touch,
            touch_cut: self.touch_cut,
            length: self.length,
            bar: self.bar,
            song_start: self.song_start,
            id: self.id,
        }
    }
}

pub struct Capture {
    hits: Box<[Hit]>,
    /// Note starts and ends ever pushed; hit `n` is at `n % MAX_HITS` while
    /// `n + MAX_HITS > pushed`.
    pushed: u64,
    /// (offset, track, event, kind), in offset order.
    input: [(u32, u8, Event, Took); MAX_INPUT],
    n_input: usize,
    touch: Box<[TouchHit]>,
    /// Pressure values ever kept; value `n` is at `n % MAX_TOUCH_HITS`
    /// while `n + MAX_TOUCH_HITS > touch_pushed`.
    touch_pushed: u64,
    /// (offset, track, pad or note, value, kind), in offset order.
    touch_input: [(u32, u8, u8, u8, Holds); MAX_TOUCH_INPUT],
    n_touch_input: usize,
    /// Voice 0 is the armed track's; the others a routed track's while
    /// it holds notes.
    voices: Box<[Voice]>,
    /// The capture clock at the end of the last period.
    clock: f64,
    /// While rolling: clock = song + shift, a whole number of bars.
    shift: i64,
    /// The last period's end rolled, and the song tick there.
    rolling: bool,
    song_end: Tick,
    bar: i64,
    /// Notes held on all the tracks.
    n_held: usize,
    /// When the last held note ended.
    quiet_since: f64,
    head: Option<Head>,
    /// The phrase CAPTURE kept last, by [`Phrase::id`].
    kept: Option<u64>,
    dropped: u64,
}

impl Default for Capture {
    fn default() -> Self {
        Capture::new()
    }
}

impl Capture {
    /// An empty history. Allocates the rings once.
    pub fn new() -> Capture {
        Capture {
            hits: vec![NO_HIT; MAX_HITS].into_boxed_slice(),
            pushed: 0,
            input: [(0, ARMED, Event::AllOff, Took::PAD); MAX_INPUT],
            n_input: 0,
            touch: vec![NO_TOUCH; MAX_TOUCH_HITS].into_boxed_slice(),
            touch_pushed: 0,
            touch_input: [(0, ARMED, 0, 0, Holds::Hits); MAX_TOUCH_INPUT],
            n_touch_input: 0,
            voices: vec![VOICE; MAX_VOICES].into_boxed_slice(),
            clock: 0.0,
            shift: 0,
            rolling: false,
            song_end: 0,
            bar: Meter::default().bar_ticks(),
            n_held: 0,
            quiet_since: f64::NEG_INFINITY,
            head: None,
            kept: None,
            dropped: 0,
        }
    }

    /// A note for the coming [`Capture::process`] call, at `offset` in the
    /// period. They may come in any order; they are kept in offset order,
    /// and notes on the same offset keep the order they came in. A pad's
    /// pressure ([`Event::Pressure`], the pad and its raw value) has its
    /// own room and comes after the notes on its offset; it is kept while
    /// its pad's note is held. The notes are the armed track's
    /// ([`ARMED`]) pads.
    pub fn input(&mut self, offset: u32, event: Event) {
        self.input_on(ARMED, offset, event, Took::PAD);
    }

    /// [`Capture::input`] of what `took` says: a pad or a note, and its
    /// pressure (refactor E4).
    pub fn input_as(&mut self, offset: u32, event: Event, took: Took) {
        self.input_on(ARMED, offset, event, took);
    }

    /// [`Capture::input_as`], for `track`: [`ARMED`], or a track a route
    /// plays (0 to 254). Each track holds its own notes.
    pub fn input_on(&mut self, track: u8, offset: u32, event: Event, took: Took) {
        if let Event::Pressure { note, value } = event {
            if self.n_touch_input == MAX_TOUCH_INPUT {
                self.dropped += 1;
                return;
            }
            let mut i = self.n_touch_input;
            while i > 0 && self.touch_input[i - 1].0 > offset {
                self.touch_input[i] = self.touch_input[i - 1];
                i -= 1;
            }
            self.touch_input[i] = (offset, track, note, value, took.holds);
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
        self.input[i] = (offset, track, event, took);
        self.n_input += 1;
    }

    /// The voice holding `track`'s notes, if any.
    fn voice_of(&self, track: u8) -> Option<usize> {
        if track == ARMED {
            return Some(0);
        }
        (1..MAX_VOICES).find(|&v| self.voices[v].track == track)
    }

    /// The voice for a note starting on `track`: its own, or one free
    /// (holding nothing). None when all hold notes.
    fn voice_for(&mut self, track: u8) -> Option<usize> {
        if let Some(v) = self.voice_of(track) {
            return Some(v);
        }
        let v = (1..MAX_VOICES).find(|&v| self.voices[v].n_held == 0)?;
        self.voices[v].reset(track);
        Some(v)
    }

    /// Put the period's notes on the clock, by the transport's `block`.
    pub fn process(&mut self, block: &Block) {
        let mut next = 0;
        let mut next_touch = 0;
        for span in block.spans() {
            let bar = span.meter.bar_ticks();
            let timed = span.state.rolling();
            if timed {
                let ticks = span.ticks();
                // A start, a jump or a new meter: carry on from the clock,
                // on the next point with the song's place in the bar.
                if !self.rolling || ticks.start != self.song_end || bar != self.bar {
                    self.shift = ceil_to(self.clock.ceil() as i64 - ticks.start, bar);
                }
                self.rolling = true;
                self.song_end = ticks.end;
            } else {
                self.rolling = false;
            }
            self.bar = bar;
            let end = span.offset + span.len;
            loop {
                let note = (next < self.n_input)
                    .then(|| self.input[next].0)
                    .filter(|&o| o < end);
                // Pressure after the notes on its offset.
                let touch = (next_touch < self.n_touch_input)
                    .then(|| self.touch_input[next_touch].0)
                    .filter(|&o| o < end);
                if let Some(t) = touch {
                    if note.is_none_or(|a| t < a) {
                        let (offset, track, note, value, holds) = self.touch_input[next_touch];
                        next_touch += 1;
                        let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
                        self.touch_value(span, timed, frame, track, key(holds, note), value);
                        continue;
                    }
                }
                if note.is_none() {
                    break;
                }
                let (offset, track, event, took) = self.input[next];
                next += 1;
                let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
                // Pressure whose time came before this note is kept first.
                if let Some(v) = self.voice_of(track) {
                    match event {
                        Event::NoteOn { note, .. } | Event::NoteOff { note } => {
                            self.settle(v, key(took.holds, note), span, timed, frame + 1);
                        }
                        Event::AllOff => self.settle_voice(v, span, timed, frame + 1),
                        _ => {}
                    }
                }
                let (at, song) = self.place(span, timed, frame);
                self.hit(at, song, bar as u32, timed, (track, took), event);
            }
            // One-step values whose time is up within this span.
            self.settle_all(span, timed, span.frame + u64::from(span.len));
            if timed {
                self.clock = span.tick_at(span.frame + u64::from(span.len)) + self.shift as f64;
            } else {
                self.clock += ticks_in(f64::from(span.len), span.bpm, span.rate);
            }
        }
        // Offsets past the period's end are the host's mistake.
        self.dropped += (self.n_input - next) as u64;
        self.dropped += (self.n_touch_input - next_touch) as u64;
        self.n_input = 0;
        self.n_touch_input = 0;
    }

    /// Where `frame` of `span` is: on the capture clock, and the song tick
    /// (0 when stopped). `self.clock` is the clock at the span's start.
    fn place(&self, span: &Span, timed: bool, frame: u64) -> (f64, f64) {
        if timed {
            let song = span.tick_at(frame);
            // Never before the clock: a jump's first frame can fall a
            // fraction of a tick early.
            ((song + self.shift as f64).max(self.clock), song)
        } else {
            (
                self.clock + ticks_in((frame - span.frame) as f64, span.bpm, span.rate),
                0.0,
            )
        }
    }

    /// The pressure `value` of the pad or note at [`key`] `n` on `track`,
    /// on `frame` of `span`: thinned onto its held note (see the crate's
    /// notes).
    fn touch_value(
        &mut self,
        span: &Span,
        timed: bool,
        frame: u64,
        track: u8,
        n: usize,
        value: u8,
    ) {
        let Some(voice) = self.voice_of(track).filter(|&v| self.voices[v].held[n]) else {
            return;
        };
        // A one-step value whose time came before this one is kept first.
        self.settle(voice, n, span, timed, frame + 1);
        let v = value.min(127);
        let t = &mut self.voices[voice].thin[n];
        if v.abs_diff(t.kept) >= TOUCH_STEP {
            t.wait = None;
            self.keep_touch(voice, n, span, timed, frame, v);
        } else if v == t.kept {
            t.wait = None;
        } else if t.wait.is_none_or(|(w, _)| w != v) {
            t.wait = Some((v, frame));
        }
    }

    /// Keep voice `voice`'s held note `n`'s waiting one-step value if its
    /// time is up before frame `before` (its time falls in `span`).
    fn settle(&mut self, voice: usize, n: usize, span: &Span, timed: bool, before: u64) {
        let Some((v, from)) = self.voices[voice].thin[n].wait else {
            return;
        };
        let due = from + u64::from(span.rate) * u64::from(TOUCH_SETTLE_MS) / 1000;
        if due < before {
            self.voices[voice].thin[n].wait = None;
            self.keep_touch(voice, n, span, timed, due.max(span.frame), v);
        }
    }

    fn settle_voice(&mut self, voice: usize, span: &Span, timed: bool, before: u64) {
        for n in 0..KEYS {
            let x = &self.voices[voice];
            if x.held[n] && x.thin[n].wait.is_some() {
                self.settle(voice, n, span, timed, before);
            }
        }
    }

    fn settle_all(&mut self, span: &Span, timed: bool, before: u64) {
        for v in 0..MAX_VOICES {
            if self.voices[v].n_held > 0 {
                self.settle_voice(v, span, timed, before);
            }
        }
    }

    /// Keep `value` on voice `voice`'s held note at [`key`] `n`, on
    /// `frame` of `span`.
    fn keep_touch(
        &mut self,
        voice: usize,
        n: usize,
        span: &Span,
        timed: bool,
        frame: u64,
        value: u8,
    ) {
        self.voices[voice].thin[n].kept = value;
        let (at, _) = self.place(span, timed, frame);
        let (holds, note) = unkey(n);
        self.touch[(self.touch_pushed % MAX_TOUCH_HITS as u64) as usize] = TouchHit {
            at,
            after: self.pushed,
            track: self.voices[voice].track,
            holds,
            note,
            value,
        };
        self.touch_pushed += 1;
    }

    fn hit(
        &mut self,
        at: f64,
        song: f64,
        bar: u32,
        timed: bool,
        (track, took): (u8, Took),
        event: Event,
    ) {
        let holds = took.holds;
        let mut h = Hit {
            at,
            song,
            kind: Kind::AllOff,
            track,
            holds,
            paired: took.paired,
            note: 0,
            velocity: 0,
        };
        match event {
            // CAPTURE keeps the notes played, not parameter locks or a
            // clip's program change; pressure has its own queue.
            Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Unlock { .. }
            | Event::Pressure { .. }
            | Event::Program { .. } => return,
            // A MIDI-style note-on at velocity 0 is a release.
            Event::NoteOn { note, velocity: 0 } | Event::NoteOff { note } => {
                let n = key(holds, note);
                let Some(v) = self.voice_of(track).filter(|&v| self.voices[v].held[n]) else {
                    return;
                };
                let x = &mut self.voices[v];
                x.held[n] = false;
                x.thin[n].wait = None;
                x.n_held -= 1;
                self.n_held -= 1;
                if self.n_held == 0 {
                    self.quiet_since = at;
                }
                h.kind = Kind::Off;
                h.note = note & 127;
            }
            Event::NoteOn { note, velocity } => {
                let n = key(holds, note);
                let Some(v) = self.voice_for(track) else {
                    self.dropped += 1;
                    return;
                };
                let gap = (i64::from(bar) * GAP_BARS) as f64;
                let new = match self.head {
                    None => true,
                    Some(p) => {
                        p.timed != timed
                            || p.bar != bar
                            || self.n_held == 0 && at - self.quiet_since >= gap
                    }
                };
                let x = &mut self.voices[v];
                if !x.held[n] {
                    x.held[n] = true;
                    x.n_held += 1;
                    self.n_held += 1;
                }
                // A new note's pressure starts again, as recording's does.
                x.thin[n] = THIN;
                let seq = self.pushed;
                match &mut self.head {
                    Some(p) if !new => {
                        p.last = seq;
                        p.last_at = at;
                    }
                    _ => {
                        self.head = Some(Head {
                            first: seq,
                            timed,
                            bar,
                            first_at: at,
                            last: seq,
                            last_at: at,
                        })
                    }
                }
                h.kind = Kind::On;
                h.note = note & 127;
                h.velocity = velocity.min(127);
            }
            // The track's notes all end.
            Event::AllOff => {
                let Some(v) = self.voice_of(track).filter(|&v| self.voices[v].n_held > 0) else {
                    return;
                };
                let x = &mut self.voices[v];
                self.n_held -= x.n_held;
                x.reset(track);
                if self.n_held == 0 {
                    self.quiet_since = at;
                }
            }
        }
        self.hits[(self.pushed % MAX_HITS as u64) as usize] = h;
        self.pushed += 1;
    }

    /// Notes and pressure values lost because the input tables were full.
    /// It should stay 0.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Note starts and ends in the history.
    pub fn len(&self) -> usize {
        self.pushed.min(MAX_HITS as u64) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.pushed == 0
    }

    /// What CAPTURE would keep now: None if nothing was played since the
    /// last phrase it kept. Cheap enough for every screen update.
    pub fn pending(&self) -> Option<Pending> {
        let h = self.head.filter(|h| self.kept != Some(h.last))?;
        let bar = i64::from(h.bar).max(1);
        let start = if h.timed {
            floor_to(h.first_at.round() as i64, bar) as f64
        } else {
            h.first_at
        };
        let bars = (h.last_at - start).round() as i64 / bar + 1;
        Some(Pending {
            bars: bars.clamp(1, MAX_BARS),
            timed: h.timed,
        })
    }

    /// The last phrase as clip notes, or None if nothing was played.
    /// Allocates: never call it on the audio thread.
    pub fn phrase(&self) -> Option<Phrase> {
        let h = self.head?;
        let bar = i64::from(h.bar).max(1);
        let oldest = self.pushed.saturating_sub(MAX_HITS as u64);
        let hit = |seq: u64| &self.hits[(seq % MAX_HITS as u64) as usize];
        let start_of = |at: f64| {
            if h.timed {
                floor_to(at.round() as i64, bar) as f64
            } else {
                at
            }
        };
        let tick = |at: f64, start: f64| (at - start).round() as i64;
        // The first note start whose phrase fits in MAX_BARS.
        let mut seq = h.first.max(oldest);
        let (first, start) = loop {
            if seq > h.last {
                return None;
            }
            let x = hit(seq);
            if x.kind == Kind::On && tick(h.last_at, start_of(x.at)) < MAX_BARS * bar {
                break (seq, start_of(x.at));
            }
            seq += 1;
        };
        let length = (tick(h.last_at, start) / bar + 1) * bar;
        let mut played: Vec<Played> = Vec::new();
        // Each played note's pressure, as it comes.
        let mut runs: Vec<Vec<TouchPoint>> = Vec::new();
        // Per track and note: its index in `played` and its start on the
        // clock.
        let mut open: HashMap<(u8, usize), (usize, f64)> = HashMap::new();
        let close = |played: &mut Vec<Played>, o: (usize, f64), end: f64| {
            played[o.0].note.len = ((end - o.1).round() as Tick).clamp(1, MAX_CLIP_TICKS);
        };
        // Pressure kept after the phrase's first note start: each value
        // goes on the note its pad held then.
        let touch_hit = |seq: u64| &self.touch[(seq % MAX_TOUCH_HITS as u64) as usize];
        let touch_oldest = self.touch_pushed.saturating_sub(MAX_TOUCH_HITS as u64);
        let mut ts = touch_oldest;
        while ts < self.touch_pushed && touch_hit(ts).after <= first {
            ts += 1;
        }
        // Values pushed out of the ring came before the oldest one kept.
        let touch_cut = touch_oldest > 0 && touch_hit(touch_oldest).after > first;
        let take_touch = |upto: u64,
                          ts: &mut u64,
                          open: &HashMap<(u8, usize), (usize, f64)>,
                          runs: &mut Vec<Vec<TouchPoint>>| {
            while *ts < self.touch_pushed && touch_hit(*ts).after <= upto {
                let x = *touch_hit(*ts);
                *ts += 1;
                if let Some(&(i, from)) = open.get(&(x.track, key(x.holds, x.note))) {
                    let run = &mut runs[i];
                    let at =
                        ((x.at - from).round().max(0.0) as u32).max(run.last().map_or(0, |p| p.at));
                    run.push(TouchPoint { at, value: x.value });
                }
            }
        };
        for s in first..self.pushed {
            take_touch(s, &mut ts, &open, &mut runs);
            let x = *hit(s);
            match x.kind {
                Kind::On => {
                    if let Some(o) = open.remove(&(x.track, key(x.holds, x.note))) {
                        close(&mut played, o, x.at);
                    }
                    let note = Note {
                        tick: tick(x.at, start),
                        len: 1,
                        note: x.note,
                        velocity: x.velocity.max(1),
                        chance: 100,
                        cond: mpfree_seq::Cond::Always,
                        ratchet: 1,
                        touch: mpfree_seq::Touch::NONE,
                        on: true,
                        ramp: 0,
                        join: mpfree_seq::Join::Attack,
                    };
                    open.insert((x.track, key(x.holds, x.note)), (played.len(), x.at));
                    played.push(Played {
                        note,
                        song: h.timed.then_some(x.song),
                        track: x.track,
                        holds: x.holds,
                        paired: x.paired,
                    });
                    runs.push(Vec::new());
                }
                Kind::Off => {
                    if let Some(o) = open.remove(&(x.track, key(x.holds, x.note))) {
                        close(&mut played, o, x.at);
                    }
                }
                Kind::AllOff => {
                    open.retain(|&(t, _), &mut o| {
                        if t == x.track {
                            close(&mut played, o, x.at);
                        }
                        t != x.track
                    });
                }
            }
        }
        take_touch(self.pushed, &mut ts, &open, &mut runs);
        // Still held: it ends now.
        for (_, o) in open.drain() {
            close(&mut played, o, self.clock);
        }
        if played.len() > MAX_NOTES {
            let over = played.len() - MAX_NOTES;
            played.drain(..over);
            runs.drain(..over);
        }
        let mut touch = Vec::with_capacity(runs.iter().map(Vec::len).sum());
        for (p, run) in played.iter_mut().zip(&runs) {
            if !run.is_empty() {
                p.note.touch = Touch {
                    first: touch.len() as u32,
                    len: run.len() as u32,
                };
                touch.extend_from_slice(run);
            }
        }
        // The clock and the song differ by whole bars, so this is the bar
        // line the phrase starts on, in the song as its first note was.
        let song_start = h.timed.then(|| {
            let x = hit(first);
            start as i64 - ((x.at - x.song) / bar as f64).round() as i64 * bar
        });
        Some(Phrase {
            played,
            touch,
            touch_cut,
            length,
            bar,
            song_start,
            id: h.last,
        })
    }

    /// CAPTURE kept phrase `id`: [`Capture::pending`] says nothing until
    /// another note is played.
    pub fn mark_kept(&mut self, id: u64) {
        self.kept = Some(id);
    }

    /// The kept phrase was undone: it can be kept again.
    pub fn forget_kept(&mut self) {
        self.kept = None;
    }

    /// Forget everything played, for example when another project is
    /// loaded. The clock goes on.
    pub fn clear(&mut self) {
        self.pushed = 0;
        self.touch_pushed = 0;
        for v in self.voices.iter_mut() {
            v.reset(ARMED);
        }
        self.n_held = 0;
        self.quiet_since = f64::NEG_INFINITY;
        self.head = None;
        self.kept = None;
    }
}

/// The last multiple of `step` at or before `tick`.
fn floor_to(tick: Tick, step: i64) -> Tick {
    tick - tick.rem_euclid(step)
}

/// Where the transport is when CAPTURE is pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Now {
    pub position: Tick,
    pub rolling: bool,
    pub meter: Meter,
    pub loop_range: Option<(Tick, Tick)>,
}

/// What [`keep`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kept {
    pub track: usize,
    pub slot: usize,
    /// The capture's take number.
    pub take: u32,
    pub notes: usize,
    pub bars: i64,
    /// A new clip, and the launch that plays it (it replaces the track's
    /// launch). None when the notes went into the clip that was playing.
    pub launch: Option<Launch>,
    /// Pressure values kept on the notes.
    pub touch: usize,
    /// Pressure values the playing clip had no room for, left off its
    /// notes. It should stay 0.
    pub touch_lost: usize,
    /// What the notes are: pads, or notes (refactor E4).
    pub holds: Holds,
    /// Touches kept only as the other kind, left out: pads that played no
    /// note into a clip of notes, or notes that came with no pad (the
    /// arp's) into a clip of pads.
    pub left_out: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepError {
    /// Nothing was played, or nothing since the last capture.
    Nothing,
    NoTrack(usize),
    /// Every slot of the track holds a clip.
    NoSlot(usize),
    /// Everything played was kept only as the other kind than the clip
    /// it would go in holds (this kind).
    OtherKind(Holds),
    Model(ModelError),
}

/// Keep `phrase` in `track` as a take of its own (see the crate's notes):
/// into the clip playing on the track when it was played in time over it,
/// in that clip's kind, as an overdub records; else as a new clip holding
/// `new` (what a new clip for a take on the track holds), named `name`, in
/// the track's first empty slot. Touches kept only as the other kind are
/// left out. Nothing changes on an error.
#[allow(clippy::too_many_arguments)]
pub fn keep(
    phrase: &Phrase,
    project: &mut Project,
    launches: &[Launch],
    takes: &mut Takes,
    track: usize,
    now: Now,
    name: &str,
    new: Holds,
) -> Result<Kept, KeepError> {
    if phrase.played.is_empty() {
        return Err(KeepError::Nothing);
    }
    if track >= project.tracks.len() {
        return Err(KeepError::NoTrack(track));
    }
    // Over the clip that was playing: from where each note was played.
    let playing = launches
        .iter()
        .find(|l| l.track == track)
        .and_then(|l| Some((l, project.clip(track, l.slot)?)))
        .filter(|(l, _)| {
            phrase.played[0]
                .song
                .is_some_and(|first| first >= l.origin as f64)
        });
    // A clip holds one kind (refactor E4): the playing clip's, else what a
    // new clip there holds.
    let holds = playing.map_or(new, |(_, clip)| clip.holds);
    let left_out = phrase
        .played
        .iter()
        .filter(|p| p.holds != holds && !p.paired)
        .count();
    let only;
    let phrase = if phrase.played.iter().any(|p| p.holds != holds) {
        only = phrase.part(|p| p.holds == holds);
        &only
    } else {
        phrase
    };
    if phrase.played.is_empty() {
        return Err(KeepError::OtherKind(holds));
    }
    let bars = phrase.bars();
    let with_touch = project.tracks[track].record_touch;
    if let Some((l, clip)) = playing {
        let song_len = clip.speed.to_song(clip.length as f64);
        // The notes' pressure, on the clip's ticks as their lengths.
        let mut touch: Vec<TouchPoint> = Vec::new();
        let notes: Vec<Note> = phrase
            .played
            .iter()
            .filter_map(|p| {
                let from = p.song? - l.origin as f64;
                if from < 0.0 || song_len <= 0.0 {
                    return None;
                }
                let mut tick = clip.speed.to_clip(from.rem_euclid(song_len)).round() as Tick;
                if tick >= clip.length {
                    tick -= clip.length;
                }
                let len = (clip.speed.to_clip(p.note.len as f64).round() as Tick)
                    .clamp(1, MAX_CLIP_TICKS);
                let run = phrase.touch_of(&p.note);
                let mut note = Note {
                    tick: tick.max(0),
                    len,
                    touch: Touch::NONE,
                    ..p.note
                };
                if with_touch && !run.is_empty() {
                    note.touch = Touch {
                        first: touch.len() as u32,
                        len: run.len() as u32,
                    };
                    let mut last = 0;
                    touch.extend(run.iter().map(|x| {
                        last = (clip.speed.to_clip(f64::from(x.at)).round() as u32).max(last);
                        TouchPoint { at: last, ..*x }
                    }));
                }
                Some(note)
            })
            .collect();
        let n = notes.len();
        let (take, touch_lost) = takes
            .capture_notes(project, track, l.slot, &notes, &touch)
            .map_err(KeepError::Model)?;
        return Ok(Kept {
            track,
            slot: l.slot,
            take,
            notes: n,
            bars,
            launch: None,
            touch: touch.len() - touch_lost,
            touch_lost,
            holds,
            left_out,
        });
    }
    let slot = (0..SLOTS)
        .find(|&s| project.tracks[track].slots[s].is_none())
        .ok_or(KeepError::NoSlot(track))?;
    let mut clip = Clip::new(name, phrase.length);
    clip.holds = holds;
    clip.notes = phrase.notes();
    if with_touch {
        clip.touch = phrase.touch.clone();
    } else {
        for n in &mut clip.notes {
            n.touch = Touch::NONE;
        }
    }
    let n = clip.notes.len();
    let origin = match phrase.song_start {
        // In time: on the song's bar where it was played, moved back by
        // whole passes so it plays from bar 1, the loop's start and now.
        Some(s) => {
            let floor = now
                .position
                .min(now.loop_range.map_or(0, |(a, _)| a))
                .min(0)
                .min(s);
            floor - (floor - s).rem_euclid(phrase.length)
        }
        // Free: from the next bar line (stopped: this bar's).
        None if now.rolling => now.meter.next_bar(now.position),
        None => floor_to(now.position, now.meter.bar_ticks()),
    };
    let clip_touch = clip.touch.len();
    let take = takes
        .capture_clip(project, track, slot, clip)
        .map_err(KeepError::Model)?;
    Ok(Kept {
        track,
        slot,
        take,
        notes: n,
        bars,
        launch: Some(Launch {
            track,
            slot,
            origin,
            // Its passes count from where it was played.
            start: phrase.song_start.unwrap_or(origin),
        }),
        touch: clip_touch,
        touch_lost: 0,
        holds,
        left_out,
    })
}

/// The gestures (from `MoveLog::phrase`) that overlap song ticks
/// `from..to` (the phrase CAPTURE kept, in time), on the ticks of `clip`
/// playing from song tick `origin` on `track`: each as the target, map and
/// points `Takes::capture_moves` writes. A parameter on the track gets a
/// track-relative lane, any other a fixed one.
pub fn moves_for_clip(
    gestures: &[Gesture],
    from: f64,
    to: f64,
    clip: &Clip,
    origin: Tick,
    track: u8,
) -> Vec<MoveWrite> {
    let song_len = clip.speed.to_song(clip.length as f64);
    if song_len <= 0.0 {
        return Vec::new();
    }
    gestures
        .iter()
        .filter_map(|g| {
            let at0 = g.points.first()?.0;
            let last = g.points.last()?.0;
            let end = g.song + (last - at0);
            if end < from || g.song >= to || g.song < origin as f64 {
                return None;
            }
            let first = clip
                .speed
                .to_clip((g.song - origin as f64).rem_euclid(song_len));
            let points = g
                .points
                .iter()
                .map(|&(at, u, k)| (first + clip.speed.to_clip(at - at0), u, k))
                .collect();
            Some((Target::for_track(g.addr, track), g.map, points))
        })
        .collect()
}

#[cfg(test)]
mod tests;
