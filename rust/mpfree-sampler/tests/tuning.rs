//! Kits in a tuning (the tunings plan, step T2): keygroup notes play the
//! tuning's pitch for each key with the sound's root at its 12-TET pitch;
//! pad hits and slices don't move; the wheel moves along the tuning's keys;
//! held notes follow a change; 16 Levels Tune steps by the tuning's keys;
//! in 12-TET nothing changes to the bit.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use mpfree_tuning::{Builtin, Tuning};

const RATE: u32 = 44_100;
/// The keygroup's root: A3, where its sine (220 Hz) plays as recorded.
const ROOT: u8 = 57;

fn sine(seconds: f64, hz: f64) -> Source {
    let n = (seconds * f64::from(RATE)) as usize;
    let v = (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(RATE)).sin()) as f32)
        .collect();
    Source::new("sine", RATE, 1, v).unwrap()
}

/// A gate keygroup over the whole keyboard, A3 at its root.
fn keygroup(mono: Mono) -> Project {
    let s = sine(4.0, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Gate;
    let mut pad = Pad::new("keys", Layer::new(r));
    pad.mono = mono;
    p.set_pad(0, Some(pad), None).unwrap();
    p.spread_to_keys(0, ROOT).unwrap();
    p
}

fn engine(p: &Project) -> Engine {
    let mut e = Engine::new(RATE, 16);
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

/// The frequency of a steady tone from its rising zero crossings, placed
/// between samples: a hundredth of a cent on a clean sine.
fn hz(x: &[f32]) -> f64 {
    let ups: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] < 0.0 && w[1] >= 0.0)
        .map(|(i, w)| i as f64 + f64::from(w[0] / (w[0] - w[1])))
        .collect();
    assert!(ups.len() > 10, "no steady tone");
    let (a, b) = (ups[0], ups[ups.len() - 1]);
    (ups.len() - 1) as f64 / (b - a) * f64::from(RATE)
}

fn cents(a: f64, b: f64) -> f64 {
    1200.0 * (b / a).log2()
}

fn key(e: &mut Engine, note: u8) {
    e.apply(Event::Key {
        note,
        velocity: 100,
    });
}

/// What key `note` plays, after the attack, in tuning `t` (None: never set).
fn played(p: &Project, t: Option<&Tuning>, note: u8) -> Vec<f32> {
    let mut e = engine(p);
    if let Some(t) = t {
        e.set_tuning(0, t);
    }
    key(&mut e, note);
    let out = render(&mut e, 30_000);
    out[4_000..].to_vec()
}

fn twelve_tet(note: u8) -> f64 {
    440.0 * 2f64.powf((f64::from(note) - 69.0) / 12.0)
}

#[test]
fn a_keygroup_plays_the_tunings_pitch_with_its_root_at_its_12_tet_pitch() {
    let p = keygroup(Mono::default());
    let just = Builtin::Just.tuning(0, 440.0);
    let c = hz(&played(&p, Some(&just), 60));
    let e = hz(&played(&p, Some(&just), 64));
    let g = hz(&played(&p, Some(&just), 67));
    // C keeps its 12-TET pitch; E and G are pure above it.
    assert!(cents(twelve_tet(60), c).abs() < 0.5, "C {c}");
    assert!((cents(c, e) - 386.314).abs() < 0.5, "E {}", cents(c, e));
    assert!((cents(c, g) - 701.955).abs() < 0.5, "G {}", cents(c, g));
    // Each key sounds where the tuning puts it.
    for note in [45u8, 57, 61, 70, 81] {
        let got = hz(&played(&p, Some(&just), note));
        let want = f64::from(just.hz(f32::from(note)));
        assert!(cents(want, got).abs() < 0.5, "key {note}: {got} vs {want}");
    }
    // A4 = 432 moves the keygroup 31.8 cents, its root too.
    let a432 = Tuning::equal(12, 432.0);
    for note in [ROOT, 69, 76] {
        let got = hz(&played(&p, Some(&a432), note));
        let off = cents(twelve_tet(note), got);
        assert!((off + 31.767).abs() < 0.5, "key {note}: {off}");
    }
    // Quarter tones: each key a quarter tone, A4 on key 69.
    let q = Tuning::equal(24, 440.0);
    let up = hz(&played(&p, Some(&q), 70));
    assert!((cents(440.0, up) - 50.0).abs() < 0.5, "{up}");
}

#[test]
fn in_12_tet_a_tuning_set_and_put_back_changes_nothing() {
    // Keys at or below the root compare bit for bit: a voice read
    // band-limited (over two semitones up) reads its octave copies once
    // they are made, which can differ from run to run.
    let p = keygroup(Mono::default());
    let never = played(&p, None, 52);
    assert_eq!(played(&p, Some(&Tuning::standard()), 52), never);
    let mut e = engine(&p);
    e.set_tuning(0, &Builtin::Just.tuning(2, 432.0));
    e.set_tuning(0, &Tuning::standard());
    assert!(e.kit_tuning(0).unwrap().is_standard());
    assert!(e.kit_tuning(1).is_none());
    key(&mut e, 52);
    let out = render(&mut e, 30_000);
    assert_eq!(out[4_000..], never[..]);
    // A tuning on another kit leaves this one alone.
    let mut e = Engine::with_kits(RATE, 16, 2);
    e.set_kit_view(0, EngineView::build(&p, RATE));
    e.set_tuning(1, &Builtin::Just.tuning(0, 440.0));
    key(&mut e, 52);
    let out = render(&mut e, 30_000);
    assert_eq!(out[4_000..], never[..]);
}

#[test]
fn pad_hits_and_slices_play_as_tuned_on_the_pad() {
    let just = Builtin::Just.tuning(0, 432.0);
    // A pad hit: no keys.
    let s = sine(2.0, 220.0);
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(&s);
    r.mode = PlayMode::Gate;
    p.set_pad(0, Some(Pad::new("hit", Layer::new(r))), None)
        .unwrap();
    let hit = |t: Option<&Tuning>| {
        let mut e = engine(&p);
        if let Some(t) = t {
            e.set_tuning(0, t);
        }
        e.apply(Event::NoteOn {
            pad: 0,
            velocity: 100,
        });
        render(&mut e, 20_000)
    };
    assert_eq!(hit(Some(&just)), hit(None));
    // Slices on keys: each key picks a slice at the sound's own pitch.
    let n = 4 * 11_025;
    let data: Vec<f32> = (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * 330.0 * i as f64 / f64::from(RATE)).sin()) as f32)
        .collect();
    let s = Source::new("steps", RATE, 1, data).unwrap();
    let mut p = Project::new();
    p.add_source(s.clone());
    let chop = ChopSet::grid("steps", &s, 4).unwrap();
    p.chop_to_keys(chop, 0, 4, 0, SLICE_KEYS_FROM, None)
        .unwrap();
    for k in 0..4u8 {
        let slice = |t: Option<&Tuning>| {
            let mut e = engine(&p);
            if let Some(t) = t {
                e.set_tuning(0, t);
            }
            key(&mut e, SLICE_KEYS_FROM + k);
            render(&mut e, 12_000)
        };
        assert_eq!(slice(Some(&just)), slice(None), "slice {k}");
    }
}

#[test]
fn a_held_note_follows_a_change_of_tuning() {
    let p = keygroup(Mono::default());
    let mut e = engine(&p);
    key(&mut e, 64);
    let before = render(&mut e, 20_000);
    assert!(cents(twelve_tet(64), hz(&before[4_000..])).abs() < 0.5);
    // Just on C: E moves 13.7 cents down, gliding there as for Tune.
    e.set_tuning(0, &Builtin::Just.tuning(0, 440.0));
    let tuned = render(&mut e, 30_000);
    let off = cents(twelve_tet(64), hz(&tuned[10_000..]));
    assert!((off + 13.686).abs() < 0.5, "{off}");
    // And back.
    e.set_tuning(0, &Tuning::standard());
    let back = render(&mut e, 30_000);
    let off = cents(twelve_tet(64), hz(&back[10_000..]));
    assert!(off.abs() < 0.5, "{off}");
}

#[test]
fn the_wheel_moves_along_the_tunings_keys() {
    let p = keygroup(Mono::default());
    // In quarter tones, two keys up is a semitone.
    let q = Tuning::equal(24, 440.0);
    let mut e = engine(&p);
    e.set_tuning(0, &q);
    e.apply(Event::Bend { cents: 200.0 });
    key(&mut e, 69);
    let out = render(&mut e, 30_000);
    let up = cents(440.0, hz(&out[4_000..]));
    assert!((up - 100.0).abs() < 0.5, "{up}");
    // In just on C, a held D bent two keys lands on the pure E.
    let just = Builtin::Just.tuning(0, 440.0);
    let mut e = engine(&p);
    e.set_tuning(0, &just);
    key(&mut e, 62);
    render(&mut e, 4_000);
    e.apply(Event::Bend { cents: 200.0 });
    let out = render(&mut e, 30_000);
    let got = hz(&out[10_000..]);
    let want = f64::from(just.hz(64.0));
    assert!(cents(want, got).abs() < 0.5, "{got} vs {want}");
    // Back to the middle: the tuned D again.
    e.apply(Event::Bend { cents: 0.0 });
    let out = render(&mut e, 30_000);
    let got = hz(&out[10_000..]);
    let want = f64::from(just.hz(62.0));
    assert!(cents(want, got).abs() < 0.5, "{got} vs {want}");
}

#[test]
fn sixteen_levels_of_tune_step_by_the_tunings_keys_from_the_pads_root() {
    let mut p = keygroup(Mono::default());
    p.set_pads_play(PadsPlay::Levels {
        pad: 0,
        by: Levels::Tune,
    })
    .unwrap();
    // Just on A, the pad's root (A3): pad 13 is a pure third above it,
    // pad 9 plays as recorded.
    let just = Builtin::Just.tuning(9, 440.0);
    let level = |t: Option<&Tuning>, pad: u8| {
        let mut e = engine(&p);
        if let Some(t) = t {
            e.set_tuning(0, t);
        }
        e.apply(Event::NoteOn { pad, velocity: 100 });
        let out = render(&mut e, 30_000);
        out[4_000..].to_vec()
    };
    let third = hz(&level(Some(&just), 12));
    assert!((cents(220.0, third) - 386.314).abs() < 0.5, "{third}");
    let plain = hz(&level(None, 12));
    assert!((cents(220.0, plain) - 400.0).abs() < 0.5, "{plain}");
    assert_eq!(level(Some(&just), 8), level(None, 8));
    // A4 = 432 moves nothing: the steps count from the pad as recorded
    // (a step a semitone up, so it compares bit for bit).
    assert_eq!(level(Some(&Tuning::equal(12, 432.0)), 9), level(None, 9));
}

#[test]
fn legato_and_glide_land_on_the_tunings_pitch() {
    let just = Builtin::Just.tuning(0, 440.0);
    // Legato: the sounding note moves from E to G without starting again.
    let p = keygroup(Mono {
        on: true,
        legato: true,
        ..Mono::default()
    });
    let mut e = engine(&p);
    e.set_tuning(0, &just);
    key(&mut e, 64);
    render(&mut e, 8_000);
    key(&mut e, 67);
    let out = render(&mut e, 30_000);
    let got = hz(&out[4_000..]);
    let want = f64::from(just.hz(67.0));
    assert!(cents(want, got).abs() < 0.5, "legato {got} vs {want}");
    // Glide on every note: E then G alone, landing on the pure G.
    let p = keygroup(Mono {
        on: true,
        glide_ms: 50.0,
        ..Mono::default()
    });
    let mut e = engine(&p);
    e.set_tuning(0, &just);
    key(&mut e, 64);
    render(&mut e, 8_000);
    e.apply(Event::KeyOff { note: 64 });
    key(&mut e, 67);
    let out = render(&mut e, 40_000);
    let got = hz(&out[20_000..]);
    assert!(cents(want, got).abs() < 0.5, "glide {got} vs {want}");
}

#[test]
fn the_pads_playing_notes_and_a_sample_on_every_key_follow_too() {
    let just = Builtin::Just.tuning(0, 440.0);
    // The pads play notes from C3 (48): pad 13 is E3.
    let p = keygroup(Mono::default());
    let mut e = engine(&p);
    e.set_tuning(0, &just);
    e.apply(Event::NoteOn {
        pad: 16,
        velocity: 100,
    });
    let out = render(&mut e, 30_000);
    let got = hz(&out[4_000..]);
    let want = f64::from(just.hz(64.0));
    assert!(cents(want, got).abs() < 0.5, "{got} vs {want}");
    // A sound on each key (one-note keygroups, as Auto-sample makes at
    // every semitone) follows, unlike slices on keys.
    let mut p = Project::new();
    let tones: Vec<Source> = [261.626, 277.183, 293.665]
        .iter()
        .map(|&hz| sine(2.0, hz))
        .collect();
    for (i, s) in tones.iter().enumerate() {
        p.add_source(s.clone());
        let mut r = Region::whole(s);
        r.mode = PlayMode::Gate;
        p.set_pad(i, Some(Pad::new("note", Layer::new(r))), None)
            .unwrap();
    }
    let keys = p.map_to_keys(&[(0, 60), (1, 61), (2, 62)]).unwrap();
    assert_eq!(keys[1], Keys::one(61));
    let got = hz(&played(&p, Some(&just), 61));
    let want = f64::from(just.hz(61.0));
    assert!(cents(want, got).abs() < 0.5, "{got} vs {want}");
}

#[test]
fn split_the_lanes_voices_follow_a_change_of_tuning_too() {
    let p = keygroup(Mono::default());
    let just = Builtin::Just.tuning(2, 432.0);
    let play = |split: bool| {
        let mut e = Engine::new(RATE, 16);
        e.set_view(EngineView::build(&p, RATE));
        if split {
            e.split(mpfree_sampler::engine::LaneConfig {
                name: "tuning-lane".into(),
                threaded: false,
                ..Default::default()
            })
            .unwrap();
        }
        // Below the root, so no voice reads band-limited: whether its
        // octave copies are ready yet would differ from run to run.
        for note in [45, 48, 52, 55] {
            key(&mut e, note);
        }
        let mut out = render(&mut e, 6_000);
        e.set_tuning(0, &just);
        out.extend(render(&mut e, 12_000));
        e.apply(Event::Bend { cents: 100.0 });
        out.extend(render(&mut e, 6_000));
        out
    };
    let (one, two) = (play(false), play(true));
    assert!(one.iter().any(|x| x.abs() > 0.1), "it plays");
    let worst = one
        .iter()
        .zip(&two)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 1e-5, "split differs from unsplit by {worst}");
}
