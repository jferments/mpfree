//! mpfree-disktest: how fast, and above all how steadily, the test stick's
//! data partition takes the audio tracks' streaming, to size the RAM
//! buffer in front of it.
//!
//! On the MPC it runs only against `/run/mpfree-data`, and only if that is
//! the mount of the stick's partition 4: an ext4 file system labelled
//! `MPFREE-DATA` on a USB disk, with no MMC device on the system at all.
//! It writes only inside its own folder there (`mpfree-disktest/`), with
//! files it makes itself (`O_EXCL`, `O_NOFOLLOW`), and deletes the folder at
//! the end, after an error, on Ctrl-C, or when the ssh session closes. It reads the partition's
//! superblock (read-only) to check the label. Free space is printed before
//! and after.
//!
//! The steps, every I/O in 128 KiB blocks (the engine's chunk):
//!
//! 1. Sequential write, as fast as it goes, then fsync: throughput.
//! 2. Paced write at 3.1 MB/s (8 stereo tracks recording 48 kHz float)
//!    for 60 s, fdatasync every second: each write's and each sync's time,
//!    and how far behind its schedule it fell (the buffer it would need).
//! 3. Reads after dropping the file's pages from the cache
//!    (`posix_fadvise DONTNEED`, that file only): sequential as fast as it
//!    goes, then 8 streams at different places, paced at 3.1 MB/s for 60 s.
//! 4. Mixed for 60 s: 4 streams writing and 4 reading at once, each half
//!    at 1.55 MB/s, on two threads (as the recorder and player would).
//!
//! The files are sized to fit: at most 448 MiB at once, and at most half
//! the free space; under 32 MiB it refuses. Smaller files are named in the
//! report (a paced write that reaches the end of its file goes on from the
//! start, and says how often).
//!
//! On a desktop, `--desktop-test DIR` runs against any empty-or-missing
//! folder's parent instead (no mount checks), and `--quick` cuts every
//! step to 2 s and 8 MiB, for the tests.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const MOUNT: &str = "/run/mpfree-data";
const LABEL: &str = "MPFREE-DATA";
const FOLDER: &str = "mpfree-disktest";
const BLOCK: usize = 128 * 1024;
const RATE: f64 = 3.1e6;
const MIB: u64 = 1 << 20;
const MAX_BYTES: u64 = 448 * MIB;
const MIN_BYTES: u64 = 32 * MIB;
const STREAMS: usize = 8;
/// Every file the tool may make, and so the only ones it deletes.
const FILES: [&str; 6] = [
    "seq.bin",
    "paced.bin",
    "mix0.bin",
    "mix1.bin",
    "mix2.bin",
    "mix3.bin",
];

/// Print, ignoring a closed output (the ssh session gone): the folder must
/// still be cleaned up, so printing never stops the program.
macro_rules! say {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout(), $($t)*);
    }};
}

macro_rules! esay {
    ($($t:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stderr(), $($t)*);
    }};
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn stopped() -> Result<(), String> {
    if STOP.load(Ordering::Relaxed) {
        Err("stopped (Ctrl-C, SIGTERM or the session closed)".into())
    } else {
        Ok(())
    }
}

struct Opts {
    base: PathBuf,
    desktop: bool,
    quick: bool,
}

fn parse() -> Result<Opts, String> {
    let mut o = Opts {
        base: PathBuf::from(MOUNT),
        desktop: false,
        quick: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--desktop-test" => {
                o.base = PathBuf::from(args.next().ok_or("--desktop-test needs a folder")?);
                o.desktop = true;
            }
            "--quick" => o.quick = true,
            "-h" | "--help" => {
                say!("usage: mpfree-disktest [--desktop-test DIR] [--quick]");
                say!("Without --desktop-test it runs only on {MOUNT}, the stick's partition 4.");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument {a:?}")),
        }
    }
    if o.quick && !o.desktop {
        return Err("--quick is only for --desktop-test".into());
    }
    Ok(o)
}

/// The mount source of `point` from /proc/self/mountinfo, and its type.
fn mount_of(point: &str) -> Result<(String, String), String> {
    let info = fs::read_to_string("/proc/self/mountinfo").map_err(|e| format!("mountinfo: {e}"))?;
    let mut found = None;
    for line in info.lines() {
        let f: Vec<&str> = line.split(' ').collect();
        let Some(dash) = f.iter().position(|&x| x == "-") else {
            continue;
        };
        if f.len() > dash + 2 && f.get(4) == Some(&point) {
            // The last mount on a point is the one in use.
            found = Some((f[dash + 2].to_string(), f[dash + 1].to_string()));
        }
    }
    found.ok_or_else(|| format!("{point} is not a mount point"))
}

/// The ext4 volume label of block device `dev`, read from its superblock
/// (read-only).
fn ext4_label(dev: &str) -> Result<String, String> {
    let mut f = File::open(dev).map_err(|e| format!("{dev}: {e}"))?;
    let mut sb = [0u8; 1024];
    f.seek(SeekFrom::Start(1024))
        .and_then(|_| f.read_exact(&mut sb))
        .map_err(|e| format!("{dev} superblock: {e}"))?;
    if u16::from_le_bytes([sb[56], sb[57]]) != 0xEF53 {
        return Err(format!("{dev} is not ext2/3/4"));
    }
    let name = &sb[120..136];
    let end = name.iter().position(|&b| b == 0).unwrap_or(16);
    Ok(String::from_utf8_lossy(&name[..end]).into_owned())
}

/// Rule zero: only the stick's partition 4.
fn check_device() -> Result<(), String> {
    for dir in ["/dev", "/sys/block"] {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with("mmcblk") {
                    return Err(format!(
                        "{dir}/{} exists: refusing while any MMC device is present",
                        e.file_name().to_string_lossy()
                    ));
                }
            }
        }
    }
    let (src, fstype) = mount_of(MOUNT)?;
    if fstype != "ext4" {
        return Err(format!("{MOUNT} is {fstype}, not ext4"));
    }
    let name = src
        .strip_prefix("/dev/")
        .filter(|n| !n.is_empty() && !n.contains('/'))
        .ok_or_else(|| format!("{MOUNT} comes from {src}, not a /dev block device"))?;
    let sys = fs::canonicalize(format!("/sys/class/block/{name}"))
        .map_err(|e| format!("/sys/class/block/{name}: {e}"))?;
    if !sys.to_string_lossy().contains("/usb") {
        return Err(format!("{src} is not on a USB disk ({})", sys.display()));
    }
    let part = fs::read_to_string(format!("/sys/class/block/{name}/partition")).unwrap_or_default();
    if part.trim() != "4" {
        return Err(format!("{src} is not partition 4"));
    }
    let label = ext4_label(&src)?;
    if label != LABEL {
        return Err(format!("{src} is labelled {label:?}, not {LABEL}"));
    }
    say!("device: {MOUNT} is {src} (USB, partition 4, ext4 {LABEL:?}); no MMC device");
    Ok(())
}

fn free_bytes(p: &Path) -> Result<u64, String> {
    let c = std::ffi::CString::new(p.as_os_str().to_string_lossy().as_bytes())
        .map_err(|e| e.to_string())?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: c is a valid C string and s a valid statvfs to fill.
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return Err(format!(
            "statvfs {}: {}",
            p.display(),
            std::io::Error::last_os_error()
        ));
    }
    Ok(s.f_bavail as u64 * s.f_frsize as u64)
}

/// Delete the tool's folder: only its own regular files, then the folder.
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
    for e in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        let m = fs::symlink_metadata(e.path()).map_err(|e| e.to_string())?;
        if !FILES.contains(&n.as_str()) || !m.file_type().is_file() {
            return Err(format!(
                "{} holds {n:?}, which is not the tool's: left alone",
                dir.display()
            ));
        }
    }
    for n in FILES {
        match fs::remove_file(dir.join(n)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("remove {n}: {e}")),
        }
    }
    fs::remove_dir(dir).map_err(|e| format!("remove {}: {e}", dir.display()))
}

fn create(dir: &Path, name: &str) -> Result<File, String> {
    debug_assert!(FILES.contains(&name));
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(name))
        .map_err(|e| format!("create {name}: {e}"))
}

fn open_read(dir: &Path, name: &str) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(dir.join(name))
        .map_err(|e| format!("open {name}: {e}"))
}

fn drop_cache(f: &File) {
    // SAFETY: a valid descriptor; advice only, on this file's pages.
    unsafe {
        libc::posix_fadvise(f.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

/// Times of one kind of operation, in ms.
#[derive(Default)]
struct Times {
    ms: Vec<f64>,
}

impl Times {
    fn time<T>(&mut self, f: impl FnOnce() -> std::io::Result<T>) -> Result<T, String> {
        let t = Instant::now();
        let r = f().map_err(|e| e.to_string());
        self.ms.push(t.elapsed().as_secs_f64() * 1e3);
        r
    }

    fn report(&self, what: &str) -> String {
        if self.ms.is_empty() {
            return format!("  {what}: none");
        }
        let mut v = self.ms.clone();
        v.sort_by(f64::total_cmp);
        let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        let over = |l: f64| v.iter().filter(|&&x| x > l).count();
        format!(
            "  {what}: {} ops, p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms; over 50/100/250/500 ms: {}/{}/{}/{}",
            v.len(),
            q(0.5),
            q(0.99),
            v[v.len() - 1],
            over(50.0),
            over(100.0),
            over(250.0),
            over(500.0)
        )
    }
}

/// Paces operations at `rate` bytes a second and keeps how far behind
/// schedule they fell.
struct Pace {
    start: Instant,
    interval: f64,
    n: u64,
    behind_ms: f64,
}

impl Pace {
    fn new(rate: f64) -> Pace {
        Pace {
            start: Instant::now(),
            interval: BLOCK as f64 / rate,
            n: 0,
            behind_ms: 0.0,
        }
    }

    /// Wait for the next slot; false once `secs` have passed.
    fn next(&mut self, secs: f64) -> bool {
        let due = self.n as f64 * self.interval;
        if due >= secs {
            return false;
        }
        let now = self.start.elapsed().as_secs_f64();
        if now < due {
            std::thread::sleep(Duration::from_secs_f64(due - now));
        } else {
            self.behind_ms = self.behind_ms.max((now - due) * 1e3);
        }
        self.n += 1;
        true
    }
}

fn mbps(bytes: u64, t: Duration) -> f64 {
    bytes as f64 / 1e6 / t.as_secs_f64().max(1e-9)
}

fn pattern(buf: &mut [u8], n: u64) {
    for (i, b) in buf.iter_mut().enumerate() {
        *b = (i as u64).wrapping_mul(31).wrapping_add(n) as u8;
    }
}

struct Sizes {
    seq: u64,
    paced: u64,
    mix: u64,
    secs: f64,
}

fn run(dir: &Path, z: &Sizes) -> Result<(), String> {
    let mut buf = vec![0u8; BLOCK];
    let blocks = |bytes: u64| bytes / BLOCK as u64;

    // 1. Sequential write.
    say!("\n1. Sequential write, {} MiB, then fsync", z.seq / MIB);
    let mut f = create(dir, "seq.bin")?;
    let mut w = Times::default();
    let t = Instant::now();
    for n in 0..blocks(z.seq) {
        stopped()?;
        pattern(&mut buf, n);
        w.time(|| f.write_all(&buf))?;
    }
    let mut s = Times::default();
    s.time(|| f.sync_all())?;
    say!("  {:.1} MB/s including the fsync", mbps(z.seq, t.elapsed()));
    say!("{}", w.report("write"));
    say!("{}", s.report("fsync"));
    drop_cache(&f);
    drop(f);

    // 2. Paced write.
    say!(
        "\n2. Paced write at {:.2} MB/s for {:.0} s, fdatasync every second, in a {} MiB file",
        RATE / 1e6,
        z.secs,
        z.paced / MIB
    );
    let mut f = create(dir, "paced.bin")?;
    let (mut w, mut s) = (Times::default(), Times::default());
    let mut p = Pace::new(RATE);
    let (mut at, mut wraps, mut last_sync) = (0u64, 0u32, Instant::now());
    while p.next(z.secs) {
        stopped()?;
        if at + BLOCK as u64 > z.paced {
            f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            at = 0;
            wraps += 1;
        }
        pattern(&mut buf, p.n);
        w.time(|| f.write_all(&buf))?;
        at += BLOCK as u64;
        if last_sync.elapsed() >= Duration::from_secs(1) {
            s.time(|| f.sync_data())?;
            last_sync = Instant::now();
        }
    }
    s.time(|| f.sync_data())?;
    say!("{}", w.report("write"));
    say!("{}", s.report("fdatasync"));
    say!(
        "  furthest behind schedule: {:.1} ms (the buffer a recording would need)",
        p.behind_ms
    );
    if wraps > 0 {
        say!("  the file filled and was written again from the start {wraps} times");
    }
    drop(f);
    clean_one(dir, "paced.bin")?;

    // 3. Reads.
    say!(
        "\n3a. Sequential read of {} MiB, cache dropped first",
        z.seq / MIB
    );
    let mut f = open_read(dir, "seq.bin")?;
    drop_cache(&f);
    let mut r = Times::default();
    let t = Instant::now();
    for _ in 0..blocks(z.seq) {
        stopped()?;
        r.time(|| f.read_exact(&mut buf))?;
    }
    say!("  {:.1} MB/s", mbps(z.seq, t.elapsed()));
    say!("{}", r.report("read"));
    say!(
        "\n3b. {STREAMS} streams read at {:.2} MB/s in all for {:.0} s, cache dropped first",
        RATE / 1e6,
        z.secs
    );
    drop_cache(&f);
    let stride = blocks(z.seq) / STREAMS as u64;
    let mut pos = [0u64; STREAMS];
    let mut r = Times::default();
    let mut p = Pace::new(RATE);
    while p.next(z.secs) {
        stopped()?;
        let k = (p.n as usize - 1) % STREAMS;
        let b = k as u64 * stride + pos[k] % stride.max(1);
        pos[k] += 1;
        r.time(|| {
            f.seek(SeekFrom::Start(b * BLOCK as u64))?;
            f.read_exact(&mut buf)
        })?;
    }
    say!("{}", r.report("read"));
    say!(
        "  furthest behind schedule: {:.1} ms (the read-ahead playback would need)",
        p.behind_ms
    );

    // 4. Mixed.
    say!(
        "\n4. Mixed for {:.0} s: 4 streams write and 4 read, {:.2} MB/s each half, on two threads",
        z.secs,
        RATE / 2e6
    );
    drop_cache(&f);
    let secs = z.secs;
    let mix = z.mix;
    let wdir = dir.to_path_buf();
    let writer = std::thread::spawn(move || -> Result<(Times, Times, f64), String> {
        let mut files = Vec::new();
        for n in ["mix0.bin", "mix1.bin", "mix2.bin", "mix3.bin"] {
            files.push((create(&wdir, n)?, 0u64));
        }
        let mut buf = vec![0u8; BLOCK];
        let (mut w, mut s) = (Times::default(), Times::default());
        let mut p = Pace::new(RATE / 2.0);
        let mut last_sync = Instant::now();
        while p.next(secs) {
            stopped()?;
            let (f, at) = &mut files[(p.n as usize - 1) % 4];
            if *at + BLOCK as u64 > mix / 4 {
                f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
                *at = 0;
            }
            pattern(&mut buf, p.n);
            w.time(|| f.write_all(&buf))?;
            *at += BLOCK as u64;
            if last_sync.elapsed() >= Duration::from_secs(1) {
                for (f, _) in &files {
                    s.time(|| f.sync_data())?;
                }
                last_sync = Instant::now();
            }
        }
        Ok((w, s, p.behind_ms))
    });
    let mut r = Times::default();
    let mut p = Pace::new(RATE / 2.0);
    let mut read_err = None;
    while p.next(secs) {
        if let Err(e) = stopped() {
            read_err = Some(e);
            break;
        }
        let k = (p.n as usize - 1) % 4;
        let b = k as u64 * stride + pos[k] % stride.max(1);
        pos[k] += 1;
        if let Err(e) = r.time(|| {
            f.seek(SeekFrom::Start(b * BLOCK as u64))?;
            f.read_exact(&mut buf)
        }) {
            read_err = Some(e);
            STOP.store(true, Ordering::Relaxed);
            break;
        }
    }
    let wr = writer
        .join()
        .map_err(|_| "the writer thread failed".to_string())?;
    if let Some(e) = read_err {
        return Err(e);
    }
    let (w, s, wbehind) = wr?;
    say!("{}", w.report("write"));
    say!("{}", s.report("fdatasync"));
    say!("{}", r.report("read"));
    say!(
        "  furthest behind schedule: writing {wbehind:.1} ms, reading {:.1} ms",
        p.behind_ms
    );
    Ok(())
}

fn clean_one(dir: &Path, name: &str) -> Result<(), String> {
    debug_assert!(FILES.contains(&name));
    fs::remove_file(dir.join(name)).map_err(|e| format!("remove {name}: {e}"))
}

fn main() -> ExitCode {
    let o = match parse() {
        Ok(o) => o,
        Err(e) => {
            esay!("mpfree-disktest: {e}");
            return ExitCode::from(2);
        }
    };
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
    }
    say!(
        "mpfree-disktest: streaming speed and stalls, {} KiB blocks",
        BLOCK / 1024
    );
    if !o.desktop {
        if let Err(e) = check_device() {
            esay!("mpfree-disktest: refused: {e}. Nothing was written.");
            return ExitCode::from(3);
        }
    }
    let base = match fs::canonicalize(&o.base) {
        Ok(b) if o.desktop || b == Path::new(MOUNT) => b,
        Ok(b) => {
            esay!(
                "mpfree-disktest: refused: {} is not {MOUNT}. Nothing was written.",
                b.display()
            );
            return ExitCode::from(3);
        }
        Err(e) => {
            esay!(
                "mpfree-disktest: refused: {}: {e}. Nothing was written.",
                o.base.display()
            );
            return ExitCode::from(3);
        }
    };
    let dir = base.join(FOLDER);
    if let Err(e) = clean(&dir) {
        esay!("mpfree-disktest: refused: {e}. Nothing was written.");
        return ExitCode::from(3);
    }
    let free = match free_bytes(&base) {
        Ok(f) => f,
        Err(e) => {
            esay!("mpfree-disktest: refused: {e}. Nothing was written.");
            return ExitCode::from(3);
        }
    };
    say!(
        "free before: {:.1} MiB on {}",
        free as f64 / MIB as f64,
        base.display()
    );
    let room = if o.quick {
        8 * MIB
    } else {
        (free / 2).min(MAX_BYTES)
    };
    if room < if o.quick { 8 * MIB } else { MIN_BYTES } {
        esay!(
            "mpfree-disktest: refused: {:.1} MiB free, it needs at least {} MiB. Nothing was written.",
            free as f64 / MIB as f64,
            2 * MIN_BYTES / MIB
        );
        return ExitCode::from(3);
    }
    // seq + paced at most at once (paced is gone before mix is made).
    let round = |b: u64| (b / BLOCK as u64).max(STREAMS as u64) * BLOCK as u64;
    let z = Sizes {
        seq: round(room * 4 / 7),
        paced: round(room * 3 / 7),
        mix: round(room * 3 / 7),
        secs: if o.quick { 2.0 } else { 60.0 },
    };
    if room < MAX_BYTES && !o.quick {
        say!(
            "files sized to the room: {} MiB at most at once, not {}",
            room / MIB,
            MAX_BYTES / MIB
        );
    }
    let t = Instant::now();
    let r = fs::DirBuilder::new()
        .create(&dir)
        .map_err(|e| format!("create {}: {e}", dir.display()))
        .and_then(|()| run(&dir, &z));
    let cleaned = clean(&dir);
    let after = free_bytes(&base).map(|f| format!("{:.1} MiB", f as f64 / MIB as f64));
    say!(
        "\nfree after: {} (the folder {})",
        after.unwrap_or_else(|e| e),
        match &cleaned {
            Ok(()) => "is deleted".to_string(),
            Err(e) => format!("could NOT be deleted: {e}"),
        }
    );
    say!("took {:.0} s", t.elapsed().as_secs_f64());
    match (r, cleaned) {
        (Ok(()), Ok(())) => {
            say!("done");
            ExitCode::SUCCESS
        }
        (Err(e), _) => {
            esay!("mpfree-disktest: failed: {e}");
            ExitCode::FAILURE
        }
        (Ok(()), Err(_)) => ExitCode::FAILURE,
    }
}
