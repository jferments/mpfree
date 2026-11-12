//! The MIDI, Sync and CV sections of the mixer's I/O view.
//!
//! The owner's calls (2026-11-12): a project's routing lives in an I/O view
//! inside the mixer, opened by the I/O icon beside the mixer icon. The
//! mixer (`mpfree-mixer-ui`) owns the view and its Audio section; this crate
//! gives it three more through [`mpfree_mixer_ui::IoSection`]:
//!
//! - **MIDI**: each port's input (Plays tracks, Learn only, Both) and thru,
//!   and each track's one output setting, "Plays on", the port and the
//!   channel together, with its program.
//! - **Sync**: where the tempo comes from, what following does, the clock
//!   arriving now, and each port's clock out (Off, While playing, Always).
//! - **CV**: the eight CV/gate outputs: each one's job, track and main
//!   setting, with the volts it gives now.
//!
//! The sections edit a [`Shared`] [`IoModel`]: the project's
//! [`MidiSetup`] (saved with the project) and [`Live`] facts the app fills
//! in (which ports are plugged in, track names, the clock arriving, the CV
//! volts, and which parts of MIDI are built yet). After a section answers
//! `Redraw`, the app reads the setup and applies it.
//!
//! **Saying in place when nothing will happen** (the owner's rule): a
//! setting for a device that isn't plugged in, or one whose engine isn't
//! in this build yet ([`Built`]), says so on its own row. Settings are kept
//! either way, so a project made now works when those parts arrive.
//!
//! Facts about the rig rather than the project (each port's shown name and
//! delay, each CV output's tuning) live in Prefs, not here.
#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mpfree_midi::cv::CV_OUTS;
pub use mpfree_midi::setup::MidiSetup;

mod cv;
mod form;
mod midi;
mod sync;

pub mod sample;

/// A port as the app knows it now.
#[derive(Clone, Debug, PartialEq)]
pub struct PortLive {
    /// The name a project uses for it ([`mpfree_midi::setup::DIN_NAME`] for
    /// the 5-pin jacks, a USB device's own name otherwise).
    pub name: String,
    /// What the screen says under its name ("5-pin jacks", "USB").
    pub kind: String,
    pub has_in: bool,
    pub has_out: bool,
    /// When something last arrived on its input (the app's clock).
    pub last_in: Option<Duration>,
}

/// Which parts of MIDI work in this run. A setting whose part is missing
/// is kept and says on its row that it does nothing yet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Built {
    /// Tracks play out of the 5-pin MIDI Out jack.
    pub notes_out: bool,
    /// A port's thru works.
    pub thru: bool,
    /// The tempo can follow another device's clock.
    pub clock_in: bool,
    /// Ports send clock.
    pub clock_out: bool,
    /// The CV/gate outputs driven, counted from CV 1 (0: CV out is off).
    pub cv_outs: u8,
}

/// What the app tells the sections about the world now.
#[derive(Clone, Debug, PartialEq)]
pub struct Live {
    /// The ports plugged in now, the 5-pin jacks first.
    pub ports: Vec<PortLive>,
    /// The project's tracks by name, in order (at most 16).
    pub tracks: Vec<String>,
    /// Clock arriving now: the port's name and its tempo.
    pub clock_in: Option<(String, f32)>,
    /// The volts each CV output gives now.
    pub cv_volts: [f32; CV_OUTS],
    pub built: Built,
}

impl Default for Live {
    fn default() -> Self {
        Live {
            ports: vec![PortLive {
                name: mpfree_midi::setup::DIN_NAME.to_string(),
                kind: "5-pin jacks".to_string(),
                has_in: true,
                has_out: true,
                last_in: None,
            }],
            tracks: Vec::new(),
            clock_in: None,
            cv_volts: [0.0; CV_OUTS],
            built: Built::default(),
        }
    }
}

impl Live {
    pub fn port(&self, name: &str) -> Option<&PortLive> {
        self.ports.iter().find(|p| p.name == name)
    }
}

/// What the sections show and edit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IoModel {
    pub setup: MidiSetup,
    pub live: Live,
    /// Lines of the project's MIDI part that couldn't be read when it was
    /// opened. The rest was; the MIDI section says how many, and the app
    /// logs each one.
    pub warnings: Vec<String>,
    /// Goes up each time the setup changes (a section, or a project
    /// opened), so the app copies the setup only when it has changed.
    pub gen: u64,
}

/// The project's `midi` part (`parts/midi.txt`): the setup the sections
/// edit, saved and opened with the project.
pub struct MidiPart(pub Shared);

/// The part's file name in a project.
pub const PART: &str = "midi";

impl mpfree_library::Part for MidiPart {
    fn owner(&self) -> &str {
        PART
    }

    fn save(&self) -> String {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .setup
            .to_text()
    }

    /// Reads what it can. Lines it can't read are skipped, kept in
    /// [`IoModel::warnings`] and shown, never dropped silently; the project
    /// opens either way.
    fn load(&mut self, text: &str) -> Result<(), String> {
        let (setup, warnings) = MidiSetup::parse(text);
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.setup = setup;
        m.warnings = warnings;
        m.gen += 1;
        Ok(())
    }
}

/// The model the app and the sections share.
pub type Shared = Arc<Mutex<IoModel>>;

/// The sections' titles in the I/O view's list, to find one by name
/// (`mpfree_mixer_ui::io_select` takes its place in the list).
pub const MIDI: &str = "MIDI";
pub const SYNC: &str = "Sync";
pub const CV: &str = "CV";

/// The three sections, in the order the I/O view lists them, all on one
/// model. Hand each to `mpfree_mixer_ui::add_io_section`.
pub fn sections(model: &Shared) -> Vec<Box<dyn mpfree_mixer_ui::IoSection>> {
    vec![
        Box::new(form::FormSection::new(midi::Midi, model.clone())),
        Box::new(form::FormSection::new(sync::Sync, model.clone())),
        Box::new(form::FormSection::new(cv::Cv, model.clone())),
    ]
}

/// A note's name as the app writes it (48 is C3, 60 is C4).
pub fn note_name(n: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    format!("{}{}", NAMES[n as usize % 12], n as i32 / 12 - 1)
}

/// How long an input's light stays lit after something arrives.
pub const LIGHT: Duration = Duration::from_millis(250);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_saves_and_opens_and_keeps_what_it_cant_read() {
        use mpfree_library::Part;
        let model: Shared = Arc::new(Mutex::new(sample::rig()));
        let text = MidiPart(model.clone()).save();
        let other: Shared = Arc::default();
        let mut part = MidiPart(other.clone());
        assert_eq!(part.owner(), "midi");
        part.load(&text).unwrap();
        assert_eq!(other.lock().unwrap().setup, model.lock().unwrap().setup);
        assert!(other.lock().unwrap().warnings.is_empty());
        part.load(&format!("{text}cv.9 = off\n")).unwrap();
        assert_eq!(other.lock().unwrap().warnings.len(), 1);
        assert_eq!(other.lock().unwrap().setup, model.lock().unwrap().setup);
    }

    #[test]
    fn notes_are_named_as_in_the_app() {
        assert_eq!(note_name(48), "C3");
        assert_eq!(note_name(60), "C4");
        assert_eq!(note_name(37), "C#2");
        assert_eq!(note_name(0), "C-1");
    }
}
