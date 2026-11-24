//! The open's own line (live shows A4): how long an open took, part by
//! part, and the memory it left, so the device round measures real songs
//! with the app as it is ([live shows](../../../docs/dev/design/live-shows.md#measuring-opens-a4)).
//!
//! An open's parts go on side by side on different threads, so after the
//! read each is said as the time it was done, counted from the read's
//! answer:
//!
//! - **read**: the library's read of the project's parts and the check of
//!   its files (the browser's worker), before the clock starts;
//! - **built**: the screen thread reads the parts and builds what is
//!   costly (the time engine, the mixer's effects, the instruments),
//!   starting the plugins' processes, whose share is said too;
//! - **in the audio loop**: the engines are swapped in and the old ones
//!   are back;
//! - **kits**: their sounds read and decoded (the job service's file lane);
//! - **loops** and **takes**: read on threads of their own.
//!
//! The sound files the kits and the loops read are counted as they are
//! read (files, bytes, and the time in the reads alone), so the stick's
//! share is told apart from decoding.
//!
//! The memory is read once, on the screen thread, when the open is in:
//! the process's resident memory and its peak (`/proc/self/status`) and
//! the memory free (`/proc/meminfo`'s MemAvailable). Never on the audio
//! loop.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The sound files an open read, counted by the threads that read them.
#[derive(Debug, Default)]
pub struct Reads {
    files: AtomicU64,
    bytes: AtomicU64,
    micros: AtomicU64,
}

impl Reads {
    /// Reads `path` whole, as `std::fs::read`, counting it when it reads.
    pub fn read(&self, path: &std::path::Path) -> std::io::Result<Vec<u8>> {
        let at = Instant::now();
        let r = std::fs::read(path);
        if let Ok(b) = &r {
            self.files.fetch_add(1, Ordering::Relaxed);
            self.bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
            self.micros
                .fetch_add(at.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        r
    }

    /// The files read so far, their bytes and the time spent in the reads.
    pub fn so_far(&self) -> (u64, u64, Duration) {
        (
            self.files.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
            Duration::from_micros(self.micros.load(Ordering::Relaxed)),
        )
    }
}

/// The parts of an open that land after it is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Engines,
    Kits,
    Loops,
    Takes,
}

impl Part {
    fn words(self) -> &'static str {
        match self {
            Part::Engines => "in the audio loop",
            Part::Kits => "kits",
            Part::Loops => "loops",
            Part::Takes => "takes",
        }
    }
}

/// The memory when an open is in, in kB; None where it can't be read (off
/// Linux).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory {
    pub resident_kb: Option<u64>,
    pub peak_kb: Option<u64>,
    pub free_kb: Option<u64>,
}

impl Memory {
    /// Read now from /proc (read-only).
    pub fn now() -> Memory {
        let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
        let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
        Memory::of(&status, &meminfo)
    }

    /// From the texts of `/proc/self/status` and `/proc/meminfo`.
    pub fn of(status: &str, meminfo: &str) -> Memory {
        use crate::perf::kb_field;
        Memory {
            resident_kb: kb_field(status, "VmRSS"),
            peak_kb: kb_field(status, "VmHWM"),
            free_kb: kb_field(meminfo, "MemAvailable"),
        }
    }

    /// "memory 212 MB in use (peak 236 MB), 1480 MB free", or what of it
    /// is known.
    pub fn words(&self) -> String {
        let mb = |kb: u64| (kb as f64 / 1024.0).round() as u64;
        let mut w = Vec::new();
        match (self.resident_kb, self.peak_kb) {
            (Some(r), Some(p)) => w.push(format!("{} MB in use (peak {} MB)", mb(r), mb(p))),
            (Some(r), None) => w.push(format!("{} MB in use", mb(r))),
            _ => {}
        }
        if let Some(f) = self.free_kb {
            w.push(format!("{} MB free", mb(f)));
        }
        if w.is_empty() {
            "memory not known here".to_string()
        } else {
            format!("memory {}", w.join(", "))
        }
    }
}

/// One open, from the read's answer until all of it is in.
#[derive(Debug)]
pub struct OpenClock {
    name: String,
    start: Instant,
    read: Option<Duration>,
    built: Option<(Duration, u64, Duration)>,
    /// The parts this open waits for, each with when it was done.
    waits: Vec<(Part, Option<Duration>)>,
    reads: std::sync::Arc<Reads>,
}

impl OpenClock {
    /// The project `name`'s read came at `start`; the library took `read`
    /// over it (None when it can't say).
    pub fn new(name: &str, start: Instant, read: Option<Duration>) -> OpenClock {
        OpenClock {
            name: name.to_string(),
            start,
            read,
            built: None,
            waits: Vec::new(),
            reads: Default::default(),
        }
    }

    /// The counter the threads reading its sound files add to.
    pub fn reads(&self) -> std::sync::Arc<Reads> {
        self.reads.clone()
    }

    /// It was built at `at`; `plugins` is how many plugin starts that took
    /// and their time (the rack's [`starts`] across the build).
    ///
    /// [`starts`]: mpfree_plughost::rack::Rack::starts
    pub fn built(&mut self, at: Instant, plugins: (u64, Duration)) {
        self.built = Some((
            at.saturating_duration_since(self.start),
            plugins.0,
            plugins.1,
        ));
    }

    /// The open has this part to wait for.
    pub fn wait_for(&mut self, part: Part) {
        if !self.waits.iter().any(|(p, _)| *p == part) {
            self.waits.push((part, None));
        }
    }

    /// `part` was done at `at` (the first time counts).
    pub fn done(&mut self, part: Part, at: Instant) {
        let since = at.saturating_duration_since(self.start);
        if let Some((_, t @ None)) = self.waits.iter_mut().find(|(p, _)| *p == part) {
            *t = Some(since);
        }
    }

    /// Built, and every part it waits for done.
    pub fn is_in(&self) -> bool {
        self.built.is_some() && self.waits.iter().all(|(_, t)| t.is_some())
    }

    /// The whole open: the read, then the last part done.
    pub fn took(&self) -> Duration {
        let last = self
            .waits
            .iter()
            .filter_map(|(_, t)| *t)
            .chain(self.built.map(|b| b.0))
            .max()
            .unwrap_or_default();
        self.read.unwrap_or_default() + last
    }

    /// The open's line for the log, with the memory once it is in.
    pub fn line(&self, memory: &Memory) -> String {
        let s = |d: Duration| format!("{:.2} s", d.as_secs_f64());
        let mut parts = Vec::new();
        if let Some(r) = self.read {
            parts.push(format!("read in {}", s(r)));
        }
        if let Some((b, n, p)) = self.built {
            parts.push(match n {
                0 => format!("built in {}", s(b)),
                1 => format!("built in {} (1 plugin started in {})", s(b), s(p)),
                n => format!("built in {} ({n} plugins started in {})", s(b), s(p)),
            });
        }
        for (part, t) in &self.waits {
            if let Some(t) = t {
                parts.push(format!("{} at {}", part.words(), s(*t)));
            }
        }
        let (files, bytes, took) = self.reads.so_far();
        let files = match files {
            0 => String::new(),
            1 => format!(
                "; 1 sound file ({:.1} MB) read in {}",
                bytes as f64 / 1_048_576.0,
                s(took)
            ),
            n => format!(
                "; {n} sound files ({:.1} MB) read in {}",
                bytes as f64 / 1_048_576.0,
                s(took)
            ),
        };
        format!(
            "project: {} opened in {}: {}{files}; {}",
            self.name,
            s(self.took()),
            parts.join(", "),
            memory.words()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn an_open_says_each_part_as_it_was_done_and_the_memory_it_left() {
        let t0 = Instant::now();
        let mut c = OpenClock::new("Night Market", t0, Some(50 * MS));
        c.wait_for(Part::Engines);
        c.wait_for(Part::Kits);
        c.wait_for(Part::Loops);
        c.wait_for(Part::Kits);
        assert!(!c.is_in(), "not built yet");
        c.built(t0 + 620 * MS, (2, 400 * MS));
        c.done(Part::Engines, t0 + 670 * MS);
        c.done(Part::Loops, t0 + 700 * MS);
        assert!(!c.is_in(), "the kits are still coming");
        // A part it doesn't wait for, and a second word for one done,
        // change nothing.
        c.done(Part::Takes, t0 + 900 * MS);
        c.done(Part::Loops, t0 + 2_000 * MS);
        c.done(Part::Kits, t0 + 2_360 * MS);
        assert!(c.is_in());
        assert_eq!(c.took(), 2_410 * MS);
        let reads = c.reads();
        let dir = std::env::temp_dir().join(format!("mpfree-open-clock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.wav"), vec![0u8; 1_048_576]).unwrap();
        std::fs::write(dir.join("b.wav"), vec![0u8; 524_288]).unwrap();
        assert!(reads.read(&dir.join("a.wav")).is_ok());
        assert!(reads.read(&dir.join("b.wav")).is_ok());
        assert!(reads.read(&dir.join("gone.wav")).is_err());
        let (files, bytes, _) = reads.so_far();
        assert_eq!(
            (files, bytes),
            (2, 1_572_864),
            "a file that isn't there isn't counted"
        );
        let mem = Memory::of(
            "Name:\tmpfree\nVmHWM:\t  241664 kB\nVmRSS:\t  217088 kB\n",
            "MemTotal: 2067188 kB\nMemAvailable: 1515520 kB\n",
        );
        let line = c.line(&mem);
        let (head, said_reads) = line
            .split_once("; 2 sound files (1.5 MB) read in ")
            .unwrap();
        assert_eq!(
            head,
            "project: Night Market opened in 2.41 s: read in 0.05 s, \
             built in 0.62 s (2 plugins started in 0.40 s), in the audio loop at 0.67 s, \
             kits at 2.36 s, loops at 0.70 s"
        );
        assert!(
            said_reads.ends_with(" s; memory 212 MB in use (peak 236 MB), 1480 MB free"),
            "{said_reads}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plain_open_says_only_what_it_had() {
        let t0 = Instant::now();
        let mut c = OpenClock::new("Song", t0, None);
        c.wait_for(Part::Engines);
        c.built(t0 + 10 * MS, (1, 6 * MS));
        c.done(Part::Engines, t0 + 30 * MS);
        assert!(c.is_in());
        assert_eq!(
            c.line(&Memory::default()),
            "project: Song opened in 0.03 s: built in 0.01 s (1 plugin started in 0.01 s), \
             in the audio loop at 0.03 s; memory not known here"
        );
        let mut c = OpenClock::new("Song", t0, Some(MS));
        c.built(t0 + 10 * MS, (0, Duration::ZERO));
        assert_eq!(
            c.line(&Memory {
                resident_kb: Some(102_400),
                ..Memory::default()
            }),
            "project: Song opened in 0.01 s: read in 0.00 s, built in 0.01 s; memory 100 MB in use"
        );
    }

    #[test]
    fn the_memory_reads_here_on_linux() {
        let m = Memory::now();
        if cfg!(target_os = "linux") {
            assert!(m.resident_kb.is_some() && m.peak_kb.is_some() && m.free_kb.is_some());
            assert!(m.peak_kb >= m.resident_kb);
        }
    }
}
