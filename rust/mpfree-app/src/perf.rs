//! Debug mode's process and system numbers: CPU per thread, memory, load,
//! temperature and clock, read from /proc and /sys (read-only) by the
//! screen thread, never by the audio loop.
//!
//! - CPU per thread: `/proc/self/task/<tid>/stat` (utime + stime, in clock
//!   ticks of 1/100 s: USER_HZ is 100 in the Linux ABI on ARM and x86) and
//!   its `comm` for the name. 100 % is one core.
//! - The process: `/proc/self/status` (VmRSS, VmHWM, Threads).
//! - The system: the `cpu` line of `/proc/stat` (100 % is every core),
//!   `/proc/meminfo`, `/proc/loadavg`, `/sys/class/thermal/thermal_zone*`
//!   and `/sys/devices/system/cpu/cpu*/cpufreq/scaling_cur_freq`, where
//!   they exist (on the MPC One they are assumed, not yet seen).
//!
//! Parsing is separate from reading, so the tests feed fixture text.

use std::fs;
use std::path::Path;
use std::time::Instant;

/// Clock ticks per second in /proc (USER_HZ).
const TICKS_PER_S: f64 = 100.0;

/// One thread's CPU time so far.
#[derive(Clone, Debug, PartialEq)]
pub struct ThreadTicks {
    pub tid: u32,
    pub name: String,
    pub ticks: u64,
}

/// utime + stime from a `stat` line. The name is in parentheses and may
/// hold spaces, so fields are counted after the last ')'.
pub fn stat_ticks(line: &str) -> Option<u64> {
    let rest = &line[line.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // After the name: state (field 3) ... utime is field 14, stime 15.
    let utime: u64 = f.get(11)?.parse().ok()?;
    let stime: u64 = f.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// A `kB` value from /proc/self/status or /proc/meminfo.
pub fn kb_field(text: &str, key: &str) -> Option<u64> {
    text.lines().find_map(|l| {
        let v = l.strip_prefix(key)?.strip_prefix(':')?;
        v.split_whitespace().next()?.parse().ok()
    })
}

/// (busy, total) ticks from the `cpu` line of /proc/stat, and the number of
/// cores (`cpuN` lines).
pub fn cpu_line(text: &str) -> Option<(u64, u64, usize)> {
    let mut lines = text.lines();
    let first = lines.next()?;
    let f: Vec<u64> = first
        .strip_prefix("cpu ")?
        .split_whitespace()
        .filter_map(|x| x.parse().ok())
        .collect();
    if f.len() < 4 {
        return None;
    }
    let total: u64 = f.iter().take(8).sum();
    // idle and iowait are not busy.
    let idle = f[3] + f.get(4).copied().unwrap_or(0);
    let cores = text
        .lines()
        .filter(|l| l.starts_with("cpu") && l.as_bytes().get(3).is_some_and(u8::is_ascii_digit))
        .count();
    Some((total - idle, total, cores.max(1)))
}

/// What the Performance page shows about the process and the system.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcView {
    /// Seconds the numbers were measured over (0 on the first sample).
    pub over_s: f64,
    /// Threads by CPU, busiest first: name, % of one core.
    pub threads: Vec<(String, f64)>,
    /// The whole process, % of one core.
    pub process_cpu: f64,
    /// The whole system, % of every core, and the number of cores.
    pub system_cpu: Option<f64>,
    pub cores: usize,
    pub rss_kb: Option<u64>,
    pub peak_rss_kb: Option<u64>,
    pub thread_count: usize,
    pub mem_total_kb: Option<u64>,
    pub mem_available_kb: Option<u64>,
    pub load: Option<String>,
    /// Each thermal zone: type, °C.
    pub temps: Vec<(String, f64)>,
    /// Each core's clock, MHz.
    pub mhz: Vec<u32>,
}

fn mb(kb: u64) -> String {
    format!("{:.1} MB", kb as f64 / 1024.0)
}

impl ProcView {
    /// The `perf:` line for the log.
    pub fn line(&self) -> String {
        let threads: Vec<String> = self
            .threads
            .iter()
            .filter(|(_, c)| *c >= 0.05)
            .take(6)
            .map(|(n, c)| format!("{n} {c:.1}"))
            .collect();
        let opt = |v: Option<u64>| v.map_or("-".to_string(), mb);
        format!("perf: cpu {:.1} % of a core ({}), system {} of {} cores, rss {} (peak {}), {} threads, \
                 memory available {} of {}, load {}, temp {}, clock {} MHz",
                self.process_cpu, if threads.is_empty() { "idle".to_string() } else { threads.join(", ") },
                self.system_cpu.map_or("-".to_string(), |c| format!("{c:.1} %")), self.cores,
                opt(self.rss_kb), opt(self.peak_rss_kb), self.thread_count, opt(self.mem_available_kb),
                opt(self.mem_total_kb), self.load.as_deref().unwrap_or("-"),
                if self.temps.is_empty() { "-".to_string() } else {
                    self.temps.iter().map(|(n, c)| format!("{n} {c:.1} C")).collect::<Vec<_>>().join(", ")
                },
                if self.mhz.is_empty() { "-".to_string() } else {
                    self.mhz.iter().map(|m| m.to_string()).collect::<Vec<_>>().join("/")
                })
    }

    /// Rows for the Process and system panel.
    pub fn rows(&self) -> Vec<(String, String)> {
        let opt = |v: Option<u64>| v.map_or("-".to_string(), mb);
        let mut rows = vec![
            (
                "MPFree, CPU".to_string(),
                format!("{:.1} % of a core", self.process_cpu),
            ),
            (
                "Memory in use · peak".to_string(),
                format!("{} · {}", opt(self.rss_kb), opt(self.peak_rss_kb)),
            ),
            ("Threads".to_string(), self.thread_count.to_string()),
            (
                "System CPU".to_string(),
                self.system_cpu.map_or("-".to_string(), |c| {
                    format!("{c:.1} % of {} cores", self.cores)
                }),
            ),
            (
                "Memory available".to_string(),
                format!(
                    "{} of {}",
                    opt(self.mem_available_kb),
                    opt(self.mem_total_kb)
                ),
            ),
            (
                "Load (1, 5, 15 min)".to_string(),
                self.load.clone().unwrap_or_else(|| "-".to_string()),
            ),
        ];
        for (name, c) in self.temps.iter().take(2) {
            rows.push((format!("Temperature, {name}"), format!("{c:.1} °C")));
        }
        if !self.mhz.is_empty() {
            rows.push((
                "Clock per core".to_string(),
                self.mhz
                    .iter()
                    .map(|m| m.to_string())
                    .collect::<Vec<_>>()
                    .join(" · ")
                    + " MHz",
            ));
        }
        rows
    }
}

/// The previous sample: when, each thread's ticks, and the system's busy
/// and total ticks.
struct Previous {
    at: Instant,
    threads: Vec<ThreadTicks>,
    system: Option<(u64, u64)>,
}

/// Where the files are (a fixture directory in tests).
pub struct Sampler {
    proc_self: String,
    proc: String,
    sys: String,
    last: Option<Previous>,
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    fs::read_to_string(path).ok()
}

impl Sampler {
    pub fn new() -> Sampler {
        Sampler::at("/proc/self", "/proc", "/sys")
    }

    pub fn at(proc_self: &str, proc: &str, sys: &str) -> Sampler {
        Sampler {
            proc_self: proc_self.to_string(),
            proc: proc.to_string(),
            sys: sys.to_string(),
            last: None,
        }
    }

    fn threads(&self) -> Vec<ThreadTicks> {
        let dir = format!("{}/task", self.proc_self);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut out: Vec<ThreadTicks> = entries
            .filter_map(|e| {
                let e = e.ok()?;
                let tid: u32 = e.file_name().to_str()?.parse().ok()?;
                let ticks = stat_ticks(&read(e.path().join("stat"))?)?;
                let name = read(e.path().join("comm"))
                    .map(|n| n.trim().to_string())
                    .unwrap_or_else(|| tid.to_string());
                Some(ThreadTicks { tid, name, ticks })
            })
            .collect();
        out.sort_by_key(|t| t.tid);
        out
    }

    fn temps(&self) -> Vec<(String, f64)> {
        let Ok(entries) = fs::read_dir(format!("{}/class/thermal", self.sys)) else {
            return Vec::new();
        };
        let mut zones: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with("thermal_zone"))
            })
            .collect();
        zones.sort_by_key(|e| e.file_name());
        zones
            .iter()
            .filter_map(|e| {
                let milli: f64 = read(e.path().join("temp"))?.trim().parse().ok()?;
                let name = read(e.path().join("type"))
                    .map(|t| t.trim().to_string())
                    .unwrap_or_else(|| "zone".to_string());
                Some((name, milli / 1000.0))
            })
            .collect()
    }

    fn clocks(&self) -> Vec<u32> {
        (0..8)
            .map_while(|i| {
                let khz: u32 = read(format!(
                    "{}/devices/system/cpu/cpu{i}/cpufreq/scaling_cur_freq",
                    self.sys
                ))?
                .trim()
                .parse()
                .ok()?;
                Some(khz / 1000)
            })
            .collect()
    }

    /// Read everything once. CPU figures are since the last call (none on
    /// the first).
    pub fn sample(&mut self) -> ProcView {
        let now = Instant::now();
        let threads = self.threads();
        let status = read(format!("{}/status", self.proc_self)).unwrap_or_default();
        let stat = read(format!("{}/stat", self.proc)).and_then(|t| cpu_line(&t));
        let meminfo = read(format!("{}/meminfo", self.proc)).unwrap_or_default();
        let mut v = ProcView {
            rss_kb: kb_field(&status, "VmRSS"),
            peak_rss_kb: kb_field(&status, "VmHWM"),
            thread_count: threads.len(),
            mem_total_kb: kb_field(&meminfo, "MemTotal"),
            mem_available_kb: kb_field(&meminfo, "MemAvailable"),
            load: read(format!("{}/loadavg", self.proc))
                .map(|l| l.split_whitespace().take(3).collect::<Vec<_>>().join(" ")),
            temps: self.temps(),
            mhz: self.clocks(),
            cores: stat.map_or(1, |s| s.2),
            ..ProcView::default()
        };
        if let Some(Previous {
            at,
            threads: before,
            system: sys_before,
        }) = &self.last
        {
            let dt = now.duration_since(*at).as_secs_f64().max(1e-3);
            v.over_s = dt;
            let mut list: Vec<(String, f64)> = threads
                .iter()
                .map(|t| {
                    let was = before
                        .iter()
                        .find(|b| b.tid == t.tid)
                        .map_or(t.ticks, |b| b.ticks);
                    (
                        format!("{} ({})", t.name, t.tid),
                        t.ticks.saturating_sub(was) as f64 / TICKS_PER_S / dt * 100.0,
                    )
                })
                .collect();
            v.process_cpu = list.iter().map(|(_, c)| c).sum();
            list.sort_by(|a, b| b.1.total_cmp(&a.1));
            v.threads = list;
            if let (Some((busy0, total0)), Some((busy1, total1, _))) = (sys_before, stat) {
                let dt_total = total1.saturating_sub(*total0);
                if dt_total > 0 {
                    v.system_cpu =
                        Some(busy1.saturating_sub(*busy0) as f64 / dt_total as f64 * 100.0);
                }
            }
        } else {
            v.threads = threads
                .iter()
                .map(|t| (format!("{} ({})", t.name, t.tid), 0.0))
                .collect();
        }
        self.last = Some(Previous {
            at: now,
            threads,
            system: stat.map(|s| (s.0, s.1)),
        });
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_lines_with_spaces_in_the_name() {
        let line = "1234 (mpfree screen) S 1 1234 1234 0 -1 4194560 120 0 0 0 250 75 0 0 20 0 3 0 100 1000 200";
        assert_eq!(stat_ticks(line), Some(325));
        assert_eq!(stat_ticks("garbage"), None);
    }

    #[test]
    fn status_meminfo_and_proc_stat() {
        let status = "Name:\tmpfree\nVmHWM:\t   45120 kB\nVmRSS:\t   41000 kB\nThreads:\t4\n";
        assert_eq!(kb_field(status, "VmRSS"), Some(41000));
        assert_eq!(kb_field(status, "VmHWM"), Some(45120));
        assert_eq!(kb_field(status, "VmSwap"), None);
        let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 25 0 12 200 12 0 0 0 0 0\ncpu1 25 0 13 200 13 0 0 0 0 0\nintr 1\n";
        assert_eq!(cpu_line(stat), Some((150, 1000, 2)));
    }

    #[test]
    fn a_sampler_reads_a_fixture_tree_and_measures_between_samples() {
        let root = std::env::temp_dir().join(format!("mpfree-perf-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let task = root.join("self/task/77");
        fs::create_dir_all(&task).unwrap();
        fs::create_dir_all(root.join("sys/class/thermal/thermal_zone0")).unwrap();
        fs::create_dir_all(root.join("sys/devices/system/cpu/cpu0/cpufreq")).unwrap();
        let stat =
            |ticks: u64| format!("77 (mpfree) R 1 1 1 0 -1 0 0 0 0 0 {ticks} 0 0 0 20 0 1 0 1 1 1");
        fs::write(task.join("stat"), stat(100)).unwrap();
        fs::write(task.join("comm"), "mpfree\n").unwrap();
        fs::write(
            root.join("self/status"),
            "VmHWM:\t2048 kB\nVmRSS:\t1024 kB\n",
        )
        .unwrap();
        fs::write(
            root.join("stat"),
            "cpu  10 0 0 90 0 0 0 0\ncpu0 10 0 0 90 0 0 0 0\n",
        )
        .unwrap();
        fs::write(
            root.join("meminfo"),
            "MemTotal: 2048000 kB\nMemAvailable: 1024000 kB\n",
        )
        .unwrap();
        fs::write(root.join("loadavg"), "0.50 0.40 0.30 1/80 1234\n").unwrap();
        fs::write(root.join("sys/class/thermal/thermal_zone0/temp"), "48500\n").unwrap();
        fs::write(
            root.join("sys/class/thermal/thermal_zone0/type"),
            "soc-thermal\n",
        )
        .unwrap();
        fs::write(
            root.join("sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq"),
            "1608000\n",
        )
        .unwrap();
        let r = root.to_str().unwrap();
        let mut s = Sampler::at(&format!("{r}/self"), r, &format!("{r}/sys"));
        let first = s.sample();
        assert_eq!(
            (first.over_s, first.process_cpu, first.thread_count),
            (0.0, 0.0, 1)
        );
        assert_eq!(first.rss_kb, Some(1024));
        assert_eq!(first.temps, vec![("soc-thermal".to_string(), 48.5)]);
        assert_eq!(first.mhz, vec![1608]);
        fs::write(task.join("stat"), stat(110)).unwrap();
        fs::write(
            root.join("stat"),
            "cpu  20 0 0 180 0 0 0 0\ncpu0 20 0 0 180 0 0 0 0\n",
        )
        .unwrap();
        let second = s.sample();
        assert!(second.over_s > 0.0);
        assert!(second.process_cpu > 0.0, "10 ticks over a short time");
        assert_eq!(second.threads[0].0, "mpfree (77)");
        assert_eq!(second.system_cpu.map(|c| c.round()), Some(10.0));
        let line = second.line();
        assert!(
            line.starts_with("perf: cpu ")
                && line.contains("rss 1.0 MB (peak 2.0 MB)")
                && line.contains("load 0.50 0.40 0.30")
                && line.contains("soc-thermal 48.5 C")
                && line.contains("clock 1608 MHz"),
            "{line}"
        );
        assert!(second
            .rows()
            .iter()
            .any(|(k, v)| k == "Memory available" && v == "1000.0 MB of 2000.0 MB"));
        let _ = fs::remove_dir_all(&root);
    }
}
