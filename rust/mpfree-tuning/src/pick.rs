//! What a musician picks: a built-in tuning or a tuning file, its root and
//! A4. The song's tuning is one pick; an instrument with a tuning of its
//! own will hold another. The app, the sampler and MIDI out share these
//! types, so a pick means the same tuning everywhere.

use mpfree_scale::NOTE_NAMES;

use crate::{root_hz, scala, Builtin, Tuning};

/// The lowest and highest A4 a pick offers, in Hz.
pub const A4_MIN: u16 = 380;
pub const A4_MAX: u16 = 480;

/// A tuning from a Scala file the musician put in the tunings folder
/// (files/tunings/ on the data partition): its scale (`NAME.scl`) and,
/// with a keyboard map beside it (`NAME.kbm`), the tuning the map fixes.
#[derive(Clone, Debug, PartialEq)]
pub struct TuningFile {
    /// The file's name without `.scl`, as the options menu shows it.
    pub name: String,
    scale: scala::Scale,
    /// The tuning the keyboard map gives, root and pitch included; None
    /// without a map.
    mapped: Option<Tuning>,
}

impl TuningFile {
    /// Reads a scale and its keyboard map, if there is one, or says why
    /// not. Allocates: call it before the sound starts.
    pub fn parse(name: &str, scl: &str, kbm: Option<&str>) -> Result<TuningFile, String> {
        let scale = scala::parse_scl(scl).map_err(|e| e.to_string())?;
        let mapped = match kbm {
            Some(text) => {
                let map = scala::parse_kbm(text).map_err(|e| format!("its .kbm: {e}"))?;
                Some(scala::tuning(&scale, &map).map_err(|e| e.to_string())?)
            }
            None => None,
        };
        Ok(TuningFile {
            name: name.to_string(),
            scale,
            mapped,
        })
    }

    /// Whether a keyboard map sets the root and the pitch, so the Tuning
    /// root and A4 options don't apply.
    pub fn mapped(&self) -> bool {
        self.mapped.is_some()
    }

    /// Notes in one repeat of the scale (12 for most).
    pub fn notes(&self) -> usize {
        self.scale.len()
    }

    /// The tuning: the keyboard map's, or else the scale with its 1/1 on
    /// pitch class `root` (0 = C, in octave 4), which keeps its 12-TET
    /// pitch for `a4_hz`, and each key the next note of the scale, as the
    /// built-in tunings do. Never allocates.
    pub fn tuning(&self, root: u8, a4_hz: f32) -> Tuning {
        match &self.mapped {
            Some(t) => t.clone(),
            None => {
                let key = 60 + i32::from(root % 12);
                self.scale.rooted(key, root_hz(key, a4_hz))
            }
        }
    }
}

/// A tuning as picked: a built-in tuning or a tuning file, its root for
/// the tunings that have one (0 = C to 11 = B) and A4 in whole Hz. The
/// song's tuning is one (the Synth tab's options today); every engine but
/// Drums plays through it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuningPick {
    pub builtin: Builtin,
    /// A tuning file (its place in the list of tuning files offered, the
    /// app's `Synth::tuning_files`), which plays instead of `builtin`.
    pub file: Option<u8>,
    pub root: u8,
    pub a4_hz: u16,
}

impl Default for TuningPick {
    fn default() -> Self {
        TuningPick {
            builtin: Builtin::Equal12,
            file: None,
            root: 0,
            a4_hz: 440,
        }
    }
}

impl TuningPick {
    /// The tuning file picked, out of `files`.
    pub fn file<'a>(&self, files: &'a [TuningFile]) -> Option<&'a TuningFile> {
        files.get(usize::from(self.file?))
    }

    /// The tuning itself, with `files` the tuning files offered. Never
    /// allocates.
    pub fn tuning(&self, files: &[TuningFile]) -> Tuning {
        let a4 = f32::from(self.a4_hz.clamp(A4_MIN, A4_MAX));
        match self.file(files) {
            Some(f) => f.tuning(self.root % 12, a4),
            None => self.builtin.tuning(self.root % 12, a4),
        }
    }

    /// "Just (5-limit) on D, A4 432 Hz"; "12-TET" on its own at 440; a
    /// tuning file by its name ("werckmeister3 on C"), alone when its
    /// keyboard map sets the root and the pitch.
    pub fn label(&self, files: &[TuningFile]) -> String {
        let (name, has_root, fixed) = match self.file(files) {
            Some(f) => (f.name.as_str(), true, f.mapped()),
            None => (self.builtin.name(), self.builtin.has_root(), false),
        };
        if fixed {
            return name.to_string();
        }
        let root = if has_root {
            format!(" on {}", NOTE_NAMES[usize::from(self.root % 12)])
        } else {
            String::new()
        };
        let a4 = if self.a4_hz == 440 {
            String::new()
        } else {
            format!(", A4 {} Hz", self.a4_hz)
        };
        format!("{name}{root}{a4}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DORIAN_SCL: &str = "Dorian, just\n7\n9/8\n6/5\n4/3\n3/2\n5/3\n9/5\n2/1\n";
    /// The dorian scale on D (key 62) with A4 at 432 Hz, on the white keys.
    const DORIAN_KBM: &str = "12\n0\n127\n62\n69\n432.0\n7\n0\nx\n1\n2\nx\n3\nx\n4\nx\n5\n6\nx\n";

    #[test]
    fn a_pick_makes_its_tuning_and_names_it() {
        let files = vec![
            TuningFile::parse("dorian", DORIAN_SCL, None).unwrap(),
            TuningFile::parse("dorian-d", DORIAN_SCL, Some(DORIAN_KBM)).unwrap(),
        ];
        let plain = TuningPick::default();
        assert!(plain.tuning(&files).is_standard());
        assert_eq!(plain.label(&files), "12-TET");
        let just = TuningPick {
            builtin: Builtin::Just,
            root: 2,
            a4_hz: 432,
            ..plain
        };
        assert_eq!(just.tuning(&[]), Builtin::Just.tuning(2, 432.0));
        assert_eq!(just.label(&[]), "Just (5-limit) on D, A4 432 Hz");
        // A4 is kept to the range the options offer.
        let low = TuningPick { a4_hz: 5, ..plain };
        assert_eq!(low.tuning(&[]), Tuning::equal(12, f32::from(A4_MIN)));
        // A file plays instead of the built-in tuning; with its map, the
        // root and A4 don't apply.
        let file = TuningPick {
            file: Some(0),
            root: 7,
            ..just
        };
        assert_eq!(file.label(&files), "dorian on G, A4 432 Hz");
        assert_eq!(files[0].notes(), 7);
        let mapped = TuningPick {
            file: Some(1),
            ..file
        };
        assert!(files[1].mapped() && !files[0].mapped());
        assert_eq!(mapped.label(&files), "dorian-d");
        let other = TuningPick {
            root: 0,
            a4_hz: 440,
            ..mapped
        };
        assert_eq!(mapped.tuning(&files), other.tuning(&files));
        assert!((mapped.tuning(&files).hz(69.0) - 432.0).abs() < 1e-3);
        // A pick naming a file that isn't offered plays its built-in.
        let gone = TuningPick {
            file: Some(9),
            ..just
        };
        assert_eq!(gone.tuning(&files), just.tuning(&[]));
        assert!(TuningFile::parse("x", "desc\n2\n3/2\n", None).is_err());
    }
}
