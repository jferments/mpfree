//! Tape: wow and flutter on a pad, and tape stop and start on the whole
//! sampler.

use mpfree_analysis::yin;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn sine(seconds: f64, hz: f64) -> Source {
    let n = (seconds * f64::from(RATE)) as usize;
    let v = (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin()) as f32)
        .collect();
    Source::new("sine", RATE, 1, v).unwrap()
}

/// Pad 1 plays four seconds of 440 Hz once, with `wow`.
fn project(wow: f32) -> Project {
    let s = sine(4.0, 440.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut pad = Pad::new("sine", Layer::new(Region::whole(&s)));
    pad.wow = wow;
    p.set_pad(0, Some(pad), None).unwrap();
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    e
}

fn render(e: &mut Engine, frames: usize) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

fn pitch(x: &[f32]) -> f32 {
    yin(x, RATE, 1024, 200.0, 3000.0, 0.15).expect("pitched").0
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// The pitch every 50 ms over `x`.
fn pitches(x: &[f32]) -> Vec<f32> {
    x.chunks_exact(2205)
        .filter(|c| c.len() >= 2048)
        .map(|c| pitch(&c[..2048]))
        .collect()
}

#[test]
fn wow_and_flutter_wander_the_pitch_and_none_holds_it_still() {
    // Peak to peak, in cents: still without wow; at full wow, the slow
    // wobbles' 50 cents and the flutter's 10 at most.
    for (wow, lo, hi) in [(0.0f32, 0.0f32, 0.05f32), (1.0, 10.0, 60.0)] {
        let p = project(wow);
        let mut e = engine(&p);
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let f = pitches(&render(&mut e, 3 * RATE as usize));
        let cents = |a: f32| 1200.0 * (a / 440.0).log2();
        let max = f.iter().map(|&x| cents(x)).fold(f32::MIN, f32::max);
        let min = f.iter().map(|&x| cents(x)).fold(f32::MAX, f32::min);
        let swing = max - min;
        assert!(
            (lo..=hi).contains(&swing),
            "wow {wow}: pitch swings {min:.1} to {max:.1} cents"
        );
    }
}

#[test]
fn a_tape_stop_slows_the_sound_to_a_silent_halt_and_a_start_brings_it_back() {
    let p = project(0.0);
    let mut e = engine(&p);
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    render(&mut e, 4410);
    e.apply(Event::Tape {
        stop: true,
        ms: 1000,
    });
    // A quarter of the way down: about three quarters of the speed.
    let x = render(&mut e, RATE as usize / 4 + 2048);
    let f = pitch(&x[x.len() - 2048..]);
    assert!((f / 440.0 - 0.73).abs() < 0.05, "{f}");
    // Past a second, the tape has stopped: silence, the voice still there.
    render(&mut e, RATE as usize);
    assert_eq!(e.tape_speed(), 0.0);
    assert!(rms(&render(&mut e, 4096)) < 1e-6);
    assert_eq!(e.active_voices(), 1);
    // A hit while it is stopped waits silent too.
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    assert!(rms(&render(&mut e, 4096)) < 1e-6);
    // Started at once: both play at full speed again.
    e.apply(Event::Tape { stop: false, ms: 0 });
    let x = render(&mut e, 8192);
    assert_eq!(e.tape_speed(), 1.0);
    assert!((pitch(&x[4096..6144]) - 440.0).abs() < 1.0);
    assert!(rms(&x) > 0.1);
}

#[test]
fn a_running_tape_plays_exactly_as_without_one() {
    // Started and stopped before a sound plays, the tape leaves no trace.
    let p = project(0.0);
    let mut a = engine(&p);
    let mut b = engine(&p);
    b.apply(Event::Tape { stop: true, ms: 10 });
    render(&mut b, 1024);
    b.apply(Event::Tape {
        stop: false,
        ms: 10,
    });
    render(&mut b, 1024);
    render(&mut a, 2048);
    for e in [&mut a, &mut b] {
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 100,
        });
    }
    assert_eq!(render(&mut a, 8192), render(&mut b, 8192));
}

#[test]
fn wow_is_kept_between_0_and_1() {
    let s = sine(0.1, 440.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut pad = Pad::new("sine", Layer::new(Region::whole(&s)));
    pad.wow = 1.5;
    assert!(p.set_pad(0, Some(pad.clone()), None).is_err());
    pad.wow = f32::NAN;
    assert!(p.set_pad(0, Some(pad), None).is_err());
}
