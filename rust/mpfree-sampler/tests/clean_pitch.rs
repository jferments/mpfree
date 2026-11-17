//! Clean pitching: a pad pitched up, or a file recorded at a higher rate,
//! plays without aliasing; a Vintage pad keeps the old read and its
//! aliasing; up to two semitones up (where what folds lands above
//! 19 kHz), at or below its pitch, a voice sounds exactly as before.

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;

const RATE: u32 = 44_100;

fn sine(rate: u32, seconds: f64, hz: f64) -> Source {
    let n = (seconds * f64::from(rate)) as usize;
    let v = (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / f64::from(rate)).sin()) as f32)
        .collect();
    Source::new("sine", rate, 1, v).unwrap()
}

/// A project with `s` on pad 0, tuned `cents`, played as a gate.
fn pad(s: &Source, cents: i32, vintage: bool) -> Project {
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    let mut pad = Pad::new("tone", Layer::new(r));
    pad.tune_cents = cents;
    pad.vintage = vintage;
    p.set_pad(0, Some(pad), None).unwrap();
    p
}

fn play(p: &Project, ev: Event, frames: usize, block: usize) -> Vec<f32> {
    let mut e = Engine::new(RATE, 16);
    e.set_view(EngineView::build(p, RATE));
    e.apply(ev);
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
        e.render(a, b);
    }
    l
}

fn hit() -> Event {
    Event::NoteOn {
        pad: 0,
        velocity: 127,
    }
}

/// The level (peak) of `hz` in `x`, by correlation over a stretch away
/// from the start, with a Hann window so neighbouring tones don't leak in.
fn level(x: &[f32], hz: f64) -> f64 {
    let x = &x[2000..2000 + 8192];
    let n = x.len() as f64;
    let (mut re, mut im, mut wsum) = (0.0, 0.0, 0.0);
    for (i, &v) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n).cos();
        let ph = std::f64::consts::TAU * hz * i as f64 / f64::from(RATE);
        re += w * f64::from(v) * ph.cos();
        im += w * f64::from(v) * ph.sin();
        wsum += w;
    }
    2.0 * (re * re + im * im).sqrt() / wsum
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

#[test]
fn a_tone_pitched_past_nyquist_is_gone_not_folded_back() {
    // 15 kHz an octave up is 30 kHz, above what 44.1 kHz carries: it
    // folds to 14.1 kHz unless it is filtered out.
    let s = sine(RATE, 1.0, 15_000.0);
    let clean = play(&pad(&s, 1200, false), hit(), 12_000, 128);
    let vintage = play(&pad(&s, 1200, true), hit(), 12_000, 128);
    let alias = 2.0 * f64::from(RATE) / 2.0 - 30_000.0;
    let (c, v) = (db(level(&clean, alias)), db(level(&vintage, alias)));
    eprintln!("15 kHz up an octave: alias {c:.1} dB clean, {v:.1} dB Vintage");
    assert!(v > -10.0, "Vintage keeps the old read's alias: {v:.1} dB");
    assert!(c < -50.0, "clean: the alias is {c:.1} dB");
}

#[test]
fn a_tone_pitched_up_inside_the_band_keeps_its_level() {
    // 5 kHz up a fifth is about 7.49 kHz, well inside the band.
    let s = sine(RATE, 1.0, 5_000.0);
    let at_pitch = play(&pad(&s, 0, false), hit(), 12_000, 128);
    let up = play(&pad(&s, 700, false), hit(), 12_000, 128);
    let hz = 5_000.0 * (7.0f64 / 12.0).exp2();
    let (a, b) = (db(level(&at_pitch, 5_000.0)), db(level(&up, hz)));
    assert!(
        (a - b).abs() < 0.3,
        "at pitch {a:.2} dB, up a fifth {b:.2} dB"
    );
    // And it is the only tone there.
    assert!(db(level(&up, 5_000.0)) < -60.0);
}

#[test]
fn a_file_at_a_higher_rate_plays_without_its_ultrasonic_content_folding() {
    // A 96 kHz file holding 30 kHz: at its own pitch on a 44.1 kHz output
    // the 30 kHz must vanish, not land at 14.1 kHz.
    let s = sine(96_000, 0.3, 30_000.0);
    let clean = play(&pad(&s, 0, false), hit(), 12_000, 128);
    let vintage = play(&pad(&s, 0, true), hit(), 12_000, 128);
    let alias = f64::from(RATE) - 30_000.0;
    let (c, v) = (db(level(&clean, alias)), db(level(&vintage, alias)));
    eprintln!("96 kHz file, 30 kHz tone: alias {c:.1} dB clean, {v:.1} dB Vintage");
    assert!(v > -20.0, "Vintage: {v:.1} dB");
    assert!(c < -50.0, "clean: {c:.1} dB");
}

#[test]
fn a_keygroup_two_octaves_up_reads_an_octave_copy_and_stays_clean() {
    // 8 kHz two octaves up is 32 kHz, which folds to 12.1 kHz. (The
    // sounds are long enough to last the measurement at four times speed.)
    let s = sine(RATE, 2.5, 8_000.0);
    // A sound this long gets its copies on their own thread; made here,
    // the test doesn't hang on its timing.
    s.make_octaves();
    let mut p = pad(&s, 0, false);
    p.spread_to_keys(0, 48).unwrap();
    let key = Event::Key {
        note: 72,
        velocity: 127,
    };
    let out = play(&p, key, 12_000, 128);
    let alias = f64::from(RATE) - 32_000.0;
    let c = db(level(&out, alias));
    eprintln!("8 kHz two octaves up: alias {c:.1} dB clean");
    assert!(c < -50.0, "two octaves up: the alias is {c:.1} dB");
    // A tone that stays in the band, two octaves up, keeps its level.
    let s = sine(RATE, 2.5, 2_000.0);
    s.make_octaves();
    let mut p = pad(&s, 0, false);
    p.spread_to_keys(0, 48).unwrap();
    let up = play(&p, key, 12_000, 128);
    let root = play(
        &p,
        Event::Key {
            note: 48,
            velocity: 127,
        },
        12_000,
        128,
    );
    let (a, b) = (db(level(&root, 2_000.0)), db(level(&up, 8_000.0)));
    assert!(
        (a - b).abs() < 0.3,
        "root {a:.2} dB, two octaves up {b:.2} dB"
    );
}

#[test]
fn up_to_two_semitones_up_a_voice_sounds_exactly_as_a_vintage_one() {
    let s = sine(RATE, 0.5, 3_000.0);
    for cents in [200, 150, 1, 0, -700, -1200] {
        let clean = play(&pad(&s, cents, false), hit(), 8_000, 128);
        let vintage = play(&pad(&s, cents, true), hit(), 8_000, 128);
        assert_eq!(clean, vintage, "tuned {cents}");
        assert!(clean.iter().any(|x| x.abs() > 0.1));
    }
    // Just past two semitones the read changes.
    let clean = play(&pad(&s, 201, false), hit(), 8_000, 128);
    let vintage = play(&pad(&s, 201, true), hit(), 8_000, 128);
    assert_ne!(clean, vintage);
}

#[test]
fn a_pitched_voice_sounds_the_same_whatever_the_block_size() {
    // Raised by the program's tune while it plays, from below its pitch to
    // above it: the switch to the clean read lands on the same frame.
    let s = sine(RATE, 0.5, 6_000.0);
    let p = pad(&s, -50, false);
    let run = |block: usize| {
        let mut e = Engine::new(RATE, 16);
        e.set_view(EngineView::build(&p, RATE));
        e.apply(hit());
        let frames = 9_000;
        let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
        let mut at = 0;
        while at < frames {
            let n = block.min(frames - at);
            if at <= 3_000 && 3_000 < at + n {
                e.schedule(
                    (3_000 - at) as u32,
                    Event::ProgramLive {
                        field: mpfree_sampler::engine::ProgramField::Tune,
                        value: 300.0,
                    },
                );
            }
            e.render(&mut l[at..at + n], &mut r[at..at + n]);
            at += n;
        }
        l
    };
    let want = run(128);
    assert!(want.iter().any(|x| x.abs() > 0.1));
    for block in [1, 7, 64, 1000] {
        let got = run(block);
        let first = got.iter().zip(&want).position(|(a, b)| a != b);
        assert_eq!(first, None, "block {block}");
    }
}

#[test]
fn a_voice_two_octaves_up_plays_whether_or_not_its_copies_are_made_yet() {
    // A long sound's copies are made on their own thread: a voice that
    // starts before they are there reads the sound itself (the sinc
    // stretched as far as it goes), and is just as loud and in tune.
    let s = sine(RATE, 2.5, 1_000.0);
    let mut p = pad(&s, 0, false);
    p.spread_to_keys(0, 48).unwrap();
    let mut e = Engine::new(RATE, 16);
    let view = EngineView::build(&p, RATE);
    e.set_view(view);
    e.apply(Event::Key {
        note: 72,
        velocity: 127,
    });
    let (mut l, mut r) = (vec![0.0; 12_000], vec![0.0; 12_000]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    assert!(db(level(&l, 4_000.0)) > -10.0);
}
