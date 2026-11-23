//! `mpfree-disktest --under-load`: playback's streams and a recording while
//! other work fights them for the stick and the processor (D2's second part
//! in docs/dev/design/disk-streaming.md).
//!
//! Every step runs the same two sets of streams, reading the audio under
//! `library/` and the files under `backup/` the way `--read-only` does
//! ([`super::read`]): 8 streams of 24-bit with 128 KiB reads (the engine's
//! chunk), then 24 with 512 KiB reads (its joined reads), 20 s each. A
//! recording runs beside them the whole step: 8 stereo float tracks
//! (3.1 MB/s) written in 128 KiB blocks to a new file, `fdatasync` every
//! second. What each step adds:
//!
//! 1. Nothing (the step the others are read against).
//! 2. A heavy write: a copy as fast as the stick takes it, `fdatasync`
//!    every second.
//! 3. A heavy read: other pieces read as fast as the stick gives them,
//!    512 KiB at a time (a whole sound loaded, a peaks job).
//! 4. The heavy write and the heavy read at once.
//! 5. A busy processor: one thread on each core, filtering a 256 KiB
//!    buffer over and over.
//! 6. Heavy task switching: 8 pairs of threads passing a byte back and
//!    forth, and 32 threads sleeping 200 microseconds at a time.
//! 7. Steps 5 and 6 at once.
//! 8. Everything: steps 2 to 6 at once.
//!
//! Every thread is an ordinary one (`SCHED_OTHER`, nice 0), like the
//! streams', so the pad player's real-time audio thread and the interrupt
//! threads still come first. The busy threads are pinned one to each core;
//! the rest go where Linux puts ordinary threads.
//!
//! **Heat.** The load steps (5 to 8) are watched: the temperature is read
//! every half second (`/sys/class/thermal`, read-only), and the busy and
//! switching threads stop at 75 °C, or at the lowest trip point the kernel
//! gives that is lower (trip points under 40 °C are not heat limits). Once
//! stopped, the remaining load steps are left out. On the MPC, with no
//! temperature to read, the load steps are left out from the start; on a
//! desktop test they run unwatched.
//!
//! **Files.** It writes only `rec.bin` and `copy.bin` in its own folder,
//! made new for each step and deleted after it, together at most the room
//! the write run allows (448 MiB, half the free space). A file that fills
//! is written again from its start. The library and backup are only read,
//! with `O_NOFOLLOW`, and checked at the end to be as they were.
//!
//! For each step: the CPUs' busy and disk-wait shares and the task switches
//! a second (`/proc/stat`), the temperature and clocks, how late a 1 ms
//! sleep wakes, each stream set's read times, worst shortfall and the
//! plan's window, the recording's write and sync times and how far behind
//! it fell, and the heavy jobs' speed. Then a summary table.

use super::read::{self, Config, Fail, Piece, KIB, RATE_24};
use super::{create, mbps, pattern, stopped, Pace, Times, BLOCK, MIB, RATE, STOP};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread::{Builder, Scope};
use std::time::{Duration, Instant};

/// Where the temperature is read (read-only).
pub(super) const THERMAL: &str = "/sys/class/thermal";
/// The busy and switching threads stop here at the latest.
const HOT_CAP_C: f64 = 75.0;
/// Trip points under this are not heat limits.
const TRIP_FLOOR_C: f64 = 40.0;
const PAIRS: usize = 8;
const SLEEPERS: usize = 32;
const SLEEP: Duration = Duration::from_micros(200);
/// The busy and switching threads' stacks.
const SMALL_STACK: usize = 128 * 1024;
/// Floats each busy thread filters per pass (256 KiB).
const HOG_SAMPLES: usize = 64 * 1024;
/// The heavy read's size.
const HEAVY_READ: usize = 512 * KIB;
/// A stream set that falls this many times its length behind is stopped,
/// so a stick that can't keep up still ends the step.
const CAP: f64 = 3.0;

#[derive(Clone, Copy, Default, PartialEq, Debug)]
struct Load {
    heavy_write: bool,
    heavy_read: bool,
    cpu: bool,
    switch: bool,
}

impl Load {
    /// Steps that heat the processor are watched and can be left out.
    fn heats(self) -> bool {
        self.cpu || self.switch
    }
}

const NONE: Load = Load {
    heavy_write: false,
    heavy_read: false,
    cpu: false,
    switch: false,
};

const STEPS: [(&str, Load); 8] = [
    ("Nothing else", NONE),
    (
        "A heavy write",
        Load {
            heavy_write: true,
            ..NONE
        },
    ),
    (
        "A heavy read",
        Load {
            heavy_read: true,
            ..NONE
        },
    ),
    (
        "A heavy write and a heavy read",
        Load {
            heavy_write: true,
            heavy_read: true,
            ..NONE
        },
    ),
    ("A busy processor", Load { cpu: true, ..NONE }),
    (
        "Heavy task switching",
        Load {
            switch: true,
            ..NONE
        },
    ),
    (
        "A busy processor and heavy task switching",
        Load {
            cpu: true,
            switch: true,
            ..NONE
        },
    ),
    (
        "Everything at once",
        Load {
            heavy_write: true,
            heavy_read: true,
            cpu: true,
            switch: true,
        },
    ),
];

/// A thermal zone whose temperature reads, and its trip points in °C.
#[derive(Debug, PartialEq)]
pub(super) struct Zone {
    pub(super) name: String,
    pub(super) temp: PathBuf,
    pub(super) trips: Vec<(String, f64)>,
}

pub(super) fn millidegrees(p: &Path) -> Option<f64> {
    fs::read_to_string(p)
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
        .map(|m| m / 1000.0)
}

/// The zones under `root` (`/sys/class/thermal`) whose temperature reads.
pub(super) fn zones(root: &Path) -> Vec<Zone> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(root) else {
        return out;
    };
    let mut dirs: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("thermal_zone"))
        })
        .collect();
    dirs.sort();
    for d in dirs {
        let temp = d.join("temp");
        if millidegrees(&temp).is_none() {
            continue;
        }
        let name = fs::read_to_string(d.join("type"))
            .map(|t| t.trim().to_string())
            .unwrap_or_else(|_| "?".into());
        let mut trips = Vec::new();
        for n in 0..16 {
            let Some(t) = millidegrees(&d.join(format!("trip_point_{n}_temp"))) else {
                continue;
            };
            let kind = fs::read_to_string(d.join(format!("trip_point_{n}_type")))
                .map(|t| t.trim().to_string())
                .unwrap_or_else(|_| "?".into());
            trips.push((kind, t));
        }
        out.push(Zone { name, temp, trips });
    }
    out
}

/// Where the load stops: 75 °C, or the lowest trip point at or over 40 °C
/// if that is lower.
pub(super) fn stop_at(zones: &[Zone]) -> f64 {
    zones
        .iter()
        .flat_map(|z| z.trips.iter().map(|t| t.1))
        .filter(|&t| t >= TRIP_FLOOR_C)
        .fold(HOT_CAP_C, f64::min)
}

/// The hottest zone now, in °C.
pub(super) fn hottest(zones: &[Zone]) -> Option<f64> {
    zones
        .iter()
        .filter_map(|z| millidegrees(&z.temp))
        .reduce(f64::max)
}

/// Each CPU's clock now, in MHz.
fn clocks(cpus: usize) -> Vec<Option<u64>> {
    (0..cpus)
        .map(|c| {
            fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{c}/cpufreq/scaling_cur_freq"
            ))
            .ok()
            .and_then(|f| f.trim().parse::<u64>().ok())
            .map(|k| k / 1000)
        })
        .collect()
}

/// `/proc/stat`'s counters: each CPU's (busy, waiting on a disk, all)
/// ticks, and the task switches since the start.
#[derive(Debug, PartialEq)]
pub(super) struct Stat {
    pub(super) cpus: Vec<(u64, u64, u64)>,
    pub(super) switches: u64,
}

pub(super) fn parse_stat(text: &str) -> Option<Stat> {
    let mut cpus = Vec::new();
    let mut switches = None;
    for line in text.lines() {
        let mut f = line.split_whitespace();
        match f.next() {
            Some(n) if n.starts_with("cpu") && n.len() > 3 => {
                let v: Vec<u64> = f.filter_map(|x| x.parse().ok()).collect();
                if v.len() < 5 {
                    return None;
                }
                // user nice system idle iowait irq softirq steal (guest
                // time is already in user and nice).
                let all: u64 = v.iter().take(8).sum();
                let (idle, wait) = (v[3], v[4]);
                cpus.push((all - idle - wait, wait, all));
            }
            Some("ctxt") => switches = f.next().and_then(|x| x.parse().ok()),
            _ => {}
        }
    }
    Some(Stat {
        cpus,
        switches: switches?,
    })
}

fn read_stat() -> Option<(Stat, Instant)> {
    let t = Instant::now();
    parse_stat(&fs::read_to_string("/proc/stat").ok()?).map(|s| (s, t))
}

/// "busy 99/98/100/100 %, waiting on a disk 1/0/0/0 %; 123,456 task
/// switches a second", or None if the CPUs changed.
pub(super) fn load_line(a: &Stat, b: &Stat, secs: f64) -> Option<(String, f64, f64)> {
    if a.cpus.len() != b.cpus.len() || a.cpus.is_empty() {
        return None;
    }
    let mut busy = Vec::new();
    let mut wait = Vec::new();
    let (mut busy_all, mut all_all) = (0u64, 0u64);
    for (x, y) in a.cpus.iter().zip(&b.cpus) {
        let all = y.2.saturating_sub(x.2).max(1);
        let bz = y.0.saturating_sub(x.0);
        busy.push(format!("{:.0}", 100.0 * bz as f64 / all as f64));
        wait.push(format!(
            "{:.0}",
            100.0 * y.1.saturating_sub(x.1) as f64 / all as f64
        ));
        busy_all += bz;
        all_all += all;
    }
    let per_s = b.switches.saturating_sub(a.switches) as f64 / secs.max(1e-9);
    let pct = 100.0 * busy_all as f64 / all_all.max(1) as f64;
    Some((
        format!(
            "CPUs busy {} %, waiting on a disk {} %; {} task switches a second",
            busy.join("/"),
            wait.join("/"),
            thousands(per_s.round() as u64)
        ),
        pct,
        per_s,
    ))
}

pub(super) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub(super) fn cpus_online() -> usize {
    // SAFETY: sysconf only reads a value.
    let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if n < 1 {
        1
    } else {
        n as usize
    }
}

/// Pin the calling thread to `cpu`; false if Linux refused.
fn pin(cpu: usize) -> bool {
    // SAFETY: a zeroed cpu_set_t is an empty set; CPU_SET writes inside it;
    // the call reads it and changes only this thread's affinity.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_SET(cpu, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set) == 0
    }
}

/// What every load thread checks: the step goes on, nobody pressed
/// Ctrl-C, and (for the heating ones) the MPC isn't hot.
fn going(on: &AtomicBool) -> bool {
    on.load(Ordering::Relaxed) && !STOP.load(Ordering::Relaxed)
}

/// One busy thread: filter its buffer until told to stop. Passes made, and
/// whether it was pinned.
pub(super) fn hog(cpu: usize, on: &AtomicBool, hot: &AtomicBool) -> (u64, bool) {
    let pinned = pin(cpu);
    let mut buf: Vec<f32> = (0..HOG_SAMPLES)
        .map(|i| ((i as f32) * 0.01).sin())
        .collect();
    let (mut z1, mut z2, mut passes) = (0f32, 0f32, 0u64);
    // A low-pass biquad (stable); the small offset keeps it off denormals.
    let (b0, b1, b2, a1, a2) = (0.067f32, 0.135f32, 0.067f32, -1.143f32, 0.413f32);
    while going(on) && !hot.load(Ordering::Relaxed) {
        for x in buf.iter_mut() {
            let y = b0 * *x + z1;
            z1 = b1 * *x - a1 * y + z2;
            z2 = b2 * *x - a2 * y;
            *x = y + 1e-3;
        }
        passes += 1;
    }
    std::hint::black_box(&buf);
    (passes, pinned)
}

/// Threads that only spin or switch need little stack.
fn small() -> Builder {
    Builder::new().stack_size(SMALL_STACK)
}

/// One pair of threads passing a byte back and forth, adding its round
/// trips to `trips`. The starter stops by dropping its end, which ends the
/// other's read; if the starter can't start, its end is dropped with it.
fn pair<'scope>(
    s: &'scope Scope<'scope, '_>,
    on: &'scope AtomicBool,
    hot: &'scope AtomicBool,
    trips: &'scope AtomicU64,
) -> std::io::Result<()> {
    let (mut a, mut b) = UnixStream::pair()?;
    small().spawn_scoped(s, move || {
        let mut byte = [0u8; 1];
        loop {
            match b.read(&mut byte) {
                Ok(1) => {
                    if b.write_all(&byte).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                _ => break,
            }
        }
    })?;
    small().spawn_scoped(s, move || {
        let mut byte = [1u8; 1];
        let mut n = 0u64;
        while going(on) && !hot.load(Ordering::Relaxed) {
            if a.write_all(&byte).is_err() || a.read_exact(&mut byte).is_err() {
                break;
            }
            n += 1;
        }
        trips.fetch_add(n, Ordering::Relaxed);
    })?;
    Ok(())
}

/// A background job failed: keep its error and stop the run, unless the
/// run was already stopping (then the error only follows from that).
fn failed(first: &Mutex<Option<String>>, e: String) -> String {
    if !STOP.swap(true, Ordering::SeqCst) {
        first
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_or_insert_with(|| e.clone());
    }
    e
}

/// A thread waking every 200 microseconds.
fn sleeper(on: &AtomicBool, hot: &AtomicBool) {
    while going(on) && !hot.load(Ordering::Relaxed) {
        std::thread::sleep(SLEEP);
    }
}

/// How late a 1 ms sleep wakes, and the hottest reading. Reads the
/// temperature at once and every half second after, and sets `hot` at
/// `limit`.
fn probe(on: &AtomicBool, hot: &AtomicBool, zones: &[Zone], limit: Option<f64>) -> (Times, f64) {
    let mut late = Times::default();
    let mut top = f64::NAN;
    let mut last: Option<Instant> = None;
    while going(on) {
        if last.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
            last = Some(Instant::now());
            if let Some(c) = hottest(zones) {
                top = if top.is_nan() { c } else { top.max(c) };
                if limit.is_some_and(|l| c >= l) {
                    hot.store(true, Ordering::Relaxed);
                }
            }
        }
        let t = Instant::now();
        std::thread::sleep(Duration::from_millis(1));
        late.ms
            .push((t.elapsed().as_secs_f64() * 1e3 - 1.0).max(0.0));
    }
    (late, top)
}

#[derive(Default)]
struct Written {
    writes: Times,
    syncs: Times,
    bytes: u64,
    wraps: u32,
    behind_ms: f64,
    secs: f64,
}

/// Write `name` new, in blocks, until the step ends: paced at `rate`, or
/// as fast as it goes with `None`. `fdatasync` every second; a full file
/// is written again from its start.
fn writer(
    dir: &Path,
    name: &str,
    cap: u64,
    rate: Option<f64>,
    on: &AtomicBool,
) -> Result<Written, String> {
    let mut f = create(dir, name)?;
    let mut buf = vec![0u8; BLOCK];
    let mut w = Written::default();
    let mut pace = rate.map(Pace::new);
    let (t, mut last_sync, mut at, mut n) = (Instant::now(), Instant::now(), 0u64, 0u64);
    while going(on) {
        if let Some(p) = pace.as_mut() {
            if !p.next(f64::INFINITY) {
                break;
            }
        }
        stopped()?;
        if at + BLOCK as u64 > cap {
            f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            at = 0;
            w.wraps += 1;
        }
        pattern(&mut buf, n);
        n += 1;
        w.writes.time(|| f.write_all(&buf))?;
        at += BLOCK as u64;
        w.bytes += BLOCK as u64;
        if last_sync.elapsed() >= Duration::from_secs(1) {
            w.syncs.time(|| f.sync_data())?;
            last_sync = Instant::now();
        }
    }
    w.syncs.time(|| f.sync_data())?;
    w.secs = t.elapsed().as_secs_f64();
    w.behind_ms = pace.map_or(0.0, |p| p.behind_ms);
    Ok(w)
}

#[derive(Default)]
struct HeavyRead {
    reads: Times,
    bytes: u64,
    secs: f64,
}

/// Read `pieces` in turn from `*next`, 512 KiB at a time, as fast as they
/// come, dropping each piece's pages after it, until the step ends.
fn heavy_reader(
    pieces: &[&Piece],
    dev: u64,
    next: &mut usize,
    on: &AtomicBool,
) -> Result<HeavyRead, String> {
    let mut buf = vec![0u8; HEAVY_READ];
    let mut r = HeavyRead::default();
    let t = Instant::now();
    // A step starts where the last left off, even past the last piece, so
    // only a whole pass from the first piece that read nothing is an error.
    let mut empty_pass = true;
    let mut whole_pass = false;
    'pieces: while going(on) {
        if *next >= pieces.len() {
            if whole_pass && empty_pass {
                return Err("the heavy read's files are empty now".into());
            }
            *next = 0;
            empty_pass = true;
            whole_pass = true;
        }
        let a = pieces[*next];
        *next += 1;
        let mut f =
            read::open_piece(a, dev).map_err(|e| format!("open a file on partition 4: {e}"))?;
        read::drop_range(&f, a.start, a.len);
        let mut left = a.len;
        while left > 0 {
            if !going(on) {
                read::drop_range(&f, a.start, a.len);
                break 'pieces;
            }
            let want = left.min(buf.len() as u64) as usize;
            let n = r.reads.time(|| read::fill(&mut f, &mut buf[..want]))?;
            r.bytes += n as u64;
            left -= n as u64;
            if n > 0 {
                empty_pass = false;
            }
            if n < want {
                break;
            }
        }
        read::drop_range(&f, a.start, a.len);
    }
    r.secs = t.elapsed().as_secs_f64();
    Ok(r)
}

/// What one step measured.
#[derive(Default)]
struct Measured {
    /// Each stream set's result (None: too little to read for it).
    sets: Vec<Option<read::Result1>>,
    rec: Written,
    copy: Option<Written>,
    heavy_read: Option<HeavyRead>,
    /// How late a 1 ms sleep woke.
    late: Times,
    /// The hottest reading during the step, and the reading at its end.
    top_c: f64,
    end_c: Option<f64>,
    /// The `/proc/stat` line, the CPUs' busy share and switches a second.
    load: Option<(String, f64, f64)>,
    mhz: Vec<Option<u64>>,
    /// Each busy thread's passes, and whether it was pinned.
    hogs: Vec<(u64, bool)>,
    trips: u64,
    /// The busy and switching threads stopped for the heat.
    hot: bool,
}

/// One row of the summary.
enum Row {
    LeftOut(String),
    Ran {
        /// Per stream set: worst shortfall, slowest read (ms), window (s),
        /// and whether it was stopped at the cap.
        sets: Vec<Option<(f64, f64, f64, bool)>>,
        rec_behind_ms: f64,
        rec_sync_max_ms: f64,
        wake_max_ms: f64,
        busy_pct: f64,
        switches: f64,
        top_c: f64,
    },
}

/// What the run needs, found before its folder is made.
pub(super) struct Ready {
    found: read::Found,
    before: read::Snapshot,
    dev: u64,
    sets: Vec<Config>,
    secs: f64,
    settle: f64,
    piece: u64,
    zones: Vec<Zone>,
    desktop: bool,
}

/// The two stream sets (8 x 128 KiB, 24 x 512 KiB; 2 x 64 and 4 x 128 KiB
/// at 16 times the rate in a quick run), 20 s each (0.4 s quick), the
/// settling time before them and the piece size.
fn plan(quick: bool) -> (Vec<Config>, f64, f64, u64) {
    let (speed, a, b) = if quick {
        (16.0, (2, 64), (4, 128))
    } else {
        (1.0, (8, 128), (24, 512))
    };
    let sets = [a, b]
        .into_iter()
        .map(|(streams, read)| Config {
            streams,
            read: read * KIB,
            rate: RATE_24 * speed,
            kind: "24-bit",
        })
        .collect();
    if quick {
        (sets, 0.4, 0.1, MIB)
    } else {
        (sets, 20.0, 2.0, read::PIECE)
    }
}

/// The bytes a stream of `c` reads in `secs`, and one read more.
fn wanted(c: &Config, secs: f64) -> u64 {
    (c.rate * secs) as u64 + c.read as u64
}

/// Find what to read and check there is enough; nothing is written yet.
pub(super) fn prepare(
    base: &Path,
    quick: bool,
    desktop: bool,
    thermal: &Path,
) -> Result<Ready, Fail> {
    let (sets, secs, settle, piece) = plan(quick);
    let (found, before) = read::find(base, piece)?;
    let dev = fs::symlink_metadata(base)
        .map_err(|e| Fail::Refused(format!("{}: {e}", base.display())))?
        .dev();
    let first = sets[0];
    if read::assign(
        &found.pieces,
        first.streams,
        wanted(&first, secs),
        first.read,
    )
    .is_none()
    {
        return Err(Fail::Refused(format!(
            "too little to read in {}/ and {}/: it needs at least {} files of 64 KiB or more",
            read::LIBRARY,
            read::BACKUP,
            first.streams
        )));
    }
    Ok(Ready {
        found,
        before,
        dev,
        sets,
        secs,
        settle,
        piece,
        zones: zones(thermal),
        desktop,
    })
}

fn set_label(c: &Config) -> String {
    format!(
        "{} streams, {} ({:.1} kB/s each), {} KiB reads",
        c.streams,
        c.kind,
        c.rate / 1e3,
        c.read / KIB
    )
}

/// Everything a step needs that stays the same from step to step.
struct Env<'a> {
    dir: &'a Path,
    dev: u64,
    sets: &'a [Config],
    /// Each set's pieces, one list a stream (None: too little for it).
    groups: Vec<Option<Vec<Vec<&'a Piece>>>>,
    /// The pieces the heavy read takes, in turn.
    heavy: Vec<&'a Piece>,
    secs: f64,
    settle: f64,
    zones: &'a [Zone],
    /// Where the busy and switching threads stop (None: not watched).
    limit: Option<f64>,
    cpus: usize,
    rec_cap: u64,
    copy_cap: u64,
}

/// Keep a thread's handle, or the first reason one couldn't start.
fn started<T>(r: std::io::Result<T>, err: &mut Option<String>) -> Option<T> {
    match r {
        Ok(h) => Some(h),
        Err(e) => {
            err.get_or_insert_with(|| format!("a thread could not start: {e}"));
            None
        }
    }
}

/// Tells the load threads to stop when dropped, so the scope's wait for
/// them always ends, whatever happens in the step.
struct StopOnDrop<'a>(&'a AtomicBool);

impl Drop for StopOnDrop<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Run one step: start its jobs, let them settle, run each stream set,
/// stop the jobs and collect what they measured.
fn run_step(env: &Env, load: Load, heavy_next: &mut usize) -> Result<Measured, String> {
    let (on, hot, trips) = (
        AtomicBool::new(true),
        AtomicBool::new(false),
        AtomicU64::new(0),
    );
    let first: Mutex<Option<String>> = Mutex::new(None);
    let (on, hot, trips, first) = (&on, &hot, &trips, &first);
    let (dir, dev, zones, heavy) = (env.dir, env.dev, env.zones, &env.heavy[..]);
    let (rec_cap, copy_cap) = (env.rec_cap, env.copy_cap);
    let watched = if load.heats() { env.limit } else { None };
    std::thread::scope(|s| {
        let _stop = StopOnDrop(on);
        let mut err: Option<String> = None;
        let rec = started(
            Builder::new().spawn_scoped(s, move || {
                writer(dir, "rec.bin", rec_cap, Some(RATE), on).map_err(|e| failed(first, e))
            }),
            &mut err,
        );
        let watch = started(
            Builder::new().spawn_scoped(s, move || probe(on, hot, zones, watched)),
            &mut err,
        );
        let copy = if load.heavy_write {
            started(
                Builder::new().spawn_scoped(s, move || {
                    writer(dir, "copy.bin", copy_cap, None, on).map_err(|e| failed(first, e))
                }),
                &mut err,
            )
        } else {
            None
        };
        let heavy_read = if load.heavy_read {
            started(
                Builder::new().spawn_scoped(s, move || {
                    heavy_reader(heavy, dev, heavy_next, on).map_err(|e| failed(first, e))
                }),
                &mut err,
            )
        } else {
            None
        };
        let hogs: Vec<_> = if load.cpu {
            (0..env.cpus)
                .filter_map(|c| started(small().spawn_scoped(s, move || hog(c, on, hot)), &mut err))
                .collect()
        } else {
            Vec::new()
        };
        if load.switch {
            for _ in 0..PAIRS {
                started(pair(s, on, hot, trips), &mut err);
            }
            for _ in 0..SLEEPERS {
                started(small().spawn_scoped(s, move || sleeper(on, hot)), &mut err);
            }
        }

        let mut m = Measured::default();
        if err.is_none() {
            std::thread::sleep(Duration::from_secs_f64(env.settle));
            let st0 = read_stat();
            for (c, g) in env.sets.iter().zip(&env.groups) {
                match g {
                    Some(g) => match read::run_config(*c, g, dev, env.secs, CAP * env.secs) {
                        Ok(r) => m.sets.push(Some(r)),
                        Err(e) => {
                            err = Some(e);
                            break;
                        }
                    },
                    None => m.sets.push(None),
                }
            }
            if let (Some((a, ta)), Some((b, tb))) = (st0, read_stat()) {
                m.load = load_line(&a, &b, tb.duration_since(ta).as_secs_f64());
            }
            m.end_c = hottest(zones);
            m.mhz = clocks(env.cpus);
        }
        m.hot = hot.load(Ordering::Relaxed);
        on.store(false, Ordering::Relaxed);

        let rec = rec.map(|h| {
            h.join()
                .unwrap_or_else(|_| Err("the recording's thread failed".into()))
        });
        (m.late, m.top_c) = watch
            .map(|h| h.join().unwrap_or_default())
            .unwrap_or_default();
        let copy = copy.map(|h| {
            h.join()
                .unwrap_or_else(|_| Err("the heavy write's thread failed".into()))
        });
        let heavy_read = heavy_read.map(|h| {
            h.join()
                .unwrap_or_else(|_| Err("the heavy read's thread failed".into()))
        });
        m.hogs = hogs
            .into_iter()
            .map(|h| h.join().unwrap_or((0, false)))
            .collect();
        // The pairs and sleepers have stopped too; the scope waits for them.

        // A background job's own failure first (the streams then only saw
        // the run stopping), then a thread that couldn't start or a stream's.
        if let Some(e) = first.lock().unwrap_or_else(|p| p.into_inner()).take() {
            return Err(e);
        }
        if let Some(e) = err {
            return Err(e);
        }
        m.rec = rec.ok_or_else(|| "the recording did not start".to_string())??;
        m.copy = copy.transpose()?;
        m.heavy_read = heavy_read.transpose()?;
        m.trips = trips.load(Ordering::Relaxed);
        Ok(m)
    })
}

fn celsius(c: Option<f64>) -> String {
    match c {
        Some(c) if c.is_finite() => format!("{c:.1} °C"),
        _ => "not read".into(),
    }
}

/// Print one step's measurements; its summary row.
fn report_step(env: &Env, load: Load, m: &Measured) -> Row {
    let mib = |b: u64| b as f64 / MIB as f64;
    let (busy_pct, switches) = match &m.load {
        Some((text, pct, sw)) => {
            say!("  load: {text}");
            (*pct, *sw)
        }
        None => {
            say!("  load: /proc/stat could not be read");
            (f64::NAN, f64::NAN)
        }
    };
    let mhz: Vec<String> = m
        .mhz
        .iter()
        .map(|m| m.map_or("?".into(), |m| m.to_string()))
        .collect();
    say!(
        "  temperature: {} at the end, {} at the most; clocks {} MHz",
        celsius(m.end_c),
        celsius(Some(m.top_c)),
        mhz.join("/")
    );
    if load.cpu {
        say!(
            "  busy threads: {} ({} pinned to their core), {} passes in all",
            m.hogs.len(),
            m.hogs.iter().filter(|h| h.1).count(),
            thousands(m.hogs.iter().map(|h| h.0).sum())
        );
    }
    if load.switch {
        say!(
            "  switching threads: {PAIRS} pairs ({} round trips), {SLEEPERS} sleepers",
            thousands(m.trips)
        );
    }
    if m.hot {
        say!(
            "  the busy and switching threads stopped early: the MPC reached {:.0} °C",
            env.limit.unwrap_or(HOT_CAP_C)
        );
    }
    say!("{}", m.late.report("a 1 ms sleep woke late by"));
    let mut sets = Vec::new();
    for (c, r) in env.sets.iter().zip(&m.sets) {
        match r {
            Some(r) => {
                let w = read::window_s(r.behind_ms);
                say!("  {}", set_label(c));
                say!("  {}", r.reads.report_tails("read"));
                say!(
                    "    worst shortfall {:.1} ms, longest behind {:.1} ms; the plan's window {w:.1} s a stream",
                    r.behind_ms,
                    r.longest_behind_ms
                );
                if r.cut {
                    say!(
                        "    stopped at {:.0} s: the streams could not keep up, so the shortfall is at least this",
                        CAP * env.secs
                    );
                }
                if let Some(b) = &r.beside {
                    say!("    {}", b.line());
                    say!("    {}", b.slow_line());
                }
                sets.push(Some((r.behind_ms, r.reads.max(), w, r.cut)));
            }
            None => {
                say!(
                    "  {}: skipped, too little to give each stream two reads of its own",
                    set_label(c)
                );
                sets.push(None);
            }
        }
    }
    say!(
        "  recording ({:.2} MB/s, fdatasync every second): {:.1} MiB",
        RATE / 1e6,
        mib(m.rec.bytes)
    );
    say!("  {}", m.rec.writes.report("write"));
    say!("  {}", m.rec.syncs.report("fdatasync"));
    say!(
        "    furthest behind schedule: {:.1} ms{}",
        m.rec.behind_ms,
        if m.rec.wraps > 0 {
            format!(
                " (the file filled and was written again from the start {} times)",
                m.rec.wraps
            )
        } else {
            String::new()
        }
    );
    if let Some(c) = &m.copy {
        say!(
            "  heavy write: {:.1} MB/s including the syncs ({:.1} MiB)",
            mbps(c.bytes, Duration::from_secs_f64(c.secs)),
            mib(c.bytes)
        );
        say!("  {}", c.writes.report("write"));
        say!("  {}", c.syncs.report("fdatasync"));
    }
    if let Some(h) = &m.heavy_read {
        say!(
            "  heavy read: {:.1} MB/s ({:.1} MiB)",
            mbps(h.bytes, Duration::from_secs_f64(h.secs)),
            mib(h.bytes)
        );
        say!("  {}", h.reads.report("read"));
    }
    Row::Ran {
        sets,
        rec_behind_ms: m.rec.behind_ms,
        rec_sync_max_ms: m.rec.syncs.max(),
        wake_max_ms: m.late.max(),
        busy_pct,
        switches,
        top_c: m.top_c,
    }
}

/// Why a step that heats the processor is left out, if it is.
fn left_out(
    load: Load,
    earlier: &Option<String>,
    limit: Option<f64>,
    zones: &[Zone],
) -> Option<String> {
    if !load.heats() {
        return None;
    }
    if let Some(why) = earlier {
        return Some(why.clone());
    }
    match (limit, hottest(zones)) {
        (Some(l), Some(c)) if c >= l => Some(format!(
            "the MPC is at {c:.1} °C, at or over the stop at {l:.0} °C"
        )),
        _ => None,
    }
}

fn summary(sets: &[Config], rows: &[Row]) {
    say!("\nSummary: for each stream set the worst shortfall and the slowest read (ms) and the plan's window (s a stream); the recording's furthest behind and slowest fdatasync, and the latest 1 ms wake-up (ms); how busy the CPUs were (%), the task switches a second and the hottest reading.");
    let head = |i: usize| {
        sets.get(i).map_or(String::new(), |c| {
            format!("{} x {} KiB", c.streams, c.read / KIB)
        })
    };
    say!(
        "  {:>4}  {:^22}  {:^22}  {:^16}",
        "",
        head(0),
        head(1),
        "recording"
    );
    say!(
        "  {:>4}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>6}  {:>8}  {:>8}  {:>4}  {:>10}  hottest",
        "step",
        "short",
        "max",
        "window",
        "short",
        "max",
        "window",
        "behind",
        "sync max",
        "wake max",
        "busy",
        "switches/s"
    );
    for (k, row) in rows.iter().enumerate() {
        match row {
            Row::LeftOut(why) => say!("  {:>4}  left out: {why}", k + 1),
            Row::Ran {
                sets,
                rec_behind_ms,
                rec_sync_max_ms,
                wake_max_ms,
                busy_pct,
                switches,
                top_c,
            } => {
                let cell = |i: usize| match sets.get(i) {
                    Some(Some((short, max, w, false))) => {
                        format!("{short:>6.1}  {max:>6.1}  {w:>6.1}")
                    }
                    Some(Some((short, max, w, true))) => {
                        format!("{short:>5.0}+  {max:>6.1}  {w:>6.1}")
                    }
                    _ => format!("{:^22}", "skipped"),
                };
                let busy = if busy_pct.is_finite() {
                    format!("{busy_pct:.0}")
                } else {
                    "?".into()
                };
                let sw = if switches.is_finite() {
                    thousands(switches.round() as u64)
                } else {
                    "?".into()
                };
                say!(
                    "  {:>4}  {}  {}  {rec_behind_ms:>6.1}  {rec_sync_max_ms:>8.1}  {wake_max_ms:>8.1}  {busy:>4}  {sw:>10}  {}",
                    k + 1,
                    cell(0),
                    cell(1),
                    celsius(Some(*top_c))
                );
            }
        }
    }
    let cut = rows
        .iter()
        .any(|r| matches!(r, Row::Ran { sets, .. } if sets.iter().flatten().any(|s| s.3)));
    if cut {
        say!(
            "  (+: the streams could not keep up and were stopped; the shortfall is at least this)"
        );
    }
}

/// Delete a step's files, whatever happened in it.
fn remove_step_files(dir: &Path) -> Result<(), String> {
    for n in ["rec.bin", "copy.bin"] {
        match fs::remove_file(dir.join(n)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("remove {n}: {e}")),
        }
    }
    Ok(())
}

/// The run, in `dir` (made and emptied by the caller), with at most `room`
/// bytes of its own files at once.
pub(super) fn run(base: &Path, dir: &Path, room: u64, z: Ready) -> Result<(), String> {
    let mib = |b: u64| b as f64 / MIB as f64;
    let found = &z.found;
    say!(
        "under load: reads the audio under {}/{} and the files under {}/{}, and writes only in {}",
        base.display(),
        read::LIBRARY,
        base.display(),
        read::BACKUP,
        dir.display()
    );
    say!(
        "found: {} in {}/ ({:.1} MiB), {} in {}/ ({:.1} MiB); {} pieces to read",
        read::count(found.files[0], "audio file"),
        read::LIBRARY,
        mib(found.bytes[0]),
        read::count(found.files[1], "file"),
        read::BACKUP,
        mib(found.bytes[1]),
        found.pieces.len()
    );

    // Each set's pieces; the heavy read takes the others (all of them if
    // the sets use every one).
    let groups: Vec<_> = z
        .sets
        .iter()
        .map(|c| read::assign(&found.pieces, c.streams, wanted(c, z.secs), c.read))
        .collect();
    let used: std::collections::HashSet<*const Piece> = groups
        .iter()
        .flatten()
        .flatten()
        .flatten()
        .map(|p| *p as *const Piece)
        .collect();
    let mut heavy: Vec<&Piece> = found
        .pieces
        .iter()
        .filter(|p| !used.contains(&(*p as *const Piece)))
        .collect();
    if heavy.is_empty() {
        heavy = found.pieces.iter().collect();
    }
    let round = |b: u64| (b / BLOCK as u64).max(1) * BLOCK as u64;
    let env = Env {
        dir,
        dev: z.dev,
        sets: &z.sets,
        groups,
        heavy,
        secs: z.secs,
        settle: z.settle,
        zones: &z.zones,
        limit: (!z.zones.is_empty()).then(|| stop_at(&z.zones)),
        cpus: cpus_online(),
        rec_cap: round(room * 3 / 7),
        copy_cap: round(room * 4 / 7),
    };
    say!(
        "files: the recording up to {:.0} MiB, the heavy write up to {:.0} MiB, each new for a step and deleted after it",
        mib(env.rec_cap),
        mib(env.copy_cap)
    );
    say!("processor: {} CPUs online", env.cpus);
    for zn in env.zones {
        let trips: Vec<String> = zn
            .trips
            .iter()
            .map(|(k, t)| format!("{k} {t:.0} °C"))
            .collect();
        say!(
            "  temperature: {} now {}; trip points: {}",
            zn.name,
            celsius(millidegrees(&zn.temp)),
            if trips.is_empty() {
                "none given".to_string()
            } else {
                trips.join(", ")
            }
        );
    }
    // Why the load steps are left out, once one is.
    let mut hot_note: Option<String> = match env.limit {
        Some(l) => {
            say!("  the busy and switching threads stop at {l:.0} °C");
            None
        }
        None if z.desktop => {
            say!("  no temperature to read (a desktop test: the load steps run unwatched)");
            None
        }
        None => {
            say!("  no temperature to read: the load steps (5 to 8) are left out");
            Some("no temperature to read".into())
        }
    };

    let mut heavy_next = 0usize;
    let mut rows = Vec::new();
    for (k, &(name, load)) in STEPS.iter().enumerate() {
        stopped()?;
        say!("\n{}. {name}, with the recording", k + 1);
        if let Some(why) = left_out(load, &hot_note, env.limit, env.zones) {
            say!("  left out: {why}");
            hot_note.get_or_insert_with(|| why.clone());
            rows.push(Row::LeftOut(why));
            continue;
        }
        let m = run_step(&env, load, &mut heavy_next);
        let removed = remove_step_files(dir);
        let m = m?;
        removed?;
        rows.push(report_step(&env, load, &m));
        if load.heats() && env.limit.is_some_and(|l| m.top_c >= l) {
            hot_note = Some(format!(
                "the MPC reached {:.1} °C in step {}",
                m.top_c,
                k + 1
            ));
        }
    }
    summary(env.sets, &rows);

    let (_, after) = read::find(base, z.piece).map_err(|e| match e {
        Fail::Refused(e) | Fail::Failed(e) => e,
    })?;
    if after == z.before {
        say!(
            "\nthe files read in {}/ and {}/ are as they were",
            read::LIBRARY,
            read::BACKUP
        );
    } else {
        say!(
            "\nfiles in {}/ or {}/ changed during the run (something else saved there); this tool only read them",
            read::LIBRARY,
            read::BACKUP
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A step that starts where the last one left off, past the last piece
    /// (step 3 read the last piece and stopped there), goes round to the
    /// first piece; only a whole pass that reads nothing is an error.
    /// Train 149's CI failed "the heavy read's files are empty now" this way.
    #[test]
    fn a_heavy_read_that_starts_past_the_last_piece_goes_round() {
        use std::os::unix::fs::MetadataExt;
        let dir =
            std::env::temp_dir().join(format!("mpfree-disktest-round-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bin");
        std::fs::write(&path, vec![7u8; 4096]).unwrap();
        let dev = std::fs::metadata(&path).unwrap().dev();
        let piece = Piece {
            path: path.clone(),
            start: 0,
            len: 4096,
        };
        let pieces = [&piece];
        let on = AtomicBool::new(true);
        let mut next = pieces.len();
        let r = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                on.store(false, Ordering::Relaxed);
            });
            heavy_reader(&pieces, dev, &mut next, &on)
        });
        std::fs::remove_dir_all(&dir).unwrap();
        let r = r.expect("a step starting past the last piece goes round");
        assert!(r.bytes >= 4096, "read {} bytes", r.bytes);
        // A pass over files that are all empty is still an error.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"").unwrap();
        let on = AtomicBool::new(true);
        let mut next = 0;
        let r = heavy_reader(&pieces, dev, &mut next, &on);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            r.err().as_deref(),
            Some("the heavy read's files are empty now")
        );
    }

    #[test]
    fn the_steps_match_the_design() {
        assert_eq!(STEPS.len(), 8);
        assert_eq!(STEPS[0].1, NONE);
        assert!(STEPS[3].1.heavy_write && STEPS[3].1.heavy_read && !STEPS[3].1.heats());
        assert!(STEPS[4].1.cpu && !STEPS[4].1.switch);
        assert!(STEPS[5].1.switch && !STEPS[5].1.cpu);
        assert!(
            STEPS[7].1.heavy_write && STEPS[7].1.heavy_read && STEPS[7].1.cpu && STEPS[7].1.switch
        );
        // Only steps 5 to 8 heat the processor, so only they are watched.
        let heats: Vec<bool> = STEPS.iter().map(|s| s.1.heats()).collect();
        assert_eq!(heats, [false, false, false, false, true, true, true, true]);
        let (sets, secs, _, _) = plan(false);
        assert_eq!((sets[0].streams, sets[0].read), (8, 128 * KIB));
        assert_eq!((sets[1].streams, sets[1].read), (24, 512 * KIB));
        assert_eq!(secs, 20.0);
    }

    fn zone(trips: &[f64]) -> Zone {
        Zone {
            name: "cpu".into(),
            temp: PathBuf::from("/nonexistent"),
            trips: trips.iter().map(|&t| ("passive".to_string(), t)).collect(),
        }
    }

    #[test]
    fn the_load_stops_at_75_or_the_lowest_real_trip_point() {
        assert_eq!(stop_at(&[]), 75.0);
        assert_eq!(stop_at(&[zone(&[90.0, 110.0])]), 75.0);
        assert_eq!(stop_at(&[zone(&[90.0]), zone(&[70.0, 85.0])]), 70.0);
        // A trip point under 40 °C is not a heat limit.
        assert_eq!(stop_at(&[zone(&[0.0, 30.0, 72.5])]), 72.5);
    }

    #[test]
    fn zones_read_from_sysfs() {
        let d = std::env::temp_dir().join(format!("mpfree-disktest-zones-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let z0 = d.join("thermal_zone0");
        let z1 = d.join("thermal_zone1");
        let z2 = d.join("thermal_zone2");
        for z in [&z0, &z1, &z2] {
            fs::create_dir_all(z).unwrap();
        }
        fs::write(z0.join("type"), "cpu-thermal\n").unwrap();
        fs::write(z0.join("temp"), "52500\n").unwrap();
        fs::write(z0.join("trip_point_0_temp"), "70000\n").unwrap();
        fs::write(z0.join("trip_point_0_type"), "passive\n").unwrap();
        fs::write(z0.join("trip_point_1_temp"), "90000\n").unwrap();
        fs::write(z0.join("trip_point_1_type"), "critical\n").unwrap();
        fs::write(z1.join("type"), "gpu-thermal\n").unwrap();
        fs::write(z1.join("temp"), "48000\n").unwrap();
        // No temperature: left out.
        fs::write(z2.join("type"), "broken\n").unwrap();
        fs::create_dir_all(d.join("cooling_device0")).unwrap();
        let zs = zones(&d);
        assert_eq!(zs.len(), 2);
        assert_eq!(zs[0].name, "cpu-thermal");
        assert_eq!(
            zs[0].trips,
            vec![
                ("passive".to_string(), 70.0),
                ("critical".to_string(), 90.0)
            ]
        );
        assert_eq!(hottest(&zs), Some(52.5));
        assert_eq!(stop_at(&zs), 70.0);
        assert!(zones(&d.join("missing")).is_empty());
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_watch_stops_the_load_at_the_limit() {
        let d = std::env::temp_dir().join(format!("mpfree-disktest-hot-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let z = d.join("thermal_zone0");
        fs::create_dir_all(&z).unwrap();
        fs::write(z.join("temp"), "71000\n").unwrap();
        fs::write(z.join("trip_point_0_temp"), "70000\n").unwrap();
        let zs = zones(&d);
        let (on, hot) = (AtomicBool::new(true), AtomicBool::new(false));
        std::thread::scope(|s| {
            let h = s.spawn(|| probe(&on, &hot, &zs, Some(stop_at(&zs))));
            // The first reading comes at once.
            let t = Instant::now();
            while !hot.load(Ordering::Relaxed) && t.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(5));
            }
            on.store(false, Ordering::Relaxed);
            let (late, top) = h.join().unwrap();
            assert_eq!(top, 71.0);
            assert!(!late.ms.is_empty());
        });
        assert!(hot.load(Ordering::Relaxed));
        // A busy thread told it is hot does no work; under the limit the
        // watch leaves it alone.
        assert_eq!(hog(0, &AtomicBool::new(true), &hot).0, 0);
        fs::write(z.join("temp"), "69000\n").unwrap();
        let (on, cool) = (AtomicBool::new(true), AtomicBool::new(false));
        std::thread::scope(|s| {
            let h = s.spawn(|| probe(&on, &cool, &zs, Some(70.0)));
            std::thread::sleep(Duration::from_millis(50));
            on.store(false, Ordering::Relaxed);
            h.join().unwrap();
        });
        assert!(!cool.load(Ordering::Relaxed));
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn proc_stat_gives_busy_shares_and_switches() {
        let a = parse_stat(
            "cpu  10 0 10 80 0 0 0 0 0 0\ncpu0 5 0 5 40 0 0 0 0 0 0\ncpu1 5 0 5 40 0 0 0 0 0 0\nintr 1 2\nctxt 1000\nbtime 1\n",
        )
        .unwrap();
        let b = parse_stat(
            "cpu  110 0 10 80 0 0 0 0 0 0\ncpu0 105 0 5 40 0 0 0 0 0 0\ncpu1 5 0 5 130 10 0 0 0 0 0\nctxt 3000\n",
        )
        .unwrap();
        assert_eq!(a.cpus, vec![(10, 0, 50), (10, 0, 50)]);
        assert_eq!(b.cpus[1], (10, 10, 150));
        let (text, pct, per_s) = load_line(&a, &b, 2.0).unwrap();
        assert_eq!(
            text,
            "CPUs busy 100/0 %, waiting on a disk 0/10 %; 1,000 task switches a second"
        );
        assert!((pct - 50.0).abs() < 1e-9);
        assert_eq!(per_s, 1000.0);
        assert!(parse_stat("cpu0 1 2\nctxt 5\n").is_none());
        assert!(parse_stat("cpu0 1 2 3 4 5 6 7\n").is_none());
    }

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
