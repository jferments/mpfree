//! Expression in (powerhouse 3.7): what a keyboard does to its notes
//! besides playing them, for MPFree's own sounds.
//!
//! [`MpeIn`] reads one port's channel messages and says what each does to
//! the notes it holds ([`Expr`]):
//!
//! - **Any keyboard.** Pitch bend bends every note ([`Expr::Bend`], -1 to
//!   1, which the instrument's own bend range scales). Channel pressure
//!   (mono aftertouch) presses every note held on its channel
//!   ([`Expr::Pressure`]), so an instrument's pressure destinations work
//!   from a keyboard without poly aftertouch.
//! - **MPE** ([`Zone`]): each note on its own member channel, so that
//!   channel's bend slides only that note ([`Expr::Slide`], in semitones,
//!   48 across the bend's range unless the controller sets another with
//!   RPN 0), its pressure presses only that note and its CC 74 sets that
//!   note's tone ([`Expr::Tone`], 0 to 1). A member channel's values from
//!   before its note come with the note, as MPE controllers send them just
//!   before it. The zone's master channel bends and presses every note.
//!
//! A port's zone is its profile's `mpe` line, or what the controller says
//! with MPE's configuration message (RPN 6 on channel 1 for the lower
//! zone, 16 for the upper; 0 channels turns it off), which most MPE
//! controllers send when they start. RPN 0 on a member channel sets the
//! zone's slide range.
//!
//! Notes themselves are the caller's: [`MpeIn::msg`] returns false for
//! them (it only learns which channel holds which note). Nothing
//! allocates.

use crate::msg::Msg;

/// The bend range of a member channel unless the controller says another
/// (MPE's default).
pub const MEMBER_BEND: u8 = 48;
/// The CC that carries an MPE note's tone (its Y axis).
pub const TONE_CC: u8 = 74;
/// Notes one channel remembers holding (MPE puts one on each).
const PER_CHANNEL: usize = 8;

/// An MPE zone: its master channel and how many member channels follow
/// it (1 to 15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Zone {
    #[default]
    Off,
    /// Master channel 1, members 2 up.
    Lower(u8),
    /// Master channel 16, members 15 down.
    Upper(u8),
}

impl Zone {
    /// The master channel, counted from 0.
    pub fn master(self) -> Option<u8> {
        match self {
            Zone::Off => None,
            Zone::Lower(_) => Some(0),
            Zone::Upper(_) => Some(15),
        }
    }

    /// Whether channel `ch` (from 0) is one of the zone's members.
    pub fn is_member(self, ch: u8) -> bool {
        match self {
            Zone::Off => false,
            Zone::Lower(n) => (1..=n.clamp(1, 15)).contains(&ch),
            Zone::Upper(n) => (15 - n.clamp(1, 15)..15).contains(&ch),
        }
    }

    /// The saved words: `off`, `lower 15`, `upper 7`.
    pub fn to_text(self) -> String {
        match self {
            Zone::Off => "off".into(),
            Zone::Lower(n) => format!("lower {n}"),
            Zone::Upper(n) => format!("upper {n}"),
        }
    }

    pub fn parse(s: &str) -> Result<Zone, String> {
        let mut w = s.split_whitespace();
        let kind = w.next().unwrap_or("");
        let n = match w.next() {
            None => 15,
            Some(n) => n
                .parse::<u8>()
                .ok()
                .filter(|n| (1..=15).contains(n))
                .ok_or(format!("\"{n}\" is not 1 to 15 channels"))?,
        };
        match kind {
            "off" => Ok(Zone::Off),
            "lower" => Ok(Zone::Lower(n)),
            "upper" => Ok(Zone::Upper(n)),
            k => Err(format!("\"{k}\" is not off, lower or upper")),
        }
    }

    /// In words, for the log and the screen.
    pub fn words(self) -> String {
        match self {
            Zone::Off => "MPE off".into(),
            Zone::Lower(n) => format!("MPE lower zone, channels 2 to {}", n + 1),
            Zone::Upper(n) => format!("MPE upper zone, channels {} to 15", 15 - n),
        }
    }
}

/// What a message does to the notes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expr {
    /// Every note, -1 to 1 of the instrument's bend range.
    Bend(f32),
    /// One note's own slide, in semitones.
    Slide { note: u8, semis: f32 },
    /// One note's tone, 0 to 1.
    Tone { note: u8, value: f32 },
    /// One note's pressure, 0 to 127.
    Pressure { note: u8, value: u8 },
}

/// What a message did besides its expression.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Took {
    /// Not expression: the caller's (a note, a knob, a program).
    No,
    /// Expression, said through `out`.
    Expr,
    /// The controller set its MPE zone (RPN 6).
    Zone(Zone),
}

#[derive(Clone, Copy, Debug)]
struct Channel {
    notes: [u8; PER_CHANNEL],
    n: usize,
    bend: i16,
    pressure: u8,
    tone: Option<u8>,
    /// The RPN picked with CC 101 and 100 (127 and 127: none).
    rpn: (u8, u8),
}

const CHANNEL: Channel = Channel {
    notes: [0; PER_CHANNEL],
    n: 0,
    bend: 0,
    pressure: 0,
    tone: None,
    rpn: (127, 127),
};

impl Channel {
    fn press(&mut self, note: u8) {
        self.release(note);
        if self.n == PER_CHANNEL {
            self.notes.copy_within(1.., 0);
            self.n -= 1;
        }
        self.notes[self.n] = note;
        self.n += 1;
    }

    fn release(&mut self, note: u8) {
        if let Some(i) = self.notes[..self.n].iter().position(|n| *n == note) {
            self.notes.copy_within(i + 1..self.n, i);
            self.n -= 1;
        }
    }

    fn held(&self) -> &[u8] {
        &self.notes[..self.n]
    }
}

/// One port's expression, as its messages come.
#[derive(Clone, Debug)]
pub struct MpeIn {
    zone: Zone,
    /// The zone the port's setting gives (a profile's `mpe`); the
    /// controller's own RPN 6 changes the zone until this changes.
    set: Zone,
    /// The members' slide range, in semitones.
    range: u8,
    ch: [Channel; 16],
    /// The last bend for every note wasn't the middle.
    bent: bool,
}

impl Default for MpeIn {
    fn default() -> Self {
        MpeIn::new()
    }
}

impl MpeIn {
    pub fn new() -> MpeIn {
        MpeIn {
            zone: Zone::Off,
            set: Zone::Off,
            range: MEMBER_BEND,
            ch: [CHANNEL; 16],
            bent: false,
        }
    }

    pub fn zone(&self) -> Zone {
        self.zone
    }

    /// The zone the port's setting gives. A change takes over from what
    /// the controller said; the same again changes nothing.
    pub fn set_zone(&mut self, z: Zone) {
        if z != self.set {
            self.set = z;
            self.zone = z;
            self.range = MEMBER_BEND;
        }
    }

    /// Forgets the notes held and the channels' values (the port went).
    /// True when its bend for every note was off the middle, so the
    /// caller can put it back.
    pub fn reset(&mut self) -> bool {
        self.ch = [CHANNEL; 16];
        self.zone = self.set;
        self.range = MEMBER_BEND;
        std::mem::take(&mut self.bent)
    }

    /// One channel message from the port: its expression through `out`.
    pub fn msg(&mut self, msg: Msg, out: &mut impl FnMut(Expr)) -> Took {
        let zone = self.zone;
        match msg {
            Msg::NoteOn { ch, note, vel } if vel > 0 => {
                let c = &mut self.ch[usize::from(ch & 15)];
                c.press(note);
                if zone.is_member(ch) {
                    // What the controller sent for this note before it.
                    let c = *c;
                    if c.bend != 0 {
                        out(Expr::Slide {
                            note,
                            semis: slide(c.bend, self.range),
                        });
                    }
                    if let Some(t) = c.tone {
                        out(Expr::Tone {
                            note,
                            value: f32::from(t) / 127.0,
                        });
                    }
                    if c.pressure > 0 {
                        out(Expr::Pressure {
                            note,
                            value: c.pressure,
                        });
                    }
                }
                Took::No
            }
            Msg::NoteOn { ch, note, .. } | Msg::NoteOff { ch, note, .. } => {
                let c = &mut self.ch[usize::from(ch & 15)];
                c.release(note);
                if zone.is_member(ch) && c.n == 0 {
                    // The channel's values were the note's: the next note
                    // on it starts from what the controller sends for it.
                    c.bend = 0;
                    c.pressure = 0;
                    c.tone = None;
                }
                Took::No
            }
            Msg::PitchBend { ch, value } if zone.is_member(ch) => {
                let c = &mut self.ch[usize::from(ch & 15)];
                c.bend = value;
                for &note in c.held() {
                    out(Expr::Slide {
                        note,
                        semis: slide(value, self.range),
                    });
                }
                Took::Expr
            }
            Msg::PitchBend { value, .. } => {
                self.bent = value != 0;
                out(Expr::Bend(f32::from(value) / 8192.0));
                Took::Expr
            }
            Msg::ChanPressure { ch, value } => {
                if zone.is_member(ch) {
                    self.ch[usize::from(ch & 15)].pressure = value;
                }
                // The master presses every note of the zone; outside a
                // zone, a channel's pressure every note on it.
                let all = zone.master() == Some(ch);
                for (i, c) in self.ch.iter().enumerate() {
                    let i = i as u8;
                    if i == ch || (all && zone.is_member(i)) {
                        for &note in c.held() {
                            out(Expr::Pressure { note, value });
                        }
                    }
                }
                Took::Expr
            }
            Msg::Cc { ch, cc, value } if cc == TONE_CC && zone.is_member(ch) => {
                let c = &mut self.ch[usize::from(ch & 15)];
                c.tone = Some(value);
                for &note in c.held() {
                    out(Expr::Tone {
                        note,
                        value: f32::from(value) / 127.0,
                    });
                }
                Took::Expr
            }
            Msg::Cc { ch, cc, value } => self.rpn(ch, cc, value),
            _ => Took::No,
        }
    }

    /// RPN 0 (bend range) and 6 (MPE's configuration). The CCs still go
    /// on to Learn, as any CC does.
    fn rpn(&mut self, ch: u8, cc: u8, value: u8) -> Took {
        let c = &mut self.ch[usize::from(ch & 15)];
        match cc {
            101 => c.rpn.0 = value,
            100 => c.rpn.1 = value,
            6 => match c.rpn {
                (0, 0) if self.zone.is_member(ch) => self.range = value.min(96),
                (0, 6) if ch == 0 || ch == 15 => {
                    let z = match (ch, value.min(15)) {
                        (_, 0) => Zone::Off,
                        (0, n) => Zone::Lower(n),
                        (_, n) => Zone::Upper(n),
                    };
                    // A zone turned off only turns off its own side.
                    if z != Zone::Off || self.zone.master() == Some(ch) {
                        self.zone = z;
                        self.range = MEMBER_BEND;
                        return Took::Zone(z);
                    }
                }
                _ => {}
            },
            _ => {}
        }
        Took::No
    }
}

fn slide(bend: i16, range: u8) -> f32 {
    f32::from(bend) / 8192.0 * f32::from(range)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(m: &mut MpeIn, msgs: &[Msg]) -> (Vec<Expr>, Vec<Took>) {
        let mut got = Vec::new();
        let took = msgs
            .iter()
            .map(|&x| m.msg(x, &mut |e| got.push(e)))
            .collect();
        (got, took)
    }

    fn on(ch: u8, note: u8) -> Msg {
        Msg::NoteOn { ch, note, vel: 100 }
    }

    fn off(ch: u8, note: u8) -> Msg {
        Msg::NoteOff { ch, note, vel: 0 }
    }

    #[test]
    fn any_keyboards_bend_bends_every_note_and_its_pressure_presses_its_channels() {
        let mut m = MpeIn::new();
        let (got, took) = run(
            &mut m,
            &[
                on(0, 60),
                on(0, 64),
                on(1, 67),
                Msg::PitchBend { ch: 0, value: 4096 },
                Msg::ChanPressure { ch: 0, value: 90 },
                off(0, 60),
                Msg::ChanPressure { ch: 0, value: 30 },
            ],
        );
        assert_eq!(
            got,
            [
                Expr::Bend(0.5),
                Expr::Pressure {
                    note: 60,
                    value: 90
                },
                Expr::Pressure {
                    note: 64,
                    value: 90
                },
                Expr::Pressure {
                    note: 64,
                    value: 30
                },
            ]
        );
        assert_eq!(took[..3], [Took::No; 3]);
        assert_eq!(took[3..5], [Took::Expr; 2]);
        // The port going says the bend was off the middle, once.
        assert!(m.clone().reset());
        let mut back = m.clone();
        run(&mut back, &[Msg::PitchBend { ch: 0, value: 0 }]);
        assert!(!back.reset());
        // CC 74 is a plain knob outside a zone: Learn's.
        let (got, took) = run(
            &mut m,
            &[Msg::Cc {
                ch: 0,
                cc: 74,
                value: 9,
            }],
        );
        assert!(got.is_empty());
        assert_eq!(took, [Took::No]);
    }

    #[test]
    fn an_mpe_controllers_channels_move_their_own_notes() {
        let mut m = MpeIn::new();
        m.set_zone(Zone::Lower(15));
        let (got, _) = run(
            &mut m,
            &[
                // Sent just before the note: they come with it.
                Msg::PitchBend { ch: 1, value: 2048 },
                Msg::Cc {
                    ch: 1,
                    cc: 74,
                    value: 127,
                },
                on(1, 60),
                on(2, 64),
                Msg::PitchBend {
                    ch: 2,
                    value: -8192,
                },
                Msg::ChanPressure { ch: 2, value: 70 },
                // The master bends and presses every note.
                Msg::PitchBend { ch: 0, value: 8191 },
                Msg::ChanPressure { ch: 0, value: 5 },
            ],
        );
        assert_eq!(
            got,
            [
                Expr::Slide {
                    note: 60,
                    semis: 12.0
                },
                Expr::Tone {
                    note: 60,
                    value: 1.0
                },
                Expr::Slide {
                    note: 64,
                    semis: -48.0
                },
                Expr::Pressure {
                    note: 64,
                    value: 70
                },
                Expr::Bend(8191.0 / 8192.0),
                Expr::Pressure { note: 60, value: 5 },
                Expr::Pressure { note: 64, value: 5 },
            ]
        );
        // A channel's values end with its note: the next note on it starts
        // neutral.
        let (got, _) = run(&mut m, &[off(1, 60), on(1, 62)]);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn the_controller_sets_its_zone_and_range_with_rpns() {
        let mut m = MpeIn::new();
        let rpn = |ch, lsb, v| {
            [
                Msg::Cc {
                    ch,
                    cc: 101,
                    value: 0,
                },
                Msg::Cc {
                    ch,
                    cc: 100,
                    value: lsb,
                },
                Msg::Cc {
                    ch,
                    cc: 6,
                    value: v,
                },
            ]
        };
        // MPE's configuration on channel 16: the upper zone, 7 channels.
        let (_, took) = run(&mut m, &rpn(15, 6, 7));
        assert_eq!(took[2], Took::Zone(Zone::Upper(7)));
        assert!(m.zone().is_member(8) && m.zone().is_member(14));
        assert!(!m.zone().is_member(7) && !m.zone().is_member(15));
        // RPN 0 on a member: a 24-semitone slide.
        run(&mut m, &rpn(10, 0, 24));
        let (got, _) = run(
            &mut m,
            &[
                on(10, 50),
                Msg::PitchBend {
                    ch: 10,
                    value: 4096,
                },
            ],
        );
        assert_eq!(
            got,
            [Expr::Slide {
                note: 50,
                semis: 12.0
            }]
        );
        // The lower zone's off doesn't end the upper zone.
        let (_, took) = run(&mut m, &rpn(0, 6, 0));
        assert_eq!(took[2], Took::No);
        assert_eq!(m.zone(), Zone::Upper(7));
        let (_, took) = run(&mut m, &rpn(15, 6, 0));
        assert_eq!(took[2], Took::Zone(Zone::Off));
        // The port's setting wins over what the controller said, and stays.
        m.set_zone(Zone::Lower(15));
        assert_eq!(m.zone(), Zone::Lower(15));
        run(&mut m, &rpn(0, 6, 3));
        m.set_zone(Zone::Lower(15));
        assert_eq!(m.zone(), Zone::Lower(3), "the same setting changes nothing");
    }

    #[test]
    fn zones_read_back_and_say_their_channels() {
        for z in [Zone::Off, Zone::Lower(15), Zone::Upper(4)] {
            assert_eq!(Zone::parse(&z.to_text()), Ok(z));
        }
        assert_eq!(Zone::parse("lower"), Ok(Zone::Lower(15)));
        assert!(Zone::parse("lower 16").is_err());
        assert!(Zone::parse("middle").is_err());
        assert_eq!(Zone::Lower(15).words(), "MPE lower zone, channels 2 to 16");
        assert_eq!(Zone::Upper(4).words(), "MPE upper zone, channels 11 to 15");
    }
}
