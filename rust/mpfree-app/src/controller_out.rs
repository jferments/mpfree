//! Controller mode (powerhouse 3.10; Jay's card, 18 Nov 2026 01:21Z: a
//! Controller switch in I/O): while the I/O view's Controller section has
//! it on, the MPC's pads send notes and its Q-Links CCs out of one port,
//! and MPFree's sounds don't take the pads.
//!
//! - **Pads.** A pad sends its note (by [`ControllerSetup::pad_out`]:
//!   the port's profile's pad lines, MPFree's kit notes, or one note after
//!   another) with the velocity the pad velocity setting and Full Level
//!   give, and its note-off on the same note and channel even if the bank
//!   or the setup changed while it was held. PAD BANK picks A to H (SHIFT
//!   or a second press for E to H, as for kits). Pressure is off, poly
//!   aftertouch on the pad's note, or channel pressure (the hardest pad
//!   held, and 0 once the last is let go).
//! - **Q-Links.** Each sends its CC as an absolute value, 0 to 127, from
//!   the last value it sent (0 at first), one step per detent. Q-LINK
//!   steps the bank: Q-Links 1 to 4, 5 to 8, 9 to 12, 13 to 16.
//! - **Off.** Turning it off, or a change of port or channel, ends every
//!   note it left on first. MIDI out's own bookkeeping keeps them too
//!   ([`mpfree_midi::output::OutputRouter::controller`]), so a panic ends
//!   them.
//!
//! It only says what to send ([`Controller`]'s `send`); MIDI out sends it
//! while Prefs' Send MIDI and CV out is on. Nothing here allocates on a
//! pad or a turn.

use mpfree_midi::msg::Msg;
use mpfree_midi::port::PortId;
use mpfree_midi::profile::Profile;
use mpfree_midi::setup::{ControllerSetup, PadPressure, CONTROLLER_KNOBS};

/// The MPC's pads, and its Q-Links in one bank.
const PADS: usize = 16;
const KNOBS: usize = 4;
/// The banks of Q-Links: 16 in fours.
pub const BANKS: usize = CONTROLLER_KNOBS / KNOBS;
/// PAD BANK's A to H.
pub const PAD_BANKS: usize = 8;

/// What it plays by while it is on: the setup, the port's id (None while
/// it isn't plugged in) and its profile.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub setup: ControllerSetup,
    pub to: Option<PortId>,
    pub profile: Option<Profile>,
    /// The port's name as the screen shows it.
    pub shown: String,
}

#[derive(Clone, Debug, Default)]
pub struct Controller {
    plan: Option<Plan>,
    /// The Q-Links' bank (0: Q-Links 1 to 4).
    bank: usize,
    /// PAD BANK: 0 is A.
    pad_bank: usize,
    /// The value each Q-Link sent last.
    values: [Option<u8>; CONTROLLER_KNOBS],
    /// What each pad holds: the port, note and channel its note-on went to.
    held: [Option<(PortId, u8, u8)>; PADS],
    /// Each pad's pressure, for channel pressure.
    press: [u8; PADS],
    /// The channel pressure sent last.
    chan_press: u8,
    /// Something the screen shows changed.
    changed: bool,
}

impl Controller {
    pub fn new() -> Controller {
        Controller::default()
    }

    /// On: the pads and Q-Links are the controller's.
    pub fn on(&self) -> bool {
        self.plan.is_some()
    }

    pub fn plan(&self) -> Option<&Plan> {
        self.plan.as_ref()
    }

    /// The setup now (None: off). A change of port or channel, or off,
    /// ends the notes it left on first, through `send`.
    pub fn set(&mut self, plan: Option<Plan>, send: &mut impl FnMut(PortId, Msg)) {
        let plan = plan.filter(|p| p.setup.on);
        if plan == self.plan {
            return;
        }
        let moved = |a: &Option<Plan>, b: &Option<Plan>| {
            a.as_ref().map(|p| (p.to, p.setup.channel))
                != b.as_ref().map(|p| (p.to, p.setup.channel))
        };
        if plan.is_none() || moved(&self.plan, &plan) {
            self.let_go(send);
            self.values = [None; CONTROLLER_KNOBS];
        }
        if plan.is_none() {
            self.bank = 0;
            self.pad_bank = 0;
        }
        self.plan = plan;
        self.changed = true;
    }

    /// Ends every note it holds, and puts channel pressure back to 0.
    fn let_go(&mut self, send: &mut impl FnMut(PortId, Msg)) {
        for h in &mut self.held {
            if let Some((to, note, ch)) = h.take() {
                send(to, Msg::NoteOff { ch, note, vel: 64 });
            }
        }
        if self.chan_press > 0 {
            if let Some(p) = &self.plan {
                if let Some(to) = p.to {
                    send(
                        to,
                        Msg::ChanPressure {
                            ch: p.setup.channel,
                            value: 0,
                        },
                    );
                }
            }
        }
        self.press = [0; PADS];
        self.chan_press = 0;
    }

    /// Pad `pad` (0 to 15) pressed with `velocity` (already through the
    /// pad velocity setting). True when it was the controller's.
    pub fn pad_down(
        &mut self,
        pad: usize,
        velocity: u8,
        send: &mut impl FnMut(PortId, Msg),
    ) -> bool {
        let Some(p) = &self.plan else {
            return false;
        };
        if pad >= PADS {
            return true;
        }
        // A pad pressed again before its note-off (it can't, but a bank
        // change can't leave a note on either way).
        if let Some((to, note, ch)) = self.held[pad].take() {
            send(to, Msg::NoteOff { ch, note, vel: 64 });
        }
        let Some(to) = p.to else { return true };
        if let Some((note, ch)) = p
            .setup
            .pad_out(self.pad_bank * PADS + pad, p.profile.as_ref())
        {
            send(
                to,
                Msg::NoteOn {
                    ch,
                    note,
                    vel: velocity.clamp(1, 127),
                },
            );
            self.held[pad] = Some((to, note, ch));
        }
        true
    }

    /// Pad `pad` let go. True when it was the controller's.
    pub fn pad_up(&mut self, pad: usize, send: &mut impl FnMut(PortId, Msg)) -> bool {
        if self.plan.is_none() {
            return false;
        }
        if let Some((to, note, ch)) = self.held.get_mut(pad).and_then(Option::take) {
            send(to, Msg::NoteOff { ch, note, vel: 64 });
        }
        if let Some(p) = self.press.get_mut(pad) {
            *p = 0;
        }
        self.channel_pressure(send);
        true
    }

    /// Pressure on pad `pad`, 0 to 127. True when it was the
    /// controller's.
    pub fn pressure(&mut self, pad: usize, value: u8, send: &mut impl FnMut(PortId, Msg)) -> bool {
        let Some(p) = &self.plan else {
            return false;
        };
        let (Some(held), Some(press)) = (self.held.get(pad).copied(), self.press.get_mut(pad))
        else {
            return true;
        };
        let value = value.min(127);
        *press = value;
        match p.setup.pressure {
            PadPressure::Off => {}
            PadPressure::Poly => {
                if let Some((to, note, ch)) = held {
                    send(to, Msg::PolyPressure { ch, note, value });
                }
            }
            PadPressure::Channel => self.channel_pressure(send),
        }
        true
    }

    /// Channel pressure: the hardest pad held, when it changed.
    fn channel_pressure(&mut self, send: &mut impl FnMut(PortId, Msg)) {
        let Some(p) = &self.plan else { return };
        let (PadPressure::Channel, Some(to)) = (p.setup.pressure, p.to) else {
            return;
        };
        let v = self
            .press
            .iter()
            .zip(&self.held)
            .filter(|(_, h)| h.is_some())
            .map(|(v, _)| *v)
            .max()
            .unwrap_or(0);
        if v != self.chan_press {
            self.chan_press = v;
            send(
                to,
                Msg::ChanPressure {
                    ch: p.setup.channel,
                    value: v,
                },
            );
        }
    }

    /// Q-Link `knob` (0 to 3) turned `delta` detents. True when it was
    /// the controller's.
    pub fn turn(&mut self, knob: usize, delta: i32, send: &mut impl FnMut(PortId, Msg)) -> bool {
        let Some(p) = &self.plan else {
            return false;
        };
        if knob >= KNOBS || delta == 0 {
            return true;
        }
        let k = self.bank * KNOBS + knob;
        let was = self.values[k];
        let v = (i32::from(was.unwrap_or(0)) + delta).clamp(0, 127) as u8;
        if was == Some(v) {
            return true;
        }
        self.values[k] = Some(v);
        self.changed = true;
        if let Some(to) = p.to {
            send(
                to,
                Msg::Cc {
                    ch: p.setup.channel,
                    cc: p.setup.ccs[k],
                    value: v,
                },
            );
        }
        true
    }

    /// Q-LINK: the next bank of Q-Links. True when it was the
    /// controller's.
    pub fn next_bank(&mut self) -> bool {
        if self.plan.is_none() {
            return false;
        }
        self.bank = (self.bank + 1) % BANKS;
        self.changed = true;
        true
    }

    /// PAD BANK `button` (0 to 3), with SHIFT or not: the pads' bank, as
    /// for kits. True when it was the controller's.
    pub fn pad_bank_button(&mut self, button: u8, shift: bool) -> bool {
        if self.plan.is_none() {
            return false;
        }
        self.pad_bank = crate::app::bank_target(self.pad_bank, button, shift) % PAD_BANKS;
        self.changed = true;
        true
    }

    pub fn pad_bank(&self) -> usize {
        self.pad_bank
    }

    pub fn bank(&self) -> usize {
        self.bank
    }

    /// Whether pad `pad` holds a note now.
    pub fn held(&self, pad: usize) -> bool {
        self.held.get(pad).is_some_and(Option::is_some)
    }

    /// Whether pad `pad` of the bank showing sends a note.
    pub fn plays(&self, pad: usize) -> bool {
        self.plan.as_ref().is_some_and(|p| {
            p.setup
                .pad_out(self.pad_bank * PADS + pad, p.profile.as_ref())
                .is_some()
        })
    }

    /// The bank's four Q-Links: each one's CC, what the port's profile (or
    /// the MIDI spec) calls it, and the value it sent last.
    pub fn knobs(&self) -> [(u8, Option<String>, Option<u8>); KNOBS] {
        std::array::from_fn(|i| {
            let k = self.bank * KNOBS + i;
            let p = self.plan.as_ref();
            let cc = p.map_or(0, |p| p.setup.ccs[k]);
            let name = mpfree_midi::profile::cc_name(p.and_then(|p| p.profile.as_ref()), cc)
                .map(str::to_string);
            (cc, name, self.values[k])
        })
    }

    /// Something the screen shows changed since the last call.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi::setup::PadNotes;

    const OUT: PortId = PortId(3);

    fn plan(f: impl FnOnce(&mut ControllerSetup)) -> Option<Plan> {
        let mut setup = ControllerSetup {
            on: true,
            port: "Juno-X".into(),
            ..ControllerSetup::default()
        };
        f(&mut setup);
        Some(Plan {
            setup,
            to: Some(OUT),
            profile: None,
            shown: "Juno-X".into(),
        })
    }

    fn run(
        c: &mut Controller,
        f: impl FnOnce(&mut Controller, &mut dyn FnMut(PortId, Msg)),
    ) -> Vec<Msg> {
        let mut got = Vec::new();
        f(c, &mut |to, m| {
            assert_eq!(to, OUT);
            got.push(m)
        });
        got
    }

    /// The pads send their notes and let them go on the same note after
    /// PAD BANK moved; off, they are MPFree's again.
    #[test]
    fn the_pads_send_notes_through_the_banks_and_off_gives_them_back() {
        let mut c = Controller::new();
        let mut none = |_: PortId, _: Msg| panic!("nothing to send");
        assert!(!c.pad_down(0, 100, &mut none), "off: MPFree's");
        c.set(plan(|s| s.pads = PadNotes::From(36)), &mut none);
        assert!(c.on());
        let got = run(&mut c, |c, s| {
            c.pad_down(0, 100, &mut |t, m| s(t, m));
            c.pad_bank_button(1, false);
            c.pad_down(1, 90, &mut |t, m| s(t, m));
            c.pad_up(0, &mut |t, m| s(t, m));
        });
        assert_eq!(
            got,
            [
                Msg::NoteOn {
                    ch: 0,
                    note: 36,
                    vel: 100
                },
                Msg::NoteOn {
                    ch: 0,
                    note: 36 + 16 + 1,
                    vel: 90
                },
                Msg::NoteOff {
                    ch: 0,
                    note: 36,
                    vel: 64
                },
            ]
        );
        assert_eq!(c.pad_bank(), 1);
        // Off: the note still held ends, and the pads are MPFree's.
        let got = run(&mut c, |c, s| c.set(None, &mut |t, m| s(t, m)));
        assert_eq!(
            got,
            [Msg::NoteOff {
                ch: 0,
                note: 53,
                vel: 64
            }]
        );
        assert!(!c.on() && c.pad_bank() == 0);
    }

    /// Poly pressure goes on the pad's note; channel pressure is the
    /// hardest pad held, back to 0 when the last is let go.
    #[test]
    fn pressure_is_poly_or_the_hardest_pad_on_the_channel() {
        let mut c = Controller::new();
        c.set(plan(|s| s.pressure = PadPressure::Poly), &mut |_, _| {});
        let got = run(&mut c, |c, s| {
            c.pad_down(0, 100, &mut |t, m| s(t, m));
            c.pressure(0, 70, &mut |t, m| s(t, m));
            c.pressure(5, 70, &mut |t, m| s(t, m));
        });
        assert_eq!(
            got[1],
            Msg::PolyPressure {
                ch: 0,
                note: 37,
                value: 70
            }
        );
        assert_eq!(got.len(), 2, "pad 6 holds nothing");
        let mut c = Controller::new();
        c.set(plan(|s| s.pressure = PadPressure::Channel), &mut |_, _| {});
        let got = run(&mut c, |c, s| {
            c.pad_down(0, 100, &mut |t, m| s(t, m));
            c.pad_down(1, 100, &mut |t, m| s(t, m));
            c.pressure(0, 40, &mut |t, m| s(t, m));
            c.pressure(1, 90, &mut |t, m| s(t, m));
            c.pressure(0, 50, &mut |t, m| s(t, m));
            c.pad_up(1, &mut |t, m| s(t, m));
            c.pad_up(0, &mut |t, m| s(t, m));
        });
        let press: Vec<u8> = got
            .iter()
            .filter_map(|m| match m {
                Msg::ChanPressure { value, .. } => Some(*value),
                _ => None,
            })
            .collect();
        assert_eq!(press, [40, 90, 50, 0]);
    }

    /// A Q-Link sends its CC from the last value it sent, clamped; Q-LINK
    /// steps the bank; a new channel starts the values again.
    #[test]
    fn the_q_links_send_their_ccs_bank_by_bank() {
        let mut c = Controller::new();
        let mut none = |_: PortId, _: Msg| panic!("nothing to send");
        assert!(!c.turn(0, 1, &mut none) && !c.next_bank());
        c.set(plan(|s| s.ccs[5] = 74), &mut none);
        let got = run(&mut c, |c, s| {
            c.turn(0, 3, &mut |t, m| s(t, m));
            c.turn(0, -9, &mut |t, m| s(t, m));
            c.turn(0, -1, &mut |t, m| s(t, m));
            c.next_bank();
            c.turn(1, 200, &mut |t, m| s(t, m));
        });
        assert_eq!(
            got,
            [
                Msg::Cc {
                    ch: 0,
                    cc: 16,
                    value: 3
                },
                Msg::Cc {
                    ch: 0,
                    cc: 16,
                    value: 0
                },
                Msg::Cc {
                    ch: 0,
                    cc: 74,
                    value: 127
                },
            ]
        );
        assert_eq!(c.knobs()[1], (74, Some("Cutoff".into()), Some(127)));
        assert!(c.take_changed() && !c.take_changed());
        c.set(
            plan(|s| {
                s.ccs[5] = 74;
                s.channel = 4;
            }),
            &mut none,
        );
        assert_eq!(c.knobs()[1].2, None, "a new channel starts again");
        assert_eq!(c.bank(), 1, "the bank stays");
    }

    /// Without its port plugged in, the pads and Q-Links stay the
    /// controller's and send nothing.
    #[test]
    fn unplugged_it_takes_the_pads_and_sends_nothing() {
        let mut c = Controller::new();
        let mut none = |_: PortId, _: Msg| panic!("nothing to send");
        let mut p = plan(|_| {});
        p.as_mut().unwrap().to = None;
        c.set(p, &mut none);
        assert!(c.pad_down(0, 100, &mut none));
        assert!(c.pad_up(0, &mut none));
        assert!(c.turn(0, 1, &mut none));
    }
}
