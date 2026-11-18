//! Instrument presets the musician saves: one file each, in a folder per
//! engine under Instruments.
//!
//! ```text
//! Instruments/
//!   Keys/
//!     Night Tines.preset    the engine's preset text (mpfree-synth),
//!     Gospel Reed.preset    written whole by the app's synth
//!   Poly/ ...
//! ```
//!
//! - The folder is the engine's display name (`Keys`, `FM`), as the
//!   browser's preset picker opens on it. It is made the first time a
//!   preset is saved for that engine.
//! - The library never reads inside a preset; the app does.
//! - Saving under a name already there is refused unless the musician said
//!   to replace it; then the old file goes to the Trash first, so it can
//!   be put back.

use crate::{clean_name, mkdir, sync_dir, Library, Loc, Place};
use std::fs;
use std::io;

/// A preset file's ending.
pub const PRESET_EXT: &str = ".preset";

/// The largest preset text saved or read: a preset is a few kilobytes.
pub const MAX_PRESET_TEXT: usize = 1 << 20;

/// How [`Library::save_preset`] went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresetSave {
    /// Written, here.
    Saved(Loc),
    /// A preset of that name is there and replacing it wasn't asked for:
    /// nothing was written.
    Taken(Loc),
}

/// The name a preset file shows as: without its `.preset`.
pub fn preset_name(file: &str) -> &str {
    file.strip_suffix(PRESET_EXT).unwrap_or(file)
}

impl Library {
    /// Saves preset `text` for `engine` as `Instruments/<engine>/<name>.preset`,
    /// making the engine's folder if it isn't there. `name` is cleaned as
    /// any name is (a bad character becomes `-`). A preset already of that
    /// name is left alone ([`PresetSave::Taken`]) unless `replace`; then it
    /// goes to the Trash before the new one is written. Written to a
    /// `.part` file, synced and renamed, so it is there whole or not at all.
    pub fn save_preset(
        &self,
        engine: &str,
        name: &str,
        text: &str,
        replace: bool,
    ) -> io::Result<PresetSave> {
        if text.len() > MAX_PRESET_TEXT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the preset is too big",
            ));
        }
        let folder = self.preset_folder(engine)?;
        let file = format!("{}{PRESET_EXT}", clean_name(name, "Preset"));
        let loc = folder.join(&file)?;
        if self.exists(&loc) {
            if !replace {
                return Ok(PresetSave::Taken(loc));
            }
            self.delete(&loc)?;
        }
        self.write_new(&folder, &file, text.as_bytes())
            .map(PresetSave::Saved)
    }

    /// `Instruments/<engine>`, made if it isn't there.
    fn preset_folder(&self, engine: &str) -> io::Result<Loc> {
        let top = Loc::top(Place::Instruments);
        let folder = top.join(engine)?;
        let p = self.path(&folder)?;
        match fs::symlink_metadata(&p) {
            Ok(m) if m.file_type().is_dir() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} is there but is not a folder", folder.display()),
                ))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                mkdir(&p)?;
                sync_dir(&self.path(&top)?)?;
            }
            Err(e) => return Err(e),
        }
        Ok(folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;

    fn lib(tag: &str) -> (Library, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mpfree-preset-{tag}-{}-{}",
            std::process::id(),
            crate::unique_id()
        ));
        fs::create_dir_all(&dir).unwrap();
        (Library::at(&dir).unwrap(), dir)
    }

    #[test]
    fn a_preset_is_saved_in_its_engines_folder_made_the_first_time() {
        let (lib, dir) = lib("save");
        let r = lib
            .save_preset("Keys", "Night Tines", "text 1\n", false)
            .unwrap();
        let loc = Loc::new(Place::Instruments, &["Keys", "Night Tines.preset"]).unwrap();
        assert_eq!(r, PresetSave::Saved(loc.clone()));
        let path = dir.join("library/Instruments/Keys/Night Tines.preset");
        assert_eq!(fs::read_to_string(&path).unwrap(), "text 1\n");
        assert_eq!(Kind::of_file(loc.name()), Kind::Preset);
        // A second one goes beside it; the folder is not made again.
        lib.save_preset("Keys", "Gospel Reed", "text 2\n", false)
            .unwrap();
        let names: Vec<_> = lib
            .list(&Loc::new(Place::Instruments, &["Keys"]).unwrap())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names.len(), 2, "{names:?}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_name_already_there_is_left_alone_unless_replacing_and_then_trashed() {
        let (lib, dir) = lib("taken");
        lib.save_preset("FM", "Bell", "old\n", false).unwrap();
        let loc = Loc::new(Place::Instruments, &["FM", "Bell.preset"]).unwrap();
        let path = dir.join("library/Instruments/FM/Bell.preset");
        assert_eq!(
            lib.save_preset("FM", "Bell", "new\n", false).unwrap(),
            PresetSave::Taken(loc.clone())
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "old\n");
        assert_eq!(
            lib.save_preset("FM", "Bell", "new\n", true).unwrap(),
            PresetSave::Saved(loc)
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
        // The old one is in the Trash, to be put back.
        let trash = lib.trash().unwrap();
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].name, "Bell.preset");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn bad_names_are_cleaned_and_huge_texts_refused() {
        let (lib, dir) = lib("names");
        let r = lib.save_preset("Poly", "Lead: 1/2", "x\n", false).unwrap();
        assert_eq!(
            r,
            PresetSave::Saved(Loc::new(Place::Instruments, &["Poly", "Lead- 1-2.preset"]).unwrap())
        );
        let big = "x".repeat(MAX_PRESET_TEXT + 1);
        assert!(lib.save_preset("Poly", "Big", &big, false).is_err());
        // An engine name that can't be a folder is refused, not cleaned.
        assert!(lib.save_preset("../Poly", "Lead", "x\n", false).is_err());
        assert_eq!(preset_name("Lead.preset"), "Lead");
        assert_eq!(preset_name("Lead"), "Lead");
        fs::remove_dir_all(dir).unwrap();
    }
}
