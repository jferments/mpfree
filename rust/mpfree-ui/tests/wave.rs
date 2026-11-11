//! The waveform widgets: what they draw, how touches move markers, scroll
//! and zoom, and that a moving playhead redraws only narrow strips.

use std::sync::Arc;

use mpfree_ui::wave::{Beats, Marker, MarkerKind, Peaks, RawPeaks, WaveOverview, WaveView, Waveform};
use mpfree_ui::{Canvas, Color, EventKind, Fonts, PaintCx, Rect, SoftCanvas, Theme, Touch, Ui, Widget};

const FRAMES: usize = 44_100;
const R: Rect = Rect::new(0, 0, 1000, 300);

/// A second of a 0.8 sine with a 20-frame period (a full swing in every
/// column at the whole view); stereo has the right side at half level.
fn sound(channels: usize) -> Arc<dyn Peaks> {
    let data: Vec<f32> = (0..FRAMES)
        .flat_map(|i| {
            let x = 0.8 * (i as f32 * std::f32::consts::TAU / 20.0).sin();
            [x, x * 0.5].into_iter().take(channels)
        })
        .collect();
    Arc::new(RawPeaks::new(data.into(), channels).unwrap())
}

fn wave(channels: usize) -> Waveform {
    let mut w = Waveform::new();
    w.set_peaks(Some(sound(channels)), Waveform::wave_rect(R).w);
    w
}

fn paint(w: &dyn Widget, r: Rect) -> SoftCanvas {
    let mut c = SoftCanvas::new(r.right(), r.bottom());
    let theme = Theme::dark();
    let mut fonts = Fonts::embedded();
    w.paint(&mut PaintCx { canvas: &mut c, theme: &theme, fonts: &mut fonts }, r);
    c
}

/// The y of level `v` in a mono waveform's lane.
fn level_y(v: f32) -> i32 {
    let lane = Waveform::wave_rect(R).inset_xy(0, 6);
    let half = (lane.h / 2 - 2) as f32;
    lane.y + lane.h / 2 - (v * half).round() as i32
}

/// The screen x of `frame` in the waveform.
fn x_of(w: &Waveform, frame: f64) -> i32 {
    Waveform::wave_rect(R).x + w.view().x_of(frame).round() as i32
}

#[test]
fn views_zoom_about_a_frame_and_stay_inside_the_sound() {
    let v = WaveView::whole(44_100, 1000);
    assert_eq!((v.start, v.frames_per_px), (0.0, 44.1));
    assert!((v.x_of(v.frame_at(123.0)) - 123.0).abs() < 1e-9);
    let z = v.zoomed(4.0, 22_050.0, 500.0, 44_100, 1000);
    assert!((z.frames_per_px - 11.025).abs() < 1e-9 && (z.frame_at(500.0) - 22_050.0).abs() < 1e-6, "{z:?}");
    // No further out than the whole sound, no closer than 16 pixels a frame.
    assert_eq!(z.zoomed(0.001, 0.0, 0.0, 44_100, 1000), v);
    assert_eq!(v.zoomed(1e9, 100.0, 0.0, 44_100, 1000).frames_per_px, WaveView::MIN_FRAMES_PER_PX);
    // Zooming near the end keeps the view inside the sound.
    let e = v.zoomed(10.0, 44_000.0, 999.0, 44_100, 1000);
    assert!(e.end(1000) <= 44_100.0 + 1e-6 && e.start >= 0.0, "{e:?}");
    let bad = WaveView { start: f64::NAN, frames_per_px: f64::INFINITY }.clamped(44_100, 1000);
    assert_eq!(bad, v);
}

#[test]
fn the_wave_is_drawn_bright_inside_the_region_and_dim_outside_with_the_loop_shaded() {
    let t = Theme::dark();
    let mut w = wave(1);
    w.set_region(Some((10_000, 30_000)));
    w.set_loop(Some((15_000, 25_000)));
    let c = paint(&w, R);
    let (inside, outside) = (x_of(&w, 20_000.0), x_of(&w, 5_000.0));
    // Between the RMS (0.57) and the peak (0.8): the wave's own colour.
    let y = level_y(0.7);
    assert_eq!(c.pixel(inside, y), t.accent);
    assert_eq!(c.pixel(outside, y), t.accent.mix(t.surface, 0.62));
    // Above the peak: the panel, tinted green over the loop.
    let above = level_y(0.95);
    assert_eq!(c.pixel(outside, above), t.surface);
    assert_eq!(c.pixel(x_of(&w, 12_000.0), above), t.surface);
    let shaded = c.pixel(inside, above);
    assert!(shaded != t.surface && shaded.g > t.surface.g, "the loop is shaded green: {shaded:?}");
}

#[test]
fn stereo_has_a_lane_for_each_side() {
    let c = paint(&wave(2), R);
    let lane = Waveform::wave_rect(R).inset_xy(0, 6);
    let h = (lane.h - 6) / 2;
    let x = 500;
    // Each lane's centre line is drawn on; the right side is half as loud,
    // so its lane is empty where the left's peak would reach.
    let (l_mid, r_mid) = (lane.y + h / 2, lane.y + h + 6 + h / 2);
    let reach = |mid: i32, v: f32| mid - (v * (h / 2 - 2) as f32).round() as i32;
    let t = Theme::dark();
    assert_eq!(c.pixel(x, reach(l_mid, 0.7)), t.accent);
    assert_eq!(c.pixel(x, reach(r_mid, 0.7)), t.surface);
    assert_ne!(c.pixel(x, reach(r_mid, 0.3)), t.surface);
}

#[test]
fn markers_bar_lines_and_fades_are_drawn() {
    let t = Theme::dark();
    let mut w = wave(1);
    w.set_markers(vec![Marker::new(10_000, "START", MarkerKind::Edge), Marker::new(20_000, "LOOP", MarkerKind::Loop)]);
    w.set_region(Some((10_000, 40_000)));
    w.set_fades(4_410, 0);
    w.set_beats(Some(Beats { first: 0.0, frames_per_beat: 5_512.5, per_bar: 4 }));
    let c = paint(&w, R);
    let top = Waveform::wave_rect(R).y + 2; // above the lane, where no wave is drawn
    assert_eq!(c.pixel(x_of(&w, 10_000.0), top), t.accent);
    assert_eq!(c.pixel(x_of(&w, 20_000.0), top), t.good);
    // Tabs in the strip, coloured like their markers.
    assert_eq!(c.pixel(x_of(&w, 20_000.0) - 20, R.y + 8), t.good);
    // The second bar line (beat 4) is drawn.
    assert_eq!(c.pixel(x_of(&w, 22_050.0), top), t.text_faint);
    // The fade-in curve starts at the bottom at the region's start and is
    // halfway up halfway through.
    let wave_r = Waveform::wave_rect(R);
    let x = x_of(&w, 12_205.0);
    let gain = (w.view().frame_at(f64::from(x - wave_r.x)) - 10_000.0) / 4_410.0;
    let y = wave_r.bottom() - 2 - ((wave_r.h - 4) as f64 * gain).round() as i32;
    assert!((gain - 0.5).abs() < 0.02, "{gain}");
    assert_eq!(c.pixel(x, y), t.warn);
}

#[test]
fn markers_drag_by_their_tabs() {
    let mut w = wave(1);
    w.set_markers(vec![Marker::new(10_000, "START", MarkerKind::Edge), Marker::new(30_000, "END", MarkerKind::Edge)]);
    paint(&w, R); // tabs are hit where they were drawn
    let x = x_of(&w, 10_000.0);
    assert_eq!(w.touch(1, Touch::Down, x, R.y + 15, R), Some(EventKind::Marker { index: 0, frame: 10_000 }));
    let to = w.view().frame_at(f64::from(x + 100 - Waveform::wave_rect(R).x)).round() as u64;
    assert_eq!(w.touch(1, Touch::Move, x + 100, R.y + 200, R), Some(EventKind::Marker { index: 0, frame: to }));
    assert_eq!(w.markers()[0].frame, to, "the widget moves it at once");
    assert_eq!(w.touch(1, Touch::Up, x + 100, R.y + 200, R), None);
    // Dragged past the end of the sound, it stops there.
    let end = x_of(&w, 30_000.0);
    w.touch(2, Touch::Down, end, R.y + 15, R);
    assert_eq!(w.touch(2, Touch::Move, R.right() + 500, R.y + 15, R), Some(EventKind::Marker { index: 1, frame: FRAMES as u64 }));
    w.touch(2, Touch::Up, R.right() + 500, R.y + 15, R);
    // The strip away from any tab grabs nothing; the app can put a marker back.
    assert_eq!(w.touch(3, Touch::Down, x_of(&w, 2_000.0), R.y + 15, R), None);
    w.touch(3, Touch::Up, x_of(&w, 2_000.0), R.y + 15, R);
    w.set_marker_frame(0, 10_000);
    assert_eq!(w.markers()[0].frame, 10_000);
}

#[test]
fn tap_scroll_and_pinch() {
    let mut w = wave(1);
    let wr = Waveform::wave_rect(R);
    // A tap picks a frame.
    w.touch(1, Touch::Down, 400, 150, R);
    let at = w.view().frame_at(f64::from(400 - wr.x)).round() as u64;
    assert_eq!(w.touch(1, Touch::Up, 403, 150, R), Some(EventKind::Frame(at)), "where it went down");
    // Zoomed in four times, a drag scrolls (the wave follows the finger).
    w.zoom(4.0, wr.w);
    let before = w.view();
    w.touch(2, Touch::Down, 600, 150, R);
    assert_eq!(w.touch(2, Touch::Move, 603, 150, R), None, "a few pixels is still a tap");
    let ev = w.touch(2, Touch::Move, 500, 150, R);
    let start = before.start + 100.0 * before.frames_per_px;
    assert_eq!(ev, Some(EventKind::View { start, frames_per_px: before.frames_per_px }));
    assert_eq!(w.touch(2, Touch::Up, 500, 150, R), None, "a drag is not a tap");
    // Two fingers spreading zoom in about the frame between them, which
    // follows their midpoint.
    let v = w.view();
    let between = v.frame_at(f64::from(400 - wr.x));
    w.touch(3, Touch::Down, 300, 150, R);
    w.touch(4, Touch::Down, 500, 150, R);
    let Some(EventKind::View { frames_per_px, .. }) = w.touch(4, Touch::Move, 700, 150, R) else { panic!("a pinch moves the view") };
    assert!((frames_per_px - v.frames_per_px / 2.0).abs() < 1e-9);
    assert!((w.view().frame_at(f64::from(500 - wr.x)) - between).abs() < 1e-6);
    w.touch(3, Touch::Up, 300, 150, R);
    assert_eq!(w.touch(4, Touch::Up, 700, 150, R), None, "the end of a pinch is not a tap");
}

#[test]
fn a_moving_playhead_redraws_only_its_strips() {
    let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1024, 324);
    ui.add("wave", Waveform::new());
    ui.add_page("sound", "col(wave)").unwrap();
    let r = ui.rect_of("wave").unwrap();
    let wr = Waveform::wave_rect(r);
    ui.get_mut::<Waveform>("wave").unwrap().set_peaks(Some(sound(1)), wr.w);
    let mut c = SoftCanvas::new(1024, 324);
    ui.paint(&mut c);

    ui.get_mut::<Waveform>("wave").unwrap().set_playheads(&[11_025]);
    let d = ui.paint(&mut c);
    assert_eq!(d.len(), 1, "{d:?}");
    assert!(d[0].w <= 4 && d[0].h == wr.h, "{d:?}");
    let x = wr.x + ui.get::<Waveform>("wave").unwrap().view().x_of(11_025.0).round() as i32;
    assert_eq!(c.pixel(x, wr.y + 2), Theme::dark().text);

    ui.get_mut::<Waveform>("wave").unwrap().set_playheads(&[22_050]);
    let d = ui.paint(&mut c);
    assert_eq!(d.len(), 2, "where it was and where it is: {d:?}");
    assert!(d.iter().all(|p| p.w <= 4));
    assert_ne!(c.pixel(x, wr.y + 2), Theme::dark().text, "the old playhead is gone");

    // Anything else redraws the whole waveform.
    ui.get_mut::<Waveform>("wave").unwrap().set_markers(vec![Marker::new(100, "S", MarkerKind::Slice)]);
    let d = ui.paint(&mut c);
    assert_eq!(d, vec![r]);
}

#[test]
fn the_overview_boxes_the_view_and_moves_it() {
    let t = Theme::dark();
    let r = Rect::new(0, 0, 1000, 60);
    let mut o = WaveOverview::new();
    o.set_peaks(Some(sound(2)));
    let view = WaveView::whole(FRAMES as u64, 988).zoomed(4.0, 22_050.0, 494.0, FRAMES as u64, 988);
    o.set_shown(view, 988);
    let c = paint(&o, r);
    // The box's left edge sits where the view starts (a quarter of the way in).
    let inner_x = 6;
    let left = inner_x + (view.start / (FRAMES as f64 / 988.0)).round() as i32 - 2;
    assert_eq!(c.pixel(left, 30), t.accent);
    assert_eq!(c.pixel(inner_x + 10, 3), t.surface, "nothing boxed at the start");
    // A touch says where to centre the view.
    assert_eq!(o.touch(1, Touch::Down, 6 + 494, 30, r), Some(EventKind::Frame(22_050)));
    assert_eq!(o.touch(1, Touch::Up, 6 + 494, 30, r), None);
    let _: Color = c.pixel(0, 0);
    let _ = c.size();
}
