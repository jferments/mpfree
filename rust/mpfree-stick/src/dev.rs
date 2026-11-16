//! Developer tools for Akai's own system (`--dev-tools FOLDER`, opt-in): a
//! folder of plain files put on the setup partition (5, FAT32, which Akai's
//! system mounts at `/media/MPFREE`) under `mpfree-dev/`, with `SHA256SUMS`
//! and `MPFREE-DEV.txt` made here. MPFree never reads them. Akai's system
//! mounts the stick noexec, so a tool there is a shell script run with `sh`,
//! or files a step copies elsewhere first. The rules are
//! `offline-kexec-stick.py --dev-tools`'s (plain names, no links, at most
//! 64 MiB, 512 files and 6 folders deep), plus two for FAT: no name ends in
//! a dot, and no two names in a folder differ only in case.

use crate::bad;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

/// The folder on the setup partition.
pub const DIR: &str = "mpfree-dev";
pub const MAX_BYTES: u64 = 64 << 20;
pub const MAX_FILES: usize = 512;
pub const MAX_DEPTH: usize = 6;
pub const MARK: &str = "MPFREE-DEV.txt";
pub const MARK_TEXT: &str =
    "MPFree developer tools for Akai's system (mpfree-stick --dev-tools).\n\
Akai's system mounts this partition noexec: run a script here with sh, never directly.\n\
SHA256SUMS lists every file. MPFree itself does not read this folder.\n";

/// The tools, checked: folders and files by their path under [`DIR`]
/// (`/` between names), in a stable order.
pub struct DevTools {
    pub source: PathBuf,
    pub folders: Vec<String>,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Letters, digits, `.`, `_`, `-` and single spaces between them, starting
/// with a letter or digit, at most 100 characters (the Python builder's
/// `DEV_NAME`); and, for FAT, not ending in a dot.
pub fn plain_name(name: &str) -> bool {
    let b = name.as_bytes();
    if b.is_empty() || b.len() > 100 || !b[0].is_ascii_alphanumeric() {
        return false;
    }
    if matches!(b[b.len() - 1], b' ' | b'.') {
        return false;
    }
    b.iter().enumerate().all(|(i, &c)| {
        c.is_ascii_alphanumeric()
            || matches!(c, b'.' | b'_' | b'-')
            || (c == b' ' && b.get(i + 1) != Some(&b' '))
    })
}

impl DevTools {
    pub fn from_dir(top: &Path) -> io::Result<DevTools> {
        let what = |p: &Path| format!("--dev-tools: {}", p.display());
        if !std::fs::metadata(top).map(|m| m.is_dir()).unwrap_or(false) {
            return Err(bad(format!("{} is not a folder", what(top))));
        }
        let mut t = DevTools {
            source: std::fs::canonicalize(top)?,
            folders: Vec::new(),
            files: Vec::new(),
        };
        let mut total = 0u64;
        // (folder on disk, its path under DIR, depth)
        let mut stack = vec![(t.source.clone(), String::new(), 0usize)];
        while let Some((here, rel, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(bad(format!(
                    "{} is more than {MAX_DEPTH} folders deep",
                    what(&here)
                )));
            }
            let mut names = Vec::new();
            for e in std::fs::read_dir(&here)? {
                let e = e?;
                let name = e
                    .file_name()
                    .into_string()
                    .map_err(|_| bad(format!("{} has a name that is not text", what(&e.path()))))?;
                names.push(name);
            }
            names.sort();
            let mut seen = HashSet::new();
            let mut subdirs = Vec::new();
            for name in names {
                let path = here.join(&name);
                if !plain_name(&name) {
                    return Err(bad(format!(
                        "{} has an unusual name (letters, digits, '.', '_', '-' and single spaces only, \
                         starting with a letter or digit, not ending with a space or a dot)",
                        what(&path)
                    )));
                }
                if !seen.insert(name.to_ascii_lowercase()) {
                    return Err(bad(format!(
                        "{} differs from another name there only in case (FAT can't hold both)",
                        what(&path)
                    )));
                }
                if rel.is_empty()
                    && (name.eq_ignore_ascii_case(MARK) || name.eq_ignore_ascii_case("SHA256SUMS"))
                {
                    return Err(bad(format!(
                        "{} is a name this tool makes itself",
                        what(&path)
                    )));
                }
                let m = std::fs::symlink_metadata(&path)?;
                let under = if rel.is_empty() {
                    name.clone()
                } else {
                    format!("{rel}/{name}")
                };
                if m.file_type().is_symlink() {
                    return Err(bad(format!(
                        "{} is a link; only plain files and folders go on the stick",
                        what(&path)
                    )));
                } else if m.is_dir() {
                    t.folders.push(under.clone());
                    subdirs.push((path, under));
                } else if m.is_file() {
                    total += m.len();
                    if total > MAX_BYTES || t.files.len() >= MAX_FILES {
                        return Err(bad(format!(
                            "--dev-tools: more than {} MiB or {MAX_FILES} files",
                            MAX_BYTES >> 20
                        )));
                    }
                    let data = std::fs::read(&path)?;
                    t.files.push((under, data));
                } else {
                    return Err(bad(format!("{} is not a plain file", what(&path))));
                }
            }
            // Folders in name order, each before what is in it.
            for (path, under) in subdirs.into_iter().rev() {
                stack.push((path, under, depth + 1));
            }
        }
        if t.files.is_empty() {
            return Err(bad(format!("{} holds no files", what(top))));
        }
        Ok(t)
    }

    /// `SHA256SUMS`: paths relative to [`DIR`], so `sha256sum -c` works from there.
    pub fn sums(&self) -> String {
        self.files
            .iter()
            .map(|(p, d)| format!("{}  {p}\n", crate::update::hex(&Sha256::digest(d))))
            .collect()
    }

    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|(_, d)| d.len() as u64).sum()
    }

    /// The report's lines: what goes where, then each file's hash.
    pub fn report(&self) -> Vec<String> {
        let mut out = vec![format!(
            "developer tools for Akai's system (--dev-tools {}): {} files, {} bytes, on partition 5 \
             under /{DIR} with SHA256SUMS and {MARK}; MPFree never reads them, and Akai's system \
             mounts them noexec (run with sh)",
            self.source.display(),
            self.files.len(),
            self.bytes()
        )];
        for line in self.sums().lines() {
            if let Some((digest, path)) = line.split_once("  ") {
                out.push(format!("  dev tool /{DIR}/{path}  sha256 {digest}"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_follow_the_python_builders_rule_and_fats() {
        for ok in [
            "fxprobe",
            "fxprobe.sh",
            "MPFree - VST - FXProbe Source",
            "a_b-c.d",
            "9",
        ] {
            assert!(plain_name(ok), "{ok}");
        }
        for bad in [
            "",
            ".hidden",
            "-x",
            "_x",
            " x",
            "x ",
            "a  b",
            "x.",
            "a/b",
            "a:b",
            "é",
            &"x".repeat(101),
        ] {
            assert!(!plain_name(bad), "{bad:?}");
        }
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mpfree-dev-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn reads_a_kit_in_order_with_its_folders_and_sums() {
        let d = folder("kit");
        std::fs::create_dir_all(d.join("fxprobe/MPFree - VST - FXProbe Source/VST")).unwrap();
        std::fs::write(d.join("fxprobe/fxprobe.sh"), b"#!/bin/sh\n").unwrap();
        std::fs::write(
            d.join("fxprobe/MPFree - VST - FXProbe Source/VST/a.so"),
            b"\x7fELF",
        )
        .unwrap();
        std::fs::write(d.join("README.txt"), b"hi\n").unwrap();
        let t = DevTools::from_dir(&d).unwrap();
        let files: Vec<&str> = t.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(
            files,
            [
                "README.txt",
                "fxprobe/fxprobe.sh",
                "fxprobe/MPFree - VST - FXProbe Source/VST/a.so"
            ]
        );
        assert_eq!(
            t.folders,
            [
                "fxprobe",
                "fxprobe/MPFree - VST - FXProbe Source",
                "fxprobe/MPFree - VST - FXProbe Source/VST"
            ]
        );
        assert_eq!(t.bytes(), 4 + 10 + 3);
        assert!(t.sums().starts_with(
            "98ea6e4f216f2fb4b69fff9b3a44842c38686ca685f3f55dc48c5d3fb1107be4  README.txt\n"
        ));
        let r = t.report();
        assert!(r[0].contains(": 3 files, 17 bytes, on partition 5 under /mpfree-dev"));
        assert_eq!(r.len(), 4);
        assert!(r[2].starts_with("  dev tool /mpfree-dev/fxprobe/fxprobe.sh  sha256 "));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn refuses_links_odd_names_case_twins_its_own_names_and_nothing() {
        let d = folder("refuse");
        assert!(DevTools::from_dir(&d)
            .err()
            .unwrap()
            .to_string()
            .contains("holds no files"));
        std::fs::write(d.join("a.sh"), b"x").unwrap();
        std::fs::write(d.join("A.sh"), b"y").unwrap();
        assert!(DevTools::from_dir(&d)
            .err()
            .unwrap()
            .to_string()
            .contains("only in case"));
        std::fs::remove_file(d.join("A.sh")).unwrap();
        std::fs::write(d.join("sha256sums"), b"y").unwrap();
        assert!(DevTools::from_dir(&d)
            .err()
            .unwrap()
            .to_string()
            .contains("makes itself"));
        std::fs::remove_file(d.join("sha256sums")).unwrap();
        std::fs::write(d.join("x."), b"y").unwrap();
        assert!(DevTools::from_dir(&d)
            .err()
            .unwrap()
            .to_string()
            .contains("unusual name"));
        std::fs::remove_file(d.join("x.")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("a.sh", d.join("link")).unwrap();
            assert!(DevTools::from_dir(&d)
                .err()
                .unwrap()
                .to_string()
                .contains("is a link"));
            std::fs::remove_file(d.join("link")).unwrap();
        }
        let mut deep = d.clone();
        for i in 0..7 {
            deep = deep.join(format!("d{i}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("f"), b"x").unwrap();
        assert!(DevTools::from_dir(&d)
            .err()
            .unwrap()
            .to_string()
            .contains("folders deep"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
