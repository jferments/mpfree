//! The filter types sampler R5.6 adds beside low, high and band pass: a
//! Notch, a 4-pole Ladder low-pass with Drive, and a Vowel (two formants
//! the cutoff moves from "oo" to "ee"). Measured on a pad of the engine.

use mpfree_sampler::dsp::{
    ladder_db, vowel_db, vowel_formants, vowel_name, Filter, FilterKind, MAX_DRIVE_DB,
};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;
const TAU: f32 = std::f32::consts::TAU;

fn source(frames: usize, f: impl Fn(usize) -> f32) -> Source {
    Source::new("s", RATE, 1, (0..frames).map(f).collect()).unwrap()
}
fn sine(hz: f32, level: f32) -> Source {
    source(RATE as usize * 2, |i| {
        level * (i as f32 * hz * TAU / RATE as f32).sin()
    })
}

fn filter(kind: FilterKind, poles: u8, cutoff_hz: f32, resonance: f32, drive_db: f32) -> Filter {
    Filter {
        kind,
        poles,
        cutoff_hz,
        resonance,
        env_octaves: 0.0,
        drive_db,
    }
}

fn project(s: &Source, f: Filter) -> Project {
    let mut pad = Pad::new("p", Layer::new(Region::whole(s)));
    pad.filter = f;
    let mut p = Project::new();
    p.add_source(s.clone());
    p.set_pad(0, Some(pad), None).unwrap();
    p
}

fn run(e: &mut Engine, frames: usize) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

/// `seconds` of the pad hit at full velocity.
fn played(s: &Source, f: Filter, seconds: f32) -> Vec<f32> {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&project(s, f), RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    run(&mut e, (seconds * RATE as f32) as usize)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// The steady part (0.3 to 0.8 s), past every envelope's start.
fn steady(x: &[f32]) -> &[f32] {
    &x[(0.3 * RATE as f32) as usize..(0.8 * RATE as f32) as usize]
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// The level of a sine at `hz` through `f`, against the same with none.
fn gain_db(hz: f32, f: Filter) -> f32 {
    let s = sine(hz, 0.25);
    let off = filter(FilterKind::Off, 2, 20_000.0, 0.0, 0.0);
    db(rms(steady(&played(&s, f, 1.0))) / rms(steady(&played(&s, off, 1.0))))
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
fn the_notch_takes_out_its_cutoff_and_leaves_the_rest() {
    let n2 = filter(FilterKind::Notch, 2, 1_000.0, 0.0, 0.0);
    assert!(gain_db(1_000.0, n2) < -30.0, "{}", gain_db(1_000.0, n2));
    assert!(gain_db(100.0, n2).abs() < 0.5, "{}", gain_db(100.0, n2));
    assert!(
        gain_db(10_000.0, n2).abs() < 0.5,
        "{}",
        gain_db(10_000.0, n2)
    );
    // Four poles: two notches, a wider dip.
    let n4 = Filter { poles: 4, ..n2 };
    assert!(gain_db(800.0, n4) < gain_db(800.0, n2) - 3.0);
}

#[test]
fn the_ladder_falls_24_db_an_octave() {
    let lad = filter(FilterKind::Ladder, 4, 500.0, 0.0, 0.0);
    // Flat well below the cutoff, -12 dB at it (four stages at -3 dB),
    // and about 24 dB an octave above.
    assert!(gain_db(60.0, lad).abs() < 0.5, "{}", gain_db(60.0, lad));
    let at = gain_db(500.0, lad);
    assert!((at + 12.0).abs() < 1.0, "{at}");
    let (two, four) = (gain_db(2_000.0, lad), gain_db(4_000.0, lad));
    assert!((two - four - 24.0).abs() < 3.0, "{two} {four}");
}

#[test]
fn a_ladder_at_full_resonance_sings_at_its_cutoff_at_a_steady_level() {
    // A click into it, then nothing: it goes on ringing at its cutoff.
    let s = source(RATE as usize * 2, |i| if i < 32 { 0.5 } else { 0.0 });
    let lad = filter(FilterKind::Ladder, 4, 1_000.0, 1.0, 0.0);
    let x = played(&s, lad, 1.5);
    let (a, b) = (
        &x[(0.5 * RATE as f32) as usize..(0.7 * RATE as f32) as usize],
        &x[(1.2 * RATE as f32) as usize..(1.4 * RATE as f32) as usize],
    );
    let (ra, rb) = (rms(a), rms(b));
    assert!(ra > 0.02, "it rings: {ra}");
    assert!((db(ra) - db(rb)).abs() < 1.0, "steady: {ra} {rb}");
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 1.0, "within full scale: {peak}");
    // Near its cutoff (the loop's phase moves it a little).
    let crossings = b
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let hz = crossings as f32 / 2.0 / 0.2;
    assert!((hz / 1_000.0).log2().abs() < 0.25, "{hz} Hz");
}

#[test]
fn drive_adds_harmonics_and_keeps_the_level() {
    // -12 dBFS at the filter (the pad's centre pan takes 3 dB).
    let s = sine(200.0, 0.25 * std::f32::consts::SQRT_2);
    let clean = filter(FilterKind::Ladder, 4, 18_000.0, 0.0, 0.0);
    let driven = Filter {
        drive_db: MAX_DRIVE_DB,
        ..clean
    };
    let (a, b) = (played(&s, clean, 1.0), played(&s, driven, 1.0));
    let (a, b) = (steady(&a), steady(&b));
    // The level a sine at -12 dBFS comes in at, within 0.5 dB.
    assert!(
        (db(rms(b)) - db(rms(a))).abs() < 0.5,
        "{} {}",
        rms(a),
        rms(b)
    );
    // The third harmonic: next to nothing clean, plenty driven.
    let h3 = |x: &[f32]| db(level_at(x, 600.0) / level_at(x, 200.0));
    assert!(h3(a) < -60.0, "clean {}", h3(a));
    assert!(h3(b) > -20.0, "driven {}", h3(b));
    // A driven sound never goes past the drive's ceiling, however loud.
    let loud = sine(200.0, 1.0);
    let x = played(&loud, driven, 1.0);
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 0.5, "{peak}");
}

#[test]
fn drive_turned_up_on_a_held_note_does_not_click() {
    let s = sine(110.0, 0.25);
    let clean = filter(FilterKind::Ladder, 4, 2_000.0, 0.3, 0.0);
    let p = project(&s, clean);
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let before = run(&mut e, (0.4 * RATE as f32) as usize);
    let mut q = p.clone();
    let mut pad = q.pad(0).unwrap().clone();
    pad.filter.drive_db = 6.0;
    q.set_pad(0, Some(pad), None).unwrap();
    drop(e.edit_view(EngineView::build(&q, RATE)));
    let after = run(&mut e, (0.4 * RATE as f32) as usize);
    // The sharpest step from one frame to the next around the change, against
    // the steady sound's.
    let d = |x: &[f32]| {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    let steady_d = d(&before[before.len() - 4_410..]);
    let around = [&before[before.len() - 64..], &after[..4_410]].concat();
    assert!(
        d(&around) < steady_d * 1.6,
        "{} against {steady_d}",
        d(&around)
    );
    // And it is heard: the driven sound has its third harmonic.
    let tail = &after[after.len() - 8_820..];
    assert!(db(level_at(tail, 330.0) / level_at(tail, 110.0)) > -45.0);
}

#[test]
fn the_vowel_moves_from_oo_to_ee_with_the_cutoff() {
    assert_eq!(vowel_name(200.0), "oo");
    assert_eq!(vowel_name(1_000.0), "ah");
    assert_eq!(vowel_name(5_000.0), "ee");
    let (oo1, oo2) = vowel_formants(150.0);
    assert_eq!((oo1.round(), oo2.round()), (300.0, 870.0));
    let (ee1, ee2) = vowel_formants(8_000.0);
    assert_eq!((ee1.round(), ee2.round()), (270.0, 2290.0));
    // Glides between: "ah" to "eh" half way has its formants between.
    let (m1, m2) = vowel_formants(200.0 * 25f32.powf(2.5 / 4.0));
    assert!(m1 > 530.0 && m1 < 730.0 && m2 > 1090.0 && m2 < 1840.0);

    // On a pad: "ah" lets 730 Hz through over 2290 Hz, "ee" the other way,
    // as the picture's curve has it.
    let ah = filter(FilterKind::Vowel, 2, 1_000.0, 0.5, 0.0);
    let ee = filter(FilterKind::Vowel, 2, 5_000.0, 0.5, 0.0);
    assert!(gain_db(730.0, ah) > gain_db(2_290.0, ah) + 10.0);
    assert!(gain_db(2_290.0, ee) > gain_db(730.0, ee) + 10.0);
    for (hz, f) in [(730.0, ah), (2_290.0, ah), (2_290.0, ee), (400.0, ee)] {
        let picture = vowel_db(f.cutoff_hz, f.resonance, hz);
        assert!((gain_db(hz, f) - picture).abs() < 1.0, "{hz} {picture}");
    }
}

#[test]
fn a_saw_through_any_vowel_keeps_near_its_level() {
    let saw = source(RATE as usize * 2, |i| {
        let t = (i as f32 * 110.0 / RATE as f32).fract();
        0.25 * (2.0 * t - 1.0)
    });
    let off = filter(FilterKind::Off, 2, 20_000.0, 0.0, 0.0);
    let dry = rms(steady(&played(&saw, off, 1.0)));
    for cutoff in [200.0, 1_000.0, 5_000.0] {
        for res in [0.0, 1.0] {
            let f = filter(FilterKind::Vowel, 2, cutoff, res, 0.0);
            let d = db(rms(steady(&played(&saw, f, 1.0))) / dry);
            assert!(d > -8.0 && d < 0.0, "{cutoff} Hz, resonance {res}: {d} dB");
        }
    }
}

#[test]
fn the_ladder_is_as_its_picture() {
    for res in [0.0, 0.5] {
        let f = filter(FilterKind::Ladder, 4, 1_000.0, res, 0.0);
        for hz in [100.0, 700.0, 1_000.0, 1_400.0, 3_000.0] {
            let picture = ladder_db(1_000.0, res, hz);
            assert!(
                (gain_db(hz, f) - picture).abs() < 1.0,
                "{res} {hz} {picture}"
            );
        }
    }
}
