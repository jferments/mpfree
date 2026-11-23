//! What an ext4 partition of a disk image holds, read with MPFree's own
//! reader ([`mpfree_diskimage::ext4`]). For SD card step S3: the MPC's
//! Internal drive (partition 8, GPT name `data`) in the owner's eMMC backup,
//! on the workstation only. It answers what S3's design leaves to settle:
//! whether the reader takes the drive's features, which top folders it has,
//! how big its folders are and how long they take to list, and whether its
//! files read whole. It opens the image read-only and writes nothing; the
//! report goes to standard output and holds names and counts, never a
//! file's contents.
//!
//! ```text
//! mpfree-ext4-survey --newest-backup [--partition NAME]
//! mpfree-ext4-survey --image FILE [--partition NAME]
//! ```
//!
//! - `--newest-backup`: the newest `~/mpfree-backups/mpc-one-emmc-*/mmcblk0.img`
//!   (by name, which starts with its UTC time; `.part` folders left out).
//! - `--image FILE`: a whole-disk image with a GPT.
//! - `--partition NAME`: the partition's GPT name; `data` if not given.
//!
//! It exits 0 when the reader opened the partition and read every folder
//! and file, 1 when it couldn't (the report says where it stopped), 2 for
//! wrong arguments.

use mpfree_diskimage::ext4::{Ext4, Inode, Kind};
use mpfree_diskimage::{gpt, Offset, ReadAt};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

/// The endings the pad player plays (mpfree-library's audio formats).
const SOUND_ENDINGS: [&str; 6] = ["wav", "wave", "aif", "aiff", "aifc", "flac"];
/// The largest sound the place would read: the Sample page's own length
/// check (`mpfree_sampler::wav::MAX_SAMPLES` × 8 bytes, plus 1 MiB).
const MOST_SOUND: u64 = (64 << 20) * 8 + (1 << 20);
/// Inode flag: the folder is hash-indexed (htree).
const INDEX_FL: u32 = 0x1000;
/// Inode flag: the data is kept in the inode itself.
const INLINE_DATA_FL: u32 = 0x1000_0000;

const COMPAT: [(u32, &str); 9] = [
    (0x1, "dir_prealloc"),
    (0x2, "imagic_inodes"),
    (0x4, "has_journal"),
    (0x8, "ext_attr"),
    (0x10, "resize_inode"),
    (0x20, "dir_index"),
    (0x200, "sparse_super2"),
    (0x400, "fast_commit"),
    (0x1000, "orphan_file"),
];
const INCOMPAT: [(u32, &str); 16] = [
    (0x1, "compression"),
    (0x2, "filetype"),
    (0x4, "needs_recovery"),
    (0x8, "journal_dev"),
    (0x10, "meta_bg"),
    (0x40, "extent"),
    (0x80, "64bit"),
    (0x100, "mmp"),
    (0x200, "flex_bg"),
    (0x400, "ea_inode"),
    (0x1000, "dirdata"),
    (0x2000, "metadata_csum_seed"),
    (0x4000, "large_dir"),
    (0x8000, "inline_data"),
    (0x10000, "encrypt"),
    (0x20000, "casefold"),
];
const RO_COMPAT: [(u32, &str); 14] = [
    (0x1, "sparse_super"),
    (0x2, "large_file"),
    (0x4, "btree_dir"),
    (0x8, "huge_file"),
    (0x10, "gdt_csum"),
    (0x20, "dir_nlink"),
    (0x40, "extra_isize"),
    (0x100, "quota"),
    (0x200, "bigalloc"),
    (0x400, "metadata_csum"),
    (0x1000, "readonly"),
    (0x2000, "project"),
    (0x8000, "verity"),
    (0x10000, "orphan_present"),
];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut image = None;
    let mut newest = false;
    let mut partition = "data".to_string();
    let mut i = 0;
    while i < args.len() {
        match (args[i].as_str(), args.get(i + 1)) {
            ("--newest-backup", _) => newest = true,
            ("--image", Some(v)) => {
                image = Some(PathBuf::from(v));
                i += 1;
            }
            ("--partition", Some(v)) => {
                partition = v.clone();
                i += 1;
            }
            (a, _) => {
                eprintln!(
                    "mpfree-ext4-survey: {a}: not an option this takes (see its source's top)"
                );
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    let image = match (image, newest) {
        (Some(p), false) => p,
        (None, true) => {
            let Some(home) = std::env::var_os("HOME") else {
                eprintln!("mpfree-ext4-survey: HOME isn't set, so there is no ~/mpfree-backups");
                return ExitCode::from(2);
            };
            match newest_backup(&Path::new(&home).join("mpfree-backups")) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("mpfree-ext4-survey: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        _ => {
            eprintln!("mpfree-ext4-survey: give --newest-backup or --image FILE (one of them)");
            return ExitCode::from(2);
        }
    };
    let out = io::stdout();
    let mut out = out.lock();
    match survey(&image, &partition, &mut out) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            let _ = writeln!(out, "stopped: {e}");
            ExitCode::FAILURE
        }
    }
}

/// The newest backup's disk image in `dir`: the last `mpc-one-emmc-*`
/// folder by name that holds `mmcblk0.img`.
fn newest_backup(dir: &Path) -> Result<PathBuf, String> {
    let list = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut found: Vec<PathBuf> = list
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("mpc-one-emmc-") && !n.ends_with(".part")
        })
        .map(|e| e.path().join("mmcblk0.img"))
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found.pop().ok_or_else(|| {
        format!(
            "no backup in {} (no mpc-one-emmc-*/mmcblk0.img)",
            dir.display()
        )
    })
}

/// Names of the bits set in `word`, from `names`, and any left unnamed.
fn bits(word: u32, names: &[(u32, &str)]) -> String {
    let mut out = Vec::new();
    let mut left = word;
    for (bit, name) in names {
        if word & bit != 0 {
            out.push((*name).to_string());
            left &= !bit;
        }
    }
    if left != 0 {
        out.push(format!("unnamed {left:#x}"));
    }
    if out.is_empty() {
        "none".to_string()
    } else {
        out.join(" ")
    }
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

/// What the walk found.
#[derive(Default)]
struct Tally {
    files: u64,
    bytes: u64,
    folders: u64,
    hashed: u64,
    links: u64,
    special: u64,
    deepest: usize,
    /// The folder with the most entries: (entries, path, hashed, time to list).
    largest: Option<(usize, String, bool, Duration)>,
    slowest_list: Duration,
    /// Files whose blocks don't cover their size (holes), and of them sounds.
    holed: u64,
    holed_sounds: u64,
    hole_bytes: u64,
    endings: BTreeMap<String, (u64, u64)>,
    sounds: u64,
    largest_sound: u64,
    over_sound_limit: u64,
    /// Files read whole as a stream, their bytes, the time, and the slowest.
    read: u64,
    read_bytes: u64,
    read_time: Duration,
    slowest_read: (Duration, u64),
    /// `.wav`/`.wave` files whose first 12 bytes are a RIFF/RF64 WAVE header.
    wav_headers: u64,
    wav_not: u64,
    errors: Vec<String>,
    /// Files and folders under the folders the place leaves out.
    hidden: BTreeMap<String, u64>,
    /// The reads' buffer, 1 MiB once made.
    buf: Vec<u8>,
}

/// The ending of a name, lowercased ("" when it has none).
fn ending(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, e)) if !stem.is_empty() && e.len() <= 8 => e.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// The top folder entry the place leaves out that `path` is in, if any:
/// Akai's `system` (its /etc and /var layers), `lost+found`, and names
/// starting with a dot.
fn hidden_under(path: &str) -> Option<&str> {
    let top = path.trim_start_matches('/').split('/').next()?;
    (top == "system" || top == "lost+found" || top.starts_with('.')).then_some(top)
}

/// Reads the partition, writing the report to `out`. Ok(true) when every
/// folder and file read; Ok(false) when the report has errors.
fn survey(image: &Path, name: &str, out: &mut dyn Write) -> io::Result<bool> {
    let disk = File::open(image)?;
    let size = disk.metadata()?.len();
    writeln!(out, "image: {} ({size} bytes)", image.display())?;
    let table = gpt::read(&disk)?;
    let names: Vec<String> = table
        .parts
        .iter()
        .enumerate()
        .filter_map(|(i, p)| p.as_ref().map(|p| format!("{} {}", i + 1, p.name)))
        .collect();
    writeln!(out, "partitions: {}", names.join(", "))?;
    let Some((number, part)) = table.by_name(name) else {
        writeln!(out, "stopped: no single partition named {name}")?;
        return Ok(false);
    };
    let (start, len) = (part.start_bytes(), part.len_bytes());
    writeln!(
        out,
        "partition: {name}, number {number}, from byte {start}, {len} bytes"
    )?;
    let part = Offset {
        inner: disk,
        start,
        len,
    };
    let mut sb = [0u8; 1024];
    part.read_at(&mut sb, 1024)?;
    writeln!(
        out,
        "features: compat {}; incompat {}; ro_compat {}",
        bits(le32(&sb, 0x5C), &COMPAT),
        bits(le32(&sb, 0x60), &INCOMPAT),
        bits(le32(&sb, 0x64), &RO_COMPAT)
    )?;
    let started = Instant::now();
    let fs = match Ext4::open(part) {
        Ok(fs) => fs,
        Err(e) => {
            writeln!(out, "reader: refuses it: {e}")?;
            return Ok(false);
        }
    };
    writeln!(
        out,
        "reader: opens it in {}: label {:?}, {} blocks of {} bytes, {} inodes in {} groups",
        ms(started.elapsed()),
        fs.label,
        fs.blocks_count,
        fs.block_size,
        fs.inodes_count,
        fs.groups
    )?;
    writeln!(
        out,
        "journal: {}",
        if fs.needs_recovery() {
            "unclean (needs_recovery set: the newest changes may be only in the journal, which the reader never replays)"
        } else {
            "clean"
        }
    )?;
    let root = fs.inode(mpfree_diskimage::ext4::ROOT)?;
    let mut top = Vec::new();
    for e in fs.read_dir(&root)? {
        let n = String::from_utf8_lossy(&e.name).into_owned();
        let kind = match fs.inode(e.ino).map(|i| i.kind()) {
            Ok(Some(Kind::Dir)) => "folder",
            Ok(Some(Kind::File)) => "file",
            Ok(Some(Kind::Symlink)) => "link",
            Ok(_) => "other",
            Err(_) => "unreadable",
        };
        let left_out = if hidden_under(&n).is_some() {
            ", left out by the place"
        } else {
            ""
        };
        top.push(format!("{n} ({kind}{left_out})"));
    }
    top.sort();
    writeln!(out, "top folder: {}", top.join(", "))?;

    let mut t = Tally::default();
    let walked = Instant::now();
    let walk = fs.walk(&mut |path, ino| {
        tally(&fs, path, ino, &mut t);
        Ok(())
    });
    // The walk's own time: the reads below are timed apart.
    let walk_time = walked.elapsed().saturating_sub(t.read_time);
    match &walk {
        Ok(()) => writeln!(
            out,
            "walk: whole, in {} (reading the files apart)",
            ms(walk_time)
        )?,
        Err(e) => writeln!(
            out,
            "walk: stopped after {} (reading the files apart): {e}",
            ms(walk_time)
        )?,
    }
    writeln!(
        out,
        "entries: {} files ({} bytes), {} folders ({} hashed), {} links (never followed), {} other; deepest {} folders down",
        t.files, t.bytes, t.folders, t.hashed, t.links, t.special, t.deepest
    )?;
    let hidden: Vec<String> = t.hidden.iter().map(|(k, v)| format!("{k} {v}")).collect();
    writeln!(
        out,
        "left out by the place (entries under each): {}",
        if hidden.is_empty() {
            "none".to_string()
        } else {
            hidden.join(", ")
        }
    )?;
    if let Some((n, path, hashed, took)) = &t.largest {
        writeln!(
            out,
            "largest folder: {path}, {n} entries, {}, listed in {}; slowest listing {}",
            if *hashed { "hashed" } else { "not hashed" },
            ms(*took),
            ms(t.slowest_list)
        )?;
    }
    let mut endings: Vec<_> = t.endings.iter().collect();
    endings.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
    let shown: Vec<String> = endings
        .iter()
        .take(16)
        .map(|(e, (n, b))| {
            let e = if e.is_empty() { "(none)" } else { e.as_str() };
            format!("{e} {n} ({b} bytes)")
        })
        .collect();
    writeln!(out, "endings (shown files): {}", shown.join(", "))?;
    writeln!(
        out,
        "sounds (shown): {}, the largest {} bytes; {} over the place's limit of {MOST_SOUND} bytes",
        t.sounds, t.largest_sound, t.over_sound_limit
    )?;
    writeln!(
        out,
        "wav headers: {} RIFF/RF64 WAVE, {} not",
        t.wav_headers, t.wav_not
    )?;
    writeln!(
        out,
        "holes: {} files whose blocks don't cover their size ({} of them sounds), {} bytes read as zeros",
        t.holed, t.holed_sounds, t.hole_bytes
    )?;
    let secs = t.read_time.as_secs_f64();
    writeln!(
        out,
        "read: {} files whole as streams, {} bytes in {:.2} s ({:.0} MB/s); the slowest {} ({} bytes)",
        t.read,
        t.read_bytes,
        secs,
        if secs > 0.0 {
            t.read_bytes as f64 / secs / 1e6
        } else {
            0.0
        },
        ms(t.slowest_read.0),
        t.slowest_read.1
    )?;
    writeln!(out, "errors: {}", t.errors.len())?;
    for e in t.errors.iter().take(20) {
        writeln!(out, "  {e}")?;
    }
    if t.errors.len() > 20 {
        writeln!(out, "  … and {} more", t.errors.len() - 20)?;
    }
    let ok = walk.is_ok() && t.errors.is_empty();
    writeln!(
        out,
        "RESULT: {}",
        if ok {
            "every folder and file read"
        } else {
            "the reader stopped somewhere: see above"
        }
    )?;
    Ok(ok)
}

/// Counts one entry of the walk, and reads it as the place would.
fn tally<R: ReadAt>(fs: &Ext4<R>, path: &str, ino: &Inode, t: &mut Tally) {
    if path == "/" {
        return;
    }
    let depth = path.matches('/').count();
    if let Some(top) = hidden_under(path) {
        *t.hidden.entry(top.to_string()).or_default() += 1;
    }
    let shown = hidden_under(path).is_none();
    match ino.kind() {
        Some(Kind::Dir) => {
            t.folders += 1;
            t.deepest = t.deepest.max(depth);
            let hashed = ino.flags & INDEX_FL != 0;
            if hashed {
                t.hashed += 1;
            }
            let started = Instant::now();
            match fs.read_dir(ino) {
                Ok(list) => {
                    let took = started.elapsed();
                    t.slowest_list = t.slowest_list.max(took);
                    if t.largest.as_ref().is_none_or(|(n, ..)| list.len() > *n) {
                        t.largest = Some((list.len(), path.to_string(), hashed, took));
                    }
                }
                Err(e) => t.errors.push(format!("{path}: listing: {e}")),
            }
        }
        Some(Kind::File) => {
            t.files += 1;
            t.bytes += ino.size;
            let held = if ino.flags & INLINE_DATA_FL != 0 {
                ino.size
            } else {
                let attrs = if ino.file_acl != 0 { fs.block_size } else { 0 };
                (ino.sectors * 512).saturating_sub(attrs)
            };
            let whole = ino.size.div_ceil(fs.block_size) * fs.block_size;
            let name = path.rsplit('/').next().unwrap_or(path);
            let end = ending(name);
            let sound = SOUND_ENDINGS.contains(&end.as_str());
            if ino.flags & INLINE_DATA_FL == 0 && held < whole {
                t.holed += 1;
                t.hole_bytes += whole - held;
                if sound {
                    t.holed_sounds += 1;
                }
            }
            if shown {
                let e = t.endings.entry(end.clone()).or_default();
                e.0 += 1;
                e.1 += ino.size;
                if sound {
                    t.sounds += 1;
                    t.largest_sound = t.largest_sound.max(ino.size);
                    if ino.size > MOST_SOUND {
                        t.over_sound_limit += 1;
                    }
                }
            }
            read_whole(fs, path, ino, &end, t);
        }
        Some(Kind::Symlink) => t.links += 1,
        _ => t.special += 1,
    }
}

/// Reads a file whole as a stream, 1 MiB at a time, checking it gives
/// exactly its size, and a WAV's first 12 bytes.
fn read_whole<R: ReadAt>(fs: &Ext4<R>, path: &str, ino: &Inode, end: &str, t: &mut Tally) {
    let started = Instant::now();
    let mut r = match fs.open_file(ino) {
        Ok(r) => r,
        Err(e) => {
            t.errors.push(format!("{path}: opening: {e}"));
            return;
        }
    };
    let mut buf = std::mem::take(&mut t.buf);
    buf.resize(1 << 20, 0);
    let mut got = 0u64;
    let mut head = Vec::new();
    loop {
        match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if head.len() < 12 {
                    let take = (12 - head.len()).min(n);
                    head.extend_from_slice(&buf[..take]);
                }
                got += n as u64;
            }
            Err(e) => {
                t.errors.push(format!("{path}: reading at byte {got}: {e}"));
                t.buf = buf;
                return;
            }
        }
    }
    t.buf = buf;
    let took = started.elapsed();
    if got != ino.size {
        t.errors
            .push(format!("{path}: gave {got} bytes of {}", ino.size));
        return;
    }
    t.read += 1;
    t.read_bytes += got;
    t.read_time += took;
    if took > t.slowest_read.0 {
        t.slowest_read = (took, got);
    }
    if (end == "wav" || end == "wave") && hidden_under(path).is_none() {
        let riff = head.len() == 12
            && (&head[0..4] == b"RIFF" || &head[0..4] == b"RF64")
            && &head[8..12] == b"WAVE";
        if riff {
            t.wav_headers += 1;
        } else {
            t.wav_not += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};
    use std::process::Command;

    fn tool(names: &[&str]) -> Option<String> {
        for n in names {
            for dir in ["/usr/sbin", "/sbin"] {
                let p = format!("{dir}/{n}");
                if Path::new(&p).exists() {
                    return Some(p);
                }
            }
        }
        None
    }

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn wav(len: usize) -> Vec<u8> {
        let mut v = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        v.extend((0..len.saturating_sub(16)).map(|i| (i * 7 + 3) as u8));
        v
    }

    /// A disk image with a GPT whose partition 2, `data`, holds an ext4
    /// filesystem made from a folder like the Internal drive's.
    fn disk(dir: &Path) -> Option<PathBuf> {
        let mke2fs = tool(&["mke2fs"])?;
        let src = dir.join("src");
        let drums = src.join("Samples/Drums");
        std::fs::create_dir_all(&drums).unwrap();
        for i in 0..600 {
            std::fs::write(drums.join(format!("Kick {i:04}.wav")), wav(200 + i * 3)).unwrap();
        }
        // A sound with 8 KiB of silence in its middle: mke2fs -d stores
        // each whole block of zeros as a hole.
        let mut silent = wav(4096);
        silent.extend(vec![0u8; 8192]);
        silent.extend(wav(4096));
        std::fs::write(src.join("Samples/Silent middle.wav"), silent).unwrap();
        std::fs::write(src.join("Samples/not a wav.wav"), b"hello").unwrap();
        std::fs::create_dir_all(src.join("Projects/Song/Song_[ProjectData]")).unwrap();
        std::fs::write(src.join("Projects/Song/Song.xpj"), b"{}").unwrap();
        std::fs::create_dir_all(src.join("system/etc")).unwrap();
        std::fs::write(src.join("system/etc/hostname"), b"mpc\n").unwrap();
        std::fs::write(src.join(".hidden"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("Samples", src.join("link")).unwrap();
        let fsimg = dir.join("data.img");
        File::create(&fsimg).unwrap().set_len(8 << 20).unwrap();
        let out = Command::new(mke2fs)
            .args([
                "-q",
                "-F",
                "-t",
                "ext4",
                "-L",
                "data",
                "-E",
                "root_owner=0:0",
            ])
            .arg("-d")
            .arg(&src)
            .arg(&fsimg)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "mke2fs: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        if let Some(e2fsck) = tool(&["e2fsck"]) {
            // Index the big folder (htree), as a long-used drive's is.
            let out = Command::new(e2fsck)
                .args(["-f", "-y", "-D"])
                .arg(&fsimg)
                .output()
                .unwrap();
            assert!(out.status.code().unwrap_or(9) <= 1, "e2fsck");
        }
        let disk = dir.join("mmcblk0.img");
        let sectors = 2048 + 2 * 16384 + 2048;
        let f = File::create(&disk).unwrap();
        f.set_len(sectors * 512).unwrap();
        let linux = gpt::guid(gpt::LINUX_FS).unwrap();
        let part = |first: u64, name: &str| {
            Some(gpt::Part {
                type_guid: linux,
                guid: gpt::random_guid(),
                first,
                last: first + 16383,
                attrs: 0,
                name: name.into(),
            })
        };
        let table = gpt::Table {
            disk_guid: gpt::random_guid(),
            parts: vec![part(2048, "content"), part(2048 + 16384, "data")],
        };
        gpt::write(&f, sectors, &table).unwrap();
        let mut f = f;
        f.seek(SeekFrom::Start((2048 + 16384) * 512)).unwrap();
        f.write_all(&std::fs::read(&fsimg).unwrap()).unwrap();
        Some(disk)
    }

    fn run(image: &Path, name: &str) -> (bool, String) {
        let mut out = Vec::new();
        let ok = survey(image, name, &mut out).unwrap();
        (ok, String::from_utf8(out).unwrap())
    }

    fn line<'a>(report: &'a str, key: &str) -> &'a str {
        report
            .lines()
            .find(|l| l.starts_with(key))
            .unwrap_or_else(|| panic!("no {key} line in:\n{report}"))
    }

    #[test]
    fn surveys_a_drive_like_the_internal_one() {
        let scratch = Scratch(
            std::env::temp_dir().join(format!("mpfree-ext4-survey-{}", std::process::id())),
        );
        std::fs::create_dir_all(&scratch.0).unwrap();
        let Some(image) = disk(&scratch.0) else {
            eprintln!("mke2fs not installed; skipping");
            return;
        };
        let (ok, r) = run(&image, "data");
        assert!(ok, "{r}");
        assert_eq!(line(&r, "partitions:"), "partitions: 1 content, 2 data");
        assert!(line(&r, "partition:")
            .starts_with("partition: data, number 2, from byte 9437184, 8388608 bytes"));
        assert!(line(&r, "features:").contains("has_journal"), "{r}");
        assert!(line(&r, "features:").contains("extent"), "{r}");
        assert!(line(&r, "reader:").contains("label \"data\""), "{r}");
        assert_eq!(line(&r, "journal:"), "journal: clean");
        assert_eq!(
            line(&r, "top folder:"),
            "top folder: .hidden (file, left out by the place), Projects (folder), Samples (folder), \
             link (link), lost+found (folder, left out by the place), system (folder, left out by the place)"
        );
        assert!(line(&r, "walk:").starts_with("walk: whole"), "{r}");
        assert!(
            line(&r, "entries:").starts_with("entries: 605 files"),
            "{r}"
        );
        assert!(line(&r, "entries:").contains("1 links"), "{r}");
        assert_eq!(
            line(&r, "left out by the place"),
            "left out by the place (entries under each): .hidden 1, lost+found 1, system 3"
        );
        let largest = line(&r, "largest folder:");
        assert!(
            largest.starts_with("largest folder: /Samples/Drums, 600 entries"),
            "{r}"
        );
        if tool(&["e2fsck"]).is_some() {
            assert!(largest.contains(", hashed,"), "{r}");
        }
        assert!(
            line(&r, "endings (shown files):").starts_with("endings (shown files): wav 602"),
            "{r}"
        );
        assert!(
            line(&r, "sounds (shown):")
                .starts_with("sounds (shown): 602, the largest 16384 bytes; 0 over"),
            "{r}"
        );
        assert_eq!(
            line(&r, "wav headers:"),
            "wav headers: 601 RIFF/RF64 WAVE, 1 not"
        );
        assert!(
            line(&r, "holes:").starts_with(
                "holes: 1 files whose blocks don't cover their size (1 of them sounds), 8192 bytes"
            ),
            "{r}"
        );
        assert!(
            line(&r, "read:").starts_with("read: 605 files whole as streams"),
            "{r}"
        );
        assert_eq!(line(&r, "errors:"), "errors: 0");
        assert_eq!(line(&r, "RESULT:"), "RESULT: every folder and file read");
    }

    #[test]
    fn says_when_the_partition_is_missing_or_not_ext4() {
        let scratch = Scratch(
            std::env::temp_dir().join(format!("mpfree-ext4-survey-no-{}", std::process::id())),
        );
        std::fs::create_dir_all(&scratch.0).unwrap();
        let Some(image) = disk(&scratch.0) else {
            eprintln!("mke2fs not installed; skipping");
            return;
        };
        let (ok, r) = run(&image, "nope");
        assert!(!ok);
        assert_eq!(
            line(&r, "stopped:"),
            "stopped: no single partition named nope"
        );
        // `content` is all zeros here: no ext4 in it.
        let (ok, r) = run(&image, "content");
        assert!(!ok);
        assert_eq!(
            line(&r, "features:"),
            "features: compat none; incompat none; ro_compat none"
        );
        assert!(
            line(&r, "reader: refuses it:").contains("no magic number"),
            "{r}"
        );
        assert!(!r.contains("RESULT"), "{r}");
    }

    #[test]
    fn finds_the_newest_backup_by_name() {
        let scratch = Scratch(
            std::env::temp_dir().join(format!("mpfree-ext4-survey-new-{}", std::process::id())),
        );
        let dir = scratch.0.join("mpfree-backups");
        for (name, img) in [
            ("mpc-one-emmc-20261109T233604Z-datprr67", true),
            ("mpc-one-emmc-20261117T173519Z", true),
            ("mpc-one-emmc-20261120T000000Z", false),
            ("mpc-one-emmc-20261121T000000Z.part", true),
            ("other", true),
        ] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            if img {
                std::fs::write(dir.join(name).join("mmcblk0.img"), b"").unwrap();
            }
        }
        assert_eq!(
            newest_backup(&dir).unwrap(),
            dir.join("mpc-one-emmc-20261117T173519Z/mmcblk0.img")
        );
        let e = newest_backup(&scratch.0.join("none")).unwrap_err();
        assert!(e.contains("none"), "{e}");
        assert_eq!(
            bits(0x4 | 0x40 | 0x4000_0000, &INCOMPAT),
            "needs_recovery extent unnamed 0x40000000"
        );
        assert_eq!(ending("Kick 01.WAV"), "wav");
        assert_eq!(ending(".hidden"), "");
        assert_eq!(hidden_under("/system/etc"), Some("system"));
        assert_eq!(hidden_under("/Samples/x"), None);
    }
}
