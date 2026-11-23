//! A hit's own part of its pad's filter (the filter gap plan's F3): the
//! cutoff following the key, velocity on the filter envelope's depth and
//! attack, a random per hit, and a live envelope depth and Drive reaching
//! a note while it plays. Measured on a pad of the engine; the engine's own
//! tests (`engine::tests`) hold the numbers inside a voice.

use mpfree_sampler::dsp::{Filter, FilterKind, MAX_DRIVE_DB};
use mpfree_sampler::engine::{Engine, EngineView, Event, PadField};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const TAU: f32 = std::f32::consts::TAU;
/// The keygroup's root: A3, where its sine (220 Hz) plays as recorded.
const ROOT: u8 = 57;

fn sine(hz: f32, level: f32) -> Source {
    let n = RATE as usize * 3;
    let v = (0..n)
        .map(|i| level * (i as f32 * hz * TAU / RATE as f32).sin())
        .collect();
    Source::new("sine", RATE, 1, v).unwrap()
}

fn low_pass(cutoff_hz: f32) -> Filter {
    Filter {
        kind: FilterKind::LowPass,
        poles: 2,
        cutoff_hz,
        resonance: 0.0,
        env_octaves: 0.0,
        drive_db: 0.0,
    }
}

/// A pad of `s` (a gate, so it plays while held), its level the same at
/// any velocity, as `shape` leaves it; a keygroup over the whole keyboard
/// with `keys`.
fn project(s: &Source, keys: bool, shape: impl FnOnce(&mut Pad)) -> Project {
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    let mut pad = Pad::new("p", Layer::new(r));
    pad.vel_to_level = 0.0;
    shape(&mut pad);
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(pad), None).unwrap();
    if keys {
        p.spread_to_keys(0, ROOT).unwrap();
    }
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    e
}

fn run(e: &mut Engine, seconds: f32) -> Vec<f32> {
    let frames = (seconds * RATE as f32) as usize;
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

fn hit(e: &mut Engine, velocity: u8) {
    e.apply(Event::NoteOn { pad: 0, velocity });
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// Frames `a` to `b` seconds.
fn span(x: &[f32], a: f32, b: f32) -> &[f32] {
    &x[(a * RATE as f32) as usize..(b * RATE as f32) as usize]
}

/// The level of `x` at `hz` (one DFT bin over whole cycles), as a peak.
fn level_at(x: &[f32], hz: f32) -> f32 {
    let cycles = (x.len() as f32 * hz / RATE as f32).floor().max(1.0);
    let n = (cycles * RATE as f32 / hz) as usize;
    let (mut re, mut im) = (0.0f64, 0.0f64);
    for (i, v) in x[..n].iter().enumerate() {
        let w = f64::from(TAU * hz) * i as f64 / f64::from(RATE);
        re += f64::from(*v) * w.cos();
        im += f64::from(*v) * w.sin();
    }
    (2.0 * (re * re + im * im).sqrt() / n as f64) as f32
}

#[test]
fn with_key_tracking_a_note_an_octave_up_is_as_bright_as_the_root() {
    // A 220 Hz sine under a low-pass at 220 Hz: at the root it is at the
    // cutoff, 3 dB down. An octave up it plays at 440 Hz; tracking the
    // key, the cutoff goes up with it and the note is as loud, where a
    // cutoff left behind takes 12 dB off it.
    let s = sine(220.0, 0.25);
    let level = |track: f32, note: u8| {
        let p = project(&s, true, |pad| {
            pad.filter = low_pass(220.0);
            pad.key_track = track;
        });
        let mut e = engine(&p);
        e.apply(Event::Key {
            note,
            velocity: 100,
        });
        db(rms(span(&run(&mut e, 0.5), 0.2, 0.5)))
    };
    let root = level(1.0, ROOT);
    let up = level(1.0, ROOT + 12);
    let down = level(1.0, ROOT - 12);
    assert!((up - root).abs() < 0.1, "up {up} root {root}");
    assert!((down - root).abs() < 0.1, "down {down} root {root}");
    let behind = level(0.0, ROOT + 12) - level(0.0, ROOT);
    assert!(behind < -8.0, "{behind}");
}

#[test]
fn velocity_on_the_envelope_darkens_a_soft_hit_and_slows_its_attack() {
    // A 2 kHz sine under a low-pass at 250 Hz that the filter envelope
    // opens 3 octaves, to 2 kHz.
    let s = sine(2_000.0, 0.25);
    let played = |shape: fn(&mut Pad), velocity: u8, seconds: f32| {
        let p = project(&s, false, |pad| {
            pad.filter = Filter {
                env_octaves: 3.0,
                ..low_pass(250.0)
            };
            shape(pad);
        });
        let mut e = engine(&p);
        hit(&mut e, velocity);
        run(&mut e, seconds)
    };
    // On the depth: the hardest hit opens it all the way; the softest
    // hardly at all.
    let depth = |pad: &mut Pad| pad.vel_to_env = 1.0;
    let hard = db(rms(span(&played(depth, 127, 0.3), 0.1, 0.3)));
    let soft = db(rms(span(&played(depth, 1, 0.3), 0.1, 0.3)));
    assert!(soft < hard - 25.0, "soft {soft} hard {hard}");
    // On the attack: the hardest hit is open at once; the softest opens
    // over about a second.
    let attack = |pad: &mut Pad| pad.vel_to_attack = 1.0;
    let hard = played(attack, 127, 1.3);
    let soft = played(attack, 1, 1.3);
    let (h, s0, s1) = (
        db(rms(span(&hard, 0.02, 0.06))),
        db(rms(span(&soft, 0.02, 0.06))),
        db(rms(span(&soft, 1.1, 1.3))),
    );
    assert!(s0 < h - 25.0, "soft at the start {s0}, hard {h}");
    assert!((s1 - h).abs() < 0.5, "soft once open {s1}, hard {h}");
}

#[test]
fn random_moves_each_hit_and_the_same_performance_plays_the_same() {
    // A 1 kHz sine at a 1 kHz cutoff: up to 2 octaves either way on each
    // hit moves its level by several dB.
    let s = sine(1_000.0, 0.25);
    let p = project(&s, false, |pad| {
        pad.filter = Filter {
            resonance: 0.3,
            ..low_pass(1_000.0)
        };
        pad.cutoff_random = 1.0;
        pad.res_random = 1.0;
        pad.polyphony = 1;
    });
    let hits = || {
        let mut e = engine(&p);
        (0..6)
            .flat_map(|_| {
                hit(&mut e, 127);
                run(&mut e, 0.1)
            })
            .collect::<Vec<f32>>()
    };
    let (a, b) = (hits(), hits());
    assert_eq!(a, b, "the same performance, to the bit");
    let levels: Vec<f32> = (0..6)
        .map(|k| db(rms(&a[k * 4_410 + 1_500..(k + 1) * 4_410])))
        .collect();
    let (lo, hi) = levels
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
    assert!(hi - lo > 3.0, "{levels:?}");
    // Without it, every hit is the same.
    let mut q = p.clone();
    let mut pad = q.pad(0).cloned().unwrap();
    (pad.cutoff_random, pad.res_random) = (0.0, 0.0);
    q.set_pad(0, Some(pad), None).unwrap();
    let mut e = engine(&q);
    let levels: Vec<f32> = (0..6)
        .map(|_| {
            hit(&mut e, 127);
            db(rms(&run(&mut e, 0.1)[1_500..]))
        })
        .collect();
    assert!(
        levels.iter().all(|&x| (x - levels[0]).abs() < 0.01),
        "{levels:?}"
    );
}

#[test]
fn a_live_envelope_depth_and_drive_reach_the_note_playing() {
    // The envelope's depth: a 2 kHz sine under a low-pass at 250 Hz, then
    // a Q-Link (or a lock) opening the envelope 3 octaves, to 2 kHz.
    let s = sine(2_000.0, 0.25);
    let p = project(&s, false, |pad| pad.filter = low_pass(250.0));
    let mut e = engine(&p);
    hit(&mut e, 127);
    let shut = db(rms(span(&run(&mut e, 0.3), 0.1, 0.3)));
    e.apply(Event::PadLive {
        pad: 0,
        field: PadField::EnvDepth,
        value: 3.0,
    });
    let open = db(rms(span(&run(&mut e, 0.3), 0.1, 0.3)));
    assert!(open > shut + 25.0, "shut {shut} open {open}");
    assert_eq!(e.pad_live_value(0, PadField::EnvDepth), Some(3.0));
    // Out of range, it is kept to 4 octaves; NaN gives the pad's own back.
    e.apply(Event::PadLive {
        pad: 0,
        field: PadField::EnvDepth,
        value: 9.0,
    });
    assert_eq!(e.pad_live_value(0, PadField::EnvDepth), Some(4.0));
    e.apply(Event::PadLive {
        pad: 0,
        field: PadField::EnvDepth,
        value: f32::NAN,
    });
    let back = db(rms(span(&run(&mut e, 0.3), 0.1, 0.3)));
    assert!((back - shut).abs() < 0.5, "shut {shut} back {back}");

    // The Drive: a 200 Hz sine through an open Ladder gets its third
    // harmonic once driven.
    let s = sine(200.0, 0.25);
    let p = project(&s, false, |pad| {
        pad.filter = Filter {
            kind: FilterKind::Ladder,
            poles: 4,
            ..low_pass(18_000.0)
        };
    });
    let mut e = engine(&p);
    hit(&mut e, 127);
    let h3 = |x: &[f32]| db(level_at(x, 600.0) / level_at(x, 200.0));
    let clean = h3(span(&run(&mut e, 0.3), 0.1, 0.3));
    e.apply(Event::PadLive {
        pad: 0,
        field: PadField::Drive,
        value: MAX_DRIVE_DB,
    });
    let driven = h3(span(&run(&mut e, 0.3), 0.1, 0.3));
    assert!(
        clean < -60.0 && driven > -20.0,
        "clean {clean} driven {driven}"
    );
}
