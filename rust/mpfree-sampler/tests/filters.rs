//! The filter types sampler R5.6 adds beside low, high and band pass: a
//! Notch, a 4-pole Ladder low-pass with Drive, a Vowel (two formants the
//! cutoff moves from "oo" to "ee") and a Comb with feedback. Measured on a
//! pad of the engine.

use mpfree_sampler::dsp::{
    comb_db, comb_gain, ladder_db, vowel_db, vowel_formants, vowel_name, Filter, FilterKind,
    MAX_DRIVE_DB,
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
    // -12 dBFS at the filter (the pad's level and pan come after it).
    let s = sine(200.0, 0.25);
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

/// `seconds` of the pad at `level_db`, hit at `velocity`.
fn played_with(s: &Source, f: Filter, level_db: f32, velocity: u8, seconds: f32) -> Vec<f32> {
    let mut p = project(s, f);
    let mut pad = p.pad(0).cloned().unwrap();
    pad.level_db = level_db;
    p.set_pad(0, Some(pad), None).unwrap();
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e.apply(Event::NoteOn { pad: 0, velocity });
    run(&mut e, (seconds * RATE as f32) as usize)
}

#[test]
fn the_filter_hears_the_sound_before_the_pads_level_and_velocity() {
    // A driven, resonant Ladder's harmonics follow how hard the sound
    // drives it, so they show where the gains are: the pad's level and a
    // hit's velocity come after the filter (filter gap plan F2), so a
    // quieter pad or a softer hit is only quieter, driven just as hard.
    let s = sine(200.0, 0.25);
    let driven = filter(FilterKind::Ladder, 4, 2_000.0, 0.5, 12.0);
    let h3 = |x: &[f32]| db(level_at(x, 600.0) / level_at(x, 200.0));
    let full = played_with(&s, driven, 0.0, 127, 1.0);
    let (full_rms, full_h3) = (rms(steady(&full)), h3(steady(&full)));
    assert!(full_h3 > -40.0, "driven: {full_h3}");
    for (level_db, velocity) in [(-12.0, 127), (0.0, 40)] {
        let soft = played_with(&s, driven, level_db, velocity, 1.0);
        let (soft_rms, soft_h3) = (rms(steady(&soft)), h3(steady(&soft)));
        assert!(
            soft_rms < 0.5 * full_rms,
            "{level_db} dB, velocity {velocity}: {soft_rms} against {full_rms}"
        );
        assert!(
            (soft_h3 - full_h3).abs() < 0.01,
            "{level_db} dB, velocity {velocity}: third harmonic {soft_h3} against {full_h3}"
        );
    }
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

/// A click at the start of a sound `frames` long, the rest silence.
fn click(frames: usize) -> Source {
    source(frames, |i| if i == 0 { 0.5 } else { 0.0 })
}

/// Where the loudest of `x`'s period candidates from `lo` to `hi` frames
/// is: the lag (frames) at which `x` best matches itself.
fn period(x: &[f32], lo: usize, hi: usize) -> usize {
    (lo..=hi)
        .max_by(|&a, &b| {
            let c = |lag: usize| x.iter().zip(&x[lag..]).map(|(p, q)| p * q).sum::<f32>();
            c(a).total_cmp(&c(b))
        })
        .unwrap()
}

#[test]
fn the_comb_rings_at_its_cutoff() {
    // 441 Hz: a cycle of 100 frames.
    let f = filter(FilterKind::Comb, 2, 441.0, 1.0, 0.0);
    let out = played(&click(RATE as usize), f, 0.5);
    assert_eq!(period(&out[1_000..10_000], 50, 150), 100);
    // A higher pitch, between frames: 44100 / 3000 = 14.7.
    let f = filter(FilterKind::Comb, 2, 3_000.0, 1.0, 0.0);
    let out = played(&click(RATE as usize), f, 0.5);
    let p = period(&out[1_000..10_000], 10, 20);
    assert!((14..=15).contains(&p), "{p}");
    // Resonance is its feedback: at 0 the click passes as it is.
    let f = filter(FilterKind::Comb, 2, 441.0, 0.0, 0.0);
    let out = played(&click(RATE as usize), f, 0.5);
    assert!(out[200..].iter().all(|x| x.abs() < 1e-3));
}

#[test]
fn the_combs_picture_matches_the_engine() {
    for res in [0.3f32, 0.8] {
        let f = filter(FilterKind::Comb, 2, 200.0, res, 0.0);
        for hz in [200.0f32, 300.0, 1_000.0, 1_100.0] {
            let got = gain_db(hz, f);
            let want = comb_db(200.0, res, hz);
            assert!((got - want).abs() < 1.0, "{res} {hz} Hz: {got} {want}");
        }
    }
    // At full feedback a peak stands out, a gap between peaks is deep.
    assert!(comb_db(200.0, 1.0, 400.0) > 17.0);
    assert!(comb_db(200.0, 1.0, 500.0) < -15.0);
    assert!((comb_gain(0.0) - 1.0).abs() < 1e-6);
}

#[test]
fn the_comb_rings_on_after_its_sound_ends_and_stops_once_quiet() {
    // A sound of 1000 frames: the comb rings well past it, at its level,
    // then the voice ends.
    let f = filter(FilterKind::Comb, 2, 441.0, 1.0, 0.0);
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&project(&click(1_000), f), RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let out = run(&mut e, RATE as usize / 4);
    let at = |a: usize| out[a..a + 100].iter().fold(0f32, |m, x| m.max(x.abs()));
    assert!(at(900) > 0.01, "{}", at(900));
    // No step where the sound ended: the ringing goes straight on.
    assert!(at(1_000) > 0.9 * at(900), "{} {}", at(900), at(1_000));
    assert!(at(5_000) > 0.01);
    assert_eq!(e.active_voices(), 1);
    let _ = run(&mut e, 6 * RATE as usize);
    assert_eq!(e.active_voices(), 0, "it stops once too quiet to hear");
}

#[test]
fn a_cut_stops_the_combs_ringing() {
    let f = filter(FilterKind::Comb, 2, 441.0, 1.0, 0.0);
    let mut p = project(&click(1_000), f);
    let mut other = Pad::new("q", Layer::new(Region::whole(&click(1_000))));
    other.mute_group = Some(1);
    let mut pd = p.pad(0).unwrap().clone();
    pd.mute_group = Some(1);
    p.set_pad(0, Some(pd), None).unwrap();
    p.set_pad(1, Some(other), None).unwrap();
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let _ = run(&mut e, 4_000);
    assert_eq!(e.active_voices(), 1, "ringing");
    // Pad 2 of the same mute group chokes it: its ringing fades out and
    // doesn't start again.
    e.apply(Event::NoteOn {
        pad: 1,
        velocity: 127,
    });
    let _ = run(&mut e, 4_000);
    assert_eq!(e.active_voices(), 0);
}

/// A pad of `layers` stacked (every one plays), each a sound at a level
/// (dB), through `f`.
fn layered(layers: &[(&Source, f32)], f: Filter) -> Project {
    let mut p = Project::new();
    let mut pad: Option<Pad> = None;
    for &(s, level_db) in layers {
        p.add_source(s.clone());
        let mut l = Layer::new(Region::whole(s));
        l.level_db = level_db;
        match pad.as_mut() {
            None => pad = Some(Pad::new("p", l)),
            Some(pd) => pd.layers.push(l),
        }
    }
    let mut pad = pad.unwrap();
    pad.filter = f;
    p.set_pad(0, Some(pad), None).unwrap();
    p
}

/// `frames` of pad 0 of `p` hit at full velocity.
fn hit(p: &Project, frames: usize) -> Vec<f32> {
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(p, RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    run(&mut e, frames)
}

fn worst(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

#[test]
fn a_hits_layers_mix_before_one_filter() {
    // The layers a hit plays are mixed, each at its own level, and the
    // mix goes through one filter (filter gap plan F2c). Quiet enough
    // that the resonance guard stays out, the filter is linear: the hit
    // is its layers' sum, each filtered alone.
    let f = filter(FilterKind::LowPass, 2, 800.0, 0.4, 0.0);
    let (a, b) = (sine(220.0, 0.02), sine(1_500.0, 0.02));
    let n = RATE as usize / 2;
    let both = hit(&layered(&[(&a, 0.0), (&b, -6.0)], f), n);
    let one = hit(&layered(&[(&a, 0.0)], f), n);
    let two = hit(&layered(&[(&b, -6.0)], f), n);
    let sum: Vec<f32> = one.iter().zip(&two).map(|(x, y)| x + y).collect();
    let peak = sum.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak > 0.005, "{peak}");
    assert!(worst(&both, &sum) < 1e-3 * peak, "{}", worst(&both, &sum));
    // And one filter it is: through a driven Ladder, two tones
    // intermodulate (a tone at 2 x 200 - 300 = 100 Hz), as they would
    // through one filter, never through one each.
    let drive = filter(FilterKind::Ladder, 4, 2_000.0, 0.3, 12.0);
    let (lo, hi) = (sine(200.0, 0.25), sine(300.0, 0.25));
    let mixed = hit(&layered(&[(&lo, 0.0), (&hi, 0.0)], drive), RATE as usize);
    let apart: Vec<f32> = hit(&layered(&[(&lo, 0.0)], drive), RATE as usize)
        .iter()
        .zip(&hit(&layered(&[(&hi, 0.0)], drive), RATE as usize))
        .map(|(x, y)| x + y)
        .collect();
    let im = |x: &[f32]| db(level_at(steady(x), 100.0) / level_at(steady(x), 200.0));
    assert!(im(&mixed) > -45.0, "one filter: {}", im(&mixed));
    assert!(im(&apart) < -80.0, "a filter each: {}", im(&apart));
}

#[test]
fn a_hit_plays_on_while_any_of_its_layers_does() {
    // One layer's sound is short, the other's long, the short one first
    // or second: the hit's filter goes on with the long one until it
    // ends, then the voices end. Where the short one ends, the filter
    // rings out what it held of it (a layer alone is cut there): the hit
    // is its layers filtered alone before that and once the ringing has
    // died away, and has no step between.
    let f = filter(FilterKind::LowPass, 2, 800.0, 0.4, 0.0);
    let ends = RATE as usize / 10;
    let short = source(ends, |i| {
        0.02 * (i as f32 * 220.0 * TAU / RATE as f32).sin()
    });
    let long = source(RATE as usize * 6 / 10, |i| {
        0.02 * (i as f32 * 330.0 * TAU / RATE as f32).sin()
    });
    let n = RATE as usize * 8 / 10;
    let sum: Vec<f32> = hit(&layered(&[(&short, 0.0)], f), n)
        .iter()
        .zip(&hit(&layered(&[(&long, 0.0)], f), n))
        .map(|(x, y)| x + y)
        .collect();
    let step = |x: &[f32]| {
        x.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    for order in [[&short, &long], [&long, &short]] {
        let p = layered(&[(order[0], 0.0), (order[1], 0.0)], f);
        let mut e = Engine::new(RATE, 8);
        e.set_view(EngineView::build(&p, RATE));
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let out = run(&mut e, n);
        let rung = ends + RATE as usize / 20;
        assert!(worst(&out[..ends], &sum[..ends]) < 1e-5);
        assert!(worst(&out[rung..], &sum[rung..]) < 1e-5);
        assert!(
            step(&out[ends - 100..ends + 100]) < 1.2 * step(&out[ends - 1_000..ends - 100]),
            "a step where the short one ends"
        );
        assert!(rms(&out[RATE as usize / 5..RATE as usize / 2]) > 0.005);
        assert_eq!(e.active_voices(), 0, "the hit ends with its last layer");
    }
}

#[test]
fn a_layers_level_moved_while_it_plays_moves_that_layer_alone() {
    // The layers share a filter, but a level edit of one, the first or
    // another, glides that one to its new level: once there, the hit plays
    // as a hit at the new levels does.
    let f = filter(FilterKind::LowPass, 2, 800.0, 0.4, 0.0);
    let (a, b) = (sine(220.0, 0.02), sine(1_500.0, 0.02));
    let p = layered(&[(&a, 0.0), (&b, 0.0)], f);
    let at = RATE as usize / 5;
    for moved in 0..2 {
        let mut q = p.clone();
        let mut pad = q.pad(0).unwrap().clone();
        pad.layers[moved].level_db = -12.0;
        q.set_pad(0, Some(pad), None).unwrap();
        let mut e = Engine::new(RATE, 8);
        e.set_view(EngineView::build(&p, RATE));
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 127,
        });
        let before = run(&mut e, at);
        drop(e.edit_view(EngineView::build(&q, RATE)));
        let after = run(&mut e, at);
        let want = hit(&q, 2 * at);
        // 0.1 s on, past the glide and the filter's memory of before.
        let from = RATE as usize / 10;
        let off = worst(&after[from..], &want[at + from..]);
        assert!(off < 1e-3 * rms(&want[at..]), "layer {moved}: {off}");
        // No step as it moves.
        let d = |x: &[f32]| {
            x.windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max)
        };
        let around = [&before[at - 64..], &after[..2_000]].concat();
        assert!(d(&around) < 1.2 * d(&before[at / 2..]), "layer {moved}");
    }
}

#[test]
fn an_edit_reaches_a_hit_whose_first_layer_has_ended() {
    // The first layer's sound ends; its voice filters the other layer's
    // on, and the pad's cutoff turned then reaches that filter.
    let f = filter(FilterKind::LowPass, 2, 800.0, 0.4, 0.0);
    let short = source(RATE as usize / 10, |i| {
        0.02 * (i as f32 * 220.0 * TAU / RATE as f32).sin()
    });
    let long = sine(1_500.0, 0.02);
    let p = layered(&[(&short, 0.0), (&long, 0.0)], f);
    let mut q = p.clone();
    let mut pad = q.pad(0).unwrap().clone();
    pad.filter.cutoff_hz = 3_000.0;
    q.set_pad(0, Some(pad), None).unwrap();
    let mut e = Engine::new(RATE, 8);
    e.set_view(EngineView::build(&p, RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let at = RATE as usize / 5;
    let _ = run(&mut e, at);
    drop(e.edit_view(EngineView::build(&q, RATE)));
    let after = run(&mut e, at);
    let want = hit(&layered(&[(&long, 0.0)], q.pad(0).unwrap().filter), 2 * at);
    let from = RATE as usize / 10;
    let off = worst(&after[from..], &want[at + from..]);
    assert!(off < 1e-3 * rms(&want[at..]), "{off}");
    // At 3 kHz the 1.5 kHz tone passes louder than at 800 Hz.
    assert!(rms(&after[from..]) > 2.0 * rms(&hit(&layered(&[(&long, 0.0)], f), 2 * at)[at..]));
}
