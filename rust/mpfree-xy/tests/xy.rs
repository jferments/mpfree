//! The XY engine from outside: the pad's glide and stay, layouts checked
//! before they play, each curve read back to where it stands, and a
//! gesture playing a layout from landing to lift.

use mpfree_param::{Owner, ParamAddr, Strip};
use mpfree_xy::{
    Axis, Compiled, Device, Gesture, How, Layout, LayoutError, Lift, MapCurve, Pad, Phase, Start,
    Step, Target, XyInput, SLOTS, TARGETS,
};

fn addr(id: u16) -> ParamAddr {
    ParamAddr::new(Owner::Insert(Strip::Master, 4), id)
}

fn layout(across: &[Target], up: &[Target]) -> Layout {
    Layout {
        name: "Test".to_string(),
        across: Axis::new("Across", across),
        up: Axis::new("Up", up),
        rest: (0.5, 0.0),
        lift: Lift::GoBack { snap_ms: 60.0 },
        gate: None,
        hold: false,
        start: Start::Jump,
        follow: false,
    }
}

#[test]
fn the_pad_glides_back_when_the_finger_lifts_or_stays() {
    let mut p = Pad::new((0.5, 0.0), Lift::GoBack { snap_ms: 60.0 });
    assert!(p.at_rest());
    assert!(!p.tick(16.0), "nothing to do at rest");
    p.touch(1.0, 1.0);
    assert!(p.touched() && !p.at_rest());
    assert!(!p.tick(10.0), "no glide under a finger");
    assert_eq!(p.position(), (1.0, 1.0));
    // Lifted: back where it rests within five glide times, a frame at a
    // time.
    p.release();
    let mut ms = 0.0;
    while p.tick(16.0) {
        ms += 16.0;
        assert!(ms < 5.0 * 60.0 + 32.0, "still gliding after {ms} ms");
    }
    assert!(p.at_rest());
    assert_eq!(p.position(), (0.5, 0.0));
    // Staying: it keeps where the finger left it until it goes back again.
    p.set_lift(Lift::Stay);
    assert!(p.stays());
    p.touch(0.25, 0.5);
    p.release();
    assert!(!p.tick(100.0));
    assert_eq!(p.position(), (0.25, 0.5));
    p.set_lift(Lift::GoBack { snap_ms: 60.0 });
    assert!(p.tick(16.0));
    let (x, y) = p.position();
    assert!(x > 0.25 && x < 0.5 && y < 0.5, "gliding back: {x}, {y}");
    // A touch off the pad lands on its edge; a rest off it too.
    p.touch(-1.0, f32::NAN);
    assert_eq!(p.position(), (0.0, 0.0));
    assert_eq!(Pad::new((2.0, -1.0), Lift::Stay).rest(), (1.0, 0.0));
}

#[test]
fn a_layout_that_cant_play_says_why() {
    let t = Target::linear(addr(1), 0.0, 100.0);
    let u = Target::linear(addr(2), 0.0, 100.0);
    assert!(layout(&[t], &[u]).compile().is_ok());
    assert!(
        layout(&[], &[]).compile().is_ok(),
        "an empty pad plays nothing"
    );
    // Eight a way at most.
    let way = |first: u16, n: usize| -> Vec<Target> {
        (0..n)
            .map(|i| Target::linear(addr(first + i as u16), 0.0, 1.0))
            .collect()
    };
    assert!(layout(&way(10, TARGETS), &way(20, TARGETS))
        .compile()
        .is_ok());
    let nine = way(30, TARGETS + 1);
    let e = layout(&[t], &nine).compile().unwrap_err();
    assert_eq!(
        e,
        LayoutError::TooMany {
            axis: "Up",
            count: 9
        }
    );
    assert_eq!(e.to_string(), "Up moves 9 settings; it can move 8 at most");
    let bad = Target::linear(addr(2), 0.0, f32::INFINITY);
    assert_eq!(
        layout(&[bad], &[]).compile(),
        Err(LayoutError::NotANumber(addr(2)))
    );
    let mut off = layout(&[t], &[u]);
    off.rest = (0.5, 1.5);
    assert_eq!(off.compile(), Err(LayoutError::Rest));
    let mut glide = layout(&[t], &[u]);
    for snap_ms in [-5.0, 2000.5, f32::NAN] {
        glide.lift = Lift::GoBack { snap_ms };
        assert_eq!(glide.compile(), Err(LayoutError::Glide), "{snap_ms}");
    }
    for snap_ms in [0.0, 120.0, 2000.0] {
        glide.lift = Lift::GoBack { snap_ms };
        assert!(glide.compile().is_ok(), "{snap_ms}: 0 lets go at once");
    }
    glide.lift = Lift::Stay;
    assert!(glide.compile().is_ok());
    // A setting goes on a layout once: not on both ways, not twice on one,
    // not as the gate as well.
    for (across, up) in [(vec![t], vec![t]), (vec![t, u, t], vec![])] {
        let e = layout(&across, &up).compile().unwrap_err();
        assert_eq!(e, LayoutError::Twice(addr(1)));
        assert!(e.to_string().contains("twice"), "{e}");
    }
    let gated = Layout {
        gate: Some(addr(2)),
        ..layout(&[t], &[u])
    };
    assert_eq!(gated.compile(), Err(LayoutError::Twice(addr(2))));
    // An Add setting pushes at most its whole travel either way.
    assert!(layout(&[Target::add(addr(1), -1.0, 1.0)], &[])
        .compile()
        .is_ok());
    assert_eq!(
        layout(&[Target::add(addr(1), 0.0, 1.5)], &[]).compile(),
        Err(LayoutError::Push(addr(1)))
    );
}

/// Putting a setting on a direction (SHIFT + touch, the Modulate chips):
/// once per layout, moving from the other direction, eight a way.
#[test]
fn a_setting_goes_on_one_direction_once() {
    let mut l = Layout::new("Squelch");
    assert_eq!(
        l.lift,
        Lift::GoBack { snap_ms: 120.0 },
        "Go back by default"
    );
    assert!(!l.hold && !l.follow && l.start == Start::Jump);
    let t = Target::linear(addr(1), 0.0, 100.0);
    assert_eq!(l.put(false, t), Ok(false));
    assert_eq!(l.way_of(addr(1)), Some(false));
    // Again on the same way: its range changes, nothing is added.
    assert_eq!(l.put(false, Target::linear(addr(1), 20.0, 80.0)), Ok(false));
    assert_eq!(l.across.targets.len(), 1);
    assert_eq!(l.across.targets[0].from, 20.0);
    // On the other way: it moves there.
    assert_eq!(l.put(true, t), Ok(true));
    assert_eq!(l.way_of(addr(1)), Some(true));
    assert!(l.across.targets.is_empty() && l.up.targets.len() == 1);
    assert!(l.compile().is_ok());
    for i in 0..7 {
        l.put(true, Target::linear(addr(10 + i), 0.0, 1.0)).unwrap();
    }
    assert_eq!(
        l.put(true, Target::linear(addr(30), 0.0, 1.0)),
        Err(LayoutError::TooMany {
            axis: "Up",
            count: 9
        }),
        "eight a way"
    );
    assert!(l.take_off(addr(1)) && !l.take_off(addr(1)));
    assert_eq!(l.way_of(addr(1)), None);
}

/// Where a touch puts the pad: at the finger, or from where it is.
#[test]
fn a_touch_jumps_or_moves_the_pad_from_where_it_is() {
    let here = (0.5, 0.5);
    let landed = (0.9, 0.1);
    assert_eq!(Start::Jump.place(here, landed, (0.8, 0.2)), (0.8, 0.2));
    let p = Start::FromHere.place(here, landed, (0.8, 0.2));
    assert!(
        (p.0 - 0.4).abs() < 1e-6 && (p.1 - 0.6).abs() < 1e-6,
        "{p:?}"
    );
    assert_eq!(Start::FromHere.place(here, landed, landed), here);
    assert_eq!(Start::FromHere.place(here, landed, (0.0, 1.0)), (0.0, 1.0));
}

/// The sound loop's view: each setting in its place with how it plays,
/// and the same layout pointed at another track's settings.
#[test]
fn a_compiled_layout_gives_each_setting_its_place_and_how() {
    let mut l = Layout::new("Wobble");
    l.put(false, Target::linear(addr(1), 0.0, 10.0)).unwrap();
    l.put(false, Target::add(addr(2), 0.0, 0.5)).unwrap();
    l.put(true, Target::add(addr(3), 0.5, -0.5)).unwrap();
    l.hold = true;
    l.start = Start::FromHere;
    l.follow = true;
    let c = l.compile().unwrap();
    assert!(c.hold() && c.follows() && c.start() == Start::FromHere);
    assert_eq!(c.lift(), Lift::GoBack { snap_ms: 120.0 });
    assert_eq!(c.adds(), 2);
    let mut got = Vec::new();
    c.each(0.5, 1.0, |i, t, v| got.push((i, t.addr, t.how, v)));
    assert_eq!(
        got,
        [
            (0, addr(1), How::Set, 5.0),
            (1, addr(2), How::Add, 0.25),
            (TARGETS, addr(3), How::Add, -0.5),
        ]
    );
    assert_eq!(c.slot(TARGETS).map(|t| t.addr), Some(addr(3)));
    assert!(c.slot(SLOTS).is_none() && c.slot(2).is_none());
    let other = ParamAddr::new(Owner::Insert(Strip::Master, 9), 0);
    let m = c.mapped(|a| if a == addr(2) { other } else { a });
    assert!(m.plays(other) && !m.plays(addr(2)) && m.plays(addr(1)));
    assert_eq!(m.slot(1).map(|t| t.how), Some(How::Add));
}

#[test]
fn each_curve_reads_back_to_where_the_pad_stands() {
    for curve in [MapCurve::Linear, MapCurve::Exp, MapCurve::Log, MapCurve::S] {
        // Up and down: a `to` below `from` turns the setting the other
        // way.
        for (from, to) in [(-100.0, 100.0), (20.0, -4.0)] {
            let t = Target {
                curve,
                ..Target::linear(addr(1), from, to)
            };
            assert_eq!(t.value(0.0), from);
            assert_eq!(t.value(1.0), to);
            for k in 0..=20 {
                let pos = k as f32 / 20.0;
                let back = t.position(t.value(pos)).unwrap();
                assert!(
                    (back - pos).abs() < 1e-3,
                    "{curve:?} {from} {to}: {pos} {back}"
                );
            }
        }
    }
    // Off the ends: the nearer end.
    let t = Target::linear(addr(1), 0.0, 100.0);
    assert_eq!(t.value(2.0), 100.0);
    assert_eq!(t.position(-50.0), Some(0.0));
    // A setting that doesn't move, or a value that isn't one, has no place.
    assert_eq!(Target::linear(addr(1), 5.0, 5.0).position(5.0), None);
    assert_eq!(t.position(f32::NAN), None);
}

#[test]
fn a_compiled_layout_plays_its_settings_in_order() {
    let across = [
        Target::linear(addr(1), 0.0, 10.0),
        Target::linear(addr(2), 10.0, 0.0),
    ];
    let up = [Target {
        curve: MapCurve::Exp,
        ..Target::linear(addr(3), 0.0, 100.0)
    }];
    let c = Compiled::new(&across, &up, Some(addr(9))).unwrap();
    let mut got = Vec::new();
    c.values(0.5, 0.5, true, |a, v| got.push((a, v)));
    assert_eq!(
        got,
        [
            (addr(1), 5.0),
            (addr(2), 5.0),
            (addr(3), 25.0),
            (addr(9), 1.0)
        ]
    );
    assert!(c.plays(addr(2)) && c.plays(addr(9)) && !c.plays(addr(4)));
    // Where it stands, from each way's first setting.
    let at = c.position(|a| got.iter().find(|g| g.0 == a).map(|g| g.1));
    assert_eq!(at, (Some(0.5), Some(0.5)));
    assert_eq!(c.position(|_| None), (None, None));
    let none = Compiled::new(&[], &[], None).unwrap();
    assert_eq!(none.position(|_| Some(1.0)), (None, None));
}

#[test]
fn a_gesture_plays_a_layout_from_landing_to_lift() {
    let l = Layout {
        gate: Some(addr(9)),
        lift: Lift::Stay,
        ..layout(
            &[Target::linear(addr(1), 0.0, 100.0)],
            &[Target::linear(addr(2), 0.0, 100.0)],
        )
    };
    let c = l.compile().unwrap();
    let mut pad = Pad::new(l.rest, l.lift);
    let mut g = Gesture::default();
    let mut sent = Vec::new();
    let s = Device::Screen;
    let inputs = [
        XyInput::new(s, 1, 0, 0, Phase::Begin, 0.2, 0.4),
        // A second finger: ignored by its id.
        XyInput::new(s, 2, 1, 0, Phase::Begin, 0.9, 0.9),
        XyInput::new(s, 1, 2, 0, Phase::Move, 0.3, 0.4),
        XyInput::new(s, 1, 3, 0, Phase::End, 0.3, 0.4),
    ];
    for i in &inputs {
        match g.input(i) {
            Some(Step::Begin { x, y } | Step::Move { x, y }) => pad.touch(x, y),
            Some(Step::End { x, y }) => {
                pad.touch(x, y);
                pad.release();
            }
            None => continue,
        }
        let (x, y) = pad.position();
        c.values(x, y, pad.touched(), |a, v| sent.push((a, v)));
    }
    let gate: Vec<f32> = sent
        .iter()
        .filter(|s| s.0 == addr(9))
        .map(|s| s.1)
        .collect();
    assert_eq!(gate, [1.0, 1.0, 0.0], "open while held, shut on the lift");
    assert!(
        !sent.iter().any(|s| s.0 == addr(1) && s.1 > 50.0),
        "the second finger played"
    );
    assert_eq!(pad.position(), (0.3, 0.4), "it stays");
    assert_eq!(g.ignored, 1);
}
