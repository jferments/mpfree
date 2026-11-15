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
//!   mpfree-gpu-bench flip DIR [--hold-ms N] [--frames N] [--card PATH]
//!       the GPU drawing straight into the buffers the panel shows (DRM/KMS
//!       page flips, nothing read back): checks first, with the screen
//!       untouched, that what the GPU wrote into a screen buffer is the
//!       picture expected; then shows each page for N ms; then times a
//!       whole page and a tick's redraw the GPU's way, and software's (draw,
//!       turn into the panel's layout, copy into a screen buffer that is
//!       not shown, each timed on its own); then puts
//!       the screen back as it was (STATE-CHANGING, in RAM only: see below)
//!   mpfree-gpu-bench panel DIR [--hold-ms N] [--frames N]
//!       the GPU's pictures on the panel: draws each page on the GPU, reads
//!       it back and writes it to /dev/fb0, holding each for N ms; then times
//!       a whole page drawn and copied to the panel on the GPU and in
//!       software, frame after frame (--fb and --sysfs name another
//!       framebuffer, for tests)
//!
//! It draws through Mesa's surfaceless EGL on a render node. It reads DIR
//! and everything goes to stdout. `describe`, `screens` and `compute` open
//! no display, take no DRM master and change no display setting. `panel`
//! writes whole frames to the framebuffer, and `0,0` to its sysfs `pan`
//! file, as the app's screen does (`mpfree_ui::fb::FbTarget`); it never
//! creates either. `flip` opens the display card (`/dev/dri/card0`), makes
//! three screen buffers there, takes the card's master and points the lit
//! CRTC at its own buffers in the mode it already runs: a display change,
//! in RAM only. It reads the CRTC's settings first and sets them back at
//! the end, on an error, and on SIGINT, SIGTERM or SIGHUP; if it dies
//! anyway, the kernel frees its buffers and the framebuffer console takes
//! the screen back when the card's last file closes, and a power cycle
//! undoes everything. A failure prints `STOP: reason` and exits with
//! status 1.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mpfree_gpu::kms::{KmsDisplay, XRGB8888};
use mpfree_gpu::scanout::{self, Display, Scanout};
use mpfree_gpu::{compare, compute, Gpu, Renderer};
use mpfree_screen_log::{Geometry, PixelFormat, Rotation};
use mpfree_ui::fb::{FbLayout, FbTarget};
use mpfree_ui::record::{replay, Frame, Store};
use mpfree_ui::{Canvas, SoftCanvas};

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
    let usage = "usage: mpfree-gpu-bench describe | screens DIR [--frames N] | \
                 compute [--iterations N] | panel DIR [--hold-ms N] [--frames N] | \
                 flip DIR [--hold-ms N] [--frames N] [--card PATH]";
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
        Some("panel") => {
            let dir = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let mut fb = PathBuf::from("/dev/fb0");
            let mut sysfs = PathBuf::from("/sys/class/graphics/fb0");
            let mut rest = args[2..].iter();
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--hold-ms" | "--frames" => {
                        rest.next();
                    }
                    "--fb" => fb = rest.next().ok_or("--fb needs a path")?.into(),
                    "--sysfs" => sysfs = rest.next().ok_or("--sysfs needs a path")?.into(),
                    _ => return Err(format!("unknown argument {a}")),
                }
            }
            panel(
                Path::new(dir),
                number(args, "--hold-ms", 1500, 10_000)?,
                number(args, "--frames", 60, 1000)?,
                &fb,
                &sysfs,
            )
        }
        Some("flip") => {
            let dir = args.get(1).filter(|a| !a.starts_with("--")).ok_or(usage)?;
            let mut card = PathBuf::from("/dev/dri/card0");
            let mut rest = args[2..].iter();
            while let Some(a) = rest.next() {
                match a.as_str() {
                    "--hold-ms" | "--frames" => {
                        rest.next();
                    }
                    "--card" => card = rest.next().ok_or("--card needs a path")?.into(),
                    _ => return Err(format!("unknown argument {a}")),
                }
            }
            flip(
                Path::new(dir),
                number(args, "--hold-ms", 1500, 10_000)?,
                number(args, "--frames", 60, 1000)?,
                &card,
            )
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
    match scanout::import_support(&gpu) {
        Ok(()) => println!("gpu: draws into screen buffers (dma-buf import): yes"),
        Err(e) => println!("gpu: draws into screen buffers (dma-buf import): no: {e}"),
    }
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
    draws: usize,
}

/// Every NAME.mpdl in `dir`, in name order.
fn page_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "mpdl"))
        .collect();
    files.sort();
    if files.is_empty() {
        return Err(format!("no .mpdl files in {}", dir.display()));
    }
    Ok(files)
}

/// A page's name and its first frame, which must be `w` x `h`.
fn first_frame(path: &Path, w: i32, h: i32) -> Result<(String, Frame), String> {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let first = Frame::decode_all(&bytes)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into_iter()
        .next()
        .ok_or_else(|| format!("{}: no frames", path.display()))?;
    if (first.w, first.h) != (w, h) {
        return Err(format!(
            "{}: a {}x{} page; this bench draws {w}x{h}",
            path.display(),
            first.w,
            first.h
        ));
    }
    Ok((name, first))
}

/// The GPU's frame (native rows, XRGB words) as the framebuffer's bytes.
fn to_frame(px: &[u32], g: &Geometry, out: &mut [u8]) {
    for (ny, row) in px.chunks_exact(g.width).enumerate() {
        let line = &mut out[ny * g.stride..ny * g.stride + g.width * 4];
        for (dst, p) in line.as_chunks_mut::<4>().0.iter_mut().zip(row) {
            *dst = p.to_le_bytes();
        }
    }
}

/// The GPU's pictures on the panel: each page drawn on the GPU, read back
/// and written to the framebuffer, held `hold_ms`; then one page drawn and
/// copied `frames` times on the GPU and in software, timed.
fn panel(dir: &Path, hold_ms: usize, frames: usize, fb: &Path, sysfs: &Path) -> Result<(), String> {
    let files = page_files(dir)?;
    let mut target = FbTarget::open(fb, sysfs, ROTATION)?;
    let g = target.layout.geometry;
    if (g.width, g.height) != NATIVE || g.format != PixelFormat::Xrgb8888 {
        return Err(format!(
            "the panel is {}x{} at {:?}; this bench draws {}x{} at 32 bits a pixel",
            g.width, g.height, g.format, NATIVE.0, NATIVE.1
        ));
    }
    let (w, h) = target.logical_size();
    let gpu = Gpu::surfaceless()?;
    println!("gpu: {}", gpu.describe());
    println!(
        "panel: {} pages drawn on the GPU and shown {hold_ms} ms each; panel {}x{} turned 270, stride {}; times in ms",
        files.len(),
        g.width,
        g.height,
        g.stride
    );
    println!(
        "{:<24} {:>5} {:>5} | {:>6} {:>6} {:>6} {:>6}",
        "page", "ops", "draws", "draw", "read", "copy", "whole"
    );
    let mut r = Renderer::new(&gpu, g.width as i32, g.height as i32, ROTATION)?;
    let mut frame = vec![0u8; g.frame_len()];
    let (mut cleared, mut alone) = (0usize, 0usize);
    let mut busiest: Option<(String, Frame, Store)> = None;
    let mut checked = false;
    for path in &files {
        let (name, first) = first_frame(path, w, h)?;
        // Each page was recorded on its own, so its masks are numbered from
        // 0 again: a store of its own, and a renderer that forgets the last
        // page's masks (live test 3's garbled text was the last page's
        // glyphs drawn under this page's numbers).
        let mut store = Store::new(usize::MAX);
        store.apply(&first);
        r.forget_all(&gpu);
        let t = Instant::now();
        let drawn = r.play(&gpu, &first.ops, &store, &first.dropped);
        r.finish(&gpu);
        let t_draw = t.elapsed();
        let px = r.read_native(&gpu);
        let t_read = t.elapsed();
        to_frame(&px, &g, &mut frame);
        target.present_frame(&frame)?;
        let t_copy = t.elapsed();
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        println!(
            "{:<24} {:>5} {:>5} | {:>6.2} {:>6.2} {:>6.2} {:>6.2}",
            name,
            first.ops.len(),
            drawn.draws,
            ms(t_draw),
            ms(t_read - t_draw),
            ms(t_copy - t_read),
            ms(t_copy)
        );
        cleared += drawn.atlas_cleared;
        alone += drawn.masks_alone;
        if !checked {
            // The framebuffer holds what was written (read back once).
            checked = true;
            match mpfree_ui::fb::capture(fb, &target.layout) {
                Ok(img) => {
                    let mut soft = SoftCanvas::new(w, h);
                    soft.px.copy_from_slice(&img.px);
                    let mut back = Vec::new();
                    target.layout.rows(&soft, 0, g.height, &mut back);
                    let same = back
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(frame.as_chunks::<4>().0)
                        .all(|(a, b)| a[..3] == b[..3]);
                    println!(
                        "panel: the framebuffer holds what was written: {}",
                        if same { "yes" } else { "NO" }
                    );
                    if !same {
                        return Err("the framebuffer does not hold what was written".into());
                    }
                }
                Err(e) => println!("panel: could not read the framebuffer back ({e})"),
            }
        }
        if busiest
            .as_ref()
            .is_none_or(|(_, b, _)| first.ops.len() > b.ops.len())
        {
            busiest = Some((name, first, store));
        }
        std::thread::sleep(Duration::from_millis(hold_ms as u64));
    }
    // One page, again and again, to the panel: the GPU's way and software's.
    let (name, page, store) = busiest.ok_or("no pages")?;
    r.forget_all(&gpu);
    let mut soft = SoftCanvas::new(w, h);
    let mut buf = Vec::with_capacity(g.frame_len());
    let (mut g_draw, mut g_read, mut g_copy, mut g_all) =
        (Times::new(), Times::new(), Times::new(), Times::new());
    let (mut s_draw, mut s_turn, mut s_copy, mut s_all) =
        (Times::new(), Times::new(), Times::new(), Times::new());
    for _ in 0..frames {
        let t = Instant::now();
        g_draw.time(|| {
            r.play(&gpu, &page.ops, &store, &[]);
            r.finish(&gpu);
        });
        let px = g_read.time(|| r.read_native(&gpu));
        g_copy.time(|| {
            to_frame(&px, &g, &mut frame);
            target.present_frame(&frame)
        })?;
        g_all.0.push(t.elapsed().as_secs_f64() * 1e3);
        let t = Instant::now();
        s_draw.time(|| replay(&page.ops, &store, &mut soft));
        // The turn into the panel's layout and the write to the panel, apart:
        // drawing in the panel's own layout would save the first, not the second.
        s_turn.time(|| target.layout.rows(&soft, 0, g.height, &mut buf));
        s_copy.time(|| target.present_frame(&buf))?;
        s_all.0.push(t.elapsed().as_secs_f64() * 1e3);
    }
    println!(
        "summary: panel, page {name} ({} ops), {frames} frames each way, whole frame to the panel",
        page.ops.len()
    );
    println!(
        "summary: panel gpu: draw p50 {:.2}, read back p50 {:.2}, copy p50 {:.2}; whole p50 {:.2} p95 {:.2} max {:.2}",
        g_draw.at(0.5),
        g_read.at(0.5),
        g_copy.at(0.5),
        g_all.at(0.5),
        g_all.at(0.95),
        g_all.at(1.0)
    );
    println!(
        "summary: panel software: draw p50 {:.2}, turn p50 {:.2}, copy p50 {:.2}; whole p50 {:.2} p95 {:.2} max {:.2}",
        s_draw.at(0.5),
        s_turn.at(0.5),
        s_copy.at(0.5),
        s_all.at(0.5),
        s_all.at(0.95),
        s_all.at(1.0)
    );
    println!(
        "summary: panel showed {} pages from the GPU; atlas cleared {cleared} times, {alone} masks drawn alone",
        files.len()
    );
    Ok(())
}

/// A page's name and its frames (the first, then the tick's redraw), which
/// must be `w` x `h`.
fn page_frames(path: &Path, w: i32, h: i32) -> Result<(String, Vec<Frame>), String> {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let frames = Frame::decode_all(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    match frames.first() {
        None => Err(format!("{}: no frames", path.display())),
        Some(f) if (f.w, f.h) != (w, h) => Err(format!(
            "{}: a {}x{} page; this bench draws {w}x{h}",
            path.display(),
            f.w,
            f.h
        )),
        Some(_) => Ok((name, frames)),
    }
}

/// CPU time this process has used, all threads.
fn cpu_time() -> Duration {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a plain clock_gettime into a timespec on the stack.
    unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut t) };
    Duration::new(t.tv_sec as u64, t.tv_nsec as u32)
}

/// Sleeps `ms`, waking early when a signal asks the program to stop.
fn hold(ms: usize) {
    let until = Instant::now() + Duration::from_millis(ms as u64);
    while !scanout::stopping() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The GPU drawing straight into the buffers the panel shows.
fn flip(dir: &Path, hold_ms: usize, frames: usize, card: &Path) -> Result<(), String> {
    scanout::stop_on_signals();
    let files = page_files(dir)?;
    let gpu = Gpu::surfaceless()?;
    println!("gpu: {}", gpu.describe());
    scanout::import_support(&gpu)?;
    // Three screen buffers: two the panel shows in turn, one for timing
    // software's copy (never shown). Nothing on the screen changes yet.
    let mut kms = KmsDisplay::open(card, 3)?;
    let (nw, nh) = kms.size();
    let o = kms.output;
    let mode = o.mode();
    println!(
        "flip: {}: connector {} on CRTC {}, mode {} at {} Hz, showing framebuffer {} from {},{}",
        card.display(),
        o.connector,
        o.crtc.id,
        mode.name(),
        mode.vrefresh,
        o.crtc.fb,
        o.crtc.x,
        o.crtc.y
    );
    if (nw as usize, nh as usize) != NATIVE {
        return Err(format!(
            "the screen runs {nw}x{nh}; this bench draws {}x{}",
            NATIVE.0, NATIVE.1
        ));
    }
    let mut r = Renderer::new(&gpu, nw, nh, ROTATION)?;
    let (w, h) = r.logical_size();
    let mut imports = Vec::new();
    for i in 0..2 {
        let b = kms.buffer(i);
        println!(
            "flip: screen buffer {i}: {}x{}, pitch {}, {} bytes",
            b.width, b.height, b.pitch, b.size
        );
        let fd = kms.dmabuf(i)?;
        use std::os::fd::AsFd;
        match scanout::import_dmabuf(&gpu, fd.as_fd(), nw, nh, b.pitch, XRGB8888) {
            Ok(imp) => imports.push(imp),
            Err(e) => {
                for imp in imports {
                    imp.release(&gpu);
                }
                return Err(e);
            }
        }
    }
    let result = flip_run(
        &gpu, &mut r, &mut kms, &imports, &files, hold_ms, frames, w, h,
    );
    // Put the screen back first, then free the GPU's side.
    let restored = kms.restore();
    for imp in imports {
        imp.release(&gpu);
    }
    let flips = kms.flips();
    drop(kms);
    match &restored {
        Ok(()) => println!(
            "flip: the screen is back as it was (CRTC {} showing framebuffer {}); {flips} page flips in all",
            o.crtc.id, o.crtc.fb
        ),
        Err(e) => println!("flip: putting the screen back failed: {e}"),
    }
    result?;
    restored?;
    if scanout::stopping() {
        return Err("stopped by a signal (the screen was put back)".into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn flip_run(
    gpu: &Gpu,
    r: &mut Renderer,
    kms: &mut KmsDisplay,
    imports: &[scanout::Imported],
    files: &[PathBuf],
    hold_ms: usize,
    frames: usize,
    w: i32,
    h: i32,
) -> Result<(), String> {
    let (nw, nh) = r.native_size();
    let g = Geometry::new(nw as usize, nh as usize, kms.buffer(0).pitch as usize, 32)?;
    let layout = FbLayout {
        geometry: g,
        rotation: ROTATION,
    };
    // 1. With the screen untouched: the GPU draws the first page into
    //    screen buffer 0, and the CPU reads that buffer's own memory. It
    //    must hold the expected picture, row 0 first, in XRGB8888: what the
    //    panel would show. Anything else stops here, before any change.
    let (name, first) = page_frames(&files[0], w, h)?;
    let mut store = Store::new(usize::MAX);
    store.apply(&first[0]);
    let target = imports[0].target;
    r.play_on(gpu, target, None, &first[0].ops, &store, &first[0].dropped);
    r.finish(gpu);
    let want = r.read_target(gpu, target);
    let mapped = kms.map(0)?;
    let pitch = g.stride;
    let mut wrong = 0usize;
    for (ny, row) in want.chunks_exact(nw as usize).enumerate() {
        let line = &mapped.bytes()[ny * pitch..ny * pitch + nw as usize * 4];
        for (p, b) in row.iter().zip(line.as_chunks::<4>().0) {
            wrong += (u32::from_le_bytes(*b) & 0xFF_FFFF != *p) as usize;
        }
    }
    drop(mapped);
    let mut soft = SoftCanvas::new(w, h);
    replay(&first[0].ops, &store, &mut soft);
    let mut native = Vec::new();
    layout.rows(&soft, 0, nh as usize, &mut native);
    let sw: Vec<u32> = native
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b) & 0xFF_FFFF)
        .collect();
    let d = compare(&sw, &want);
    println!(
        "flip: check before showing anything: page {name}: {wrong} pixels in the screen buffer differ from what the GPU drew; the GPU's picture is at most {} levels from software's",
        d.max
    );
    if wrong != 0 || d.max > 3 {
        return Err(
            "the screen buffer does not hold the expected picture; nothing was shown".into(),
        );
    }
    // 2. The pages, each shown straight from the GPU's buffers.
    let mut scan = Scanout::new(gpu, r, imports.iter().map(|i| i.target).collect())?;
    println!(
        "flip: SCREEN CHANGES NOW: {} pages drawn on the GPU straight into the buffers the panel shows, {hold_ms} ms each",
        files.len()
    );
    println!(
        "{:<24} {:>5} {:>6} {:>6} | {:>6} {:>6} {:>6} {:>6}",
        "page", "ops", "plainK", "shadeK", "wait", "draw", "gpu", "whole"
    );
    let mut busiest: Option<(String, Vec<Frame>, Store)> = None;
    for path in files {
        if scanout::stopping() {
            return Ok(());
        }
        let (name, list) = page_frames(path, w, h)?;
        let mut store = Store::new(usize::MAX);
        store.apply(&list[0]);
        r.forget_all(gpu);
        let t = Instant::now();
        let shown = scan.present(gpu, r, kms, &list[0].ops, &store, &list[0].dropped)?;
        let whole = t.elapsed();
        if let Some(p) = shown {
            let ms = |d: Duration| d.as_secs_f64() * 1e3;
            println!(
                "{:<24} {:>5} {:>6} {:>6} | {:>6.2} {:>6.2} {:>6.2} {:>6.2}",
                name,
                list[0].ops.len(),
                p.draw.plain_px / 1000,
                p.draw.shaded_px / 1000,
                ms(p.waited),
                ms(p.drawn),
                ms(p.finished),
                ms(whole)
            );
        }
        if busiest
            .as_ref()
            .is_none_or(|(_, b, _)| list[0].ops.len() > b[0].ops.len())
        {
            busiest = Some((name, list, store));
        }
        hold(hold_ms);
    }
    // 3. One page again and again: whole, then the tick's redraw, the GPU's
    //    way (each frame shown by a page flip) and software's (draw, turn
    //    and copy into screen buffer 2, never shown).
    let (name, list, mut store) = busiest.ok_or("no pages")?;
    r.forget_all(gpu);
    let tick = list.get(1).cloned().unwrap_or_default();
    store.apply(&tick);
    let mut timed = |ops: &[mpfree_ui::record::Op], label: &str| -> Result<(), String> {
        let (mut wait, mut draw, mut fin, mut all) =
            (Times::new(), Times::new(), Times::new(), Times::new());
        let cpu = cpu_time();
        let started = Instant::now();
        for _ in 0..frames {
            if scanout::stopping() {
                return Ok(());
            }
            let t = Instant::now();
            if let Some(p) = scan.present(gpu, r, kms, ops, &store, &[])? {
                wait.0.push(p.waited.as_secs_f64() * 1e3);
                draw.0.push(p.drawn.as_secs_f64() * 1e3);
                fin.0.push(p.finished.as_secs_f64() * 1e3);
            }
            all.0.push(t.elapsed().as_secs_f64() * 1e3);
        }
        kms.wait()?;
        let wall = started.elapsed().as_secs_f64();
        let used = (cpu_time() - cpu).as_secs_f64();
        println!(
            "summary: flip gpu {label}: wait for the flip p50 {:.2}, draw p50 {:.2}, gpu finish p50 {:.2} p95 {:.2}; whole p50 {:.2} p95 {:.2} max {:.2}; CPU {:.0} % of one core",
            wait.at(0.5),
            draw.at(0.5),
            fin.at(0.5),
            fin.at(0.95),
            all.at(0.5),
            all.at(0.95),
            all.at(1.0),
            100.0 * used / wall.max(1e-9)
        );
        Ok(())
    };
    println!(
        "summary: flip, page {name} ({} ops whole, {} ops a tick), {frames} frames each",
        list[0].ops.len(),
        tick.ops.len()
    );
    timed(&list[0].ops, "whole page")?;
    timed(&tick.ops, "tick")?;
    let mut sw_buf = kms.map(2)?;
    let mut soft = SoftCanvas::new(w, h);
    let mut rows = Vec::with_capacity(g.frame_len());
    let mut sw = |ops: &[mpfree_ui::record::Op], label: &str| {
        let (mut draw, mut turn, mut copy) = (Times::new(), Times::new(), Times::new());
        let cpu = cpu_time();
        let started = Instant::now();
        for _ in 0..frames {
            if scanout::stopping() {
                return;
            }
            draw.time(|| {
                soft.set_clip(mpfree_ui::Rect::new(0, 0, w, h));
                replay(ops, &store, &mut soft)
            });
            turn.time(|| {
                rows.clear();
                layout.rows(&soft, 0, nh as usize, &mut rows);
            });
            copy.time(|| {
                let n = rows.len().min(sw_buf.bytes().len());
                sw_buf.bytes_mut()[..n].copy_from_slice(&rows[..n]);
            });
        }
        let wall = started.elapsed().as_secs_f64();
        let used = (cpu_time() - cpu).as_secs_f64();
        println!(
            "summary: flip software {label}: draw p50 {:.2}, turn p50 {:.2}, copy into the screen buffer p50 {:.2} (whole frame); whole p50 {:.2}; CPU {:.0} % of one core",
            draw.at(0.5),
            turn.at(0.5),
            copy.at(0.5),
            draw.at(0.5) + turn.at(0.5) + copy.at(0.5),
            100.0 * used / wall.max(1e-9)
        );
    };
    sw(&list[0].ops, "whole page");
    sw(&tick.ops, "tick");
    Ok(())
}

fn screens(dir: &Path, frames: usize) -> Result<(), String> {
    let files = page_files(dir)?;
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
        "{:<24} {:>5} {:>5} {:>5} {:>6} {:>6} | {:>6} {:>6} {:>6} {:>6} {:>6} | {:>6} | {:>6} {:>6} | {:>6} {:>4}",
        "page",
        "ops",
        "draws",
        "items",
        "plainK",
        "shadeK",
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
    // The atlas's fallbacks over every page: clears, and masks drawn alone;
    // and the vector shapes the GPU worked out itself, and how often their
    // atlas filled.
    let (mut cleared, mut alone, mut shapes, mut shapes_cleared) = (0usize, 0usize, 0usize, 0usize);
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
        cleared += drawn.atlas_cleared;
        alone += drawn.masks_alone;
        shapes += drawn.shapes_drawn;
        shapes_cleared += drawn.shapes_cleared;
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
            "{:<24} {:>5} {:>5} {:>5} {:>6} {:>6} | {:>6.2} {:>6.2} {:>6.2} {:>6.2} {:>6.2} | {:>6.2} | {:>6.2} {:>6.2} | {:>6} {:>4}",
            name,
            first.ops.len(),
            drawn.draws,
            first.added.len(),
            drawn.plain_px / 1000,
            drawn.shaded_px / 1000,
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
            draws: drawn.draws,
        });
    }
    // The software path's turn into the panel's layout (banded), whole frame:
    // the copy to the panel itself is timed by `panel` and `flip`, on the MPC.
    let soft = SoftCanvas::new(w, h);
    let mut turn = Times::new();
    let mut out = Vec::with_capacity(NATIVE.0 * NATIVE.1 * 4);
    for _ in 0..frames {
        out.clear();
        turn.time(|| layout.rows(&soft, 0, NATIVE.1, &mut out));
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
    let mut draws: Vec<usize> = pages.iter().map(|p| p.draws).collect();
    draws.sort_unstable();
    println!(
        "summary: gpu draw calls per whole page median {} worst {}; atlas cleared {cleared} times, {alone} masks drawn alone; {shapes} shapes worked out on the GPU, their atlas cleared {shapes_cleared} times",
        draws[draws.len() / 2],
        draws.last().copied().unwrap_or(0)
    );
    println!(
        "summary: software whole page median {:.2} worst {}; software tick median {:.2} worst {}",
        median(|p| p.cpu_full),
        worst(|p| p.cpu_full),
        median(|p| p.cpu_tick),
        worst(|p| p.cpu_tick)
    );
    println!(
        "summary: software turn into the panel's layout (whole frame, banded) p50 {:.2} max {:.2}",
        turn.at(0.5),
        turn.at(1.0)
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
