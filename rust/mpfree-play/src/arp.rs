//! A track through the arpeggiator: the player's notes for that track hold
//! and release notes on an `mpfree-arp` engine, and the engine's notes go
//! on to the sampler (or any other destination).
//!
//! Each period, [`ArpHost::process`] walks the transport's spans. Before
//! every engine call it gives the engine the span's clock (`Engine::sync`,
//! the hook added in PR #42), and it splits a span at
//! each note from the player, so a press lands on its exact frame. The
//! engine's own timing does not depend on how its calls are cut, so the
//! output is the same for any period size. No allocation: fixed buffers.
//!
//! **No ending is ever lost** (step sequencer refactor A2; playback.md,
//! "Endings always get through"). The notes a period brings an arp wait
//! in a table of [`MAX_INPUT`] places. They come from up to three
//! [sources](CLIPS), each pairing its own note-ons and note-offs, and the
//! arp keeps, by source and note, how many it let in and has not seen
//! end ("owed"). A note-on or a pressure value gets in only while the
//! places taken and the notes owed leave room for it and for its note-off
//! ([`PANICS`] places stay free for panics); past that the moment is too
//! busy, and it is left out and counted ([`ArpHost::refused`]). A
//! note-off ending an owed note takes one place and frees one owed, so it
//! always fits. One the arp doesn't owe (its note-on was left out) is
//! left out too ([`ArpHost::unowed`]): the arp never pressed it. A panic
//! has a place of its own and ends what its source owes. [`ArpHost::lost`]
//! counts an ending that found no place anyway: a bug, which must stay 0.

use mpfree_arp::{Clock, Engine, EventKind, Events};
use mpfree_notefx::{Ev, Notes};
use mpfree_transport::transport::MAX_COMMANDS_PER_PERIOD;
use mpfree_transport::{Block, Mark, Span};

use crate::player::Event;

/// Notes one period can carry to the arp: the places in its table.
pub const MAX_INPUT: usize = 256;

/// The player's notes for a track whose clips play through its arp (and
/// [`ArpHost::input`]'s).
pub const CLIPS: usize = 0;
/// The live pads' ([`crate::TrackArps::pad`]).
pub const PADS: usize = 1;
/// What a track's MIDI effects above the arp make of its notes.
pub const EFFECTS: usize = 2;
/// Sources an arp keeps apart: [`CLIPS`], [`PADS`] and [`EFFECTS`].
pub const SOURCES: usize = 3;
/// Places an arp's table keeps for panics: one for each STOP a period
/// can carry (the transport takes at most [`MAX_COMMANDS_PER_PERIOD`]
/// commands a period, and the player sends one panic for each), and one
/// for the switch "Clips through the arp" changing.
pub const PANICS: usize = MAX_COMMANDS_PER_PERIOD + 1;

pub struct ArpHost {
    /// The engine; change its settings, latch and pattern between periods.
    pub engine: Engine,
    events: Events,
    input: [(u32, Event); MAX_INPUT],
    n_input: usize,
    /// Presses per note number not yet released, from every source.
    presses: [u8; 128],
    /// Notes let in and not yet ended, by source, then note.
    owed: [[u8; 128]; SOURCES],
    n_owed: usize,
    refused: u64,
    unowed: u64,
    lost: u64,
}

impl ArpHost {
    pub fn new(engine: Engine) -> ArpHost {
        ArpHost {
            engine,
            events: Events::new(),
            input: [(0, Event::AllOff); MAX_INPUT],
            n_input: 0,
            presses: [0; 128],
            owed: [[0; 128]; SOURCES],
            n_owed: 0,
            refused: 0,
            unowed: 0,
            lost: 0,
        }
    }

    /// Notes that gave way so far: note-ons and pressure left out of
    /// periods too busy for them, endings lost (a bug), and engine notes
    /// that did not fit in one engine call. It should stay 0.
    pub fn dropped(&self) -> u64 {
        self.refused + self.lost + self.events.dropped
    }

    /// Note-ons and pressure values left out of a period with no room for
    /// them (and their endings), so far.
    pub fn refused(&self) -> u64 {
        self.refused
    }

    /// Note-offs left out because their note-on was, so far. Nothing
    /// hangs from them: the arp never pressed their notes.
    pub fn unowed(&self) -> u64 {
        self.unowed
    }

    /// Endings (note-offs of notes let in, panics) that found no place, so
    /// far. A bug: it must stay 0.
    pub fn lost(&self) -> u64 {
        self.lost
    }

    /// Whether `source` has let `note` in and not yet ended it here.
    pub fn owes(&self, source: usize, note: u8) -> bool {
        self.owed
            .get(source)
            .is_some_and(|o| o[usize::from(note & 127)] > 0)
    }

    /// [`ArpHost::input_from`] the player's notes ([`CLIPS`]).
    pub fn input(&mut self, offset: u32, event: Event) -> bool {
        self.input_from(CLIPS, offset, event)
    }

    /// A note for the coming [`ArpHost::process`] call from `source`, at
    /// `offset` in the period: the player's notes for a through track and
    /// its panics ([`CLIPS`]), the live pads' ([`PADS`]), what a track's
    /// MIDI effects make of them ([`EFFECTS`]). Notes from several sources
    /// can come in any order; they are kept in offset order, and notes on
    /// the same offset keep the order they came in. Whether it went in:
    /// see the module notes for what is left out. A panic (`AllOff`) from
    /// the pads lets go of what they hold here rather than stopping the
    /// arp.
    pub fn input_from(&mut self, source: usize, offset: u32, event: Event) -> bool {
        let source = source.min(SOURCES - 1);
        // Places the table can still give: each owed note keeps one for its
        // note-off, and the last PANICS stay for panics.
        let free = (MAX_INPUT - PANICS).saturating_sub(self.n_input + self.n_owed);
        match event {
            Event::NoteOn { note, .. } => {
                let o = &mut self.owed[source][usize::from(note & 127)];
                if free < 2 || *o == u8::MAX {
                    self.refused += 1;
                    return false;
                }
                *o += 1;
                self.n_owed += 1;
            }
            Event::NoteOff { note } => {
                let o = &mut self.owed[source][usize::from(note & 127)];
                if *o == 0 {
                    self.unowed += 1;
                    return false;
                }
                *o -= 1;
                self.n_owed -= 1;
            }
            Event::AllOff if source == PADS => {
                self.let_go(offset);
                return true;
            }
            // A panic ends the notes its own source owes, which come in
            // offset order. The others end their own notes as they always
            // do: the pads when they let go, the effects when the panic
            // cuts them (a stray note-off after a panic presses nothing).
            Event::AllOff => {
                for n in self.owed[source].iter_mut() {
                    self.n_owed -= usize::from(*n);
                    *n = 0;
                }
            }
            Event::Pressure { .. }
            | Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Unlock { .. }
            | Event::Program { .. } => {
                if free < 1 {
                    self.refused += 1;
                    return false;
                }
            }
        }
        if self.n_input == MAX_INPUT {
            // Only an ending can get here (the rule keeps room for the
            // rest): a bug, counted.
            self.lost += 1;
            return false;
        }
        self.insert(offset, event);
        true
    }

    /// The pads let go of every note they hold on this arp, at `offset`.
    fn let_go(&mut self, offset: u32) {
        for note in 0..128u8 {
            for _ in 0..self.owed[PADS][usize::from(note)] {
                self.input_from(PADS, offset, Event::NoteOff { note });
            }
        }
    }

    /// `event` into the table at its place by offset.
    fn insert(&mut self, offset: u32, event: Event) {
        let mut i = self.n_input;
        while i > 0 && self.input[i - 1].0 > offset {
            self.input[i] = self.input[i - 1];
            i -= 1;
        }
        self.input[i] = (offset, event);
        self.n_input += 1;
    }

    /// Where the first panic (`AllOff`) among the coming period's notes
    /// is, if there is one.
    pub fn first_all_off(&self) -> Option<u32> {
        self.input[..self.n_input]
            .iter()
            .find(|(_, e)| matches!(e, Event::AllOff))
            .map(|(offset, _)| *offset)
    }

    /// Moves the coming period's notes on and off to `out`, on the frame
    /// count from `start` (the period's first frame), in order, for a
    /// track's MIDI effects before the arp; what they make comes back
    /// through [`ArpHost::input`]. A panic and pressure stay.
    pub fn take_notes(&mut self, start: u64, out: &mut Notes) {
        let mut kept = 0;
        for i in 0..self.n_input {
            let (offset, e) = self.input[i];
            let at = start + u64::from(offset);
            match e {
                Event::NoteOn { note, velocity } => {
                    out.push(Ev::on(at, note, velocity));
                }
                Event::NoteOff { note } => {
                    out.push(Ev::off(at, note));
                }
                _ => {
                    self.input[kept] = (offset, e);
                    kept += 1;
                }
            }
        }
        self.n_input = kept;
    }

    /// Run the engine over the period described by `block`. `out` gets
    /// every note the engine plays, with its offset in the period, in time
    /// order.
    pub fn process(&mut self, block: &Block, out: &mut impl FnMut(u32, EventKind)) {
        let markers = block.markers();
        let mut next = 0;
        for span in block.spans() {
            let jumped = markers.iter().any(|m| {
                m.offset == span.offset
                    && matches!(m.mark, Mark::Looped { .. } | Mark::Located { .. })
            });
            let end = span.offset + span.len;
            let mut at = span.offset;
            while at < end {
                self.engine.sync(&clock(
                    span,
                    block.start_frame + u64::from(at),
                    jumped && at == span.offset,
                ));
                while next < self.n_input && self.input[next].0 <= at {
                    self.apply(self.input[next].1);
                    next += 1;
                }
                let stop = if next < self.n_input {
                    self.input[next].0.min(end)
                } else {
                    end
                };
                self.engine.process(stop - at, &mut self.events);
                for e in self.events.as_slice() {
                    out(at + e.offset, e.kind);
                }
                at = stop;
            }
        }
        // Notes on no span's frames cannot happen; keep the engine in step anyway.
        while next < self.n_input {
            self.apply(self.input[next].1);
            next += 1;
        }
        self.n_input = 0;
    }

    /// A note held by a pad and a clip at once is pressed on the engine
    /// once, and released when the last of them lets go.
    fn apply(&mut self, e: Event) {
        match e {
            Event::NoteOn { note, velocity } => {
                let p = &mut self.presses[usize::from(note & 127)];
                *p = p.saturating_add(1);
                if *p == 1 {
                    self.engine.press(note, velocity);
                }
            }
            Event::NoteOff { note } => {
                let p = &mut self.presses[usize::from(note & 127)];
                if *p > 0 {
                    *p -= 1;
                    if *p == 0 {
                        self.engine.release(note);
                    }
                }
            }
            Event::AllOff => {
                self.presses = [0; 128];
                self.engine.panic();
            }
            // A clip's pressure on a held note: the arp's repeats follow it.
            Event::Pressure { note, value } => self.engine.pressure(note, value),
            // A parameter lock or move is for the host's resolver, and a
            // program change for outside gear, not the arp.
            Event::Lock { .. }
            | Event::Lane { .. }
            | Event::Unlock { .. }
            | Event::Program { .. } => {}
        }
    }
}

/// The arp's clock for an engine call starting at absolute frame `frame`
/// inside `span`.
pub fn clock(span: &Span, frame: u64, jumped: bool) -> Clock {
    let swing = span.swing;
    Clock {
        anchor_tick: span.anchor_tick,
        frames_since_anchor: frame.saturating_sub(span.anchor_frame),
        bpm: span.bpm,
        playing: span.state.rolling(),
        jumped,
        // Straight transport swing leaves the arp's own swing in charge.
        swing: (swing.amount > 50).then_some((swing.grid, swing.amount)),
        bar_ticks: span.meter.bar_ticks() as u32,
    }
}
