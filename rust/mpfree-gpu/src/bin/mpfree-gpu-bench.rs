//! mpfree-gpu-bench: what drawing on the GPU costs, against software, on
//! the machine it runs on (the MPC One for the performance plan's step G2).
//!
//!   mpfree-gpu-bench describe
//!       the GPU, its OpenGL ES version and EGL's, and whether OpenGL ES 3.1
//!       (compute) is there
//!   mpfree-gpu-bench screens DIR [--frames N]
//!       plays every NAME.mpdl in DIR (from `mpfree --display-lists DIR`) on
//!       the GPU and in software at the MPC's size (an 800 x 1280 panel
//!       turned 270 degrees), and times each page: its first frame (with
//!       every upload), the whole page again (the frame a page change
//!       costs), the redraw a tick later, and reading the frame back; and
//!       checks the GPU's picture against the software one
//!   mpfree-gpu-bench compute [--iterations N]
//!       a GPU compute round trip (an FIR filter over a stereo block: upload,
//!       run, read back) against the same job on one CPU core
//!
//! It draws offscreen only (Mesa's surfaceless EGL, a render node): it opens
//! no display, takes no DRM master and changes no display setting. It reads
//! DIR and writes nothing; everything goes to stdout. A failure prints
//! `STOP: reason` and exits with status 1.

use std::path::{Path, PathBuf};
use std::time::Instant;

use mpfree_gpu::{compare, compute, Gpu, Renderer};
use mpfree_screen_log::{Geometry, Rotation};
use mpfree_ui::fb::FbLayout;
use mpfree_ui::record::{replay, Frame, Store};
use mpfree_ui::SoftCanvas;

/// The MPC One's panel: 800 x 1280, 32 bits a pixel, turned 270 degrees.
const NATIVE: (usize, usize) = (800, 1280);
const ROTATION: Rotation = Rotation::Deg270;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        println!("STOP: {e}");
        std::process::exit(1);
    }
}

fn number(args: &[String], flag: &str, default: usize, max: usize) -> Result<usize, String> {
    match args.iter().position(|a| a == flag) {
        None => Ok(default),
        Some(i) => {
            let v: usize = args
                .get(i + 1)
                .and_then(|v| v.parse().ok())
                .ok_or_else(|| format!("{flag} needs a whole number"))?;
            if v == 0 || v > max {
                return Err(format!("{flag} must be 1 to {max}"));
            }
            Ok(v)
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let usage =
        "usage: mpfree-gpu-bench describe | screens DIR [--frames N] | compute [--iterations N]";
    match args.first().map(String::as_str) {
        Some("describe") if args.len() == 1 => describe(),
        Some("screens") => {
            let dir = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let known = |a: &String| a == "--frames" || a.parse::<usize>().is_ok();
            if let Some(a) = args[2..].iter().find(|a| !known(a)) {
                return Err(format!("unknown argument {a}"));
            }
            screens(Path::new(dir), number(args, "--frames", 30, 1000)?)
        }
        Some("compute") => {
            if let Some(a) = args[1..]
                .iter()
                .find(|a| *a != "--iterations" && a.parse::<usize>().is_err())
            {
                return Err(format!("unknown argument {a}"));
            }
            compute_run(number(args, "--iterations", 500, 100_000)?)
        }
        _ => Err(usage.into()),
    }
}

fn describe() -> Result<(), String> {
    let gpu = Gpu::surfaceless()?;
    println!("gpu: {}", gpu.describe());
    drop(gpu);
    match Gpu::surfaceless_es(3, 1) {
        Ok(g) => println!("gpu: OpenGL ES 3.1 (compute): yes: {}", g.describe()),
        Err(e) => println!("gpu: OpenGL ES 3.1 (compute): no: {e}"),
    }
    Ok(())
}

/// Milliseconds, sorted, and their 50th and 95th percentiles and maximum.
struct Times(Vec<f64>);

impl Times {
    fn new() -> Times {
        Times(Vec::new())
    }

    fn time<T>(&mut self, f: impl FnOnce() -> T) -> T {
        let t = Instant::now();
        let r = f();
        self.0.push(t.elapsed().as_secs_f64() * 1e3);
        r
    }

    fn at(&self, p: f64) -> f64 {
        let mut v = self.0.clone();
        v.sort_by(f64::total_cmp);
        if v.is_empty() {
            return 0.0;
        }
        v[(((v.len() - 1) as f64) * p).round() as usize]
    }
}

struct Page {
    name: String,
    gpu_full: f64,
    gpu_tick: f64,
    cpu_full: f64,
    cpu_tick: f64,
}

fn screens(dir: &Path, frames: usize) -> Result<(), String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "mpdl"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no .mpdl files in {}", dir.display()));
    }
    let gpu = Gpu::surfaceless()?;
    println!("gpu: {}", gpu.describe());
    let geometry = Geometry::new(NATIVE.0, NATIVE.1, NATIVE.0 * 4, 32)?;
    let layout = FbLayout {
        geometry,
        rotation: ROTATION,
    };
    let (w, h) = layout.logical_size();
    println!(
        "screens: {} pages, {} frames each timed; panel {}x{} turned 270, pages {w}x{h}; times in ms",
        files.len(),
        frames,
        NATIVE.0,
        NATIVE.1
    );
    println!(
        "{:<24} {:>5} {:>5} {:>5} | {:>6} {:>6} {:>6} {:>6} {:>6} | {:>6} | {:>6} {:>6} | {:>6} {:>4}",
        "page",
        "ops",
        "draws",
        "items",
        "first",
        "full50",
        "full95",
        "fullmx",
        "tick50",
        "read50",
        "sw50",
        "swtick",
        "differ",
        "max"
    );
    let started = Instant::now();
    let mut pages = Vec::new();
    let mut worst_diff = 0u32;
    for path in &files {
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let list = Frame::decode_all(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let (first, tick) = match list.as_slice() {
            [a, b, ..] => (a, b),
            _ => return Err(format!("{}: needs two frames", path.display())),
        };
        if (first.w, first.h) != (w, h) {
            return Err(format!(
                "{}: a {}x{} page; this bench draws {w}x{h}",
                path.display(),
                first.w,
                first.h
            ));
        }
        // The GPU: the first frame with its uploads, then the whole page and
        // the tick's redraw again and again, each to the end of the GPU's work.
        let mut store = Store::new(usize::MAX);
        let mut r = Renderer::new(&gpu, NATIVE.0 as i32, NATIVE.1 as i32, ROTATION)?;
        store.apply(first);
        let mut t_first = Times::new();
        let drawn = t_first.time(|| {
            let s = r.play(&gpu, &first.ops, &store, &first.dropped);
            r.finish(&gpu);
            s
        });
        let got = r.read_native(&gpu);
        let mut soft = SoftCanvas::new(w, h);
        replay(&first.ops, &store, &mut soft);
        let mut native = Vec::new();
        layout.rows(&soft, 0, NATIVE.1, &mut native);
        let want: Vec<u32> = native
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b) & 0xFF_FFFF)
            .collect();
        let d = compare(&want, &got);
        worst_diff = worst_diff.max(d.max);
        store.apply(tick);
        r.play(&gpu, &tick.ops, &store, &tick.dropped);
        r.finish(&gpu);
        let (mut full, mut redraw, mut read) = (Times::new(), Times::new(), Times::new());
        for _ in 0..frames {
            full.time(|| {
                r.play(&gpu, &first.ops, &store, &[]);
                r.finish(&gpu);
            });
            redraw.time(|| {
                r.play(&gpu, &tick.ops, &store, &[]);
                r.finish(&gpu);
            });
        }
        for _ in 0..frames.div_ceil(4) {
            read.time(|| std::hint::black_box(r.read_native(&gpu)));
        }
        // Software: the same lists on the software canvas.
        let (mut sw_full, mut sw_tick) = (Times::new(), Times::new());
        for _ in 0..frames.div_ceil(2) {
            sw_full.time(|| replay(&first.ops, &store, &mut soft));
            sw_tick.time(|| replay(&tick.ops, &store, &mut soft));
        }
        println!(
            "{:<24} {:>5} {:>5} {:>5} | {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6.2} | {:>6.2} | {:>6.2} {:>6.2} | {:>6} {:>4}",
            name,
            first.ops.len(),
            drawn.draws,
            first.added.len(),
            t_first.at(0.5),
            full.at(0.5),
            full.at(0.95),
            full.at(1.0),
            redraw.at(0.5),
            read.at(0.5),
            sw_full.at(0.5),
            sw_tick.at(0.5),
            d.differ,
            d.max
        );
        pages.push(Page {
            name,
            gpu_full: full.at(0.5),
            gpu_tick: redraw.at(0.5),
            cpu_full: sw_full.at(0.5),
            cpu_tick: sw_tick.at(0.5),
        });
    }
    // The software path's copy to the panel (the banded turn), whole frame.
    let soft = SoftCanvas::new(w, h);
    let mut copy = Times::new();
    let mut out = Vec::with_capacity(NATIVE.0 * NATIVE.1 * 4);
    for _ in 0..frames {
        out.clear();
        copy.time(|| layout.rows(&soft, 0, NATIVE.1, &mut out));
    }
    let median = |f: fn(&Page) -> f64| {
        let mut v: Vec<f64> = pages.iter().map(f).collect();
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    };
    let worst = |f: fn(&Page) -> f64| {
        pages
            .iter()
            .max_by(|a, b| f(a).total_cmp(&f(b)))
            .map(|p| format!("{:.2} ({})", f(p), p.name))
            .unwrap_or_default()
    };
    println!(
        "summary: pages {}; gpu whole page median {:.2} worst {}; gpu tick median {:.2} worst {}",
        pages.len(),
        median(|p| p.gpu_full),
        worst(|p| p.gpu_full),
        median(|p| p.gpu_tick),
        worst(|p| p.gpu_tick)
    );
    println!(
        "summary: software whole page median {:.2} worst {}; software tick median {:.2} worst {}",
        median(|p| p.cpu_full),
        worst(|p| p.cpu_full),
        median(|p| p.cpu_tick),
        worst(|p| p.cpu_tick)
    );
    println!(
        "summary: software copy to the panel (whole frame, banded turn) p50 {:.2} max {:.2}",
        copy.at(0.5),
        copy.at(1.0)
    );
    println!(
        "summary: gpu against software: at most {worst_diff} levels apart in any channel; {:.1} s in all",
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn compute_run(iterations: usize) -> Result<(), String> {
    let gpu = Gpu::surfaceless_es(3, 1)?;
    println!("gpu: {}", gpu.describe());
    println!("compute: FIR filter over a stereo block; {iterations} round trips each (upload, run, read back); times in us");
    // One period at 44.1 kHz (128 frames, 2902 us), a longer filter, and 32 periods.
    for (frames, taps) in [(128, 64), (128, 1024), (4096, 1024)] {
        let r = compute::fir_round_trip(&gpu, frames, taps, iterations)?;
        println!(
            "compute: {frames} frames x {taps} taps: gpu p50 {:.0} p99 {:.0} max {:.0}; cpu (one core) p50 {:.0}; largest difference {:.1e}",
            r.gpu_p50_us, r.gpu_p99_us, r.gpu_max_us, r.cpu_p50_us, r.max_error
        );
    }
    Ok(())
}
