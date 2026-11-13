//! The GPU's frame against the software path's, byte for byte as the panel
//! takes it. These need a GPU through EGL (Mesa's llvmpipe is enough); with
//! none they fail and say why, rather than pass without drawing.

use super::*;
use mpfree_screen_log::Geometry;
use mpfree_ui::fb::FbLayout;
use mpfree_ui::record::{replay, Recorder};
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

/// A page of text in rows, each row clipped differently, with a rounded
/// strip behind it: no images, so it can all go in one draw call.
fn text_page(c: &mut dyn Canvas, fonts: &mut Fonts) {
    let (w, h) = c.size();
    c.fill_rect(Rect::new(0, 0, w, h), Color::rgb(0x0E1118));
    for row in 0..24 {
        let y = 12 + row * 32;
        c.set_clip(Rect::new(0, y - 4, w - row * 20, 30));
        c.fill_round_rect(
            Rect::new(8, y - 2, 340, 28),
            8,
            Color::rgb(0x223344 + row as u32 * 0x010203),
        );
        let face = [FontId::Sans, FontId::Bold, FontId::Mono][row as usize % 3];
        let px = 13.0 + (row % 5) as f32 * 3.0;
        let text = format!(
            "Track {row}: pad {} - {} BPM, swing 56 %",
            row * 7,
            90 + row
        );
        let ink = Color::rgb(0xF0F0F0).with_alpha(170 + (row as u8 % 4) * 25);
        fonts.face(face).draw(c, px, 16, y + 18, ink, &text);
    }
}

/// The software path's frame for `rotation`, as `fb.rs` sends it.
fn software(nw: usize, nh: usize, rotation: Rotation, k: u32) -> (Vec<u32>, Recorder) {
    software_of(nw, nh, rotation, &|c, f| scene(c, f, k))
}

/// The software path's frame of `draw`, and its display list.
fn software_of(
    nw: usize,
    nh: usize,
    rotation: Rotation,
    draw: &dyn Fn(&mut dyn Canvas, &mut Fonts),
) -> (Vec<u32>, Recorder) {
    let layout = FbLayout {
        geometry: Geometry::new(nw, nh, nw * 4, 32).unwrap(),
        rotation,
    };
    let (w, h) = layout.logical_size();
    let mut fonts = Fonts::embedded();
    let mut soft = SoftCanvas::new(w, h);
    draw(&mut soft, &mut fonts);
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
    draw(&mut rec, &mut Fonts::embedded());
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
fn text_shapes_and_clips_are_one_draw_call() {
    let gpu = gpu();
    let (want, rec) = software_of(800, 1280, Rotation::Deg270, &text_page);
    let masks = rec
        .ops()
        .iter()
        .filter(|op| matches!(op, Op::Mask { .. }))
        .count();
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    let s = r.play(&gpu, rec.ops(), rec.store(), &[]);
    let d = compare(&want, &r.read_native(&gpu));
    eprintln!(
        "{} ops ({masks} glyphs) in {} draws, {} uploads; {} pixels differ, at most {}",
        rec.ops().len(),
        s.draws,
        s.uploads,
        d.differ,
        d.max
    );
    assert!(masks > 500, "{masks} glyphs");
    assert_eq!((s.draws, s.atlas_cleared, s.masks_alone), (1, 0, 0));
    assert!(d.max <= 3 && d.over_1 * 1000 < want.len(), "{d:?}");
    // Again: everything is in the atlas already.
    let again = r.play(&gpu, rec.ops(), rec.store(), &[]);
    assert_eq!((again.draws, again.uploads), (1, 0));
}

#[test]
fn a_full_atlas_is_cleared_and_draws_the_same_picture() {
    let gpu = gpu();
    let (want, rec) = software_of(800, 1280, Rotation::Deg270, &text_page);
    let mut big = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    big.play(&gpu, rec.ops(), rec.store(), &[]);
    let roomy = big.read_native(&gpu);
    // Room for two rows of glyphs, and too narrow for the widest.
    let (mut wide, mut tall) = (0, 0);
    for op in rec.ops() {
        if let Op::Mask { id, .. } = op {
            let m = rec.store().mask(*id).unwrap();
            (wide, tall) = (wide.max(m.w as i32), tall.max(m.h as i32));
        }
    }
    let atlas = (wide - 1, ((tall + 7) & !7) * 2);
    let mut small = Renderer::with_atlas(&gpu, 800, 1280, Rotation::Deg270, atlas).unwrap();
    let s = small.play(&gpu, rec.ops(), rec.store(), &[]);
    eprintln!(
        "atlas {}x{}: {} draws, {} uploads, cleared {} times, {} masks alone",
        atlas.0, atlas.1, s.draws, s.uploads, s.atlas_cleared, s.masks_alone
    );
    assert!(s.atlas_cleared > 0, "{s:?}");
    assert!(s.masks_alone > 0, "{s:?}");
    assert!(
        small.read_native(&gpu) == roomy,
        "a small atlas changed the picture"
    );
    let d = compare(&want, &roomy);
    assert!(d.max <= 3 && d.over_1 * 1000 < want.len(), "{d:?}");
}

#[test]
fn a_dropped_mask_is_uploaded_again_when_it_comes_back() {
    let gpu = gpu();
    let (_, rec) = software_of(800, 1280, Rotation::Deg270, &text_page);
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    r.play(&gpu, rec.ops(), rec.store(), &[]);
    let before = r.read_native(&gpu);
    let ids: Vec<u32> = rec
        .ops()
        .iter()
        .filter_map(|op| match op {
            Op::Mask { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    let s = r.play(&gpu, rec.ops(), rec.store(), &unique[..3]);
    assert_eq!((s.draws, s.uploads), (1, 3));
    assert!(r.read_native(&gpu) == before);
}

/// Live test 3's garbled panel: pages recorded apart number their masks
/// from 0 again, so one renderer playing page after page drew the last
/// page's glyphs under the new page's numbers. With `forget_all` between
/// stores, the second page matches the software path.
#[test]
fn a_page_from_another_store_draws_its_own_glyphs_after_forget_all() {
    let gpu = gpu();
    let (_, first) = software_of(800, 1280, Rotation::Deg270, &text_page);
    let (want, second) = software_of(800, 1280, Rotation::Deg270, &|c, f| {
        let (w, h) = c.size();
        c.fill_rect(Rect::new(0, 0, w, h), Color::rgb(0x0E1118));
        for row in 0..12 {
            let text = format!("Other words {row}: xyzzy QWERTY {}", row * 13);
            f.face(FontId::Bold)
                .draw(c, 18.0, 16, 30 + row * 40, Color::rgb(0xF0F0F0), &text);
        }
    });
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    r.play(&gpu, first.ops(), first.store(), &[]);
    // As the panel bench did: the next page's list straight after. Its
    // mask numbers collide with the first page's, so it is wrong.
    r.play(&gpu, second.ops(), second.store(), &[]);
    let stale = compare(&want, &r.read_native(&gpu));
    assert!(stale.max > 3, "the collision this test is about: {stale:?}");
    r.forget_all(&gpu);
    r.play(&gpu, second.ops(), second.store(), &[]);
    let d = compare(&want, &r.read_native(&gpu));
    assert!(d.max <= 3 && d.over_1 * 1000 < want.len(), "{d:?}");
}

#[test]
fn a_box_narrower_than_nothing_draws_nothing_and_does_not_panic() {
    // A widget laid out narrower than it draws can ask for a box of
    // negative width; the software canvas draws nothing, and so must this.
    let gpu = gpu();
    let mut r = Renderer::new(&gpu, 64, 48, Rotation::Deg0).unwrap();
    let mut rec = Recorder::new(64, 48);
    rec.begin_frame();
    rec.fill_rect(Rect::new(0, 0, 64, 48), Color::rgb(0x102030));
    rec.fill_round_rect(Rect::new(10, 10, -6, 20), 8, Color::rgb(0xFFFFFF));
    rec.stroke_round_rect(Rect::new(10, 10, 20, -4), 8, 2, Color::rgb(0xFFFFFF));
    r.play(&gpu, rec.ops(), rec.store(), &[]);
    assert!(r.read_native(&gpu).iter().all(|&p| p == 0x102030));
}

#[test]
fn gpus_on_several_threads_do_not_end_each_others_display() {
    // EGL gives them all one display; one going must not end it for the
    // others (it did, now and then, when tests ran side by side).
    let threads: Vec<_> = (0..4)
        .map(|_| {
            std::thread::spawn(|| {
                for _ in 0..12 {
                    let gpu = gpu();
                    let mut r = Renderer::new(&gpu, 64, 48, Rotation::Deg0).unwrap();
                    let mut rec = Recorder::new(64, 48);
                    rec.begin_frame();
                    rec.fill_rect(Rect::new(0, 0, 64, 48), Color::rgb(0x336699));
                    r.play(&gpu, rec.ops(), rec.store(), &[]);
                    assert!(r.read_native(&gpu).iter().all(|&p| p == 0x336699));
                }
            })
        })
        .collect();
    for t in threads {
        t.join().expect("a thread's GPU failed");
    }
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

#[test]
fn each_logical_rectangle_lands_on_the_native_pixels_the_software_path_uses() {
    for (nw, nh) in [(800usize, 1280usize), (600, 520)] {
        let g = Geometry::new(nw, nh, nw * 4, 32).unwrap();
        for rotation in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ] {
            let turn = Turn::new(rotation, nw as f32, nh as f32);
            let (lw, lh) = g.logical_size(rotation);
            for r in [
                Rect::new(0, 0, lw as i32, lh as i32),
                Rect::new(10, 20, 30, 7),
                Rect::new(lw as i32 - 5, lh as i32 - 9, 5, 9),
            ] {
                let mut want = Rect::default();
                for y in r.y..r.bottom() {
                    for x in r.x..r.right() {
                        let (nx, ny) = g.native(rotation, x as usize, y as usize);
                        want = want.union(Rect::new(nx as i32, ny as i32, 1, 1));
                    }
                }
                assert_eq!(turn.rect(r), want, "{rotation:?} {r:?}");
            }
        }
    }
}

#[test]
fn changes_merge_into_few_rectangles() {
    let mut d = Damage::default();
    d.add(Rect::new(0, 0, 100, 100));
    d.add(Rect::new(10, 10, 5, 5));
    assert_eq!(d.rects(), &[Rect::new(0, 0, 100, 100)]);
    // Far apart: kept apart, so a copy does not cover the space between.
    d.add(Rect::new(900, 600, 10, 10));
    assert_eq!(d.rects().len(), 2);
    // Next to the first: merged.
    d.add(Rect::new(100, 0, 4, 100));
    assert!(d.rects().contains(&Rect::new(0, 0, 104, 100)), "{d:?}");
    // Past the limit, everything becomes one.
    let mut many = Damage::default();
    for i in 0..(MAX_DAMAGE as i32 + 1) {
        many.add(Rect::new(i * 70, (i % 2) * 400, 8, 8));
    }
    assert_eq!(many.rects().len(), 1);
    assert_eq!(many.bounds(), many.rects()[0]);
    // Nothing from nothing.
    many.add(Rect::new(5, 5, 0, 9));
    assert_eq!(many.rects().len(), 1);
}

#[test]
fn an_outline_draws_only_its_edges_and_corners() {
    let gpu = gpu();
    let draw = |c: &mut dyn Canvas, _: &mut Fonts| {
        let (w, h) = c.size();
        c.fill_rect(Rect::new(0, 0, w, h), Color::rgb(0x202830));
        c.stroke_round_rect(
            Rect::new(100, 50, 1000, 600),
            12,
            2,
            Color::rgb(0xFFCC00).with_alpha(200),
        );
    };
    let (want, rec) = software_of(800, 1280, Rotation::Deg270, &draw);
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    let s = r.play(&gpu, rec.ops(), rec.store(), &[]);
    // The background, the four straight edges; only the corners shaded.
    let edges = 2 * (1000 - 24) * 2 + 2 * (600 - 24) * 2;
    assert_eq!(s.shaded_px, 4 * 12 * 12, "{s:?}");
    assert_eq!(s.plain_px, 1280 * 800 + edges, "{s:?}");
    let d = compare(&want, &r.read_native(&gpu));
    assert!(d.max <= 3 && d.over_1 * 1000 < want.len(), "{d:?}");
}

#[test]
fn a_small_change_touches_only_its_own_pixels() {
    let gpu = gpu();
    let (_, rec) = software(800, 1280, Rotation::Deg270, 0);
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    r.play(&gpu, rec.ops(), rec.store(), &[]);
    assert_eq!(r.damage(), &[Rect::new(0, 0, 800, 1280)]);
    let mut next = Recorder::new(1280, 800);
    next.begin_frame();
    next.set_clip(Rect::new(0, 0, 105, 1000));
    next.fill_rect(Rect::new(100, 200, 10, 20), Color::rgb(0xFF0000));
    let s = r.play(&gpu, next.ops(), next.store(), &[]);
    // Logical x 100..105 (clipped), y 200..220: native x 200..220, y
    // 1280-105..1280-100.
    assert_eq!(r.damage(), &[Rect::new(200, 1175, 20, 5)]);
    assert_eq!((s.plain_px, s.shaded_px), (100, 0));
}

/// Frame after frame through two panel buffers: each one shown must be
/// the whole picture the software path has by then, though every list
/// after the first redraws only a little of it.
#[test]
fn every_frame_shown_from_two_buffers_is_the_whole_picture() {
    let gpu = gpu();
    let layout = FbLayout {
        geometry: Geometry::new(800, 1280, 3200, 32).unwrap(),
        rotation: Rotation::Deg270,
    };
    let mut r = Renderer::new(&gpu, 800, 1280, Rotation::Deg270).unwrap();
    let targets = (0..2)
        .map(|_| Target::new(&gpu, 800, 1280).unwrap())
        .collect();
    let mut scanout = scanout::Scanout::new(&gpu, &r, targets).unwrap();
    let mut display = scanout::Offscreen::default();
    let mut rec = Recorder::new(1280, 800);
    let mut soft = SoftCanvas::new(1280, 800);
    let mut fonts = Fonts::embedded();
    type Step = Box<dyn Fn(&mut dyn Canvas, &mut Fonts)>;
    let steps: Vec<Step> = vec![
        Box::new(|c, f| scene(c, f, 0)),
        Box::new(|c, _| c.fill_rect(Rect::new(300, 300, 40, 30), Color::rgb(0xFF0000))),
        Box::new(|c, f| {
            c.set_clip(Rect::new(20, 140, 400, 30));
            c.fill_rect(Rect::new(0, 0, 1280, 800), Color::rgb(0x101420));
            f.face(FontId::Bold)
                .draw(c, 20.0, 24, 162, Color::rgb(0xFFFFFF), "Changed");
        }),
        Box::new(|c, _| {
            c.fill_round_rect(
                Rect::new(900, 500, 200, 120),
                20,
                Color::rgb(0x33AA66).with_alpha(180),
            )
        }),
        Box::new(|c, _| c.fill_rect(Rect::new(1200, 10, 60, 20), Color::rgb(0x0000FF))),
        Box::new(|c, _| {
            c.stroke_round_rect(Rect::new(50, 600, 300, 150), 16, 3, Color::rgb(0xFFFFFF))
        }),
    ];
    for (i, step) in steps.iter().enumerate() {
        rec.begin_frame();
        step(&mut rec, &mut fonts);
        let (_, dropped) = rec.store_mut().take_changes();
        let shown = scanout
            .present(&gpu, &mut r, &mut display, rec.ops(), rec.store(), &dropped)
            .unwrap()
            .expect("a frame that draws is shown");
        // A frame's list starts with the whole canvas as its clip.
        soft.set_clip(Rect::new(0, 0, 1280, 800));
        replay(rec.ops(), rec.store(), &mut soft);
        let mut bytes = Vec::new();
        layout.rows(&soft, 0, 1280, &mut bytes);
        let want: Vec<u32> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| u32::from_le_bytes(*b) & 0xFF_FFFF)
            .collect();
        let got = scanout.read_front(&gpu).unwrap();
        let d = compare(&want, &got);
        eprintln!(
            "frame {i}: buffer {}, {} draws, {} px plain, {} shaded, {} copied; {d:?}",
            shown.buffer,
            shown.draw.draws,
            shown.draw.plain_px,
            shown.draw.shaded_px,
            shown.draw.copied_px
        );
        assert!(
            d.max <= 3 && d.over_1 * 1000 < want.len(),
            "frame {i}: {d:?}"
        );
        assert_eq!(shown.buffer, i % 2);
        if i >= 2 {
            // Only what the frame before changed is copied: never the panel.
            assert!(
                shown.draw.copied_px > 0 && shown.draw.copied_px < 200_000,
                "frame {i}: {} px copied",
                shown.draw.copied_px
            );
        }
    }
    assert_eq!(display.shown, vec![0, 1, 0, 1, 0, 1]);
    // A list that draws nothing shows nothing.
    rec.begin_frame();
    let none = scanout
        .present(&gpu, &mut r, &mut display, rec.ops(), rec.store(), &[])
        .unwrap();
    assert!(none.is_none());
    assert_eq!(display.shown.len(), 6);
}

#[test]
fn a_panel_with_one_buffer_is_refused() {
    let gpu = gpu();
    let r = Renderer::new(&gpu, 64, 48, Rotation::Deg0).unwrap();
    let one = vec![Target::new(&gpu, 64, 48).unwrap()];
    assert!(scanout::Scanout::new(&gpu, &r, one).is_err());
}

#[test]
fn the_gpu_says_whether_it_can_draw_into_screen_buffers() {
    // Mesa's surfaceless EGL has these with a hardware driver; llvmpipe may
    // not. Either way it must answer, not crash.
    let gpu = gpu();
    match scanout::import_support(&gpu) {
        Ok(()) => eprintln!("dma-buf import: yes"),
        Err(e) => eprintln!("dma-buf import: no ({e})"),
    }
}
