//! `mpfree-disktest --heat`: where the MPC's temperature settles with every
//! core busy, for System status's "MPC running hot" (docs/dev/design/status.md).
//! D2b ran its busy cores for about 3 minutes only (55.5 to 62.5 °C), which
//! doesn't show where the temperature levels off.
//!
//! One ordinary thread pinned to each core does what D2b's step 5 did
//! ([`super::load`]): filters a 256 KiB buffer over and over. It runs until
//! the hottest zone has risen less than [`SETTLED_C`] over the last
//! [`WINDOW`], or for [`LONGEST`] at the most, and stops at once at 75 °C
//! or at the lowest trip point the kernel gives that is lower (as the
//! under-load run does). Every [`EVERY`] it says each zone's temperature
//! and each core's clock. Each core's clock settings (`scaling_cur_freq`,
//! `scaling_max_freq`, `cpuinfo_max_freq`, `scaling_governor`) are read at
//! the start and at the end.
//!
//! It reads only `/sys/class/thermal` and `/sys/devices/system/cpu`, and
//! writes no file anywhere. Its threads are ordinary ones (`SCHED_OTHER`,
//! nice 0), so the pad player's real-time audio thread still comes first.
//! With no temperature to read it refuses, so the cores are never kept
//! busy unwatched. Ctrl-C, the ssh session closing, or the time limit stop
//! the threads.

use super::load::{cpus_online, hog, hottest, millidegrees, stop_at, Zone};
use super::STOP;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Where the cores' clocks are read (read-only).
pub(super) const CPU: &str = "/sys/devices/system/cpu";

/// Settled: the hottest zone rose less than this over [`WINDOW`].
pub(super) const SETTLED_C: f64 = 0.5;

/// How long the run's times are, as a full run on the MPC has them, or a
/// quick desktop test's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Times {
    /// How often the temperatures and clocks are said.
    pub(super) every: Duration,
    /// The span a rise is measured over.
    pub(super) window: Duration,
    /// The longest the cores are kept busy.
    pub(super) longest: Duration,
}

/// Every 10 s, over the last 5 minutes, for 30 minutes at the most.
pub(super) const EVERY: Duration = Duration::from_secs(10);
pub(super) const WINDOW: Duration = Duration::from_secs(300);
pub(super) const LONGEST: Duration = Duration::from_secs(1800);

pub(super) fn times(quick: bool) -> Times {
    if quick {
        Times {
            every: Duration::from_millis(100),
            window: Duration::from_millis(500),
            longest: Duration::from_millis(1500),
        }
    } else {
        Times {
            every: EVERY,
            window: WINDOW,
            longest: LONGEST,
        }
    }
}

/// Why the busy threads stopped.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum End {
    /// The hottest zone rose less than [`SETTLED_C`] over the window.
    Settled { rose: f64 },
    /// It reached the stop temperature.
    Hot,
    /// The time limit, still rising this much over the window.
    Longest { rose: f64 },
    /// Ctrl-C or the ssh session closing.
    Stopped,
}

/// The readings so far: seconds since the start and the hottest zone's
/// °C. Settled once the run is a window old and the hottest now is less
/// than [`SETTLED_C`] above the reading a window ago.
pub(super) fn settled(readings: &[(f64, f64)], window_s: f64) -> Option<f64> {
    let &(now_s, now_c) = readings.last()?;
    if now_s < window_s {
        return None;
    }
    // The latest reading at least a window before now.
    let &(_, then_c) = readings
        .iter()
        .rev()
        .find(|(t, _)| *t <= now_s - window_s)?;
    let rose = now_c - then_c;
    (rose < SETTLED_C).then_some(rose)
}

/// How much the hottest zone rose over the last window (or since the start).
fn rise(readings: &[(f64, f64)], window_s: f64) -> f64 {
    let Some(&(now_s, now_c)) = readings.last() else {
        return f64::NAN;
    };
    let then_c = readings
        .iter()
        .rev()
        .find(|(t, _)| *t <= now_s - window_s)
        .or(readings.first())
        .map_or(f64::NAN, |r| r.1);
    now_c - then_c
}

/// One core's clock settings, as sysfs has them ("?" for any that doesn't
/// read).
fn clock_settings(cpu_root: &Path, cpu: usize) -> String {
    let f = |name: &str| {
        fs::read_to_string(cpu_root.join(format!("cpu{cpu}/cpufreq/{name}")))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "?".into())
    };
    let mhz = |name: &str| {
        let v = f(name);
        v.parse::<u64>().map_or(v, |k| format!("{} MHz", k / 1000))
    };
    format!(
        "cpu{cpu}: now {}, max {} (scaling_max_freq), {} (cpuinfo_max_freq), governor {}",
        mhz("scaling_cur_freq"),
        mhz("scaling_max_freq"),
        mhz("cpuinfo_max_freq"),
        f("scaling_governor")
    )
}

/// Each core's clock now, in MHz, one word each ("?" for one that doesn't read).
fn clocks_now(cpu_root: &Path, cpus: usize) -> String {
    (0..cpus)
        .map(|c| {
            fs::read_to_string(cpu_root.join(format!("cpu{c}/cpufreq/scaling_cur_freq")))
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map_or_else(|| "?".into(), |k| (k / 1000).to_string())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Seconds as the report gives them: whole ones, tenths under 10 s.
fn secs(t: f64) -> String {
    if t < 10.0 {
        format!("{t:.1}")
    } else {
        format!("{t:.0}")
    }
}

fn temps(zones: &[Zone]) -> String {
    zones
        .iter()
        .map(|z| match millidegrees(&z.temp) {
            Some(c) => format!("{} {c:.1} °C", z.name),
            None => format!("{} ?", z.name),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The run. `zones` from [`super::load::zones`]; `cpu_root` is
/// `/sys/devices/system/cpu` (a desktop test may name a stand-in).
pub fn run(zones: &[Zone], cpu_root: &Path, t: Times) -> Result<(), String> {
    if zones.is_empty() {
        return Err(
            "no temperature to read (/sys/class/thermal): the cores are never kept busy unwatched"
                .into(),
        );
    }
    let cpus = cpus_online();
    let limit = stop_at(zones);
    say!(
        "heat: one busy thread on each of {cpus} cores (ordinary priority) until the hottest zone rises less than {SETTLED_C} °C in {} s, or {} s at the most; they stop at {limit:.0} °C. It writes no file.",
        secs(t.window.as_secs_f64()),
        secs(t.longest.as_secs_f64())
    );
    for z in zones {
        let trips: Vec<String> = z
            .trips
            .iter()
            .map(|(k, c)| format!("{k} {c:.0} °C"))
            .collect();
        say!(
            "  zone {}: trip points: {}",
            z.name,
            if trips.is_empty() {
                "none".to_string()
            } else {
                trips.join(", ")
            }
        );
    }
    say!("clocks at the start:");
    for c in 0..cpus {
        say!("  {}", clock_settings(cpu_root, c));
    }
    let start_c = hottest(zones).ok_or("the temperature stopped reading")?;
    say!("  {} (the hottest {start_c:.1} °C)", temps(zones));

    let (on, hot) = (AtomicBool::new(true), AtomicBool::new(false));
    let start = Instant::now();
    let mut readings = vec![(0.0, start_c)];
    let mut top = start_c;
    let (end, passes) = std::thread::scope(|s| {
        let threads: Vec<_> = (0..cpus)
            .map(|c| {
                let (on, hot) = (&on, &hot);
                std::thread::Builder::new()
                    .name(format!("disktest-heat{c}"))
                    .spawn_scoped(s, move || hog(c, on, hot))
            })
            .collect();
        let mut next = t.every;
        let end = loop {
            if STOP.load(Ordering::Relaxed) {
                break End::Stopped;
            }
            let now = start.elapsed();
            if let Some(c) = hottest(zones) {
                top = top.max(c);
                if c >= limit {
                    hot.store(true, Ordering::Relaxed);
                    readings.push((now.as_secs_f64(), c));
                    say!(
                        "  {:>5.0} s: {} (at the stop, {limit:.0} °C)",
                        now.as_secs_f64(),
                        temps(zones)
                    );
                    break End::Hot;
                }
                if now >= next {
                    next += t.every;
                    readings.push((now.as_secs_f64(), c));
                    say!(
                        "  {:>5.0} s: {}; clocks {} MHz",
                        now.as_secs_f64(),
                        temps(zones),
                        clocks_now(cpu_root, cpus)
                    );
                    if let Some(rose) = settled(&readings, t.window.as_secs_f64()) {
                        break End::Settled { rose };
                    }
                }
            }
            if now >= t.longest {
                break End::Longest {
                    rose: rise(&readings, t.window.as_secs_f64()),
                };
            }
            std::thread::sleep(Duration::from_millis(50).min(t.every));
        };
        on.store(false, Ordering::Relaxed);
        let mut passes = Vec::new();
        for th in threads {
            passes.push(match th {
                Ok(h) => h.join().ok(),
                Err(_) => None,
            });
        }
        (end, passes)
    });
    let secs = start.elapsed().as_secs_f64();
    let pinned = passes
        .iter()
        .filter(|p| p.is_some_and(|(_, pin)| pin))
        .count();
    let ran = passes.iter().filter(|p| p.is_some()).count();
    say!("clocks at the end:");
    for c in 0..cpus {
        say!("  {}", clock_settings(cpu_root, c));
    }
    let last_c = readings.last().map_or(start_c, |r| r.1);
    say!(
        "busy threads: {ran} of {cpus} ran ({pinned} pinned to their core) for {secs:.0} s; the hottest zone went from {start_c:.1} to {last_c:.1} °C, {top:.1} °C at the most"
    );
    say!(
        "heat: {}",
        match end {
            End::Settled { rose } => format!(
                "settled at {last_c:.1} °C after {secs:.0} s (it rose {rose:.1} °C in the last {:.0} s)",
                t.window.as_secs_f64()
            ),
            End::Hot => format!(
                "stopped at {last_c:.1} °C after {secs:.0} s: the stop is {limit:.0} °C"
            ),
            End::Longest { rose } => format!(
                "still rising after {secs:.0} s, the longest it runs: {last_c:.1} °C, up {rose:.1} °C in the last {:.0} s",
                t.window.as_secs_f64()
            ),
            End::Stopped => format!("stopped by you after {secs:.0} s, at {last_c:.1} °C"),
        }
    );
    if ran < cpus {
        return Err(format!("only {ran} of {cpus} busy threads ran"));
    }
    if end == End::Stopped {
        return Err("stopped".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_once_a_window_old_and_rising_less_than_half_a_degree() {
        // Rising 1 °C a minute: never settled.
        let rising: Vec<(f64, f64)> = (0..40)
            .map(|k| (k as f64 * 10.0, 55.0 + k as f64 / 6.0))
            .collect();
        assert_eq!(settled(&rising, 300.0), None);
        // Levelling off at 64 °C after 10 minutes.
        let level: Vec<(f64, f64)> = (0..100)
            .map(|k| {
                let t = k as f64 * 10.0;
                (t, 55.0 + 9.0 * (1.0 - (-t / 120.0).exp()))
            })
            .collect();
        let at = (1..=level.len())
            .find(|&n| settled(&level[..n], 300.0).is_some())
            .unwrap();
        let (t, c) = level[at - 1];
        assert!((600.0..=700.0).contains(&t), "settled at {t} s");
        assert!(c > 63.5, "{c}");
        let rose = settled(&level[..at], 300.0).unwrap();
        assert!((0.0..SETTLED_C).contains(&rose), "{rose}");
        // Not before a window has passed, even when flat.
        let flat: Vec<(f64, f64)> = (0..20).map(|k| (k as f64 * 10.0, 60.0)).collect();
        assert_eq!(settled(&flat, 300.0), None);
        assert_eq!(settled(&[], 300.0), None);
        // Falling counts as settled.
        let falling = [(0.0, 62.0), (300.0, 61.0)];
        assert_eq!(settled(&falling, 300.0), Some(-1.0));
    }

    #[test]
    fn the_rise_over_the_window_or_since_the_start() {
        let r = [(0.0, 50.0), (100.0, 52.0), (400.0, 56.0)];
        assert_eq!(rise(&r, 300.0), 4.0);
        assert_eq!(rise(&r, 1000.0), 6.0);
        assert!(rise(&[], 300.0).is_nan());
    }

    #[test]
    fn clock_settings_read_as_sysfs_has_them() {
        let d = std::env::temp_dir().join(format!("mpfree-disktest-heat-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let f = d.join("cpu0/cpufreq");
        fs::create_dir_all(&f).unwrap();
        fs::write(f.join("scaling_cur_freq"), "1608000\n").unwrap();
        fs::write(f.join("scaling_max_freq"), "1608000\n").unwrap();
        fs::write(f.join("cpuinfo_max_freq"), "1800000\n").unwrap();
        fs::write(f.join("scaling_governor"), "performance\n").unwrap();
        assert_eq!(
            clock_settings(&d, 0),
            "cpu0: now 1608 MHz, max 1608 MHz (scaling_max_freq), 1800 MHz (cpuinfo_max_freq), governor performance"
        );
        assert_eq!(
            clock_settings(&d, 1),
            "cpu1: now ?, max ? (scaling_max_freq), ? (cpuinfo_max_freq), governor ?"
        );
        assert_eq!(clocks_now(&d, 2), "1608 ?");
        fs::remove_dir_all(&d).unwrap();
    }
}
