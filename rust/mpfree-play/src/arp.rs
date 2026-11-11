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

use mpfree_arp::{Clock, Engine, EventKind, Events};
use mpfree_transport::{Block, Mark, Span};

use crate::player::Event;

/// Notes from the player one period can carry to the arp.
pub const MAX_INPUT: usize = 256;

pub struct ArpHost {
    /// The engine; change its settings, latch and pattern between periods.
    pub engine: Engine,
    events: Events,
    input: [(u32, Event); MAX_INPUT],
    n_input: usize,
    dropped_input: u64,
}

impl ArpHost {
    pub fn new(engine: Engine) -> ArpHost {
        ArpHost { engine, events: Events::new(), input: [(0, Event::AllOff); MAX_INPUT], n_input: 0, dropped_input: 0 }
    }

    /// Notes lost so far: player notes that did not fit in one period, and
    /// engine notes that did not fit in one engine call. It should stay 0.
    pub fn dropped(&self) -> u64 {
        self.dropped_input + self.events.dropped
    }

    /// A note from the player for the coming [`ArpHost::process`] call, at
    /// `offset` in the period. Call it from the player's sink, in the order
    /// the player sends them, for every event whose destination is this
    /// arp and for `AllOff` sent to [`crate::ALL`] (a panic).
    pub fn input(&mut self, offset: u32, event: Event) {
        if self.n_input == MAX_INPUT {
            self.dropped_input += 1;
            return;
        }
        self.input[self.n_input] = (offset, event);
        self.n_input += 1;
    }

    /// Run the engine over the period described by `block`. `out` gets
    /// every note the engine plays, with its offset in the period, in time
    /// order.
    pub fn process(&mut self, block: &Block, out: &mut impl FnMut(u32, EventKind)) {
        let markers = block.markers();
        let mut next = 0;
        for span in block.spans() {
            let jumped = markers
                .iter()
                .any(|m| m.offset == span.offset && matches!(m.mark, Mark::Looped { .. } | Mark::Located { .. }));
            let end = span.offset + span.len;
            let mut at = span.offset;
            while at < end {
                self.engine.sync(&clock(span, block.start_frame + u64::from(at), jumped && at == span.offset));
                while next < self.n_input && self.input[next].0 <= at {
                    self.apply(self.input[next].1);
                    next += 1;
                }
                let stop = if next < self.n_input { self.input[next].0.min(end) } else { end };
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

    fn apply(&mut self, e: Event) {
        match e {
            Event::NoteOn { note, velocity } => self.engine.press(note, velocity),
            Event::NoteOff { note } => self.engine.release(note),
            Event::AllOff => self.engine.panic(),
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
