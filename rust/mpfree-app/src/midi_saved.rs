//! The mixer's I/O view settings that belong to the desk, kept between starts.
//!
//! The settings split in two (the owner, 13 Nov 2026: project settings
//! apart from system-wide ones):
//!
//! - **The rig** (`MidiSetup::rig`): each port's input (Plays tracks, Learn
//!   only, Both), what it ignores, its gear's delay, the name given to it
//!   and its device profile, and the controls learned for every project. They describe the gear on the desk, so they
//!   are kept here for every project, in the store as `files/midi/rig.txt`.
//! - **The project** (`MidiSetup::project`): each track's "Plays on", the
//!   routes and thru, Tempo from and following, each port's clock out, the
//!   CV jobs and learned controls. They are the project's `midi` part (`mpfree_midi_ui::MidiPart`),
//!   saved and opened with the project (gui.rs), not here.
//!
//! [`restore`] reads the rig once at start, before the sound loop syncs the
//! model (lines it can't read are kept as warnings, shown in the MIDI
//! section and logged). [`load_profiles`] reads the musician's device
//! profiles (`files/midi-profiles/NAME.txt`, read only) into the model
//! beside the built-in ones, at start. [`Saver::take`] runs a few times a second in the
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

/// The store owner whose files are the device profiles folder: profiles
/// the musician puts in `files/midi-profiles/` (`NAME.txt`,
/// `mpfree_midi::profile`).
pub const PROFILES_FOLDER: &str = "midi-profiles";

/// Reads the profiles folder's `.txt` files, in name order, into `model`
/// after the built-in profiles. A file that isn't a profile, or whose name
/// another profile has, is left out; a line a profile can't read is
/// skipped. Gives a line for the log about each, and one naming what was
/// found. Only reads.
pub fn load_profiles(store: &mpfree_store::Store, model: &Shared) -> Vec<String> {
    use mpfree_midi::profile::{Profile, MAX_FILE_BYTES};
    let mut notes = Vec::new();
    let mut found = Vec::new();
    let mut m = model.lock().unwrap_or_else(|e| e.into_inner());
    for file in store.list(PROFILES_FOLDER) {
        if !file.ends_with(".txt") {
            continue;
        }
        let Some(bytes) = store.read(PROFILES_FOLDER, &file) else {
            notes.push(format!("profiles: {file} left out: it can't be read"));
            continue;
        };
        if bytes.len() > MAX_FILE_BYTES {
            notes.push(format!(
                "profiles: {file} left out: larger than {} KB",
                MAX_FILE_BYTES / 1024
            ));
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        match Profile::parse(&text) {
            Ok((p, warn)) => {
                for w in warn {
                    notes.push(format!("profiles: {file}: {w}"));
                }
                let name = p.name.clone();
                match m.profiles.add(p) {
                    Ok(()) => found.push(name),
                    Err(e) => notes.push(format!("profiles: {file} left out: {e}")),
                }
            }
            Err(e) => notes.push(format!("profiles: {file} left out: {e}")),
        }
    }
    if !found.is_empty() {
        notes.push(format!("profiles: found {}", found.join(", ")));
    }
    notes
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

    /// The profiles folder: each `.txt` that is a profile joins the
    /// built-in ones; one without a name, one whose name is taken and
    /// anything else are left out, each said.
    #[test]
    fn the_profiles_folder_adds_the_musicians_profiles() {
        let d = std::env::temp_dir().join(format!("mpfree-profiles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let f = d.join("files").join(PROFILES_FOLDER);
        std::fs::create_dir_all(&f).unwrap();
        std::fs::write(d.join("FORMAT"), "mpfree-data 1\n").unwrap();
        let w = |n: &str, t: &str| std::fs::write(f.join(n), t).unwrap();
        w(
            "juno.txt",
            "mpfree-profile 1\nname = Juno-X\nmatch = Juno-X\ncc.74 = Filter cutoff\nbad line\n",
        );
        w("gm.txt", "mpfree-profile 1\nname = General MIDI\n");
        w("blank.txt", "mpfree-profile 1\ncc.1 = Mod\n");
        w("readme.md", "not a profile");
        let store = mpfree_store::Store::at(&d, |_| {});
        let model = crate::midi_in::io_model();
        let notes = load_profiles(&store, &model);
        let m = model.lock().unwrap();
        let names: Vec<&str> = m.profiles.all().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["General MIDI", "Juno-X"]);
        assert_eq!(
            m.profile("Juno-X").and_then(|p| p.cc_name(74)),
            Some("Filter cutoff")
        );
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(
            notes[0].starts_with("profiles: blank.txt left out: no name"),
            "{notes:?}"
        );
        assert!(
            notes[1].contains("gm.txt left out: another profile is called"),
            "{notes:?}"
        );
        assert!(
            notes[2].starts_with("profiles: juno.txt: line 5"),
            "{notes:?}"
        );
        assert_eq!(notes[3], "profiles: found Juno-X");
        drop(m);
        let _ = std::fs::remove_dir_all(&d);
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
            m.setup.set_port(DIN_NAME, |p| p.delay_tenths += 5);
        });
        {
            let _held = model.lock().unwrap();
            assert!(saver.take(&model).is_none());
        }
        assert!(saver.take(&model).is_some());
    }
}
