//! 16 Levels: every pad of a bank plays one pad, each at its own step of
//! velocity, tune, cutoff, decay or start.

use mpfree_analysis::yin;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn tone(seconds: f64, hz: &[f64]) -> Source {
    let n = (seconds * f64::from(RATE)) as usize;
    let part = n / hz.len();
    let v = (0..n)
        .map(|i| {
            let f = hz[(i / part).min(hz.len() - 1)];
            (0.5 * (std::f64::consts::TAU * f * i as f64 / f64::from(RATE)).sin()) as f32
        })
        .collect();
    Source::new("tone", RATE, 1, v).unwrap()
}

/// Pad 1 holds `s` (gate, so a release lets it go); the pads play 16
/// Levels of it by `by`.
fn levels(s: &Source, by: Levels) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    p.set_pad(0, Some(Pad::new("tone", Layer::new(r))), None)
        .unwrap();
    p.set_pads_play(PadsPlay::Levels { pad: 0, by }).unwrap();
    p
}

/// Pad `pad` hit at `velocity`: what it plays for `frames`.
fn hit(p: &Project, pad: u8, velocity: u8, frames: usize) -> Vec<f32> {
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(p, RATE));
    e.apply(Event::NoteOn { pad, velocity });
    render(&mut e, frames)
}

fn render(e: &mut Engine, frames: usize) -> Vec<f32> {
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    l
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn pitch(x: &[f32]) -> f32 {
    yin(&x[1000..9192], RATE, 4096, 40.0, 3000.0, 0.15)
        .expect("pitched")
        .0
}

#[test]
fn every_pad_plays_the_one_pad_and_the_steps_are_as_documented() {
    let s = tone(1.0, &[220.0]);
    let p = levels(&s, Levels::Velocity);
    let view = EngineView::build(&p, RATE);
    assert!((0..16).all(|i| view.has_pad(i)));
    assert_eq!(
        (0..16).map(Levels::velocity).collect::<Vec<_>>(),
        vec![8, 16, 24, 32, 40, 48, 56, 64, 72, 80, 88, 96, 104, 112, 120, 127]
    );
    assert_eq!(
        (
            Levels::semitones(0),
            Levels::semitones(8),
            Levels::semitones(15)
        ),
        (-8, 0, 7)
    );
    assert!((Levels::cutoff_hz(0) - 100.0).abs() < 0.01);
    assert!((Levels::cutoff_hz(15) - 20_000.0).abs() < 1.0);
    assert!((Levels::decay_ms(15) - 4525.5).abs() < 1.0);
    // Without a sound on the pad, no pad plays.
    let mut q = p.clone();
    q.set_pad(0, None, None).unwrap();
    let view = EngineView::build(&q, RATE);
    assert!(!(0..16).any(|i| view.has_pad(i)));
}

#[test]
fn velocity_levels_grow_pad_by_pad_whatever_the_hit_and_full_level() {
    let s = tone(1.0, &[220.0]);
    let p = levels(&s, Levels::Velocity);
    let loud: Vec<f32> = [0u8, 7, 15]
        .iter()
        .map(|&k| {
            let mut e = Engine::new(RATE, 16);
            e.set_view(EngineView::build(&p, RATE));
            // Full Level doesn't flatten the levels.
            e.set_fixed_velocity(Some(127));
            e.apply(Event::NoteOn {
                pad: k,
                velocity: 30,
            });
            rms(&render(&mut e, 8192)[2048..])
        })
        .collect();
    assert!(loud[0] < loud[1] && loud[1] < loud[2], "{loud:?}");
    // Velocity 8 against 127 at full velocity-to-level: (8/127)^2.
    let ratio = loud[0] / loud[2];
    assert!((ratio - (8.0f32 / 127.0).powi(2)).abs() < 0.002, "{ratio}");
}

#[test]
fn tune_levels_are_a_semitone_a_pad_with_pad_9_as_recorded() {
    let s = tone(1.0, &[220.0]);
    let p = levels(&s, Levels::Tune);
    for (pad, hz) in [(8u8, 220.0f32), (0, 138.59), (15, 329.63), (11, 261.63)] {
        let f = pitch(&hit(&p, pad, 127, 12_000));
        assert!((f - hz).abs() < hz * 0.003, "pad {}: {f}", pad + 1);
    }
}

#[test]
fn cutoff_levels_open_the_filter_pad_by_pad() {
    // 3 kHz: pad 1's 100 Hz low-pass all but silences it.
    let s = tone(1.0, &[3000.0]);
    let p = levels(&s, Levels::Cutoff);
    let at = |k: u8| rms(&hit(&p, k, 127, 8192)[2048..]);
    let (shut, half, open) = (at(0), at(9), at(15));
    assert!(shut < 0.01 * open, "{shut} {open}");
    assert!(shut < half && half < open, "{shut} {half} {open}");
    // A pad with its own filter keeps its type and resonance.
    let mut q = p.clone();
    q.edit_pad(0, None, |pad| {
        pad.filter = Filter {
            kind: FilterKind::HighPass,
            poles: 2,
            cutoff_hz: 200.0,
            resonance: 0.0,
            env_octaves: 0.0,
        }
    })
    .unwrap();
    let at = |k: u8| rms(&hit(&q, k, 127, 8192)[2048..]);
    assert!(
        at(15) < 0.1 * at(0),
        "a high-pass at 20 kHz shuts the tone out"
    );
}

#[test]
fn decay_levels_fall_to_silence_sooner_on_the_low_pads() {
    let s = tone(1.0, &[220.0]);
    let p = levels(&s, Levels::Decay);
    // 0.4 s in: pad 1 (25 ms) is long gone, pad 16 (4.5 s) still sounds.
    let late = |k: u8| rms(&hit(&p, k, 127, 20_480)[17_000..]);
    assert!(late(0) < 1e-4, "{}", late(0));
    assert!(late(15) > 0.05, "{}", late(15));
    assert!(late(6) < late(10));
}

#[test]
fn start_levels_start_further_in_on_each_pad() {
    // Four tones, a quarter each: pad 9 starts halfway, in the third.
    let s = tone(1.0, &[220.0, 277.18, 329.63, 440.0]);
    let p = levels(&s, Levels::Start);
    for (pad, hz) in [(0u8, 220.0f32), (4, 277.18), (8, 329.63), (12, 440.0)] {
        let f = pitch(&hit(&p, pad, 127, 10_000));
        assert!((f - hz).abs() < 1.0, "pad {}: {f}", pad + 1);
    }
}

#[test]
fn each_pad_lets_go_of_its_own_level() {
    let s = tone(1.0, &[220.0]);
    let p = levels(&s, Levels::Tune);
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(&p, RATE));
    for pad in [0u8, 4, 8] {
        e.apply(Event::NoteOn { pad, velocity: 100 });
    }
    render(&mut e, 256);
    assert_eq!(e.active_voices(), 3);
    e.apply(Event::NoteOff { pad: 4 });
    render(&mut e, 4096);
    assert_eq!(e.active_voices(), 2, "only pad 5 lets go");
}

#[test]
fn levels_are_one_undo_step_and_a_pad_out_of_range_is_refused() {
    let s = tone(0.1, &[220.0]);
    let mut p = levels(&s, Levels::Decay);
    assert_eq!(
        p.pads_play(),
        PadsPlay::Levels {
            pad: 0,
            by: Levels::Decay
        }
    );
    assert!(p
        .set_pads_play(PadsPlay::Levels {
            pad: 128,
            by: Levels::Tune
        })
        .is_err());
    assert!(p.undo());
    assert_eq!(p.pads_play(), PadsPlay::Pads);
}
