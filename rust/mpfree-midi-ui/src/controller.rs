//! The Controller section (powerhouse 3.10; Jay's card, 18 Nov 2026
//! 01:21Z: a Controller switch in I/O): controller mode's switch, the
//! port and channel it sends on, what the pads send, and the CC each
//! Q-Link sends, four banks of four. The app sends
//! (`mpfree-app`'s `controller_out.rs`); this only sets it up.
//!
//! The settings are the rig's, kept for every project; the switch is never
//! saved, so MPFree starts with the pads its own. While it is on, the
//! Q-Links send their CCs on every page, this one too: here the wheel,
//! - and + and a tap change the fields.

use std::time::Duration;

use mpfree_midi::setup::{ControllerSetup, PadNotes, PadPressure, CONTROLLER_KNOBS};

use crate::form::{clamp_step, Field, Form, Row, Tone};
use crate::midi::ports;
use crate::IoModel;

pub(crate) struct Controller;

/// The note "From" starts on when it is picked (C2, a drum machine's
/// kick).
const FROM: u8 = 36;
/// The CCs a Q-Link can send: 0 to 119 (120 up are channel mode
/// messages).
const CC_MAX: u8 = 119;

/// The outputs it can send to: each port with an output, then the one it
/// names if that isn't plugged in.
fn outputs(m: &IoModel) -> Vec<String> {
    let mut v: Vec<String> = ports(m)
        .into_iter()
        .filter(|(_, l)| l.is_some_and(|l| l.has_out))
        .map(|(n, _)| n)
        .collect();
    let named = &m.setup.controller.port;
    if !named.is_empty() && !v.contains(named) {
        v.push(named.clone());
    }
    v
}

fn pads_text(p: PadNotes) -> String {
    match p {
        PadNotes::Kit => "Kit notes".into(),
        PadNotes::From(_) => "One by one".into(),
    }
}

fn pressure_text(p: PadPressure) -> &'static str {
    match p {
        PadPressure::Off => "Off",
        PadPressure::Poly => "Poly",
        PadPressure::Channel => "Channel",
    }
}

/// What pressing a held pad harder sends.
fn pressure_note(p: PadPressure) -> &'static str {
    match p {
        PadPressure::Off => "pressing harder sends nothing",
        PadPressure::Poly => "each pad's own, on its note",
        PadPressure::Channel => "one value: the hardest pad held",
    }
}

/// What pads A1 and A16 send, in a few words: "A1 C#2, A16 D#3 · banks
/// A-H", "Juno-X's: A1 C1 ..." when the port's profile names its pads.
fn pads_note(m: &IoModel, c: &ControllerSetup) -> String {
    let profile = m.profile(&c.port);
    let name = |pad: usize| {
        c.pad_out(pad, profile)
            .map_or("nothing".to_string(), |(n, ch)| {
                if ch == c.channel {
                    crate::note_name(n)
                } else {
                    format!("{} ch {}", crate::note_name(n), ch + 1)
                }
            })
    };
    let whose = match (c.pads, profile) {
        (PadNotes::Kit, Some(p)) if p.pads() > 0 => format!("{}'s: ", p.name),
        _ => String::new(),
    };
    format!("{whose}A1 {}, A16 {} · banks A-H", name(0), name(15))
}

impl Form for Controller {
    fn label(&self) -> &'static str {
        crate::CONTROLLER
    }

    fn widths(&self) -> &'static [i32] {
        &[230, 130, 130, 130, 130]
    }

    fn banner(&self, m: &IoModel) -> Option<String> {
        (!m.live.built.notes_out).then(|| {
            "MIDI out is off (Prefs' Send MIDI and CV out): controller mode sends nothing \
             until it is on."
                .to_string()
        })
    }

    fn rows(&self, m: &IoModel, _now: Duration) -> Vec<Row> {
        let c = &m.setup.controller;
        let live = m.live.port(&c.port);
        let mut rows = vec![Row::heading(
            "The MPC as a controller for your gear",
            vec!["", "", "", ""],
        )];
        let on = Row::item("on", "Controller mode").field(Field::choice(
            "Controller",
            if c.on { "On" } else { "Off" },
            usize::from(c.on),
            2,
        ));
        let shown = m.setup.shown(&c.port);
        rows.push(match (c.on, c.port.is_empty(), live) {
            (false, _, _) => on.note("Off: the pads play MPFree", Tone::Faint),
            (true, true, _) => on.note("On, but no port is picked", Tone::Warn),
            (true, false, None) => on.note(format!("On: {shown} isn't plugged in"), Tone::Warn),
            (true, false, Some(_)) if !m.live.built.notes_out => {
                on.note("On: sends nothing, MIDI out is off", Tone::Warn)
            }
            (true, false, Some(_)) => on.note("On: the pads play your gear", Tone::Normal),
        });
        let outs = outputs(m);
        let at = outs.iter().position(|n| *n == c.port);
        let to = Row::item("to", "Sends to")
            .field(
                Field::choice(
                    "Port",
                    if c.port.is_empty() {
                        "Pick a port"
                    } else {
                        shown
                    },
                    at.unwrap_or(0),
                    outs.len().max(1),
                )
                .off(!outs.is_empty())
                .warn(!c.port.is_empty() && live.is_none()),
            )
            .field(Field::choice(
                "Channel",
                format!("Ch {}", c.channel + 1),
                usize::from(c.channel),
                16,
            ));
        rows.push(match live {
            _ if outs.is_empty() => to.note("no output is plugged in", Tone::Warn),
            None if !c.port.is_empty() => to.note("kept until it is plugged in", Tone::Warn),
            _ => to.note("kept for every project", Tone::Faint),
        });
        let from = match c.pads {
            PadNotes::From(n) => n,
            PadNotes::Kit => FROM,
        };
        let kit = c.pads == PadNotes::Kit;
        let press = PadPressure::ALL
            .iter()
            .position(|p| *p == c.pressure)
            .unwrap_or(0);
        rows.push(
            Row::item("pads", "Pads send")
                .note(pads_note(m, c), Tone::Normal)
                .field(Field::choice(
                    "Notes",
                    pads_text(c.pads),
                    usize::from(!kit),
                    2,
                ))
                .field(
                    Field::choice(
                        "From",
                        format!("From {}", crate::note_name(from)),
                        usize::from(from),
                        128,
                    )
                    .off(!kit),
                ),
        );
        rows.push(
            Row::item("press", "Pad pressure")
                .note(pressure_note(c.pressure), Tone::Faint)
                .field(Field::choice(
                    "Pressure",
                    pressure_text(c.pressure),
                    press,
                    PadPressure::ALL.len(),
                )),
        );
        rows.push(Row::heading(
            "Q-Links: what each sends",
            vec!["", "", "", ""],
        ));
        let profile = m.profile(&c.port);
        for b in 0..CONTROLLER_KNOBS / 4 {
            let ccs = &c.ccs[b * 4..b * 4 + 4];
            // Each field says what its CC moves on the gear (its profile's
            // name, else the MIDI spec's) and the note says the numbers.
            let numbers: Vec<String> = ccs.iter().map(|cc| cc.to_string()).collect();
            let mut row = Row::item(
                format!("q:{b}"),
                format!("Q-Links {}-{}", b * 4 + 1, b * 4 + 4),
            )
            .note(format!("CC {}", numbers.join(", ")), Tone::Faint);
            for (k, &cc) in ccs.iter().enumerate() {
                let text = mpfree_midi::profile::cc_name(profile, cc)
                    .map_or_else(|| format!("CC {cc}"), str::to_string);
                row = row.field(Field::choice(
                    ["Q-Link 1", "Q-Link 2", "Q-Link 3", "Q-Link 4"][k],
                    text,
                    usize::from(cc),
                    usize::from(CC_MAX) + 1,
                ));
            }
            rows.push(row);
        }
        rows
    }

    fn step(&self, m: &mut IoModel, key: &str, col: usize, delta: i32) -> bool {
        if delta == 0 {
            return false;
        }
        let before = m.setup.controller.clone();
        match (key, col) {
            ("on", _) => {
                let c = &mut m.setup.controller;
                c.on = !c.on;
            }
            ("to", 0) => {
                let outs = outputs(m);
                if outs.is_empty() {
                    return false;
                }
                let c = &mut m.setup.controller;
                let at = outs.iter().position(|n| *n == c.port);
                let to = match at {
                    // A tap on the last goes round to the first.
                    Some(i) if i + 1 == outs.len() && delta == 1 => 0,
                    Some(i) => clamp_step(i, delta, outs.len()),
                    None => 0,
                };
                c.port = outs[to].clone();
            }
            ("to", _) => {
                let c = &mut m.setup.controller;
                c.channel = clamp_step(usize::from(c.channel), delta, 16) as u8;
            }
            ("pads", 0) => {
                let c = &mut m.setup.controller;
                c.pads = match c.pads {
                    PadNotes::Kit => PadNotes::From(FROM),
                    PadNotes::From(_) => PadNotes::Kit,
                };
            }
            ("pads", 1) => {
                let c = &mut m.setup.controller;
                if let PadNotes::From(n) = c.pads {
                    c.pads = PadNotes::From(clamp_step(usize::from(n), delta, 128) as u8);
                }
            }
            ("pads", _) => return false,
            ("press", _) => {
                let c = &mut m.setup.controller;
                let n = PadPressure::ALL.len() as i32;
                let i = PadPressure::ALL
                    .iter()
                    .position(|p| *p == c.pressure)
                    .unwrap_or(0) as i32;
                c.pressure = PadPressure::ALL[(i + delta).rem_euclid(n) as usize];
            }
            (k, col) => {
                let Some(b) = k.strip_prefix("q:").and_then(|b| b.parse::<usize>().ok()) else {
                    return false;
                };
                let Some(cc) = m.setup.controller.ccs.get_mut(b * 4 + col.min(3)) else {
                    return false;
                };
                *cc = clamp_step(usize::from(*cc), delta, usize::from(CC_MAX) + 1) as u8;
            }
        }
        m.setup.controller != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::paint;
    use crate::form::FormSection;
    use crate::{sample, Shared};
    use mpfree_mixer_ui::{IoInput, IoSection};
    use std::sync::{Arc, Mutex};

    fn section() -> (Shared, FormSection<Controller>) {
        let model: Shared = Arc::new(Mutex::new(sample::rig()));
        let s = FormSection::new(Controller, model.clone());
        (model, s)
    }

    /// The switch, the port among the outputs plugged in, the channel,
    /// the pads' notes and pressure and a Q-Link's CC each step and are
    /// kept in the setup (the rig's).
    #[test]
    fn its_fields_set_controller_mode() {
        let (model, _) = section();
        let mut m = model.lock().unwrap();
        let f = Controller;
        let outs = outputs(&m);
        assert!(!outs.is_empty(), "the sample rig has outputs");
        assert!(f.step(&mut m, "on", 0, 1));
        assert!(m.setup.controller.on);
        assert!(f.step(&mut m, "to", 0, 1));
        assert_eq!(m.setup.controller.port, outs[0]);
        assert!(f.step(&mut m, "to", 1, 2));
        assert_eq!(m.setup.controller.channel, 2);
        assert!(
            !f.step(&mut m, "pads", 1, 1),
            "From is off for a kit's notes"
        );
        assert!(f.step(&mut m, "pads", 0, 1));
        assert!(f.step(&mut m, "pads", 1, -12));
        assert_eq!(m.setup.controller.pads, PadNotes::From(24));
        assert!(f.step(&mut m, "press", 0, 1));
        assert_eq!(m.setup.controller.pressure, PadPressure::Poly);
        assert!(f.step(&mut m, "q:3", 3, 200));
        assert_eq!(m.setup.controller.ccs[15], CC_MAX);
        assert!(!f.step(&mut m, "q:3", 3, 1), "at its end");
        // The rows say what the pads send.
        let rows = f.rows(&m, Duration::ZERO);
        let pads = rows.iter().find(|r| r.key == "pads").unwrap();
        assert_eq!(pads.note, "A1 C1, A16 D#2 · banks A-H");
        // MPFree's pad notes for a kit.
        m.setup.controller.pads = PadNotes::Kit;
        let rows = f.rows(&m, Duration::ZERO);
        let pads = rows.iter().find(|r| r.key == "pads").unwrap();
        assert!(pads.note.starts_with("A1 C#2"), "{}", pads.note);
    }

    /// Each Q-Link field says what its CC moves on the gear: its
    /// profile's name, else the MIDI spec's, else the number.
    #[test]
    fn the_qlinks_say_what_they_move() {
        let m = sample::controlling(sample::rig());
        let rows = Controller.rows(&m, Duration::ZERO);
        let q = |b: usize| rows.iter().find(|r| r.key == format!("q:{b}")).unwrap();
        let texts: Vec<&str> = q(0).fields.iter().map(|f| f.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Filter cutoff",
                "Filter resonance",
                "Env attack",
                "Env release"
            ]
        );
        assert_eq!(q(0).note, "CC 74, 71, 73, 72");
        assert_eq!(q(1).fields[0].text, "CC 20");
        let on = rows.iter().find(|r| r.key == "on").unwrap();
        assert_eq!(on.note, "On: the pads play your gear");
    }

    /// The section draws, and its wheel and + work its rows.
    #[test]
    fn it_draws_and_the_wheel_works_it() {
        let (model, mut s) = section();
        let _ = paint(&s);
        // With nothing picked, the first row (the switch) is.
        s.input(IoInput::Step(1));
        let _ = paint(&s);
        assert!(
            model.lock().unwrap().setup.controller.on,
            "+ on the first row"
        );
        // The wheel moves down to the port and +/- picks one.
        s.input(IoInput::Wheel(1));
        s.input(IoInput::Step(1));
        assert!(!model.lock().unwrap().setup.controller.port.is_empty());
        let _ = paint(&s);
    }
}
