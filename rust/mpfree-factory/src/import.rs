//! Akai's factory kits and samples, from the owner's own eMMC backup.
//!
//! The backup's GPT names the factory content partition `content` (p7 on
//! the MPC One, mounted at /media/acvs-content by Akai's system). It is
//! read with the read-only ext4 reader; nothing in the backup changes.
//!
//! Kits follow offline-factory-kit.py: kit 1 is the first drum program by
//! path whose pads 1 to 16 have at least 8 samples found; kits 2 to N are
//! the next programs by path with at least 12 found, one per name family,
//! while the kits file stays under 48 MiB. Each sound is converted to
//! 44.1 kHz stereo 16-bit, at most 10 s. Without a drum program, a folder
//! of loose drum WAVs makes kit 1.
//!
//! This runs on the owner's own device, for their own content: nothing is
//! copied anywhere else ([factory content policy](../../../docs/dev/project/factory-content-policy.md)).

use crate::kitfile::{self, Sound, MAX_FILE, PAD_NOTES};
use crate::{wav, xpm};
use mpfree_diskimage::ext4::{Ext4, Inode, Kind};
use mpfree_diskimage::{gpt, Offset, ReadAt};
use std::collections::BTreeMap;
use std::io::{self, Write};

/// The GPT name of Akai's factory content partition.
pub const CONTENT: &str = "content";
pub const DEFAULT_KITS: usize = 8;
const MIN_PADS: usize = 8;
const MIN_EXTRA_PADS: usize = 12;
const MAX_XPM: u64 = 4 << 20;
const MAX_WAV: u64 = 64 << 20;
const MAX_ENTRIES: usize = 300_000;
/// Words that make a folder of loose WAVs look like drums.
const DRUM_WORDS: [&str; 13] = [
    "kick", "snare", "clap", "hat", "hh", "tom", "perc", "rim", "crash", "ride", "cymbal",
    "shaker", "snap",
];
/// File endings the pad player plays (mpfree-library's audio formats).
pub const SAMPLE_ENDINGS: [&str; 6] = ["wav", "wave", "aif", "aiff", "aifc", "flac"];

fn bad(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

/// Is this a sample file the pad player can play?
pub fn is_sample(path: &str) -> bool {
    path.rsplit_once('.')
        .is_some_and(|(_, e)| SAMPLE_ENDINGS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The factory content file system, listed.
pub struct Content<F: ReadAt> {
    fs: Ext4<F>,
    /// Every file: path to (inode, size).
    files: BTreeMap<String, (u32, u64)>,
    pub folders: usize,
}

impl<R: ReadAt> Content<Offset<R>> {
    /// The content partition inside a whole-disk image (the backup's
    /// mmcblk0.img).
    pub fn from_disk(disk: R) -> io::Result<Self> {
        let table = gpt::read(&disk)?;
        let (_, part) = table
            .by_name(CONTENT)
            .ok_or_else(|| bad("the backup's partition table has no partition named content"))?;
        let (start, len) = (part.start_bytes(), part.len_bytes());
        Content::from_fs(Ext4::open(Offset {
            inner: disk,
            start,
            len,
        })?)
    }
}

impl<F: ReadAt> Content<F> {
    /// Lists a file system: every file and folder, except lost+found.
    pub fn from_fs(fs: Ext4<F>) -> io::Result<Self> {
        let mut files = BTreeMap::new();
        let mut folders = 0usize;
        fs.walk(&mut |path, ino: &Inode| {
            if path == "/lost+found" || path.starts_with("/lost+found/") {
                return Ok(());
            }
            if files.len() + folders > MAX_ENTRIES {
                return Err(bad(format!(
                    "more than {MAX_ENTRIES} entries in the content partition"
                )));
            }
            match ino.kind() {
                Some(Kind::File) => {
                    files.insert(path.to_string(), (ino.ino, ino.size));
                }
                Some(Kind::Dir) => folders += 1,
                _ => {}
            }
            Ok(())
        })?;
        Ok(Content { fs, files, folders })
    }

    /// Every file and its size, by path.
    pub fn files(&self) -> impl Iterator<Item = (&str, u64)> {
        self.files.iter().map(|(p, (_, s))| (p.as_str(), *s))
    }

    /// The sample files the pad player can play, by path, with their sizes.
    pub fn samples(&self) -> Vec<(String, u64)> {
        self.files()
            .filter(|(p, _)| is_sample(p))
            .map(|(p, s)| (p.to_string(), s))
            .collect()
    }

    /// A file's bytes, if it is at most `max` bytes.
    pub fn read(&self, path: &str, max: u64) -> io::Result<Vec<u8>> {
        let &(ino, size) = self
            .files
            .get(path)
            .ok_or_else(|| bad(format!("{path} is not in the content partition")))?;
        if size > max {
            return Err(bad(format!("{path} is {size} bytes, over {max}")));
        }
        self.fs.read_file(&self.fs.inode(ino)?)
    }

    /// Copies a file into `w`; returns its length.
    pub fn copy(&self, path: &str, w: &mut dyn Write) -> io::Result<u64> {
        let &(ino, _) = self
            .files
            .get(path)
            .ok_or_else(|| bad(format!("{path} is not in the content partition")))?;
        self.fs.copy_file(&self.fs.inode(ino)?, w)
    }

    /// Drum kits from the factory programs, as a kits file, or None when
    /// there is nothing to make kits from. `say` gets the report's lines,
    /// `step(done, total)` the progress over the kits.
    pub fn kits(
        &self,
        count: usize,
        say: &mut dyn FnMut(String),
        step: &mut dyn FnMut(usize, usize),
    ) -> io::Result<Option<Vec<u8>>> {
        let count = count.clamp(1, kitfile::MAX_KITS);
        let sizes: BTreeMap<String, u64> = self
            .files
            .iter()
            .map(|(p, (_, s))| (p.clone(), *s))
            .collect();
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for p in sizes.keys() {
            by_name
                .entry(p.rsplit('/').next().unwrap_or(p).to_lowercase())
                .or_default()
                .push(p.clone());
        }
        let xpms: Vec<&String> = sizes
            .iter()
            .filter(|(p, s)| p.to_ascii_lowercase().ends_with(".xpm") && **s > 0 && **s <= MAX_XPM)
            .map(|(p, _)| p)
            .collect();
        say(format!(
            "{} files, {} folders, {} .xpm programs",
            sizes.len(),
            self.folders,
            xpms.len()
        ));
        let mut candidates: Vec<Candidate> = Vec::new();
        let mut chosen: Option<usize> = None;
        for path in xpms {
            let Ok(data) = self.read(path, MAX_XPM) else {
                continue;
            };
            let Some(prog) = xpm::program(&String::from_utf8_lossy(&data)) else {
                continue;
            };
            let (by_pad, by_note) = xpm::mappings(&prog);
            let dir = match path.rfind('/') {
                Some(0) => "/",
                Some(i) => &path[..i],
                None => "",
            };
            let found = |m: &[Option<xpm::SampleRef>; 16]| -> [Option<String>; 16] {
                std::array::from_fn(|i| {
                    m[i].as_ref()
                        .and_then(|r| xpm::resolve(r, dir, &sizes, &by_name))
                })
            };
            let (fp, fnote) = (found(&by_pad), found(&by_note));
            let n_pad = fp.iter().flatten().count();
            let n_note = fnote.iter().flatten().count();
            let use_note = n_note > n_pad;
            let (refs, wavs) = if use_note {
                (by_note, fnote)
            } else {
                (by_pad, fp)
            };
            let name = if prog.name.is_empty() {
                let base = path.rsplit('/').next().unwrap_or(path);
                base.rsplit_once('.').map_or(base, |(b, _)| b).to_string()
            } else {
                prog.name.clone()
            };
            candidates.push(Candidate {
                path: path.clone(),
                name,
                reading: if use_note { "by note" } else { "by pad number" },
                pads: std::array::from_fn(|i| (refs[i].clone(), wavs[i].clone())),
                found: n_pad.max(n_note),
            });
            if chosen.is_none() && n_pad.max(n_note) >= MIN_PADS {
                chosen = Some(candidates.len() - 1);
            }
        }
        say(format!("drum programs: {}", candidates.len()));
        let first = match chosen {
            Some(i) => candidates[i].clone(),
            None => match loose_wavs(&sizes) {
                Some(c) => {
                    say(format!("no drum program with {MIN_PADS} samples found; using the folder of drum WAVs {}", c.path));
                    c
                }
                None => {
                    say("no drum program and no folder of drum WAVs found".into());
                    return Ok(None);
                }
            },
        };
        let mut plan = vec![first.clone()];
        let mut seen = vec![xpm::family(&first.path)];
        for c in &candidates {
            if plan.len() >= count {
                break;
            }
            if c.path == first.path || c.found < MIN_EXTRA_PADS {
                continue;
            }
            let f = xpm::family(&c.path);
            if seen.contains(&f) {
                continue;
            }
            seen.push(f);
            plan.push(c.clone());
        }
        say(format!("kits: {} (asked for {count})", plan.len()));
        let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
        let mut total = 12usize;
        for (k, c) in plan.iter().enumerate() {
            step(k, plan.len());
            say(format!(
                "kit {}: {} ({}), {} pads found, read {}",
                k + 1,
                c.path,
                c.name,
                c.found,
                c.reading
            ));
            let sounds = self.convert(c, &sizes, say);
            if sounds.is_empty() {
                say(format!("kit {}: no sample could be read; left out", k + 1));
                continue;
            }
            let kit = kitfile::kit_bytes(&sounds);
            let len = kitfile::entry_len(&c.name, &kit);
            if total + len > MAX_FILE {
                say(format!(
                    "kit {}: {len} bytes would take the file over {MAX_FILE} bytes; left out",
                    k + 1
                ));
                continue;
            }
            total += len;
            entries.push((c.name.clone(), kit));
        }
        step(plan.len(), plan.len());
        if entries.is_empty() {
            say("no kit could be made".into());
            return Ok(None);
        }
        let data = kitfile::kits_bytes(&entries);
        debug_assert_eq!(data.len(), total);
        say(format!(
            "kits file: {} kits, {} bytes",
            entries.len(),
            data.len()
        ));
        Ok(Some(data))
    }

    fn convert(
        &self,
        c: &Candidate,
        sizes: &BTreeMap<String, u64>,
        say: &mut dyn FnMut(String),
    ) -> Vec<Sound> {
        let mut sounds = Vec::new();
        for (i, (r, w)) in c.pads.iter().enumerate() {
            let note = PAD_NOTES[i];
            let label = r.as_ref().map(|(s, f)| {
                if s.is_empty() {
                    f.rsplit(['/', '\\']).next().unwrap_or(f).to_string()
                } else {
                    s.clone()
                }
            });
            let Some(w) = w else { continue };
            if sizes.get(w).copied().unwrap_or(u64::MAX) > MAX_WAV {
                say(format!("  pad {:2}: {w} is too big", i + 1));
                continue;
            }
            let data = match self.read(w, MAX_WAV) {
                Ok(d) => d,
                Err(e) => {
                    say(format!("  pad {:2}: {w}: {e}", i + 1));
                    continue;
                }
            };
            match wav::convert(&data) {
                Ok((audio, d)) => {
                    let name = label
                        .filter(|l| !l.is_empty())
                        .unwrap_or_else(|| w.rsplit('/').next().unwrap_or(w).to_string());
                    say(format!(
                        "  pad {:2}  note {note:3}  {name}: {w} ({} Hz, {} ch, {}-bit{}{})",
                        i + 1,
                        d.rate,
                        d.channels,
                        d.bits,
                        if d.float { " float" } else { "" },
                        if d.cut { ", cut to 10 s" } else { "" }
                    ));
                    sounds.push(Sound { note, name, audio });
                }
                Err(e) => say(format!("  pad {:2}: {w}: {e}", i + 1)),
            }
        }
        sounds
    }
}

#[derive(Clone)]
struct Candidate {
    path: String,
    name: String,
    reading: &'static str,
    pads: [(Option<xpm::SampleRef>, Option<String>); 16],
    found: usize,
}

/// The folder with the most drum-named WAVs, as a kit of up to 16.
fn loose_wavs(sizes: &BTreeMap<String, u64>) -> Option<Candidate> {
    let mut folders: BTreeMap<&str, Vec<&String>> = BTreeMap::new();
    for p in sizes.keys() {
        let base = p.rsplit('/').next().unwrap_or(p).to_lowercase();
        if base.ends_with(".wav") && DRUM_WORDS.iter().any(|w| base.contains(w)) {
            let dir = p.rfind('/').map_or("", |i| &p[..i]);
            folders.entry(dir).or_default().push(p);
        }
    }
    let best = folders
        .iter()
        .max_by(|a, b| a.1.len().cmp(&b.1.len()).then(b.0.cmp(a.0)))?;
    let rank = |p: &String| {
        let b = p.rsplit('/').next().unwrap_or(p).to_lowercase();
        (
            DRUM_WORDS
                .iter()
                .position(|w| b.contains(w))
                .unwrap_or(DRUM_WORDS.len()),
            b,
        )
    };
    let mut wavs = best.1.clone();
    wavs.sort_by_key(|p| rank(p));
    wavs.truncate(16);
    let pads = std::array::from_fn(|i| match wavs.get(i) {
        Some(w) => (
            Some((w.rsplit('/').next().unwrap_or(w).to_string(), String::new())),
            Some((*w).clone()),
        ),
        None => (None, None),
    });
    Some(Candidate {
        path: format!("{}/", best.0),
        name: best.0.rsplit('/').next().unwrap_or(best.0).to_string(),
        reading: "loose WAVs",
        pads,
        found: wavs.len(),
    })
}

/// A small fake eMMC backup with a content partition, for tests here and
/// in the pad player: drum programs, their WAVs and a loose sample.
pub mod testdisk {
    use crate::wav::wav_bytes;
    use mpfree_diskimage::ext2w::{self, Body, Data, Meta, Tree};
    use mpfree_diskimage::{gpt, Mem, WriteAt};

    fn program(name: &str, samples: &[&str]) -> String {
        let mut s = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<MPCVObject>\n<Program type=\"Drum\">\n<ProgramName>{name}</ProgramName>\n<Instruments>\n");
        for (i, smp) in samples.iter().enumerate() {
            s += &format!("<Instrument number=\"{}\"><Layers><Layer number=\"1\"><SampleName>{smp}</SampleName><SampleFile></SampleFile></Layer></Layers></Instrument>\n", i + 1);
        }
        s + "</Instruments>\n</Program>\n</MPCVObject>\n"
    }

    /// The disk's bytes. Kits "Alpha Kit" (16 pads), "Alpha-Kit 2" (same
    /// family, left out), "Beta Kit" (12 pads) and "Gamma" (4 pads, too few).
    pub fn disk() -> Vec<u8> {
        let t = 1_763_200_000;
        let m = |perm| Meta::new(perm, 0, 0, t);
        let mut tree = Tree::new(m(0o755));
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        let names: Vec<String> = (1..=16).map(|i| format!("Hit {i:02}")).collect();
        for (dir, prog, n) in [
            ("Alpha", "Alpha Kit", 16),
            ("Alpha", "Alpha-Kit 2", 16),
            ("Beta", "Beta Kit", 12),
            ("Gamma", "Gamma", 4),
        ] {
            let refs: Vec<&str> = names[..n].iter().map(|s| s.as_str()).collect();
            files.push((
                format!("/Drums/{dir}/{prog}.xpm"),
                program(prog, &refs).into_bytes(),
            ));
            for (i, s) in refs.iter().enumerate() {
                let samples: Vec<i32> = (0..2000)
                    .map(|k| ((k * (i + 3)) % 200) as i32 * 100 - 10000)
                    .collect();
                files.push((
                    format!("/Drums/{dir}/{s}.wav"),
                    wav_bytes(48_000, 1, 16, &samples),
                ));
            }
        }
        files.push((
            "/Loops/Groove 90.wav".into(),
            wav_bytes(44_100, 2, 24, &[1000; 800]),
        ));
        files.push(("/Loops/readme.txt".into(), b"not a sample".to_vec()));
        for (path, data) in files {
            let (dir, name) = path.rsplit_once('/').unwrap();
            let d = tree.mkdir_all(dir, m(0o755)).unwrap();
            if tree.child(d, name.as_bytes()).is_none() {
                tree.add(d, name.as_bytes(), Body::File(Data::Bytes(data)), m(0o644))
                    .unwrap();
            }
        }
        let fs = Mem::new();
        let opts = ext2w::Options {
            label: "content".into(),
            uuid: [3; 16],
            time: t,
            size: ext2w::Size::Fit {
                spare_bytes: 0,
                spare_inodes: 0,
            },
        };
        let s = ext2w::write(&tree, &opts, &fs).unwrap();
        fs.set_len(s.bytes);
        let fs = fs.into_inner();
        // GPT: a small "rootfs" then "content", as on the MPC (names matter, numbers don't).
        let sectors = 2048 + 2048 + (fs.len() as u64 / 512) + 2048;
        let disk = Mem::new();
        disk.set_len(sectors * 512);
        let lin = gpt::guid(gpt::LINUX_FS).unwrap();
        let part = |first: u64, last: u64, name: &str| gpt::Part {
            type_guid: lin,
            guid: gpt::random_guid(),
            first,
            last,
            attrs: 0,
            name: name.into(),
        };
        let content_first = 4096;
        let table = gpt::Table {
            disk_guid: gpt::random_guid(),
            parts: vec![
                Some(part(2048, 4095, "rootfs")),
                Some(part(
                    content_first,
                    content_first + fs.len() as u64 / 512 - 1,
                    "content",
                )),
            ],
        };
        gpt::write(&disk, sectors, &table).unwrap();
        disk.write_at(&fs, content_first * 512).unwrap();
        disk.into_inner()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kits_and_samples_from_a_backup() {
        let disk = testdisk::disk();
        let c = Content::from_disk(&disk).unwrap();
        let mut report = Vec::new();
        let mut steps = Vec::new();
        let data = c
            .kits(DEFAULT_KITS, &mut |l| report.push(l), &mut |d, t| {
                steps.push((d, t))
            })
            .unwrap()
            .unwrap();
        let kits = kitfile::parse_kits(&data).unwrap();
        let names: Vec<&str> = kits.iter().map(|k| k.0.as_str()).collect();
        assert_eq!(names, ["Alpha Kit", "Beta Kit"], "{report:#?}");
        assert_eq!(kits[0].1.len(), 16);
        assert_eq!(kits[1].1.len(), 12);
        assert_eq!(kits[0].1[0].note, PAD_NOTES[0]);
        assert_eq!(kits[0].1[0].name, "Hit 01");
        // 2000 frames at 48 kHz become 1837 at 44.1 kHz.
        assert_eq!(kits[0].1[0].audio.len() / 2, 1837);
        assert_eq!(steps.last(), Some(&(2, 2)));
        let samples = c.samples();
        assert_eq!(samples.len(), 16 + 16 + 12 + 4 + 1 - 16); // Alpha-Kit 2 shares Alpha's WAVs
        assert!(samples.iter().any(|(p, _)| p == "/Loops/Groove 90.wav"));
        assert!(!samples.iter().any(|(p, _)| p.ends_with(".txt")));
        let mut out = Vec::new();
        let n = c.copy("/Loops/Groove 90.wav", &mut out).unwrap();
        assert_eq!(n as usize, out.len());
        assert_eq!(&out[..4], b"RIFF");
    }

    #[test]
    fn a_disk_without_content_is_refused() {
        let mut disk = testdisk::disk();
        // Rename "content" in the GPT: the table's CRC then fails, which is refused too.
        let at = disk
            .windows(13)
            .position(|w| w == "c\0o\0n\0t\0e\0n\0t".as_bytes())
            .unwrap();
        disk[at] = b'k';
        assert!(Content::from_disk(&disk).is_err());
    }
}
