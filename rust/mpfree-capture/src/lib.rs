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
#![forbid(unsafe_code)]

use mpfree_play::{Event, Launch, Takes};
use mpfree_seq::{Clip, ModelError, Note, Project, MAX_CLIP_TICKS, MAX_NOTES, SLOTS};
use mpfree_transport::time::{ceil_to, ticks_in, Meter, Tick};
use mpfree_transport::Block;

/// Note starts and ends kept in the history: about 13 minutes of 1/16
/// notes on two pads at 120 BPM.
pub const MAX_HITS: usize = 16384;
/// Notes one period can bring.
pub const MAX_INPUT: usize = 256;
/// A silence of this many bars ends a phrase.
pub const GAP_BARS: i64 = 1;
/// The longest phrase CAPTURE keeps, in bars; a longer one keeps its last
/// bars.
pub const MAX_BARS: i64 = 32;

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
    note: u8,
    velocity: u8,
}

const NO_HIT: Hit = Hit { at: 0.0, song: 0.0, kind: Kind::AllOff, note: 0, velocity: 0 };

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
}

/// The last phrase, as clip notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Phrase {
    /// In the order they were played.
    pub played: Vec<Played>,
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
}

pub struct Capture {
    hits: Box<[Hit]>,
    /// Note starts and ends ever pushed; hit `n` is at `n % MAX_HITS` while
    /// `n + MAX_HITS > pushed`.
    pushed: u64,
    input: [(u32, Event); MAX_INPUT],
    n_input: usize,
    /// The capture clock at the end of the last period.
    clock: f64,
    /// While rolling: clock = song + shift, a whole number of bars.
    shift: i64,
    /// The last period's end rolled, and the song tick there.
    rolling: bool,
    song_end: Tick,
    bar: i64,
    held: [bool; 128],
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
    /// An empty history. Allocates the ring once.
    pub fn new() -> Capture {
        Capture {
            hits: vec![NO_HIT; MAX_HITS].into_boxed_slice(),
            pushed: 0,
            input: [(0, Event::AllOff); MAX_INPUT],
            n_input: 0,
            clock: 0.0,
            shift: 0,
            rolling: false,
            song_end: 0,
            bar: Meter::default().bar_ticks(),
            held: [false; 128],
            n_held: 0,
            quiet_since: f64::NEG_INFINITY,
            head: None,
            kept: None,
            dropped: 0,
        }
    }

    /// A note for the coming [`Capture::process`] call, at `offset` in the
    /// period. They may come in any order; they are kept in offset order,
    /// and notes on the same offset keep the order they came in.
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

    /// Put the period's notes on the clock, by the transport's `block`.
    pub fn process(&mut self, block: &Block) {
        let mut next = 0;
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
            while next < self.n_input && self.input[next].0 < end {
                let (offset, event) = self.input[next];
                next += 1;
                let frame = span.frame + u64::from(offset.saturating_sub(span.offset));
                let (at, song) = if timed {
                    let song = span.tick_at(frame);
                    // Never before the clock: a jump's first frame can
                    // fall a fraction of a tick early.
                    ((song + self.shift as f64).max(self.clock), song)
                } else {
                    (self.clock + ticks_in((frame - span.frame) as f64, span.bpm, span.rate), 0.0)
                };
                self.hit(at, song, bar as u32, timed, event);
            }
            if timed {
                self.clock = span.tick_at(span.frame + u64::from(span.len)) + self.shift as f64;
            } else {
                self.clock += ticks_in(f64::from(span.len), span.bpm, span.rate);
            }
        }
        // Offsets past the period's end are the host's mistake.
        self.dropped += (self.n_input - next) as u64;
        self.n_input = 0;
    }

    fn hit(&mut self, at: f64, song: f64, bar: u32, timed: bool, event: Event) {
        let mut h = Hit { at, song, kind: Kind::AllOff, note: 0, velocity: 0 };
        match event {
            // A MIDI-style note-on at velocity 0 is a release.
            Event::NoteOn { note, velocity: 0 } | Event::NoteOff { note } => {
                let n = usize::from(note & 127);
                if !self.held[n] {
                    return;
                }
                self.held[n] = false;
                self.n_held -= 1;
                if self.n_held == 0 {
                    self.quiet_since = at;
                }
                h.kind = Kind::Off;
                h.note = n as u8;
            }
            Event::NoteOn { note, velocity } => {
                let n = usize::from(note & 127);
                let gap = (i64::from(bar) * GAP_BARS) as f64;
                let new = match self.head {
                    None => true,
                    Some(p) => p.timed != timed || p.bar != bar || self.n_held == 0 && at - self.quiet_since >= gap,
                };
                if !self.held[n] {
                    self.held[n] = true;
                    self.n_held += 1;
                }
                let seq = self.pushed;
                match &mut self.head {
                    Some(p) if !new => {
                        p.last = seq;
                        p.last_at = at;
                    }
                    _ => self.head = Some(Head { first: seq, timed, bar, first_at: at, last: seq, last_at: at }),
                }
                h.kind = Kind::On;
                h.note = n as u8;
                h.velocity = velocity.min(127);
            }
            Event::AllOff => {
                if self.n_held == 0 {
                    return;
                }
                self.quiet_since = at;
                self.held = [false; 128];
                self.n_held = 0;
            }
        }
        self.hits[(self.pushed % MAX_HITS as u64) as usize] = h;
        self.pushed += 1;
    }

    /// Notes lost because the input table was full. It should stay 0.
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
        let start = if h.timed { floor_to(h.first_at.round() as i64, bar) as f64 } else { h.first_at };
        let bars = (h.last_at - start).round() as i64 / bar + 1;
        Some(Pending { bars: bars.clamp(1, MAX_BARS), timed: h.timed })
    }

    /// The last phrase as clip notes, or None if nothing was played.
    /// Allocates: never call it on the audio thread.
    pub fn phrase(&self) -> Option<Phrase> {
        let h = self.head?;
        let bar = i64::from(h.bar).max(1);
        let oldest = self.pushed.saturating_sub(MAX_HITS as u64);
        let hit = |seq: u64| &self.hits[(seq % MAX_HITS as u64) as usize];
        let start_of = |at: f64| if h.timed { floor_to(at.round() as i64, bar) as f64 } else { at };
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
        // Per note: its index in `played` and its start on the clock.
        let mut open: [Option<(usize, f64)>; 128] = [None; 128];
        let close = |played: &mut Vec<Played>, o: (usize, f64), end: f64| {
            played[o.0].note.len = ((end - o.1).round() as Tick).clamp(1, MAX_CLIP_TICKS);
        };
        for s in first..self.pushed {
            let x = *hit(s);
            match x.kind {
                Kind::On => {
                    let n = usize::from(x.note);
                    if let Some(o) = open[n].take() {
                        close(&mut played, o, x.at);
                    }
                    let note = Note { tick: tick(x.at, start), len: 1, note: x.note, velocity: x.velocity.max(1) };
                    open[n] = Some((played.len(), x.at));
                    played.push(Played { note, song: h.timed.then_some(x.song) });
                }
                Kind::Off => {
                    if let Some(o) = open[usize::from(x.note)].take() {
                        close(&mut played, o, x.at);
                    }
                }
                Kind::AllOff => {
                    for o in open.iter_mut().filter_map(Option::take) {
                        close(&mut played, o, x.at);
                    }
                }
            }
        }
        // Still held: it ends now.
        for o in open.iter_mut().filter_map(Option::take) {
            close(&mut played, o, self.clock);
        }
        if played.len() > MAX_NOTES {
            played.drain(..played.len() - MAX_NOTES);
        }
        // The clock and the song differ by whole bars, so this is the bar
        // line the phrase starts on, in the song as its first note was.
        let song_start = h.timed.then(|| {
            let x = hit(first);
            start as i64 - ((x.at - x.song) / bar as f64).round() as i64 * bar
        });
        Some(Phrase { played, length, bar, song_start, id: h.last })
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
        self.held = [false; 128];
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeepError {
    /// Nothing was played, or nothing since the last capture.
    Nothing,
    NoTrack(usize),
    /// Every slot of the track holds a clip.
    NoSlot(usize),
    Model(ModelError),
}

/// Keep `phrase` in `track` as a take of its own (see the crate's notes):
/// into the clip playing on the track when it was played in time over it,
/// else as a new clip, named `name`, in the track's first empty slot.
/// Nothing changes on an error.
pub fn keep(phrase: &Phrase, project: &mut Project, launches: &[Launch], takes: &mut Takes, track: usize, now: Now,
            name: &str) -> Result<Kept, KeepError> {
    if phrase.played.is_empty() {
        return Err(KeepError::Nothing);
    }
    if track >= project.tracks.len() {
        return Err(KeepError::NoTrack(track));
    }
    let bars = phrase.bars();
    // Over the clip that was playing: from where each note was played.
    let playing = launches.iter().find(|l| l.track == track).and_then(|l| Some((l, project.clip(track, l.slot)?)));
    if let (Some(first), Some((l, clip))) = (phrase.played[0].song, playing) {
        if first >= l.origin as f64 {
            let song_len = clip.speed.to_song(clip.length as f64);
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
                    let len = (clip.speed.to_clip(p.note.len as f64).round() as Tick).clamp(1, MAX_CLIP_TICKS);
                    Some(Note { tick: tick.max(0), len, ..p.note })
                })
                .collect();
            let n = notes.len();
            let take = takes.capture_notes(project, track, l.slot, &notes).map_err(KeepError::Model)?;
            return Ok(Kept { track, slot: l.slot, take, notes: n, bars, launch: None });
        }
    }
    let slot = (0..SLOTS).find(|&s| project.tracks[track].slots[s].is_none()).ok_or(KeepError::NoSlot(track))?;
    let mut clip = Clip::new(name, phrase.length);
    clip.notes = phrase.notes();
    let n = clip.notes.len();
    let origin = match phrase.song_start {
        // In time: on the song's bar where it was played, moved back by
        // whole passes so it plays from bar 1, the loop's start and now.
        Some(s) => {
            let floor = now.position.min(now.loop_range.map_or(0, |(a, _)| a)).min(0).min(s);
            floor - (floor - s).rem_euclid(phrase.length)
        }
        // Free: from the next bar line (stopped: this bar's).
        None if now.rolling => now.meter.next_bar(now.position),
        None => floor_to(now.position, now.meter.bar_ticks()),
    };
    let take = takes.capture_clip(project, track, slot, clip).map_err(KeepError::Model)?;
    Ok(Kept { track, slot, take, notes: n, bars, launch: Some(Launch { track, slot, origin }) })
}

#[cfg(test)]
mod tests;
