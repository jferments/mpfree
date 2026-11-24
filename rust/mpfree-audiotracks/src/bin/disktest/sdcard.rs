//! `mpfree-disktest --sd-card`: the SD card in the MPC's slot, read and
//! written, with the stick beside it (step S2a of
//! docs/dev/hardware/sd-card-and-internal-drive.md).
//!
//! On the MPC it runs as root on the stick's system, in three parts:
//!
//! 1. **The read-only window** (step S1, `setup-sd-card.sh`), asked for as
//!    the pad player asks for it, its counter moved while the card is read.
//!    It reads the files already on the card, never printing a name: one
//!    sequential read of the largest, then 8 paced streams across them. It
//!    must close saying nothing was written to the card.
//! 2. **The write window** (step S2a, `setup-sd-card-write.sh`), which
//!    refuses a card that is not exFAT, is marked "not safely removed" or
//!    doesn't check clean. With `--repair-card` (the SD card round passes
//!    it, on the owner's go) it asks the window to repair such a card in
//!    the MPC first (`fsck.exfat -y`, then checked again; the window
//!    refuses if that doesn't leave it clean). Then the steps below, twice: idle, then with a
//!    busy thread on every core (stopping at 75 °C). It writes only in its
//!    own folders, `MPFree/mpfree-disktest` on the card and
//!    `mpfree-disktest` on the stick's partition 4, deletes both at the
//!    end (also after an error, Ctrl-C or a lost ssh session), and closes
//!    the window, which unmounts the card and checks it again (the mark
//!    clear, `fsck.exfat -n` clean).
//! 3. A table of every step's numbers, idle and busy.
//!
//! The steps, in 128 KiB blocks (the audio tracks' chunk):
//!
//!  1. SD: sequential write, then fsync.
//!  2. SD: sequential read, the file's pages dropped from the cache first.
//!  3. USB: sequential write, then fsync.
//!  4. USB: sequential read.
//!  5. SD: 8 streams playing, 3.1 MB/s in all (8 stereo tracks).
//!  6. SD: recording at 3.1 MB/s, fdatasync every second.
//!  7. SD: 4 streams recording and 4 playing at once, half each.
//!  8. Copy from USB to SD as fast as it goes, then fsync.
//!  9. Copy from SD to USB.
//! 10. SD and USB written at once.
//! 11. SD and USB read at once.
//! 12. 8 streams playing from USB while recording to SD.
//! 13. 8 streams playing from SD while recording to USB.
//! 14. Saving files to SD as MPFree will: each written as `NAME.part`,
//!     fsync, renamed, the folder synced.
//!
//! `--desktop-test DIR` plays it on a stand-in: the windows' files under
//! `DIR/run/mpfree-app` and `DIR/run/mpfree-setup` (the test answers them
//! as the stick's services would), the card at `DIR/run/mpfree-sd`, the
//! stick at `DIR/run/mpfree-data`; no mount is checked. `--quick` cuts
//! every step to 2 s and a few MiB.

use super::{
    drop_cache, free_bytes, load, mbps, mount_of, pattern, read, stopped, Pace, Times, BLOCK,
    FOLDER, MIB, RATE, STOP, STREAMS,
};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The largest sequential file on each side; at most a quarter of the
/// free space.
const SEQ_MAX: u64 = 128 * MIB;
const SEQ_MIN: u64 = 16 * MIB;
/// The read-only window's sequential read of the card's own files.
const OWN_READ_MAX: u64 = 256 * MIB;
const OWN_MIN_FILE: u64 = 64 * 1024;
const OWN_MAX_FILES: usize = 20_000;
const OWN_MAX_DEPTH: usize = 16;
/// Saving: this many files of this size.
const SAVES: usize = 40;
const SAVE_BYTES: usize = MIB as usize;
/// How long each window may take to answer: the read-only one opens in
/// seconds; the write one runs fsck.exfat -n over the whole card first,
/// and again at its close.
const READ_OPEN: Duration = Duration::from_secs(90);
const READ_CLOSE: Duration = Duration::from_secs(120);
const WRITE_OPEN: Duration = Duration::from_secs(1800);
const WRITE_CLOSE: Duration = Duration::from_secs(1800);

/// The files the run makes, and so the only ones it deletes (with the
/// saves, `save-NNN.bin` and `save-NNN.bin.part`).
const OWN: [&str; 7] = [
    "seq.bin", "copy.bin", "rec.bin", "mix0.bin", "mix1.bin", "mix2.bin", "mix3.bin",
];

/// A summary's rows: the step and what it gave.
type Rows = Vec<(String, String)>;

fn own(name: &str) -> bool {
    if OWN.contains(&name) {
        return true;
    }
    let Some(rest) = name.strip_prefix("save-") else {
        return false;
    };
    let digits = rest
        .strip_suffix(".bin.part")
        .or_else(|| rest.strip_suffix(".bin"));
    digits.is_some_and(|d| d.len() == 3 && d.bytes().all(|b| b.is_ascii_digit()))
}

/// Where everything is: the stick's places, or a desktop stand-in's.
pub(super) struct Places {
    app: PathBuf,
    setup: PathBuf,
    card: PathBuf,
    write: PathBuf,
    stick: PathBuf,
    thermal: PathBuf,
    desktop: bool,
    quick: bool,
    /// With no card in the slot, how long to keep asking (every
    /// [`CARD_AGAIN`]).
    card_wait: Duration,
    /// Ask the write window to repair a marked or unclean card first.
    repair: bool,
}

const CARD_AGAIN: Duration = Duration::from_secs(10);

impl Places {
    pub(super) fn mpc() -> Places {
        Places::under(Path::new("/"), PathBuf::from(load::THERMAL), false, false)
    }

    pub(super) fn desktop(root: &Path, thermal: PathBuf, quick: bool) -> Places {
        Places::under(root, thermal, true, quick)
    }

    fn under(root: &Path, thermal: PathBuf, desktop: bool, quick: bool) -> Places {
        Places {
            app: root.join("run/mpfree-app"),
            setup: root.join("run/mpfree-setup"),
            card: root.join("run/mpfree-sd"),
            write: root.join("run/mpfree-sd/MPFree"),
            stick: root.join("run/mpfree-data"),
            thermal,
            desktop,
            quick,
            card_wait: Duration::ZERO,
            repair: false,
        }
    }

    pub(super) fn card_wait(self, wait: Duration) -> Places {
        Places {
            card_wait: wait,
            ..self
        }
    }

    pub(super) fn repair(self, repair: bool) -> Places {
        Places { repair, ..self }
    }
}

/// One window's files: what the pad player writes and what the service
/// answers.
struct Proto {
    what: &'static str,
    request: &'static str,
    close: &'static str,
    alive: &'static str,
    status: &'static str,
}

const READ_WINDOW: Proto = Proto {
    what: "read-only window",
    request: "sd-card.request",
    close: "sd-card.close",
    alive: "sd-card.alive",
    status: "sd-card-status",
};

const WRITE_WINDOW: Proto = Proto {
    what: "write window",
    request: "sd-card-write.request",
    close: "sd-card-write.close",
    alive: "sd-card-write.alive",
    status: "sd-card-write-status",
};

fn read_status(p: &Places, proto: &Proto) -> BTreeMap<String, String> {
    fs::read_to_string(p.setup.join(proto.status))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// Write `text` to `path` whole: a new file beside it, renamed over it.
fn put(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.part"));
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    f.write_all(text.as_bytes())?;
    drop(f);
    fs::rename(&tmp, path)
}

/// 32 hex digits for a window's request.
fn new_word() -> String {
    let mut b = [0u8; 16];
    if File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut b))
        .is_err()
    {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
            ^ (std::process::id() as u128) << 64;
        b = t.to_le_bytes();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// An open window: its word, and the thread moving its counter.
struct Window<'a> {
    p: &'a Places,
    proto: &'static Proto,
    word: String,
    on: std::sync::Arc<AtomicBool>,
    beat: Option<std::thread::JoinHandle<()>>,
    closed: bool,
}

/// Why a window didn't open: its state (none, refused, failed, closed,
/// or "no answer") and its words.
struct NotOpen {
    state: String,
    message: String,
}

impl<'a> Window<'a> {
    /// Ask for the window and wait for its answer to this request. `more`:
    /// the request's lines after the word (the write window's `repair`).
    fn ask(
        p: &'a Places,
        proto: &'static Proto,
        wait: Duration,
        more: &str,
    ) -> Result<(Window<'a>, BTreeMap<String, String>), NotOpen> {
        let word = new_word();
        let on = std::sync::Arc::new(AtomicBool::new(true));
        let alive = p.app.join(proto.alive);
        let flag = on.clone();
        let beat = std::thread::spawn(move || {
            let mut n = 0u64;
            while flag.load(Ordering::Relaxed) {
                n += 1;
                let _ = put(&alive, &format!("{n:020}\n"));
                for _ in 0..10 {
                    if !flag.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
        let mut w = Window {
            p,
            proto,
            word,
            on,
            beat: Some(beat),
            closed: false,
        };
        if let Err(e) = put(&p.app.join(proto.request), &format!("{}\n{more}", w.word)) {
            w.closed = true;
            w.stop_beat();
            return Err(NotOpen {
                state: "not asked".into(),
                message: format!("{}: {e}", p.app.join(proto.request).display()),
            });
        }
        say!(
            "asked for the {} ({}{})",
            proto.what,
            w.word,
            if more.is_empty() {
                ""
            } else {
                ", and a repair if the card needs one"
            }
        );
        let start = Instant::now();
        while start.elapsed() < wait {
            if STOP.load(Ordering::Relaxed) {
                break;
            }
            let st = read_status(p, proto);
            if st.get("ask") == Some(&w.word) {
                let state = st.get("state").cloned().unwrap_or_default();
                match state.as_str() {
                    "open" => return Ok((w, st)),
                    "none" | "refused" | "failed" | "closed" => {
                        w.closed = true;
                        w.stop_beat();
                        return Err(NotOpen {
                            state,
                            message: st.get("message").cloned().unwrap_or_default(),
                        });
                    }
                    _ => {}
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(NotOpen {
            state: "no answer".into(),
            message: if STOP.load(Ordering::Relaxed) {
                "stopped (Ctrl-C, SIGTERM or the session closed)".into()
            } else {
                format!(
                    "the {} did not open within {} s",
                    proto.what,
                    wait.as_secs()
                )
            },
        })
        // Dropping w asks for the close, should it open after all.
    }

    fn stop_beat(&mut self) {
        self.on.store(false, Ordering::Relaxed);
        if let Some(t) = self.beat.take() {
            let _ = t.join();
        }
    }

    /// Ask for the close and wait for its last word: (state, message,
    /// the whole status).
    fn close(mut self, wait: Duration) -> Result<BTreeMap<String, String>, String> {
        self.closed = true;
        put(&self.p.app.join(self.proto.close), "")
            .map_err(|e| format!("could not ask for the close: {e}"))?;
        let start = Instant::now();
        let r = loop {
            let st = read_status(self.p, self.proto);
            if st.get("ask") == Some(&self.word)
                && matches!(
                    st.get("state").map(String::as_str),
                    Some("closed" | "failed" | "refused" | "none")
                )
            {
                break Ok(st);
            }
            if start.elapsed() >= wait {
                break Err(format!(
                    "the {} did not close within {} s",
                    self.proto.what,
                    wait.as_secs()
                ));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        self.stop_beat();
        r
    }
}

impl Drop for Window<'_> {
    fn drop(&mut self) {
        if !self.closed {
            let _ = put(&self.p.app.join(self.proto.close), "");
        }
        self.stop_beat();
    }
}

/// On the MPC: the card's mounts are what the window said, from the SD
/// controller, and no eMMC device is there.
fn check_mounts(p: &Places, writable: bool) -> Result<String, String> {
    let card = p.card.to_string_lossy().into_owned();
    let (src, fstype, opts) = mount_of(&card)?;
    if !opts.split(',').any(|o| o == "ro") {
        return Err(format!("{card} is mounted {opts}, not read-only"));
    }
    let name = src
        .strip_prefix("/dev/mmcblk")
        .map(|_| src.trim_start_matches("/dev/").to_string())
        .ok_or_else(|| format!("{card} comes from {src}, not the SD card"))?;
    let disk = name.split('p').next().unwrap_or(&name).to_string();
    let sys = fs::canonicalize(format!("/sys/block/{disk}"))
        .map_err(|e| format!("/sys/block/{disk}: {e}"))?;
    if !sys.to_string_lossy().contains("ff0c0000") {
        return Err(format!(
            "{src} is not under the SD card controller ({})",
            sys.display()
        ));
    }
    if let Ok(rd) = fs::read_dir("/sys/block") {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("mmcblk") && n != disk {
                return Err(format!("another MMC device is there: {n}"));
            }
        }
    }
    let mut said = format!("{card}: {src}, {fstype}, read-only, under the SD card controller");
    if writable {
        let w = p.write.to_string_lossy().into_owned();
        let (wsrc, wtype, wopts) = mount_of(&w)?;
        if wsrc != src || wtype != fstype || !wopts.split(',').any(|o| o == "rw") {
            return Err(format!(
                "{w} is {wsrc} {wtype} {wopts}, not the card's MPFree folder writable"
            ));
        }
        said.push_str(&format!("; {w}: writable"));
    }
    Ok(said)
}

/// A regular file of the card's own, for the read-only window's reads.
struct OwnFile {
    path: PathBuf,
    len: u64,
}

/// The card's folders, files and bytes, and its files of 64 KiB or more.
/// Never follows a link; never prints a name.
fn walk(root: &Path) -> (u64, u64, u64, Vec<OwnFile>) {
    let (mut dirs, mut files, mut bytes) = (0u64, 0u64, 0u64);
    let mut found = Vec::new();
    let mut todo = vec![(root.to_path_buf(), 0usize)];
    while let Some((d, depth)) = todo.pop() {
        let Ok(rd) = fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(m) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if m.is_dir() {
                dirs += 1;
                if depth + 1 < OWN_MAX_DEPTH {
                    todo.push((e.path(), depth + 1));
                }
            } else if m.is_file() {
                files += 1;
                bytes += m.len();
                if m.len() >= OWN_MIN_FILE && found.len() < OWN_MAX_FILES {
                    found.push(OwnFile {
                        path: e.path(),
                        len: m.len(),
                    });
                }
            }
        }
    }
    (dirs, files, bytes, found)
}

fn open_ro(p: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(p)
        .map_err(|e| format!("open one of the card's files: {e}"))
}

/// Part 1: the read-only window, reading the card's own files.
fn read_part(p: &Places, rows: &mut Rows) -> Result<(), read::Fail> {
    say!("\n== Part 1: the SD card read-only (step S1's window) ==");
    let start = Instant::now();
    let asked = loop {
        match Window::ask(p, &READ_WINDOW, READ_OPEN, "") {
            Err(n) if n.state == "none" && start.elapsed() + CARD_AGAIN <= p.card_wait => {
                say!(
                    "no card in the SD slot yet: put it in now (asking again in {} s, for up to {} s more)",
                    CARD_AGAIN.as_secs(),
                    p.card_wait.saturating_sub(start.elapsed()).as_secs()
                );
                let until = Instant::now() + CARD_AGAIN;
                while Instant::now() < until {
                    stopped().map_err(read::Fail::Refused)?;
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            r => break r,
        }
    };
    let (w, st) = match asked {
        Ok(x) => x,
        Err(n) if n.state == "none" => {
            return Err(read::Fail::Refused(format!(
                "there is no card in the SD slot ({})",
                n.message
            )))
        }
        Err(n) => {
            return Err(read::Fail::Failed(format!(
                "the read-only window didn't open: {}: {}",
                n.state, n.message
            )))
        }
    };
    say!(
        "open: {} ({}, {} sectors, {}); not safely removed last time: {}",
        st.get("message").map(String::as_str).unwrap_or(""),
        st.get("fs").map(String::as_str).unwrap_or("?"),
        st.get("sectors").map(String::as_str).unwrap_or("?"),
        st.get("card").map(String::as_str).unwrap_or("?"),
        st.get("dirty").map(String::as_str).unwrap_or("?")
    );
    if !p.desktop {
        match check_mounts(p, false) {
            Ok(s) => say!("mounts: {s}"),
            Err(e) => return Err(read::Fail::Failed(format!("the read-only window: {e}"))),
        }
    }
    let r = read_own(p, rows);
    let closed = w.close(READ_CLOSE);
    r?;
    let st = closed.map_err(read::Fail::Failed)?;
    let msg = st.get("message").cloned().unwrap_or_default();
    say!("closed: {msg}");
    if st.get("state").map(String::as_str) != Some("closed") || !msg.contains("Nothing was written")
    {
        return Err(read::Fail::Failed(format!(
            "the read-only window did not close cleanly: {msg}"
        )));
    }
    Ok(())
}

fn read_own(p: &Places, rows: &mut Rows) -> Result<(), read::Fail> {
    let t = Instant::now();
    let (dirs, files, bytes, mut found) = walk(&p.card);
    say!(
        "the card holds {dirs} folders and {files} files, {:.1} MiB (listed in {:.1} s); {} files of 64 KiB or more",
        bytes as f64 / MIB as f64,
        t.elapsed().as_secs_f64(),
        found.len()
    );
    rows.push((
        "card listed (folders, files)".into(),
        format!("{dirs}, {files} in {:.1} s", t.elapsed().as_secs_f64()),
    ));
    if found.is_empty() {
        say!("  no file of 64 KiB or more on the card: its reads are left out");
        return Ok(());
    }
    found.sort_by_key(|f| std::cmp::Reverse(f.len));
    let limit = if p.quick { 4 * MIB } else { OWN_READ_MAX };
    let mut chosen = Vec::new();
    let mut total = 0;
    for f in found {
        if total >= limit {
            break;
        }
        total += f.len.min(limit - total);
        chosen.push(f);
    }
    say!(
        "\nS1. Sequential read of the card's own files: {} files, {:.0} MiB, each file's pages dropped first",
        chosen.len(),
        total as f64 / MIB as f64
    );
    let mut buf = vec![0u8; BLOCK];
    let mut r = Times::default();
    let t = Instant::now();
    let mut left = total;
    for f in &chosen {
        let mut file = open_ro(&f.path).map_err(read::Fail::Failed)?;
        drop_cache(&file);
        let mut this = f.len.min(left);
        while this > 0 {
            stopped().map_err(read::Fail::Failed)?;
            let n = (this as usize).min(BLOCK);
            r.time(|| file.read_exact(&mut buf[..n]))
                .map_err(|e| read::Fail::Failed(format!("reading one of the card's files: {e}")))?;
            this -= n as u64;
            left -= n as u64;
        }
    }
    let rate = mbps(total, t.elapsed());
    say!("  {rate:.1} MB/s");
    say!("{}", r.report("read"));
    rows.push((
        "S1. card's own files, sequential".into(),
        format!("{rate:.1} MB/s"),
    ));
    let secs = if p.quick { 2.0 } else { 30.0 };
    say!(
        "\nS2. {STREAMS} streams read the card's own files at {:.2} MB/s in all for {secs:.0} s",
        RATE / 1e6
    );
    let mut opened = Vec::new();
    for f in chosen.iter().take(STREAMS) {
        let file = open_ro(&f.path).map_err(read::Fail::Failed)?;
        drop_cache(&file);
        opened.push((file, f.len));
    }
    let (r, behind) = streams(&mut opened, secs, RATE).map_err(read::Fail::Failed)?;
    say!("{}", r.report("read"));
    say!("  furthest behind schedule: {behind:.1} ms (the read-ahead playback would need)");
    rows.push((
        "S2. 8 streams on the card's own files".into(),
        format!("{behind:.0} ms behind at most"),
    ));
    Ok(())
}

/// Paced streams over `files` (each at its own place, from the start),
/// `rate` bytes a second in all, for `secs`: the read times and how far
/// behind schedule they fell.
fn streams(files: &mut [(File, u64)], secs: f64, rate: f64) -> Result<(Times, f64), String> {
    let mut buf = vec![0u8; BLOCK];
    let n = files.len().max(1);
    let mut pos = vec![0u64; files.len()];
    let mut r = Times::default();
    let mut p = Pace::new(rate);
    while p.next(secs) {
        stopped()?;
        let k = (p.n as usize - 1) % n;
        let Some((f, len)) = files.get_mut(k) else {
            break;
        };
        let blocks = (*len / BLOCK as u64).max(1);
        let b = pos[k] % blocks;
        pos[k] += 1;
        let want = (*len - b * BLOCK as u64).min(BLOCK as u64) as usize;
        r.time(|| {
            f.seek(SeekFrom::Start(b * BLOCK as u64))?;
            f.read_exact(&mut buf[..want])
        })?;
    }
    Ok((r, p.behind_ms))
}

fn create(dir: &Path, name: &str) -> Result<File, String> {
    debug_assert!(own(name));
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(name))
        .map_err(|e| format!("create {}: {e}", dir.join(name).display()))
}

fn open(dir: &Path, name: &str) -> Result<File, String> {
    open_ro(&dir.join(name)).map_err(|e| format!("{}: {e}", dir.join(name).display()))
}

fn remove(dir: &Path, name: &str) -> Result<(), String> {
    debug_assert!(own(name));
    match fs::remove_file(dir.join(name)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove {}: {e}", dir.join(name).display())),
    }
}

/// Delete the run's folder: only its own regular files, then the folder.
/// Anything else in it stops it (left as found).
fn clean(dir: &Path) -> Result<(), String> {
    let meta = match fs::symlink_metadata(dir) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("{}: {e}", dir.display())),
    };
    if !meta.is_dir() {
        return Err(format!("{} is not a folder: left alone", dir.display()));
    }
    let mut mine = Vec::new();
    for e in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let m = fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
        if !own(&n) || !m.file_type().is_file() {
            return Err(format!(
                "{} holds {n:?}, which is not the test's: left alone",
                dir.display()
            ));
        }
        mine.push(n);
    }
    for n in mine {
        remove(dir, &n)?;
    }
    fs::remove_dir(dir).map_err(|e| format!("remove {}: {e}", dir.display()))
}

/// A sequential write of `bytes`, then fsync: MB/s with the fsync, and
/// the writes' and the fsync's times.
fn seq_write(dir: &Path, name: &str, bytes: u64) -> Result<(f64, Times, Times), String> {
    let mut f = create(dir, name)?;
    let mut buf = vec![0u8; BLOCK];
    let mut w = Times::default();
    let t = Instant::now();
    for n in 0..bytes / BLOCK as u64 {
        stopped()?;
        pattern(&mut buf, n);
        w.time(|| f.write_all(&buf))?;
    }
    let mut s = Times::default();
    s.time(|| f.sync_all())?;
    drop_cache(&f);
    Ok((mbps(bytes, t.elapsed()), w, s))
}

fn seq_read(dir: &Path, name: &str, bytes: u64) -> Result<(f64, Times), String> {
    let mut f = open(dir, name)?;
    drop_cache(&f);
    let mut buf = vec![0u8; BLOCK];
    let mut r = Times::default();
    let t = Instant::now();
    for _ in 0..bytes / BLOCK as u64 {
        stopped()?;
        r.time(|| f.read_exact(&mut buf))?;
    }
    drop_cache(&f);
    Ok((mbps(bytes, t.elapsed()), r))
}

/// Recording: `rate` bytes a second into a new file of at most `size`
/// (going on from its start when full), fdatasync every second: the
/// writes' and syncs' times and how far behind schedule it fell.
fn record(
    dir: &Path,
    name: &str,
    size: u64,
    secs: f64,
    rate: f64,
) -> Result<(Times, Times, f64), String> {
    let mut f = create(dir, name)?;
    let mut buf = vec![0u8; BLOCK];
    let (mut w, mut s) = (Times::default(), Times::default());
    let mut p = Pace::new(rate);
    let (mut at, mut last) = (0u64, Instant::now());
    while p.next(secs) {
        stopped()?;
        if at + BLOCK as u64 > size {
            f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            at = 0;
        }
        pattern(&mut buf, p.n);
        w.time(|| f.write_all(&buf))?;
        at += BLOCK as u64;
        if last.elapsed() >= Duration::from_secs(1) {
            s.time(|| f.sync_data())?;
            last = Instant::now();
        }
    }
    s.time(|| f.sync_data())?;
    drop(f);
    remove(dir, name)?;
    Ok((w, s, p.behind_ms))
}

/// 8 streams playing the sequential file in `dir`, each at its own part.
fn play(dir: &Path, bytes: u64, secs: f64, rate: f64, n: usize) -> Result<(Times, f64), String> {
    let part = (bytes / n as u64 / BLOCK as u64).max(1) * BLOCK as u64;
    let mut files = Vec::new();
    for k in 0..n {
        let mut f = open(dir, "seq.bin")?;
        drop_cache(&f);
        f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        files.push((f, part, k as u64 * part));
    }
    let mut buf = vec![0u8; BLOCK];
    let mut pos = vec![0u64; n];
    let mut r = Times::default();
    let mut p = Pace::new(rate);
    while p.next(secs) {
        stopped()?;
        let k = (p.n as usize - 1) % n;
        let (f, len, start) = &mut files[k];
        let b = pos[k] % (*len / BLOCK as u64).max(1);
        pos[k] += 1;
        r.time(|| {
            f.seek(SeekFrom::Start(*start + b * BLOCK as u64))?;
            f.read_exact(&mut buf)
        })?;
    }
    Ok((r, p.behind_ms))
}

fn copy(from: &Path, to: &Path, bytes: u64) -> Result<(f64, Times, Times), String> {
    let mut src = open(from, "seq.bin")?;
    drop_cache(&src);
    let mut dst = create(to, "copy.bin")?;
    let mut buf = vec![0u8; BLOCK];
    let (mut r, mut w) = (Times::default(), Times::default());
    let t = Instant::now();
    for _ in 0..bytes / BLOCK as u64 {
        stopped()?;
        r.time(|| src.read_exact(&mut buf))?;
        w.time(|| dst.write_all(&buf))?;
    }
    w.time(|| dst.sync_all())?;
    let rate = mbps(bytes, t.elapsed());
    drop(dst);
    remove(to, "copy.bin")?;
    Ok((rate, r, w))
}

/// Saving as MPFree will: each file written as NAME.part, fsync, renamed
/// to NAME, the folder synced. Files a second and each file's time.
fn save(dir: &Path, n: usize, size: usize) -> Result<(f64, Times), String> {
    let mut buf = vec![0u8; size];
    let mut each = Times::default();
    let folder = File::open(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let t = Instant::now();
    for i in 0..n {
        stopped()?;
        let name = format!("save-{i:03}.bin");
        let part = format!("{name}.part");
        pattern(&mut buf, i as u64);
        each.time(|| {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(dir.join(&part))?;
            f.write_all(&buf)?;
            f.sync_all()?;
            drop(f);
            fs::rename(dir.join(&part), dir.join(&name))?;
            folder.sync_all()
        })?;
    }
    let rate = n as f64 / t.elapsed().as_secs_f64().max(1e-9);
    for i in 0..n {
        remove(dir, &format!("save-{i:03}.bin"))?;
    }
    Ok((rate, each))
}

struct Sizes {
    sd: u64,
    usb: u64,
    secs: f64,
    saves: usize,
    save_bytes: usize,
}

fn round_block(b: u64) -> u64 {
    (b / BLOCK as u64).max(STREAMS as u64) * BLOCK as u64
}

/// The 14 steps once, with the rows for the summary.
fn steps(sd: &Path, usb: &Path, z: &Sizes) -> Result<Rows, String> {
    let mut rows = Vec::new();
    let row = |rows: &mut Rows, step: &str, value: String| {
        rows.push((step.to_string(), value));
    };
    let both_rate = RATE;

    say!("\n1. SD: sequential write, {} MiB, then fsync", z.sd / MIB);
    let (rate, w, s) = seq_write(sd, "seq.bin", z.sd)?;
    say!(
        "  {rate:.1} MB/s including the fsync\n{}\n{}",
        w.report("write"),
        s.report("fsync")
    );
    row(&mut rows, "1. SD write", format!("{rate:.1} MB/s"));

    say!(
        "\n2. SD: sequential read, {} MiB, cache dropped first",
        z.sd / MIB
    );
    let (rate, r) = seq_read(sd, "seq.bin", z.sd)?;
    say!("  {rate:.1} MB/s\n{}", r.report("read"));
    row(&mut rows, "2. SD read", format!("{rate:.1} MB/s"));

    say!(
        "\n3. USB: sequential write, {} MiB, then fsync",
        z.usb / MIB
    );
    let (rate, w, s) = seq_write(usb, "seq.bin", z.usb)?;
    say!(
        "  {rate:.1} MB/s including the fsync\n{}\n{}",
        w.report("write"),
        s.report("fsync")
    );
    row(&mut rows, "3. USB write", format!("{rate:.1} MB/s"));

    say!(
        "\n4. USB: sequential read, {} MiB, cache dropped first",
        z.usb / MIB
    );
    let (rate, r) = seq_read(usb, "seq.bin", z.usb)?;
    say!("  {rate:.1} MB/s\n{}", r.report("read"));
    row(&mut rows, "4. USB read", format!("{rate:.1} MB/s"));

    say!(
        "\n5. SD: {STREAMS} streams playing, {:.2} MB/s in all, {:.0} s",
        both_rate / 1e6,
        z.secs
    );
    let (r, behind) = play(sd, z.sd, z.secs, both_rate, STREAMS)?;
    say!(
        "{}\n  furthest behind schedule: {behind:.1} ms",
        r.report("read")
    );
    row(
        &mut rows,
        "5. SD playing 8 streams",
        format!("{behind:.0} ms behind"),
    );

    say!(
        "\n6. SD: recording at {:.2} MB/s, fdatasync every second, {:.0} s",
        both_rate / 1e6,
        z.secs
    );
    let (w, s, behind) = record(sd, "rec.bin", z.sd, z.secs, both_rate)?;
    say!(
        "{}\n{}\n  furthest behind schedule: {behind:.1} ms",
        w.report("write"),
        s.report("fdatasync")
    );
    row(
        &mut rows,
        "6. SD recording",
        format!("{behind:.0} ms behind"),
    );

    say!(
        "\n7. SD: 4 streams recording and 4 playing, {:.2} MB/s each half, {:.0} s",
        both_rate / 2e6,
        z.secs
    );
    let (wr, rr) = std::thread::scope(|s| {
        let rec = s.spawn(|| -> Result<(Times, Times, f64), String> {
            let mut out = (Times::default(), Times::default(), 0f64);
            let per = both_rate / 2.0 / 4.0;
            let results: Vec<_> = (0..4)
                .map(|k| {
                    let name = ["mix0.bin", "mix1.bin", "mix2.bin", "mix3.bin"][k];
                    s.spawn(move || record(sd, name, z.sd / 4, z.secs, per))
                })
                .collect();
            for h in results {
                let (w, sy, b) = h
                    .join()
                    .map_err(|_| "a recording thread failed".to_string())??;
                out.0.merge(w);
                out.1.merge(sy);
                out.2 = out.2.max(b);
            }
            Ok(out)
        });
        let r = play(sd, z.sd, z.secs, both_rate / 2.0, 4);
        let w = rec.join().map_err(|_| "the recording failed".to_string());
        (w, r)
    });
    let (w, s, wb) = wr??;
    let (r, rb) = rr?;
    say!(
        "{}\n{}\n{}\n  furthest behind schedule: recording {wb:.1} ms, playing {rb:.1} ms",
        w.report("write"),
        s.report("fdatasync"),
        r.report("read")
    );
    row(
        &mut rows,
        "7. SD 4 recording + 4 playing",
        format!("{:.0} / {:.0} ms behind", wb, rb),
    );

    say!(
        "\n8. Copy from USB to SD, {} MiB, then fsync",
        z.usb.min(z.sd) / MIB
    );
    let (rate, r, w) = copy(usb, sd, z.usb.min(z.sd))?;
    say!(
        "  {rate:.1} MB/s\n{}\n{}",
        r.report("USB read"),
        w.report("SD write")
    );
    row(&mut rows, "8. copy USB to SD", format!("{rate:.1} MB/s"));

    say!(
        "\n9. Copy from SD to USB, {} MiB, then fsync",
        z.usb.min(z.sd) / MIB
    );
    let (rate, r, w) = copy(sd, usb, z.usb.min(z.sd))?;
    say!(
        "  {rate:.1} MB/s\n{}\n{}",
        r.report("SD read"),
        w.report("USB write")
    );
    row(&mut rows, "9. copy SD to USB", format!("{rate:.1} MB/s"));

    say!(
        "\n10. SD and USB written at once, {} and {} MiB",
        z.sd / MIB,
        z.usb / MIB
    );
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| {
            let r = seq_write(sd, "copy.bin", z.sd);
            let _ = remove(sd, "copy.bin");
            r
        });
        let b = s.spawn(|| {
            let r = seq_write(usb, "copy.bin", z.usb);
            let _ = remove(usb, "copy.bin");
            r
        });
        (a.join(), b.join())
    });
    let (sa, wa, _) = a.map_err(|_| "the SD writer failed".to_string())??;
    let (sb, wb, _) = b.map_err(|_| "the USB writer failed".to_string())??;
    say!(
        "  SD {sa:.1} MB/s, USB {sb:.1} MB/s\n{}\n{}",
        wa.report("SD write"),
        wb.report("USB write")
    );
    row(
        &mut rows,
        "10. SD + USB writes at once",
        format!("{sa:.1} + {sb:.1} MB/s"),
    );

    say!("\n11. SD and USB read at once");
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| seq_read(sd, "seq.bin", z.sd));
        let b = s.spawn(|| seq_read(usb, "seq.bin", z.usb));
        (a.join(), b.join())
    });
    let (sa, ra) = a.map_err(|_| "the SD reader failed".to_string())??;
    let (sb, rb) = b.map_err(|_| "the USB reader failed".to_string())??;
    say!(
        "  SD {sa:.1} MB/s, USB {sb:.1} MB/s\n{}\n{}",
        ra.report("SD read"),
        rb.report("USB read")
    );
    row(
        &mut rows,
        "11. SD + USB reads at once",
        format!("{sa:.1} + {sb:.1} MB/s"),
    );

    for (num, from, to, pl, rl, bytes, size) in [
        (12, usb, sd, "USB", "SD", z.usb, z.sd),
        (13, sd, usb, "SD", "USB", z.sd, z.usb),
    ] {
        say!(
            "\n{num}. {STREAMS} streams playing from {pl} while recording to {rl}, {:.2} MB/s each, {:.0} s",
            both_rate / 1e6,
            z.secs
        );
        let (pr, rr) = std::thread::scope(|s| {
            let rec = s.spawn(|| record(to, "rec.bin", size, z.secs, both_rate));
            let p = play(from, bytes, z.secs, both_rate, STREAMS);
            (p, rec.join())
        });
        let (r, pb) = pr?;
        let (w, sy, wb) = rr.map_err(|_| "the recording failed".to_string())??;
        say!(
            "{}\n{}\n{}\n  furthest behind schedule: playing {pb:.1} ms, recording {wb:.1} ms",
            r.report(&format!("{pl} read")),
            w.report(&format!("{rl} write")),
            sy.report("fdatasync")
        );
        row(
            &mut rows,
            &format!("{num}. play {pl} + record {rl}"),
            format!("{pb:.0} / {wb:.0} ms behind"),
        );
    }

    say!(
        "\n14. Saving {} files of {} KiB to SD: NAME.part, fsync, rename, the folder synced",
        z.saves,
        z.save_bytes / 1024
    );
    let (rate, each) = save(sd, z.saves, z.save_bytes)?;
    say!("  {rate:.1} files a second\n{}", each.report("each file"));
    row(
        &mut rows,
        "14. saving files to SD",
        format!("{rate:.1} files/s"),
    );

    remove(sd, "seq.bin")?;
    remove(usb, "seq.bin")?;
    Ok(rows)
}

/// The steps again with a busy thread on every core, stopping at the
/// temperature limit.
fn busy_steps(
    sd: &Path,
    usb: &Path,
    z: &Sizes,
    zones: &[load::Zone],
) -> Result<(Rows, bool), String> {
    let limit = load::stop_at(zones);
    let on = AtomicBool::new(true);
    let hot = AtomicBool::new(false);
    let cpus = load::cpus_online();
    say!(
        "\n-- The same steps with a busy thread on each of the {cpus} cores (they stop at {limit:.0} °C) --"
    );
    std::thread::scope(|s| {
        let watcher = s.spawn(|| {
            let mut top = f64::NAN;
            while on.load(Ordering::Relaxed) && !STOP.load(Ordering::Relaxed) {
                if let Some(c) = load::hottest(zones) {
                    top = if top.is_nan() { c } else { top.max(c) };
                    if c >= limit {
                        hot.store(true, Ordering::Relaxed);
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            top
        });
        let hogs: Vec<_> = (0..cpus)
            .map(|c| {
                let (on, hot) = (&on, &hot);
                s.spawn(move || load::hog(c, on, hot))
            })
            .collect();
        let r = steps(sd, usb, z);
        on.store(false, Ordering::Relaxed);
        for h in hogs {
            let _ = h.join();
        }
        let top = watcher.join().unwrap_or(f64::NAN);
        let was_hot = hot.load(Ordering::Relaxed);
        if top.is_nan() {
            say!("  hottest: no reading");
        } else {
            say!("  hottest during these steps: {top:.1} °C");
        }
        if was_hot {
            say!("  the busy threads stopped at {limit:.0} °C, so later steps ran with idle cores");
        }
        r.map(|rows| (rows, was_hot))
    })
}

fn print_summary(
    own: &[(String, String)],
    idle: &[(String, String)],
    busy: Option<&[(String, String)]>,
    closed: &str,
) {
    say!("\nSUMMARY (MB/s, or how far behind schedule a paced step fell at most)");
    for (k, v) in own {
        say!("  {k:<38} {v}");
    }
    say!("  {:<38} {closed}", "write window closed");
    say!("  {:<38} {:<22} {}", "step", "idle", "busy cores");
    for (i, (k, v)) in idle.iter().enumerate() {
        let b = busy
            .and_then(|b| b.get(i))
            .map(|x| x.1.as_str())
            .unwrap_or("(left out)");
        say!("  {k:<38} {v:<22} {b}");
    }
}

/// Part 2: the write window and the steps.
fn write_part(p: &Places, own_rows: &[(String, String)]) -> Result<(), read::Fail> {
    say!("\n== Part 2: the SD card written in its MPFree folder (step S2a's window) ==");
    let more = if p.repair { "repair\n" } else { "" };
    let (w, st) = match Window::ask(p, &WRITE_WINDOW, WRITE_OPEN, more) {
        Ok(x) => x,
        Err(n) if n.state == "refused" || n.state == "none" => {
            return Err(read::Fail::Refused(format!(
                "the card can't be written: {}{}",
                n.message,
                if p.repair || !n.message.contains("ask for a repair") {
                    ""
                } else {
                    " (--repair-card asks for it)"
                }
            )))
        }
        Err(n) => {
            return Err(read::Fail::Failed(format!(
                "the write window didn't open: {}: {}",
                n.state, n.message
            )))
        }
    };
    if let Some(what) = st.get("repaired") {
        say!("repaired in the MPC first (fsck.exfat -y, then checked again: clean, the mark clear); it had: {what}");
    }
    say!(
        "open: {} ({}, {} sectors; {})",
        st.get("message").map(String::as_str).unwrap_or(""),
        st.get("fs").map(String::as_str).unwrap_or("?"),
        st.get("sectors").map(String::as_str).unwrap_or("?"),
        st.get("fsck").map(String::as_str).unwrap_or("fsck.exfat")
    );
    let r = write_steps(p);
    say!("\nclosing the write window (it unmounts the card and checks it again)");
    let closed = w.close(WRITE_CLOSE);
    let (r2, said) = match closed {
        Ok(st) => {
            let msg = st.get("message").cloned().unwrap_or_default();
            say!("closed: {msg}");
            if st.get("state").map(String::as_str) == Some("closed") && msg.contains("checks clean")
            {
                (Ok(()), msg)
            } else {
                (
                    Err(read::Fail::Failed(format!("the write window: {msg}"))),
                    msg,
                )
            }
        }
        Err(e) => (Err(read::Fail::Failed(e.clone())), e),
    };
    let (idle, busy) = r?;
    print_summary(own_rows, &idle, busy.as_deref(), &said);
    r2
}

/// The steps idle and, with temperatures to watch, with busy cores: their
/// rows for the summary.
fn write_steps(p: &Places) -> Result<(Rows, Option<Rows>), read::Fail> {
    if !p.desktop {
        match check_mounts(p, true) {
            Ok(s) => say!("mounts: {s}"),
            Err(e) => return Err(read::Fail::Failed(format!("the write window: {e}"))),
        }
    }
    let sd = p.write.join(FOLDER);
    let usb = p.stick.join(FOLDER);
    for d in [&sd, &usb] {
        clean(d).map_err(read::Fail::Refused)?;
    }
    let free = |d: &Path| free_bytes(d).map_err(read::Fail::Refused);
    let (free_sd, free_usb) = (free(&p.write)?, free(&p.stick)?);
    say!(
        "free before: SD {:.1} MiB, USB {:.1} MiB",
        free_sd as f64 / MIB as f64,
        free_usb as f64 / MIB as f64
    );
    let z = if p.quick {
        Sizes {
            sd: 4 * MIB,
            usb: 4 * MIB,
            secs: 2.0,
            saves: 4,
            save_bytes: 64 * 1024,
        }
    } else {
        Sizes {
            sd: round_block((free_sd / 4).min(SEQ_MAX)),
            usb: round_block((free_usb / 4).min(SEQ_MAX)),
            secs: 30.0,
            saves: SAVES,
            save_bytes: SAVE_BYTES,
        }
    };
    if !p.quick && (z.sd < SEQ_MIN || z.usb < SEQ_MIN) {
        return Err(read::Fail::Refused(format!(
            "too little room: it needs {} MiB free on the card and on the stick",
            4 * SEQ_MIN / MIB
        )));
    }
    let zones = load::zones(&p.thermal);
    let t = Instant::now();
    let r = fs::create_dir(&sd)
        .and_then(|()| fs::create_dir(&usb))
        .map_err(|e| format!("create the test folders: {e}"))
        .and_then(|()| {
            say!("\n-- Idle --");
            let idle = steps(&sd, &usb, &z)?;
            let busy = if zones.is_empty() {
                say!("\nno temperature to read: the busy-core steps are left out (the cores are never kept busy unwatched)");
                None
            } else {
                Some(busy_steps(&sd, &usb, &z, &zones)?.0)
            };
            Ok((idle, busy))
        });
    let cleaned = clean(&sd).and(clean(&usb));
    say!(
        "\nthe test folders {} (took {:.0} s)",
        match &cleaned {
            Ok(()) => "are deleted".to_string(),
            Err(e) => format!("could NOT be deleted: {e}"),
        },
        t.elapsed().as_secs_f64()
    );
    let rows = r.map_err(read::Fail::Failed)?;
    cleaned.map_err(read::Fail::Failed)?;
    Ok(rows)
}

/// The whole run. On the MPC the caller has checked the stick's partition
/// 4 first, as every run does, while no MMC device was there.
pub(super) fn run(p: &Places) -> Result<(), read::Fail> {
    // The windows make their status folder (/run/mpfree-setup) as they
    // start; the pad player's own folder must already be there.
    if !p.app.is_dir() {
        return Err(read::Fail::Refused(format!(
            "{} is missing: is this the stick's system with the pad player?",
            p.app.display()
        )));
    }
    let mut own_rows = Vec::new();
    read_part(p, &mut own_rows)?;
    write_part(p, &own_rows)
}

#[cfg(test)]
mod tests {
    use super::own;

    #[test]
    fn only_its_own_names() {
        for n in [
            "seq.bin",
            "copy.bin",
            "rec.bin",
            "mix3.bin",
            "save-000.bin",
            "save-039.bin.part",
        ] {
            assert!(own(n), "{n}");
        }
        for n in [
            "save-1.bin",
            "save-0001.bin",
            "save-abc.bin",
            "notes.txt",
            "seq.bin.part",
            "save-001.wav",
        ] {
            assert!(!own(n), "{n}");
        }
    }
}
