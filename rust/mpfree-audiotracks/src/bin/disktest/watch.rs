//! What the rest of the stick did while the streams read: each partition's
//! reads and writes, from the kernel's block counters, and which of them
//! were going on during each slow read (one over [`SLOW_MS`]).
//!
//! D1 on the MPC found slow reads (up to 825 ms) with nothing writing on
//! partition 4. The stick has one queue (USB 2.0, queue depth 1), so a
//! write to the log partition (p2) or a read of the system (p1) waits in
//! the same line as the streams. This tells the two causes apart: a slow
//! read beside another partition's read or write, or with nothing else on
//! the stick at all (the stick's own housekeeping, or the bus).
//!
//! A thread reads each partition's `stat` and `inflight` in sysfs every
//! [`PERIOD`] (read-only, files it keeps open and reads again from the
//! start), from just before the streams start to just after they stop.
//! Partition 4's own reads are the streams', so only its writes count as
//! something else.

use std::fs::{self, File};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// A read slower than this is a slow read.
pub(super) const SLOW_MS: f64 = 100.0;
/// How often the counters are read.
const PERIOD: Duration = Duration::from_millis(10);
/// The partition the streams read (and the recorder writes).
const DATA: u32 = 4;

/// The stick being watched, found once at the start.
pub(super) static STICK: OnceLock<Stick> = OnceLock::new();

/// A disk and its partitions, as sysfs has them.
#[derive(Clone, Debug)]
pub(super) struct Stick {
    pub(super) disk: String,
    /// The disk's folder in sysfs.
    pub(super) dir: PathBuf,
    /// Each partition's number and its folder in sysfs, by number.
    pub(super) parts: Vec<(u32, PathBuf)>,
}

/// The disk `part` (a partition's folder in sysfs, as
/// `/sys/class/block/NAME` leads to it) is on, with every partition of it.
pub(super) fn stick_of(part: &Path) -> Result<Stick, String> {
    let part = fs::canonicalize(part).map_err(|e| format!("{}: {e}", part.display()))?;
    let disk = part
        .parent()
        .ok_or_else(|| format!("{} has no disk above it", part.display()))?;
    let mut parts = Vec::new();
    for e in fs::read_dir(disk)
        .map_err(|e| format!("{}: {e}", disk.display()))?
        .flatten()
    {
        let p = e.path();
        if let Ok(n) = fs::read_to_string(p.join("partition")) {
            if let Ok(n) = n.trim().parse::<u32>() {
                parts.push((n, p));
            }
        }
    }
    parts.sort();
    if !parts.iter().any(|(n, _)| *n == DATA) {
        return Err(format!("no partition {DATA} under {}", disk.display()));
    }
    Ok(Stick {
        disk: disk
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        dir: disk.to_path_buf(),
        parts,
    })
}

/// One partition's counters at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Counts {
    pub(super) reads: u64,
    pub(super) read_sectors: u64,
    pub(super) writes: u64,
    pub(super) write_sectors: u64,
    pub(super) reading: u64,
    pub(super) writing: u64,
}

/// `stat`'s fields 1, 3, 5 and 7 and `inflight`'s two (the kernel's
/// Documentation/block/stat.rst).
fn parse(stat: &str, inflight: &str) -> Option<Counts> {
    let s: Vec<u64> = stat
        .split_whitespace()
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    let i: Vec<u64> = inflight
        .split_whitespace()
        .map(|f| f.parse().ok())
        .collect::<Option<_>>()?;
    if s.len() < 7 || i.len() < 2 {
        return None;
    }
    Some(Counts {
        reads: s[0],
        read_sectors: s[2],
        writes: s[4],
        write_sectors: s[6],
        reading: i[0],
        writing: i[1],
    })
}

/// Every partition's counters at one moment, in the stick's order.
#[derive(Clone, Debug)]
pub(super) struct Sample {
    pub(super) at: Instant,
    pub(super) parts: Vec<Counts>,
}

/// The thread reading the counters, from [`Watcher::start`] to
/// [`Watcher::stop`].
pub(super) struct Watcher {
    stop: Arc<AtomicBool>,
    handle: JoinHandle<Vec<Sample>>,
    stick: Stick,
}

impl Watcher {
    /// Starts watching `stick`; `None` if its counters can't be opened or
    /// the thread can't start (the run goes on without them).
    pub(super) fn start(stick: &Stick) -> Option<Watcher> {
        let mut files = Vec::new();
        for (_, p) in &stick.parts {
            files.push((
                File::open(p.join("stat")).ok()?,
                File::open(p.join("inflight")).ok()?,
            ));
        }
        let read = move |buf: &mut [u8]| -> Option<Vec<Counts>> {
            let mut out = Vec::with_capacity(files.len());
            for (stat, inflight) in &files {
                let n = stat.read_at(buf, 0).ok()?;
                let s = String::from_utf8_lossy(&buf[..n]).into_owned();
                let n = inflight.read_at(buf, 0).ok()?;
                out.push(parse(&s, &String::from_utf8_lossy(&buf[..n]))?);
            }
            Some(out)
        };
        let mut buf = vec![0u8; 512];
        let first = read(&mut buf)?;
        let stop = Arc::new(AtomicBool::new(false));
        let on = stop.clone();
        let handle = std::thread::Builder::new()
            .name("disktest-watch".into())
            .spawn(move || {
                let mut samples = vec![Sample {
                    at: Instant::now(),
                    parts: first,
                }];
                loop {
                    let last = on.load(Ordering::Relaxed);
                    if let Some(parts) = read(&mut buf) {
                        samples.push(Sample {
                            at: Instant::now(),
                            parts,
                        });
                    }
                    if last {
                        return samples;
                    }
                    std::thread::sleep(PERIOD);
                }
            })
            .ok()?;
        Some(Watcher {
            stop,
            handle,
            stick: stick.clone(),
        })
    }

    /// Stops it (after one more reading) and says what the stick did.
    pub(super) fn stop(self, slow: &[(Instant, f64)]) -> Option<Beside> {
        self.stop.store(true, Ordering::Relaxed);
        let samples = self.handle.join().ok()?;
        Some(beside(&self.stick, &samples, slow))
    }
}

/// What the rest of the stick did over one set of streams.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Beside {
    pub(super) disk: String,
    /// Each partition's number, reads, KiB read, writes and KiB written
    /// over the set.
    pub(super) parts: Vec<(u32, u64, u64, u64, u64)>,
    /// The reads slower than [`SLOW_MS`].
    pub(super) slow: usize,
    /// Of those, how many had a read (another partition's) or a write
    /// going on or finishing on each partition, in `parts`' order.
    pub(super) slow_reading: Vec<usize>,
    pub(super) slow_writing: Vec<usize>,
    /// Slow reads with nothing else on the stick.
    pub(super) slow_alone: usize,
    /// How many times the counters were read.
    pub(super) samples: usize,
}

/// The last sample at or before `t` (the first if none), and the first at
/// or after `u` (the last if none).
fn bracket(samples: &[Sample], t: Instant, u: Instant) -> (usize, usize) {
    let lo = samples.partition_point(|s| s.at <= t).saturating_sub(1);
    let hi = samples
        .partition_point(|s| s.at < u)
        .min(samples.len().saturating_sub(1));
    (lo, hi.max(lo))
}

pub(super) fn beside(stick: &Stick, samples: &[Sample], slow: &[(Instant, f64)]) -> Beside {
    let n = stick.parts.len();
    let mut b = Beside {
        disk: stick.disk.clone(),
        slow: slow.len(),
        slow_reading: vec![0; n],
        slow_writing: vec![0; n],
        samples: samples.len(),
        ..Beside::default()
    };
    let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
        return b;
    };
    for (k, (num, _)) in stick.parts.iter().enumerate() {
        let (a, z) = (first.parts[k], last.parts[k]);
        b.parts.push((
            *num,
            z.reads.saturating_sub(a.reads),
            z.read_sectors.saturating_sub(a.read_sectors) / 2,
            z.writes.saturating_sub(a.writes),
            z.write_sectors.saturating_sub(a.write_sectors) / 2,
        ));
    }
    for &(at, ms) in slow {
        let end = at + Duration::from_secs_f64(ms / 1e3);
        let (lo, hi) = bracket(samples, at, end);
        let mut alone = true;
        for (k, (num, _)) in stick.parts.iter().enumerate() {
            let (a, z) = (samples[lo].parts[k], samples[hi].parts[k]);
            let during = &samples[lo..=hi];
            let wrote = z.writes > a.writes || during.iter().any(|s| s.parts[k].writing > 0);
            // Partition 4's reads are the streams' own.
            let read = *num != DATA
                && (z.reads > a.reads || during.iter().any(|s| s.parts[k].reading > 0));
            if wrote {
                b.slow_writing[k] += 1;
            }
            if read {
                b.slow_reading[k] += 1;
            }
            alone &= !wrote && !read;
        }
        if alone {
            b.slow_alone += 1;
        }
    }
    b
}

fn amount(kib: u64) -> String {
    if kib >= 1024 {
        format!("{:.1} MiB", kib as f64 / 1024.0)
    } else {
        format!("{kib} KiB")
    }
}

impl Beside {
    /// What the other partitions did, one line: "the rest of the stick
    /// (sda): p1 read 120 KiB in 3 reads, p2 wrote 412 KiB in 37 writes,
    /// p4 wrote nothing; 2,511 readings".
    pub(super) fn line(&self) -> String {
        let mut said = Vec::new();
        for &(num, reads, rkib, writes, wkib) in &self.parts {
            let mut what = Vec::new();
            if num != DATA && reads > 0 {
                what.push(format!("read {} in {reads}", amount(rkib)));
            }
            if writes > 0 {
                what.push(format!("wrote {} in {writes} writes", amount(wkib)));
            }
            if num == DATA && what.is_empty() {
                what.push("wrote nothing".into());
            }
            if !what.is_empty() {
                said.push(format!("p{num} {}", what.join(", ")));
            }
        }
        if said.is_empty() {
            said.push("nothing".into());
        }
        format!(
            "the rest of the stick ({}): {}; {} readings of its counters",
            self.disk,
            said.join(", "),
            self.samples
        )
    }

    /// The slow reads and what went on beside them, one line.
    pub(super) fn slow_line(&self) -> String {
        if self.slow == 0 {
            return format!("reads over {SLOW_MS:.0} ms: none");
        }
        let mut with = Vec::new();
        for (k, &(num, ..)) in self.parts.iter().enumerate() {
            if self.slow_writing[k] > 0 {
                with.push(format!("p{num} writing {}", self.slow_writing[k]));
            }
            if self.slow_reading[k] > 0 {
                with.push(format!("p{num} reading {}", self.slow_reading[k]));
            }
        }
        format!(
            "reads over {SLOW_MS:.0} ms: {}; beside {}; with nothing else on the stick {}",
            self.slow,
            if with.is_empty() {
                "nothing".to_string()
            } else {
                with.join(", ")
            },
            self.slow_alone
        )
    }

    /// The summary's short form: "3 slow: p2 w 2, p1 r 1, alone 0".
    pub(super) fn short(&self) -> String {
        let mut with = Vec::new();
        for (k, &(num, ..)) in self.parts.iter().enumerate() {
            if self.slow_writing[k] > 0 {
                with.push(format!("p{num} w {}", self.slow_writing[k]));
            }
            if self.slow_reading[k] > 0 {
                with.push(format!("p{num} r {}", self.slow_reading[k]));
            }
        }
        with.push(format!("alone {}", self.slow_alone));
        format!("{} slow: {}", self.slow, with.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stick() -> Stick {
        Stick {
            disk: "sda".into(),
            dir: PathBuf::from("sda"),
            parts: [1, 2, 4]
                .iter()
                .map(|&n| (n, PathBuf::from(format!("sda{n}"))))
                .collect(),
        }
    }

    fn c(reads: u64, writes: u64, reading: u64, writing: u64) -> Counts {
        Counts {
            reads,
            read_sectors: reads * 8,
            writes,
            write_sectors: writes * 16,
            reading,
            writing,
        }
    }

    #[test]
    fn the_kernels_counters_are_read_by_field() {
        let stat = "    4100     12   80000   9000   37    2   592   400    1   9500  9400    0    0    0    0";
        assert_eq!(
            parse(stat, "1 0\n"),
            Some(Counts {
                reads: 4100,
                read_sectors: 80000,
                writes: 37,
                write_sectors: 592,
                reading: 1,
                writing: 0,
            })
        );
        assert_eq!(parse("1 2 3", "0 0"), None);
        assert_eq!(parse(stat, "0"), None);
        assert_eq!(parse("a b c d e f g", "0 0"), None);
    }

    #[test]
    fn each_slow_read_is_matched_with_what_went_on_beside_it() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        // p1, p2, p4 every 100 ms: p2 writes between 200 and 300 ms, p1
        // reads at 600 ms, p4 has a write in flight at 900 ms.
        let parts = |p1: Counts, p2: Counts, p4: Counts| vec![p1, p2, p4];
        let mut samples = Vec::new();
        for k in 0..12u64 {
            let p1 = if k >= 6 { c(1, 0, 0, 0) } else { c(0, 0, 0, 0) };
            let p2 = if k >= 3 { c(0, 5, 0, 0) } else { c(0, 0, 0, 0) };
            let p4 = c(100 * k, 0, 1, u64::from(k == 9));
            samples.push(Sample {
                at: at(100 * k),
                parts: parts(p1, p2, p4),
            });
        }
        let slow = [
            (at(210), 150.0), // p2's writes finish in 200..400
            (at(550), 120.0), // p1's read in 500..700
            (at(850), 110.0), // p4 writing at 900
            (at(1010), 50.0), // nothing beside it
        ];
        let b = beside(&stick(), &samples, &slow);
        assert_eq!(b.slow, 4);
        assert_eq!(b.slow_writing, vec![0, 1, 1]);
        // Partition 4's own reads (the streams') never count.
        assert_eq!(b.slow_reading, vec![1, 0, 0]);
        assert_eq!(b.slow_alone, 1);
        assert_eq!(
            b.parts,
            vec![(1, 1, 4, 0, 0), (2, 0, 0, 5, 40), (4, 1100, 4400, 0, 0)]
        );
        assert_eq!(b.samples, 12);
        assert_eq!(
            b.line(),
            "the rest of the stick (sda): p1 read 4 KiB in 1, p2 wrote 40 KiB in 5 writes, p4 wrote nothing; 12 readings of its counters"
        );
        assert_eq!(
            b.slow_line(),
            "reads over 100 ms: 4; beside p1 reading 1, p2 writing 1, p4 writing 1; with nothing else on the stick 1"
        );
        assert_eq!(b.short(), "4 slow: p1 r 1, p2 w 1, p4 w 1, alone 1");
    }

    #[test]
    fn a_slow_read_outside_the_samples_takes_the_nearest() {
        let t0 = Instant::now();
        let samples = vec![
            Sample {
                at: t0 + Duration::from_millis(10),
                parts: vec![c(0, 0, 0, 0), c(0, 0, 0, 0), c(0, 0, 0, 0)],
            },
            Sample {
                at: t0 + Duration::from_millis(20),
                parts: vec![c(0, 0, 0, 0), c(0, 2, 0, 0), c(0, 0, 0, 0)],
            },
        ];
        let b = beside(&stick(), &samples, &[(t0, 500.0)]);
        assert_eq!((b.slow, b.slow_writing[1], b.slow_alone), (1, 1, 0));
        let none = beside(&stick(), &[], &[(t0, 500.0)]);
        assert_eq!((none.slow, none.parts.len(), none.slow_alone), (1, 0, 0));
        assert_eq!(
            none.slow_line(),
            "reads over 100 ms: 1; beside nothing; with nothing else on the stick 0"
        );
    }

    #[test]
    fn the_stick_is_found_from_its_partition() {
        let d = std::env::temp_dir().join(format!("mpfree-disktest-watch-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let disk = d.join("usb1/1-1/block/sdb");
        for n in [1, 2, 4] {
            let p = disk.join(format!("sdb{n}"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("partition"), format!("{n}\n")).unwrap();
            fs::write(p.join("stat"), "1 0 8 0 2 0 16 0 0 0 0\n").unwrap();
            fs::write(p.join("inflight"), "0 0\n").unwrap();
        }
        fs::create_dir_all(disk.join("queue")).unwrap();
        let s = stick_of(&disk.join("sdb4")).unwrap();
        assert_eq!(s.disk, "sdb");
        assert_eq!(
            s.parts.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            [1, 2, 4]
        );
        let w = Watcher::start(&s).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        let b = w.stop(&[]).unwrap();
        assert!(b.samples >= 2, "{b:?}");
        assert_eq!(b.slow_line(), "reads over 100 ms: none");
        assert!(stick_of(&disk.join("sdb2").join("missing")).is_err());
        fs::remove_dir_all(disk.join("sdb4")).unwrap();
        assert!(stick_of(&disk.join("sdb2")).is_err(), "no partition 4");
        fs::remove_dir_all(&d).unwrap();
    }
}
