//! The MPC's temperature and processor clock, for System status (phase 3):
//! a few small files in /sys, read every 2 s on the screen thread whatever
//! shows, never by the audio loop. Read-only.
//!
//! - **Temperature:** each `/sys/class/thermal/thermal_zone*/temp` (milli
//!   °C); the hottest zone counts. A zone's `critical` trip point is where
//!   the kernel turns the MPC off. On the MPC One (seen on 23 Nov, the disk
//!   streaming round D2b) there are two zones, `cpu-thermal` and
//!   `gpu-thermal`, each with one trip point, critical at 113 °C; they read
//!   55.5 °C at the start and 62.5 °C at the most after 3 minutes of every
//!   core busy.
//! - **The processor's clock:** each core's `cpufreq/scaling_cur_freq` and
//!   its ceiling, `cpufreq/scaling_max_freq` (kHz). The processor is
//!   slowed when a core's ceiling falls below where it was at the first
//!   read: that is what the kernel's cooling or a governor does to slow it
//!   down. A clock under its ceiling is a governor resting an idle core,
//!   not a problem, so it isn't one here. On the MPC the cores stayed at
//!   1608 MHz under every load (D2b).
//!
//! The files are found at the first read (one look in each folder); later
//! reads open only them. [`HeatReader::cost_line`] says how long a read
//! takes, so the MPC's own numbers say what this costs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The temperature over which the MPC is running hot, °C, measured on the
/// MPC (the plan: never guessed). With every core busy until it settled
/// (`mpfree-disktest --heat`, the disk streaming rounds D3 and D4 of 24 and
/// 25 Nov 2026) Jay's MPC settled at 65.4 and 67.7 °C (70.8 at the most)
/// at 1608 MHz throughout; the line is a few degrees over that, Jay's pick
/// of 25 Nov, so a heavy project never sets it off.
pub const RUNNING_HOT_C: Option<f32> = Some(73.0);

/// What one read found. A part the MPC doesn't have is None.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Heat {
    /// The hottest zone, °C.
    pub temp_c: Option<f32>,
    /// The lowest critical trip point of any zone, °C: where the kernel
    /// turns the MPC off.
    pub critical_c: Option<f32>,
    /// The slowest core's clock now, MHz.
    pub clock_mhz: Option<u32>,
    /// The lowest core ceiling now, MHz.
    pub top_mhz: Option<u32>,
    /// The lowest core ceiling at the first read, MHz: full speed.
    pub full_mhz: Option<u32>,
    /// A core's ceiling is below where it was at the first read.
    pub slowed: Option<bool>,
}

/// One core's files, and its ceiling at the first read.
#[derive(Debug)]
struct Core {
    cur: PathBuf,
    max: PathBuf,
    full: Option<u32>,
}

/// Reads the files every time it is asked (the screen thread asks every
/// 2 s).
#[derive(Debug)]
pub struct HeatReader {
    sys: PathBuf,
    found: bool,
    zones: Vec<PathBuf>,
    critical_c: Option<f32>,
    cores: Vec<Core>,
    reads: u64,
    slowest: Duration,
}

fn number(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// A milli-degree reading in °C. Some kernels give whole degrees: a value
/// under 1000 is taken as °C itself.
fn celsius(v: i64) -> f32 {
    if v.abs() < 1000 {
        v as f32
    } else {
        v as f32 / 1000.0
    }
}

impl Default for HeatReader {
    fn default() -> Self {
        HeatReader::at("/sys")
    }
}

impl HeatReader {
    /// A reader of the files under `sys` (`/sys` in the app, a fixture tree
    /// in the tests).
    pub fn at(sys: impl Into<PathBuf>) -> HeatReader {
        HeatReader {
            sys: sys.into(),
            found: false,
            zones: Vec::new(),
            critical_c: None,
            cores: Vec::new(),
            reads: 0,
            slowest: Duration::ZERO,
        }
    }

    /// The first read's look for the files, and each zone's critical trip.
    fn find(&mut self) {
        self.found = true;
        let thermal = self.sys.join("class/thermal");
        let mut zones: Vec<PathBuf> = fs::read_dir(&thermal)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("thermal_zone"))
            })
            .map(|e| e.path())
            .collect();
        zones.sort();
        for z in &zones {
            for i in 0..16 {
                let kind = z.join(format!("trip_point_{i}_type"));
                let Ok(kind) = fs::read_to_string(&kind) else {
                    break;
                };
                if kind.trim() != "critical" {
                    continue;
                }
                if let Some(v) = number(&z.join(format!("trip_point_{i}_temp"))) {
                    let c = celsius(v);
                    // A trip at or under 0 °C is a placeholder, not a trip.
                    if c > 0.0 {
                        self.critical_c = Some(self.critical_c.map_or(c, |m| m.min(c)));
                    }
                }
            }
        }
        self.zones = zones
            .into_iter()
            .map(|z| z.join("temp"))
            .filter(|t| t.exists())
            .collect();
        let cpus = self.sys.join("devices/system/cpu");
        let mut cores: Vec<(u32, PathBuf)> = fs::read_dir(&cpus)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let n: u32 = e.file_name().to_str()?.strip_prefix("cpu")?.parse().ok()?;
                Some((n, e.path().join("cpufreq")))
            })
            .filter(|(_, f)| f.join("scaling_cur_freq").exists())
            .collect();
        cores.sort();
        self.cores = cores
            .into_iter()
            .map(|(_, f)| Core {
                cur: f.join("scaling_cur_freq"),
                max: f.join("scaling_max_freq"),
                full: None,
            })
            .collect();
    }

    /// Reads the files now.
    pub fn read(&mut self) -> Heat {
        let started = Instant::now();
        if !self.found {
            self.find();
        }
        let temp_c = self
            .zones
            .iter()
            .filter_map(|z| number(z).map(celsius))
            .reduce(f32::max);
        let (mut clock, mut top, mut full, mut slowed) = (None, None, None, None);
        let min = |a: Option<u32>, b: u32| Some(a.map_or(b, |a: u32| a.min(b)));
        for c in &mut self.cores {
            // A core taken offline has no files for now: it's left out.
            if let Some(khz) = number(&c.cur) {
                clock = min(clock, (khz / 1000) as u32);
            }
            let Some(ceiling) = number(&c.max).map(|k| (k / 1000) as u32) else {
                continue;
            };
            let at_start = *c.full.get_or_insert(ceiling);
            top = min(top, ceiling);
            full = min(full, at_start);
            slowed = Some(slowed.unwrap_or(false) || ceiling < at_start);
        }
        self.reads += 1;
        self.slowest = self.slowest.max(started.elapsed());
        Heat {
            temp_c,
            critical_c: self.critical_c,
            clock_mhz: clock,
            top_mhz: top,
            full_mhz: full,
            slowed,
        }
    }

    /// How many files a read opens.
    pub fn files(&self) -> usize {
        self.zones.len() + 2 * self.cores.len()
    }

    /// A log line saying what the reads cost, after the first minute of
    /// reads (30) and then every half hour (900), so the MPC's own run says
    /// it: "status: the MPC's temperature and clock: 10 files every 2 s,
    /// slowest read 85 µs (30 reads)".
    pub fn cost_line(&self) -> Option<String> {
        if self.reads != 30 && (self.reads == 0 || !self.reads.is_multiple_of(900)) {
            return None;
        }
        Some(format!(
            "status: the MPC's temperature and clock: {} files every 2 s, slowest read {} µs \
             ({} reads)",
            self.files(),
            self.slowest.as_micros(),
            self.reads
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let root =
                std::env::temp_dir().join(format!("mpfree-heat-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            Tree(root)
        }
        fn put(&self, path: &str, text: &str) {
            let p = self.0.join(path);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, text).unwrap();
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The MPC One's tree as the D2b round saw it: two zones with a
    /// critical trip at 113 °C, four cores at 1608 MHz.
    fn mpc(name: &str) -> Tree {
        let t = Tree::new(name);
        for (z, kind, milli) in [(0, "cpu-thermal", "55500"), (1, "gpu-thermal", "53000")] {
            t.put(&format!("class/thermal/thermal_zone{z}/type"), kind);
            t.put(&format!("class/thermal/thermal_zone{z}/temp"), milli);
            t.put(
                &format!("class/thermal/thermal_zone{z}/trip_point_0_type"),
                "critical\n",
            );
            t.put(
                &format!("class/thermal/thermal_zone{z}/trip_point_0_temp"),
                "113000\n",
            );
        }
        // Not a zone: left alone.
        t.put("class/thermal/cooling_device0/type", "thermal-cpufreq-0");
        for c in 0..4 {
            t.put(
                &format!("devices/system/cpu/cpu{c}/cpufreq/scaling_cur_freq"),
                "1608000\n",
            );
            t.put(
                &format!("devices/system/cpu/cpu{c}/cpufreq/scaling_max_freq"),
                "1608000\n",
            );
        }
        // Not a core.
        t.put("devices/system/cpu/cpufreq/boost", "0");
        t
    }

    #[test]
    fn the_mpc_reads_its_hottest_zone_its_trip_and_its_clock() {
        let t = mpc("reads");
        let mut r = HeatReader::at(&t.0);
        let h = r.read();
        assert_eq!(h.temp_c, Some(55.5));
        assert_eq!(h.critical_c, Some(113.0));
        assert_eq!(h.clock_mhz, Some(1608));
        assert_eq!(
            (h.top_mhz, h.full_mhz, h.slowed),
            (Some(1608), Some(1608), Some(false))
        );
        assert_eq!(r.files(), 2 + 8);
        // The GPU hotter: it counts.
        t.put("class/thermal/thermal_zone1/temp", "71250\n");
        assert_eq!(r.read().temp_c, Some(71.25));
    }

    #[test]
    fn a_lower_ceiling_is_slowed_and_a_resting_core_is_not() {
        let t = mpc("slowed");
        let mut r = HeatReader::at(&t.0);
        assert_eq!(r.read().slowed, Some(false));
        // A governor resting core 2: its clock is down, its ceiling isn't.
        t.put(
            "devices/system/cpu/cpu2/cpufreq/scaling_cur_freq",
            "600000\n",
        );
        let h = r.read();
        assert_eq!((h.clock_mhz, h.slowed), (Some(600), Some(false)));
        // The kernel's cooling turns core 3's ceiling down.
        t.put(
            "devices/system/cpu/cpu3/cpufreq/scaling_max_freq",
            "1200000\n",
        );
        t.put(
            "devices/system/cpu/cpu3/cpufreq/scaling_cur_freq",
            "1200000\n",
        );
        let h = r.read();
        assert_eq!(
            (h.top_mhz, h.full_mhz, h.slowed),
            (Some(1200), Some(1608), Some(true))
        );
        // Back up: not slowed.
        t.put(
            "devices/system/cpu/cpu3/cpufreq/scaling_max_freq",
            "1608000\n",
        );
        assert_eq!(r.read().slowed, Some(false));
    }

    #[test]
    fn a_machine_without_the_files_reads_nothing() {
        let t = Tree::new("none");
        fs::create_dir_all(&t.0).unwrap();
        let mut r = HeatReader::at(&t.0);
        assert_eq!(r.read(), Heat::default());
        assert_eq!(r.files(), 0);
        // A zone with no trip and whole degrees: read, no critical.
        t.put("class/thermal/thermal_zone0/temp", "48\n");
        let mut r = HeatReader::at(&t.0);
        let h = r.read();
        assert_eq!((h.temp_c, h.critical_c), (Some(48.0), None));
    }

    #[test]
    fn the_cost_is_said_after_a_minute_then_every_half_hour() {
        let t = mpc("cost");
        let mut r = HeatReader::at(&t.0);
        let mut said = Vec::new();
        for _ in 0..1800 {
            r.read();
            if let Some(l) = r.cost_line() {
                said.push(l);
            }
        }
        assert_eq!(said.len(), 3, "{said:?}");
        assert!(said[0].starts_with(
            "status: the MPC's temperature and clock: 10 files every 2 s, slowest read "
        ));
        assert!(said[0].ends_with("µs (30 reads)"), "{}", said[0]);
        assert!(said[2].ends_with("(1800 reads)"), "{}", said[2]);
    }
}
