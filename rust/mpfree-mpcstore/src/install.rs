//! Unpacks the plugin folders in a release zip into MPFree's plugin folder
//! (`files/plugins/`), safely: names are checked, links are skipped, sizes
//! are capped, every file is checked against its CRC, and a folder appears
//! under its own name only once it is whole. An update keeps the files the
//! musician added to the old folder (ROMs, their own presets).

use crate::sha256;
use crate::zip::{safe_parts, Kind, Zip};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// At most this much is unpacked from one zip (the catalog's biggest
/// plugin, with its Python, is about 150 MB).
pub const MAX_TOTAL: u64 = 512 << 20;
/// Files that tell the MPC (and MPFree) a folder is a plugin.
const DESCRIPTIONS: [&str; 3] = ["plugin.json", "plugin-meta.xml", "mpc-plugin.json"];
const STAGING: &str = ".mpfree-installing-";
const OLD: &str = ".mpfree-replaced-";

/// A plugin folder inside a zip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    /// Its path in the zip, ending in `/`.
    pub prefix: String,
    /// Its name: the folder it gets in `files/plugins/`.
    pub folder: String,
}

/// The plugin folders in a zip: a folder holding `plugin.json`, or holding
/// `plugin-meta.xml` or `mpc-plugin.json` and a `.so`. A release zip's top
/// folder has `mpc-plugin.json` but no library, so only its
/// `portable/<plugin>` folder counts. Folders inside a plugin folder are
/// part of it.
pub fn plugin_folders(zip: &Zip) -> Vec<Found> {
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Has {
        json: bool,
        description: bool,
        library: bool,
    }
    let mut dirs: BTreeMap<String, Has> = BTreeMap::new();
    for e in &zip.entries {
        if e.kind != Kind::File {
            continue;
        }
        let (dir, file) = match e.name.rfind('/') {
            Some(i) => (&e.name[..=i], &e.name[i + 1..]),
            None => ("", e.name.as_str()),
        };
        let h = dirs.entry(dir.to_string()).or_default();
        h.json |= file == "plugin.json";
        h.description |= DESCRIPTIONS.contains(&file);
        h.library |= file.ends_with(".so");
    }
    let mut found: Vec<Found> = Vec::new();
    // BTreeMap order puts a folder before the folders inside it.
    for (dir, h) in dirs {
        if dir.is_empty() || !(h.json || (h.description && h.library)) {
            continue;
        }
        if found.iter().any(|f| dir.starts_with(&f.prefix)) {
            continue;
        }
        let Some(parts) = safe_parts(&dir) else {
            continue;
        };
        let folder = parts.last().expect("not empty").to_string();
        if folder.starts_with('.') {
            continue;
        }
        found.push(Found {
            prefix: dir,
            folder,
        });
    }
    found
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installed {
    pub folder: PathBuf,
    pub files: usize,
    pub bytes: u64,
    /// It replaced a folder of the same name.
    pub replaced: bool,
    /// Files the musician had added to the old folder, carried over.
    pub kept: usize,
    /// Links in the zip, left out.
    pub links_skipped: usize,
}

/// Checks a zip against the SHA-256 the catalog gives for it (when one is
/// given), then unpacks each plugin folder in it into `dest`.
pub fn install(
    zip_bytes: &[u8],
    dest: &Path,
    sha256: Option<&str>,
) -> Result<Vec<Installed>, String> {
    if let Some(want) = sha256 {
        let got = sha256::hex(zip_bytes);
        if !got.eq_ignore_ascii_case(want) {
            return Err(format!(
                "the download doesn't match the catalog (SHA-256 {got}, the catalog says {want}): nothing was installed"
            ));
        }
    }
    let zip = Zip::parse(zip_bytes)?;
    let found = plugin_folders(&zip);
    if found.is_empty() {
        return Err("no MPC plugin folder in it".into());
    }
    let total: u64 = zip
        .entries
        .iter()
        .filter(|e| found.iter().any(|f| e.name.starts_with(&f.prefix)))
        .map(|e| e.size)
        .sum();
    if total > MAX_TOTAL {
        return Err(format!(
            "it unpacks to {} MB, more than {} MB",
            total >> 20,
            MAX_TOTAL >> 20
        ));
    }
    fs::create_dir_all(dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let mut done = Vec::new();
    for f in &found {
        done.push(unpack(&zip, f, dest)?);
    }
    Ok(done)
}

fn unpack(zip: &Zip, f: &Found, dest: &Path) -> Result<Installed, String> {
    let staging = dest.join(format!("{STAGING}{}", f.folder));
    let target = dest.join(&f.folder);
    let old = dest.join(format!("{OLD}{}", f.folder));
    let io = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    // Left over from an install that was cut off.
    for p in [&staging, &old] {
        if p.exists() {
            fs::remove_dir_all(p).map_err(|e| io(p, e))?;
        }
    }
    let r = (|| {
        fs::create_dir(&staging).map_err(|e| io(&staging, e))?;
        let mut files = 0;
        let mut bytes = 0;
        let mut links = 0;
        for e in &zip.entries {
            let Some(rest) = e.name.strip_prefix(&f.prefix) else {
                continue;
            };
            if rest.is_empty() {
                continue;
            }
            let parts = safe_parts(rest).ok_or_else(|| format!("{} has an unsafe name", e.name))?;
            let path = parts.iter().fold(staging.clone(), |p, s| p.join(s));
            match e.kind {
                Kind::Link => links += 1,
                Kind::Dir => fs::create_dir_all(&path).map_err(|x| io(&path, x))?,
                Kind::File => {
                    let data = zip.read(e, MAX_TOTAL)?;
                    if let Some(parent) = path.parent() {
                        fs::create_dir_all(parent).map_err(|x| io(parent, x))?;
                    }
                    write_file(&path, &data, e.executable()).map_err(|x| io(&path, x))?;
                    files += 1;
                    bytes += data.len() as u64;
                }
            }
        }
        Ok::<_, String>((files, bytes, links))
    })();
    let (files, bytes, links) = match r {
        Ok(v) => v,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(e);
        }
    };
    let replaced = target.exists();
    let mut kept = 0;
    if replaced {
        kept = carry_over(&target, &staging, Path::new("")).map_err(|e| {
            let _ = fs::remove_dir_all(&staging);
            format!("keeping your files from the old {}: {e}", f.folder)
        })?;
        fs::rename(&target, &old).map_err(|e| io(&target, e))?;
        if let Err(e) = fs::rename(&staging, &target) {
            let _ = fs::rename(&old, &target);
            let _ = fs::remove_dir_all(&staging);
            return Err(io(&target, e));
        }
        let _ = fs::remove_dir_all(&old);
    } else {
        fs::rename(&staging, &target).map_err(|e| io(&target, e))?;
    }
    Ok(Installed {
        folder: target,
        files,
        bytes,
        replaced,
        kept,
        links_skipped: links,
    })
}

fn write_file(path: &Path, data: &[u8], exec: bool) -> std::io::Result<()> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(if exec { 0o755 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = exec;
    let mut file = o.open(path)?;
    file.write_all(data)?;
    // The stick can be pulled or the power cut: the folder appears under
    // its own name only after every file in it is on the stick.
    file.sync_all()
}

/// Moves the files under `old/rel` that the new folder hasn't got into it,
/// so an update keeps what the musician added. Returns how many.
fn carry_over(old: &Path, new: &Path, rel: &Path) -> std::io::Result<usize> {
    let mut n = 0;
    for e in fs::read_dir(old.join(rel))? {
        let e = e?;
        let r = rel.join(e.file_name());
        let t = e.file_type()?;
        if t.is_symlink() {
            continue;
        }
        let there = new.join(&r);
        if t.is_dir() {
            if there.is_dir() {
                n += carry_over(old, new, &r)?;
            } else if !there.exists() {
                fs::rename(e.path(), &there)?;
                n += 1;
            }
        } else if !there.exists() {
            fs::rename(e.path(), &there)?;
            n += 1;
        }
    }
    Ok(n)
}

/// The zips straight in `dir`, by name.
pub fn zips(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|s| s.to_str())
                            .is_some_and(|s| s.eq_ignore_ascii_case("zip"))
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::tests::make;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mpfree-mpcstore-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn release(version: &[u8]) -> Vec<u8> {
        make(&[
            ("Thing-1.0/", b"", 0o040755),
            ("Thing-1.0/install.sh", b"#!/bin/sh", 0o100755),
            ("Thing-1.0/mpc-plugin.json", b"{}", 0o100644),
            (
                "Thing-1.0/portable/Maker - VST - Thing/thing.so",
                b"\x7fELF",
                0o100755,
            ),
            (
                "Thing-1.0/portable/Maker - VST - Thing/plugin-meta.xml",
                b"<PLUGIN/>",
                0o100644,
            ),
            (
                "Thing-1.0/portable/Maker - VST - Thing/version.xml",
                version,
                0o100644,
            ),
            (
                "Thing-1.0/portable/Maker - VST - Thing/Plugin Skins/TUI.json",
                b"{}",
                0o100644,
            ),
            (
                "Thing-1.0/portable/Maker - VST - Thing/Plugin Skins/evil",
                b"/etc",
                0o120777,
            ),
        ])
    }

    #[test]
    fn finds_the_plugin_folder_of_a_release_zip() {
        let z = release(b"1");
        let zip = Zip::parse(&z).unwrap();
        assert_eq!(
            plugin_folders(&zip),
            [Found {
                prefix: "Thing-1.0/portable/Maker - VST - Thing/".into(),
                folder: "Maker - VST - Thing".into()
            }]
        );
        let sdk = make(&[
            ("Thing/plugin.json", b"{}", 0o100644),
            ("Thing/arm32/thing.so", b"x", 0o100755),
        ]);
        let zip = Zip::parse(&sdk).unwrap();
        assert_eq!(
            plugin_folders(&zip)[0].folder,
            "Thing",
            "plugin.json alone is enough"
        );
        let none = make(&[("readme.txt", b"hi", 0o100644)]);
        assert!(plugin_folders(&Zip::parse(&none).unwrap()).is_empty());
    }

    #[test]
    fn installs_checks_the_sum_and_updates_keeping_the_musicians_files() {
        let dest = scratch("install");
        let z = release(b"1");
        let wrong = "0".repeat(64);
        assert!(install(&z, &dest, Some(&wrong))
            .unwrap_err()
            .contains("doesn't match"));
        assert!(!dest.join("Maker - VST - Thing").exists());
        let sum = sha256::hex(&z);
        let done = install(&z, &dest, Some(&sum)).unwrap();
        let dir = dest.join("Maker - VST - Thing");
        assert_eq!(done[0].folder, dir);
        assert_eq!(done[0].files, 4);
        assert_eq!(done[0].links_skipped, 1);
        assert!(!done[0].replaced);
        assert!(dir.join("Plugin Skins/TUI.json").is_file());
        assert!(
            !dir.join("Plugin Skins/evil").exists(),
            "links are never unpacked"
        );
        assert!(!dest.join("install.sh").exists(), "only the plugin folder");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = fs::metadata(dir.join("thing.so"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(m & 0o111, 0o111);
        }
        // The musician adds a ROM; version 2 keeps it.
        fs::create_dir_all(dir.join("roms")).unwrap();
        fs::write(dir.join("roms/mine.bin"), b"rom").unwrap();
        let done = install(&release(b"2"), &dest, None).unwrap();
        assert!(done[0].replaced);
        assert_eq!(done[0].kept, 1);
        assert_eq!(fs::read(dir.join("version.xml")).unwrap(), b"2");
        assert_eq!(fs::read(dir.join("roms/mine.bin")).unwrap(), b"rom");
        let left: Vec<_> = fs::read_dir(&dest)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 1, "no staging or old folder left: {left:?}");
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn an_unsafe_name_inside_the_plugin_folder_stops_the_install() {
        let dest = scratch("unsafe");
        let z = make(&[
            ("P/plugin.json", b"{}", 0o100644),
            ("P/../../escape", b"x", 0o100644),
        ]);
        // `P/../../escape` starts with the prefix `P/`.
        assert!(install(&z, &dest, None).unwrap_err().contains("unsafe"));
        assert!(!dest.join("P").exists());
        assert!(
            fs::read_dir(&dest).unwrap().next().is_none(),
            "nothing left behind"
        );
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn lists_the_zips_in_a_folder() {
        let d = scratch("zips");
        fs::write(d.join("b.ZIP"), b"").unwrap();
        fs::write(d.join("a.zip"), b"").unwrap();
        fs::write(d.join("c.txt"), b"").unwrap();
        fs::create_dir(d.join("d.zip")).unwrap();
        let names: Vec<_> = zips(&d)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["a.zip", "b.ZIP"]);
        let _ = fs::remove_dir_all(&d);
    }
}
