//! Root pitch on tones whose pitch is known exactly.

use mpfree_analysis::{
    note_name, pitch_track, root_pitch, yin, AnalysisError, PitchSettings, PitchTrack,
};
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

fn hz_of(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// Notes in a row: (MIDI note, or None for a rest; seconds), a voice-like
/// tone of four harmonics; and where each starts, frames.
fn phrase(parts: &[(Option<f32>, f32)], rate: u32) -> (Vec<f32>, Vec<u64>) {
    let mut out = Vec::new();
    let mut starts = Vec::new();
    for &(note, secs) in parts {
        starts.push(out.len() as u64);
        match note {
            Some(n) => out.extend(tone(hz_of(n), &[1.0, 0.5, 0.3, 0.2], rate, secs)),
            None => out.extend(std::iter::repeat_n(0.0, (secs * rate as f32) as usize)),
        }
    }
    (out, starts)
}

fn near(got: &[u64], want: &[u64], rate: u32) {
    let ms = |f: u64| f as f64 * 1000.0 / f64::from(rate);
    assert_eq!(got.len(), want.len(), "cuts {got:?}, want {want:?}");
    for (g, w) in got.iter().zip(want) {
        assert!(
            (ms(*g) - ms(*w)).abs() <= 15.0,
            "cuts {got:?}, want {want:?}"
        );
    }
}

#[test]
fn the_track_reads_each_note_and_a_rest_reads_none() {
    for rate in [44_100, 48_000, 96_000] {
        let (x, _) = phrase(&[(Some(60.0), 0.3), (None, 0.2), (Some(64.0), 0.3)], rate);
        let t = pitch_track(&x, 1, rate).unwrap();
        assert_eq!(t.frames, x.len() as u64);
        let at = |secs: f32| t.notes[((secs * rate as f32) as u64 / t.hop) as usize];
        for (secs, want) in [(0.1, Some(60.0)), (0.35, None), (0.65, Some(64.0))] {
            match (at(secs), want) {
                (Some(g), Some(w)) => assert!((g - w).abs() < 0.1, "{rate}: {g} at {secs}"),
                (g, w) => assert_eq!(g, w, "{rate} at {secs}"),
            }
        }
        // A reading every 10 ms.
        assert!(
            (t.hop as f32 / rate as f32 - 0.01).abs() < 0.001,
            "{}",
            t.hop
        );
    }
}

#[test]
fn cuts_come_where_the_note_changes_by_enough_and_after_a_rest() {
    let rate = 44_100;
    let (x, at) = phrase(
        &[
            (Some(60.0), 0.4),
            (Some(64.0), 0.4),
            (None, 0.2),
            (Some(64.0), 0.4),
            (Some(67.0), 0.3),
            (Some(68.0), 0.3),
        ],
        rate,
    );
    let t = pitch_track(&x, 1, rate).unwrap();
    let min = rate as u64 / 20;
    // Every change of a semitone or more, and the note after the rest.
    near(&t.changes(1.0, min), &[at[1], at[3], at[4], at[5]], rate);
    // Three semitones or more: not the last step up.
    near(&t.changes(3.0, min), &[at[1], at[3], at[4]], rate);
    // Four or more: C to E, and the rest.
    near(&t.changes(4.0, min), &[at[1], at[3]], rate);
    // Twelve: only the rest.
    near(&t.changes(12.0, min), &[at[3]], rate);
    // Notes shorter than the minimum length don't count, nor a rest as
    // short: the E after it is the E before.
    near(&t.changes(1.0, rate as u64 * 7 / 20), &[at[1]], rate);
    // Cuts come within a few milliseconds of each note's start.
    for (got, want) in t.changes(1.0, min).iter().zip([at[1], at[3], at[4], at[5]]) {
        assert!(got.abs_diff(want) <= rate as u64 / 200, "{got} for {want}");
    }
}

#[test]
fn vibrato_and_a_stereo_sound_cut_only_at_the_notes() {
    let rate = 48_000;
    // A note sung with vibrato (±40 cents at 5.5 Hz), then a fifth up.
    let sung = |note: f32, secs: f32, phase0: f32| -> Vec<f32> {
        let mut phase = phase0;
        (0..(secs * rate as f32) as usize)
            .map(|i| {
                let t = i as f32 / rate as f32;
                let hz = hz_of(note + 0.4 * (TAU * 5.5 * t).sin());
                phase += TAU * hz / rate as f32;
                0.3 * (phase.sin() + 0.4 * (2.0 * phase).sin())
            })
            .collect()
    };
    let mut x = sung(57.0, 0.8, 0.0);
    let change = x.len() as u64;
    x.extend(sung(64.0, 0.8, 0.0));
    let stereo: Vec<f32> = x.iter().flat_map(|&v| [v, v * 0.5]).collect();
    let t: PitchTrack = pitch_track(&stereo, 2, rate).unwrap();
    assert_eq!(t.frames, x.len() as u64);
    near(&t.changes(1.0, rate as u64 / 20), &[change], rate);
}

#[test]
fn noise_silence_and_short_sounds_make_no_cuts() {
    let rate = 44_100;
    for x in [
        noise(rate as usize, 9, 0.5),
        vec![0.0; rate as usize],
        tone(440.0, &[1.0], rate, 0.02),
        Vec::new(),
    ] {
        let t = pitch_track(&x, 1, rate).unwrap();
        assert!(t.changes(1.0, rate as u64 / 20).is_empty(), "{t:?}");
    }
    // A note after a lead-in of noise is cut at the note.
    let mut x = noise(rate as usize / 2, 4, 0.3);
    x.extend(tone(220.0, &[1.0, 0.5], rate, 0.5));
    let t = pitch_track(&x, 1, rate).unwrap();
    near(&t.changes(1.0, rate as u64 / 20), &[rate as u64 / 2], rate);
    assert_eq!(
        pitch_track(&[0.0; 3], 2, rate),
        Err(AnalysisError::BadInput("length"))
    );
    assert_eq!(
        pitch_track(&[0.0; 4], 1, 0),
        Err(AnalysisError::BadInput("rate"))
    );
}
