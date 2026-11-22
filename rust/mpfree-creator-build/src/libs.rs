//! The MPC's system libraries for linking MPFree's programs: the C library
//! (glibc 2.39) and GCC's small support library for 32-bit ARM, taken from
//! the four Ubuntu 24.04 packages the release's cross compiler installs.
//!
//! Rust compiles MPFree's programs for the MPC on any computer; linking them
//! needs only these files and Rust's own linker (rust-lld), so no cross
//! compiler, Linux or container is needed. Each package is pinned by the
//! SHA-256 that Ubuntu's signed package index lists for it, fetched from
//! Ubuntu's archive (or Launchpad, which keeps every version), and only the
//! files the links use are taken out, into one folder.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One pinned Ubuntu package.
pub struct Package {
    pub file: &'static str,
    /// Its folder in the archive's `pool/`.
    pub pool: &'static str,
    pub sha256: &'static str,
}

/// Ubuntu 24.04's (noble's) armhf cross libraries, as `apt install
/// gcc-arm-linux-gnueabihf libc6-dev-armhf-cross` gives them on the release's
/// runners. The hashes are the ones in noble's signed package index.
pub const PACKAGES: [Package; 4] = [
    Package {
        file: "libc6-armhf-cross_2.39-0ubuntu8cross1_all.deb",
        pool: "main/c/cross-toolchain-base",
        sha256: "4c61c413e2cc45ce2c5703a1564d1886f45c98ddbbd33d9f8b05736dee045ce1",
    },
    Package {
        file: "libc6-dev-armhf-cross_2.39-0ubuntu8cross1_all.deb",
        pool: "main/c/cross-toolchain-base",
        sha256: "31bb0a43b89d9e6e9d4f2b81d53376ad01e397653e6a834e00ac4129e0e48c44",
    },
    Package {
        file: "libgcc-13-dev-armhf-cross_13.3.0-6ubuntu2~24.04.1cross1_all.deb",
        pool: "main/g/gcc-13-cross",
        sha256: "930c595a54ba1f1096463a9f8ab4757b5d1f12f267a99521aa4505ca302e7988",
    },
    Package {
        file: "libgcc-s1-armhf-cross_14.2.0-4ubuntu2~24.04.1cross1_all.deb",
        pool: "main/g/gcc-14-cross",
        sha256: "1fdbacca10af62652762869a21f07b545d5211257b5b78f245aceefd611e3ce7",
    },
];

/// The two folders of the packages the files come from (their paths in the
/// packages, without the leading `./`).
const DIRS: [&str; 2] = [
    "usr/arm-linux-gnueabihf/lib/",
    "usr/lib/gcc-cross/arm-linux-gnueabihf/13/",
];

/// The files the links use, all in one folder: what `arm-linux-gnueabihf-gcc`
/// hands the linker for a static program (crt1.o, crti.o, crtbeginT.o, the
/// `.a` libraries, crtend.o, crtn.o) and for a dynamically linked one
/// (Scrt1.o, crtbeginS.o, crtendS.o, and the shared C library and GCC's
/// support library through their linker scripts).
pub const NEEDED: [&str; 24] = [
    "crt1.o",
    "Scrt1.o",
    "crti.o",
    "crtn.o",
    "crtbeginT.o",
    "crtbeginS.o",
    "crtend.o",
    "crtendS.o",
    "libc.a",
    "libc.so",
    "libc.so.6",
    "libc_nonshared.a",
    "ld-linux-armhf.so.3",
    "libm.a",
    "libm.so",
    "libm.so.6",
    "libpthread.a",
    "libdl.a",
    "librt.a",
    "libutil.a",
    "libgcc.a",
    "libgcc_eh.a",
    "libgcc_s.so",
    "libgcc_s.so.1",
];

/// Where each package is fetched from, in order: Ubuntu's archive, then
/// Launchpad, which keeps every version after the archive drops an old one.
pub fn urls(p: &Package) -> [String; 2] {
    [
        format!(
            "https://archive.ubuntu.com/ubuntu/pool/{}/{}",
            p.pool, p.file
        ),
        format!(
            "https://launchpad.net/ubuntu/+archive/primary/+files/{}",
            p.file
        ),
    ]
}

/// The folder of library files under `out` (`out/mpc-libs`), made from the
/// pinned packages if it isn't already there; downloads are kept in
/// `out/downloads`.
pub fn ensure(out: &Path) -> Result<PathBuf, String> {
    let dir = out.join("mpc-libs");
    let stamp = dir.join("packages.txt");
    let want = stamp_text();
    if std::fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str())
        && NEEDED.iter().all(|n| dir.join(n).is_file())
    {
        println!("(already here, from {} Ubuntu packages)", PACKAGES.len());
        return Ok(dir);
    }
    let downloads = out.join("downloads");
    let mut files = BTreeMap::new();
    let mut links = Vec::new();
    for p in &PACKAGES {
        let deb = fetch(p, &downloads)?;
        take_files(&deb, &mut files, &mut links).map_err(|e| format!("{}: {e}", p.file))?;
    }
    let files = finish(files, &links)?;
    let part = out.join("mpc-libs.part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    for (name, data) in &files {
        let to = part.join(name);
        std::fs::write(&to, data).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let to = part.join("packages.txt");
    std::fs::write(&to, &want).map_err(|e| format!("{}: {e}", to.display()))?;
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&part, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    println!("{} files in {}", files.len(), dir.display());
    Ok(dir)
}

/// What `packages.txt` holds: the packages the folder was made from.
fn stamp_text() -> String {
    PACKAGES
        .iter()
        .map(|p| format!("{}  {}\n", p.sha256, p.file))
        .collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The package's bytes: from `downloads` when the file there has the pinned
/// hash, else fetched and checked.
fn fetch(p: &Package, downloads: &Path) -> Result<Vec<u8>, String> {
    let path = downloads.join(p.file);
    if let Ok(data) = std::fs::read(&path) {
        if sha256_hex(&data) == p.sha256 {
            println!("{}: already downloaded, checked", p.file);
            return Ok(data);
        }
    }
    let mut tried = Vec::new();
    for url in urls(p) {
        match download(&url, &path) {
            Ok(data) if sha256_hex(&data) == p.sha256 => {
                println!("{}: downloaded ({} KB), checked", p.file, data.len() / 1024);
                return Ok(data);
            }
            Ok(data) => {
                let _ = std::fs::remove_file(&path);
                tried.push(format!(
                    "{url}: its SHA-256 is {}, not the pinned {}",
                    sha256_hex(&data),
                    p.sha256
                ));
            }
            Err(e) => tried.push(e),
        }
    }
    Err(format!(
        "couldn't download {} (needs the internet, once):\n  {}",
        p.file,
        tried.join("\n  ")
    ))
}

/// Fetches `url` into `to` (through `to.part`) with curl, HTTPS only, and
/// returns its bytes.
pub fn download(url: &str, to: &Path) -> Result<Vec<u8>, String> {
    let dir = to.parent().ok_or("a download path with no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let part = to.with_extension("part");
    let status = Command::new("curl")
        .args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--retry",
            "3",
            "--connect-timeout",
            "30",
            "--max-time",
            "600",
            "--output",
        ])
        .arg(&part)
        .arg(url)
        .status()
        .map_err(|e| format!("curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(format!("{url}: curl couldn't fetch it ({status})"));
    }
    let data = std::fs::read(&part).map_err(|e| format!("{}: {e}", part.display()))?;
    std::fs::rename(&part, to).map_err(|e| format!("{}: {e}", to.display()))?;
    Ok(data)
}

/// The members of an ar archive (a .deb is one), by name.
pub fn ar_members(data: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    let rest = data
        .strip_prefix(b"!<arch>\n")
        .ok_or("not an ar archive (a .deb file)")?;
    let mut members = Vec::new();
    let mut at = 0;
    while at < rest.len() {
        let header = rest.get(at..at + 60).ok_or("an ar header cut short")?;
        if &header[58..60] != b"`\n" {
            return Err("a damaged ar header".into());
        }
        let field = |r: std::ops::Range<usize>| String::from_utf8_lossy(&header[r]).to_string();
        let name = field(0..16).trim_end().trim_end_matches('/').to_string();
        let size: usize = field(48..58)
            .trim()
            .parse()
            .map_err(|_| "an ar member with no size")?;
        let start = at + 60;
        let body = rest
            .get(start..start + size)
            .ok_or("an ar member cut short")?;
        members.push((name, body));
        at = start + size + size % 2;
    }
    Ok(members)
}

/// Decompresses zstd data, every frame of it.
pub fn unzstd(mut data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while !data.is_empty() {
        let mut frame =
            ruzstd::decoding::StreamingDecoder::new(&mut data).map_err(|e| format!("zstd: {e}"))?;
        frame
            .read_to_end(&mut out)
            .map_err(|e| format!("zstd: {e}"))?;
    }
    Ok(out)
}

/// Takes the [`NEEDED`] files out of one package's data: the regular files
/// into `files`, the symbolic links (name, what they point at) into `links`.
pub fn take_files(
    deb: &[u8],
    files: &mut BTreeMap<String, Vec<u8>>,
    links: &mut Vec<(String, String)>,
) -> Result<(), String> {
    let members = ar_members(deb)?;
    let (name, body) = members
        .iter()
        .find(|(n, _)| n.starts_with("data.tar"))
        .ok_or("no data.tar in the package")?;
    if name != "data.tar.zst" {
        return Err(format!("{name}: only data.tar.zst is read"));
    }
    let tar = unzstd(body)?;
    let mut archive = tar::Archive::new(tar.as_slice());
    for entry in archive.entries().map_err(|e| format!("tar: {e}"))? {
        let mut entry = entry.map_err(|e| format!("tar: {e}"))?;
        let path = String::from_utf8_lossy(&entry.path_bytes()).to_string();
        let path = path.trim_start_matches("./");
        let Some(name) = DIRS.iter().find_map(|d| path.strip_prefix(d)) else {
            continue;
        };
        if !NEEDED.contains(&name) {
            continue;
        }
        let kind = entry.header().entry_type();
        if kind.is_file() {
            let mut data = Vec::new();
            entry
                .read_to_end(&mut data)
                .map_err(|e| format!("tar: {path}: {e}"))?;
            files.insert(name.to_string(), data);
        } else if kind.is_symlink() || kind.is_hard_link() {
            let target = entry
                .link_name_bytes()
                .map(|t| String::from_utf8_lossy(&t).to_string())
                .ok_or_else(|| format!("{path}: a link to nothing"))?;
            links.push((name.to_string(), target));
        }
    }
    Ok(())
}

/// The files, with each link replaced by a copy of the file it points at (a
/// folder of plain files works on every computer, Windows included) and the
/// C library's linker script pointed at its neighbours instead of at
/// `/usr/arm-linux-gnueabihf/lib`; every [`NEEDED`] file must be there.
pub fn finish(
    mut files: BTreeMap<String, Vec<u8>>,
    links: &[(String, String)],
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    for (name, target) in links {
        let base = target.rsplit('/').next().unwrap_or(target);
        let data = files
            .get(base)
            .cloned()
            .ok_or_else(|| format!("{name} points at {target}, which isn't taken"))?;
        files.insert(name.clone(), data);
    }
    for (name, data) in files.iter_mut() {
        if data.starts_with(b"/* GNU ld script") {
            *data = local_script(name, data)?;
        }
    }
    let missing: Vec<&str> = NEEDED
        .iter()
        .copied()
        .filter(|n| !files.contains_key(*n))
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "the packages lack {}: the pins in libs.rs are wrong",
            missing.join(", ")
        ));
    }
    Ok(files)
}

/// A linker script that names its files by their full paths on Ubuntu, made
/// to name them alone: the linker then finds them beside the script.
fn local_script(name: &str, data: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(data).map_err(|_| format!("{name}: not a text script"))?;
    let text = text
        .replace("/usr/arm-linux-gnueabihf/lib/", "")
        .replace("/lib/arm-linux-gnueabihf/", "");
    // Outside its comments, no path may be left.
    let mut code = text.as_str();
    let mut plain = String::new();
    while let Some(i) = code.find("/*") {
        plain.push_str(&code[..i]);
        code = code[i..].find("*/").map_or("", |j| &code[i + j + 2..]);
    }
    plain.push_str(code);
    if plain.contains('/') {
        return Err(format!("{name}: a linker script with a path left in it"));
    }
    Ok(text.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ar(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut out = b"!<arch>\n".to_vec();
        for (name, body) in members {
            out.extend(
                format!(
                    "{name:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                    0,
                    0,
                    0,
                    100644,
                    body.len()
                )
                .bytes(),
            );
            out.extend_from_slice(body);
            if body.len() % 2 == 1 {
                out.push(b'\n');
            }
        }
        out
    }

    fn tar(entries: &[(&str, Result<&[u8], &str>)]) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        for (path, what) in entries {
            let mut h = tar::Header::new_gnu();
            match what {
                Ok(data) => {
                    h.set_entry_type(tar::EntryType::Regular);
                    h.set_size(data.len() as u64);
                    h.set_mode(0o644);
                    b.append_data(&mut h, path, *data).unwrap();
                }
                Err(target) => {
                    h.set_entry_type(tar::EntryType::Symlink);
                    h.set_size(0);
                    b.append_link(&mut h, path, target).unwrap();
                }
            }
        }
        b.into_inner().unwrap()
    }

    fn deb(entries: &[(&str, Result<&[u8], &str>)]) -> Vec<u8> {
        let data = ruzstd::encoding::compress_to_vec(
            tar(entries).as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        ar(&[
            ("debian-binary", b"2.0\n"),
            ("control.tar.zst", b"x"),
            ("data.tar.zst", &data),
        ])
    }

    #[test]
    fn ar_members_are_read_with_their_padding() {
        let a = ar(&[("one", b"abc"), ("two/", b"de")]);
        let m = ar_members(&a).unwrap();
        assert_eq!(
            m,
            vec![("one".into(), &b"abc"[..]), ("two".into(), &b"de"[..])]
        );
        assert!(ar_members(b"not an archive").is_err());
        assert!(ar_members(&a[..a.len() - 1]).is_err());
    }

    #[test]
    fn zstd_with_two_frames_is_read_whole() {
        let mut z = ruzstd::encoding::compress_to_vec(
            &b"first "[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        );
        z.extend(ruzstd::encoding::compress_to_vec(
            &b"second"[..],
            ruzstd::encoding::CompressionLevel::Fastest,
        ));
        assert_eq!(unzstd(&z).unwrap(), b"first second");
    }

    #[test]
    fn only_needed_files_in_the_two_folders_are_taken_and_links_become_copies() {
        let script = b"/* GNU ld script\n   Use the shared library.  */\nOUTPUT_FORMAT(elf32-littlearm)\nGROUP ( /usr/arm-linux-gnueabihf/lib/libc.so.6 /usr/arm-linux-gnueabihf/lib/libc_nonshared.a  AS_NEEDED ( /usr/arm-linux-gnueabihf/lib/ld-linux-armhf.so.3 ) )\n";
        let d = deb(&[
            ("./usr/arm-linux-gnueabihf/lib/libc.so", Ok(script)),
            ("./usr/arm-linux-gnueabihf/lib/libm.so.6", Ok(b"libm")),
            ("./usr/arm-linux-gnueabihf/lib/libm.so", Err("libm.so.6")),
            ("./usr/arm-linux-gnueabihf/lib/libanl.a", Ok(b"not needed")),
            (
                "./usr/arm-linux-gnueabihf/include/crt1.o",
                Ok(b"wrong folder"),
            ),
            (
                "./usr/lib/gcc-cross/arm-linux-gnueabihf/13/libgcc.a",
                Ok(b"libgcc"),
            ),
        ]);
        let mut files = BTreeMap::new();
        let mut links = Vec::new();
        take_files(&d, &mut files, &mut links).unwrap();
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            ["libc.so", "libgcc.a", "libm.so.6"]
        );
        assert_eq!(
            links,
            vec![("libm.so".to_string(), "libm.so.6".to_string())]
        );
        // Short of the other needed files, finish says which.
        let e = finish(files.clone(), &links).unwrap_err();
        assert!(e.contains("crt1.o") && !e.contains("libm.so,"), "{e}");
        for n in NEEDED {
            files
                .entry(n.to_string())
                .or_insert_with(|| n.as_bytes().to_vec());
        }
        let done = finish(files, &links).unwrap();
        assert_eq!(done["libm.so"], b"libm");
        let libc = String::from_utf8(done["libc.so"].clone()).unwrap();
        assert!(
            libc.contains(
                "GROUP ( libc.so.6 libc_nonshared.a  AS_NEEDED ( ld-linux-armhf.so.3 ) )"
            ),
            "{libc}"
        );
    }

    #[test]
    fn a_script_with_another_path_left_is_refused() {
        let s = b"/* GNU ld script */\nGROUP ( /opt/elsewhere/libc.so.6 )\n";
        assert!(local_script("libc.so", s).is_err());
        let ok =
            b"/* GNU ld script\n   paths/in/comments are fine */\nGROUP ( libgcc_s.so.1 -lgcc )\n";
        assert_eq!(local_script("libgcc_s.so", ok).unwrap(), ok.to_vec());
    }

    #[test]
    fn a_link_to_a_file_not_taken_is_an_error() {
        let links = vec![("libm.so".to_string(), "libm.so.6".to_string())];
        assert!(finish(BTreeMap::new(), &links)
            .unwrap_err()
            .contains("libm.so.6"));
    }

    #[test]
    fn the_pins_are_whole_hashes_and_both_places_are_https() {
        for p in &PACKAGES {
            assert_eq!(p.sha256.len(), 64);
            assert!(p
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(p.file.ends_with("_all.deb"));
            for u in urls(p) {
                assert!(u.starts_with("https://") && u.ends_with(p.file), "{u}");
            }
        }
        assert!(stamp_text().lines().count() == PACKAGES.len());
    }
}
