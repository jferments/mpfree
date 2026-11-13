//! The GPU process end to end, as the app runs it: the app's client
//! (`mpfree_ui::gpu_screen`) starts `mpfree-gpu-screen --offscreen`, sends
//! it frames, and must get every frame answered, the picture the software
//! path draws, and a clear error when the process fails or hangs. Needs
//! EGL (Mesa's llvmpipe is enough), as the library's tests do.
//!
//! `MPFREE_GPU_SCREEN=PROGRAM` tests another build of the process instead
//! of this one's: for the ARM build, a script that runs it under qemu-arm.

use std::path::Path;
use std::time::{Duration, Instant};

use mpfree_screen_log::{Geometry, Rotation};
use mpfree_ui::fb::FbLayout;
use mpfree_ui::gpu_screen::{GpuScreen, STUCK_AFTER};
use mpfree_ui::record::replay;
use mpfree_ui::{Canvas, Color, FontId, Fonts, Rect, SoftCanvas};

const PROGRAM: &str = env!("CARGO_BIN_EXE_mpfree-gpu-screen");

fn start(extra: &[&str]) -> GpuScreen {
    let mut args = vec!["--offscreen".to_string()];
    args.extend(extra.iter().map(|s| s.to_string()));
    let program = std::env::var("MPFREE_GPU_SCREEN").unwrap_or_else(|_| PROGRAM.to_string());
    GpuScreen::start(
        Path::new(&program),
        &args,
        1280,
        800,
        Duration::from_secs(10),
    )
    .unwrap_or_else(|e| panic!("the GPU process did not start: {e}"))
}

/// One step of a screen's life: what the frame paints.
fn paint(i: usize, c: &mut dyn Canvas, fonts: &mut Fonts) {
    match i {
        0 => {
            c.fill_rect(Rect::new(0, 0, 1280, 800), Color::rgb(0x101420));
            c.fill_round_rect(Rect::new(40, 40, 400, 200), 18, Color::rgb(0x2E86DE));
            c.stroke_round_rect(Rect::new(500, 60, 300, 160), 12, 2, Color::rgb(0xFFFFFF));
            fonts.face(FontId::Bold).draw(
                c,
                24.0,
                60,
                120,
                Color::rgb(0xFFFFFF),
                "MPFree on the GPU",
            );
        }
        1 => c.fill_rect(Rect::new(900, 400, 50, 50), Color::rgb(0xFF0000)),
        2 => {
            c.set_clip(Rect::new(40, 300, 600, 40));
            c.fill_rect(Rect::new(0, 0, 1280, 800), Color::rgb(0x101420));
            fonts
                .face(FontId::Sans)
                .draw(c, 18.0, 50, 330, Color::rgb(0xF0F0F0), "frame three");
        }
        _ => c.fill_round_rect(
            Rect::new(100 + i as i32 * 30, 600, 60, 60),
            10,
            Color::rgb(0x33AA66).with_alpha(160),
        ),
    }
}

#[test]
fn the_gpu_process_draws_what_the_software_path_draws_and_answers_every_frame() {
    let dir = std::env::temp_dir().join(format!("mpfree-gpu-screen-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let last = dir.join("last.xrgb");
    let mut g = start(&["--last-frame", last.to_str().unwrap()]);
    assert_eq!(g.native, (800, 1280));
    assert_eq!(g.how, "offscreen");
    let mut fonts = Fonts::embedded();
    let mut soft = SoftCanvas::new(1280, 800);
    let mut soft_fonts = Fonts::embedded();
    for i in 0..8 {
        paint(i, g.canvas(), &mut fonts);
        g.send().unwrap();
        soft.set_clip(Rect::new(0, 0, 1280, 800));
        paint(i, &mut soft, &mut soft_fonts);
    }
    // An empty frame is answered too, and shows nothing.
    g.canvas();
    g.send().unwrap();
    let until = Instant::now() + Duration::from_secs(10);
    while g.in_flight() > 0 && Instant::now() < until {
        g.poll().unwrap();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!((g.stats.sent, g.stats.shown, g.stats.same), (9, 8, 1));
    let status = g.stop();
    assert!(status.contains("exit status: 0"), "{status}");
    let bytes = std::fs::read(&last).unwrap();
    let got: Vec<u32> = bytes
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & 0xFF_FFFF)
        .collect();
    let layout = FbLayout {
        geometry: Geometry::new(800, 1280, 3200, 32).unwrap(),
        rotation: Rotation::Deg270,
    };
    let mut native = Vec::new();
    layout.rows(&soft, 0, 1280, &mut native);
    let want: Vec<u32> = native
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) & 0xFF_FFFF)
        .collect();
    let d = mpfree_gpu::compare(&want, &got);
    eprintln!("the GPU process's last frame against software: {d:?}");
    assert!(d.max <= 3 && d.over_1 * 1000 < want.len(), "{d:?}");
    let _ = std::fs::remove_dir_all(&dir);
    // The replay helper is the same one the app's software path uses.
    let _ = replay;
}

#[test]
fn a_gpu_process_that_fails_is_noticed_with_its_reason() {
    let mut g = start(&["--test-fail-after", "2"]);
    let mut fonts = Fonts::embedded();
    let mut err = None;
    // Small frames: under emulation (the ARM build in qemu) a whole page
    // takes longer than the client waits for an answer.
    for i in 0..20 {
        paint(1 + i % 3, g.canvas(), &mut fonts);
        if let Err(e) = g.send() {
            err = Some(e);
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let e = err.expect("the failure is noticed");
    assert!(e.contains("failing after 2 frames"), "{e}");
}

#[test]
fn a_gpu_process_that_hangs_counts_as_stuck() {
    let mut g = start(&["--test-stall-after", "1"]);
    let mut fonts = Fonts::embedded();
    let started = Instant::now();
    let mut err = None;
    for i in 0..1000 {
        paint(i % 4, g.canvas(), &mut fonts);
        if let Err(e) = g.send() {
            err = Some(e);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let e = err.expect("the hang is noticed");
    assert!(e.contains("unanswered"), "{e}");
    assert!(
        started.elapsed() < STUCK_AFTER * 4,
        "{:?}",
        started.elapsed()
    );
    // Stopping it closes its input; it is stopped (SIGTERM-free) or killed.
    let status = g.stop();
    assert!(!status.is_empty());
}

#[test]
fn frames_that_arrive_together_are_all_answered() {
    // Sent back to back, several frames reach the process in one read; each
    // must still be answered (the process once polled its input with a
    // frame already in its buffer, and sat waiting for one that had come).
    // A whole page keeps it busy while the two small frames after it wait
    // in the pipe, so they are read together.
    let mut g = start(&[]);
    let mut fonts = Fonts::embedded();
    for i in 0..60 {
        match i % 3 {
            0 => paint(0, g.canvas(), &mut fonts),
            1 => paint(1, g.canvas(), &mut fonts),
            _ => {
                g.canvas();
            }
        }
        g.send().unwrap_or_else(|e| panic!("frame {}: {e}", i + 1));
    }
    let until = Instant::now() + Duration::from_secs(10);
    while g.in_flight() > 0 && Instant::now() < until {
        g.poll().unwrap();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(g.stats.sent, 60);
    assert_eq!(g.stats.shown + g.stats.same, 60, "{:?}", g.stats);
    assert_eq!(g.stats.same, 20, "{:?}", g.stats);
    let status = g.stop();
    assert!(status.contains("exit status: 0"), "{status}");
}
