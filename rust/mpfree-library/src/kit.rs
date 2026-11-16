//! Kit files: a whole drum kit kept as one folder, so a kit is saved and
//! loaded in one go and reads on any computer.
//!
//! ```text
//! <Kit name>/
//!   kit.txt          the sampler's part for one kit (`mpfree-sampler 1`,
//!                    `kits = 1`): its pads and every setting
//!   Kick.wav         its sounds, named in kit.txt, beside it
//!   Snare.wav ...
//! ```
//!
//! - A folder holding a `kit.txt` file is a kit ([`crate::Kind::Kit`])
//!   wherever it is in the library (Kits, Samples ...). The browser loads
//!   it whole; `+` goes into it to see its sounds one by one.
//! - `kit.txt` is exactly what the sampler writes for a project's kits
//!   (`mpfree_sampler::save`), with one kit, so every pad setting comes back
//!   as it was. The library never reads inside it; the app does.
//! - The kit's name is the folder's name: Rename renames the kit.
//! - A kit is written whole or not at all ([`Library::save_kit`]): built
//!   under a hidden name and renamed into place.

use crate::project::{bad, Capped};
use crate::{
    mkdir, sync_dir, unique_folder_name, unique_id, valid_name, Library, Loc, NewSample, Place,
};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// The file that makes a folder a kit.
pub const KIT_FILE: &str = "kit.txt";
/// The largest `kit.txt` read.
pub const MAX_KIT_TEXT: u64 = 4 << 20;
/// The most sounds a kit folder is given.
pub const MAX_KIT_SOUNDS: usize = 512;

/// Whether the folder at `dir` is a kit.
pub(crate) fn is_kit(dir: &Path) -> bool {
    fs::symlink_metadata(dir.join(KIT_FILE)).is_ok_and(|m| m.is_file())
}

impl Library {
    /// Saves a kit as a new folder in `folder`, named `name` (or `name 2`
    /// ... if taken): its `kit.txt` and each of `sounds` beside it. Built
    /// under a hidden name, synced and renamed, so it is there whole or not
    /// at all. Each sound's `file` must be a name the library takes, and
    /// different from the others and from `kit.txt`.
    pub fn save_kit(
        &self,
        folder: &Loc,
        name: &str,
        text: &str,
        sounds: &[NewSample],
    ) -> io::Result<Loc> {
        self.check_writable(folder)?;
        if folder.place == Place::Projects {
            return Err(bad(
                io::ErrorKind::InvalidInput,
                "kits are not kept among projects",
            ));
        }
        if sounds.len() > MAX_KIT_SOUNDS {
            return Err(bad(
                io::ErrorKind::InvalidInput,
                "too many sounds for a kit",
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for s in sounds {
            let lower = s.file.to_lowercase();
            if !valid_name(&s.file) || lower == KIT_FILE || !seen.insert(lower) {
                return Err(bad(
                    io::ErrorKind::InvalidInput,
                    format!("{}: not a name a kit's sound can have", s.file),
                ));
            }
        }
        let (fp, m) = self.existing(folder)?;
        if !m.is_dir() {
            return Err(bad(io::ErrorKind::InvalidInput, "not a folder"));
        }
        let name = if valid_name(name) { name } else { "Kit" };
        let name = unique_folder_name(name, |n| folder.join(n).is_ok_and(|l| self.exists(&l)));
        let loc = folder.join(&name)?;
        let tmp = fp.join(format!(".kit-{}-{name}", unique_id()));
        let r = (|| {
            mkdir(&tmp)?;
            for s in sounds {
                write_whole(&tmp.join(&s.file), &mut |w| s.data.write_to(w))?;
            }
            write_whole(&tmp.join(KIT_FILE), &mut |w| w.write_all(text.as_bytes()))?;
            sync_dir(&tmp)?;
            fs::rename(&tmp, fp.join(&name))?;
            sync_dir(&fp)
        })();
        if r.is_err() {
            let _ = fs::remove_dir_all(&tmp);
        }
        r.map(|_| loc)
    }

    /// A kit's `kit.txt`, as text.
    pub fn kit_text(&self, kit: &Loc) -> io::Result<String> {
        let f = self.kit_path(kit)?.join(KIT_FILE);
        let fm = fs::symlink_metadata(&f)?;
        if !fm.is_file() || fm.len() > MAX_KIT_TEXT {
            return Err(bad(
                io::ErrorKind::InvalidData,
                format!("{}: its kit.txt is too big", kit.display()),
            ));
        }
        let mut s = String::new();
        OpenOptions::new()
            .read(true)
            .custom_flags(0x20000) // O_NOFOLLOW
            .open(&f)?
            .take(MAX_KIT_TEXT)
            .read_to_string(&mut s)?;
        Ok(s)
    }

    /// Where one of a kit's sounds is (a file named in its `kit.txt`),
    /// checked to be a file in the kit's folder and not a link.
    pub fn kit_sound_path(&self, kit: &Loc, file: &str) -> io::Result<PathBuf> {
        let (p, m) = self.existing(kit)?;
        if !m.is_dir() || !valid_name(file) || file.eq_ignore_ascii_case(KIT_FILE) {
            return Err(bad(io::ErrorKind::InvalidInput, "bad sound name"));
        }
        let f = p.join(file);
        match fs::symlink_metadata(&f) {
            Ok(m) if m.is_file() => Ok(f),
            _ => Err(bad(
                io::ErrorKind::NotFound,
                format!("sound {file} is missing"),
            )),
        }
    }

    /// Whether `kit` is a kit folder.
    pub fn is_kit(&self, kit: &Loc) -> bool {
        self.kit_path(kit).is_ok()
    }

    /// The path of the kit folder at `kit` (not a link, holding `kit.txt`).
    pub fn kit_path(&self, kit: &Loc) -> io::Result<PathBuf> {
        let (p, m) = self.existing(kit)?;
        if m.is_dir() && is_kit(&p) {
            Ok(p)
        } else {
            Err(bad(
                io::ErrorKind::InvalidInput,
                format!("{} is not a kit", kit.display()),
            ))
        }
    }
}

/// A new file written through `fill`, synced; removed if anything fails.
fn write_whole(p: &Path, fill: &mut dyn FnMut(&mut dyn Write) -> io::Result<()>) -> io::Result<()> {
    let r = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .custom_flags(0x20000) // O_NOFOLLOW
            .open(p)?;
        {
            let mut w = io::BufWriter::with_capacity(256 * 1024, Capped { f: &mut f, n: 0 });
            fill(&mut w)?;
            w.flush()?;
        }
        f.sync_all()
    })();
    if r.is_err() {
        let _ = fs::remove_file(p);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::temp;
    use crate::Kind;
    use std::sync::Arc;

    fn sound(file: &str, bytes: &'static [u8]) -> NewSample {
        NewSample::new(file, Arc::new(move |w: &mut dyn Write| w.write_all(bytes)))
    }

    #[test]
    fn a_saved_kit_is_a_folder_listed_as_a_kit() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let kits = Loc::top(Place::Kits);
        let at = lib
            .save_kit(
                &kits,
                "Boom bap",
                "mpfree-sampler 1\nkits = 1\n",
                &[sound("Kick.wav", b"RIFF"), sound("Snare.wav", b"RIFF2")],
            )
            .unwrap();
        assert_eq!(at, kits.join("Boom bap").unwrap());
        let l = lib.list(&kits).unwrap();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].kind, Kind::Kit);
        assert!(lib.is_kit(&at));
        assert_eq!(lib.kit_text(&at).unwrap(), "mpfree-sampler 1\nkits = 1\n");
        assert_eq!(
            fs::read(lib.kit_sound_path(&at, "Snare.wav").unwrap()).unwrap(),
            b"RIFF2"
        );
        // Inside, its sounds (kit.txt is a file like any other).
        let inside = lib.list(&at).unwrap();
        assert!(inside.iter().any(|e| e.name == "Kick.wav"));
        // A second save with the same name gets its own folder.
        let again = lib.save_kit(&kits, "Boom bap", "x", &[]).unwrap();
        assert_eq!(again.name(), "Boom bap 2");
        // Nothing hidden is left behind.
        let left: Vec<_> = fs::read_dir(lib.path(&kits).unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(left.iter().all(|n| !n.starts_with('.')), "{left:?}");
    }

    #[test]
    fn a_kit_that_cannot_be_written_whole_leaves_nothing() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let kits = Loc::top(Place::Kits);
        let broken = NewSample::new(
            "Kick.wav",
            Arc::new(|_: &mut dyn Write| Err(io::Error::other("disk gone"))),
        );
        assert!(lib.save_kit(&kits, "K", "t", &[broken]).is_err());
        assert_eq!(fs::read_dir(lib.path(&kits).unwrap()).unwrap().count(), 0);
        // Bad sound names are refused before anything is written.
        for bad_name in ["kit.txt", "../x.wav", ".hidden.wav"] {
            assert!(lib
                .save_kit(&kits, "K", "t", &[sound(bad_name, b"x")])
                .is_err());
        }
        assert!(lib
            .save_kit(
                &kits,
                "K",
                "t",
                &[sound("a.wav", b"x"), sound("A.wav", b"y")]
            )
            .is_err());
        assert_eq!(fs::read_dir(lib.path(&kits).unwrap()).unwrap().count(), 0);
        // Not among projects, not in the Trash.
        assert!(lib
            .save_kit(&Loc::top(Place::Projects), "K", "t", &[])
            .is_err());
    }

    #[test]
    fn kit_sounds_stay_inside_the_kit() {
        let d = temp();
        let lib = Library::at(&d).unwrap();
        let at = lib
            .save_kit(&Loc::top(Place::Kits), "K", "t", &[sound("a.wav", b"x")])
            .unwrap();
        assert!(lib.kit_sound_path(&at, "../K/a.wav").is_err());
        assert!(lib.kit_sound_path(&at, "kit.txt").is_err());
        assert!(lib.kit_sound_path(&at, "missing.wav").is_err());
        // A plain folder is not a kit.
        let plain = lib.new_folder(&Loc::top(Place::Kits), "Plain").unwrap();
        assert!(!lib.is_kit(&plain));
        assert!(lib.kit_text(&plain).is_err());
    }
}
