//! The mixer's I/O view settings that belong to the desk, kept between starts.
//!
//! The settings split in two (the owner, 13 Nov 2026: project settings
//! apart from system-wide ones):
//!
//! - **The rig** (`MidiSetup::rig`): each port's input (Plays tracks, Learn
//!   only, Both) and thru. They describe the gear on the desk, so they are
//!   kept here for every project, in the store as `files/midi/rig.txt`.
//! - **The project** (`MidiSetup::project`): each track's "Plays on", Tempo
//!   from and following, each port's clock out, the CV jobs and learned
//!   controls. They are the project's `midi` part (`mpfree_midi_ui::MidiPart`),
//!   saved and opened with the project (gui.rs), not here.
//!
//! [`restore`] reads the rig once at start, before the sound loop syncs the
//! model (lines it can't read are kept as warnings, shown in the MIDI
//! section and logged). [`Saver::take`] runs a few times a second in the
//! sound loop: it only `try_lock`s the model, and hands back the rig's text
//! when it changed since it was last kept; the store's own thread writes it.

use std::sync::TryLockError;

use mpfree_midi_ui::{MidiSetup, Shared};

/// The owner the rig is kept under in the store, and its file.
pub const OWNER: &str = mpfree_midi_ui::PART;
pub const RIG: &str = "rig.txt";

/// Puts the kept rig text into `model`. Gives the lines that couldn't be
/// read, for the log (they also show in the MIDI section).
pub fn restore(model: &Shared, rig: Option<&str>) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut m = model.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(text) = rig {
        let (setup, w) = MidiSetup::parse(text);
        warnings.extend(w.into_iter().map(|w| format!("{RIG}: {w}")));
        m.setup.set_rig(&setup);
        m.gen += 1;
    }
    m.warnings = warnings.clone();
    warnings
}

/// Hands the store the rig when it has changed.
#[derive(Debug, Default)]
pub struct Saver {
    /// The model's `gen` when it was last looked at.
    seen: Option<u64>,
    /// The text last handed to the store (or restored).
    rig: String,
}

impl Saver {
    /// A saver that counts the model as it is now as already kept, so a
    /// start doesn't write back what it just read.
    pub fn new(model: &Shared) -> Self {
        let m = model.lock().unwrap_or_else(|e| e.into_inner());
        Saver {
            seen: Some(m.gen),
            rig: m.setup.rig_text(),
        }
    }

    /// The rig's text to write, if it changed since the last call. Never
    /// waits for the screen: if the screen holds the model, the next call
    /// looks.
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
        let rig = m.setup.rig_text();
        drop(m);
        if rig == self.rig {
            return None;
        }
        self.rig = rig.clone();
        Some(rig)
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
    fn a_rig_change_is_kept_once_and_comes_back_at_the_next_start() {
        let model = crate::midi_in::io_model();
        let mut saver = Saver::new(&model);
        assert!(saver.take(&model).is_none(), "nothing changed yet");
        // A port's input is the rig's.
        changed(&model, |m| {
            m.setup
                .set_port(DIN_NAME, |p| p.input = PortInput::LearnOnly);
        });
        let rig = saver.take(&model).expect("the rig");
        assert!(saver.take(&model).is_none(), "only once");
        // Tempo from is the project's: saved with the project, not here.
        changed(&model, |m| {
            m.setup.tempo = TempoSource::Port(DIN_NAME.into());
        });
        assert!(saver.take(&model).is_none());

        // The next start: the rig comes back, the song's settings are the
        // defaults until a project opens.
        let again = crate::midi_in::io_model();
        let warnings = restore(&again, Some(&rig));
        assert!(warnings.is_empty(), "{warnings:?}");
        let (a, m) = (again.lock().unwrap(), model.lock().unwrap());
        assert_eq!(a.setup.rig(), m.setup.rig());
        assert_eq!(a.setup.tempo, MidiSetup::default().tempo);
        drop((a, m));
        // What was just read isn't written back.
        let mut saver = Saver::new(&again);
        assert!(saver.take(&again).is_none());
    }

    #[test]
    fn lines_it_cant_read_are_said_and_the_rest_comes_back() {
        let model = crate::midi_in::io_model();
        changed(&model, |m| {
            m.setup
                .set_port(DIN_NAME, |p| p.input = PortInput::LearnOnly);
        });
        let text = model.lock().unwrap().setup.rig_text();
        let again = crate::midi_in::io_model();
        let warnings = restore(&again, Some(&format!("{text}nonsense here\n")));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with(RIG));
        assert_eq!(
            again.lock().unwrap().setup.rig(),
            model.lock().unwrap().setup.rig()
        );
        assert_eq!(again.lock().unwrap().warnings.len(), 1);
    }

    #[test]
    fn a_held_model_is_looked_at_next_time() {
        let model = crate::midi_in::io_model();
        let mut saver = Saver::new(&model);
        changed(&model, |m| {
            m.setup.set_port(DIN_NAME, |p| p.thru = !p.thru);
        });
        {
            let _held = model.lock().unwrap();
            assert!(saver.take(&model).is_none());
        }
        assert!(saver.take(&model).is_some());
    }
}
