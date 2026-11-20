//! Room in one port's messages for a period, kept so that no ending is
//! ever lost (step sequencer refactor A2b): the rule of
//! `mpfree_node::room`, by channel, with the port's writer behind it.
//!
//! A port's messages for a period wait in an [`crate::output::OutQueue`],
//! then go to the port's writer thread, which keeps them until each is due
//! to leave. A [`PortRoom`] says which go in, so neither runs out of room
//! for an ending:
//!
//! - **Note-ons**: up to [`Limits::starts`] a period, and only while the
//!   note-offs the port owes and those it passed this period are fewer
//!   than [`Limits::ends`]. Each note-on is owed its own note-off, as the
//!   router sends one for each (gear may give a note struck twice two
//!   voices).
//! - **Note-offs** (and a note-on of velocity 0): one for an owed note
//!   always fits. One for a note the port doesn't owe (its note-on was
//!   left out, or never came here) is left out and counted
//!   ([`PortRoom::unowed`]); nothing sounds for it.
//! - **Pedals.** CC 64 (sustain) and CC 66 (sostenuto) going down hold
//!   notes, so they are owed like a note, and coming up is the ending.
//! - **All notes off** (CC 123, and All sound off, CC 120): up to
//!   [`Limits::all_offs`] a period; each ends the notes its channel owes
//!   (a pedal stays down, as on the gear). One past that with nothing
//!   owed on its channel ends nothing: refused, not lost.
//! - **Values**: every other channel message (controllers, bend,
//!   pressure, program, bank): up to [`Limits::values`] a period. Past
//!   that the moment is too busy: refused and counted
//!   ([`PortRoom::refused`]).
//! - **Clock** (real-time bytes, Song Position): up to [`Limits::clock`].
//! - **The writer.** A note-on, a value or a clock message goes in only
//!   while the messages still waiting in the writer
//!   ([`PortRoom::period`]), this period's, and one note-off for each
//!   owed note stay within [`Limits::wire`]. An ending moves its place
//!   from owed to this period's, so it always has one there too.
//!
//! So a queue of [`Limits::room`] places and a writer that holds
//! [`Limits::wire`] never run out of room for an ending, and
//! [`PortRoom::lost`] counts one that found none anyway: a bug, which must
//! stay 0.
//!
//! Fixed size, no allocation, no panics.

/// How many of each kind of message one period of a port takes, and what
/// its writer holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub starts: usize,
    pub values: usize,
    /// Owed notes and pedals and note-offs passed this period, together.
    pub ends: usize,
    pub all_offs: usize,
    pub clock: usize,
    /// Messages the port's writer holds at once.
    pub wire: usize,
}

impl Limits {
    /// Places a period's queue needs for everything its room lets in.
    pub const fn room(&self) -> usize {
        self.starts + self.values + self.ends + self.all_offs + self.clock
    }
}

/// What MIDI out's ports take: a period's note-ons and values as the
/// shared synth's engines do, a note-off for every note a busy period can
/// end and start again, an All notes off for each channel twice, and the
/// clock. The writer's 4,096 is `port_writer::WIRE_QUEUE` in the app.
pub const LIMITS: Limits = Limits {
    starts: 128,
    values: 256,
    ends: 512,
    all_offs: 32,
    clock: 16,
    wire: 4096,
};

/// The two pedals, a bit each by channel.
const SUSTAIN: u8 = 1;
const SOSTENUTO: u8 = 2;
const CHANNELS: usize = 16;

/// Which messages of a period go to one port; see the module notes.
#[derive(Clone, Copy, Debug)]
pub struct PortRoom {
    limits: Limits,
    /// Note-offs owed, by channel and note.
    notes: [[u8; 128]; CHANNELS],
    /// Pedals down ([`SUSTAIN`], [`SOSTENUTO`]), by channel.
    pedals: [u8; CHANNELS],
    /// Note-offs and pedals owed, in all.
    n_owed: usize,
    /// Note-offs owed on each channel.
    ch_notes: [u16; CHANNELS],
    starts: usize,
    values: usize,
    ended: usize,
    all_offs: usize,
    clock: usize,
    /// Messages let in this period.
    queued: usize,
    /// Messages still waiting in the writer as the period began.
    waiting: usize,
    /// Note-ons, values and clock left out for room, so far.
    pub refused: u64,
    /// Note-offs left out because the port didn't owe their note.
    pub unowed: u64,
    /// Endings with no room: a bug (should stay 0).
    pub lost: u64,
}

impl PortRoom {
    pub const fn new(limits: Limits) -> PortRoom {
        PortRoom {
            limits,
            notes: [[0; 128]; CHANNELS],
            pedals: [0; CHANNELS],
            n_owed: 0,
            ch_notes: [0; CHANNELS],
            starts: 0,
            values: 0,
            ended: 0,
            all_offs: 0,
            clock: 0,
            queued: 0,
            waiting: 0,
            refused: 0,
            unowed: 0,
            lost: 0,
        }
    }

    /// The limits it keeps.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// A new period: its counts from 0, with `waiting` messages still in
    /// the writer from earlier ones. What is owed stays owed.
    pub fn period(&mut self, waiting: usize) {
        self.starts = 0;
        self.values = 0;
        self.ended = 0;
        self.all_offs = 0;
        self.clock = 0;
        self.queued = 0;
        self.waiting = waiting;
    }

    /// Nothing is owed any more: the port's gear went (unplugged) and its
    /// notes were forgotten.
    pub fn forget(&mut self) {
        self.notes = [[0; 128]; CHANNELS];
        self.pedals = [0; CHANNELS];
        self.n_owed = 0;
        self.ch_notes = [0; CHANNELS];
    }

    /// Note-offs and pedals owed here.
    pub fn owed(&self) -> usize {
        self.n_owed
    }

    /// Whether note `note` on channel `ch` is owed a note-off here.
    pub fn owes(&self, ch: u8, note: u8) -> bool {
        self.notes[usize::from(ch & 0x0F)][usize::from(note & 0x7F)] > 0
    }

    /// Whether the message `b` (its bytes) goes in now, counting it if it
    /// does.
    pub fn admit(&mut self, b: &[u8]) -> bool {
        let Some(&status) = b.first() else {
            self.refused += 1;
            return false;
        };
        if status >= 0xF0 {
            return self.clock();
        }
        let ch = usize::from(status & 0x0F);
        let (d1, d2) = (
            usize::from(b.get(1).copied().unwrap_or(0) & 0x7F),
            b.get(2).copied().unwrap_or(0) & 0x7F,
        );
        match status & 0xF0 {
            0x90 if d2 > 0 => self.start(ch, d1),
            0x80 | 0x90 => self.end(ch, Some(d1)),
            0xB0 => {
                let pedal = match d1 {
                    64 => SUSTAIN,
                    66 => SOSTENUTO,
                    120 | 123 => return self.all_off(ch),
                    _ => return self.value(),
                };
                match (d2 >= 64, self.pedals[ch] & pedal != 0) {
                    (true, false) => self.start_pedal(ch, pedal),
                    (false, true) => {
                        self.pedals[ch] &= !pedal;
                        self.end(ch, None)
                    }
                    // Down further, or up with nothing held: a value.
                    _ => self.value(),
                }
            }
            _ => self.value(),
        }
    }

    /// Whether the writer has a place for one more message that isn't an
    /// ending, with `more` endings newly owed, and still one for each owed
    /// ending.
    fn wire_has(&self, more: usize) -> bool {
        self.waiting + self.queued + 1 + self.n_owed + more <= self.limits.wire
    }

    /// Whether a note-on (`Some(note)`) or a pedal going down (`None`) has
    /// room, owing one ending more; counted if so.
    fn has_start(&mut self, note: Option<(usize, usize)>) -> bool {
        let full = note.is_some_and(|(ch, n)| self.notes[ch][n] == u8::MAX);
        if !full
            && self.starts < self.limits.starts
            && self.n_owed + self.ended < self.limits.ends
            && self.wire_has(1)
        {
            self.starts += 1;
            self.queued += 1;
            self.n_owed += 1;
            true
        } else {
            self.refused += 1;
            false
        }
    }

    fn start(&mut self, ch: usize, n: usize) -> bool {
        if !self.has_start(Some((ch, n))) {
            return false;
        }
        self.notes[ch][n] += 1;
        self.ch_notes[ch] += 1;
        true
    }

    fn start_pedal(&mut self, ch: usize, pedal: u8) -> bool {
        if !self.has_start(None) {
            return false;
        }
        self.pedals[ch] |= pedal;
        true
    }

    /// A note-off (`Some(note)`), or a pedal coming up (`None`, its bit
    /// already cleared).
    fn end(&mut self, ch: usize, note: Option<usize>) -> bool {
        if let Some(n) = note {
            if self.notes[ch][n] == 0 {
                self.unowed += 1;
                return false;
            }
            self.notes[ch][n] -= 1;
            self.ch_notes[ch] -= 1;
        }
        // Owed endings and passed ones never pass the limits together:
        // this one moves from the first to the second.
        self.n_owed -= 1;
        self.ended += 1;
        self.queued += 1;
        true
    }

    fn all_off(&mut self, ch: usize) -> bool {
        let notes = usize::from(self.ch_notes[ch]);
        // Ending a note frees its place in the writer; with none owed it
        // needs a place of its own.
        if self.all_offs < self.limits.all_offs && (notes > 0 || self.wire_has(0)) {
            self.all_offs += 1;
            self.queued += 1;
            self.notes[ch] = [0; 128];
            self.n_owed -= notes;
            self.ch_notes[ch] = 0;
            true
        } else if notes == 0 {
            self.refused += 1;
            false
        } else {
            self.lost += 1;
            false
        }
    }

    fn value(&mut self) -> bool {
        if self.values < self.limits.values && self.wire_has(0) {
            self.values += 1;
            self.queued += 1;
            true
        } else {
            self.refused += 1;
            false
        }
    }

    fn clock(&mut self) -> bool {
        if self.clock < self.limits.clock && self.wire_has(0) {
            self.clock += 1;
            self.queued += 1;
            true
        } else {
            self.refused += 1;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Limits = Limits {
        starts: 4,
        values: 2,
        ends: 6,
        all_offs: 1,
        clock: 1,
        wire: 1000,
    };

    fn on(ch: u8, note: u8) -> [u8; 3] {
        [0x90 | ch, note, 100]
    }

    fn off(ch: u8, note: u8) -> [u8; 3] {
        [0x80 | ch, note, 64]
    }

    #[test]
    fn a_busy_period_refuses_starts_and_takes_every_owed_ending() {
        let mut r = PortRoom::new(L);
        // Four starts a period across channels, then refused.
        let taken: Vec<bool> = (0..6).map(|n| r.admit(&on(n, 60))).collect();
        assert_eq!(taken, [true, true, true, true, false, false]);
        assert_eq!((r.owed(), r.refused), (4, 2));
        // The same note on another channel is another note.
        assert!(r.owes(3, 60) && !r.owes(4, 60));
        // Values and clock have their own room.
        assert!(r.admit(&[0xE0, 0, 0x40]) && r.admit(&[0xB0, 1, 9]));
        assert!(!r.admit(&[0xD0, 9]));
        assert!(r.admit(&[0xF8]) && !r.admit(&[0xF8]));
        // Every owed note ends; a refused one's note-off is left out, and
        // so is a note-on of velocity 0 for it.
        let ended: Vec<bool> = (0..6).map(|n| r.admit(&off(n, 60))).collect();
        assert_eq!(ended, [true, true, true, true, false, false]);
        assert!(!r.admit(&[0x95, 60, 0]));
        assert_eq!((r.owed(), r.unowed, r.lost), (0, 3, 0));
        // A new period has room again; what is owed stays owed.
        assert!(!r.admit(&on(0, 1)), "the period's starts are spent");
        r.period(0);
        assert!(r.admit(&on(0, 1)));
        r.period(0);
        assert!(r.owes(0, 1));
        assert!(r.admit(&off(0, 1)));
        // A note struck twice is owed two note-offs (gear may give it two
        // voices); a third is left out.
        assert!(r.admit(&on(1, 70)) && r.admit(&on(1, 70)));
        assert!(r.admit(&off(1, 70)) && r.admit(&off(1, 70)));
        assert!(!r.admit(&off(1, 70)));
        assert_eq!((r.owed(), r.lost), (0, 0));
    }

    #[test]
    fn a_pedal_is_owed_like_a_note_and_all_notes_off_ends_a_channel() {
        let mut r = PortRoom::new(L);
        assert!(r.admit(&[0xB2, 64, 127]));
        assert!(r.admit(&[0xB2, 64, 100]), "further down: a value");
        assert!(r.admit(&on(2, 40)) && r.admit(&on(2, 41)) && r.admit(&on(5, 40)));
        assert_eq!(r.owed(), 4);
        // All notes off on channel 3 ends its two notes; its pedal stays.
        assert!(r.admit(&[0xB2, 123, 0]));
        assert_eq!(r.owed(), 2);
        assert!(!r.admit(&off(2, 40)), "the all notes off ended it");
        assert!(r.admit(&[0xB2, 64, 0]), "the pedal comes up");
        // A second all notes off: with nothing owed there, refused...
        assert!(!r.admit(&[0xB2, 123, 0]));
        assert_eq!(r.lost, 0);
        // ...with a note owed, no room: a bug.
        assert!(!r.admit(&[0xB5, 120, 0]));
        assert_eq!(r.lost, 1);
    }

    #[test]
    fn the_writer_keeps_a_place_for_every_owed_ending() {
        let mut r = PortRoom::new(Limits {
            starts: 100,
            values: 100,
            ends: 200,
            wire: 10,
            ..L
        });
        // Three messages still wait in the writer: of the seven places
        // left, each note takes two (its note-on, its note-off later).
        r.period(3);
        assert!((0..3).all(|n| r.admit(&on(0, n))));
        assert!(!r.admit(&on(0, 3)), "its note-off would find no place");
        // One value takes the last place the endings don't need.
        assert!(r.admit(&[0xE0, 0, 0x40]));
        assert!(!r.admit(&[0xE0, 0, 0x41]));
        assert_eq!(r.refused, 2);
        // The endings still go in.
        assert!((0..3).all(|n| r.admit(&off(0, n))));
        assert_eq!((r.lost, r.owed()), (0, 0));
        // Next period the writer has written them all: room again.
        r.period(0);
        assert!(r.admit(&on(0, 3)));
    }
}
