//! The mixer's I/O view settings kept between starts.
//!
//! The settings split in two (the owner, 13 Nov 2026: project settings
//! apart from system-wide ones):
//!
//! - **The rig** (`MidiSetup::rig`): each port's input (Plays tracks, Learn
//!   only, Both) and thru. They describe the gear on the desk, so they are
//!   kept for every project, in the store as `files/midi/rig.txt`.
//! - **The project** (`MidiSetup::project`): each track's "Plays on", Tempo
//!   from and following, each port's clock out, the CV jobs and learned
//!   controls. They are the project's `midi` part (`mpfree_midi_ui::MidiPart`).
//!   Until the app saves whole projects, the last project part is also kept
//!   in the store as `files/midi/project.txt`, so nothing is lost at a
//!   restart; that copy goes once projects save.
//!
//! [`restore`] reads both once at start, before the sound loop syncs the
//! model (lines it can't read are kept as warnings, shown in the MIDI
//! section and logged). [`Saver::take`] runs a few times a second in the
//! sound loop: it only `try_lock`s the model, and hands back each text whose
//! content changed since it was last kept; the store's own thread writes it.

use std::sync::TryLockError;

use mpfree_library::Part;
use mpfree_midi_ui::{MidiPart, MidiSetup, Shared};

/// The owner the settings are kept under in the store, and the two files.
pub const OWNER: &str = mpfree_midi_ui::PART;
pub const RIG: &str = "rig.txt";
pub const PROJECT: &str = "project.txt";

/// Puts the kept rig and project texts into `model`. Gives the lines that
/// couldn't be read, for the log (they also show in the MIDI section).
pub fn restore(model: &Shared, rig: Option<&str>, project: Option<&str>) -> Vec<String> {
    let mut warnings = Vec::new();
    if let Some(text) = rig {
        let (setup, w) = MidiSetup::parse(text);
        warnings.extend(w.into_iter().map(|w| format!("{RIG}: {w}")));
        let mut m = model.lock().unwrap_or_else(|e| e.into_inner());
        m.setup.set_rig(&setup);
        m.gen += 1;
    }
    if let Some(text) = project {
        // MidiPart::load never fails: what it can't read becomes warnings.
        let _ = MidiPart(model.clone()).load(text);
        let m = model.lock().unwrap_or_else(|e| e.into_inner());
        warnings.extend(m.warnings.iter().map(|w| format!("{PROJECT}: {w}")));
    }
    let mut m = model.lock().unwrap_or_else(|e| e.into_inner());
    m.warnings = warnings.clone();
    warnings
}

/// Hands the store the rig and the project part when they have changed.
#[derive(Debug, Default)]
pub struct Saver {
    /// The model's `gen` when it was last looked at.
    seen: Option<u64>,
    /// The texts last handed to the store (or restored).
    rig: String,
    project: String,
}

impl Saver {
    /// A saver that counts the model as it is now as already kept, so a
    /// start doesn't write back what it just read.
    pub fn new(model: &Shared) -> Self {
        let m = model.lock().unwrap_or_else(|e| e.into_inner());
        Saver {
            seen: Some(m.gen),
            rig: m.setup.rig_text(),
            project: m.setup.project_text(),
        }
    }

    /// The files to write, `(file, text)`, for what changed since the last
    /// call. Never waits for the screen: if the screen holds the model, the
    /// next call looks.
    pub fn take(&mut self, model: &Shared) -> Vec<(&'static str, String)> {
        let m = match model.try_lock() {
            Ok(m) => m,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return Vec::new(),
        };
        if self.seen == Some(m.gen) {
            return Vec::new();
        }
        self.seen = Some(m.gen);
        let (rig, project) = (m.setup.rig_text(), m.setup.project_text());
        drop(m);
        let mut out = Vec::new();
        if rig != self.rig {
            self.rig = rig.clone();
            out.push((RIG, rig));
        }
        if project != self.project {
            self.project = project.clone();
            out.push((PROJECT, project));
        }
        out
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

    fn file<'a>(out: &'a [(&str, String)], name: &str) -> Option<&'a str> {
        out.iter()
            .find(|(f, _)| *f == name)
            .map(|(_, t)| t.as_str())
    }

    #[test]
    fn a_change_is_kept_once_in_its_file_and_comes_back_at_the_next_start() {
        let model = crate::midi_in::io_model();
        let mut saver = Saver::new(&model);
        assert!(saver.take(&model).is_empty(), "nothing changed yet");
        // A port's input is the rig's; Tempo from is the project's.
        changed(&model, |m| {
            m.setup
                .set_port(DIN_NAME, |p| p.input = PortInput::LearnOnly);
        });
        let out = saver.take(&model);
        assert_eq!(out.len(), 1, "{out:?}");
        let rig = file(&out, RIG).expect("the rig").to_string();
        changed(&model, |m| {
            m.setup.tempo = TempoSource::Port(DIN_NAME.into());
        });
        let out = saver.take(&model);
        assert_eq!(out.len(), 1, "{out:?}");
        let project = file(&out, PROJECT).expect("the project").to_string();
        assert!(saver.take(&model).is_empty(), "only once");
        // The same texts again (a section bumped gen without a change).
        changed(&model, |_| {});
        assert!(saver.take(&model).is_empty());

        // The next start.
        let again = crate::midi_in::io_model();
        let warnings = restore(&again, Some(&rig), Some(&project));
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.lock().unwrap().setup, model.lock().unwrap().setup);
        // What was just read isn't written back.
        let mut saver = Saver::new(&again);
        assert!(saver.take(&again).is_empty());
    }

    #[test]
    fn lines_it_cant_read_are_said_and_the_rest_comes_back() {
        let model = crate::midi_in::io_model();
        changed(&model, |m| {
            m.setup.tempo = TempoSource::Port(DIN_NAME.into());
        });
        let text = model.lock().unwrap().setup.project_text();
        let again = crate::midi_in::io_model();
        let warnings = restore(&again, None, Some(&format!("{text}cv.9 = off\n")));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].starts_with(PROJECT));
        assert_eq!(again.lock().unwrap().setup, model.lock().unwrap().setup);
        assert_eq!(again.lock().unwrap().warnings.len(), 1);
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
            assert!(saver.take(&model).is_empty());
        }
        assert!(file(&saver.take(&model), PROJECT).is_some());
    }
}
