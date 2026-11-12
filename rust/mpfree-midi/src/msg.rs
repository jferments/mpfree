//! Channel messages as values: what the routers, MIDI Learn and the CV
//! model work with. [`Msg::from_bytes`] reads what
//! `mpfree_controller::midi::Parser` gives back, [`Msg::encode`] writes
//! the bytes. A note-on with velocity 0 is read as a note-off, as MIDI
//! 1.0 says.

/// A MIDI channel, 0 to 15 (screens show 1 to 16).
pub type Channel = u8;

/// One channel voice message. Every field is in range when it comes from
/// [`Msg::from_bytes`]; the constructors that take user values clamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Msg {
    NoteOn {
        ch: Channel,
        note: u8,
        vel: u8,
    },
    NoteOff {
        ch: Channel,
        note: u8,
        vel: u8,
    },
    PolyPressure {
        ch: Channel,
        note: u8,
        value: u8,
    },
    Cc {
        ch: Channel,
        cc: u8,
        value: u8,
    },
    Program {
        ch: Channel,
        program: u8,
    },
    ChanPressure {
        ch: Channel,
        value: u8,
    },
    /// -8192 to 8191, 0 in the middle.
    PitchBend {
        ch: Channel,
        value: i16,
    },
}

impl Msg {
    /// From a channel status byte and its data bytes (`data2` is ignored
    /// for program change and channel pressure). None for anything that
    /// is not a channel voice message.
    pub fn from_bytes(status: u8, data1: u8, data2: u8) -> Option<Msg> {
        if !(0x80..0xF0).contains(&status) {
            return None;
        }
        let ch = status & 0x0F;
        let (d1, d2) = (data1 & 0x7F, data2 & 0x7F);
        Some(match status & 0xF0 {
            0x80 => Msg::NoteOff {
                ch,
                note: d1,
                vel: d2,
            },
            0x90 if d2 == 0 => Msg::NoteOff {
                ch,
                note: d1,
                vel: 64,
            },
            0x90 => Msg::NoteOn {
                ch,
                note: d1,
                vel: d2,
            },
            0xA0 => Msg::PolyPressure {
                ch,
                note: d1,
                value: d2,
            },
            0xB0 => Msg::Cc {
                ch,
                cc: d1,
                value: d2,
            },
            0xC0 => Msg::Program { ch, program: d1 },
            0xD0 => Msg::ChanPressure { ch, value: d1 },
            _ => Msg::PitchBend {
                ch,
                value: ((i16::from(d2) << 7) | i16::from(d1)) - 8192,
            },
        })
    }

    pub fn channel(&self) -> Channel {
        match *self {
            Msg::NoteOn { ch, .. }
            | Msg::NoteOff { ch, .. }
            | Msg::PolyPressure { ch, .. }
            | Msg::Cc { ch, .. }
            | Msg::Program { ch, .. }
            | Msg::ChanPressure { ch, .. }
            | Msg::PitchBend { ch, .. } => ch,
        }
    }

    /// The same message on another channel (`ch` is taken mod 16).
    pub fn on_channel(self, ch: Channel) -> Msg {
        let ch = ch & 0x0F;
        match self {
            Msg::NoteOn { note, vel, .. } => Msg::NoteOn { ch, note, vel },
            Msg::NoteOff { note, vel, .. } => Msg::NoteOff { ch, note, vel },
            Msg::PolyPressure { note, value, .. } => Msg::PolyPressure { ch, note, value },
            Msg::Cc { cc, value, .. } => Msg::Cc { ch, cc, value },
            Msg::Program { program, .. } => Msg::Program { ch, program },
            Msg::ChanPressure { value, .. } => Msg::ChanPressure { ch, value },
            Msg::PitchBend { value, .. } => Msg::PitchBend { ch, value },
        }
    }

    /// The note a note message is about.
    pub fn note(&self) -> Option<u8> {
        match *self {
            Msg::NoteOn { note, .. }
            | Msg::NoteOff { note, .. }
            | Msg::PolyPressure { note, .. } => Some(note),
            _ => None,
        }
    }

    /// Transposed by `semis`. A note pushed out of 0..=127 gives None, so
    /// a note-on and its note-off are dropped alike.
    pub fn transposed(self, semis: i8) -> Option<Msg> {
        let shift = |n: u8| -> Option<u8> {
            let t = i16::from(n) + i16::from(semis);
            (0..=127).contains(&t).then_some(t as u8)
        };
        Some(match self {
            Msg::NoteOn { ch, note, vel } => Msg::NoteOn {
                ch,
                note: shift(note)?,
                vel,
            },
            Msg::NoteOff { ch, note, vel } => Msg::NoteOff {
                ch,
                note: shift(note)?,
                vel,
            },
            Msg::PolyPressure { ch, note, value } => Msg::PolyPressure {
                ch,
                note: shift(note)?,
                value,
            },
            other => other,
        })
    }

    /// The status byte.
    pub fn status(&self) -> u8 {
        let kind = match self {
            Msg::NoteOff { .. } => 0x80,
            Msg::NoteOn { .. } => 0x90,
            Msg::PolyPressure { .. } => 0xA0,
            Msg::Cc { .. } => 0xB0,
            Msg::Program { .. } => 0xC0,
            Msg::ChanPressure { .. } => 0xD0,
            Msg::PitchBend { .. } => 0xE0,
        };
        kind | (self.channel() & 0x0F)
    }

    /// The bytes, status first; `len` is 2 or 3.
    pub fn encode(&self) -> ([u8; 3], usize) {
        let s = self.status();
        match *self {
            Msg::NoteOn { note, vel, .. } => ([s, note & 0x7F, vel.clamp(1, 127)], 3),
            Msg::NoteOff { note, vel, .. } => ([s, note & 0x7F, vel & 0x7F], 3),
            Msg::PolyPressure { note, value, .. } => ([s, note & 0x7F, value & 0x7F], 3),
            Msg::Cc { cc, value, .. } => ([s, cc & 0x7F, value & 0x7F], 3),
            Msg::Program { program, .. } => ([s, program & 0x7F, 0], 2),
            Msg::ChanPressure { value, .. } => ([s, value & 0x7F, 0], 2),
            Msg::PitchBend { value, .. } => {
                let v = (i32::from(value).clamp(-8192, 8191) + 8192) as u16;
                ([s, (v & 0x7F) as u8, (v >> 7) as u8], 3)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let all = [
            Msg::NoteOn {
                ch: 3,
                note: 60,
                vel: 100,
            },
            Msg::NoteOff {
                ch: 0,
                note: 0,
                vel: 0,
            },
            Msg::PolyPressure {
                ch: 15,
                note: 127,
                value: 9,
            },
            Msg::Cc {
                ch: 9,
                cc: 74,
                value: 127,
            },
            Msg::Program { ch: 1, program: 5 },
            Msg::ChanPressure { ch: 2, value: 64 },
            Msg::PitchBend {
                ch: 4,
                value: -8192,
            },
            Msg::PitchBend { ch: 4, value: 0 },
            Msg::PitchBend { ch: 4, value: 8191 },
        ];
        for m in all {
            let (b, len) = m.encode();
            let back = Msg::from_bytes(b[0], b[1], if len == 3 { b[2] } else { 0 }).unwrap();
            assert_eq!(back, m);
        }
    }

    #[test]
    fn velocity_zero_is_note_off() {
        assert_eq!(
            Msg::from_bytes(0x92, 61, 0),
            Some(Msg::NoteOff {
                ch: 2,
                note: 61,
                vel: 64
            })
        );
        assert_eq!(Msg::from_bytes(0xF8, 0, 0), None);
        assert_eq!(Msg::from_bytes(0x40, 0, 0), None);
    }

    #[test]
    fn transpose_drops_notes_out_of_range() {
        let m = Msg::NoteOn {
            ch: 0,
            note: 120,
            vel: 1,
        };
        assert_eq!(
            m.transposed(7),
            Some(Msg::NoteOn {
                ch: 0,
                note: 127,
                vel: 1
            })
        );
        assert_eq!(m.transposed(8), None);
        let cc = Msg::Cc {
            ch: 0,
            cc: 1,
            value: 2,
        };
        assert_eq!(cc.transposed(100), Some(cc));
    }
}
