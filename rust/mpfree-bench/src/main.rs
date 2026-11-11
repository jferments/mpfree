//! mpfree-bench: what every MPFree audio engine costs at the MPC One's
//! period (128 frames at 44.1 kHz, 2.902 ms), against its budget.
//!
//! ```text
//! mpfree-bench [--quick] [--filter TEXT] [--calibration FILE] [--json FILE]
//!              [--check-allocs]
//! mpfree-bench --calibrate [--machine NAME]
//! mpfree-bench --compare A.json B.json
//! mpfree-bench --afford [--quick] [--calibration FILE]
//! ```
//!
//! - On the desktop it prints each case's mean, p99 and worst block. With
//!   `--calibration` (a file `--calibrate` printed on the MPC) it adds an
//!   estimate for the MPC and a verdict against the module's budget.
//! - On the MPC (an ARM build) the numbers are the device's own, and the
//!   verdicts follow `budget::judge`: an engine on its p99.9 block, the whole
//!   period on its worst.
//! - `--check-allocs` exits with status 2 if any engine allocated inside a
//!   block (the counting allocator is installed in this program).
//! - `--calibrate` times the reference kernels and prints a calibration file.
//! - `--compare A.json B.json` puts two `--json` reports side by side, case
//!   by case: B's p99 and worst as a share of A's (the default and the
//!   cortex-a17 build on the MPC, or a change against main).
//! - `--afford` measures each synth preset and the sampler at several voice
//!   counts, fits a fixed cost plus a cost per voice to the p99 blocks, and
//!   says how many voices fit in the module's budget: on this machine, as an
//!   MPC estimate with `--calibration`, or for real on the MPC.
//! - The "mem MB" column is what each engine holds once it is built (its
//!   buffers, delay lines, sample memory), from the counting allocator.
//!
//! It only reads and computes: it opens no device, writes no file unless
//! `--json` names one, and runs on one thread at normal priority.

mod cases;
#[cfg(test)]
mod golden;

use std::fmt::Write as _;
use std::process::ExitCode;

use mpfree_prof::alloc::{self, CountingAlloc};
use mpfree_prof::budget::{self, Verdict, MPC};
use mpfree_prof::model::{self, Calibration, Model};
use mpfree_prof::{Probe, Span, Summary};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

struct Options {
    quick: bool,
    filter: Option<String>,
    calibration: Option<String>,
    json: Option<String>,
    check_allocs: bool,
    calibrate: bool,
    machine: Option<String>,
    compare: Option<(String, String)>,
    afford: bool,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        quick: false,
        filter: None,
        calibration: None,
        json: None,
        check_allocs: false,
        calibrate: false,
        machine: None,
        compare: None,
        afford: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match a.as_str() {
            "--quick" => o.quick = true,
            "--filter" => o.filter = Some(value("--filter")?.to_lowercase()),
            "--calibration" => o.calibration = Some(value("--calibration")?),
            "--json" => o.json = Some(value("--json")?),
            "--check-allocs" => o.check_allocs = true,
            "--calibrate" => o.calibrate = true,
            "--machine" => o.machine = Some(value("--machine")?),
            "--afford" => o.afford = true,
            "--compare" => {
                let a = value("--compare")?;
                o.compare = Some((a, value("--compare")?));
            }
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok(o)
}

/// What this program runs on and how it was built.
fn machine() -> String {
    let cpu = std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|t| {
        t.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            matches!(k.trim(), "model name" | "Hardware").then(|| v.trim().to_string())
        })
    });
    let mhz = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq")
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
        .map(|k| format!(", {} MHz", k / 1000))
        .unwrap_or_default();
    format!(
        "{} ({}{mhz})",
        cpu.unwrap_or_else(|| "unknown CPU".to_string()),
        std::env::consts::ARCH
    )
}

fn build() -> String {
    // Arm target features cannot be read with cfg! on stable Rust, so the
    // flags are recorded at build time (build.rs).
    let flags = env!("MPFREE_BENCH_RUSTFLAGS");
    format!(
        "{}{}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        if flags.is_empty() {
            String::new()
        } else {
            format!(", RUSTFLAGS {flags}")
        }
    )
}

struct Row {
    name: String,
    module: &'static str,
    what: &'static str,
    s: Summary,
    mem_bytes: i64,
}

/// A module's budget, as a share of the period; "period" is the whole
/// audio thread's.
fn budget_pct(module: &str) -> Option<f64> {
    budget::pct_for(module)
}

/// Megabytes (2^20 bytes), for the memory column.
fn mb(bytes: i64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn json(rows: &[Row], kernels: &[(String, f64)], blocks: u64) -> String {
    let mut j = String::new();
    let _ = write!(j, "{{\n  \"schema\": \"mpfree-bench/1\",\n  \"machine\": \"{}\",\n  \"arch\": \"{}\",\n  \"build\": \"{}\",\n  \
                       \"period\": {{\"frames\": {}, \"rate\": {}}},\n  \"blocks\": {blocks},\n  \"kernels\": [",
                   esc(&machine()), std::env::consts::ARCH, build(), MPC.frames, MPC.rate);
    for (i, (k, ns)) in kernels.iter().enumerate() {
        let _ = write!(
            j,
            "{}\n    {{\"name\": \"{k}\", \"ns\": {ns:.1}}}",
            if i > 0 { "," } else { "" }
        );
    }
    j.push_str("\n  ],\n  \"cases\": [");
    for (i, r) in rows.iter().enumerate() {
        let s = &r.s;
        let _ = write!(j, "{}\n    {{\"name\": \"{}\", \"module\": \"{}\", \"what\": \"{}\", \"blocks\": {}, \"mean_ns\": {:.0}, \"p50_ns\": {}, \
                           \"p99_ns\": {}, \"p999_ns\": {}, \"max_ns\": {}, \"allocs\": {}, \"mem_bytes\": {}}}",
                       if i > 0 { "," } else { "" }, esc(&r.name), r.module, esc(r.what), s.blocks, s.mean_ns, s.p50_ns, s.p99_ns,
                       s.p999_ns, s.max_ns, s.allocs, r.mem_bytes);
    }
    j.push_str("\n  ]\n}\n");
    j
}

/// One case from a `--json` report.
#[derive(Debug, PartialEq)]
struct Saved {
    name: String,
    p99_ns: f64,
    max_ns: f64,
    mem_bytes: f64,
}

/// A JSON string value after `"key": "`, unescaped (the writer escapes only
/// `\` and `"`).
fn json_str(line: &str, key: &str) -> Option<String> {
    let rest = &line[line.find(&format!("\"{key}\": \""))? + key.len() + 5..];
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '"' => return Some(out),
            c => out.push(c),
        }
    }
    None
}

fn json_num(line: &str, key: &str) -> Option<f64> {
    let rest = &line[line.find(&format!("\"{key}\": "))? + key.len() + 4..];
    rest[..rest.find([',', '}'])?].trim().parse().ok()
}

/// The machine line and the cases of a report this program wrote (one case
/// per line).
fn read_report(path: &str) -> Result<(String, Vec<Saved>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
    if !text.contains("\"schema\": \"mpfree-bench/1\"") {
        return Err(format!("{path}: not an mpfree-bench/1 report"));
    }
    let field = |key: &str| {
        text.lines()
            .find_map(|l| json_str(l, key))
            .unwrap_or_default()
    };
    let machine = format!("{}, build {}", field("machine"), field("build"));
    let cases = text
        .lines()
        .filter(|l| l.contains("\"module\": "))
        .map(|l| {
            Some(Saved {
                name: json_str(l, "name")?,
                p99_ns: json_num(l, "p99_ns")?,
                max_ns: json_num(l, "max_ns")?,
                mem_bytes: json_num(l, "mem_bytes").unwrap_or(0.0),
            })
        })
        .collect::<Option<Vec<_>>>()
        .ok_or(format!("{path}: a case line could not be read"))?;
    Ok((machine, cases))
}

/// B against A, case by case.
fn compare(a: &str, b: &str) -> Result<String, String> {
    let ((ma, ca), (mb_, cb)) = (read_report(a)?, read_report(b)?);
    let period_ns = MPC.ns() as f64;
    let pct = |ns: f64| 100.0 * ns / period_ns;
    let mut t = String::new();
    let _ = writeln!(
        t,
        "A: {a} ({ma})\nB: {b} ({mb_})\nShares of one period; B/A below 1.00 means B is faster.\n"
    );
    let _ = writeln!(
        t,
        "{:<50} {:>7} {:>7} {:>6}  {:>7} {:>7} {:>6}",
        "case", "A p99", "B p99", "B/A", "A max", "B max", "B/A"
    );
    for x in &ca {
        match cb.iter().find(|y| y.name == x.name) {
            Some(y) => {
                let _ = writeln!(
                    t,
                    "{:<50} {:>6.2}% {:>6.2}% {:>6.2}  {:>6.2}% {:>6.2}% {:>6.2}{}",
                    x.name,
                    pct(x.p99_ns),
                    pct(y.p99_ns),
                    y.p99_ns / x.p99_ns.max(1.0),
                    pct(x.max_ns),
                    pct(y.max_ns),
                    y.max_ns / x.max_ns.max(1.0),
                    if (y.mem_bytes - x.mem_bytes).abs() > 1024.0 {
                        format!(
                            "  mem {:.2} -> {:.2} MB",
                            mb(x.mem_bytes as i64),
                            mb(y.mem_bytes as i64)
                        )
                    } else {
                        String::new()
                    }
                );
            }
            None => {
                let _ = writeln!(t, "{:<50} only in A", x.name);
            }
        }
    }
    for y in cb.iter().filter(|y| !ca.iter().any(|x| x.name == y.name)) {
        let _ = writeln!(t, "{:<50} only in B", y.name);
    }
    Ok(t)
}

/// Runs a case for `blocks` blocks after a 50-block warm-up.
fn time_case(c: &mut cases::Case, blocks: u64) -> Summary {
    let (mut l, mut r) = (
        vec![0.0f32; MPC.frames as usize],
        vec![0.0f32; MPC.frames as usize],
    );
    for b in 0..50 {
        (c.block)(b, &mut l, &mut r);
    }
    let mut probe = Probe::new(MPC.ns());
    for b in 50..50 + blocks {
        let span = Span::start();
        (c.block)(b, &mut l, &mut r);
        probe.stop(span, MPC.frames);
    }
    std::hint::black_box(&l);
    probe.lifetime()
}

/// Voices that fit in `budget_ns` at `fixed + n * per_voice`, or None if
/// even the fixed cost does not fit.
fn voices_in(budget_ns: f64, fixed: f64, per_voice: f64) -> Option<f64> {
    if fixed.max(0.0) >= budget_ns {
        None
    } else if per_voice <= 0.0 {
        Some(f64::INFINITY)
    } else {
        Some((budget_ns - fixed.max(0.0)) / per_voice)
    }
}

/// The `--afford` table: cost per voice and voices per budget.
fn afford(quick: bool, model: Option<&Model>, on_device: bool) {
    let blocks = (if quick { 2 } else { 6 } * MPC.rate / MPC.frames) as u64;
    let (factor, lo, hi) = model.map_or((1.0, 1.0, 1.0), |m| m.factor());
    let place = if on_device {
        "on this MPC"
    } else if model.is_some() {
        "on the MPC (estimate)"
    } else {
        "on this machine"
    };
    println!("mpfree-bench --afford on {}, build {}", machine(), build());
    println!("p99 block = fixed + voices x per voice, fitted over the voice counts shown. Times are {place}.");
    if model.is_some() && !on_device {
        println!("MPC estimate: this machine x {factor:.1} (kernels range {lo:.1} to {hi:.1}); \"voices\" uses x {factor:.1}, the range gives \"(x-y)\".");
    }
    println!();
    println!(
        "{:<30} {:>9} {:>11}  {:<40}  {:>8}  voices that fit",
        "engine", "fixed us", "us / voice", "p99 us at each voice count", "budget"
    );
    for mut sc in cases::scaling(MPC.rate) {
        let mut pts = Vec::new();
        for (n, c) in sc.points.iter_mut() {
            pts.push((*n, time_case(c, blocks).p99_ns as f64));
        }
        let (a, b) = cases::fit(&pts);
        let budget = budget::for_module(sc.module).expect("module budget");
        let budget_ns = budget.worst_ns(MPC) as f64;
        let fits = |f: f64| voices_in(budget_ns, a * f, b * f);
        let most = pts.iter().map(|p| p.0).max().unwrap_or(1);
        let show = |v: Option<f64>| match v {
            None => "none".to_string(),
            Some(_) if most == 1 => "fits (mono)".to_string(),
            Some(x) if x >= most as f64 => {
                if x.is_finite() {
                    format!("{most}+ ({})", x.floor() as i64)
                } else {
                    format!("{most}+")
                }
            }
            Some(x) => format!("{}", x.floor() as i64),
        };
        let range = if model.is_some() && !on_device {
            format!(" ({}-{})", show(fits(hi)), show(fits(lo)))
        } else {
            String::new()
        };
        let counts = pts
            .iter()
            .map(|(n, ns)| format!("{n}:{:.0}", ns * factor / 1e3))
            .collect::<Vec<_>>()
            .join(" ");
        let per_voice = if most == 1 {
            "-".to_string()
        } else {
            format!("{:.2}", b * factor / 1e3)
        };
        println!(
            "{:<30} {:>9.1} {:>11}  {:<40}  {:>4.0} us  {}{range}",
            sc.name,
            a * factor / 1e3,
            per_voice,
            counts,
            budget_ns / 1e3,
            show(fits(factor))
        );
    }
    println!();
    println!("\"voices that fit\" is for that engine alone in its whole module budget (synth 20 %, sampler 25 % of a period, worst block);");
    println!("several synth tracks share the synth budget. \"16+ (87)\": every count measured fits, and the line runs out at 87 (Poly plays");
    println!("at most 16). A preset whose cost does not grow with voices shows a per-voice cost near 0: its p99 is set by something else.");
    println!("In \"Sampler grains (per grain)\" a voice is one live grain (4 granular pads held, 16 to 256 grains alive): \"us / voice\" is");
    println!(
        "the cost of a grain, and 256 is the engine's grain budget, the most a burst can reach."
    );
}

fn main() -> ExitCode {
    let o = match options() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("mpfree-bench: {e}");
            }
            eprintln!("usage: mpfree-bench [--quick] [--filter TEXT] [--calibration FILE] [--json FILE] [--check-allocs]\n       \
                       mpfree-bench --calibrate [--machine NAME]\n       \
                       mpfree-bench --compare A.json B.json\n       \
                       mpfree-bench --afford [--quick] [--calibration FILE]");
            return ExitCode::from(if e.is_empty() { 0 } else { 1 });
        }
    };
    if let Some((a, b)) = &o.compare {
        return match compare(a, b) {
            Ok(text) => {
                print!("{text}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("mpfree-bench: {e}");
                ExitCode::from(1)
            }
        };
    }
    let (calls, rounds) = if o.quick { (200, 5) } else { (2000, 15) };

    if o.calibrate {
        let cal = Calibration {
            machine: format!("{}, build {}", o.machine.unwrap_or_else(machine), build()),
            kernels: model::measure(calls, rounds),
        };
        print!("{}", cal.to_text());
        // The clock's own cost, so tiny blocks can be read with care.
        let t = std::time::Instant::now();
        for _ in 0..100_000 {
            std::hint::black_box(Span::start().elapsed_ns());
        }
        println!(
            "# a span (two clock reads) costs {:.0} ns",
            t.elapsed().as_nanos() as f64 / 100_000.0
        );
        return ExitCode::SUCCESS;
    }

    let on_device = cfg!(target_arch = "arm");
    let kernels = model::measure(calls, rounds);
    let model = match &o.calibration {
        None => None,
        Some(path) => match std::fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|t| Calibration::parse(&t))
            .and_then(|c| Model::new(&kernels, &c).map(|m| (m, c.machine)))
        {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("mpfree-bench: calibration {path}: {e}");
                return ExitCode::from(1);
            }
        },
    };

    if o.afford {
        afford(o.quick, model.as_ref().map(|(m, _)| m), on_device);
        return ExitCode::SUCCESS;
    }

    // 20 s of audio per case (2 s with --quick), after a 50-block warm-up.
    let seconds = if o.quick { 2 } else { 20 };
    let blocks = (seconds * MPC.rate / MPC.frames) as u64;
    let period_ns = MPC.ns() as f64;
    println!("mpfree-bench on {}, build {}", machine(), build());
    println!("Period: {} frames at {} Hz = {:.0} us. {blocks} blocks per case. Percent = share of one period.",
             MPC.frames, MPC.rate, MPC.us());
    if !alloc::installed() {
        println!("(allocation counting is off)");
    }
    match (&model, on_device) {
        (_, true) => println!("Measured on this ARM machine: these are its own numbers, no estimate."),
        (Some((m, from)), false) => {
            let (g, lo, hi) = m.factor();
            println!("MPC estimate: this machine x {g:.1} (kernels range {lo:.1} to {hi:.1}), calibrated on {from}.");
        }
        (None, false) => println!("Desktop numbers only: pass --calibration FILE (from --calibrate on the MPC) for MPC estimates."),
    }
    println!();
    println!(
        "{:<50} {:>8} {:>8} {:>8} {:>7} {:>7} {:>8}  {}",
        "case",
        "mean us",
        "p99 us",
        "max us",
        "p99 %",
        "max %",
        "mem MB",
        if on_device || model.is_some() {
            "MPC worst vs module budget"
        } else {
            ""
        }
    );

    let mut rows = Vec::new();
    let (mut l, mut r) = (
        vec![0.0f32; MPC.frames as usize],
        vec![0.0f32; MPC.frames as usize],
    );
    for mut c in cases::all(MPC.rate) {
        if o.filter
            .as_ref()
            .is_some_and(|f| !c.name.to_lowercase().contains(f) && !c.module.contains(f.as_str()))
        {
            continue;
        }
        for b in 0..50 {
            (c.block)(b, &mut l, &mut r);
        }
        let mut probe = Probe::new(MPC.ns());
        for b in 50..50 + blocks {
            let span = Span::start();
            (c.block)(b, &mut l, &mut r);
            probe.stop(span, MPC.frames);
        }
        std::hint::black_box(&l);
        let s = probe.lifetime();
        let budget_pct = budget_pct(c.module);
        let verdict = |worst_ns: f64| {
            budget_pct.map_or(String::new(), |b| {
                let pct = MPC.pct(worst_ns);
                format!(
                    "{:>5.1} % of {:>4.1} % {}: {}",
                    pct,
                    b,
                    c.module,
                    Verdict::of(pct, b).word()
                )
            })
        };
        // On the device the verdict follows budget::judge (Jay's rule: an
        // engine on its p99.9 block, the whole period on its worst). A desktop
        // estimate always uses the p99.9: the single worst block on a desktop
        // is usually the OS, not the engine.
        let tail = if on_device {
            verdict(budget::judged_ns(c.module, &s) as f64)
        } else if let Some((m, _)) = &model {
            let (e, _, hi) = m.estimate(s.p999_ns as f64);
            format!("{} (up to {:.1} %)", verdict(e), MPC.pct(hi))
        } else {
            String::new()
        };
        println!(
            "{:<50} {:>8.1} {:>8.1} {:>8.1} {:>6.2}% {:>6.2}% {:>8.2}  {tail}{}",
            c.name,
            s.mean_ns / 1e3,
            s.p99_ns as f64 / 1e3,
            s.max_ns as f64 / 1e3,
            100.0 * s.p99_ns as f64 / period_ns,
            100.0 * s.max_ns as f64 / period_ns,
            mb(c.mem_bytes),
            if s.allocs > 0 {
                format!("  ALLOCATES ({} calls)", s.allocs)
            } else {
                String::new()
            }
        );
        rows.push(Row {
            name: c.name,
            module: c.module,
            what: c.what,
            s,
            mem_bytes: c.mem_bytes,
        });
    }

    println!();
    println!("Budgets (worst block, share of the period; see docs/project/profiling.md):");
    for b in budget::MPC_BUDGETS {
        println!(
            "  {:<8} {:>5.1} %  {:>6.0} us  {}",
            b.module,
            b.worst_pct,
            b.worst_ns(MPC) as f64 / 1e3,
            b.covers
        );
    }
    println!(
        "  whole audio thread {:.0} %",
        budget::AUDIO_THREAD_WORST_PCT
    );

    if let Some(path) = &o.json {
        if let Err(e) = std::fs::write(path, json(&rows, &kernels, blocks)) {
            eprintln!("mpfree-bench: {path}: {e}");
            return ExitCode::from(1);
        }
        println!("Wrote {path}");
    }
    let allocating: Vec<&str> = rows
        .iter()
        .filter(|r| r.s.allocs > 0)
        .map(|r| r.name.as_str())
        .collect();
    if o.check_allocs && !allocating.is_empty() {
        eprintln!(
            "mpfree-bench: allocated on the audio path: {}",
            allocating.join(", ")
        );
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reads_back_and_compares() {
        let s = |p99: u64, max: u64| Summary {
            blocks: 10,
            p99_ns: p99,
            max_ns: max,
            ..Summary::default()
        };
        let rows = [
            Row {
                name: "Chorus \"wide\" \\ 2".to_string(),
                module: "fx",
                what: "x",
                s: s(100_000, 200_000),
                mem_bytes: 4096,
            },
            Row {
                name: "Delay".to_string(),
                module: "fx",
                what: "y",
                s: s(50_000, 60_000),
                mem_bytes: 1 << 20,
            },
        ];
        let dir = std::env::temp_dir().join(format!("mpfree-bench-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b) = (dir.join("a.json"), dir.join("b.json"));
        std::fs::write(&a, json(&rows, &[("biquad".to_string(), 9.0)], 10)).unwrap();
        std::fs::write(&b, json(&rows[..1], &[], 10)).unwrap();
        let (machine, cases) = read_report(a.to_str().unwrap()).unwrap();
        assert!(machine.contains(", build "), "{machine}");
        assert_eq!(
            cases,
            vec![
                Saved {
                    name: "Chorus \"wide\" \\ 2".to_string(),
                    p99_ns: 100_000.0,
                    max_ns: 200_000.0,
                    mem_bytes: 4096.0
                },
                Saved {
                    name: "Delay".to_string(),
                    p99_ns: 50_000.0,
                    max_ns: 60_000.0,
                    mem_bytes: 1_048_576.0
                },
            ]
        );
        let t = compare(a.to_str().unwrap(), b.to_str().unwrap()).unwrap();
        assert!(
            t.contains("Chorus \"wide\" \\ 2")
                && t.contains("1.00")
                && t.contains("Delay")
                && t.contains("only in A"),
            "{t}"
        );
        assert!(read_report(dir.to_str().unwrap()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_song_counts_against_the_whole_thread() {
        assert_eq!(budget_pct("period"), Some(budget::AUDIO_THREAD_WORST_PCT));
        assert_eq!(budget_pct("synth"), Some(20.0));
        assert_eq!(budget_pct("nothing"), None);
    }
}
