//! Pitch correct and the Harmonizer do what they say: the tracker hears the
//! note sung, Pitch correct moves it onto the key's notes, the Harmonizer
//! adds the key's intervals, the vowel (the formant) stays where it was,
//! and with nothing to correct the voice comes back sample for sample, one
//! latency late.

use std::f32::consts::PI;

use mpfree_fx::ids::{harmonizer as harm, pitch_correct as tune};
use mpfree_fx::mpfree_node::{Ctx, Node};
use mpfree_fx::*;

const RATE: u32 = 48_000;

fn set_up<N: Node>(mut n: N, values: &[(u16, f32)]) -> N {
    n.prepare(RATE, 512);
    for &(id, v) in values {
        n.set_param(id, v);
    }
    n.reset();
    n
}

/// Runs `n` over a mono input in 256-frame blocks; left and right out.
fn run(n: &mut dyn Node, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut at = 0;
    while at < x.len() {
        let len = 256.min(x.len() - at);
        n.process(
            &Ctx::new(RATE, at as u64),
            &[],
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

/// A sung vowel, roughly: a sawtooth at `hz(t)` through a resonance at
/// 1 kHz (the formant), peaking at -12 dBFS.
fn vowel(hz: impl Fn(f32) -> f32, frames: usize) -> Vec<f32> {
    voiced(hz, frames, false)
}

/// A buzzier vowel: a pulse at each period (every harmonic as loud) through
/// the same formant, so the formant is where the loudest harmonic is.
fn buzz(hz: f32, frames: usize) -> Vec<f32> {
    voiced(|_| hz, frames, true)
}

fn voiced(hz: impl Fn(f32) -> f32, frames: usize, pulses: bool) -> Vec<f32> {
    let (mut phase, mut y1, mut y2) = (0.0f32, 0.0f32, 0.0f32);
    // A two-pole resonator at 1 kHz, bandwidth about 150 Hz.
    let r = (-PI * 150.0 / RATE as f32).exp();
    let (a1, a2) = (2.0 * r * (2.0 * PI * 1000.0 / RATE as f32).cos(), -r * r);
    let g = 1.0 - r;
    let y: Vec<f32> = (0..frames)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let step = hz(t) / RATE as f32;
            let x = if pulses {
                let x = if phase + step >= 1.0 { 1.0 } else { 0.0 };
                x - step
            } else {
                2.0 * phase - 1.0
            };
            phase = (phase + step).fract();
            let y = g * x + a1 * y1 + a2 * y2;
            y2 = y1;
            y1 = y;
            y
        })
        .collect();
    let top = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    y.iter().map(|v| v * 0.25 / top).collect()
}

/// The pitch of `x` in Hz, from its difference function (the first dip
/// under 0.1 of the cumulative mean, refined by a parabola), looking for
/// 60 to 1000 Hz.
fn pitch_hz(x: &[f32]) -> f32 {
    let (lo, hi) = (RATE as usize / 1000, RATE as usize / 60);
    let win = x.len() - hi - 1;
    let d: Vec<f32> = (0..=hi)
        .map(|tau| {
            (0..win)
                .map(|j| {
                    let e = x[j] - x[j + tau];
                    e * e
                })
                .sum()
        })
        .collect();
    let mut sum = 0.0;
    let mut norm = vec![1.0f32; hi + 1];
    for tau in 1..=hi {
        sum += d[tau];
        norm[tau] = d[tau] * tau as f32 / sum.max(1e-12);
    }
    let mut tau = lo;
    while tau < hi - 1 && norm[tau] >= 0.1 {
        tau += 1;
    }
    while tau < hi - 1 && norm[tau + 1] < norm[tau] {
        tau += 1;
    }
    let (a, b, c) = (norm[tau - 1], norm[tau], norm[tau + 1]);
    let shift = 0.5 * (a - c) / (a - 2.0 * b + c);
    RATE as f32 / (tau as f32 + shift)
}

/// The pitch of the 100 ms of `x` from `at_ms`.
fn pitch_at(x: &[f32], at_ms: usize) -> f32 {
    let ms = RATE as usize / 1000;
    pitch_hz(&x[at_ms * ms..(at_ms + 100) * ms])
}

#[test]
fn the_tracker_hears_the_note_and_the_correction_meter_shows_the_pull() {
    for (hz, range) in [(110.0, 0.0), (220.0, 1.0), (261.63, 1.0), (440.0, 2.0)] {
        let mut p = set_up(PitchCorrect::new(), &[(tune::RANGE, range)]);
        run(&mut p, &vowel(|_| hz, RATE as usize / 2));
        let note = p.param(tune::NOTE);
        assert_eq!(note, hz_to_midi(hz).round(), "{hz} Hz is note {note}");
        assert_eq!(VOCAL_RANGES.len(), 3);
    }
    // 224 Hz is 31 cents over A3: Chromatic pulls it down by that much.
    let mut p = set_up(PitchCorrect::new(), &[(tune::SPEED, 0.0)]);
    run(&mut p, &vowel(|_| 224.0, RATE as usize / 2));
    let want = -cents(224.0, 220.0);
    let got = p.param(tune::CORRECTION);
    assert!(
        (got - want).abs() < 3.0,
        "correction {got} cents, want {want}"
    );
    // Silence hears nothing.
    let mut p = set_up(PitchCorrect::new(), &[]);
    run(&mut p, &vec![0.0; RATE as usize / 4]);
    assert_eq!(p.param(tune::NOTE), 0.0);
}

#[test]
fn pitch_correct_moves_the_voice_onto_the_key() {
    // 224 Hz, Chromatic: to A3, 220 Hz.
    let mut p = set_up(PitchCorrect::new(), &[(tune::SPEED, 0.0)]);
    let (l, r) = run(&mut p, &vowel(|_| 224.0, RATE as usize));
    assert_eq!(l, r, "a mono voice stays mono");
    let got = pitch_at(&l, 600);
    assert!(cents(got, 220.0).abs() < 5.0, "224 Hz came out at {got}");

    // 270 Hz (C4 and a half, between C and C#) in C major: to C4.
    let mut p = set_up(
        PitchCorrect::new(),
        &[(tune::SCALE, 1.0), (tune::SPEED, 0.0)],
    );
    let (l, _) = run(&mut p, &vowel(|_| 270.0, RATE as usize));
    let got = pitch_at(&l, 600);
    assert!(cents(got, 261.63).abs() < 5.0, "270 Hz came out at {got}");

    // Amount 50: half way.
    let mut p = set_up(
        PitchCorrect::new(),
        &[(tune::SPEED, 0.0), (tune::AMOUNT, 50.0)],
    );
    let (l, _) = run(&mut p, &vowel(|_| 224.0, RATE as usize));
    let got = pitch_at(&l, 600);
    let half = 224.0 * 2f32.powf(-cents(224.0, 220.0) / 2400.0);
    assert!(cents(got, half).abs() < 5.0, "half way came out at {got}");

    // A glide from 200 to 260 Hz snaps from note to note (the hard, stepped
    // sound): nearly every 40 ms of it is on a semitone, where the glide
    // itself is on one a fifth of the time.
    let on_notes = |x: &[f32]| {
        let ms = RATE as usize / 1000;
        let looks: Vec<f32> = (5..45)
            .map(|k| pitch_hz(&x[k * 20 * ms..(k * 20 + 40) * ms]))
            .collect();
        let on = looks
            .iter()
            .filter(|&&hz| (hz_to_midi(hz) - hz_to_midi(hz).round()).abs() < 0.1)
            .count();
        on as f32 / looks.len() as f32
    };
    let glide = vowel(|t| 200.0 + 60.0 * t, RATE as usize);
    let mut p = set_up(PitchCorrect::new(), &[(tune::SPEED, 0.0)]);
    let (l, _) = run(&mut p, &glide);
    assert!(on_notes(&glide) < 0.4, "the glide: {}", on_notes(&glide));
    assert!(on_notes(&l) > 0.8, "corrected: {}", on_notes(&l));

    // Speed: a slow pull is still on its way after 30 ms and there by 600.
    let mut p = set_up(PitchCorrect::new(), &[(tune::SPEED, 200.0)]);
    let x = vowel(|_| 224.0, RATE as usize);
    let lat = p.latency() as usize;
    let (l, _) = run(&mut p, &x);
    let ms = RATE as usize / 1000;
    let early = pitch_hz(&l[lat + 60 * ms..lat + 160 * ms]);
    assert!(cents(early, 220.0) > 8.0, "still on its way: {early}");
    let late = pitch_at(&l, 800);
    assert!(cents(late, 220.0).abs() < 5.0, "there in the end: {late}");
}

#[test]
fn with_nothing_to_correct_the_voice_comes_back_one_latency_late() {
    // A voice, a breath (noise) and a glide, with Amount at zero: the
    // grains are laid as ever, at ratio 1, and must give the input back.
    let frames = RATE as usize;
    let mut seed = 1u32;
    let x: Vec<f32> = vowel(|t| 180.0 + 120.0 * t, frames)
        .iter()
        .enumerate()
        .map(|(i, v)| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5) * 0.2;
            if (20_000..28_000).contains(&i) {
                noise
            } else {
                *v
            }
        })
        .collect();
    for range in [0.0, 1.0, 2.0] {
        let mut p = set_up(
            PitchCorrect::new(),
            &[(tune::AMOUNT, 0.0), (tune::RANGE, range)],
        );
        let lat = p.latency() as usize;
        let (l, _) = run(&mut p, &x);
        let worst = (lat..frames)
            .map(|i| (l[i] - x[i - lat]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-5, "range {range}: off by {worst}");
        assert!(l[..lat].iter().all(|&v| v == 0.0), "silent until then");
    }
    // The latency is two and a half of the range's longest periods.
    let p = set_up(PitchCorrect::new(), &[]);
    assert_eq!(p.latency(), 1336);
}

#[test]
fn the_harmonizer_adds_the_keys_intervals() {
    // C major, a 3rd up: from A3 it is C4 (a minor 3rd), from G3 it is B3
    // (a major 3rd). Voice 2 off, no dry, voice 1 hard left.
    for (sung, want) in [(220.0, 261.63), (196.0, 246.94)] {
        let mut h = set_up(
            Harmonizer::new(),
            &[
                (harm::VOICE1, 7.0),
                (harm::LEVEL1, 100.0),
                (harm::PAN1, -100.0),
                (harm::VOICE2, 0.0),
                (harm::DRY, 0.0),
            ],
        );
        let (l, r) = run(&mut h, &vowel(|_| sung, RATE as usize));
        let got = pitch_at(&l, 600);
        assert!(cents(got, want).abs() < 5.0, "{sung} Hz: harmony at {got}");
        assert!(rms(&r[24_000..]) < 1e-6, "hard left");
        assert_eq!(h.param(harm::NOTE), hz_to_midi(sung).round());
    }
    // Dry alone is the voice, one latency late.
    let x = vowel(|_| 220.0, RATE as usize / 2);
    let mut h = set_up(
        Harmonizer::new(),
        &[(harm::LEVEL1, 0.0), (harm::LEVEL2, 0.0)],
    );
    let lat = h.latency() as usize;
    let (l, r) = run(&mut h, &x);
    assert_eq!(l, r);
    for i in lat..x.len() {
        assert!((l[i] - x[i - lat]).abs() < 1e-6, "dry at {i}");
    }
}

#[test]
fn the_harmony_goes_quiet_on_a_breath() {
    let frames = RATE as usize / 2;
    let mut seed = 5u32;
    let breath: Vec<f32> = (0..frames)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5) * 0.3
        })
        .collect();
    let mut h = set_up(Harmonizer::new(), &[(harm::DRY, 0.0)]);
    let (l, r) = run(&mut h, &breath);
    let tail = 12_000..frames;
    assert!(
        rms(&l[tail.clone()]) < 0.01 * rms(&breath) && rms(&r[tail]) < 0.01 * rms(&breath),
        "no harmony on noise"
    );
    assert_eq!(h.param(harm::NOTE), 0.0);
}

#[test]
fn the_vowel_stays_where_it_was() {
    // 150 Hz through a 1 kHz formant, a 5th up (Chromatic): the pitch goes
    // to 225 Hz and the loudest harmonic is still the one nearest 1 kHz
    // (900 or 1125 Hz), not 1.5 kHz as a tape-speed shift would put it.
    let mut h = set_up(
        Harmonizer::new(),
        &[
            (harm::SCALE, 0.0),
            (harm::VOICE1, 9.0),
            (harm::LEVEL1, 100.0),
            (harm::PAN1, 0.0),
            (harm::VOICE2, 0.0),
            (harm::DRY, 0.0),
        ],
    );
    let x = buzz(150.0, RATE as usize);
    let (l, _) = run(&mut h, &x);
    let f0 = 150.0 * 2f32.powf(7.0 / 12.0);
    let got = pitch_at(&l, 600);
    assert!(cents(got, f0).abs() < 5.0, "a 5th up: {got}");
    // The harmonic of `f0` loudest in `x` from 0.5 s to 1 s.
    let loudest = |x: &[f32], f0: f32| {
        let seg = &x[24_000..48_000];
        let level = |hz: f32| {
            let (mut re, mut im) = (0.0f32, 0.0f32);
            for (i, v) in seg.iter().enumerate() {
                let a = 2.0 * PI * hz * i as f32 / RATE as f32;
                re += v * a.cos();
                im += v * a.sin();
            }
            re * re + im * im
        };
        (1..16)
            .map(|k| k as f32 * f0)
            .max_by(|a, b| level(*a).total_cmp(&level(*b)))
            .unwrap()
    };
    assert_eq!(loudest(&x, 150.0), 1050.0, "the input's formant");
    let loudest = loudest(&l, got);
    assert!(
        (800.0..1250.0).contains(&loudest),
        "the formant moved to {loudest} Hz"
    );
}
