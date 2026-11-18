//! Expression out (plan item 3.7's converters): what a keyboard's pitch
//! wheel and pressure, and an MPE controller's slide, pressure and tone on
//! each of its notes, become for gear on one MIDI channel (a track's
//! Plays on).
//!
//! Gear without MPE has one bend, one pressure and one CC 74 for all its
//! notes, so [`ExprOut`] makes them from the keys held:
//!
//! - **Bend**: the wheel plus the newest key's slide, over the gear's bend
//!   range ([`DEFAULT_BEND_SEMIS`] unless its profile's `bend` line says),
//!   so a slide on the note played last bends the gear as far.
//! - **Pressure**: the hardest key's, as channel pressure.
//! - **Tone**: the newest key's, as CC 74.
//!
//! A value goes out only when it changes. When the last key lets go, the
//! bend goes back to the wheel alone and the pressure to 0. Messages are on
//! channel 0; the router puts them on the track's. Nothing allocates.

use crate::msg::Msg;

/// The gear's bend range when its profile doesn't say: the MIDI spec's
/// default, ±2 semitones.
pub const DEFAULT_BEND_SEMIS: u8 = 2;

/// The keys it keeps track of; more are let go of the oldest first (their
/// slide and pressure no longer count).
pub const HELD: usize = 16;

/// The CC a note's tone goes out on (MPE's "timbre", and most synths'
/// filter cutoff).
pub const TONE_CC: u8 = 74;

/// What came in for the track's keys.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum In {
    Down {
        note: u8,
    },
    Up {
        note: u8,
    },
    /// The pitch wheel, -1 to 1 of the bend range.
    Wheel(f32),
    /// A key's own slide, in semitones.
    Slide {
        note: u8,
        semis: f32,
    },
    /// A key's own tone, 0 to 1.
    Tone {
        note: u8,
        value: f32,
    },
    /// A key's pressure, 0 to 127.
    Pressure {
        note: u8,
        value: u8,
    },
}

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    note: u8,
    pressure: u8,
    slide: f32,
}

/// One track's expression for its gear.
#[derive(Clone, Debug)]
pub struct ExprOut {
    /// Oldest first.
    held: [Held; HELD],
    len: usize,
    wheel: f32,
    /// What went out last: the bend (-8192 to 8191), the pressure and the
    /// tone (None: none yet).
    bend: i16,
    pressure: u8,
    tone: Option<u8>,
}

impl Default for ExprOut {
    fn default() -> Self {
        Self::new()
    }
}

impl ExprOut {
    pub fn new() -> ExprOut {
        ExprOut {
            held: [Held::default(); HELD],
            len: 0,
            wheel: 0.0,
            bend: 0,
            pressure: 0,
            tone: None,
        }
    }

    /// The keys held, oldest first.
    pub fn held(&self) -> impl Iterator<Item = u8> + '_ {
        self.held[..self.len].iter().map(|h| h.note)
    }

    /// Takes `i` for gear whose bend range is `semis` (1 to 96; 0 counts
    /// as [`DEFAULT_BEND_SEMIS`]), giving `send` what changes.
    pub fn take(&mut self, i: In, semis: u8, mut send: impl FnMut(Msg)) {
        match i {
            In::Down { note } => {
                self.forget(note);
                if self.len == HELD {
                    self.held.copy_within(1.., 0);
                    self.len -= 1;
                }
                self.held[self.len] = Held {
                    note,
                    pressure: 0,
                    slide: 0.0,
                };
                self.len += 1;
            }
            In::Up { note } => self.forget(note),
            In::Wheel(v) => {
                self.wheel = if v.is_finite() {
                    v.clamp(-1.0, 1.0)
                } else {
                    0.0
                }
            }
            In::Slide { note, semis } => {
                if let Some(h) = self.find(note) {
                    h.slide = if semis.is_finite() { semis } else { 0.0 };
                }
            }
            In::Pressure { note, value } => {
                if let Some(h) = self.find(note) {
                    h.pressure = value.min(127);
                }
            }
            In::Tone { note, value } => {
                if self.newest() == Some(note) && value.is_finite() {
                    let v = (value.clamp(0.0, 1.0) * 127.0).round() as u8;
                    if self.tone != Some(v) {
                        self.tone = Some(v);
                        send(Msg::Cc {
                            ch: 0,
                            cc: TONE_CC,
                            value: v,
                        });
                    }
                }
                return;
            }
        }
        // The bend: the wheel and the newest key's slide.
        let range = f32::from(if semis == 0 {
            DEFAULT_BEND_SEMIS
        } else {
            semis.min(96)
        });
        let slide = self.held[..self.len].last().map_or(0.0, |h| h.slide);
        let b = (self.wheel + slide / range).clamp(-1.0, 1.0);
        let b = ((b * 8192.0).round() as i32).clamp(-8192, 8191) as i16;
        if b != self.bend {
            self.bend = b;
            send(Msg::PitchBend { ch: 0, value: b });
        }
        // The pressure: the hardest key's.
        let p = self.held[..self.len]
            .iter()
            .map(|h| h.pressure)
            .max()
            .unwrap_or(0);
        if p != self.pressure {
            self.pressure = p;
            send(Msg::ChanPressure { ch: 0, value: p });
        }
    }

    /// The bend (-8192 to 8191) and channel pressure (0 to 127) last sent,
    /// for recording what the gear plays (the app's `expr_rec`).
    pub fn sent(&self) -> (i16, u8) {
        (self.bend, self.pressure)
    }

    /// Puts the gear back at rest (a stop, a panic, its route changed):
    /// the bend to the middle and the pressure to 0 if they weren't, and
    /// the keys let go of. The wheel is kept: a wheel still held bends
    /// again with the next key or move.
    pub fn release(&mut self, mut send: impl FnMut(Msg)) {
        if self.bend != 0 {
            send(Msg::PitchBend { ch: 0, value: 0 });
        }
        if self.pressure != 0 {
            send(Msg::ChanPressure { ch: 0, value: 0 });
        }
        self.reset();
    }

    /// The gear lets go of everything without being told (its output
    /// went): the bend and pressure it was sent are forgotten, the wheel
    /// kept.
    pub fn reset(&mut self) {
        self.len = 0;
        self.bend = 0;
        self.pressure = 0;
        self.tone = None;
    }

    fn newest(&self) -> Option<u8> {
        self.held[..self.len].last().map(|h| h.note)
    }

    fn find(&mut self, note: u8) -> Option<&mut Held> {
        self.held[..self.len].iter_mut().find(|h| h.note == note)
    }

    fn forget(&mut self, note: u8) {
        if let Some(i) = self.held[..self.len].iter().position(|h| h.note == note) {
            self.held.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(e: &mut ExprOut, i: In, semis: u8) -> Vec<Msg> {
        let mut out = Vec::new();
        e.take(i, semis, |m| out.push(m));
        out
    }

    fn bend(value: i16) -> Msg {
        Msg::PitchBend { ch: 0, value }
    }

    fn press(value: u8) -> Msg {
        Msg::ChanPressure { ch: 0, value }
    }

    /// The wheel bends at once, held keys or not, and only a change goes.
    #[test]
    fn the_wheel_bends() {
        let mut e = ExprOut::new();
        assert_eq!(take(&mut e, In::Wheel(0.5), 0), [bend(4096)]);
        assert!(take(&mut e, In::Wheel(0.5), 0).is_empty());
        assert_eq!(take(&mut e, In::Wheel(1.0), 0), [bend(8191)]);
        assert_eq!(take(&mut e, In::Wheel(-1.0), 0), [bend(-8192)]);
        assert_eq!(take(&mut e, In::Wheel(f32::NAN), 0), [bend(0)]);
    }

    /// The newest key's slide bends over the gear's range; an older key's
    /// doesn't; when the newest lets go, the one before it is newest.
    #[test]
    fn the_newest_keys_slide_bends_the_gear() {
        let mut e = ExprOut::new();
        take(&mut e, In::Down { note: 60 }, 2);
        take(&mut e, In::Down { note: 64 }, 2);
        assert_eq!(
            take(
                &mut e,
                In::Slide {
                    note: 64,
                    semis: 1.0
                },
                2
            ),
            [bend(4096)],
            "a semitone of two"
        );
        assert!(take(
            &mut e,
            In::Slide {
                note: 60,
                semis: 2.0
            },
            2
        )
        .is_empty());
        assert_eq!(
            take(
                &mut e,
                In::Slide {
                    note: 64,
                    semis: 3.0
                },
                12
            ),
            [bend(2048)],
            "a quarter of a 12-semitone range"
        );
        // 64 lets go: 60's slide (two semitones of 12) is the bend now.
        assert_eq!(take(&mut e, In::Up { note: 64 }, 12), [bend(1365)]);
        assert_eq!(take(&mut e, In::Up { note: 60 }, 12), [bend(0)]);
        // The wheel and a slide add up, kept in range.
        take(&mut e, In::Wheel(0.75), 2);
        take(&mut e, In::Down { note: 50 }, 2);
        assert_eq!(
            take(
                &mut e,
                In::Slide {
                    note: 50,
                    semis: 1.0
                },
                2
            ),
            [bend(8191)]
        );
        // A range of 0 is the default 2.
        take(&mut e, In::Wheel(0.0), 0);
        assert_eq!(e.bend, 4096);
    }

    /// The pressure is the hardest key's, and goes back to 0 when the
    /// last key lets go.
    #[test]
    fn the_hardest_key_presses() {
        let mut e = ExprOut::new();
        take(&mut e, In::Down { note: 60 }, 0);
        take(&mut e, In::Down { note: 64 }, 0);
        assert_eq!(
            take(
                &mut e,
                In::Pressure {
                    note: 60,
                    value: 40
                },
                0
            ),
            [press(40)]
        );
        assert_eq!(
            take(
                &mut e,
                In::Pressure {
                    note: 64,
                    value: 90
                },
                0
            ),
            [press(90)]
        );
        assert!(take(
            &mut e,
            In::Pressure {
                note: 60,
                value: 80
            },
            0
        )
        .is_empty());
        assert_eq!(take(&mut e, In::Up { note: 64 }, 0), [press(80)]);
        assert!(
            take(
                &mut e,
                In::Pressure {
                    note: 61,
                    value: 127
                },
                0
            )
            .is_empty(),
            "not held"
        );
        assert_eq!(take(&mut e, In::Up { note: 60 }, 0), [press(0)]);
    }

    /// The newest key's tone is CC 74; an older key's isn't sent.
    #[test]
    fn the_newest_keys_tone_is_cc_74() {
        let mut e = ExprOut::new();
        take(&mut e, In::Down { note: 60 }, 0);
        take(&mut e, In::Down { note: 64 }, 0);
        let cc = |value| Msg::Cc {
            ch: 0,
            cc: TONE_CC,
            value,
        };
        assert_eq!(
            take(
                &mut e,
                In::Tone {
                    note: 64,
                    value: 0.5
                },
                0
            ),
            [cc(64)]
        );
        assert!(take(
            &mut e,
            In::Tone {
                note: 64,
                value: 0.5
            },
            0
        )
        .is_empty());
        assert!(take(
            &mut e,
            In::Tone {
                note: 60,
                value: 1.0
            },
            0
        )
        .is_empty());
    }

    /// A key played again is the newest; past [`HELD`] keys the oldest
    /// goes; reset forgets what was sent.
    #[test]
    fn keys_are_kept_in_fixed_room() {
        let mut e = ExprOut::new();
        for n in 0..20 {
            take(&mut e, In::Down { note: n }, 0);
        }
        assert_eq!(e.held().count(), HELD);
        assert_eq!(e.held().next(), Some(4));
        take(&mut e, In::Down { note: 4 }, 0);
        assert_eq!(e.held().last(), Some(4));
        assert_eq!(e.held().count(), HELD);
        assert_eq!(
            take(&mut e, In::Pressure { note: 4, value: 9 }, 0),
            [press(9)]
        );
        e.reset();
        assert_eq!(e.held().count(), 0);
        take(&mut e, In::Down { note: 1 }, 0);
        assert_eq!(
            take(&mut e, In::Pressure { note: 1, value: 9 }, 0),
            [press(9)],
            "sent again after a reset"
        );
    }

    /// Release puts the gear back at rest, sending only what isn't, and
    /// keeps the wheel.
    #[test]
    fn release_puts_the_gear_at_rest() {
        let mut e = ExprOut::new();
        let release = |e: &mut ExprOut| {
            let mut out = Vec::new();
            e.release(|m| out.push(m));
            out
        };
        assert!(release(&mut e).is_empty());
        take(&mut e, In::Wheel(0.25), 0);
        take(&mut e, In::Down { note: 60 }, 0);
        take(
            &mut e,
            In::Pressure {
                note: 60,
                value: 70,
            },
            0,
        );
        assert_eq!(release(&mut e), [bend(0), press(0)]);
        assert_eq!(e.held().count(), 0);
        assert!(release(&mut e).is_empty());
        // The wheel is still held: the next key bends the gear again.
        assert_eq!(take(&mut e, In::Down { note: 62 }, 0), [bend(2048)]);
    }
}
