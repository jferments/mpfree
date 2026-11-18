//! Expression out to MPE gear (plan item 3.7): each note on a member
//! channel of its own, so a key's slide, pressure and tone reach only the
//! note it plays.
//!
//! Gear whose device profile says `mpe = lower 15` (or `upper N`) speaks
//! MPE in that zone ([`Zone`]). [`MpeOut`] is one track's notes on it:
//!
//! - **Channels.** Each note-on takes a member channel: a free one, the one
//!   free longest first, so a note's release tail isn't bent by the next
//!   note. With every member busy, the note shares the channel whose
//!   newest note started longest ago (MPE's advice for running out), and
//!   the two share its bend and pressure. A note's off, pressure, slide and
//!   tone go on the channel its on went to.
//! - **Each note's expression.** A slide is the channel's pitch bend over
//!   MPE's member range ([`MEMBER_BEND`], 48 semitones), pressure the
//!   channel's pressure and tone its CC 74. Values that come for a note
//!   before its note-on (MPE controllers send them just ahead, and keys
//!   reach MIDI out before the arp plays their notes) are kept and go out
//!   on its channel just before it. A channel a note takes is put back at
//!   rest first (bend in the middle, no pressure), where it wasn't.
//! - **The whole zone.** The pitch wheel bends the master channel over the
//!   gear's own range (its profile's `bend`, else 2). Program, bank and
//!   controllers stay on the master channel, which is the track's channel
//!   for MPE gear (the app's `Applied::of`).
//! - **Configuration.** [`MpeOut::configure`] is MPE's configuration
//!   message (RPN 6 on the master channel: the zone's member count), sent
//!   with the track's program, so gear that follows it takes the zone.
//!
//! Messages carry their channel; nothing allocates.

use crate::mpe::{Zone, MEMBER_BEND, TONE_CC};
use crate::msg::Msg;

/// No channel (a note not on, a value not sent).
const NONE: u8 = 0xFF;

/// A note's expression waiting for its note-on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Early {
    slide: Option<f32>,
    pressure: Option<u8>,
    tone: Option<u8>,
}

/// One track's notes on MPE gear.
#[derive(Clone, Debug)]
pub struct MpeOut {
    zone: Zone,
    /// The member channel each note is on (NONE: off).
    chan_of: [u8; 128],
    /// Notes on each channel.
    notes: [u8; 16],
    /// When each channel last took a note, and last went free: a count of
    /// note-ons and note-offs.
    taken_at: [u32; 16],
    freed_at: [u32; 16],
    clock: u32,
    /// What each channel was last sent: bend (-8192 to 8191), pressure and
    /// tone (None: never).
    bend: [i16; 16],
    pressure: [u8; 16],
    tone: [Option<u8>; 16],
    /// The pitch wheel's bend last sent on the master channel.
    wheel: i16,
    /// Values that came for notes not on yet.
    early: [Early; 128],
}

impl Default for MpeOut {
    fn default() -> Self {
        Self::new()
    }
}

impl MpeOut {
    /// For no zone: [`MpeOut::set_zone`] gives it one.
    pub fn new() -> MpeOut {
        MpeOut {
            zone: Zone::Off,
            chan_of: [NONE; 128],
            notes: [0; 16],
            taken_at: [0; 16],
            freed_at: [0; 16],
            clock: 0,
            bend: [0; 16],
            pressure: [0; 16],
            tone: [None; 16],
            wheel: 0,
            early: [Early::default(); 128],
        }
    }

    pub fn zone(&self) -> Zone {
        self.zone
    }

    /// The gear's zone. A new one forgets every note and value (the
    /// router ends the notes where they started).
    pub fn set_zone(&mut self, zone: Zone) {
        if zone != self.zone {
            *self = MpeOut::new();
            self.zone = zone;
        }
    }

    /// The member channel note `note` is on, if it is.
    pub fn channel_of(&self, note: u8) -> Option<u8> {
        Some(self.chan_of[usize::from(note & 0x7F)]).filter(|&c| c != NONE)
    }

    /// MPE's configuration message for the zone: RPN 6 on the master
    /// channel with the member count, then the null RPN so a later data
    /// entry changes nothing. Nothing for no zone.
    pub fn configure(&self, mut send: impl FnMut(Msg)) {
        let (Some(ch), Some(n)) = (self.zone.master(), members(self.zone)) else {
            return;
        };
        for (cc, value) in [(101, 0), (100, 6), (6, n), (101, 127), (100, 127)] {
            send(Msg::Cc { ch, cc, value });
        }
    }

    /// A note-on: its member channel, put at rest with the note's early
    /// values, then the note. A note already on ends first. Nothing for no
    /// zone.
    pub fn note_on(&mut self, note: u8, vel: u8, mut send: impl FnMut(Msg)) {
        let note = note & 0x7F;
        if self.channel_of(note).is_some() {
            self.note_off(note, &mut send);
        }
        let Some(ch) = self.pick() else { return };
        self.clock = self.clock.wrapping_add(1);
        self.taken_at[usize::from(ch)] = self.clock;
        self.notes[usize::from(ch)] += 1;
        self.chan_of[usize::from(note)] = ch;
        let early = std::mem::take(&mut self.early[usize::from(note)]);
        // A free channel starts from rest; a shared one keeps what its
        // other note has.
        if self.notes[usize::from(ch)] == 1 {
            self.set_bend(ch, early.slide.unwrap_or(0.0), &mut send);
            self.set_pressure(ch, early.pressure.unwrap_or(0), &mut send);
        }
        if let Some(t) = early.tone {
            self.set_tone(ch, t, &mut send);
        }
        send(Msg::NoteOn {
            ch,
            note,
            vel: vel.clamp(1, 127),
        });
    }

    /// A note-off, on the channel its note-on went to. Nothing for a note
    /// not on (its early values are dropped).
    pub fn note_off(&mut self, note: u8, mut send: impl FnMut(Msg)) {
        let note = note & 0x7F;
        self.early[usize::from(note)] = Early::default();
        let Some(ch) = self.channel_of(note) else {
            return;
        };
        self.chan_of[usize::from(note)] = NONE;
        let n = &mut self.notes[usize::from(ch)];
        *n = n.saturating_sub(1);
        if *n == 0 {
            self.clock = self.clock.wrapping_add(1);
            self.freed_at[usize::from(ch)] = self.clock;
        }
        send(Msg::NoteOff { ch, note, vel: 64 });
    }

    /// A key let go of: values kept for its note, if the note never came
    /// (the MIDI effects moved it, or the arp didn't play it), are dropped
    /// so no later note of that pitch takes them. A note on keeps its
    /// channel until its own note-off.
    pub fn key_up(&mut self, note: u8) {
        self.early[usize::from(note & 0x7F)] = Early::default();
    }

    /// A note's own slide, in semitones: its channel's bend.
    pub fn slide(&mut self, note: u8, semis: f32, mut send: impl FnMut(Msg)) {
        let semis = if semis.is_finite() { semis } else { 0.0 };
        match self.channel_of(note) {
            Some(ch) => self.set_bend(ch, semis, &mut send),
            None => self.early[usize::from(note & 0x7F)].slide = Some(semis),
        }
    }

    /// A note's own pressure (0 to 127): its channel's pressure.
    pub fn pressure(&mut self, note: u8, value: u8, mut send: impl FnMut(Msg)) {
        let value = value.min(127);
        match self.channel_of(note) {
            Some(ch) => self.set_pressure(ch, value, &mut send),
            None => self.early[usize::from(note & 0x7F)].pressure = Some(value),
        }
    }

    /// A note's own tone (0 to 1): its channel's CC 74.
    pub fn tone(&mut self, note: u8, value: f32, mut send: impl FnMut(Msg)) {
        if !value.is_finite() {
            return;
        }
        let v = (value.clamp(0.0, 1.0) * 127.0).round() as u8;
        match self.channel_of(note) {
            Some(ch) => self.set_tone(ch, v, &mut send),
            None => self.early[usize::from(note & 0x7F)].tone = Some(v),
        }
    }

    /// The pitch wheel (-1 to 1): the master channel's bend, which the
    /// gear spreads over its own range for every note. Only a change goes.
    pub fn wheel(&mut self, v: f32, mut send: impl FnMut(Msg)) {
        let Some(ch) = self.zone.master() else { return };
        let v = if v.is_finite() {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        let b = ((v * 8192.0).round() as i32).clamp(-8192, 8191) as i16;
        if b != self.wheel {
            self.wheel = b;
            send(Msg::PitchBend { ch, value: b });
        }
    }

    /// Every channel back at rest (a stop, a panic, the track's route
    /// changed): bend in the middle and no pressure where they weren't,
    /// the master's bend too, and every note and early value forgotten.
    /// The router ends the notes; the zone is kept.
    pub fn release(&mut self, mut send: impl FnMut(Msg)) {
        if let Some(ch) = self.zone.master().filter(|_| self.wheel != 0) {
            send(Msg::PitchBend { ch, value: 0 });
        }
        for ch in 0..16u8 {
            if !self.zone.is_member(ch) {
                continue;
            }
            if self.bend[usize::from(ch)] != 0 {
                send(Msg::PitchBend { ch, value: 0 });
            }
            if self.pressure[usize::from(ch)] != 0 {
                send(Msg::ChanPressure { ch, value: 0 });
            }
        }
        self.reset();
    }

    /// Forgets every note and value without a message (the gear's output
    /// went).
    pub fn reset(&mut self) {
        let zone = self.zone;
        *self = MpeOut::new();
        self.zone = zone;
    }

    /// The member channel the next note takes: the free one free longest,
    /// else the one whose newest note started longest ago.
    fn pick(&self) -> Option<u8> {
        let members = (0..16u8).filter(|&c| self.zone.is_member(c));
        let free = members
            .clone()
            .filter(|&c| self.notes[usize::from(c)] == 0)
            .min_by_key(|&c| self.freed_at[usize::from(c)]);
        free.or_else(|| members.min_by_key(|&c| self.taken_at[usize::from(c)]))
    }

    fn set_bend(&mut self, ch: u8, semis: f32, send: &mut impl FnMut(Msg)) {
        let b = (semis / f32::from(MEMBER_BEND)).clamp(-1.0, 1.0);
        let b = ((b * 8192.0).round() as i32).clamp(-8192, 8191) as i16;
        if b != self.bend[usize::from(ch)] {
            self.bend[usize::from(ch)] = b;
            send(Msg::PitchBend { ch, value: b });
        }
    }

    fn set_pressure(&mut self, ch: u8, value: u8, send: &mut impl FnMut(Msg)) {
        if value != self.pressure[usize::from(ch)] {
            self.pressure[usize::from(ch)] = value;
            send(Msg::ChanPressure { ch, value });
        }
    }

    fn set_tone(&mut self, ch: u8, value: u8, send: &mut impl FnMut(Msg)) {
        if self.tone[usize::from(ch)] != Some(value) {
            self.tone[usize::from(ch)] = Some(value);
            send(Msg::Cc {
                ch,
                cc: TONE_CC,
                value,
            });
        }
    }
}

/// The zone's member count (1 to 15), None for no zone.
fn members(zone: Zone) -> Option<u8> {
    match zone {
        Zone::Off => None,
        Zone::Lower(n) | Zone::Upper(n) => Some(n.clamp(1, 15)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(f: impl FnOnce(&mut dyn FnMut(Msg))) -> Vec<Msg> {
        let mut out = Vec::new();
        f(&mut |m| out.push(m));
        out
    }

    fn on(m: &mut MpeOut, note: u8) -> Vec<Msg> {
        run(|s| m.note_on(note, 100, s))
    }

    fn off(m: &mut MpeOut, note: u8) -> Vec<Msg> {
        run(|s| m.note_off(note, s))
    }

    fn note_on(ch: u8, note: u8) -> Msg {
        Msg::NoteOn { ch, note, vel: 100 }
    }

    /// Lower zone of 15: master channel 1 (0), members 2 to 16 (1 to 15).
    /// Each note takes the free channel free longest; its off goes there.
    #[test]
    fn each_note_takes_a_member_channel_of_its_own() {
        let mut m = MpeOut::new();
        m.set_zone(Zone::Lower(15));
        assert_eq!(on(&mut m, 60), [note_on(1, 60)]);
        assert_eq!(on(&mut m, 64), [note_on(2, 64)]);
        assert_eq!(on(&mut m, 67), [note_on(3, 67)]);
        assert_eq!(
            off(&mut m, 64),
            [Msg::NoteOff {
                ch: 2,
                note: 64,
                vel: 64
            }]
        );
        // Channel 2 was freed last: the next note takes 4, the one free
        // longest (never used).
        assert_eq!(on(&mut m, 72), [note_on(4, 72)]);
        assert_eq!(m.channel_of(60), Some(1));
        // A note not on ends nothing.
        assert!(off(&mut m, 50).is_empty());
        // Upper zone of 3: members 15 down to 13 (14 to 12 from 0).
        let mut u = MpeOut::new();
        u.set_zone(Zone::Upper(3));
        let chans: Vec<u8> = [60, 61, 62]
            .iter()
            .map(|&n| {
                on(&mut u, n);
                u.channel_of(n).unwrap()
            })
            .collect();
        assert_eq!(chans, [12, 13, 14]);
        // No zone: nothing goes.
        let mut none = MpeOut::new();
        assert!(on(&mut none, 60).is_empty());
    }

    /// A slide bends only its note's channel over 48 semitones; pressure
    /// and tone go on its channel; only a change goes.
    #[test]
    fn a_notes_slide_pressure_and_tone_stay_on_its_channel() {
        let mut m = MpeOut::new();
        m.set_zone(Zone::Lower(15));
        on(&mut m, 60);
        on(&mut m, 64);
        assert_eq!(
            run(|s| m.slide(64, 12.0, s)),
            [Msg::PitchBend { ch: 2, value: 2048 }],
            "12 of 48 semitones"
        );
        assert!(run(|s| m.slide(64, 12.0, s)).is_empty());
        assert_eq!(
            run(|s| m.pressure(60, 90, s)),
            [Msg::ChanPressure { ch: 1, value: 90 }]
        );
        assert_eq!(
            run(|s| m.tone(60, 1.0, s)),
            [Msg::Cc {
                ch: 1,
                cc: 74,
                value: 127
            }]
        );
    }

    /// Values for a key whose note never played go when the key does, so
    /// a later note of that pitch starts at rest.
    #[test]
    fn a_keys_values_go_with_it_when_its_note_never_came() {
        let mut m = MpeOut::new();
        m.set_zone(Zone::Lower(15));
        assert!(run(|s| m.slide(60, 12.0, s)).is_empty());
        m.key_up(60);
        assert_eq!(on(&mut m, 60), [note_on(1, 60)]);
    }

    /// Values that come before a note's on go out on its channel just
    /// before it; a channel taken again starts from rest.
    #[test]
    fn early_values_lead_their_note_and_a_reused_channel_starts_at_rest() {
        let mut m = MpeOut::new();
        m.set_zone(Zone::Lower(1));
        assert!(run(|s| m.slide(60, -24.0, s)).is_empty());
        assert!(run(|s| m.pressure(60, 40, s)).is_empty());
        assert!(run(|s| m.tone(60, 0.5, s)).is_empty());
        assert_eq!(
            on(&mut m, 60),
            [
                Msg::PitchBend {
                    ch: 1,
                    value: -4096
                },
                Msg::ChanPressure { ch: 1, value: 40 },
                Msg::Cc {
                    ch: 1,
                    cc: 74,
                    value: 64
                },
                note_on(1, 60),
            ]
        );
        off(&mut m, 60);
        // The one member again: back to rest before the next note.
        assert_eq!(
            on(&mut m, 62),
            [
                Msg::PitchBend { ch: 1, value: 0 },
                Msg::ChanPressure { ch: 1, value: 0 },
                note_on(1, 62),
            ]
        );
    }

    /// With every member busy a note shares the channel whose newest note
    /// started longest ago; the same note again ends first.
    #[test]
    fn out_of_channels_a_note_shares_the_oldest() {
        let mut m = MpeOut::new();
        m.set_zone(Zone::Lower(2));
        on(&mut m, 60);
        on(&mut m, 62);
        assert_eq!(on(&mut m, 64), [note_on(1, 64)], "60's channel");
        assert_eq!(on(&mut m, 65), [note_on(2, 65)], "62's channel");
        // 60 lets go: channel 1 still holds 64, so it isn't free.
        off(&mut m, 60);
        assert_eq!(on(&mut m, 67), [note_on(1, 67)], "oldest newest note");
        // The same note again: its off first, then a channel.
        let again = on(&mut m, 67);
        assert_eq!(
            again[0],
            Msg::NoteOff {
                ch: 1,
                note: 67,
                vel: 64
            }
        );
        assert_eq!(again.last(), Some(&note_on(2, 67)));
    }

    /// The configuration message: RPN 6 on the master channel, then the
    /// null RPN. Release puts each member back at rest where it wasn't.
    #[test]
    fn it_configures_the_zone_and_rests_its_channels() {
        let mut m = MpeOut::new();
        m.set_zone(Zone::Upper(7));
        let cc = |cc, value| Msg::Cc { ch: 15, cc, value };
        assert_eq!(
            run(|s| m.configure(s)),
            [cc(101, 0), cc(100, 6), cc(6, 7), cc(101, 127), cc(100, 127)]
        );
        on(&mut m, 60);
        let ch = m.channel_of(60).unwrap();
        run(|s| m.slide(60, 1.0, s));
        // The wheel bends the master channel, only on a change.
        assert_eq!(
            run(|s| m.wheel(0.5, s)),
            [Msg::PitchBend {
                ch: 15,
                value: 4096
            }]
        );
        assert!(run(|s| m.wheel(0.5, s)).is_empty());
        assert_eq!(
            run(|s| m.release(s)),
            [
                Msg::PitchBend { ch: 15, value: 0 },
                Msg::PitchBend { ch, value: 0 }
            ]
        );
        assert_eq!(m.channel_of(60), None);
        assert_eq!(m.zone(), Zone::Upper(7));
        assert!(run(|s| MpeOut::new().configure(s)).is_empty());
    }
}
