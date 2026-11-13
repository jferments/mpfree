//! The mixer's I/O view settings kept between starts.
//!
//! The project's MIDI settings (each port's input and thru, each track's
//! "Plays on", the tempo source, clock out, the CV outputs and MIDI Learn)
//! are the project's `midi` part (`mpfree_midi_ui::MidiPart`). Until the app
//! saves and opens whole projects, the same text is kept in the store as
//! `files/midi/setup.txt`, so the I/O view comes back as it was left:
//!
//! - [`restore`] reads it once at start, before the sound loop syncs the
//!   model, through the part's own `load` (lines it can't read are kept as
//!   warnings, shown in the MIDI section and logged).
//! - [`Saver::take`] runs a few times a second in the sound loop. It only
//!   `try_lock`s the model, and hands back the setup's text when the model's
//!   `gen` has moved and the text differs from what was last kept; the
//!   store's own thread writes it.
//!
//! Once a project is opened, its `midi` part replaces what was restored, and
//! the next change is kept here as before.

use std::sync::TryLockError;

use mpfree_library::Part;
use mpfree_midi_ui::{MidiPart, Shared};

/// The owner and file the setup is kept under in the store.
pub const OWNER: &str = mpfree_midi_ui::PART;
pub const FILE: &str = "setup.txt";

/// Puts the kept setup into `model`. Gives the lines that couldn't be read,
/// for the log (they also show in the MIDI section).
pub fn restore(model: &Shared, text: &str) -> Vec<String> {
    let mut part = MidiPart(model.clone());
    // MidiPart::load never fails: what it can't read becomes warnings.
    let _ = part.load(text);
    let m = model.lock().unwrap_or_else(|e| e.into_inner());
    m.warnings.clone()
}

/// Hands the store the setup when it has changed.
#[derive(Debug, Default)]
pub struct Saver {
    /// The model's `gen` when it was last looked at.
    seen: Option<u64>,
    /// The text last handed to the store (or restored).
    kept: Option<String>,
}

impl Saver {
    /// A saver that counts the model as it is now as already kept, so a
    /// start doesn't write back what it just read.
    pub fn new(model: &Shared) -> Self {
        let m = model.lock().unwrap_or_else(|e| e.into_inner());
        Saver {
            seen: Some(m.gen),
            kept: Some(m.setup.to_text()),
        }
    }

    /// The setup's text, when it changed since the last call. Never waits
    /// for the screen: if the screen holds the model, the next call looks.
    pub fn take(&mut self, model: &Shared) -> Option<String> {
        let m = match model.try_lock() {
            Ok(m) => m,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return None,
        };
        if self.seen == Some(m.gen) {
            return None;
        }
        self.seen = Some(m.gen);
        let text = m.setup.to_text();
        drop(m);
        if self.kept.as_deref() == Some(text.as_str()) {
            return None;
        }
        self.kept = Some(text.clone());
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_midi::setup::{PortInput, TempoSource, DIN_NAME};

    fn changed(model: &Shared, f: impl FnOnce(&mut mpfree_midi_ui::IoModel)) {
        let mut m = model.lock().unwrap();
        f(&mut m);
        m.gen += 1;
    }

    #[test]
    fn a_change_is_kept_once_and_comes_back_at_the_next_start() {
        let model = crate::midi_in::io_model();
        let mut saver = Saver::new(&model);
        assert_eq!(saver.take(&model), None, "nothing changed yet");
        changed(&model, |m| {
            m.setup
                .set_port(DIN_NAME, |p| p.input = PortInput::LearnOnly);
            m.setup.tempo = TempoSource::Port(DIN_NAME.into());
        });
        let text = saver.take(&model).expect("the change is handed over");
        assert_eq!(saver.take(&model), None, "only once");
        // The same text again (a section bumped gen without a change).
        changed(&model, |_| {});
        assert_eq!(saver.take(&model), None);

        // The next start.
        let again = crate::midi_in::io_model();
        let warnings = restore(&again, &text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.lock().unwrap().setup, model.lock().unwrap().setup);
        // What was just read isn't written back.
        let mut saver = Saver::new(&again);
        assert_eq!(saver.take(&again), None);
    }

    #[test]
    fn lines_it_cant_read_are_said_and_the_rest_comes_back() {
        let model = crate::midi_in::io_model();
        changed(&model, |m| {
            m.setup.tempo = TempoSource::Port(DIN_NAME.into());
        });
        let text = model.lock().unwrap().setup.to_text();
        let again = crate::midi_in::io_model();
        let warnings = restore(&again, &format!("{text}cv.9 = off\n"));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(again.lock().unwrap().setup, model.lock().unwrap().setup);
    }

    #[test]
    fn a_held_model_is_looked_at_next_time() {
        let model = crate::midi_in::io_model();
        let mut saver = Saver::new(&model);
        changed(&model, |m| {
            m.setup.follow_start_stop = !m.setup.follow_start_stop
        });
        {
            let _held = model.lock().unwrap();
            assert_eq!(saver.take(&model), None);
        }
        assert!(saver.take(&model).is_some());
    }
}
