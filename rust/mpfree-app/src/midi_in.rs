//! MIDI from outside the MPC: the 5-pin MIDI In jack and USB-MIDI keyboards
//! on the USB-A socket. Read only: nothing here ever sends a byte.
//!
//! - **MIDI In** is the controller's cable 2 ("MIDI Port"), seen in the
//!   owner's read-only captures of 2026-11-12
//!   (docs/dev/hardware/controller-protocol.md). The app already reads that
//!   cable with the controller's other two; [`MusicIn::bytes`] takes what
//!   came in on it. Its bytes are never read as the MPC's own pads, knobs or
//!   buttons: a keyboard on channel 1 would otherwise press buttons, and one
//!   on channel 10 would hit pads.
//! - **USB keyboards** are ALSA cards of their own. [`usb_midi_cards`] finds
//!   them in `/proc/asound/cards`, the app opens each one's first input for
//!   reading, and [`MusicIn::plugged`] / [`MusicIn::unplugged`] keep the
//!   port table (a keyboard plugged back in gets its old port number).
//!
//! Every message goes through `mpfree_midi`'s `InputRouter` in its
//! Selected mode: whatever plays on any port and channel plays the armed
//! track, as on Akai's MPC ("Keys play: the track on screen", agreed by the
//! owner on 2026-11-12). A note-off always ends what its note-on started.
//! The app turns the [`Key`]s into notes for the armed track.
//!
//! Clock (F8) and active sensing (FE) are counted, not listed: a keyboard
//! sending clock the whole time otherwise filled the log with 83,000 lines
//! in 29 minutes on the owner's unit (2026-11-12). The log says when clock
//! starts and stops arriving instead. No port is the clock source yet, so
//! nothing follows that clock.

use mpfree_midi::input::{In, InputRouter};
use mpfree_midi::msg::Msg;
pub use mpfree_midi::port::DIN;
use mpfree_midi::port::{PortId, Ports, MAX_PORTS};

/// What a keyboard did, for the armed track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Down {
        note: u8,
        velocity: u8,
    },
    Up {
        note: u8,
    },
    /// Poly aftertouch on a held key.
    Pressure {
        note: u8,
        value: u8,
    },
}

/// Clock counted as "stopped" this long after its last tick.
const CLOCK_GAP_S: f64 = 1.0;

/// Per port: what the log counts instead of listing.
#[derive(Clone, Copy, Debug, Default)]
struct Seen {
    clock_ticks: u64,
    sensing: u64,
    clock_running: bool,
    last_tick: f64,
    /// Inside a SysEx message, and its length so far.
    sysex: Option<usize>,
    sysex_messages: u64,
    messages: u64,
}

pub struct MusicIn {
    router: InputRouter,
    ports: Ports,
    seen: [Seen; MAX_PORTS],
}

impl Default for MusicIn {
    fn default() -> Self {
        Self::new()
    }
}

impl MusicIn {
    pub fn new() -> MusicIn {
        let mut router = InputRouter::new();
        // The app plays keys on the armed track itself, so the router only
        // needs one track to route to; it still pairs every note-off with
        // its note-on.
        router.set_selected(Some(0));
        let mut ports = Ports::new();
        ports.set_din(true);
        MusicIn {
            router,
            ports,
            seen: [Seen::default(); MAX_PORTS],
        }
    }

    /// The name the log uses for a port.
    pub fn label(&self, port: PortId) -> String {
        if port == DIN {
            return "MIDI In".to_string();
        }
        self.ports
            .get(port)
            .map_or_else(|| format!("port {}", port.0), |p| p.name.clone())
    }

    /// A USB keyboard was opened for reading: its port number, or None when
    /// every port number is taken.
    pub fn plugged(&mut self, name: &str) -> Option<PortId> {
        self.ports.plugged(name, true, false)
    }

    /// A USB keyboard went away: every key it held is let go.
    pub fn unplugged(&mut self, port: PortId, keys: &mut Vec<Key>) {
        self.router.port_gone(port, 0, |e| key_of(e, keys));
        self.ports.unplugged(port);
        if let Some(s) = self.seen.get_mut(usize::from(port.0)) {
            s.clock_running = false;
            s.sysex = None;
        }
    }

    /// Bytes that came in on `port` at `t` seconds: the keys they play go
    /// to `keys`, and the log gets one line per message (clock and active
    /// sensing only counted).
    pub fn bytes(
        &mut self,
        port: PortId,
        data: &[u8],
        t: f64,
        keys: &mut Vec<Key>,
        log: &mut dyn FnMut(String),
    ) {
        let p = usize::from(port.0);
        if p >= MAX_PORTS {
            return;
        }
        let label = self.label(port);
        for &b in data {
            let s = &mut self.seen[p];
            match b {
                0xF8 => {
                    s.clock_ticks += 1;
                    s.last_tick = t;
                    if !s.clock_running {
                        s.clock_running = true;
                        log(format!(
                            "+{t:.3} s {label}: clock arriving (each tick is counted, not listed; \
                             nothing follows it yet)"
                        ));
                    }
                }
                0xFE => {
                    s.sensing += 1;
                    if s.sensing == 1 {
                        log(format!(
                            "+{t:.3} s {label}: active sensing (counted, not listed)"
                        ));
                    }
                }
                0xFA | 0xFB | 0xFC | 0xFF => {
                    let what = match b {
                        0xFA => "start",
                        0xFB => "continue",
                        0xFC => "stop",
                        _ => "reset",
                    };
                    s.messages += 1;
                    log(format!("+{t:.3} s {label}: {b:02X} = {what}"));
                }
                0xF9 | 0xFD => {}
                0xF0 => s.sysex = Some(1),
                0xF7 => {
                    if let Some(n) = s.sysex.take() {
                        s.sysex_messages += 1;
                        s.messages += 1;
                        log(format!(
                            "+{t:.3} s {label}: SysEx, {} bytes = ignored",
                            n + 1
                        ));
                    }
                }
                0x80..=0xF6 => s.sysex = None,
                _ => {
                    if let Some(n) = s.sysex.as_mut() {
                        *n += 1;
                    }
                }
            }
            let frame = 0;
            let mut seen = None;
            self.router.feed(port, b, frame, |e| match e {
                In::Seen { msg, .. } => seen = Some(msg),
                other => key_of(other, keys),
            });
            if let Some(msg) = seen {
                self.seen[p].messages += 1;
                let (bytes, n) = msg.encode();
                log(format!(
                    "+{t:.3} s {label}: {} = {}",
                    hex(&bytes[..n]),
                    describe(&msg)
                ));
            }
        }
    }

    /// Once in a while: say when a port's clock stopped arriving.
    pub fn tick(&mut self, t: f64, log: &mut dyn FnMut(String)) {
        for p in 0..MAX_PORTS {
            let s = &mut self.seen[p];
            if s.clock_running && t - s.last_tick > CLOCK_GAP_S {
                s.clock_running = false;
                let ticks = s.clock_ticks;
                let label = self.label(PortId(p as u8));
                log(format!(
                    "+{t:.3} s {label}: clock stopped arriving ({ticks} ticks so far)"
                ));
            }
        }
    }

    /// One line per port that saw anything, for the report at the end.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        for p in 0..MAX_PORTS {
            let s = &self.seen[p];
            if s.messages + s.clock_ticks + s.sensing == 0 {
                continue;
            }
            lines.push(format!(
                "{}: {} messages ({} SysEx, ignored), {} clock ticks, {} active sensing",
                self.label(PortId(p as u8)),
                s.messages,
                s.sysex_messages,
                s.clock_ticks,
                s.sensing
            ));
        }
        let st = self.router.stats;
        if st.unrouted + st.bad_port > 0 {
            lines.push(format!(
                "MIDI in: {} messages no track took, {} for a port past the table",
                st.unrouted, st.bad_port
            ));
        }
        lines
    }
}

/// The router's notes as keys; CCs, bends and program changes wait for
/// MIDI Learn and the track settings.
fn key_of(e: In, keys: &mut Vec<Key>) {
    let In::Track { msg, .. } = e else {
        return;
    };
    match msg {
        Msg::NoteOn { note, vel, .. } => keys.push(Key::Down {
            note,
            velocity: vel,
        }),
        Msg::NoteOff { note, .. } => keys.push(Key::Up { note }),
        Msg::PolyPressure { note, value, .. } => keys.push(Key::Pressure { note, value }),
        _ => {}
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A message in words, channels counted from 1 and notes named as the
/// Synth tab names them (48 is C3).
pub fn describe(msg: &Msg) -> String {
    let name = |n: u8| crate::synth::note_name(n);
    match *msg {
        Msg::NoteOn { ch, note, vel } => {
            format!("note on · ch {} · {} · vel {vel}", ch + 1, name(note))
        }
        Msg::NoteOff { ch, note, .. } => format!("note off · ch {} · {}", ch + 1, name(note)),
        Msg::PolyPressure { ch, note, value } => {
            format!("pressure · ch {} · {} · {value}", ch + 1, name(note))
        }
        Msg::Cc { ch, cc, value } => format!("CC {cc} · ch {} · {value}", ch + 1),
        Msg::Program { ch, program } => format!("program {} · ch {}", program + 1, ch + 1),
        Msg::ChanPressure { ch, value } => format!("channel pressure · ch {} · {value}", ch + 1),
        Msg::PitchBend { ch, value } => format!("pitch bend · ch {} · {value:+}", ch + 1),
    }
}

/// USB-MIDI cards to listen to: every card in `/proc/asound/cards` (as
/// `(number, id, description)`) that has a MIDI device and is not one of
/// `skip` (the MPC's controller and its sound card), with the name it
/// gives itself.
pub fn usb_midi_cards(
    cards: &[(u32, String, String)],
    skip: &[u32],
    has_midi: impl Fn(u32) -> bool,
) -> Vec<(u32, String)> {
    cards
        .iter()
        .filter(|(n, _, _)| !skip.contains(n) && has_midi(*n))
        .map(|(n, id, desc)| {
            // "USB-Audio - Keystation 49" names the device; the id is the
            // short form ("Keystation").
            let name = desc.split(" - ").nth(1).unwrap_or(desc).trim();
            let name = if name.is_empty() { id.as_str() } else { name };
            (*n, name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(m: &mut MusicIn, port: PortId, data: &[u8]) -> (Vec<Key>, Vec<String>) {
        let mut keys = Vec::new();
        let mut lines = Vec::new();
        m.bytes(port, data, 1.0, &mut keys, &mut |l| lines.push(l));
        (keys, lines)
    }

    #[test]
    fn notes_on_any_channel_play_and_are_named() {
        let mut m = MusicIn::new();
        // Jay's keyboard in the first capture: channel 6, C3 at velocity 48.
        let (keys, lines) = run(&mut m, DIN, &[0x95, 0x30, 0x30, 0x85, 0x30, 0x00]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 48,
                    velocity: 48
                },
                Key::Up { note: 48 }
            ]
        );
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "+1.000 s MIDI In: 95 30 30 = note on · ch 6 · C3 · vel 48"
        );
        assert_eq!(
            lines[1],
            "+1.000 s MIDI In: 85 30 00 = note off · ch 6 · C3"
        );
    }

    #[test]
    fn channel_one_and_ten_are_keys_not_mpc_controls() {
        // On cable 1 and 0 these would be a button press and a pad hit; on
        // MIDI In they are just notes.
        let mut m = MusicIn::new();
        let (keys, _) = run(&mut m, DIN, &[0x90, 0x31, 0x7F, 0x99, 0x25, 0x40]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 0x31,
                    velocity: 0x7F
                },
                Key::Down {
                    note: 0x25,
                    velocity: 0x40
                }
            ]
        );
    }

    #[test]
    fn running_status_and_velocity_zero_note_off() {
        let mut m = MusicIn::new();
        let (keys, _) = run(&mut m, DIN, &[0x90, 60, 100, 64, 90, 60, 0, 64, 0]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 60,
                    velocity: 100
                },
                Key::Down {
                    note: 64,
                    velocity: 90
                },
                Key::Up { note: 60 },
                Key::Up { note: 64 }
            ]
        );
    }

    #[test]
    fn clock_is_counted_not_listed() {
        let mut m = MusicIn::new();
        let mut ticks = vec![0xF8; 2400];
        // A note in the middle of the clock still comes through whole.
        ticks.splice(1000..1000, [0x90, 60, 100]);
        let (keys, lines) = run(&mut m, DIN, &ticks);
        assert_eq!(keys.len(), 1);
        assert_eq!(
            lines.len(),
            2,
            "one line for the clock, one for the note: {lines:?}"
        );
        assert!(lines[0].contains("clock arriving"));
        let mut later = Vec::new();
        m.tick(1.5, &mut |l| later.push(l));
        assert!(later.is_empty(), "still within a second of the last tick");
        m.tick(2.5, &mut |l| later.push(l));
        assert_eq!(later.len(), 1);
        assert!(later[0].contains("clock stopped arriving (2400 ticks so far)"));
        assert!(m.summary()[0].contains("2400 clock ticks"));
    }

    #[test]
    fn active_sensing_once_then_counted() {
        let mut m = MusicIn::new();
        let (_, lines) = run(&mut m, DIN, &[0xFE; 50]);
        assert_eq!(lines.len(), 1);
        assert!(m.summary()[0].contains("50 active sensing"));
    }

    #[test]
    fn start_stop_and_sysex_are_listed() {
        let mut m = MusicIn::new();
        let (keys, lines) = run(
            &mut m,
            DIN,
            &[0xFA, 0xF0, 0x7E, 0x00, 0x06, 0x01, 0xF7, 0xFC],
        );
        assert!(keys.is_empty());
        assert_eq!(
            lines,
            vec![
                "+1.000 s MIDI In: FA = start".to_string(),
                "+1.000 s MIDI In: SysEx, 6 bytes = ignored".to_string(),
                "+1.000 s MIDI In: FC = stop".to_string(),
            ]
        );
    }

    #[test]
    fn unplugging_lets_go_of_held_keys() {
        let mut m = MusicIn::new();
        let usb = m.plugged("Keystation 49").expect("a port number");
        assert_ne!(usb, DIN);
        let (keys, lines) = run(&mut m, usb, &[0x90, 60, 100, 62, 80]);
        assert_eq!(keys.len(), 2);
        assert!(lines[0].contains("Keystation 49: 90 3C 64"));
        let mut gone = Vec::new();
        m.unplugged(usb, &mut gone);
        gone.sort_by_key(|k| match k {
            Key::Up { note } => *note,
            _ => 0,
        });
        assert_eq!(gone, vec![Key::Up { note: 60 }, Key::Up { note: 62 }]);
        // Plugged back in, it keeps its number.
        assert_eq!(m.plugged("Keystation 49"), Some(usb));
    }

    #[test]
    fn pressure_reaches_the_track_but_ccs_wait_for_learn() {
        let mut m = MusicIn::new();
        let (keys, lines) = run(&mut m, DIN, &[0x90, 60, 100, 0xA0, 60, 33, 0xB0, 74, 64]);
        assert_eq!(
            keys,
            vec![
                Key::Down {
                    note: 60,
                    velocity: 100
                },
                Key::Pressure {
                    note: 60,
                    value: 33
                }
            ]
        );
        assert!(lines[2].ends_with("B0 4A 40 = CC 74 · ch 1 · 64"));
    }

    #[test]
    fn describe_every_kind() {
        assert_eq!(
            describe(&Msg::Program { ch: 0, program: 11 }),
            "program 12 · ch 1"
        );
        assert_eq!(
            describe(&Msg::PitchBend { ch: 2, value: -100 }),
            "pitch bend · ch 3 · -100"
        );
        assert_eq!(
            describe(&Msg::ChanPressure { ch: 15, value: 7 }),
            "channel pressure · ch 16 · 7"
        );
    }

    #[test]
    fn usb_cards_skip_the_mpc_and_need_a_midi_device() {
        let cards = vec![
            (0, "MPCOne".to_string(), "USB-Audio - MPC One".to_string()),
            (
                1,
                "rockchiprk817co".to_string(),
                "rockchip,rk817-codec".to_string(),
            ),
            (
                2,
                "Keystation".to_string(),
                "USB-Audio - Keystation 49 MK3".to_string(),
            ),
            (
                3,
                "Camera".to_string(),
                "USB-Audio - USB Camera".to_string(),
            ),
        ];
        let found = usb_midi_cards(&cards, &[0, 1], |n| n != 3);
        assert_eq!(found, vec![(2, "Keystation 49 MK3".to_string())]);
    }
}
