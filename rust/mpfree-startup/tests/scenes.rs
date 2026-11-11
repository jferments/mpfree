use mpfree_startup::{scene, Kind};
use mpfree_ui::SoftCanvas;

const KINDS: [Kind; 3] = Kind::ALL;

fn frame(kind: Kind, w: i32, h: i32, t: f32) -> Vec<u32> {
    let mut s = scene(kind, w, h);
    let mut c = SoftCanvas::new(w, h);
    s.draw(t, &mut c);
    c.px
}

#[test]
fn synthwave_is_the_default() {
    assert_eq!(Kind::from_name("default"), Some(Kind::Synthwave));
}

#[test]
fn the_same_time_draws_the_same_frame() {
    for k in KINDS {
        let mut s = scene(k, 1280, 800);
        let mut c = SoftCanvas::new(1280, 800);
        s.draw(2.7, &mut c);
        s.draw(0.4, &mut c);
        s.draw(2.7, &mut c);
        assert!(c.px == frame(k, 1280, 800, 2.7), "{k:?}");
    }
}

#[test]
fn it_moves_and_ends_with_something_to_see() {
    for k in KINDS {
        let len = scene(k, 1280, 800).length();
        let (a, b) = (frame(k, 1280, 800, 0.8), frame(k, 1280, 800, 1.6));
        assert!(a != b, "{k:?} does not move");
        let last = frame(k, 1280, 800, len + 0.5);
        let bright = last
            .iter()
            .filter(|&&p| ((p >> 16) & 255) + ((p >> 8) & 255) + (p & 255) > 400)
            .count();
        assert!(
            bright > 2000,
            "{k:?}: only {bright} bright pixels at the end"
        );
    }
}

#[test]
fn other_screen_sizes_work() {
    for k in KINDS {
        for (w, h) in [(800, 480), (320, 200), (1920, 1080)] {
            let mut s = scene(k, w, h);
            let mut c = SoftCanvas::new(w, h);
            for t in [0.0, 1.0, 2.0, 3.0, 6.0] {
                s.draw(t, &mut c);
            }
        }
    }
}
