//! Root pitch on tones whose pitch is known exactly.

use mpfree_analysis::{note_name, root_pitch, yin, AnalysisError, PitchSettings};
use std::f32::consts::TAU;

/// A sum of harmonics: `amps[k]` is the level of harmonic k+1.
fn tone(hz: f32, amps: &[f32], rate: u32, secs: f32) -> Vec<f32> {
    let n = (secs * rate as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            amps.iter()
                .enumerate()
                .map(|(k, a)| a * (TAU * hz * (k + 1) as f32 * t).sin())
                .sum::<f32>()
                * 0.3
        })
        .collect()
}

fn noise(n: usize, seed: u32, amp: f32) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp
        })
        .collect()
}

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

#[test]
fn yin_finds_pure_tones_to_within_two_cents() {
    for rate in [44_100, 48_000] {
        for hz in [41.2, 55.0, 110.0, 220.0, 261.63, 440.0, 1000.0, 1760.0] {
            let x = tone(hz, &[1.0], rate, 0.2);
            let (got, ap) = yin(&x, rate, (rate / 10) as usize, 40.0, 2000.0, 0.15).unwrap();
            assert!(cents(got, hz).abs() < 2.0, "{hz} Hz at {rate}: {got}");
            assert!(ap < 0.05, "{hz}: aperiodicity {ap}");
        }
    }
}

#[test]
fn rich_and_weak_fundamental_tones_do_not_slip_an_octave() {
    let rate = 44_100;
    // Saw-like: every harmonic at 1/k.
    let saw: Vec<f32> = (1..=20).map(|k| 1.0 / k as f32).collect();
    // The fundamental quiet under strong 2nd and 3rd harmonics (a bass through a small speaker).
    let weak = [0.2, 1.0, 0.8, 0.3];
    for hz in [55.0, 98.0, 196.0, 330.0] {
        for amps in [&saw[..], &weak[..]] {
            let x = tone(hz, amps, rate, 0.3);
            let p = root_pitch(&x, 1, rate, PitchSettings::default())
                .unwrap()
                .unwrap();
            assert!(
                cents(p.hz, hz).abs() < 5.0,
                "{hz} Hz with {} harmonics: {} Hz",
                amps.len(),
                p.hz
            );
        }
    }
}

#[test]
fn notes_cents_and_names() {
    let rate = 48_000;
    // A3 tuned 30 cents sharp.
    let hz = 220.0 * (30.0f32 / 1200.0).exp2();
    let x = tone(hz, &[1.0, 0.5, 0.25], rate, 0.5);
    let p = root_pitch(&x, 1, rate, PitchSettings::default())
        .unwrap()
        .unwrap();
    assert_eq!(p.note, 57);
    assert_eq!(note_name(p.note), "A3");
    assert!((p.cents - 30.0).abs() < 2.0, "{}", p.cents);
    assert!(p.confidence > 0.9, "{}", p.confidence);
    assert!((p.semitones_to(57) + 0.3).abs() < 0.02);
    assert_eq!(note_name(60), "C4");
    assert_eq!(note_name(61), "C#4");
    assert_eq!(note_name(69), "A4");
    assert_eq!(note_name(0), "C-1");
    assert_eq!(note_name(127), "G9");
}

#[test]
fn a_plucked_note_with_a_noisy_attack() {
    // A noise burst, then a decaying string-like tone at D3 (146.83 Hz), in stereo.
    let rate = 44_100;
    let hz = 146.83;
    let mut mono = noise(2_000, 7, 0.8);
    let body = tone(hz, &[1.0, 0.6, 0.4, 0.3, 0.2, 0.1], rate, 1.5);
    mono.extend(
        body.iter()
            .enumerate()
            .map(|(i, v)| v * (-(i as f32) / rate as f32 * 2.5).exp()),
    );
    let stereo: Vec<f32> = mono.iter().flat_map(|&v| [v, v * 0.9]).collect();
    let p = root_pitch(&stereo, 2, rate, PitchSettings::default())
        .unwrap()
        .unwrap();
    assert_eq!(note_name(p.note), "D3");
    assert!(p.cents.abs() < 3.0, "{}", p.cents);
    assert!(p.frame >= 2_000, "read from the attack at {}", p.frame);
}

#[test]
fn an_808_glide_settles_on_its_note() {
    // A sine that falls from 70 Hz to G1 (49 Hz) over 40 ms, then holds and decays.
    let rate = 44_100;
    let n = rate as usize * 2;
    let mut phase = 0.0f32;
    let x: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let f = 49.0 + 21.0 * (-t / 0.015).exp();
            phase += TAU * f / rate as f32;
            phase.sin() * 0.8 * (-t * 1.2).exp()
        })
        .collect();
    let p = root_pitch(&x, 1, rate, PitchSettings::default())
        .unwrap()
        .unwrap();
    assert_eq!(note_name(p.note), "G1");
    assert!(p.cents.abs() < 5.0, "{}", p.cents);
}

#[test]
fn the_longest_note_wins_and_silence_is_skipped() {
    let rate = 44_100;
    let mut x = vec![0.0f32; rate as usize / 2];
    x.extend(tone(220.0, &[1.0, 0.3], rate, 1.4)); // A3, longer
    x.extend(tone(329.63, &[1.0, 0.3], rate, 0.6)); // E4, shorter
    let p = root_pitch(&x, 1, rate, PitchSettings::default())
        .unwrap()
        .unwrap();
    assert_eq!(note_name(p.note), "A3");
    assert!(p.frame >= rate as u64 / 2);
    assert!(
        p.confidence > 0.5 && p.confidence < 0.9,
        "two notes share the weight: {}",
        p.confidence
    );
}

#[test]
fn noise_silence_and_short_input_have_no_pitch() {
    let rate = 44_100;
    let s = PitchSettings::default();
    assert_eq!(
        root_pitch(&noise(rate as usize, 3, 0.5), 1, rate, s).unwrap(),
        None
    );
    assert_eq!(
        root_pitch(&vec![0.0; rate as usize], 1, rate, s).unwrap(),
        None
    );
    assert_eq!(
        root_pitch(&tone(440.0, &[1.0], rate, 0.01), 1, rate, s).unwrap(),
        None
    );
    assert_eq!(
        root_pitch(&[0.0; 4], 3, rate, s),
        Err(AnalysisError::BadInput("channels"))
    );
    let bad = PitchSettings {
        min_hz: 500.0,
        max_hz: 100.0,
        ..s
    };
    assert_eq!(
        root_pitch(&[0.0; 4], 1, rate, bad),
        Err(AnalysisError::BadInput("pitch range"))
    );
}
