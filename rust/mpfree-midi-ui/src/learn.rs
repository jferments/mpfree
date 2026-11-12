//! The MIDI section's "MIDI Learn" rows: each control learned on a device,
//! what it moves, how it moves it, and a way to forget it. Controls are
//! learned from the chain editor's Learn (the app arms it); this lists
//! them.

use mpfree_midi::learn::{Encoding, Mode, Saved, Source};
use mpfree_midi::setup::PortInput;

use crate::form::{Field, Row, Tone};
use crate::{note_name, IoModel};

/// How a learned control can move its parameter, in the order the field
/// steps through them, as the field names them.
const MODES: [(Mode, &str); 6] = [
    (Mode::Absolute, "Knob or fader"),
    (
        Mode::Relative(Encoding::TwosComplement),
        "Endless (1 up, 127 down)",
    ),
    (
        Mode::Relative(Encoding::Offset64),
        "Endless (65 up, 63 down)",
    ),
    (Mode::Relative(Encoding::SignBit), "Endless (1 up, 65 down)"),
    (Mode::Toggle, "Button: on, then off"),
    (Mode::Momentary, "Button: while held"),
];

/// "CC 74 · ch 3 · Keystep 37", as the I/O view and the log name a
/// learned control.
pub fn control_text(source: &Source, channel: u8, port: &str) -> String {
    let what = match *source {
        Source::Cc(n) => format!("CC {n}"),
        Source::Note(n) => format!("key {}", note_name(n)),
        Source::PitchBend => "pitch bend".to_string(),
        Source::ChanPressure => "channel pressure".to_string(),
    };
    format!("{what} · ch {} · {port}", channel + 1)
}

fn port_text(l: &Saved) -> &str {
    l.port.as_deref().unwrap_or("any port")
}

/// The rows after the section's others: a heading, then one row per
/// learned control (or one saying how to learn one).
pub(crate) fn rows(m: &IoModel, rows: &mut Vec<Row>) {
    rows.push(Row::heading("MIDI Learn", vec!["Works as", ""]));
    if m.setup.learn.is_empty() {
        rows.push(
            Row::item("learn:none", "No controls learned")
                .note("use Learn in the chain editor to add one", Tone::Faint),
        );
        return;
    }
    for (i, l) in m.setup.learn.iter().enumerate() {
        let name = m
            .live
            .learn_names
            .get(i)
            .cloned()
            .unwrap_or_else(|| l.target.to_string());
        let mode = MODES.iter().position(|x| x.0 == l.mode).unwrap_or(0);
        let row = Row::item(
            format!("learn:{i}"),
            control_text(&l.source, l.channel, port_text(l)),
        )
        .field(Field::choice("Works as", MODES[mode].1, mode, MODES.len()))
        .field(Field::choice("Forget", "Forget", 0, 1).tap_only());
        // The port is named in the title, so the notes leave it out (and
        // fit the row).
        let row = match &l.port {
            Some(p) if m.live.port(p).is_none() => {
                row.note(format!("not plugged in: moves {name}"), Tone::Warn)
            }
            Some(p) if m.setup.port(p).input == PortInput::PlaysTracks => {
                row.note("does nothing: its port only plays tracks", Tone::Warn)
            }
            _ => row.note(format!("moves {name}"), Tone::Normal),
        };
        rows.push(row);
    }
}

/// A step on a learned control's row: None when `key` isn't one.
pub(crate) fn step(m: &mut IoModel, key: &str, col: usize, delta: i32) -> Option<bool> {
    let i: usize = key.strip_prefix("learn:")?.parse().ok()?;
    if i >= m.setup.learn.len() {
        return Some(false);
    }
    Some(match col {
        0 => {
            let l = &mut m.setup.learn[i];
            let at = MODES.iter().position(|x| x.0 == l.mode).unwrap_or(0) as i32;
            let to = MODES[(at + delta).rem_euclid(MODES.len() as i32) as usize].0;
            let changed = to != l.mode;
            l.mode = to;
            changed
        }
        _ if delta == 0 => false,
        _ => {
            m.setup.learn.remove(i);
            if i < m.live.learn_names.len() {
                m.live.learn_names.remove(i);
            }
            true
        }
    })
}
