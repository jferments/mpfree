//! Every process on the machine and what it uses: the data behind a "top"
//! page in the Dev tools. Read-only: it reads `/proc` and nothing else.
//!
//! - Per process, from `/proc/<pid>/stat`: name, state, CPU time (utime +
//!   stime), threads, scheduling policy and real-time priority, nice, and
//!   resident memory (pages times the page size, 4096 bytes on ARMv7 and
//!   x86-64 Linux; an assumption the page states).
//! - Per core and in total, from `/proc/stat`.
//! - Per thread of one chosen process, from `/proc/<pid>/task/<tid>/stat`,
//!   for a closer look at MPFree's own threads or anyone else's.
//!
//! CPU figures are the change between two samples, as a share of one core
//! (a process can show more than 100 % across several cores). Call
//! [`ProcTable::sample`] from the screen thread every second or so, never
//! from the audio thread: it lists directories and allocates.

use std::fs;

/// Clock ticks per second in /proc (USER_HZ is 100 in the Linux ABI on ARM
/// and x86).
pub const TICKS_PER_S: f64 = 100.0;
/// Bytes per page for RSS (4 KiB pages on ARMv7 and x86-64 Linux).
pub const PAGE_BYTES: u64 = 4096;

/// What a process's `stat` line holds that the page needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stat {
    pub pid: u32,
    pub name: String,
    pub state: char,
    /// utime + stime, in ticks.
    pub ticks: u64,
    pub nice: i64,
    pub threads: u64,
    pub rss_pages: u64,
    /// 0 for normal processes, 1 to 99 for SCHED_FIFO / SCHED_RR.
    pub rt_priority: u64,
    /// 0 OTHER, 1 FIFO, 2 RR, 3 BATCH, 5 IDLE, 6 DEADLINE.
    pub policy: u64,
}

/// Parses a `/proc/<pid>/stat` (or task stat) line. The name sits in
/// parentheses and may hold spaces or parentheses, so fields are counted
/// after the last `)`.
pub fn parse_stat(line: &str) -> Option<Stat> {
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    let pid = line[..open].trim().parse().ok()?;
    let name = line.get(open + 1..close)?.to_string();
    let f: Vec<&str> = line[close + 1..].split_whitespace().collect();
    // f[0] is field 3 (state); field n is f[n - 3].
    let num = |n: usize| -> Option<u64> { f.get(n - 3)?.parse().ok() };
    Some(Stat {
        pid,
        name,
        state: f.first()?.chars().next()?,
        ticks: num(14)? + num(15)?,
        nice: f.get(19 - 3)?.parse().ok()?,
        threads: num(20)?,
        rss_pages: num(24)?,
        rt_priority: num(40).unwrap_or(0),
        policy: num(41).unwrap_or(0),
    })
}

/// A policy as the page shows it.
pub fn policy_name(policy: u64, rt_priority: u64) -> String {
    match policy {
        1 => format!("FIFO {rt_priority}"),
        2 => format!("RR {rt_priority}"),
        3 => "batch".to_string(),
        5 => "idle".to_string(),
        6 => "deadline".to_string(),
        _ => "normal".to_string(),
    }
}

/// (busy, total) ticks for each `cpu` line of /proc/stat: the total first,
/// then each core.
pub fn parse_cpu_lines(text: &str) -> Vec<(u64, u64)> {
    text.lines()
        .filter(|l| l.starts_with("cpu"))
        .filter_map(|l| {
            let v: Vec<u64> = l
                .split_whitespace()
                .skip(1)
                .filter_map(|x| x.parse().ok())
                .collect();
            if v.len() < 4 {
                return None;
            }
            let total: u64 = v.iter().take(8).sum();
            let idle = v[3] + v.get(4).copied().unwrap_or(0);
            Some((total - idle, total))
        })
        .collect()
}

/// One row of the process page.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcRow {
    pub pid: u32,
    pub name: String,
    pub state: char,
    /// Share of one core since the last sample.
    pub cpu_pct: f64,
    pub rss_kb: u64,
    pub threads: u64,
    pub policy: String,
    pub nice: i64,
}

/// One sample of the whole machine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProcView {
    /// Seconds since the previous sample (0 on the first: no CPU figures).
    pub over_s: f64,
    /// Busiest first, then by memory.
    pub rows: Vec<ProcRow>,
    /// The whole machine, share of every core.
    pub total_cpu_pct: Option<f64>,
    /// Each core, share of that core.
    pub core_cpu_pct: Vec<f64>,
}

impl ProcView {
    /// The busiest `n` as a log line.
    pub fn line(&self, n: usize) -> String {
        let top: Vec<String> = self
            .rows
            .iter()
            .take(n)
            .map(|r| {
                format!(
                    "{} ({}) {:.1} % {} MB",
                    r.name,
                    r.pid,
                    r.cpu_pct,
                    r.rss_kb / 1024
                )
            })
            .collect();
        let cores: Vec<String> = self
            .core_cpu_pct
            .iter()
            .map(|c| format!("{c:.0}"))
            .collect();
        format!(
            "procs: system {}, cores {} %, top: {}",
            self.total_cpu_pct
                .map_or("-".to_string(), |c| format!("{c:.1} %")),
            if cores.is_empty() {
                "-".to_string()
            } else {
                cores.join("/")
            },
            top.join(", ")
        )
    }
}

/// Reads `/proc` (or a fixture tree in tests) and keeps the previous sample
/// for CPU deltas.
pub struct ProcTable {
    root: String,
    last_ticks: Vec<(u32, u64)>,
    last_cpu: Vec<(u64, u64)>,
    last_at: Option<std::time::Instant>,
}

impl Default for ProcTable {
    fn default() -> ProcTable {
        ProcTable::new()
    }
}

impl ProcTable {
    pub fn new() -> ProcTable {
        ProcTable::at("/proc")
    }

    pub fn at(root: &str) -> ProcTable {
        ProcTable {
            root: root.trim_end_matches('/').to_string(),
            last_ticks: Vec::new(),
            last_cpu: Vec::new(),
            last_at: None,
        }
    }

    fn stats(dir: &str) -> Vec<Stat> {
        let Ok(entries) = fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut v: Vec<Stat> = entries
            .filter_map(|e| {
                let e = e.ok()?;
                e.file_name().to_str()?.parse::<u32>().ok()?;
                // A process can end between the listing and the read.
                parse_stat(&fs::read_to_string(e.path().join("stat")).ok()?)
            })
            .collect();
        v.sort_by_key(|s| s.pid);
        v
    }

    /// Reads every process once. CPU figures are since the last call.
    pub fn sample(&mut self) -> ProcView {
        let now = std::time::Instant::now();
        let stats = ProcTable::stats(&self.root);
        let cpu = fs::read_to_string(format!("{}/stat", self.root))
            .map(|t| parse_cpu_lines(&t))
            .unwrap_or_default();
        let dt = self
            .last_at
            .map(|t| now.duration_since(t).as_secs_f64().max(1e-3));
        let mut rows: Vec<ProcRow> = stats
            .iter()
            .map(|s| {
                let cpu_pct = match (dt, self.last_ticks.binary_search_by_key(&s.pid, |p| p.0)) {
                    (Some(dt), Ok(i)) => {
                        s.ticks.saturating_sub(self.last_ticks[i].1) as f64 / TICKS_PER_S / dt
                            * 100.0
                    }
                    _ => 0.0,
                };
                ProcRow {
                    pid: s.pid,
                    name: s.name.clone(),
                    state: s.state,
                    cpu_pct,
                    rss_kb: s.rss_pages * PAGE_BYTES / 1024,
                    threads: s.threads,
                    policy: policy_name(s.policy, s.rt_priority),
                    nice: s.nice,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.cpu_pct
                .total_cmp(&a.cpu_pct)
                .then(b.rss_kb.cmp(&a.rss_kb))
        });
        let pct = |now: &(u64, u64), was: &(u64, u64)| {
            let total = now.1.saturating_sub(was.1);
            (total > 0).then(|| now.0.saturating_sub(was.0) as f64 / total as f64 * 100.0)
        };
        let mut view = ProcView {
            over_s: dt.unwrap_or(0.0),
            rows,
            ..ProcView::default()
        };
        if dt.is_some() && cpu.len() == self.last_cpu.len() && !cpu.is_empty() {
            view.total_cpu_pct = pct(&cpu[0], &self.last_cpu[0]);
            view.core_cpu_pct = cpu[1..]
                .iter()
                .zip(&self.last_cpu[1..])
                .filter_map(|(n, w)| pct(n, w))
                .collect();
        }
        self.last_ticks = stats.iter().map(|s| (s.pid, s.ticks)).collect();
        self.last_cpu = cpu;
        self.last_at = Some(now);
        view
    }

    /// The threads of one process, with their total CPU ticks so far (the
    /// caller diffs two calls for a rate, as [`ProcTable::sample`] does).
    pub fn threads(&self, pid: u32) -> Vec<Stat> {
        ProcTable::stats(&format!("{}/{pid}/task", self.root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(pid: u32, name: &str, ticks: u64, rss: u64, policy: u64, rt: u64) -> String {
        // Fields 3..41: state, then 10 zeros to field 13, utime 14, stime 15,
        // 16-18, nice 19, threads 20, 21-23, rss 24, 25-39, rt 40, policy 41.
        let mut f = vec!["0".to_string(); 39];
        f[0] = "S".to_string();
        f[14 - 3] = ticks.to_string();
        f[15 - 3] = "0".to_string();
        f[19 - 3] = "-5".to_string();
        f[20 - 3] = "3".to_string();
        f[24 - 3] = rss.to_string();
        f[40 - 3] = rt.to_string();
        f[41 - 3] = policy.to_string();
        format!("{pid} ({name}) {}", f.join(" "))
    }

    #[test]
    fn parses_stat_lines_with_awkward_names() {
        let s = parse_stat(&stat(42, "MPC (audio) x", 250, 1000, 1, 80)).unwrap();
        assert_eq!(
            (
                s.pid,
                s.name.as_str(),
                s.state,
                s.ticks,
                s.nice,
                s.threads,
                s.rss_pages
            ),
            (42, "MPC (audio) x", 'S', 250, -5, 3, 1000)
        );
        assert_eq!(policy_name(s.policy, s.rt_priority), "FIFO 80");
        assert_eq!(policy_name(0, 0), "normal");
        assert!(parse_stat("nonsense").is_none());
        // An old kernel without the rt fields still parses.
        let full = stat(1, "init", 5, 10, 0, 0);
        let short: Vec<&str> = full.split(' ').take(26).collect();
        assert_eq!(parse_stat(&short.join(" ")).unwrap().policy, 0);
    }

    #[test]
    fn cpu_lines() {
        let t = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 60 0 30 400 10 0 0 0\ncpu1 40 0 20 400 40 0 0 0\nintr 5\n";
        assert_eq!(parse_cpu_lines(t), vec![(150, 1000), (90, 500), (60, 500)]);
    }

    #[test]
    fn a_table_reads_a_fixture_tree() {
        let root = std::env::temp_dir().join(format!("mpfree-procs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (pid, name, rss, pol, rt) in [
            (1u32, "init", 100u64, 0u64, 0u64),
            (300, "MPC", 250_000, 1, 80),
            (400, "mpfree-hello", 10_000, 0, 0),
        ] {
            fs::create_dir_all(root.join(pid.to_string())).unwrap();
            fs::write(
                root.join(format!("{pid}/stat")),
                stat(pid, name, 100, rss, pol, rt),
            )
            .unwrap();
        }
        fs::create_dir_all(root.join("400/task/401")).unwrap();
        fs::write(
            root.join("400/task/401/stat"),
            stat(401, "audio", 50, 10_000, 1, 70),
        )
        .unwrap();
        fs::create_dir_all(root.join("self")).unwrap();
        fs::write(
            root.join("stat"),
            "cpu  10 0 0 90 0 0 0 0\ncpu0 5 0 0 45 0 0 0 0\ncpu1 5 0 0 45 0 0 0 0\n",
        )
        .unwrap();
        let r = root.to_str().unwrap();
        let mut t = ProcTable::at(r);
        let first = t.sample();
        assert_eq!(first.rows.len(), 3, "only numbered directories");
        assert_eq!(first.rows[0].name, "MPC", "no CPU yet, so by memory");
        assert_eq!(first.rows[0].rss_kb, 1_000_000);
        assert_eq!(first.total_cpu_pct, None);
        fs::write(
            root.join("400/stat"),
            stat(400, "mpfree-hello", 150, 10_000, 0, 0),
        )
        .unwrap();
        fs::write(
            root.join("stat"),
            "cpu  60 0 0 140 0 0 0 0\ncpu0 50 0 0 50 0 0 0 0\ncpu1 10 0 0 90 0 0 0 0\n",
        )
        .unwrap();
        let second = t.sample();
        assert_eq!(second.rows[0].name, "mpfree-hello");
        assert!(second.rows[0].cpu_pct > 0.0);
        assert_eq!(second.total_cpu_pct, Some(50.0));
        assert_eq!(second.core_cpu_pct, vec![90.0, 10.0]);
        assert_eq!(
            second.rows.iter().find(|r| r.pid == 300).unwrap().policy,
            "FIFO 80"
        );
        assert!(
            second
                .line(2)
                .starts_with("procs: system 50.0 %, cores 90/10 %, top: mpfree-hello (400) "),
            "{}",
            second.line(2)
        );
        let th = t.threads(400);
        assert_eq!(
            (th.len(), th[0].name.as_str(), th[0].rt_priority),
            (1, "audio", 70)
        );
        let _ = fs::remove_dir_all(&root);
    }
}
