//! `mpfree-disktest --cut-test` and `--after-cut`: the pull test (WP in
//! docs/dev/design/disk-streaming.md), and later the cut test (D6).
//!
//! `--cut-test` records [`TAKES`] takes through MPFree's own recorder
//! (the recording file protocol's writer, at its defaults) while an import
//! beside them publishes files one after another the way checked copies
//! are published ([`mpfree_stream::publish`]). Every sample of every take,
//! and every byte of every import, is one only its place has, so what
//! comes back can be checked exactly. As it goes it prints what it has
//! been promised: each take's committed frames (synced, with its
//! checkpoint) and the imports published (synced and named). That text
//! must be kept off the stick (a terminal, `tee` to another disk, or the
//! workstation's log over ssh), because the stick is then pulled, or the
//! power cut, mid-take. It never deletes its takes or imports: that is
//! `--after-cut`'s job.
//!
//! `--after-cut --promised FILE` reads the last promise in that saved text,
//! runs recovery on the folder as MPFree's start does
//! ([`mpfree_stream::recover`]), and checks:
//!
//! - every take comes back with at least the frames it was promised, and
//!   every frame it comes back with is exactly what was recorded;
//! - the whole-take check passes on each finished take;
//! - every import promised is there and whole, any other is whole too (the
//!   one in flight at the cut), and part files a cut left are swept.
//!
//! On a pass it deletes the folder (its own names only); on a fail it
//! keeps it for a look, and says so.
//!
//! Only with `--desktop-test` for now: WP pulls a spare stick on a
//! computer. The cut on the MPC (D6) has its own safety review and go, so
//! the MPC's stick is refused here until then.
//!
//! `--cut-test --fat` (or `--fat-hidden`) is the same pull on a spare FAT32
//! or exFAT stick, for saving to the SD card (S2 in
//! docs/dev/hardware/sd-card-and-internal-drive.md): no takes, only whole
//! files published one after another, small and large, as S2 saves them.
//! [`Way::Rename`] publishes as MPFree does (a part file, synced, renamed,
//! the folder synced), whose rename FAT makes in two steps;
//! [`Way::Hidden`] writes each file under its own name, hidden, and shows
//! it once synced, one directory entry changed in place. The saved text
//! names the way, and `--after-cut` then checks the folder without writing
//! anything, as MPFree treats a card a cut left marked in use: every
//! promised import there, shown and whole, anything else shown whole too,
//! and an import found under two names (a rename cut between its steps,
//! two names on one file's clusters) said, for the computer's own `fsck -n`
//! to confirm.

use super::read::{Fail, KIB};
use super::{stopped, MIB, STOP};
use mpfree_stream::fsio::{Fs, StdFs};
use mpfree_stream::publish;
use mpfree_stream::rec::{Meta, RecFormat, RecId, TakeState, DATA_AT};
use mpfree_stream::recover::{self, Found};
use mpfree_stream::writer::{Gate, RealClock, Recorder, Status, TakeNo, WriterConfig};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The test's own folder beside the stick's others. The other runs never
/// clear it, and it outlives the cut.
pub(super) const FOLDER: &str = "mpfree-cuttest";
pub(super) const TAKES: u32 = 4;
const FORMAT: RecFormat = RecFormat {
    float: true,
    channels: 2,
    rate: 48_000,
};
/// The feed's period, as in D4.
const FEED_S: f64 = 0.01;
/// The promise line starts with this.
const PROMISED: &str = "promised: ";
/// Imports kept at once; the oldest goes before the next is published.
const KEEP_IMPORTS: usize = 4;
/// The saved text's line naming a FAT run's way.
const WAY: &str = "cut test way: ";

/// How a FAT or exFAT run publishes each import: the two ways S2 weighs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Way {
    /// As MPFree's publish: `NAME.<job>.part` written, synced, read back,
    /// renamed to `NAME`, the folder synced. FAT renames in two steps (the
    /// new name added, then the old removed), so a cut between them leaves
    /// two names on the same clusters.
    Rename,
    /// Made under its own name and hidden straight away, written, synced,
    /// then shown and synced again: the last step changes one directory
    /// entry in place.
    Hidden,
}

impl Way {
    fn word(self) -> &'static str {
        match self {
            Way::Rename => "fat-rename",
            Way::Hidden => "fat-hidden",
        }
    }

    fn from_word(w: &str) -> Option<Way> {
        [Way::Rename, Way::Hidden]
            .into_iter()
            .find(|v| v.word() == w)
    }

    /// The way the saved text names, if it is a FAT run's.
    pub(super) fn in_text(text: &str) -> Option<Way> {
        text.lines()
            .find_map(|l| Way::from_word(l.trim_end().strip_prefix(WAY)?))
    }
}

/// How big and how long, for a real run or a test's quick one.
#[derive(Clone, Copy, Debug)]
pub(super) struct Sizes {
    /// An import's size (every fourth import's, in a FAT run).
    import_bytes: u64,
    /// The other imports' size in a FAT run; `import_bytes` otherwise.
    small_bytes: u64,
    /// How long a FAT run writes before it stops by itself.
    longest_s: f64,
    /// The takes' room, all of them.
    takes_room: u64,
    /// When to say the stick can be pulled.
    cut_after_s: f64,
    /// How often the promise is said (when it changed).
    say_every_s: f64,
    /// The writer's interval.
    interval_s: f64,
    /// How long a said promise is given to reach the saved text before
    /// the import it no longer lists is removed.
    travel_s: f64,
}

impl Sizes {
    pub(super) fn new(quick: bool) -> Sizes {
        if quick {
            Sizes {
                import_bytes: 256 * KIB as u64,
                small_bytes: 256 * KIB as u64,
                longest_s: 0.0,
                takes_room: 8 * MIB,
                cut_after_s: 0.3,
                say_every_s: 0.05,
                interval_s: 0.1,
                travel_s: 0.02,
            }
        } else {
            Sizes {
                import_bytes: 16 * MIB,
                small_bytes: 16 * MIB,
                longest_s: 0.0,
                takes_room: 368 * MIB,
                cut_after_s: 30.0,
                say_every_s: 0.5,
                interval_s: WriterConfig::default().interval_s,
                travel_s: 2.0,
            }
        }
    }

    /// A FAT run's: files as S2 saves them, a project's or a kit's
    /// (256 KiB) and every fourth a sample's (4 MiB), for five minutes.
    pub(super) fn fat(quick: bool) -> Sizes {
        if quick {
            Sizes {
                import_bytes: 64 * KIB as u64,
                small_bytes: 16 * KIB as u64,
                longest_s: 1.5,
                takes_room: 0,
                cut_after_s: 0.3,
                say_every_s: 0.05,
                interval_s: 0.1,
                travel_s: 0.02,
            }
        } else {
            Sizes {
                import_bytes: 4 * MIB,
                small_bytes: 256 * KIB as u64,
                longest_s: 300.0,
                takes_room: 0,
                cut_after_s: 30.0,
                say_every_s: 0.5,
                interval_s: WriterConfig::default().interval_s,
                travel_s: 2.0,
            }
        }
    }

    /// Import `n`'s size.
    fn bytes_of(&self, n: u32) -> u64 {
        if n.is_multiple_of(4) {
            self.import_bytes
        } else {
            self.small_bytes
        }
    }

    /// The most the run has on the stick at once.
    pub(super) fn room(&self) -> u64 {
        self.takes_room + (KEEP_IMPORTS as u64 + 1) * self.import_bytes
    }

    fn take_frames(&self) -> u64 {
        self.takes_room / u64::from(TAKES) / FORMAT.block_align() as u64
    }
}

/// splitmix64's finaliser: neighbouring inputs give unrelated outputs.
fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The sample take `take` holds at `frame` on channel `ch`: a value only
/// that place has, a 24-bit whole number over 2^24, so a 32-bit float
/// holds it exactly.
pub(super) fn sample(take: u32, frame: u64, ch: u32) -> f32 {
    let x = mix((u64::from(take) << 56) ^ (frame << 1) ^ u64::from(ch));
    ((x >> 40) as i32 - (1 << 23)) as f32 / (1u32 << 24) as f32
}

/// The bytes of import `n` from `at` on.
fn import_fill(n: u32, at: u64, buf: &mut [u8]) {
    debug_assert!(at.is_multiple_of(8) && buf.len().is_multiple_of(8));
    for (k, c) in buf.chunks_exact_mut(8).enumerate() {
        let w = at / 8 + k as u64;
        c.copy_from_slice(&mix((u64::from(n) << 48) ^ w ^ 0x5eed).to_le_bytes());
    }
}

/// Whether the file at `path` is import `n` whole, `bytes` long: `None`
/// if so, else where it first differs (its length if it is short or long).
fn import_differs(path: &Path, n: u32, bytes: u64) -> Result<Option<u64>, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let (mut got, mut want) = (vec![0u8; 256 * KIB], vec![0u8; 256 * KIB]);
    let mut at = 0u64;
    while at < len.min(bytes) {
        let k = (len.min(bytes) - at).min(got.len() as u64) as usize;
        f.read_exact(&mut got[..k])
            .map_err(|e| format!("{}: {e}", path.display()))?;
        // A file a cut left can end anywhere: the fill goes in 8s.
        import_fill(n, at, &mut want[..k.next_multiple_of(8)]);
        if let Some(i) = (0..k).find(|&i| got[i] != want[i]) {
            return Ok(Some(at + i as u64));
        }
        at += k as u64;
    }
    Ok((len != bytes).then_some(len.min(bytes)))
}

fn import_name(n: u32) -> String {
    format!("import{n}.bin")
}

/// FAT's and exFAT's attribute calls (`include/uapi/linux/msdos_fs.h`;
/// both drivers take them in 6.18), and the hidden bit, the same in both.
const FAT_IOCTL_GET_ATTRIBUTES: u32 = 0x8004_7210;
const FAT_IOCTL_SET_ATTRIBUTES: u32 = 0x4004_7211;
const ATTR_HIDDEN: u32 = 0x02;
/// The stand-in for the hidden bit in a test's quick run, whose folder
/// isn't on FAT: the sticky bit, which a file's use ignores.
const STAND_IN: u32 = 0o1000;

/// Whether `f` is hidden. `stand_in` (a test's quick run) reads the
/// sticky bit instead.
fn is_hidden(f: &fs::File, stand_in: bool) -> io::Result<bool> {
    if stand_in {
        return Ok(f.metadata()?.permissions().mode() & STAND_IN != 0);
    }
    let mut attr = 0u32;
    // SAFETY: the call writes one u32 to the address given.
    if unsafe { libc::ioctl(f.as_raw_fd(), FAT_IOCTL_GET_ATTRIBUTES as _, &mut attr) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(attr & ATTR_HIDDEN != 0)
}

/// The kernel's error for an attribute call, said plainly when its driver
/// doesn't take the call at all.
fn attr_error() -> io::Error {
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::ENOTTY) {
        return io::Error::other(
            "this kernel's driver for the stick doesn't take FAT_IOCTL_SET_ATTRIBUTES, so --fat-hidden can't run on it",
        );
    }
    e
}

/// Hide or show `f`, its other attributes kept.
fn set_hidden(f: &fs::File, hidden: bool, stand_in: bool) -> io::Result<()> {
    if stand_in {
        let mode = f.metadata()?.permissions().mode() & 0o7777;
        let mode = if hidden {
            mode | STAND_IN
        } else {
            mode & !STAND_IN
        };
        return f.set_permissions(fs::Permissions::from_mode(mode));
    }
    let mut attr = 0u32;
    // SAFETY: as in is_hidden; then the call reads one u32.
    if unsafe { libc::ioctl(f.as_raw_fd(), FAT_IOCTL_GET_ATTRIBUTES as _, &mut attr) } < 0 {
        return Err(attr_error());
    }
    let attr = if hidden {
        attr | ATTR_HIDDEN
    } else {
        attr & !ATTR_HIDDEN
    };
    if unsafe { libc::ioctl(f.as_raw_fd(), FAT_IOCTL_SET_ATTRIBUTES as _, &attr) } < 0 {
        return Err(attr_error());
    }
    Ok(())
}

/// [`Way::Hidden`]'s publish: `dir/name` made new and hidden, written,
/// synced (data, size and entry), then shown and synced again, and the
/// folder synced as [`publish::publish`] does. A file this leaves hidden
/// was never published. On an error it removes what it made.
fn publish_hidden(
    dir: &Path,
    name: &str,
    write: &mut publish::Writer,
    stand_in: bool,
) -> io::Result<()> {
    let path = dir.join(name);
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    let r = (|| {
        set_hidden(&f, true, stand_in)?;
        write(&mut |b: &[u8]| f.write_all(b))?;
        f.sync_data()?;
        set_hidden(&f, false, stand_in)?;
        f.sync_all()?;
        fs::File::open(dir)?.sync_all()
    })();
    if r.is_err() {
        let _ = fs::remove_file(&path);
    }
    r
}

/// What was promised: each take's committed frames and the imports
/// published, in one line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Promise {
    pub(super) takes: BTreeMap<u32, u64>,
    pub(super) imports: BTreeSet<u32>,
}

impl Promise {
    pub(super) fn line(&self) -> String {
        let takes: Vec<String> = self.takes.iter().map(|(k, f)| format!("{k}={f}")).collect();
        let imports: Vec<String> = self.imports.iter().map(u32::to_string).collect();
        format!(
            "{PROMISED}takes {}; imports {}",
            if takes.is_empty() {
                "none".to_string()
            } else {
                takes.join(" ")
            },
            if imports.is_empty() {
                "none".to_string()
            } else {
                imports.join(" ")
            }
        )
    }

    pub(super) fn parse(line: &str) -> Option<Promise> {
        let rest = line.trim_end().strip_prefix(PROMISED)?;
        let (takes, imports) = rest.split_once("; ")?;
        let mut p = Promise::default();
        let takes = takes.strip_prefix("takes ")?;
        if takes != "none" {
            for t in takes.split(' ') {
                let (k, f) = t.split_once('=')?;
                p.takes.insert(k.parse().ok()?, f.parse().ok()?);
            }
        }
        let imports = imports.strip_prefix("imports ")?;
        if imports != "none" {
            for n in imports.split(' ') {
                p.imports.insert(n.parse().ok()?);
            }
        }
        Some(p)
    }

    /// The last promise in a run's saved text.
    pub(super) fn last_in(text: &str) -> Option<Promise> {
        text.lines().rev().find_map(Promise::parse)
    }
}

/// One of the test's own names: a take's files, an import, a part file.
pub(super) fn is_own(name: &str) -> bool {
    let take = name.strip_prefix("rec").and_then(|r| {
        let (k, rest) = r.split_once('_')?;
        let k: u32 = k.parse().ok()?;
        (1..=TAKES).contains(&k).then_some(rest)
    });
    if let Some(rest) = take {
        let id = rest.get(..32).unwrap_or("");
        let ext = rest.get(32..).unwrap_or("");
        return id.len() == 32
            && id.bytes().all(|b| b.is_ascii_hexdigit())
            && [
                ".wav",
                ".wav.recording",
                ".wav.checkpoint",
                ".tail.unverified.wav",
                ".unverified.wav",
                ".unverified.raw",
            ]
            .contains(&ext);
    }
    let import = name
        .strip_prefix("import")
        .and_then(|r| r.strip_suffix(".bin"))
        .or_else(|| {
            let r = name.strip_prefix("import")?.strip_suffix(".part")?;
            let (n, job) = r.split_once(".bin.")?;
            job.bytes().all(|b| b.is_ascii_digit()).then_some(n)
        });
    import.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn take_of(stem: &str) -> Option<u32> {
    stem.strip_prefix("rec")?.split_once('_')?.0.parse().ok()
}

/// What the two sides share: the imports kept, and the imports the last
/// promise said.
#[derive(Default)]
struct Shared {
    kept: Mutex<BTreeSet<u32>>,
    said: Mutex<BTreeSet<u32>>,
}

/// The import side: publish one import after another, the oldest removed
/// (and the folder synced) before each new one once [`KEEP_IMPORTS`] are
/// there. An import joins `kept` only once it is published. It leaves
/// `kept` first, and is removed only once a promise without it has been
/// said and has had time to reach the saved text, so no promise in that
/// text ever names an import this run removes.
fn imports(
    dir: &Path,
    z: Sizes,
    sh: &Shared,
    on: &AtomicBool,
    way: Option<Way>,
    stand_in: bool,
) -> Result<(), String> {
    let going = || on.load(Ordering::Relaxed) && !STOP.load(Ordering::Relaxed);
    let kept = &sh.kept;
    let mut fs = StdFs::new();
    let mut n = 0u32;
    while on.load(Ordering::Relaxed) && !STOP.load(Ordering::Relaxed) {
        n += 1;
        let oldest = {
            let k = kept.lock().unwrap();
            (k.len() >= KEEP_IMPORTS).then(|| *k.iter().next().unwrap())
        };
        if let Some(old) = oldest {
            kept.lock().unwrap().remove(&old);
            while going() && sh.said.lock().unwrap().contains(&old) {
                std::thread::sleep(Duration::from_millis(5));
            }
            let t = Instant::now();
            while going() && t.elapsed().as_secs_f64() < z.travel_s {
                std::thread::sleep(Duration::from_millis(5));
            }
            if !going() {
                break;
            }
            fs.remove(&dir.join(import_name(old)))
                .and_then(|()| fs.sync_dir(dir))
                .map_err(|e| format!("import {old} could not be removed: {e}"))?;
        }
        let mut buf = vec![0u8; 256 * KIB];
        let bytes = z.bytes_of(n);
        let mut write = |sink: &mut publish::Sink| -> io::Result<()> {
            let mut at = 0;
            while at < bytes {
                if !on.load(Ordering::Relaxed) || STOP.load(Ordering::Relaxed) {
                    return Err(std::io::Error::other("the run ended"));
                }
                let len = (bytes - at).min(buf.len() as u64) as usize;
                import_fill(n, at, &mut buf[..len]);
                sink(&buf[..len])?;
                at += len as u64;
            }
            Ok(())
        };
        let r = match way {
            Some(Way::Hidden) => publish_hidden(dir, &import_name(n), &mut write, stand_in),
            _ => publish::publish(&mut fs, dir, &import_name(n), n, &mut write).map(|_| ()),
        };
        match r {
            Ok(()) => {
                kept.lock().unwrap().insert(n);
            }
            // The run ending mid-import leaves no part file: publish removes it.
            Err(_) if !on.load(Ordering::Relaxed) || STOP.load(Ordering::Relaxed) => break,
            Err(e) => return Err(format!("import {n} could not be published: {e}")),
        }
    }
    Ok(())
}

/// `--cut-test` in `base`: record and import until the stick goes, the
/// takes reach their room, or Ctrl-C. With a `way`, a FAT run: imports
/// only, published that way, for [`Sizes::fat`]'s five minutes.
pub(super) fn run(base: &Path, z: Sizes, way: Option<Way>, stand_in: bool) -> Result<(), Fail> {
    let dir = base.join(FOLDER);
    if dir.exists() {
        return Err(Fail::Refused(format!(
            "{} is already there: run --after-cut with the last run's saved text first",
            dir.display()
        )));
    }
    fs::create_dir(&dir).map_err(|e| Fail::Refused(format!("create {}: {e}", dir.display())))?;
    match way {
        Some(way) => started_fat(&dir, z, way, stand_in),
        None => started(&dir, z),
    }
    .map_err(Fail::Failed)
}

/// A FAT run: the import thread publishes, this one says the promises.
fn started_fat(dir: &Path, z: Sizes, way: Way, stand_in: bool) -> Result<(), String> {
    say!("{WAY}{}", way.word());
    say!(
        "cut test on FAT: imports only, as saving to a card writes them, {}, one after another in {}: {} KiB each, every fourth {} KiB",
        match way {
            Way::Rename => "each a part file, synced, read back and renamed",
            Way::Hidden => "each written under its own name hidden, synced, then shown",
        },
        dir.display(),
        z.small_bytes / KIB as u64,
        z.import_bytes / KIB as u64
    );
    say!(
        "files: at most {:.1} MiB at once, the oldest import removed once {KEEP_IMPORTS} are there; --after-cut checks the rest without writing",
        z.room() as f64 / MIB as f64
    );
    say!(
        "keep this text off the stick: its last \"{}\" line is what --after-cut checks against",
        PROMISED.trim_end()
    );
    let sh = Shared::default();
    let on = AtomicBool::new(true);
    std::thread::scope(|s| {
        let import = std::thread::Builder::new()
            .name("disktest-import".into())
            .spawn_scoped(s, || {
                let r = imports(dir, z, &sh, &on, Some(way), stand_in);
                if r.is_err() {
                    STOP.store(true, Ordering::Relaxed);
                }
                r
            })
            .map_err(|e| format!("the import's thread could not start: {e}"))?;
        let r = promises(z, &sh);
        on.store(false, Ordering::Relaxed);
        let i = import
            .join()
            .unwrap_or_else(|_| Err("the import's thread failed".into()));
        // The import's own error (the stick gone) says more than the stop
        // it caused here.
        i.and(r)
    })?;
    let p = Promise {
        takes: BTreeMap::new(),
        imports: sh.kept.into_inner().unwrap(),
    };
    say!(
        "imports at the end: {}",
        p.line().split("; ").nth(1).unwrap_or("")
    );
    say!("no cut was made; run --after-cut with this text to check what is there (it writes nothing)");
    Ok(())
}

/// A FAT run's promises, said every so often when they change, until
/// [`Sizes::longest_s`] or a stop.
fn promises(z: Sizes, sh: &Shared) -> Result<(), String> {
    let start = Instant::now();
    let mut said: Option<Promise> = None;
    let mut said_at = f64::NEG_INFINITY;
    let mut told = false;
    loop {
        stopped()?;
        let now = start.elapsed().as_secs_f64();
        if now >= z.longest_s {
            return Ok(());
        }
        if !told && now >= z.cut_after_s {
            told = true;
            say!(
                "CUT NOW: pull the stick any time in the next {:.0} s, while imports are written",
                z.longest_s - now
            );
        }
        if now - said_at >= z.say_every_s {
            said_at = now;
            let p = Promise {
                takes: BTreeMap::new(),
                imports: sh.kept.lock().unwrap().clone(),
            };
            if said.as_ref() != Some(&p) {
                say!("{}", p.line());
                sh.said.lock().unwrap().clone_from(&p.imports);
                said = Some(p);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn started(dir: &Path, z: Sizes) -> Result<(), String> {
    say!(
        "cut test: {TAKES} takes (stereo float, 48 kHz, {:.2} MB/s in all) and {} MiB imports beside them, in {}",
        f64::from(FORMAT.rate) * FORMAT.block_align() as f64 * f64::from(TAKES) / 1e6,
        z.import_bytes as f64 / MIB as f64,
        dir.display()
    );
    say!(
        "files: at most {:.0} MiB at once; none is deleted by this run: --after-cut checks them and deletes them",
        z.room() as f64 / MIB as f64
    );
    say!(
        "keep this text off the stick: its last \"{}\" line is what --after-cut checks against",
        PROMISED.trim_end()
    );
    let cfg = WriterConfig {
        interval_s: z.interval_s,
        max_interval_s: WriterConfig::default().max_interval_s.max(z.interval_s),
        ..WriterConfig::default()
    };
    let mut rec = Recorder::start(
        Box::new(StdFs::new()),
        Arc::new(RealClock::new()),
        Arc::new(Gate::new()),
        cfg,
    )
    .map_err(|e| format!("the recorder could not start: {e}"))?;
    let sh = Shared::default();
    let on = AtomicBool::new(true);
    let r = std::thread::scope(|s| {
        let import = std::thread::Builder::new()
            .name("disktest-import".into())
            .spawn_scoped(s, || {
                let r = imports(dir, z, &sh, &on, None, false);
                if r.is_err() {
                    STOP.store(true, Ordering::Relaxed);
                }
                r
            })
            .map_err(|e| format!("the import's thread could not start: {e}"))?;
        let r = record(&mut rec, dir, z, &sh);
        on.store(false, Ordering::Relaxed);
        let i = import
            .join()
            .unwrap_or_else(|_| Err("the import's thread failed".into()));
        r.and(i)
    });
    rec.close();
    let p = Promise {
        takes: BTreeMap::new(),
        imports: sh.kept.into_inner().unwrap(),
    };
    say!(
        "imports at the end: {}",
        p.line().split("; ").nth(1).unwrap_or("")
    );
    say!("no cut was made; run --after-cut with this text to check what is there and delete it");
    r
}

/// The recording side, on the caller's thread.
fn record(rec: &mut Recorder, dir: &Path, z: Sizes, sh: &Shared) -> Result<(), String> {
    let kept = &sh.kept;
    let nos: Vec<TakeNo> = (1..=TAKES)
        .map(|k| {
            rec.arm(
                dir,
                &format!("rec{k}"),
                RecId::random(),
                FORMAT,
                Meta::default(),
            )
        })
        .collect();
    let t = Instant::now();
    while nos
        .iter()
        .any(|no| !matches!(rec.status(*no), Some(Status::Take(_))))
    {
        stopped()?;
        if let Some(Status::Failed(why)) = nos.iter().find_map(|no| match rec.status(*no) {
            f @ Some(Status::Failed(_)) => f,
            _ => None,
        }) {
            return Err(format!("a take could not be armed: {why}"));
        }
        if t.elapsed() > Duration::from_secs(30) {
            return Err("the takes' files were not made within 30 s".into());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let frames = (f64::from(FORMAT.rate) * FEED_S).round() as u64;
    let limit = z.take_frames();
    let mut buf = vec![0f32; frames as usize * 2];
    let start = Instant::now();
    let (mut at, mut n) = (0u64, 0u64);
    let mut said = Promise::default();
    let mut said_at = 0.0f64;
    let mut told = false;
    let mut end = Ok(());
    while at < limit {
        if let Err(e) = stopped() {
            end = Err(e);
            break;
        }
        let due = n as f64 * FEED_S;
        let now = start.elapsed().as_secs_f64();
        if now < due {
            std::thread::sleep(Duration::from_secs_f64(due - now));
        }
        let len = frames.min(limit - at);
        for (k, no) in nos.iter().enumerate() {
            let take = k as u32 + 1;
            for f in 0..len {
                for ch in 0..2u32 {
                    buf[(f * 2) as usize + ch as usize] = sample(take, at + f, ch);
                }
            }
            if rec.push(*no, &buf[..len as usize * 2]) < len as usize {
                end = Err(format!(
                    "take {take}'s capture memory filled: the stick fell more than its capture memory behind"
                ));
            }
        }
        at += len;
        n += 1;
        if end.is_err() {
            break;
        }
        if !told && now >= z.cut_after_s {
            told = true;
            say!(
                "CUT NOW: pull the stick any time in the next {:.0} s, while the takes record and an import is written",
                (limit - at) as f64 / f64::from(FORMAT.rate)
            );
        }
        if now - said_at >= z.say_every_s {
            said_at = now;
            let p = promise(rec, &nos, kept)?;
            if p != said {
                say!("{}", p.line());
                sh.said.lock().unwrap().clone_from(&p.imports);
                said = p;
            }
        }
    }
    for no in &nos {
        rec.stop(*no);
    }
    // Saved: finished, synced and named; a minute at the most.
    let t = Instant::now();
    loop {
        let states: Vec<Option<TakeState>> = nos
            .iter()
            .map(|no| match rec.status(*no) {
                Some(Status::Take(v)) => Some(v.state),
                _ => None,
            })
            .collect();
        if states
            .iter()
            .all(|s| matches!(s, Some(TakeState::Saved | TakeState::Stopped(_))))
            || t.elapsed() > Duration::from_secs(60)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let p = promise(rec, &nos, kept)?;
    if p != said {
        say!("{}", p.line());
    }
    let saved = nos
        .iter()
        .filter(
            |no| matches!(rec.status(**no), Some(Status::Take(v)) if v.state == TakeState::Saved),
        )
        .count();
    say!(
        "the takes stopped after {:.1} s ({}); {saved} of {TAKES} saved",
        at as f64 / f64::from(FORMAT.rate),
        if at >= limit {
            "their room"
        } else {
            "stopped early"
        }
    );
    for no in &nos {
        rec.forget(*no);
    }
    end
}

/// What the run is promised now.
fn promise(rec: &Recorder, nos: &[TakeNo], kept: &Mutex<BTreeSet<u32>>) -> Result<Promise, String> {
    let mut p = Promise {
        imports: kept.lock().unwrap().clone(),
        ..Promise::default()
    };
    for (k, no) in nos.iter().enumerate() {
        match rec.status(*no) {
            Some(Status::Take(v)) => {
                // Saved means every frame captured is on the stick.
                let f = if v.state == TakeState::Saved {
                    v.captured
                } else {
                    v.committed
                };
                p.takes.insert(k as u32 + 1, f);
            }
            Some(Status::Failed(why)) => return Err(format!("take {} failed: {why}", k + 1)),
            _ => {
                p.takes.insert(k as u32 + 1, 0);
            }
        }
    }
    Ok(p)
}

/// A take's audio file checked against what was recorded: its frames,
/// and how many from the start are exactly as recorded.
struct Back {
    frames: u64,
    good: u64,
}

fn check_audio(path: &Path, take: u32, first: u64) -> Result<Back, String> {
    let mut f = fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    let align = FORMAT.block_align() as u64;
    let frames = len.saturating_sub(DATA_AT) / align;
    let mut head = vec![0u8; DATA_AT as usize];
    f.read_exact(&mut head)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = vec![0u8; 512 * KIB];
    let (mut frame, mut good, mut bad) = (first, 0u64, false);
    let mut left = frames * align;
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        f.read_exact(&mut buf[..want])
            .map_err(|e| format!("{}: {e}", path.display()))?;
        for fr in buf[..want].chunks_exact(align as usize) {
            let ok = fr.chunks_exact(4).enumerate().all(|(ch, b)| {
                f32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_bits()
                    == sample(take, frame, ch as u32).to_bits()
            });
            if !ok {
                bad = true;
            }
            if !bad {
                good += 1;
            }
            frame += 1;
        }
        left -= want as u64;
    }
    Ok(Back { frames, good })
}

/// `--after-cut`: recover, check every promise, and delete the folder if
/// they all held. A FAT run's text (it names its way) is checked without
/// writing anything instead.
pub(super) fn after(base: &Path, promised: &str, quick: bool) -> Result<bool, Fail> {
    let dir = base.join(FOLDER);
    let way = Way::in_text(promised);
    let p = Promise::last_in(promised).ok_or_else(|| {
        Fail::Refused(format!(
            "no \"{}\" line in the saved text",
            PROMISED.trim_end()
        ))
    })?;
    if !dir.is_dir() {
        return Err(Fail::Refused(format!(
            "{} isn't there: nothing to check",
            dir.display()
        )));
    }
    let names: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| Fail::Refused(format!("{}: {e}", dir.display())))?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    // A FAT run's folder holds no takes.
    if let Some(other) = names
        .iter()
        .find(|n| !is_own(n) || (way.is_some() && n.starts_with("rec")))
    {
        return Err(Fail::Refused(format!(
            "{other} in {} isn't one of the test's: nothing is touched",
            dir.display()
        )));
    }
    if let Some(way) = way {
        return check_fat(&dir, &p, &names, Sizes::fat(quick), way, quick).map_err(Fail::Failed);
    }
    // A FAT stick a cut left is never written to: only a FAT run's text,
    // checked without writing, is taken there.
    let info = fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    if let Some(m) = mount_above(&info, &dir).filter(|m| is_fat(&m.kind)) {
        return Err(Fail::Refused(format!(
            "{} is on {} ({}), but the saved text is an ext4 run's, which sweeps and deletes: a FAT run's text names its way (\"{}\")",
            dir.display(),
            m.source,
            m.kind,
            WAY.trim_end()
        )));
    }
    check(&dir, &p, &names, Sizes::new(quick)).map_err(Fail::Failed)
}

/// Whether a file system type is FAT's or exFAT's.
fn is_fat(kind: &str) -> bool {
    matches!(kind, "vfat" | "msdos" | "exfat")
}

/// A FAT run's check, which writes nothing: every promised import there,
/// shown and whole; every shown import whole; hidden ones and part files
/// (in flight at the cut) said; an import under both its names said, and a
/// fail if it was promised (its rename was synced, so its old name must
/// not come back).
fn check_fat(
    dir: &Path,
    p: &Promise,
    names: &[String],
    z: Sizes,
    way: Way,
    stand_in: bool,
) -> Result<bool, String> {
    say!(
        "after the cut, in {} ({}): checked without writing anything",
        dir.display(),
        way.word()
    );
    say!("{}", p.line());
    let mut pass = true;
    let (mut shown, mut hidden) = (BTreeSet::new(), BTreeSet::new());
    let mut parts: Vec<(u32, String)> = Vec::new();
    for name in names {
        if let Some(n) = name
            .strip_prefix("import")
            .and_then(|r| r.strip_suffix(".bin"))
            .and_then(|n| n.parse::<u32>().ok())
        {
            let path = dir.join(name);
            let f = fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            if is_hidden(&f, stand_in).map_err(|e| format!("{name}'s attributes: {e}"))? {
                hidden.insert(n);
            } else {
                shown.insert(n);
            }
        } else if let Some(n) = name
            .strip_prefix("import")
            .and_then(|r| r.split_once(".bin."))
            .and_then(|(n, _)| n.parse::<u32>().ok())
        {
            parts.push((n, name.clone()));
        }
    }
    let mut whole = 0;
    for &n in &shown {
        match import_differs(&dir.join(import_name(n)), n, z.bytes_of(n))? {
            None => whole += 1,
            Some(at) => {
                pass = false;
                say!("  import {n}: FAIL: shown, but not as written from byte {at} on");
            }
        }
    }
    let state = |d: Option<u64>| match d {
        None => "whole".to_string(),
        Some(at) => format!("as written up to byte {at}"),
    };
    for &n in &hidden {
        let d = import_differs(&dir.join(import_name(n)), n, z.bytes_of(n))?;
        if p.imports.contains(&n) {
            pass = false;
            say!(
                "  import {n}: FAIL: promised, but hidden again ({})",
                state(d)
            );
        } else {
            say!(
                "  import {n}: hidden, in flight at the cut ({}): never shown, so not judged",
                state(d)
            );
        }
    }
    let mut twice = Vec::new();
    for (n, name) in &parts {
        let d = import_differs(&dir.join(name), *n, z.bytes_of(*n))?;
        if shown.contains(n) {
            twice.push(n.to_string());
            if p.imports.contains(n) {
                pass = false;
                say!(
                    "  import {n}: FAIL: promised, but its part file {name} ({}) is back beside it: the rename's second step was lost after the folder was synced",
                    state(d)
                );
            } else {
                say!(
                    "  import {n}: under two names, import{n}.bin and {name} ({}): the cut came between the rename's two steps, and both names may be on the same clusters (cross-linked); fsck -n says",
                    state(d)
                );
            }
        } else {
            say!(
                "  part file {name}: in flight at the cut ({}): never renamed, so not judged",
                state(d)
            );
        }
    }
    let missing: Vec<String> = p.imports.difference(&shown).map(u32::to_string).collect();
    let extra: Vec<String> = shown.difference(&p.imports).map(u32::to_string).collect();
    if !missing.is_empty() {
        pass = false;
    }
    say!(
        "  imports: {} promised, {}; {whole} of {} shown there whole{}; in flight: {} hidden, {} part files",
        p.imports.len(),
        if missing.is_empty() {
            "all there".to_string()
        } else {
            format!("FAIL: {} missing or hidden", missing.join(", "))
        },
        shown.len(),
        if extra.is_empty() {
            String::new()
        } else {
            format!(
                " (there but not promised: published, or about to be removed, when the stick went: {})",
                extra.join(", ")
            )
        },
        hidden.len(),
        parts.len()
    );
    say!(
        "  under two names: {}",
        if twice.is_empty() {
            "none".to_string()
        } else {
            twice.join(", ")
        }
    );
    if pass {
        say!("RESULT: PASS, every promise kept; nothing was written, so format the stick before its next run");
    } else {
        say!(
            "RESULT: FAIL, see above; nothing was written ({})",
            dir.display()
        );
    }
    Ok(pass)
}

fn check(dir: &Path, p: &Promise, names: &[String], z: Sizes) -> Result<bool, String> {
    say!("after the cut, in {}", dir.display());
    say!("{}", p.line());
    let mut fs_ = StdFs::new();
    let mut pass = true;
    let found = recover::recover(&mut fs_, dir).map_err(|e| format!("recovery failed: {e}"))?;
    let mut finished: BTreeMap<u32, PathBuf> = BTreeMap::new();
    // Audio recovery couldn't prove (nothing committed for it yet: a take
    // cut off as it started), kept apart by design. A promise for that take
    // fails below; without one it is said, not judged.
    let mut unproved: BTreeMap<u32, PathBuf> = BTreeMap::new();
    for f in &found {
        let k = take_of(f.stem());
        let what = match f {
            Found::Finished { record, .. } => {
                format!("its stop was cut off; finished at {} frames", record.frames)
            }
            Found::CutOff { record, .. } => {
                format!(
                    "cut off while recording; finished at {} frames",
                    record.frames
                )
            }
            Found::Unchecked { record, .. } => {
                format!(
                    "finished before, {} frames, its whole-take check still to run",
                    record.frames
                )
            }
            Found::Unverified { path, .. } => {
                if let Some(k) = take_of(f.stem()) {
                    unproved.insert(k, path.clone());
                }
                format!(
                    "its audio can't be proved (nothing committed for it); kept apart as {}",
                    file_name(path)
                )
            }
            Found::Removed { .. } => "no audio: its files went".to_string(),
            Found::Newer { .. } => {
                pass = false;
                "FAIL: named as a newer MPFree's".to_string()
            }
            Found::Failed { why, .. } => {
                pass = false;
                format!("FAIL: recovery failed: {why}")
            }
        };
        say!(
            "  recovery, take {}: {what}",
            k.map_or("?".to_string(), |k| k.to_string())
        );
        if let Some((path, r)) = f.to_check() {
            let ok = recover::check_whole(&mut fs_, path, r)
                .map_err(|e| format!("{}: {e}", path.display()))?;
            if ok {
                recover::checked(&mut fs_, dir, f.stem())
                    .map_err(|e| format!("{}: {e}", f.stem()))?;
            } else {
                pass = false;
            }
            say!(
                "  take {}: the whole-take check {}",
                k.map_or("?".to_string(), |k| k.to_string()),
                if ok { "passes" } else { "FAILS" }
            );
            if let Some(k) = k {
                finished.insert(k, path.to_path_buf());
            }
        }
    }
    // Takes saved before the cut, whose checkpoints had already gone.
    for n in names {
        if let Some(stem) = n.strip_suffix(".wav") {
            if let Some(k) = take_of(stem) {
                if !stem.ends_with(".unverified") && !stem.ends_with(".tail.unverified") {
                    finished.entry(k).or_insert_with(|| dir.join(n));
                }
            }
        }
    }
    for (&k, &want) in &p.takes {
        let Some(path) = finished.get(&k) else {
            let kept = unproved.get(&k).map(|u| match check_audio(u, k, 0) {
                Ok(b) => format!(
                    "only audio that can't be proved came back ({} frames, {} of them as recorded; kept apart)",
                    b.frames, b.good
                ),
                Err(e) => format!("only audio that can't be proved came back ({e}; kept apart)"),
            });
            let what = kept.unwrap_or_else(|| "nothing came back".to_string());
            if want > 0 {
                pass = false;
                say!("  take {k}: FAIL: {what}; {want} frames were promised");
            } else {
                say!("  take {k}: {what}, and nothing was promised");
            }
            continue;
        };
        let b = check_audio(path, k, 0)?;
        let exact = b.good == b.frames;
        let kept_promise = b.frames >= want;
        pass &= exact && kept_promise;
        say!(
            "  take {k}: {} frames back ({:.2} s), {}; promised {want}: {}",
            b.frames,
            b.frames as f64 / f64::from(FORMAT.rate),
            if exact {
                "every sample as recorded".to_string()
            } else {
                format!("FAIL: as recorded only up to frame {}", b.good)
            },
            if kept_promise {
                format!("kept ({} more)", b.frames - want)
            } else {
                format!("FAIL: {} short", want - b.frames)
            }
        );
        // Audio past the record, kept apart: said, not judged.
        let stem = file_name(path);
        let stem = stem.strip_suffix(".wav").unwrap_or(&stem).to_string();
        let tail = dir.join(format!("{stem}.tail.unverified.wav"));
        if tail.exists() {
            match check_audio(&tail, k, b.frames) {
                Ok(t) => say!(
                    "  take {k}: {} frames past its record kept apart ({}), {} of them as recorded",
                    t.frames,
                    file_name(&tail),
                    t.good
                ),
                Err(e) => say!("  take {k}: a tail kept apart ({e})"),
            }
        }
    }
    let swept =
        publish::sweep_parts(&mut fs_, dir).map_err(|e| format!("sweeping part files: {e}"))?;
    let mut there: BTreeSet<u32> = BTreeSet::new();
    for n in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let n = n.file_name().to_string_lossy().into_owned();
        if let Some(num) = n
            .strip_prefix("import")
            .and_then(|r| r.strip_suffix(".bin"))
        {
            if let Ok(num) = num.parse() {
                there.insert(num);
            }
        }
    }
    let mut whole = 0;
    for &n in &there {
        match import_differs(&dir.join(import_name(n)), n, z.bytes_of(n))? {
            None => whole += 1,
            Some(at) => {
                pass = false;
                say!("  import {n}: FAIL: not as written from byte {at} on");
            }
        }
    }
    let missing: Vec<String> = p.imports.difference(&there).map(u32::to_string).collect();
    let extra: Vec<String> = there.difference(&p.imports).map(u32::to_string).collect();
    if !missing.is_empty() {
        pass = false;
    }
    say!(
        "  imports: {} promised, {}; {whole} of {} there whole{}; part files swept: {swept}",
        p.imports.len(),
        if missing.is_empty() {
            "all there".to_string()
        } else {
            format!("FAIL: {} missing", missing.join(", "))
        },
        there.len(),
        if extra.is_empty() {
            String::new()
        } else {
            format!(
                " (there but not promised: in flight, or about to be removed, when the run ended: {})",
                extra.join(", ")
            )
        }
    );
    if pass {
        let left: Vec<String> = fs::read_dir(dir)
            .map_err(|e| e.to_string())?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        for n in &left {
            if !is_own(n) {
                return Err(format!("{n} appeared in the folder: left alone"));
            }
            fs::remove_file(dir.join(n)).map_err(|e| format!("remove {n}: {e}"))?;
        }
        fs::remove_dir(dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
        say!("RESULT: PASS, every promise kept; the folder is deleted");
    } else {
        say!(
            "RESULT: FAIL, see above; the folder is kept for a look ({})",
            dir.display()
        );
    }
    Ok(pass)
}

/// Mountinfo's escapes (`\040` for a space) undone.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && i + 3 < b.len()
            && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c))
        {
            out.push((b[i + 1] - b'0') * 64 + (b[i + 2] - b'0') * 8 + (b[i + 3] - b'0'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A mount, from mountinfo.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Mount {
    pub(super) point: PathBuf,
    pub(super) source: String,
    pub(super) kind: String,
    /// The mount's own options, then the file system's.
    pub(super) options: String,
}

/// The mount `path` is on (the longest mount point above it), from
/// mountinfo's text.
pub(super) fn mount_above(info: &str, path: &Path) -> Option<Mount> {
    let mut best: Option<Mount> = None;
    for line in info.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let Some(dash) = f.iter().position(|&x| x == "-") else {
            continue;
        };
        if f.len() <= dash + 2 || f.len() < 6 {
            continue;
        }
        let point = PathBuf::from(unescape(f[4]));
        if path.starts_with(&point)
            && best
                .as_ref()
                .is_none_or(|b| point.as_os_str().len() >= b.point.as_os_str().len())
        {
            best = Some(Mount {
                point,
                source: unescape(f[dash + 2]),
                kind: f[dash + 1].to_string(),
                options: format!("{}; {}", f[5], f.get(dash + 3).copied().unwrap_or("")),
            });
        }
    }
    best
}

/// WP is for a spare stick: `base` must be on an ext4 file system (as the
/// MPC's partition 4 is), or with `fat` a FAT or exFAT one (as SD cards
/// are), on a USB disk. Says which, and its options, so the right stick is
/// pulled and the result can be read.
pub(super) fn on_usb_disk(base: &Path, fat: bool) -> Result<(), String> {
    let info = fs::read_to_string("/proc/self/mountinfo").map_err(|e| format!("mountinfo: {e}"))?;
    let Mount {
        point,
        source,
        kind,
        options,
    } = mount_above(&info, base).ok_or_else(|| format!("no mount above {}", base.display()))?;
    if fat && !is_fat(&kind) {
        return Err(format!(
            "{} is on {source} ({kind}): the FAT pull test wants vfat or exfat, as SD cards are",
            base.display()
        ));
    }
    if !fat && kind != "ext4" {
        return Err(format!(
            "{} is on {source} ({kind}): the pull test wants ext4, as the MPC's partition 4 is",
            base.display()
        ));
    }
    let name = source
        .strip_prefix("/dev/")
        .filter(|n| !n.contains('/'))
        .ok_or_else(|| format!("{} is on {source}, not a disk under /dev", base.display()))?;
    let sys = fs::canonicalize(format!("/sys/class/block/{name}"))
        .map_err(|e| format!("/sys/class/block/{name}: {e}"))?;
    if !sys.to_string_lossy().contains("/usb") {
        return Err(format!(
            "{} is on {source}, which isn't a USB disk: the pull test is for a spare stick",
            base.display()
        ));
    }
    say!(
        "the folder goes on {source} ({kind}, mounted at {}), a USB disk: that is the stick to pull",
        point.display()
    );
    say!("its mount options: {options}");
    Ok(())
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_place_has_its_own_sample_and_a_float_holds_it_exactly() {
        let mut seen = BTreeSet::new();
        for take in 1..=TAKES {
            for frame in 0..2000 {
                for ch in 0..2 {
                    let s = sample(take, frame, ch);
                    assert!((-0.5..0.5).contains(&s));
                    // A whole number over 2^24: exact.
                    let x = s * (1u32 << 24) as f32;
                    assert_eq!(x, x.round());
                    seen.insert(s.to_bits());
                }
            }
        }
        // Nearly all distinct (24 bits each, 16,000 samples).
        assert!(seen.len() > 15_990, "{}", seen.len());
        assert_ne!(sample(1, 0, 0), sample(2, 0, 0));
        assert_ne!(sample(1, 0, 0), sample(1, 0, 1));
        assert_ne!(sample(1, 0, 0), sample(1, 1, 0));
    }

    #[test]
    fn an_import_is_the_same_however_it_is_filled_and_a_change_is_found() {
        let bytes = 600 * KIB as u64;
        let mut whole = vec![0u8; bytes as usize];
        import_fill(3, 0, &mut whole);
        let mut part = vec![0u8; 8 * KIB];
        import_fill(3, 64 * KIB as u64, &mut part);
        assert_eq!(&part[..], &whole[64 * KIB..72 * KIB]);
        let mut other = vec![0u8; 8 * KIB];
        import_fill(4, 64 * KIB as u64, &mut other);
        assert_ne!(part, other);
        let d = std::env::temp_dir().join(format!("mpfree-cut-unit-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        let p = d.join("import3.bin");
        fs::write(&p, &whole).unwrap();
        assert_eq!(import_differs(&p, 3, bytes).unwrap(), None);
        assert_eq!(import_differs(&p, 4, bytes).unwrap(), Some(0));
        assert_eq!(import_differs(&p, 3, bytes + 8).unwrap(), Some(bytes));
        whole[300_001] ^= 1;
        fs::write(&p, &whole).unwrap();
        assert_eq!(import_differs(&p, 3, bytes).unwrap(), Some(300_001));
        fs::write(&p, &whole[..1000]).unwrap();
        assert_eq!(import_differs(&p, 3, bytes).unwrap(), Some(1000));
        // A length a cut left, not in 8s: as written up to its end.
        whole[300_001] ^= 1;
        fs::write(&p, &whole[..1003]).unwrap();
        assert_eq!(import_differs(&p, 3, bytes).unwrap(), Some(1003));
        whole[1001] ^= 1;
        fs::write(&p, &whole[..1003]).unwrap();
        assert_eq!(import_differs(&p, 3, bytes).unwrap(), Some(1001));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_promise_reads_back_as_said_and_the_last_one_counts() {
        let p = Promise {
            takes: [(1, 96_000), (2, 95_520), (3, 0), (4, 10)].into(),
            imports: [5, 6, 7].into(),
        };
        assert_eq!(
            p.line(),
            "promised: takes 1=96000 2=95520 3=0 4=10; imports 5 6 7"
        );
        assert_eq!(Promise::parse(&p.line()), Some(p.clone()));
        let none = Promise {
            takes: [(1, 0)].into(),
            imports: BTreeSet::new(),
        };
        assert_eq!(none.line(), "promised: takes 1=0; imports none");
        assert_eq!(Promise::parse(&none.line()), Some(none.clone()));
        let text = format!(
            "{}\nCUT NOW: …\n{}\r\nthe takes stopped\n",
            none.line(),
            p.line()
        );
        assert_eq!(Promise::last_in(&text), Some(p));
        assert_eq!(Promise::last_in("nothing here\n"), None);
        assert_eq!(Promise::parse("promised: takes 1=x; imports none"), None);
    }

    #[test]
    fn only_its_own_names_are_its_own() {
        let id = "0123456789abcdef0123456789abcdef";
        for ext in [
            "wav",
            "wav.recording",
            "wav.checkpoint",
            "tail.unverified.wav",
            "unverified.wav",
            "unverified.raw",
        ] {
            assert!(is_own(&format!("rec1_{id}.{ext}")), "{ext}");
            assert!(is_own(&format!("rec4_{id}.{ext}")), "{ext}");
        }
        for ok in ["import1.bin", "import12.bin", "import3.bin.3.part"] {
            assert!(is_own(ok), "{ok}");
        }
        for bad in [
            format!("rec5_{id}.wav"),
            format!("rec0_{id}.wav"),
            format!("rec1_{}.wav", &id[..31]),
            format!("rec1_{id}.wav.part"),
            "import.bin".to_string(),
            "importx.bin".to_string(),
            "import1.bin.x.part".to_string(),
            "notes.txt".to_string(),
            "Project.mpfp".to_string(),
        ] {
            assert!(!is_own(&bad), "{bad}");
        }
        assert_eq!(take_of(&format!("rec3_{id}")), Some(3));
    }

    #[test]
    fn the_mount_is_the_longest_point_above_the_folder() {
        let info = "22 1 8:2 / / rw,relatime shared:1 - ext4 /dev/sda2 rw\n\
            30 22 0:26 / /media rw shared:2 - tmpfs tmpfs rw\n\
            41 30 8:33 / /media/jesse/MPFREE\\040DATA rw,nosuid shared:3 - ext4 /dev/sdc4 rw\n";
        assert_eq!(
            mount_above(info, Path::new("/media/jesse/MPFREE DATA/x")),
            Some(Mount {
                point: PathBuf::from("/media/jesse/MPFREE DATA"),
                source: "/dev/sdc4".to_string(),
                kind: "ext4".to_string(),
                options: "rw,nosuid; rw".to_string(),
            })
        );
        assert_eq!(
            mount_above(info, Path::new("/home/jesse")).map(|m| m.source),
            Some("/dev/sda2".to_string())
        );
        assert_eq!(
            mount_above(info, Path::new("/media/other")).map(|m| m.kind),
            Some("tmpfs".to_string())
        );
        let fat = "22 1 8:2 / / rw shared:1 - ext4 /dev/sda2 rw\n\
            50 22 8:49 / /run/media/jesse/WP-FAT rw,nosuid,nodev,relatime shared:9 - vfat /dev/sdd1 rw,fmask=0022,dmask=0022,shortname=mixed,utf8,flush,errors=remount-ro\n";
        let m = mount_above(fat, Path::new("/run/media/jesse/WP-FAT/wp")).unwrap();
        assert!(is_fat(&m.kind) && !is_fat("ext4") && is_fat("exfat"));
        assert_eq!(
            m.options,
            "rw,nosuid,nodev,relatime; rw,fmask=0022,dmask=0022,shortname=mixed,utf8,flush,errors=remount-ro"
        );
        assert_eq!(mount_above("", Path::new("/")), None);
    }

    #[test]
    fn a_fat_runs_promise_has_no_takes_and_its_text_names_its_way() {
        let p = Promise {
            takes: BTreeMap::new(),
            imports: [7, 8].into(),
        };
        assert_eq!(p.line(), "promised: takes none; imports 7 8");
        assert_eq!(Promise::parse(&p.line()), Some(p.clone()));
        assert_eq!(
            Promise::parse("promised: takes none; imports none"),
            Some(Promise::default())
        );
        for way in [Way::Rename, Way::Hidden] {
            let text = format!("mpfree-disktest: …\n{WAY}{}\r\n{}\n", way.word(), p.line());
            assert_eq!(Way::in_text(&text), Some(way));
        }
        assert_eq!(
            Way::in_text("cut test: 4 takes\npromised: takes 1=0; imports none\n"),
            None
        );
        assert_eq!(Way::in_text("cut test way: fat-other\n"), None);
    }

    #[test]
    fn a_fat_run_writes_small_files_and_every_fourth_a_large_one() {
        let z = Sizes::fat(false);
        assert_eq!(z.bytes_of(1), 256 * KIB as u64);
        assert_eq!(z.bytes_of(3), 256 * KIB as u64);
        assert_eq!(z.bytes_of(4), 4 * MIB);
        assert_eq!(z.room(), 5 * 4 * MIB);
        assert!(z.cut_after_s < z.longest_s / 5.0);
        // An ext4 run's imports are all one size, as before.
        let e = Sizes::new(false);
        assert_eq!((e.bytes_of(1), e.bytes_of(4)), (16 * MIB, 16 * MIB));
    }

    #[test]
    fn a_hidden_publish_ends_shown_and_whole_and_an_ended_one_leaves_nothing() {
        let d = std::env::temp_dir().join(format!("mpfree-cut-hidden-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let bytes = 40 * KIB as u64;
        let mut seen_hidden = false;
        let path = d.join(import_name(9));
        let mut write = |sink: &mut publish::Sink| -> io::Result<()> {
            let mut buf = vec![0u8; 8 * KIB];
            let mut at = 0;
            while at < bytes {
                // Hidden while it is written.
                seen_hidden |= is_hidden(&fs::File::open(&path)?, true)?;
                import_fill(9, at, &mut buf);
                sink(&buf)?;
                at += buf.len() as u64;
            }
            Ok(())
        };
        publish_hidden(&d, &import_name(9), &mut write, true).unwrap();
        assert!(seen_hidden);
        let f = fs::File::open(&path).unwrap();
        assert!(!is_hidden(&f, true).unwrap());
        assert_eq!(import_differs(&path, 9, bytes).unwrap(), None);
        // A second publish under the same name is refused, the first kept.
        assert!(publish_hidden(&d, &import_name(9), &mut |_| Ok(()), true).is_err());
        assert_eq!(import_differs(&path, 9, bytes).unwrap(), None);
        // One ended mid-write leaves no file.
        let r = publish_hidden(
            &d,
            &import_name(10),
            &mut |sink| {
                sink(&[0u8; 8])?;
                Err(io::Error::other("the run ended"))
            },
            true,
        );
        assert!(r.is_err());
        assert!(!d.join(import_name(10)).exists());
        // The real call, here (not FAT): refused plainly, nothing left.
        let e = publish_hidden(&d, &import_name(11), &mut |_| Ok(()), false).unwrap_err();
        assert!(
            e.to_string()
                .contains("doesn't take FAT_IOCTL_SET_ATTRIBUTES"),
            "{e}"
        );
        assert!(!d.join(import_name(11)).exists());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_real_run_fits_the_write_runs_room() {
        let z = Sizes::new(false);
        assert!(z.room() <= 448 * MIB, "{}", z.room() / MIB);
        // About four minutes of takes: time to pull the stick after the
        // first half minute.
        let secs = z.take_frames() as f64 / f64::from(FORMAT.rate);
        assert!((200.0..300.0).contains(&secs), "{secs}");
        assert!(z.cut_after_s < secs / 2.0);
    }
}
