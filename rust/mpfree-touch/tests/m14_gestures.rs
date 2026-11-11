//! M14: release velocity, swipes, fling and hold jitter. Synthetic input at
//! the panel's measured ~113 Hz (one report every 8.8 ms).

use mpfree_touch::gesture::{Fling, Gesture, GestureConfig, GestureRecognizer, SwipeDirection};
use mpfree_touch::jitter::hold_stats;
use mpfree_touch::mt::{ContactId, Phase};
use mpfree_touch::pointer::PointerEvent;

const DT: u64 = 8_800;

fn ev(id: u64, phase: Phase, x: f32, y: f32, t: u64) -> PointerEvent {
    PointerEvent {
        id: ContactId(id),
        phase,
        x,
        y,
        time_us: t,
    }
}

/// A one-finger stroke from (x0, y0) moving by (vx, vy) px per report for
/// `steps` reports, then `pause_us` still, then lift. Returns all gestures.
fn stroke(
    cfg: GestureConfig,
    x0: f32,
    y0: f32,
    vx: f32,
    vy: f32,
    steps: u64,
    pause_us: u64,
) -> Vec<Gesture> {
    let mut g = GestureRecognizer::new(cfg);
    let mut out = Vec::new();
    g.frame(&[ev(1, Phase::Down, x0, y0, 0)], &mut out);
    let (mut x, mut y) = (x0, y0);
    for i in 1..=steps {
        x += vx;
        y += vy;
        g.frame(&[ev(1, Phase::Move, x, y, i * DT)], &mut out);
    }
    let up = steps * DT + pause_us + DT;
    g.frame(&[ev(1, Phase::Up, x, y, up)], &mut out);
    out
}

fn drag_end(out: &[Gesture]) -> (f32, f32) {
    out.iter()
        .find_map(|g| match *g {
            Gesture::DragEnd { vx, vy, .. } => Some((vx, vy)),
            _ => None,
        })
        .expect("drag end")
}

fn swipe(out: &[Gesture]) -> Option<(SwipeDirection, f32)> {
    out.iter().find_map(|g| match *g {
        Gesture::Swipe {
            direction, speed, ..
        } => Some((direction, speed)),
        _ => None,
    })
}

#[test]
fn release_velocity_matches_the_stroke() {
    // 10 px per 8.8 ms = 1136 px/s to the right.
    let out = stroke(GestureConfig::default(), 100.0, 400.0, 10.0, 0.0, 30, 0);
    let (vx, vy) = drag_end(&out);
    assert!((vx - 1136.4).abs() < 1.0, "{vx}");
    assert_eq!(vy, 0.0);
}

#[test]
fn a_fast_straight_stroke_is_a_swipe_in_each_direction() {
    let c = GestureConfig::default();
    let cases = [
        ((10.0, 0.0), SwipeDirection::Right),
        ((-10.0, 0.0), SwipeDirection::Left),
        ((0.0, 10.0), SwipeDirection::Down),
        ((1.0, -10.0), SwipeDirection::Up),
    ];
    for ((vx, vy), want) in cases {
        let out = stroke(c, 640.0, 400.0, vx, vy, 20, 0);
        let (dir, speed) = swipe(&out).unwrap_or_else(|| panic!("no swipe for {want:?}"));
        assert_eq!(dir, want);
        assert!(speed > 1000.0);
        // The swipe comes right after its DragEnd.
        let n = out.len();
        assert!(matches!(
            out[n - 2],
            Gesture::DragEnd {
                cancelled: false,
                ..
            }
        ));
    }
}

#[test]
fn a_finger_that_stops_before_lifting_is_not_a_swipe() {
    // Fast stroke, then 200 ms still: the panel sends nothing, velocity is 0.
    let out = stroke(
        GestureConfig::default(),
        100.0,
        400.0,
        10.0,
        0.0,
        30,
        200_000,
    );
    assert_eq!(drag_end(&out), (0.0, 0.0));
    assert!(swipe(&out).is_none());
}

#[test]
fn slow_short_or_diagonal_strokes_are_not_swipes() {
    let c = GestureConfig::default();
    // Slow: 2 px per report = 227 px/s.
    assert!(swipe(&stroke(c, 100.0, 400.0, 2.0, 0.0, 100, 0)).is_none());
    // Short: fast but only 60 px.
    assert!(swipe(&stroke(c, 100.0, 400.0, 15.0, 0.0, 4, 0)).is_none());
    // Diagonal.
    assert!(swipe(&stroke(c, 100.0, 100.0, 10.0, 8.0, 30, 0)).is_none());
}

#[test]
fn a_cancelled_drag_has_no_velocity_and_no_swipe() {
    let mut g = GestureRecognizer::new(GestureConfig::default());
    let mut out = Vec::new();
    g.frame(&[ev(1, Phase::Down, 0.0, 0.0, 0)], &mut out);
    for i in 1..=20 {
        g.frame(
            &[ev(1, Phase::Move, i as f32 * 10.0, 0.0, i * DT)],
            &mut out,
        );
    }
    g.frame(&[ev(1, Phase::Cancel, 200.0, 0.0, 21 * DT)], &mut out);
    assert!(
        matches!(out.last(), Some(Gesture::DragEnd { cancelled: true, vx, vy, .. }) if *vx == 0.0 && *vy == 0.0)
    );
}

#[test]
fn a_tap_is_still_a_tap() {
    let out = stroke(GestureConfig::default(), 300.0, 300.0, 0.5, 0.0, 4, 0);
    assert!(matches!(out.as_slice(), [Gesture::Tap { .. }]), "{out:?}");
}

#[test]
fn fling_slows_down_and_stops_where_it_said() {
    let mut f = Fling::new(1200.0);
    let promised = f.remaining();
    let mut travelled = 0.0;
    let mut frames = 0;
    while !f.done() {
        travelled += f.step(1.0 / 60.0);
        frames += 1;
        assert!(frames < 600, "fling never stopped");
    }
    // Stopping below 20 px/s leaves out only the last few px.
    assert!(travelled > 0.0 && travelled <= promised);
    assert!(
        promised - travelled < 20.0 / f32::ln(2.0) * 0.25 + 1.0,
        "{promised} vs {travelled}"
    );
    // About 1.5 s for 1200 px/s to fall below 20 px/s with a 0.25 s half-life.
    assert!((80..=100).contains(&frames), "{frames}");
    assert_eq!(f.step(1.0 / 60.0), 0.0);
}

#[test]
fn fling_ignores_tiny_or_broken_velocities() {
    assert!(Fling::new(5.0).done());
    assert!(Fling::new(f32::NAN).done());
    let mut f = Fling::new(-800.0);
    assert!(f.step(0.1) < 0.0);
    assert_eq!(f.step(-1.0), 0.0);
}

#[test]
fn hold_stats_summarise_a_wandering_finger() {
    // 40 reports over ~3.5 s that alternate one px either side of (500, 300).
    let reports: Vec<_> = (0..40)
        .map(|i| {
            let d = if i % 2 == 0 { 1.0 } else { -1.0 };
            ev(
                1,
                if i == 0 { Phase::Down } else { Phase::Move },
                500.0 + d,
                300.0,
                i * 90_000,
            )
        })
        .collect();
    let s = hold_stats(&reports).unwrap();
    assert_eq!(s.samples, 40);
    assert_eq!(s.duration_us, 39 * 90_000);
    assert!((s.rate_hz - 11.1).abs() < 0.1, "{}", s.rate_hz);
    assert!((s.std_x - 1.0).abs() < 1e-4 && s.std_y == 0.0);
    assert!((s.max_dev - 2.0).abs() < 1e-4);
    assert!(hold_stats(&reports[..1]).is_none());
}

// ---- GestureRouter ------------------------------------------------------------

use mpfree_touch::route::GestureRouter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum W {
    Pad,
    Pages,
    List,
}

#[test]
fn a_held_pad_and_a_swipe_elsewhere_stay_independent() {
    let mut r = GestureRouter::new();
    // Pages wants swipes; the pad isn't registered (raw touches only).
    r.register(W::Pages, GestureConfig::default());
    let mut out = Vec::new();
    r.frame(
        &[
            (W::Pad, ev(1, Phase::Down, 100.0, 600.0, 0)),
            (W::Pages, ev(2, Phase::Down, 900.0, 300.0, 0)),
        ],
        &mut out,
    );
    for i in 1..=20u64 {
        r.frame(
            &[(
                W::Pages,
                ev(2, Phase::Move, 900.0 - 12.0 * i as f32, 300.0, i * DT),
            )],
            &mut out,
        );
    }
    r.frame(
        &[(W::Pages, ev(2, Phase::Up, 660.0, 300.0, 21 * DT))],
        &mut out,
    );
    // No two-finger gesture, nothing for the pad, and a left swipe for Pages.
    assert!(out.iter().all(|(k, _)| *k == W::Pages));
    assert!(!out
        .iter()
        .any(|(_, g)| matches!(g, Gesture::TwoFingerStart { .. })));
    assert!(out.iter().any(|(_, g)| matches!(
        g,
        Gesture::Swipe {
            direction: SwipeDirection::Left,
            ..
        }
    )));
}

#[test]
fn each_owner_has_its_own_config_and_long_press_comes_from_tick() {
    let mut r = GestureRouter::new();
    r.register(
        W::List,
        GestureConfig {
            long_press_us: 300_000,
            ..GestureConfig::default()
        },
    );
    r.register(
        W::Pad,
        GestureConfig {
            long_press_us: 800_000,
            ..GestureConfig::default()
        },
    );
    let mut out = Vec::new();
    r.frame(
        &[
            (W::List, ev(1, Phase::Down, 10.0, 10.0, 0)),
            (W::Pad, ev(2, Phase::Down, 500.0, 500.0, 0)),
        ],
        &mut out,
    );
    assert!(out.is_empty());
    r.tick(400_000, &mut out);
    assert_eq!(out.len(), 1);
    assert!(matches!(out[0], (W::List, Gesture::LongPress { .. })));
    r.tick(900_000, &mut out);
    assert!(matches!(out[1], (W::Pad, Gesture::LongPress { .. })));
}

#[test]
fn unregistering_or_clearing_drops_an_owner() {
    let mut r = GestureRouter::new();
    r.register(W::List, GestureConfig::default());
    assert!(r.is_registered(W::List));
    r.unregister(W::List);
    let mut out = Vec::new();
    r.frame(
        &[
            (W::List, ev(1, Phase::Down, 0.0, 0.0, 0)),
            (W::List, ev(1, Phase::Up, 0.0, 0.0, 50_000)),
        ],
        &mut out,
    );
    assert!(out.is_empty());
    r.register(W::Pages, GestureConfig::default());
    r.clear();
    assert!(!r.is_registered(W::Pages));
}
