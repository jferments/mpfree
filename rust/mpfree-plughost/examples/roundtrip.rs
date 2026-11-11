//! What the sandbox costs per block: the test gain at the MPC's period (128
//! frames at 44.1 kHz), in-process and sandboxed, and the difference. The
//! difference is the round trip (wake, two context switches, the copies);
//! a real plugin's own work comes on top either way.
//!
//! `cargo run --release -p mpfree-plughost --example roundtrip`
//! (from `rust/`). Finds `mpfree-plughost` and the test plugins' library in
//! the build folder; `--exe FILE` and `--plugin FILE` name them otherwise.
//! Touches nothing outside its own processes.

use mpfree_clap::testplug::GAIN_ID;
use mpfree_clap::Library;
use mpfree_node::{Ctx, Node};
use mpfree_plughost::{spawn, Deadline};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RATE: u32 = 44_100;
const FRAMES: usize = 128;
const WARM: usize = 500;
const BLOCKS: usize = 20_000;

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    [dir.to_path_buf(), dir.join("deps"), dir.join("..")]
        .into_iter()
        .map(|d| d.join(name))
        .find(|p| p.exists())
}

/// Microseconds per block: mean, median, 99th percentile, worst.
fn time(n: &mut dyn Node) -> (f64, f64, f64, f64) {
    let (mut l, mut r) = (vec![0.1f32; FRAMES], vec![-0.1f32; FRAMES]);
    let mut us = Vec::with_capacity(BLOCKS);
    for b in 0..WARM + BLOCKS {
        l.fill(0.1);
        r.fill(-0.1);
        let c = Ctx::new(RATE, (b * FRAMES) as u64);
        let t = Instant::now();
        n.process(&c, &[], &mut l, &mut r);
        if b >= WARM {
            us.push(t.elapsed().as_nanos() as f64 / 1000.0);
        }
    }
    let mean = us.iter().sum::<f64>() / us.len() as f64;
    us.sort_by(f64::total_cmp);
    let at = |q: f64| us[((us.len() - 1) as f64 * q) as usize];
    (mean, at(0.5), at(0.99), us[us.len() - 1])
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut exe, mut plugin) = (None, None);
    while let Some(a) = args.next() {
        match (a.as_str(), args.next()) {
            ("--exe", Some(v)) => exe = Some(PathBuf::from(v)),
            ("--plugin", Some(v)) => plugin = Some(PathBuf::from(v)),
            _ => {
                eprintln!("usage: roundtrip [--exe mpfree-plughost] [--plugin libmpfree_clap_testplug.so]");
                std::process::exit(64);
            }
        }
    }
    let here = std::env::current_exe().expect("own path");
    let dir = here.parent().and_then(Path::parent).expect("build folder");
    let lib_name = format!(
        "{}mpfree_clap_testplug{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let exe = exe
        .or_else(|| find(dir, "mpfree-plughost"))
        .expect("mpfree-plughost not found: build it, or pass --exe");
    let plugin = plugin
        .or_else(|| find(dir, &lib_name))
        .expect("test plugin library not found: pass --plugin");

    let mut local = Library::open(&plugin)
        .expect("open")
        .create(GAIN_ID)
        .expect("gain");
    local.prepare(RATE, FRAMES);
    let (mut boxed, sandbox) = spawn(&exe, &plugin, GAIN_ID).expect("spawn");
    // Generous, so every block is measured to its end rather than cut off.
    boxed.set_deadline(Deadline::Fixed(Duration::from_secs(1)));
    boxed.prepare(RATE, FRAMES);

    let period_us = FRAMES as f64 / f64::from(RATE) * 1e6;
    println!("The test gain, {FRAMES} frames at {RATE} Hz (period {period_us:.0} us), {BLOCKS} blocks each, {}.", if cfg!(debug_assertions) { "debug build" } else { "release build" });
    println!(
        "{:<14} {:>9} {:>9} {:>9} {:>9} {:>11}",
        "", "mean us", "p50 us", "p99 us", "max us", "p99 % period"
    );
    let a = time(&mut local);
    let b = time(&mut boxed);
    for (name, t) in [("in-process", a), ("sandboxed", b)] {
        println!(
            "{name:<14} {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>11.1}",
            t.0,
            t.1,
            t.2,
            t.3,
            t.2 / period_us * 100.0
        );
    }
    println!(
        "{:<14} {:>9.1} {:>9.1} {:>9.1}",
        "round trip",
        b.0 - a.0,
        b.1 - a.1,
        b.2 - a.2
    );
    let s = boxed.stats();
    println!(
        "sandboxed blocks {}, misses {}, busy {}, down {}",
        s.blocks, s.misses, s.busy, s.down
    );
    drop(boxed);
    drop(sandbox);
}
