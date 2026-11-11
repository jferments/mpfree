//! Behaviour of the sampler engine and model, rendered offline.

use mpfree_sampler::dsp::{Ahdsr, Filter, FilterKind};
use mpfree_sampler::engine::{Engine, EngineView, Event, KILL_FRAMES};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const C: f32 = std::f32::consts::FRAC_1_SQRT_2; // centre pan gain

/// A mono ramp 0.001, 0.002, ... so every frame is recognisable.
fn ramp(frames: usize) -> Source {
    Source::new(
        "ramp",
        RATE,
        1,
        (1..=frames).map(|i| i as f32 * 0.001).collect(),
    )
    .unwrap()
}

fn dc(frames: usize, rate: u32) -> Source {
    Source::new("dc", rate, 1, vec![0.5; frames]).unwrap()
}

fn project_with(pads: &[(usize, Pad)], sources: Vec<Source>) -> Project {
    let mut p = Project::new();
    for s in sources {
        p.add_source(s);
    }
    for (i, pad) in pads {
        p.set_pad(*i, Some(pad.clone()), None).unwrap();
    }
    p
}

fn engine(p: &Project, voices: usize) -> Engine {
    let mut e = Engine::new(RATE, voices);
    e.set_view(EngineView::build(p, RATE));
    e
}

fn run(e: &mut Engine, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0; frames];
    let mut r = vec![0.0; frames];
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    (l, r)
}

fn last_nonzero(x: &[f32]) -> Option<usize> {
    x.iter().rposition(|v| v.abs() > 1e-9)
}

#[test]
fn one_shot_plays_the_region_exactly() {
    let s = ramp(1000);
    let mut r = Region::whole(&s);
    r.start = 100;
    r.end = 400;
    let p = project_with(&[(0, Pad::new("a", Layer::new(r)))], vec![s]);
    let mut e = engine(&p, 8);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    e.apply(Event::NoteOff { pad: 0 }); // one-shot ignores it
    let (l, rr) = run(&mut e, 512);
    for k in 0..300 {
        let want = (101 + k) as f32 * 0.001 * C;
        assert!((l[k] - want).abs() < 1e-5, "frame {k}: {} vs {want}", l[k]);
        assert_eq!(l[k], rr[k]);
    }
    assert_eq!(last_nonzero(&l), Some(299));
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn tuning_and_source_rate_change_the_length() {
    // An octave up plays a 1000-frame region in 500 frames.
    let s = dc(1000, RATE);
    let mut r = Region::whole(&s);
    r.tune_cents = 1200;
    let p = project_with(&[(0, Pad::new("up", Layer::new(r)))], vec![s]);
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 2048);
    assert_eq!(last_nonzero(&l), Some(499));

    // A 22.05 kHz source plays at its own speed: twice as many output frames.
    let s = dc(1000, 22_050);
    let p = project_with(
        &[(0, Pad::new("slow", Layer::new(Region::whole(&s))))],
        vec![s],
    );
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 4096);
    assert_eq!(last_nonzero(&l), Some(1999));
}

#[test]
fn reverse_plays_backwards() {
    let s = ramp(100);
    let mut r = Region::whole(&s);
    r.reverse = true;
    let p = project_with(&[(0, Pad::new("rev", Layer::new(r)))], vec![s]);
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 256);
    assert!((l[0] - 0.100 * C).abs() < 1e-6);
    assert!((l[99] - 0.001 * C).abs() < 1e-6);
    assert_eq!(last_nonzero(&l), Some(99));
}

fn looped(mode: PlayMode, loop_mode: LoopMode, crossfade: u32) -> (Project, Source) {
    let s = ramp(1000);
    let mut r = Region::whole(&s);
    r.mode = mode;
    r.sustain_loop = Some(Loop {
        start: 200,
        end: 300,
        crossfade,
        mode: loop_mode,
    });
    let p = project_with(&[(0, Pad::new("loop", Layer::new(r)))], vec![s.clone()]);
    (p, s)
}

#[test]
fn loop_mode_holds_then_plays_the_tail() {
    let (p, _) = looped(PlayMode::Loop, LoopMode::Forward, 0);
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 5000);
    assert_eq!(e.active_voices(), 1, "still looping while held");
    // Inside the loop, values stay between frame 200 and 300 of the ramp.
    for x in &l[1000..5000] {
        assert!(*x >= 0.200 * C - 1e-6 && *x <= 0.300 * C + 1e-6, "{x}");
    }
    e.apply(Event::NoteOff { pad: 0 });
    let (l, _) = run(&mut e, 2000);
    let end = last_nonzero(&l).unwrap();
    assert!(
        (l[end] - 1.0 * C).abs() < 1e-5,
        "the tail reaches the region's last frame"
    );
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn ping_pong_turns_at_both_loop_points() {
    let (p, _) = looped(PlayMode::Loop, LoopMode::PingPong, 0);
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 1000);
    // After entering the loop at frame 200 the ramp goes up to 300, down to 200, up again.
    let w = &l[200..600];
    let max = w.iter().cloned().fold(f32::MIN, f32::max);
    let min = w.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        (max - 0.300 * C).abs() < 2e-3 && (min - 0.201 * C).abs() < 2e-3,
        "{min} {max}"
    );
    assert!(l[350] < l[300], "going down on the way back");
}

#[test]
fn crossfaded_loop_has_no_jump_at_the_seam() {
    let (p, _) = looped(PlayMode::Loop, LoopMode::Forward, 50);
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 2000);
    let biggest = l[400..2000]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    // Without the crossfade the seam jumps by 0.1 * C (frame 300 -> 200).
    assert!(biggest < 0.01, "biggest step {biggest}");
}

#[test]
fn gate_releases_and_toggle_stops_on_the_next_hit() {
    let s = dc(100_000, RATE);
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Gate;
    let mut pad = Pad::new("gate", Layer::new(r.clone()));
    pad.amp = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 10.0,
    };
    r.mode = PlayMode::Toggle;
    let toggle = Pad::new("toggle", Layer::new(r));
    let p = project_with(&[(0, pad), (1, toggle)], vec![s]);
    let mut e = engine(&p, 8);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    e.apply(Event::NoteOn {
        pad: 1,
        velocity: 127,
    });
    run(&mut e, 1000);
    e.apply(Event::NoteOff { pad: 0 });
    e.apply(Event::NoteOff { pad: 1 }); // toggle ignores lifting
    run(&mut e, 1000); // 10 ms release is about 441 frames
    assert_eq!(e.pad_voices(0), 0);
    assert_eq!(e.pad_voices(1), 1);
    e.apply(Event::NoteOn {
        pad: 1,
        velocity: 127,
    });
    run(&mut e, 1000);
    assert_eq!(e.active_voices(), 0);
}

#[test]
fn mute_groups_and_one_way_chokes() {
    let s = dc(100_000, RATE);
    let mk = |n: &str| Pad::new(n, Layer::new(Region::whole(&s)));
    let mut open_hat = mk("open");
    open_hat.mute_group = Some(1);
    let mut closed_hat = mk("closed");
    closed_hat.mute_group = Some(1);
    let mut stopper = mk("stopper");
    stopper.chokes = vec![3];
    let target = mk("target");
    let p = project_with(
        &[(0, open_hat), (1, closed_hat), (2, stopper), (3, target)],
        vec![s.clone()],
    );
    let mut e = engine(&p, 16);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 100,
    });
    e.apply(Event::NoteOn {
        pad: 1,
        velocity: 100,
    });
    run(&mut e, usize::from(KILL_FRAMES) + 1);
    assert_eq!(
        (e.pad_voices(0), e.pad_voices(1)),
        (0, 1),
        "same group: the newer hit wins"
    );
    e.apply(Event::NoteOn {
        pad: 2,
        velocity: 100,
    });
    e.apply(Event::NoteOn {
        pad: 3,
        velocity: 100,
    });
    run(&mut e, 200);
    assert_eq!(
        (e.pad_voices(2), e.pad_voices(3)),
        (1, 1),
        "hitting the target does not stop the stopper"
    );
    e.apply(Event::NoteOn {
        pad: 2,
        velocity: 100,
    });
    run(&mut e, 200);
    assert_eq!(e.pad_voices(3), 0, "hitting the stopper stops the target");
}

#[test]
fn polyphony_limit_fades_the_oldest() {
    let s = dc(100_000, RATE);
    let mut pad = Pad::new("mono", Layer::new(Region::whole(&s)));
    pad.polyphony = 1;
    let p = project_with(&[(0, pad)], vec![s]);
    let mut e = engine(&p, 8);
    for _ in 0..4 {
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
    }
    let (l, _) = run(&mut e, 512);
    assert_eq!(e.pad_voices(0), 1);
    assert_eq!(e.active_voices(), 1);
    assert!(
        (l[200] - 0.5 * C).abs() < 1e-6,
        "after the fade only the newest hit sounds"
    );
}

#[test]
fn layers_by_velocity_cycle_and_stack() {
    let a = dc(10_000, RATE);
    let b = Source::new("b", RATE, 1, vec![0.25; 10_000]).unwrap();
    let mut soft = Layer::new(Region::whole(&a));
    soft.vel_hi = 63;
    let mut hard = Layer::new(Region::whole(&b));
    hard.vel_lo = 64;
    let mut by_vel = Pad::new("vel", soft.clone());
    by_vel.layers.push(hard.clone());
    by_vel.vel_to_level = 0.0;
    let mut cycle = Pad::new("rr", Layer::new(Region::whole(&a)));
    cycle.layers.push(Layer::new(Region::whole(&b)));
    cycle.select = LayerSelect::Cycle;
    cycle.vel_to_level = 0.0;
    let mut stack = cycle.clone();
    stack.select = LayerSelect::All;
    let p = project_with(&[(0, by_vel), (1, cycle), (2, stack)], vec![a, b]);
    // A fresh engine per hit, so nothing carries over.
    let hit = |pad: u8, vel: u8| -> f32 {
        let mut e = engine(&p, 16);
        e.apply(Event::NoteOn { pad, velocity: vel });
        run(&mut e, 4).0[0] / C
    };
    assert!(
        (hit(0, 30) - 0.5).abs() < 1e-6,
        "soft hit plays the soft layer"
    );
    assert!(
        (hit(0, 100) - 0.25).abs() < 1e-6,
        "hard hit plays the hard layer"
    );
    // Cycle alternates between the two on one engine.
    let mut e = engine(&p, 16);
    let mut seen = Vec::new();
    for _ in 0..4 {
        let before = run(&mut e, 1).0[0];
        e.apply(Event::NoteOn {
            pad: 1,
            velocity: 100,
        });
        let after = run(&mut e, 1).0[0];
        seen.push(((after - before) / C * 100.0).round() / 100.0);
    }
    assert_eq!(seen, vec![0.5, 0.25, 0.5, 0.25]);
    assert!((hit(2, 100) - 0.75).abs() < 1e-5, "all layers stack");
}

#[test]
fn full_level_ignores_velocity() {
    let s = dc(1000, RATE);
    let p = project_with(
        &[(0, Pad::new("v", Layer::new(Region::whole(&s))))],
        vec![s],
    );
    let mut e = engine(&p, 4);
    e.set_fixed_velocity(Some(127));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 10,
    });
    let (l, _) = run(&mut e, 2);
    assert!((l[0] - 0.5 * C).abs() < 1e-6);
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 64,
    });
    let (l, _) = run(&mut e, 2);
    let v = 64.0f32 / 127.0;
    assert!(
        (l[0] - 0.5 * C * v * v).abs() < 1e-6,
        "velocity follows a square law by default"
    );
}

#[test]
fn events_land_on_their_frame() {
    let s = dc(1000, RATE);
    let p = project_with(
        &[(0, Pad::new("v", Layer::new(Region::whole(&s))))],
        vec![s],
    );
    let mut e = engine(&p, 4);
    assert!(e.schedule(
        37,
        Event::NoteOn {
            pad: 0,
            velocity: 127
        }
    ));
    assert!(e.schedule(
        300,
        Event::NoteOn {
            pad: 0,
            velocity: 127
        }
    ));
    let (l, _) = run(&mut e, 512);
    assert_eq!(l.iter().position(|x| x.abs() > 0.0), Some(37));
    assert!(
        l[299] < 0.6 && l[300] > 0.6,
        "second hit at 300 adds a voice"
    );
}

#[test]
fn stealing_keeps_the_pool_bounded() {
    let s = dc(100_000, RATE);
    let p = project_with(
        &[(0, Pad::new("v", Layer::new(Region::whole(&s))))],
        vec![s],
    );
    let mut e = engine(&p, 2);
    for _ in 0..5 {
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
    }
    run(&mut e, 128);
    assert!(e.active_voices() <= 2);
    assert!(e.stats().steals >= 1);
}

#[test]
fn renders_are_deterministic() {
    let s = ramp(5000);
    let mut pad = Pad::new("v", Layer::new(Region::whole(&s)));
    pad.layers.push(Layer::new(Region::whole(&s)));
    pad.select = LayerSelect::Random;
    pad.filter = Filter {
        kind: FilterKind::LowPass,
        poles: 4,
        cutoff_hz: 800.0,
        resonance: 0.4,
        env_octaves: 2.0,
    };
    pad.filter_env = Ahdsr {
        attack_ms: 1.0,
        hold_ms: 0.0,
        decay_ms: 30.0,
        sustain: 0.2,
        release_ms: 50.0,
    };
    let p = project_with(&[(0, pad)], vec![s]);
    let script = |e: &mut Engine| {
        for k in 0..20u32 {
            e.schedule(
                k * 97 % 128,
                Event::NoteOn {
                    pad: 0,
                    velocity: (k * 13 % 127 + 1) as u8,
                },
            );
            run(e, 128);
        }
        run(e, 4096).0
    };
    let a = script(&mut engine(&p, 8));
    let b = script(&mut engine(&p, 8));
    assert_eq!(a, b);
    assert!(a.iter().all(|x| x.is_finite()));
}

#[test]
fn low_pass_takes_the_edge_off() {
    let s = Source::new(
        "square",
        RATE,
        1,
        (0..20_000)
            .map(|i| if (i / 4) % 2 == 0 { 0.5 } else { -0.5 })
            .collect(),
    )
    .unwrap();
    let mut open = Pad::new("open", Layer::new(Region::whole(&s)));
    let mut closed = open.clone();
    closed.filter = Filter {
        kind: FilterKind::LowPass,
        poles: 4,
        cutoff_hz: 300.0,
        resonance: 0.0,
        env_octaves: 0.0,
    };
    open.vel_to_level = 0.0;
    closed.vel_to_level = 0.0;
    let p = project_with(&[(0, open), (1, closed)], vec![s]);
    let energy = |pad: u8| {
        let mut e = engine(&p, 4);
        e.apply(Event::NoteOn { pad, velocity: 127 });
        run(&mut e, 8192).0[1000..]
            .iter()
            .map(|x| x * x)
            .sum::<f32>()
    };
    assert!(energy(1) < energy(0) * 0.05);
}

#[test]
fn view_swap_lets_playing_voices_finish() {
    let s = dc(2000, RATE);
    let p = project_with(
        &[(0, Pad::new("v", Layer::new(Region::whole(&s))))],
        vec![s],
    );
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    run(&mut e, 100);
    let old = e.set_view(EngineView::empty());
    drop(old);
    let (l, _) = run(&mut e, 4000);
    assert_eq!(last_nonzero(&l), Some(1899));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    assert_eq!(e.active_voices(), 0, "the new view has no pad 0");
}

#[test]
fn regions_are_checked() {
    let s = dc(1000, RATE);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.end = 2000;
    assert!(p
        .set_pad(0, Some(Pad::new("x", Layer::new(r))), None)
        .is_err());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Loop;
    assert!(
        p.set_pad(0, Some(Pad::new("x", Layer::new(r.clone()))), None)
            .is_err(),
        "loop mode needs loop points"
    );
    r.sustain_loop = Some(Loop {
        start: 100,
        end: 200,
        crossfade: 101,
        mode: LoopMode::Forward,
    });
    assert!(
        p.set_pad(0, Some(Pad::new("x", Layer::new(r.clone()))), None)
            .is_err(),
        "crossfade longer than half the loop"
    );
    r.sustain_loop = Some(Loop {
        start: 100,
        end: 200,
        crossfade: 50,
        mode: LoopMode::Forward,
    });
    assert!(p
        .set_pad(0, Some(Pad::new("x", Layer::new(r))), None)
        .is_ok());
    let other = dc(10, 48_000);
    assert!(
        p.set_pad(
            1,
            Some(Pad::new("y", Layer::new(Region::whole(&other)))),
            None
        )
        .is_err(),
        "source not in the project"
    );
}

#[test]
fn undo_redo_and_gestures() {
    let s = dc(1000, RATE);
    let mut p = Project::new();
    let id = p.add_source(s.clone());
    assert_eq!(p.add_source(s.clone()), id, "the same audio is stored once");
    assert_eq!(p.sources().len(), 1);
    p.set_pad(0, Some(Pad::new("a", Layer::new(Region::whole(&s)))), None)
        .unwrap();
    for k in 0..10 {
        p.edit_pad(0, Some(7), |pad| pad.amp.attack_ms = k as f32)
            .unwrap();
    }
    assert_eq!(p.pad(0).unwrap().amp.attack_ms, 9.0);
    assert!(p.undo(), "one undo reverses the whole knob turn");
    assert_eq!(p.pad(0).unwrap().amp.attack_ms, 0.0);
    assert!(p.redo());
    assert_eq!(p.pad(0).unwrap().amp.attack_ms, 9.0);
    assert_eq!(p.users_of(id), vec![0]);
    while p.undo() {}
    assert!(p.pad(0).is_none());
    assert!(
        p.sources().is_empty(),
        "undo reaches back past adding the source"
    );
    while p.redo() {}
    assert_eq!(p.pad(0).unwrap().amp.attack_ms, 9.0);
    assert_eq!(p.sources().len(), 1);
}

#[test]
fn a_cleared_history_keeps_the_project_as_it_is() {
    let s = dc(1000, RATE);
    let mut p = project_with(
        &[(0, Pad::new("a", Layer::new(Region::whole(&s))))],
        vec![s.clone()],
    );
    p.edit_pad(0, Some(1), |pad| pad.level_db = -3.0).unwrap();
    assert!(p.undo());
    let revision = p.revision();
    p.clear_history();
    assert!(!p.can_undo() && !p.can_redo());
    assert!(!p.undo(), "the loading steps are gone");
    assert_eq!(
        (p.pad(0).unwrap().level_db, p.sources().len(), p.revision()),
        (0.0, 1, revision)
    );
    p.edit_pad(0, Some(2), |pad| pad.level_db = -6.0).unwrap();
    assert!(p.undo());
    assert_eq!(p.pad(0).unwrap().level_db, 0.0);
    assert!(!p.undo());
}

#[test]
fn chop_sets_slice_and_go_to_pads() {
    let s = ramp(1600);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut c = ChopSet::grid("break", &s, 4).unwrap();
    assert_eq!(c.markers(), &[400, 800, 1200]);
    assert_eq!(c.add_marker(100).unwrap(), 5);
    c.merge(0).unwrap();
    assert_eq!(c.markers(), &[400, 800, 1200]);
    assert!(c.move_marker(1, 300).is_err(), "markers keep their order");
    c.move_marker(1, 700).unwrap();
    assert_eq!(c.bounds(1), Some((400, 700)));
    p.chop_sets.push(c);
    p.slices_to_pads(0, 0, 4, 4).unwrap();
    assert_eq!(p.pad(5).unwrap().name, "break 2");
    assert_eq!(p.pad(5).unwrap().layers[0].region.start, 400);
    let mut e = engine(&p, 8);
    e.apply(Event::NoteOn {
        pad: 7,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 1024);
    assert_eq!(last_nonzero(&l), Some(399), "the last slice is 1200..1600");
    assert!((l[0] - 1.201 * C).abs() < 1e-5);
}

#[test]
fn fades_shape_the_ends() {
    let s = dc(1000, RATE);
    let mut r = Region::whole(&s);
    r.fade_in = Fade {
        frames: 100,
        curve: FadeCurve::Linear,
    };
    r.fade_out = Fade {
        frames: 100,
        curve: FadeCurve::Exp,
    };
    let mut pad = Pad::new("f", Layer::new(r));
    pad.vel_to_level = 0.0;
    let p = project_with(&[(0, pad)], vec![s]);
    let mut e = engine(&p, 4);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (l, _) = run(&mut e, 1024);
    assert_eq!(l[0], 0.0);
    assert!((l[50] - 0.5 * C * 0.5).abs() < 1e-6);
    assert!((l[500] - 0.5 * C).abs() < 1e-6);
    assert!(
        (l[949] - 0.5 * C * 0.25).abs() < 1e-6,
        "exp fade out at its midpoint: {}",
        l[949] / (0.5 * C)
    );
}
