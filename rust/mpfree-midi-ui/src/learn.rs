//! A learned control in words, as the app's log and the Monitor name it.
//! The learned controls themselves are listed and edited in the Learn
//! section (`crate::learn_section`).

use mpfree_midi::learn::Source;

use crate::note_name;

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
