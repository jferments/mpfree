//! mpfree-bench: what every MPFree audio engine costs at the MPC One's
//! period (128 frames at 44.1 kHz, 2.902 ms), against its budget.
//!
//! ```text
//! mpfree-bench [--quick] [--filter TEXT] [--calibration FILE] [--json FILE]
//!              [--check-allocs]
//! mpfree-bench --calibrate [--machine NAME]
//! mpfree-bench --compare A.json B.json
//! mpfree-bench --afford [--quick] [--filter TEXT] [--calibration FILE]
//! mpfree-bench --starts [--quick] [--filter TEXT] [--calibration FILE]
//! mpfree-bench --lanes [--lane-cores A,B] [--quick] [--json FILE]
//! mpfree-bench --count [--blocks N] [--filter TEXT]
//! mpfree-bench --soak [--blocks N] [--filter TEXT]
//! ```
//!
//! Any mode takes `--plain`: the NEON kernels (`mpfree-simd`) off, so every
//! engine runs their plain Rust twins, for the device round's A/B (the same
//! samples, timed both ways).
//!
//! - On the desktop it prints each case's mean, p99 and worst block. With
//!   `--calibration` (a file `--calibrate` printed on the MPC) it adds an
//!   estimate for the MPC and a verdict against the module's budget.
//! - On the MPC (an ARM build) the numbers are the device's own, and the
//!   verdicts follow `budget::judge`: an engine on its p99.9 block, the whole
//!   period on its worst.
//! - `--check-allocs` exits with status 2 if any engine allocated inside a
//!   block (the counting allocator is installed in this program). The
//!   step-edit cases (module `control`, refactor B1) are the time engine's
//!   control half, which may allocate, so they are left out.
//! - `--calibrate` times the reference kernels and prints a calibration file.
//! - `--compare A.json B.json` puts two `--json` reports side by side, case
//!   by case: B's p99 and worst as a share of A's (the default and the
//!   cortex-a17 build on the MPC, or a change against main).
//! - `--afford` measures each synth preset of every keyboard engine and the
//!   sampler at several voice counts, fits a fixed cost plus a cost per
//!   voice to the p99 blocks, and says how many voices fit in the module's
//!   budget: on this machine, as an MPC estimate with `--calibration`, or
//!   for real on the MPC. It also gives the worst block over the mean at
//!   the most voices. `--filter` keeps the engines whose name holds TEXT.
//! - `--starts` starts 16 notes in one block, from silence, on every
//!   preset of every keyboard engine, and says what that block costs
//!   against the blocks after it: the work a note does as it starts
//!   (tables, coefficients, resets), which shows as a worst block many
//!   times the mean. `--filter` keeps the presets whose name holds TEXT.
//! - The "mem MB" column is what each engine holds once it is built (its
//!   buffers, delay lines, sample memory), from the counting allocator.
//! - Every mode but `--count` turns the processor's flush-to-zero on first,
//!   as the app's audio thread and lanes do (`mpfree_lanes::fpu`), so a
//!   denormal costs here what it costs in the app; `--keep-denormals`
//!   leaves it off, to see what an engine's own flushing misses. `--count`
//!   leaves it off: callgrind counts the same either way, and doesn't
//!   model the mode.
//!
//! - `--lanes` runs only the lane cases: the same work on one core and split
//!   over two with a lane (`mpfree-lanes`), plus the handoff alone, and
//!   eight synth tracks in the track host on one, two and three cores. With
//!   `--lane-cores A,B` this thread pins itself to core A and each lane's
//!   worker to core B (on the MPC: 3,2), and the three-core case's second
//!   lane to the first core from 1 that is neither (1 on the MPC); without
//!   it nothing is pinned.
//! - `--count` runs each case for N blocks (200 by default) between two
//!   callgrind client requests, so a run under `valgrind --tool=callgrind`
//!   writes one count of instructions per case. `tools/ci/instr_gate.py`
//!   runs it and fails CI when a case costs more than 5 % above the
//!   baseline in `rust/mpfree-bench/instructions.txt`. Instruction counts
//!   don't drift with the machine's load, as times on a shared VM do.
//! - `--soak` runs each whole-period case (the song) for N blocks, 2^21 by
//!   default (about 101 minutes of sound), with the counting allocator
//!   watching every block, and exits with status 2 if any block allocated.
//!   It says which blocks did, and how many blocks were slower than the
//!   period. A long run finds what 20 s can't: a buffer that grows only
//!   when a count passes a power of two (#273: the write's gaps grew after
//!   51 minutes on the MPC). The nightly workflow runs it.
//!
//! It only reads and computes: it opens no device, writes no file unless
//! `--json` names one, and runs on one thread at normal priority. The one
//! exception is `--lanes`, which starts a worker thread per lane case (at
//! normal priority) and closes it before the next case; pinning, asked for
//! with `--lane-cores`, changes only this program's own threads and ends
//! with it.

mod cases;
#[cfg(test)]
mod golden;
mod vg;

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
    starts: bool,
    keep_denormals: bool,
    plain: bool,
    lanes: bool,
    lane_cores: Option<(usize, usize)>,
    count: bool,
    soak: bool,
    blocks: Option<u64>,
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
        starts: false,
        keep_denormals: false,
        plain: false,
        lanes: false,
        lane_cores: None,
        count: false,
        soak: false,
        blocks: None,
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
            "--starts" => o.starts = true,
            "--keep-denormals" => o.keep_denormals = true,
            "--plain" => o.plain = true,
            "--lanes" => o.lanes = true,
            "--count" => o.count = true,
            "--soak" => o.soak = true,
            "--blocks" => {
                let v = value("--blocks")?;
                o.blocks = Some(v.parse().ok().filter(|&n| n > 0).ok_or(format!(
                    "--blocks takes a number of blocks above 0 (not {v:?})"
                ))?);
            }
            "--lane-cores" => o.lane_cores = Some(lane_cores(&value("--lane-cores")?)?),
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

/// Blocks per case for `--count`: enough for a chord change, a few hits and
/// the looper's next pass to be in the count, few enough for callgrind
/// (about 50 times slower than the CPU) to count every case in minutes.
const COUNT_BLOCKS: u64 = 200;

/// `--count`: each case's blocks between a callgrind zero and a dump named
/// after the case, after the same 50-block warm-up the timings use. Under
/// callgrind each dump is that case's instructions; on a real CPU the
/// requests do nothing and this just runs the blocks.
fn count(o: &Options) {
    let blocks = o.blocks.unwrap_or(COUNT_BLOCKS);
    if vg::running() == 0 {
        eprintln!(
            "mpfree-bench: --count reports through callgrind; run it with tools/ci/instr_gate.py"
        );
    }
    println!("counting {blocks} blocks per case");
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
        // A step edit costs far more a block than an engine (a detent
        // compiles a full clip), and costs the same every block, so the
        // step cases count a tenth of the blocks after a short warm-up,
        // and the big cases one in 200 (`cases::blocks_for`); the
        // listing tells the gate how many each case counted.
        let (warm, n) = cases::blocks_for(&c, blocks);
        for b in 0..warm {
            (c.block)(b, &mut l, &mut r);
        }
        // The dump's name is made before the count starts: made between
        // the two requests, its allocation was counted as the case's, and
        // malloc's cost there depends on what the cases before left on the
        // heap (up to 87,000 instructions, 435 a block over 200 blocks).
        let why = vg::trigger(&format!("{COUNT_TAG}{}", c.name));
        vg::zero_stats();
        for b in warm..warm + n {
            (c.block)(b, &mut l, &mut r);
        }
        vg::dump_stats_at(&why);
        std::hint::black_box((&l, &r));
        println!("{}\t{n}", c.name);
    }
}

/// What each `--count` dump's trigger starts with, before the case's name.
const COUNT_TAG: &str = "mpfree-bench case: ";

/// Blocks per case for `--soak`: 2^21, past the 2^20 periods where a
/// doubling buffer would grow a 21st time, about 101 minutes of sound.
const SOAK_BLOCKS: u64 = 1 << 21;

/// A progress line every 2^18 blocks (about 12.7 minutes of sound).
const SOAK_PROGRESS: u64 = 1 << 18;

/// What a soak saw of one case.
#[derive(Debug, Default)]
struct Soak {
    blocks: u64,
    /// The worst block, ns, and which block it was.
    worst_ns: u64,
    worst_at: u64,
    /// Blocks slower than the period.
    slow: u64,
    /// Blocks that allocated or freed, their calls, and the first few
    /// (block, calls).
    alloc_blocks: u64,
    alloc_calls: u64,
    first_allocs: Vec<(u64, u64)>,
}

impl Soak {
    /// Block `b` took `ns` and made `calls` allocator calls.
    fn add(&mut self, b: u64, ns: u64, calls: u64, period_ns: u64) {
        self.blocks += 1;
        if ns > self.worst_ns {
            self.worst_ns = ns;
            self.worst_at = b;
        }
        if ns > period_ns {
            self.slow += 1;
        }
        if calls > 0 {
            self.alloc_blocks += 1;
            self.alloc_calls += calls;
            if self.first_allocs.len() < 8 {
                self.first_allocs.push((b, calls));
            }
        }
    }

    /// One line for the log.
    fn line(&self, name: &str, period_ns: u64) -> String {
        let minutes = self.blocks as f64 * MPC.frames as f64 / MPC.rate as f64 / 60.0;
        let allocs = if self.alloc_blocks == 0 {
            "no block allocated".to_string()
        } else {
            let first: Vec<String> = self
                .first_allocs
                .iter()
                .map(|(b, c)| format!("block {b} ({c} calls)"))
                .collect();
            format!(
                "{} blocks ALLOCATED, {} calls; first: {}",
                self.alloc_blocks,
                self.alloc_calls,
                first.join(", ")
            )
        };
        format!(
            "{name}: {} blocks ({minutes:.1} min of sound), worst {:.0} us ({:.0} % of the period) at block {}, \
             {} slower than the period; {allocs}",
            self.blocks,
            self.worst_ns as f64 / 1e3,
            100.0 * self.worst_ns as f64 / period_ns as f64,
            self.worst_at,
            self.slow
        )
    }
}

/// `--soak`: see the module docs. The cases are those of the whole period
/// (the song), or those `--filter` names.
fn soak(o: &Options) -> ExitCode {
    let blocks = o.blocks.unwrap_or(SOAK_BLOCKS);
    let period_ns = MPC.ns();
    println!("mpfree-bench --soak on {}, build {}", machine(), build());
    if !alloc::installed() {
        eprintln!("mpfree-bench: --soak needs the counting allocator");
        return ExitCode::from(1);
    }
    let (mut l, mut r) = (
        vec![0.0f32; MPC.frames as usize],
        vec![0.0f32; MPC.frames as usize],
    );
    let mut allocated = Vec::new();
    for mut c in cases::all(MPC.rate) {
        let wanted = match &o.filter {
            Some(f) => c.name.to_lowercase().contains(f) || c.module.contains(f.as_str()),
            None => c.module == "period",
        };
        if !wanted {
            continue;
        }
        println!("{}: {blocks} blocks", c.name);
        for b in 0..50 {
            (c.block)(b, &mut l, &mut r);
        }
        let mut s = Soak::default();
        let started = std::time::Instant::now();
        for b in 50..50 + blocks {
            let calls = alloc::thread_count();
            let span = Span::start();
            (c.block)(b, &mut l, &mut r);
            let ns = span.elapsed_ns();
            s.add(b, ns, alloc::thread_count() - calls, period_ns);
            if s.blocks % SOAK_PROGRESS == 0 && s.blocks < blocks {
                println!(
                    "  {} blocks in {:.0} s: worst {:.0} us, {} slow, {} allocated",
                    s.blocks,
                    started.elapsed().as_secs_f64(),
                    s.worst_ns as f64 / 1e3,
                    s.slow,
                    s.alloc_blocks
                );
            }
        }
        std::hint::black_box((&l, &r));
        println!(
            "{} (in {:.0} s)",
            s.line(&c.name, period_ns),
            started.elapsed().as_secs_f64()
        );
        if s.alloc_blocks > 0 {
            allocated.push(c.name);
        }
    }
    if allocated.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "mpfree-bench: allocated on the audio path in a long run: {}",
            allocated.join(", ")
        );
        ExitCode::from(2)
    }
}

/// `A,B`: this thread's core and the lanes' worker core.
/// The core for the three-core case's second lane: the first from 1 that
/// is neither this thread's nor the worker's (1 on the MPC, with 3,2).
fn third_core(here: usize, worker: usize) -> usize {
    (1..).find(|c| *c != here && *c != worker).unwrap_or(1)
}

fn lane_cores(text: &str) -> Result<(usize, usize), String> {
    let bad = || format!("--lane-cores takes two core numbers, such as 3,2 (not {text:?})");
    let (a, b) = text.split_once(',').ok_or_else(bad)?;
    let a = a.trim().parse().map_err(|_| bad())?;
    let b = b.trim().parse().map_err(|_| bad())?;
    if a == b {
        return Err(format!(
            "--lane-cores: the two cores must differ (both {a})"
        ));
    }
    Ok((a, b))
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

/// The sample voices the grain budget leaves room for in the sampler's
/// share (a kit's worth playing at once).
const A6_SAMPLE_VOICES: usize = 16;
/// The scaling cases the grain budget is worked out from.
const A6_GRAINS: &str = "Sampler grains (per grain)";
const A6_VOICES: &str = "Sampler, looping pads held";

/// R7 step A6, from the `--afford` fits (ns, on the MPC): the grains that
/// fit in the sampler's share `budget_ns` beside [`A6_SAMPLE_VOICES`]
/// sample voices, rounded down to 16 (the engine's `GRAIN_BUDGET` is set
/// from it), and what a sample voice costs in grains (the engine's
/// `VOICE_COST`). `grains` and `voices` are (fixed, per voice). None if
/// a grain's cost isn't measured.
fn grain_budget(budget_ns: f64, grains: (f64, f64), voices: (f64, f64)) -> Option<(usize, f64)> {
    let (fixed, per_grain) = grains;
    if per_grain <= 0.0 || !per_grain.is_finite() {
        return None;
    }
    let room = budget_ns - fixed.max(0.0) - A6_SAMPLE_VOICES as f64 * voices.1.max(0.0);
    let n = (room / per_grain).max(0.0).floor() as usize;
    Some((n / 16 * 16, voices.1.max(0.0) / per_grain))
}

/// The `--afford` table: cost per voice and voices per budget.
fn afford(quick: bool, filter: Option<&str>, model: Option<&Model>, on_device: bool) {
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
        "{:<36} {:>9} {:>11}  {:<40}  {:>10}  {:>8}  voices that fit",
        "engine", "fixed us", "us / voice", "p99 us at each voice count", "worst/mean", "budget"
    );
    // (fixed, per voice) of the cases the grain budget needs, scaled
    // (with a filter that leaves them out, there is no budget line).
    let (mut grains, mut sample_voices, mut sampler_ns) = (None, None, 0.0);
    let wanted = |name: &str| filter.is_none_or(|f| name.to_lowercase().contains(f));
    for mut sc in cases::scaling(MPC.rate)
        .into_iter()
        .filter(|s| wanted(&s.name))
    {
        let mut pts = Vec::new();
        // The worst block against the mean at the most voices: how much
        // room spikes take that steady work doesn't.
        let mut spike = 1.0;
        for (n, c) in sc.points.iter_mut() {
            let sum = time_case(c, blocks);
            spike = sum.max_ns as f64 / sum.mean_ns.max(1.0);
            pts.push((*n, sum.p99_ns as f64));
        }
        let (a, b) = cases::fit(&pts);
        let budget = budget::for_module(sc.module).expect("module budget");
        let budget_ns = budget.worst_ns(MPC) as f64;
        if sc.name == A6_GRAINS {
            (grains, sampler_ns) = (Some((a * factor, b * factor)), budget_ns);
        } else if sc.name == A6_VOICES {
            sample_voices = Some((a * factor, b * factor));
        }
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
            "{:<36} {:>9.1} {:>11}  {:<40}  {:>10.1}  {:>4.0} us  {}{range}",
            sc.name,
            a * factor / 1e3,
            per_voice,
            counts,
            spike,
            budget_ns / 1e3,
            show(fits(factor))
        );
    }
    println!();
    println!("\"voices that fit\" is for that engine alone in its whole module budget (synth 20 %, sampler 25 % of a period, worst block);");
    println!("several synth tracks share the synth budget. \"16+ (87)\": every count measured fits, and the line runs out at 87 (Poly plays");
    println!("at most 16). A preset whose cost does not grow with voices shows a per-voice cost near 0: its p99 is set by something else");
    println!("(an organ's wheels turn whatever is held). \"worst/mean\" is the worst block over the mean at the most voices measured.");
    println!("In \"Sampler grains (per grain)\" a voice is one live grain (4 granular pads held, 16 to 256 grains alive): \"us / voice\" is");
    println!(
        "the cost of a grain, and 256 is the engine's grain budget, the most a burst can reach."
    );
    if let (Some(g), Some(v)) = (grains, sample_voices) {
        if let Some((n, voice_cost)) = grain_budget(sampler_ns, g, v) {
            println!();
            println!(
                "Grain budget (R7 step A6), {place}: a grain {:.2} us, a looping sample voice {:.2} us (about {voice_cost:.1} grains; \
                 the engine's VOICE_COST is {}).",
                g.1 / 1e3,
                v.1 / 1e3,
                mpfree_sampler::engine::VOICE_COST
            );
            println!(
                "grain budget: {n} grains fit in the sampler's {:.0} us beside {A6_SAMPLE_VOICES} sample voices (GRAIN_BUDGET is {} now)",
                sampler_ns / 1e3,
                mpfree_sampler::engine::GRAIN_BUDGET
            );
        }
    }
}

/// `--starts`: see the module docs.
fn starts(
    quick: bool,
    filter: Option<&str>,
    model: Option<&Model>,
    on_device: bool,
    flushing: bool,
) {
    let rounds = if quick { 8 } else { 32 };
    let factor = model.map_or(1.0, |m| m.factor().0);
    let place = if on_device {
        "on this MPC"
    } else if model.is_some() {
        "on the MPC (estimate)"
    } else {
        "on this machine"
    };
    let budget_ns = budget::for_module("synth")
        .expect("module budget")
        .worst_ns(MPC) as f64;
    println!("mpfree-bench --starts on {}, build {}", machine(), build());
    println!(
        "{} notes start on a block's first frame, from silence, {rounds} times a preset (medians). Times are {place}{}. \
         Denormals {}.",
        cases::START_NOTES,
        if model.is_some() && !on_device {
            format!(" (this machine x {factor:.1})")
        } else {
            String::new()
        },
        if flushing {
            "flushed to zero, as in the app"
        } else {
            "kept"
        }
    );
    println!();
    println!(
        "{:<40} {:>9} {:>9} {:>10} {:>13} {:>10} {:>10}",
        "preset", "start us", "held us", "start/held", "us per start", "worst us", "change us"
    );
    let wanted = |name: &str| filter.is_none_or(|f| name.to_lowercase().contains(f));
    let mut rows = Vec::new();
    for p in cases::keyboard_presets(MPC.rate)
        .into_iter()
        .filter(|p| wanted(&p.name))
    {
        let c = cases::start_cost(&p, MPC.rate, MPC.frames as usize, rounds);
        let notes = if p.mono { 1 } else { cases::START_NOTES };
        let per_start = (c.start_ns - c.held_ns).max(0.0) / notes as f64;
        let ratio = c.start_ns / c.held_ns.max(1.0);
        let over = if c.worst_start_ns * factor > budget_ns {
            "  over the synth budget"
        } else {
            ""
        };
        println!(
            "{:<40} {:>9.1} {:>9.1} {:>10.1} {:>13.2} {:>10.1} {:>10.1}{over}",
            format!("{}{}", p.name, if p.mono { " (mono)" } else { "" }),
            c.start_ns * factor / 1e3,
            c.held_ns * factor / 1e3,
            ratio,
            per_start * factor / 1e3,
            c.worst_start_ns * factor / 1e3,
            c.change_ns * factor / 1e3,
        );
        rows.push((p.name, ratio, c));
    }
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    println!();
    println!(
        "\"start/held\" is the start block over the blocks after it; \"us per start\" is the difference shared by the notes \
         started (one on a mono preset). \"worst us\" is the worst start block, against the synth budget ({:.0} us, the worst \
         block of a whole module). \"change us\" is the next chord change: every note ends and as many start a fifth up.",
        budget_ns / 1e3
    );
    println!("Most costly starts against their held blocks:");
    for (name, ratio, c) in rows.iter().take(10) {
        println!(
            "  {name}: {ratio:.1} x ({:.1} us against {:.1} us)",
            c.start_ns * factor / 1e3,
            c.held_ns * factor / 1e3
        );
    }
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
                       mpfree-bench --afford [--quick] [--filter TEXT] [--calibration FILE]\n       \
                       mpfree-bench --starts [--quick] [--filter TEXT] [--calibration FILE]\n       \
                       mpfree-bench --lanes [--lane-cores A,B] [--quick] [--json FILE]\n       \
                       mpfree-bench --count [--blocks N] [--filter TEXT]\n       \
                       mpfree-bench --soak [--blocks N] [--filter TEXT]");
            eprintln!("       any mode: --plain runs the NEON kernels' plain twins (mpfree-simd)");
            return ExitCode::from(if e.is_empty() { 0 } else { 1 });
        }
    };
    if o.plain {
        mpfree_simd::set_enabled(false);
    }
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
    if o.count {
        count(&o);
        return ExitCode::SUCCESS;
    }
    let flushing = !o.keep_denormals && mpfree_lanes::fpu::flush_to_zero();
    if o.soak {
        return soak(&o);
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
        afford(
            o.quick,
            o.filter.as_deref(),
            model.as_ref().map(|(m, _)| m),
            on_device,
        );
        return ExitCode::SUCCESS;
    }
    if o.starts {
        starts(
            o.quick,
            o.filter.as_deref(),
            model.as_ref().map(|(m, _)| m),
            on_device,
            flushing,
        );
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
    println!(
        "NEON kernels: {}{}",
        mpfree_simd::status(),
        if o.plain { " (--plain)" } else { "" }
    );
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
    let list = if o.lanes {
        if let Some((here, _)) = o.lane_cores {
            match mpfree_lanes::os::pin(here) {
                Ok(()) => println!("This thread: pinned to core {here}."),
                Err(e) => println!("This thread: not pinned to core {here} ({e})."),
            }
        }
        cases::lanes(
            MPC.rate,
            o.lane_cores.map(|(_, worker)| worker),
            o.lane_cores.map(|(here, worker)| third_core(here, worker)),
        )
    } else {
        cases::all(MPC.rate)
    };
    for mut c in list {
        if o.filter
            .as_ref()
            .is_some_and(|f| !c.name.to_lowercase().contains(f) && !c.module.contains(f.as_str()))
        {
            continue;
        }
        // The big-song cases take a fifth of a second a block, the note
        // editor's biggest edit about a second: a few do.
        let (warm, n) = if cases::big(&c) {
            cases::blocks_for(&c, blocks)
        } else {
            (50, blocks)
        };
        for b in 0..warm {
            (c.block)(b, &mut l, &mut r);
        }
        let mut probe = Probe::new(MPC.ns());
        for b in warm..warm + n {
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
    println!("Budgets (worst block, share of the period; see docs/dev/design/profiling.md):");
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
    // The step edits are the control half's and the Song view's frame the
    // screen's, which may allocate (they show ALLOCATES all the same).
    let allocating: Vec<&str> = rows
        .iter()
        .filter(|r| r.s.allocs > 0 && r.module != cases::CONTROL && r.module != cases::SCREEN)
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
    fn the_grain_budget_leaves_room_for_a_kit_and_rounds_down_to_16() {
        // 725 us for the sampler; grains 100 us fixed and 1.5 us each;
        // a sample voice 12 us (8 grains): 725 - 100 - 192 = 433 us,
        // 288 grains, 288 exactly.
        let (n, cost) = grain_budget(725_000.0, (100_000.0, 1_500.0), (0.0, 12_000.0)).unwrap();
        assert_eq!((n, cost), (288, 8.0));
        let (n, _) = grain_budget(725_000.0, (100_000.0, 1_600.0), (0.0, 12_000.0)).unwrap();
        assert_eq!(n, 256, "270 rounds down");
        // Nothing left: none, never less.
        let (n, _) = grain_budget(100_000.0, (90_000.0, 1_500.0), (0.0, 12_000.0)).unwrap();
        assert_eq!(n, 0);
        assert!(grain_budget(725_000.0, (100_000.0, 0.0), (0.0, 12_000.0)).is_none());
    }

    #[test]
    fn a_soak_counts_slow_blocks_and_the_first_that_allocate() {
        let period = MPC.ns();
        let mut s = Soak::default();
        s.add(50, 100_000, 0, period);
        s.add(51, period + 1, 0, period);
        for b in 52..62 {
            s.add(b, 1_000, 2, period);
        }
        assert_eq!((s.blocks, s.slow, s.worst_at), (12, 1, 51));
        assert_eq!((s.alloc_blocks, s.alloc_calls), (10, 20));
        assert_eq!(s.first_allocs.len(), 8);
        assert_eq!(s.first_allocs[0], (52, 2));
        let line = s.line("Song", period);
        assert!(line.starts_with("Song: 12 blocks (0.0 min of sound), worst 2902 us (100 % of the period) at block 51, 1 slower than the period; 10 blocks ALLOCATED, 20 calls; first: block 52 (2 calls), "), "{line}");
        let quiet = Soak {
            blocks: 1 << 21,
            ..Soak::default()
        };
        assert!(quiet.line("Song", period).contains("(101.4 min of sound)"));
        assert!(quiet.line("Song", period).ends_with("no block allocated"));
    }

    #[test]
    fn the_third_lane_core_is_neither_of_the_two_nor_core_0() {
        assert_eq!(third_core(3, 2), 1);
        assert_eq!(third_core(1, 2), 3);
        assert_eq!(third_core(2, 1), 3);
        assert_eq!(third_core(0, 1), 2);
    }

    #[test]
    fn lane_cores_read_two_different_cores() {
        assert_eq!(lane_cores("3,2"), Ok((3, 2)));
        assert_eq!(lane_cores(" 3 , 2 "), Ok((3, 2)));
        assert!(lane_cores("3").is_err());
        assert!(lane_cores("3,x").is_err());
        assert!(lane_cores("2,2").is_err());
    }

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
