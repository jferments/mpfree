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

use mpfree_seq::Note;
use mpfree_transport::time::{Swing, Tick};
use mpfree_transport::{Block, Mark, Marker, State};

use crate::player::Event;
use crate::view::{ClipView, PlayView};

/// Notes one period can bring to the recorder.
pub const MAX_INPUT: usize = 256;
/// Finished notes waiting for the playhead, and notes handed over in one
/// period.
pub const MAX_PENDING: usize = 256;

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

const NOT_HELD: Held = Held { on: false, tick: 0, velocity: 0 };
const NO_TAKE: Taken = Taken { track: 0, slot: 0, note: Note { tick: 0, len: 1, note: 0, velocity: 1 }, take: 0 };

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
            pending: [Pending { taken: NO_TAKE, release: 0 }; MAX_PENDING],
            n_pending: 0,
            taken: [NO_TAKE; MAX_PENDING],
            n_taken: 0,
            swing: Swing::default(),
            take: 0,
            dropped: 0,
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

    /// Notes lost because a table was full. It should stay 0.
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
        if view.armed != self.armed {
            self.held = [NOT_HELD; 128];
            self.armed = view.armed;
        }
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
                        self.held[n] = Held { on: true, tick, velocity };
                    }
                    Event::AllOff => self.finish_all(view, span.swing, tick),
                }
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
            Mark::RecordOn { .. } => self.take = self.take.wrapping_add(1),
            // The playhead goes on: notes wait for it as before.
            Mark::RecordOff { tick } => self.finish_all(view, self.swing, tick),
            // The playhead leaves: nothing recorded here will be reached
            // in this pass, so everything goes now.
            Mark::Looped { end: tick, .. } | Mark::Located { from: tick, .. } | Mark::Stopped { tick } => {
                self.finish_all(view, self.swing, tick);
                self.release(Tick::MAX);
            }
            _ => {}
        }
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
        let Some(clip) = view.tracks.get(track).and_then(|t| t.clip.as_ref()) else { return };
        let Some((tick, pass)) = clip_tick(clip, h.tick) else { return };
        let len = (clip.speed.to_clip((end - h.tick).max(1) as f64).round() as Tick).clamp(1, clip.length);
        // Where the player will play it in this pass.
        let at = clip.quantise.map_or(tick as f64, |q| q.apply(tick as f64));
        let straight = clip.origin as f64 + pass as f64 * clip.song_len + clip.speed.to_song(at);
        let plays = clip.swing.unwrap_or(transport_swing).apply(straight).round() as Tick;
        if self.n_pending == MAX_PENDING {
            self.dropped += 1;
            return;
        }
        let note = Note { tick, len, note: n as u8, velocity: h.velocity.min(127) };
        // A tick of margin for rounding.
        self.pending[self.n_pending] = Pending { taken: Taken { track, slot: clip.slot, note, take: self.take }, release: plays + 2 };
        self.n_pending += 1;
    }

    /// Hand over the waiting notes due at or before song tick `now`, by
    /// due tick, ties in the order they finished.
    fn release(&mut self, now: Tick) {
        loop {
            let mut best: Option<usize> = None;
            for (i, p) in self.pending[..self.n_pending].iter().enumerate() {
                if p.release <= now && best.is_none_or(|b| p.release < self.pending[b].release) {
                    best = Some(i);
                }
            }
            let Some(i) = best else { return };
            if self.n_taken == MAX_PENDING {
                self.dropped += 1;
            } else {
                self.taken[self.n_taken] = self.pending[i].taken;
                self.n_taken += 1;
            }
            self.pending.copy_within(i + 1..self.n_pending, i);
            self.n_pending -= 1;
        }
    }
}

/// The clip tick of song tick `song`, and the pass it falls in. None before
/// the clip's origin.
fn clip_tick(clip: &ClipView, song: Tick) -> Option<(Tick, i64)> {
    if song < clip.origin || clip.song_len <= 0.0 {
        return None;
    }
    let from = (song - clip.origin) as f64;
    let mut pass = (from / clip.song_len).floor() as i64;
    let mut tick = clip.speed.to_clip(from - pass as f64 * clip.song_len).round() as Tick;
    if tick >= clip.length {
        tick -= clip.length;
        pass += 1;
    }
    Some((tick.max(0), pass))
}
