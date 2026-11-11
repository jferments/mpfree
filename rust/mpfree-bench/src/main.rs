//! mpfree-bench: what every MPFree audio engine costs at the MPC One's
//! period (128 frames at 44.1 kHz, 2.902 ms), against its budget.
//!
//! ```text
//! mpfree-bench [--quick] [--filter TEXT] [--calibration FILE] [--json FILE]
//!              [--check-allocs]
//! mpfree-bench --calibrate [--machine NAME]
//! ```
//!
//! - On the desktop it prints each case's mean, p99 and worst block. With
//!   `--calibration` (a file `--calibrate` printed on the MPC) it adds an
//!   estimate for the MPC and a verdict against the module's budget.
//! - On the MPC (an ARM build) the numbers are the device's own, and the
//!   verdicts use them directly.
//! - `--check-allocs` exits with status 2 if any engine allocated inside a
//!   block (the counting allocator is installed in this program).
//! - `--calibrate` times the reference kernels and prints a calibration file.
//! - The "mem MB" column is what each engine holds once it is built (its
//!   buffers, delay lines, sample memory), from the counting allocator.
//!
//! It only reads and computes: it opens no device, writes no file unless
//! `--json` names one, and runs on one thread at normal priority.

mod cases;

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
}

fn options() -> Result<Options, String> {
    let mut o = Options { quick: false, filter: None, calibration: None, json: None, check_allocs: false, calibrate: false, machine: None };
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
    let mhz = std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq").ok()
        .and_then(|t| t.trim().parse::<u32>().ok()).map(|k| format!(", {} MHz", k / 1000)).unwrap_or_default();
    format!("{} ({}{mhz})", cpu.unwrap_or_else(|| "unknown CPU".to_string()), std::env::consts::ARCH)
}

fn build() -> String {
    // Arm target features cannot be read with cfg! on stable Rust, so the
    // flags are recorded at build time (build.rs).
    let flags = env!("MPFREE_BENCH_RUSTFLAGS");
    format!("{}{}", if cfg!(debug_assertions) { "debug" } else { "release" },
            if flags.is_empty() { String::new() } else { format!(", RUSTFLAGS {flags}") })
}

struct Row {
    name: String,
    module: &'static str,
    what: &'static str,
    s: Summary,
    mem_bytes: i64,
}

/// Megabytes (2^20 bytes), for the memory column.
fn mb(bytes: i64) -> f64 { bytes as f64 / (1024.0 * 1024.0) }

fn esc(s: &str) -> String { s.replace('\\', "\\\\").replace('"', "\\\"") }

fn json(rows: &[Row], kernels: &[(String, f64)], blocks: u64) -> String {
    let mut j = String::new();
    let _ = write!(j, "{{\n  \"schema\": \"mpfree-bench/1\",\n  \"machine\": \"{}\",\n  \"arch\": \"{}\",\n  \"build\": \"{}\",\n  \
                       \"period\": {{\"frames\": {}, \"rate\": {}}},\n  \"blocks\": {blocks},\n  \"kernels\": [",
                   esc(&machine()), std::env::consts::ARCH, build(), MPC.frames, MPC.rate);
    for (i, (k, ns)) in kernels.iter().enumerate() {
        let _ = write!(j, "{}\n    {{\"name\": \"{k}\", \"ns\": {ns:.1}}}", if i > 0 { "," } else { "" });
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

fn main() -> ExitCode {
    let o = match options() {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() { eprintln!("mpfree-bench: {e}"); }
            eprintln!("usage: mpfree-bench [--quick] [--filter TEXT] [--calibration FILE] [--json FILE] [--check-allocs]\n       \
                       mpfree-bench --calibrate [--machine NAME]");
            return ExitCode::from(if e.is_empty() { 0 } else { 1 });
        }
    };
    let (calls, rounds) = if o.quick { (200, 5) } else { (2000, 15) };

    if o.calibrate {
        let cal = Calibration { machine: format!("{}, build {}", o.machine.unwrap_or_else(machine), build()), kernels: model::measure(calls, rounds) };
        print!("{}", cal.to_text());
        // The clock's own cost, so tiny blocks can be read with care.
        let t = std::time::Instant::now();
        for _ in 0..100_000 { std::hint::black_box(Span::start().elapsed_ns()); }
        println!("# a span (two clock reads) costs {:.0} ns", t.elapsed().as_nanos() as f64 / 100_000.0);
        return ExitCode::SUCCESS;
    }

    let on_device = cfg!(target_arch = "arm");
    let kernels = model::measure(calls, rounds);
    let model = match &o.calibration {
        None => None,
        Some(path) => match std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| Calibration::parse(&t)).and_then(|c| Model::new(&kernels, &c).map(|m| (m, c.machine))) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("mpfree-bench: calibration {path}: {e}");
                return ExitCode::from(1);
            }
        },
    };

    // 20 s of audio per case (2 s with --quick), after a 50-block warm-up.
    let seconds = if o.quick { 2 } else { 20 };
    let blocks = (seconds * MPC.rate / MPC.frames) as u64;
    let period_ns = MPC.ns() as f64;
    println!("mpfree-bench on {}, build {}", machine(), build());
    println!("Period: {} frames at {} Hz = {:.0} us. {blocks} blocks per case. Percent = share of one period.",
             MPC.frames, MPC.rate, MPC.us());
    if !alloc::installed() { println!("(allocation counting is off)"); }
    match (&model, on_device) {
        (_, true) => println!("Measured on this ARM machine: these are its own numbers, no estimate."),
        (Some((m, from)), false) => {
            let (g, lo, hi) = m.factor();
            println!("MPC estimate: this machine x {g:.1} (kernels range {lo:.1} to {hi:.1}), calibrated on {from}.");
        }
        (None, false) => println!("Desktop numbers only: pass --calibration FILE (from --calibrate on the MPC) for MPC estimates."),
    }
    println!();
    println!("{:<50} {:>8} {:>8} {:>8} {:>7} {:>7} {:>8}  {}", "case", "mean us", "p99 us", "max us", "p99 %", "max %", "mem MB",
             if on_device || model.is_some() { "MPC worst vs module budget" } else { "" });

    let mut rows = Vec::new();
    let (mut l, mut r) = (vec![0.0f32; MPC.frames as usize], vec![0.0f32; MPC.frames as usize]);
    for mut c in cases::all(MPC.rate) {
        if o.filter.as_ref().is_some_and(|f| !c.name.to_lowercase().contains(f) && !c.module.contains(f.as_str())) { continue; }
        for b in 0..50 { (c.block)(b, &mut l, &mut r); }
        let mut probe = Probe::new(MPC.ns());
        for b in 50..50 + blocks {
            let span = Span::start();
            (c.block)(b, &mut l, &mut r);
            probe.stop(span, MPC.frames);
        }
        std::hint::black_box(&l);
        let s = probe.lifetime();
        let budget = budget::for_module(c.module);
        let verdict = |worst_ns: f64| budget.map_or(String::new(), |b| {
            let pct = MPC.pct(worst_ns);
            format!("{:>5.1} % of {:>4.1} % {}: {}", pct, b.worst_pct, c.module, Verdict::of(pct, b.worst_pct).word())
        });
        // The p99.9 block is the "worst" for verdicts: the single worst block
        // on a desktop is usually the OS, not the engine. On the device both
        // are shown and the max is what underruns.
        let tail = if on_device {
            verdict(s.max_ns as f64)
        } else if let Some((m, _)) = &model {
            let (e, _, hi) = m.estimate(s.p999_ns as f64);
            format!("{} (up to {:.1} %)", verdict(e), MPC.pct(hi))
        } else {
            String::new()
        };
        println!("{:<50} {:>8.1} {:>8.1} {:>8.1} {:>6.2}% {:>6.2}% {:>8.2}  {tail}{}", c.name, s.mean_ns / 1e3, s.p99_ns as f64 / 1e3,
                 s.max_ns as f64 / 1e3, 100.0 * s.p99_ns as f64 / period_ns, 100.0 * s.max_ns as f64 / period_ns, mb(c.mem_bytes),
                 if s.allocs > 0 { format!("  ALLOCATES ({} calls)", s.allocs) } else { String::new() });
        rows.push(Row { name: c.name, module: c.module, what: c.what, s, mem_bytes: c.mem_bytes });
    }

    println!();
    println!("Budgets (worst block, share of the period; see docs/project/profiling.md):");
    for b in budget::MPC_BUDGETS {
        println!("  {:<8} {:>5.1} %  {:>6.0} us  {}", b.module, b.worst_pct, b.worst_ns(MPC) as f64 / 1e3, b.covers);
    }
    println!("  whole audio thread {:.0} %", budget::AUDIO_THREAD_WORST_PCT);

    if let Some(path) = &o.json {
        if let Err(e) = std::fs::write(path, json(&rows, &kernels, blocks)) {
            eprintln!("mpfree-bench: {path}: {e}");
            return ExitCode::from(1);
        }
        println!("Wrote {path}");
    }
    let allocating: Vec<&str> = rows.iter().filter(|r| r.s.allocs > 0).map(|r| r.name.as_str()).collect();
    if o.check_allocs && !allocating.is_empty() {
        eprintln!("mpfree-bench: allocated on the audio path: {}", allocating.join(", "));
        return ExitCode::from(2);
    }
    ExitCode::SUCCESS
}
