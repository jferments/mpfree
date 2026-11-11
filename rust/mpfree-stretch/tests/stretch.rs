use std::sync::Arc;

use mpfree_analysis::{yin, OnsetAnalysis};
use mpfree_stretch::{stretch, Job, Settings, StretchError, TimeMap};

const RATE: u32 = 44_100;

fn tone(frames: usize, hz: [f64; 2]) -> Vec<f32> {
    let mut v = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let t = i as f64 / f64::from(RATE);
        for h in hz {
            v.push((0.5 * (std::f64::consts::TAU * h * t).sin()) as f32);
        }
    }
    v
}

fn channel(data: &[f32], c: usize) -> Vec<f32> {
    data.iter().skip(c).step_by(2).copied().collect()
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len().max(1) as f64).sqrt()
}

/// The pitch of `x` round its middle, in Hz.
fn pitch_mid(x: &[f32]) -> f32 {
    let m = x.len() / 2;
    yin(&x[m..], RATE, 4096, 60.0, 2000.0, 0.15)
        .expect("pitched")
        .0
}

fn cents(a: f32, b: f32) -> f32 {
    1200.0 * (a / b).log2()
}

/// Noise bursts, 15 ms long and falling fast, every `every` frames;
/// mono, and the frame of each.
fn clicks(frames: usize, every: usize) -> (Vec<f32>, Vec<usize>) {
    let mut out = vec![0.0f32; frames];
    let mut seed = 0x1234_5678u32;
    let mut hits = Vec::new();
    let mut at = every / 2;
    while at + 2000 < frames {
        hits.push(at);
        for i in 0..660 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let n = seed as f32 / u32::MAX as f32 * 2.0 - 1.0;
            out[at + i] += 0.8 * n * (-(i as f32) / 120.0).exp();
        }
        at += every;
    }
    (out, hits)
}

#[test]
fn a_stretched_tone_keeps_its_pitch_and_level_and_lasts_exactly() {
    let frames = 2 * RATE as usize;
    let input = tone(frames, [440.0, 330.0]);
    let level = rms(&channel(&input, 0));
    for ratio in [0.5, 0.8, 1.25, 1.5, 2.0, 3.0] {
        let map = TimeMap::ratio(frames as u64, ratio).unwrap();
        let out = stretch(&input, 2, RATE, map, Settings::default()).unwrap();
        let want = (frames as f64 * ratio).round() as usize;
        assert_eq!(out.len(), want * 2, "ratio {ratio}");
        for (c, hz) in [(0, 440.0), (1, 330.0)] {
            let x = channel(&out, c);
            let p = pitch_mid(&x);
            assert!(
                cents(p, hz).abs() < 5.0,
                "ratio {ratio} channel {c}: {p} Hz"
            );
            let mid = &x[want / 4..want * 3 / 4];
            let db = 20.0 * (rms(mid) / level).log10();
            assert!(db.abs() < 1.0, "ratio {ratio} channel {c}: {db:.2} dB");
        }
    }
}

#[test]
fn a_pitch_shift_moves_the_pitch_and_keeps_the_length() {
    let frames = 2 * RATE as usize;
    let input = tone(frames, [220.0, 220.0]);
    let level = rms(&channel(&input, 0));
    for semis in [-12.0f32, -5.0, 3.0, 7.0, 12.0] {
        let map = TimeMap::ratio(frames as u64, 1.0).unwrap();
        let s = Settings {
            semitones: semis,
            ..Settings::default()
        };
        let out = stretch(&input, 2, RATE, map, s).unwrap();
        assert_eq!(out.len(), frames * 2, "{semis} st");
        let x = channel(&out, 0);
        let want = 220.0 * 2f32.powf(semis / 12.0);
        let p = pitch_mid(&x);
        assert!(
            cents(p, want).abs() < 5.0,
            "{semis} st: {p} Hz, want {want}"
        );
        let db = 20.0 * (rms(&x[frames / 4..frames * 3 / 4]) / level).log10();
        assert!(db.abs() < 1.0, "{semis} st: {db:.2} dB");
    }
    // Both at once: half as fast and a fifth up.
    let map = TimeMap::ratio(frames as u64, 2.0).unwrap();
    let s = Settings {
        semitones: 7.0,
        ..Settings::default()
    };
    let out = stretch(&input, 2, RATE, map, s).unwrap();
    assert_eq!(out.len(), frames * 4);
    let p = pitch_mid(&channel(&out, 1));
    assert!(cents(p, 220.0 * 2f32.powf(7.0 / 12.0)).abs() < 5.0, "{p}");
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn hits_land_where_the_map_puts_them_and_keep_their_attack() {
    let frames = 3 * RATE as usize;
    let (mono, hits) = clicks(frames, 11_025);
    for ratio in [0.7, 1.5, 2.0] {
        let map = TimeMap::ratio(frames as u64, ratio).unwrap();
        let mut job = Job::new(
            mono.clone().into(),
            1,
            RATE,
            map.clone(),
            Settings::default(),
        )
        .unwrap();
        while !job.run(4096).finished {}
        let (found, resets) = job.hits();
        assert_eq!(found, hits.len(), "ratio {ratio}: the hits found");
        assert!(resets >= hits.len() - 1, "ratio {ratio}: {resets} restarts");
        let out = job.into_output().unwrap();
        let onsets = OnsetAnalysis::analyse(&out, 1, RATE).unwrap().pick(0.6);
        assert_eq!(onsets.len(), hits.len(), "ratio {ratio}: {onsets:?}");
        for (&h, &o) in hits.iter().zip(&onsets) {
            let want = map.output_at(h as f64);
            let ms = (o as f64 - want) / f64::from(RATE) * 1000.0;
            assert!(
                ms.abs() < 6.0,
                "ratio {ratio}: hit at {h} landed {ms:.1} ms off"
            );
            // The finder puts a hit a few frames off, so look from 3 ms
            // before.
            let w = want as usize - 128;
            let (p, p_in) = (peak(&out[w..w + 1000]), peak(&mono[h..h + 900]));
            assert!(p > 0.97 * p_in, "ratio {ratio}: attack {p} of {p_in}");
        }
    }
    // At twice the length every hit lands on a whole frame, and the first
    // quarter window from where the finder put it comes out as it went in.
    let map = TimeMap::ratio(frames as u64, 2.0).unwrap();
    let out = stretch(&mono, 1, RATE, map, Settings::default()).unwrap();
    for u in OnsetAnalysis::analyse(&mono, 1, RATE).unwrap().pick(0.6) {
        let u = u as usize;
        let worst = (0..512)
            .map(|i| (out[2 * u + i] - mono[u + i]).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "the attack at {u} is off by {worst}");
    }
}

/// A 4/4 beat at `bpm`: kick on 1 and 3, snare on 2 and 4, a closed hat
/// on every eighth between; mono, and each hit's frame.
fn beat(bpm: f64, bars: usize) -> (Vec<f32>, Vec<usize>) {
    let rate = f64::from(RATE);
    let eighth = 60.0 / bpm / 2.0 * rate;
    let mut out = vec![0.0f32; (eighth * 8.0 * bars as f64) as usize + RATE as usize / 2];
    let mut seed = 0x1234_5678u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut hits = Vec::new();
    for e in 0..8 * bars {
        let at = (e as f64 * eighth).round() as usize;
        hits.push(at);
        let (len, kind) = match e % 8 {
            0 | 4 => (0.8, 0),
            2 | 6 => (0.5, 1),
            _ => (0.12, 2),
        };
        let mut phase = 0.0f32;
        for i in 0..((len * rate) as usize).min(out.len() - at) {
            let t = i as f32 / RATE as f32;
            out[at + i] += match kind {
                0 => {
                    phase +=
                        std::f32::consts::TAU * (50.0 + 100.0 * (-t * 30.0).exp()) / RATE as f32;
                    0.8 * phase.sin() * (-t * 9.0).exp()
                }
                1 => {
                    0.7 * (0.6 * noise() * (-t * 18.0).exp()
                        + 0.4 * (std::f32::consts::TAU * 190.0 * t).sin() * (-t * 25.0).exp())
                }
                _ => 0.25 * 0.5 * (noise() - noise()) * (-t * 80.0).exp(),
            };
        }
    }
    (out, hits)
}

#[test]
fn a_beat_at_a_new_tempo_keeps_every_hit_on_the_grid() {
    let (mono, hits) = beat(120.0, 2);
    let n = mono.len() as u64;
    for ratio in [0.8, 1.25, 1.6] {
        let map = TimeMap::ratio(n, ratio).unwrap();
        let out = stretch(&mono, 1, RATE, map.clone(), Settings::default()).unwrap();
        let onsets = OnsetAnalysis::analyse(&out, 1, RATE).unwrap().pick(0.6);
        // The very start is never an onset.
        for &h in &hits[1..] {
            let want = map.output_at(h as f64);
            let near = onsets
                .iter()
                .map(|&o| (o as f64 - want).abs())
                .fold(f64::MAX, f64::min);
            let ms = near / f64::from(RATE) * 1000.0;
            assert!(
                ms < 6.0,
                "ratio {ratio}: the hit at {h} is {ms:.1} ms from any onset"
            );
        }
    }
}

#[test]
fn a_steady_chord_is_not_taken_for_hits() {
    // Two partials beat in the mono sum, which can fool the hit finder;
    // starting the phases again there would wobble the pitch.
    let frames = 2 * RATE as usize;
    let input = tone(frames, [440.0, 330.0]);
    let map = TimeMap::ratio(frames as u64, 1.5).unwrap();
    let mut job = Job::new(input.into(), 2, RATE, map, Settings::default()).unwrap();
    while !job.run(1 << 16).finished {}
    assert_eq!(job.hits().1, 0, "no restarts in a steady sound");
}

#[test]
fn warp_markers_move_each_part_by_its_own_amount() {
    let frames = 4 * RATE as usize;
    let (mono, hits) = clicks(frames, 22_050);
    let n = frames as f64;
    // The first half twice as long, the second half at two thirds.
    let map = TimeMap::new(vec![(0.0, 0.0), (n / 2.0, n), (n, n + n / 3.0)]).unwrap();
    let out = stretch(&mono, 1, RATE, map.clone(), Settings::default()).unwrap();
    assert_eq!(out.len(), map.output_frames() as usize);
    let onsets = OnsetAnalysis::analyse(&out, 1, RATE).unwrap().pick(0.6);
    assert_eq!(onsets.len(), hits.len());
    for (&h, &o) in hits.iter().zip(&onsets) {
        let ms = (o as f64 - map.output_at(h as f64)) / f64::from(RATE) * 1000.0;
        assert!(ms.abs() < 6.0, "hit at {h}: {ms:.1} ms off");
    }
}

#[test]
fn stereo_stays_where_it_was() {
    let frames = RATE as usize;
    // Left only.
    let mut left_only = tone(frames, [500.0, 0.0]);
    for v in left_only.iter_mut().skip(1).step_by(2) {
        *v = 0.0;
    }
    let map = TimeMap::ratio(frames as u64, 1.5).unwrap();
    let out = stretch(&left_only, 2, RATE, map.clone(), Settings::default()).unwrap();
    let (l, r) = (channel(&out, 0), channel(&out, 1));
    assert!(
        20.0 * (rms(&r) / rms(&l)).log10() < -60.0,
        "right stays silent"
    );
    // Out of phase stays out of phase.
    let mut anti = tone(frames, [500.0, 500.0]);
    for v in anti.iter_mut().skip(1).step_by(2) {
        *v = -*v;
    }
    let out = stretch(&anti, 2, RATE, map, Settings::default()).unwrap();
    let (l, r) = (channel(&out, 0), channel(&out, 1));
    let sum: f64 = l
        .iter()
        .zip(&r)
        .map(|(a, b)| f64::from(a + b).powi(2))
        .sum();
    assert!(10.0 * (sum / (2.0 * rms(&l).powi(2) * l.len() as f64)).log10() < -60.0);
}

#[test]
fn any_step_size_gives_the_same_sound_and_no_change_is_a_copy() {
    let frames = RATE as usize;
    let input = tone(frames, [300.0, 450.0]);
    let map = TimeMap::ratio(frames as u64, 1.3).unwrap();
    let s = Settings {
        semitones: 2.0,
        ..Settings::default()
    };
    let run = |step: usize| {
        let mut job = Job::new(Arc::from(input.clone()), 2, RATE, map.clone(), s).unwrap();
        let mut last = 0.0;
        loop {
            let p = job.run(step);
            assert!(p.done >= last, "progress goes up");
            last = p.done;
            if p.finished {
                break;
            }
        }
        assert_eq!(last, 1.0);
        job.into_output().unwrap()
    };
    assert_eq!(run(100), run(1 << 20));
    let same = stretch(
        &input,
        2,
        RATE,
        TimeMap::ratio(frames as u64, 1.0).unwrap(),
        Settings::default(),
    )
    .unwrap();
    assert_eq!(same, input);
}

#[test]
fn bad_jobs_are_refused() {
    let input = tone(1000, [300.0, 300.0]);
    let map = TimeMap::ratio(1000, 1.5).unwrap();
    let job = |ch: u8, rate: u32, map: TimeMap, s: Settings| {
        Job::new(Arc::from(input.clone()), ch, rate, map, s).err()
    };
    let d = Settings::default();
    assert_eq!(job(3, RATE, map.clone(), d), Some(StretchError::Channels));
    assert_eq!(job(2, 1000, map.clone(), d), Some(StretchError::Rate));
    let far = Settings {
        semitones: 25.0,
        ..d
    };
    assert_eq!(job(2, RATE, map.clone(), far), Some(StretchError::Pitch));
    let odd = Settings { window: 1000, ..d };
    assert_eq!(job(2, RATE, map, odd), Some(StretchError::Window));
    let short = TimeMap::ratio(500, 1.5).unwrap();
    assert!(matches!(job(2, RATE, short, d), Some(StretchError::Map(_))));
}
