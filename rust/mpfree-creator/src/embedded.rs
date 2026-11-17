//! MPFree's programs for the MPC, when this build carries them (release
//! builds, see `build.rs`), and the private temporary folder they are put in
//! while the window is open. The step that makes a stick reads them there
//! (`--programs`), with the administrator rights it runs with.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

include!(concat!(env!("OUT_DIR"), "/embedded.rs"));

/// A folder holding the programs, removed when this is dropped.
pub struct Unpacked {
    pub dir: PathBuf,
}

impl Drop for Unpacked {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Puts `files` in a new folder under `parent`, readable by this user only
/// (on Linux and macOS): a fresh name each time, and every file made new,
/// so nothing already there is followed or written over. `None` when there
/// is nothing to unpack (a developer's build).
pub fn unpack(files: &[(&str, &[u8])], parent: &Path) -> io::Result<Option<Unpacked>> {
    if files.is_empty() {
        return Ok(None);
    }
    // The system's random hash keys: a name no one can guess beforehand.
    let mut h = RandomState::new().build_hasher();
    h.write_u32(std::process::id());
    let dir = parent.join(format!("mpfree-creator-programs-{:016x}", h.finish()));
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(&dir)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", dir.display())))?;
    let u = Unpacked { dir };
    for (name, bytes) in files {
        let p = u.dir.join(name);
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&p)
            .and_then(|mut f| f.write_all(bytes).and_then(|_| f.sync_all()))
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", p.display())))?;
    }
    Ok(Some(u))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_into_a_new_private_folder_and_removes_it() {
        let parent = std::env::temp_dir();
        assert!(unpack(&[], &parent).unwrap().is_none());
        let files: &[(&str, &[u8])] = &[("mpfree", b"one"), ("mpfree-startup", b"two")];
        let a = unpack(files, &parent).unwrap().unwrap();
        let b = unpack(files, &parent).unwrap().unwrap();
        assert_ne!(a.dir, b.dir);
        assert_eq!(std::fs::read(a.dir.join("mpfree")).unwrap(), b"one");
        assert_eq!(std::fs::read(a.dir.join("mpfree-startup")).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&a.dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        let dir = a.dir.clone();
        drop(a);
        assert!(!dir.exists());
        drop(b);
    }

    #[test]
    fn a_developer_build_carries_no_programs_unless_asked() {
        // CI's and a developer's builds don't set MPFREE_CREATOR_PROGRAMS;
        // the release workflow does, and checks the result itself.
        if option_env!("MPFREE_CREATOR_PROGRAMS").is_none() {
            assert!(EMBEDDED.is_empty());
        }
    }
}
