//! The GPU's frame against the software path's, byte for byte as the panel
//! takes it. These need a GPU through EGL (Mesa's llvmpipe is enough); with
//! none they fail and say why, rather than pass without drawing.

use super::*;
use mpfree_screen_log::Geometry;
use mpfree_ui::fb::FbLayout;
use mpfree_ui::record::Recorder;
use mpfree_ui::{Canvas, FontId, Fonts, Image, SoftCanvas};

/// Everything the six calls can do: opaque and see-through fills, round
/// rectangles big and small, outlines, text in three faces, an image, and
/// clips that cut through all of them.
fn scene(c: &mut dyn Canvas, fonts: &mut Fonts, k: u32) {
    let (w, h) = c.size();
    c.fill_rect(Rect::new(0, 0, w, h), Color::rgb(0x101420));
    c.fill_rect(Rect::new(10, 10, w / 3, 40), Color::rgb(0x2E86DE));
    c.fill_rect(
        Rect::new(w / 4, 30, w / 3, 50),
        Color::rgb(0xF0A020).with_alpha(120),
    );
    c.fill_round_rect(Rect::new(20, 70, 160, 60), 14, Color::rgb(0x55CC88 ^ k));
    c.fill_round_rect(
        Rect::new(200, 70, 30, 30),
        40,
        Color::rgb(0xEEEEEE).with_alpha(200),
    );
    c.stroke_round_rect(Rect::new(250, 60, 140, 80), 18, 3, Color::rgb(0xFF4466));
    c.stroke_round_rect(
        Rect::new(400, 60, 60, 40),
        4,
        1,
        Color::rgb(0xFFFFFF).with_alpha(90),
    );
    c.stroke_round_rect(Rect::new(470, 60, 12, 12), 6, 8, Color::rgb(0x88AAFF));
    c.set_clip(Rect::new(15, 140, w / 2, 70));
    for (i, (face, px)) in [
        (FontId::Sans, 14.0),
        (FontId::Bold, 22.0),
        (FontId::Mono, 17.0),
    ]
    .into_iter()
    .enumerate()
    {
        let y = 160 + i as i32 * 24;
        fonts.face(face).draw(
            c,
            px,
            20,
            y,
            Color::rgb(0xF5F5F5).with_alpha(230),
            "MPFree on the GPU · 128 frames",
        );
    }
    c.set_clip(Rect::new(-50, -50, 10_000, 10_000));
    let mut img = Image::new(37, 23);
    for (i, p) in img.px.iter_mut().enumerate() {
        *p = (i as u32).wrapping_mul(0x9E3779B1) & 0xFF_FFFF;
    }
    c.draw_image(w - 60, h - 40, &img);
    c.set_clip(Rect::new(w - 50, h - 35, 20, 100));
    c.draw_image(w - 60, h - 40, &img);
    c.fill_round_rect(
        Rect::new(w - 70, h - 60, 50, 50),
        12,
        Color::rgb(0x202020).with_alpha(160),
    );
}

/// The software path's frame for `rotation`, as `fb.rs` sends it.
fn software(nw: usize, nh: usize, rotation: Rotation, k: u32) -> (Vec<u32>, Recorder) {
    let layout = FbLayout {
        geometry: Geometry::new(nw, nh, nw * 4, 32).unwrap(),
        rotation,
    };
    let (w, h) = layout.logical_size();
    let mut fonts = Fonts::embedded();
    let mut soft = SoftCanvas::new(w, h);
    scene(&mut soft, &mut fonts, k);
    let mut bytes = Vec::new();
    layout.rows(&soft, 0, nh, &mut bytes);
    let px = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| u32::from_le_bytes(*b) & 0xFF_FFFF)
        .collect();
    let mut rec = Recorder::new(w, h);
    rec.begin_frame();
    scene(&mut rec, &mut Fonts::embedded(), k);
    (px, rec)
}

fn gpu() -> Gpu {
    Gpu::surfaceless().unwrap_or_else(|e| panic!("no GPU through EGL: {e}"))
}

#[test]
fn the_gpu_draws_the_mpc_screen_as_the_software_path_does() {
    let gpu = gpu();
    eprintln!("GPU: {}", gpu.describe());
    // The MPC One: an 800 x 1280 panel turned 270 degrees.
    let (want, rec) = software(800, 1280, Rotation::Deg270, 0);
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    assert_eq!(r.logical_size(), (1280, 800));
    let stats = r.play(&gpu, rec.ops(), rec.store(), &[]);
    let got = r.read_native(&gpu);
    let d = compare(&want, &got);
    eprintln!(
        "{} ops in {} draws, {} uploads; {} of {} pixels differ, {} by more than 1, at most {}",
        rec.ops().len(),
        stats.draws,
        stats.uploads,
        d.differ,
        want.len(),
        d.over_1,
        d.max
    );
    assert!(d.max <= 3, "a channel differs by {}", d.max);
    assert!(
        d.over_1 * 1000 < want.len(),
        "{} pixels differ by more than 1",
        d.over_1
    );
}

#[test]
fn every_turn_lands_each_pixel_where_the_software_path_puts_it() {
    let gpu = gpu();
    for rotation in [
        Rotation::Deg0,
        Rotation::Deg90,
        Rotation::Deg180,
        Rotation::Deg270,
    ] {
        let (want, rec) = software(600, 520, rotation, 0x123456);
        let mut r = Renderer::new(&gpu, 600, 520, rotation).unwrap();
        r.play(&gpu, rec.ops(), rec.store(), &[]);
        let d = compare(&want, &r.read_native(&gpu));
        assert!(d.max <= 3, "{rotation:?}: a channel differs by {}", d.max);
        assert!(
            d.over_1 * 1000 < want.len(),
            "{rotation:?}: {} pixels differ by more than 1",
            d.over_1
        );
    }
}

#[test]
fn a_frame_keeps_the_last_one_where_it_draws_nothing() {
    let gpu = gpu();
    let (_, rec) = software(800, 1280, Rotation::Deg270, 0);
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    r.play(&gpu, rec.ops(), rec.store(), &[]);
    let before = r.read_native(&gpu);
    // A small change: one fill, as a partial redraw would send.
    let mut next = Recorder::new(1280, 800);
    next.begin_frame();
    next.fill_rect(Rect::new(100, 100, 10, 10), Color::rgb(0xFF0000));
    let s = r.play(&gpu, next.ops(), next.store(), &[]);
    assert_eq!((s.draws, s.uploads), (1, 0));
    let after = r.read_native(&gpu);
    let changed = before.iter().zip(&after).filter(|(a, b)| a != b).count();
    assert!(changed > 0 && changed <= 100, "{changed} pixels changed");
}

#[test]
fn a_compute_job_comes_back_with_the_cpus_answer() {
    let gpu = Gpu::surfaceless_es(3, 1).unwrap_or_else(|e| panic!("no OpenGL ES 3.1: {e}"));
    for (frames, taps) in [(128, 64), (37, 5)] {
        let r = compute::fir_round_trip(&gpu, frames, taps, 20).unwrap();
        eprintln!(
            "FIR {frames} frames x {taps} taps: GPU round trip p50 {:.0} us, CPU {:.0} us, error {:.1e}",
            r.gpu_p50_us, r.cpu_p50_us, r.max_error
        );
        assert!(
            r.max_error < 1e-4,
            "the GPU's answer is {} off",
            r.max_error
        );
        assert!(r.gpu_p50_us > 0.0 && r.gpu_p99_us >= r.gpu_p50_us);
    }
}
