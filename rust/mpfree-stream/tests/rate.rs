//! Files at their own rate (the later phases' P3, step 3): stems and
//! clips at another rate resampled ahead into engine-rate units.

use std::f64::consts::TAU;

use mpfree_stream::coord::Plan;
use mpfree_stream::cursor::{Cursor, Read};
use mpfree_stream::player::{unit_frames, Config};
use mpfree_stream::resample::Rate;
use mpfree_stream::testkit::{float_wav, folder, Rig};

fn sine(rate: u32, hz: f64, frames: usize, channels: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for f in 0..frames {
        let x = (TAU * hz * f as f64 / f64::from(rate)).sin() * 0.5;
        for c in 0..channels {
            out.push((x * if c == 0 { 1.0 } else { -1.0 }) as f32);
        }
    }
    out
}

/// The whole file resampled in one pass.
fn whole(rate: &Rate, channels: usize, input: &[f32]) -> Vec<f32> {
    let mut out = vec![0.0; rate.frames() as usize * channels];
    rate.run(channels, input, 0, 0, &mut out);
    out
}

fn rms(x: &[f32]) -> f64 {
    (x.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

#[test]
fn places_are_exact_and_there_is_no_delay() {
    let r = Rate::new(48_000, 44_100, 48_000).expect("converts");
    assert_eq!(r.frames(), 44_100);
    assert!((r.ratio() - 48.0 / 44.1).abs() < 1e-12);
    assert_eq!(
        Rate::new(44_100, 44_100, 10),
        None,
        "44.1 kHz isn't converted"
    );
    assert_eq!(Rate::new(0, 44_100, 10), None);
    assert_eq!(Rate::new(48_000, 44_100, 1).expect("one").frames(), 1);
    // Ten hours of 48 kHz audio: the last frame's place doesn't drift.
    let long = 48_000u64 * 36_000;
    let r = Rate::new(48_000, 44_100, long).expect("converts");
    assert_eq!(r.frames(), 44_100 * 36_000);
    // An impulse at file frame 48,000 (one second) peaks at engine frame
    // 44,100: the kernel is symmetric, so nothing needs making up.
    let mut input = vec![0.0f32; 96_000];
    input[48_000] = 1.0;
    let r = Rate::new(48_000, 44_100, 96_000).expect("converts");
    let out = whole(&r, 1, &input);
    let peak = (0..out.len())
        .max_by(|&a, &b| out[a].abs().total_cmp(&out[b].abs()))
        .expect("some");
    assert_eq!(peak, 44_100);
}

#[test]
fn the_pass_band_is_kept_and_what_would_fold_is_cut() {
    // 1 kHz at 48 kHz comes out as 1 kHz at 44.1 kHz, within a hair.
    let input = sine(48_000, 1_000.0, 48_000, 1);
    let r = Rate::new(48_000, 44_100, 48_000).expect("converts");
    let out = whole(&r, 1, &input);
    let want = sine(44_100, 1_000.0, out.len(), 1);
    let mid = 2_000..out.len() - 2_000;
    let err = out[mid.clone()]
        .iter()
        .zip(&want[mid.clone()])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(err < 1e-3, "{err}");
    // 15 kHz is still flat (within 0.1 dB).
    let input = sine(48_000, 15_000.0, 48_000, 1);
    let out = whole(&r, 1, &input);
    let db = 20.0 * (rms(&out[mid.clone()]) / rms(&input[2_000..46_000])).log10();
    assert!(db.abs() < 0.1, "{db} dB");
    // 26 kHz in a 96 kHz file is past what 44.1 kHz carries: it would
    // fold to 18.1 kHz, and is cut by more than 55 dB instead.
    let input = sine(96_000, 26_000.0, 96_000, 1);
    let r = Rate::new(96_000, 44_100, 96_000).expect("converts");
    let out = whole(&r, 1, &input);
    let db = 20.0 * (rms(&out[mid.clone()]) / rms(&input)).log10();
    assert!(db < -55.0, "{db} dB");
    // Going up (22.05 kHz to 44.1 kHz), 5 kHz comes through clean.
    let input = sine(22_050, 5_000.0, 22_050, 2);
    let r = Rate::new(22_050, 44_100, 22_050).expect("converts");
    let out = whole(&r, 2, &input);
    assert_eq!(out.len(), 44_100 * 2);
    let want = sine(44_100, 5_000.0, 44_100, 2);
    let mid = 4_000..out.len() - 4_000;
    let err = out[mid.clone()]
        .iter()
        .zip(&want[mid])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(err < 2e-3, "{err}");
}

#[test]
fn a_unit_made_from_its_span_is_the_whole_pass_bit_for_bit() {
    let frames = 100_000usize;
    let input = sine(48_000, 440.0, frames, 2);
    let r = Rate::new(48_000, 44_100, frames as u64).expect("converts");
    let all = whole(&r, 2, &input);
    let uf = unit_frames(2);
    // Every unit, any order, each from its own span only.
    for u in (0..r.frames().div_ceil(uf)).rev() {
        let first = u * uf;
        let n = (r.frames() - first).min(uf);
        let (a, b) = r.span(first, n);
        assert!(b - a < n * 2, "a unit reads about its own length");
        let mut out = vec![0.0; n as usize * 2];
        r.run(
            2,
            &input[a as usize * 2..b as usize * 2],
            a,
            first,
            &mut out,
        );
        assert_eq!(
            out[..],
            all[first as usize * 2..(first + n) as usize * 2],
            "unit {u}"
        );
    }
    // A short piece anywhere (a seek into the middle of a unit) too.
    let (first, n) = (12_345u64, 77u64);
    let (a, b) = r.span(first, n);
    let mut out = vec![0.0; n as usize * 2];
    r.run(
        2,
        &input[a as usize * 2..b as usize * 2],
        a,
        first,
        &mut out,
    );
    assert_eq!(out[..], all[first as usize * 2..(first + n) as usize * 2]);
}

fn rig() -> Rig {
    Rig::new(
        Config {
            slots: 256,
            ..Config::default()
        },
        Plan::default(),
    )
}

/// Play `k` for `periods` periods of 256 frames; gives what came out and
/// the frames missed.
fn play(rig: &mut Rig, k: &mut [&mut Cursor], periods: usize) -> (Vec<Vec<f32>>, u64) {
    let mut got = vec![Vec::new(); k.len()];
    let mut missed = 0;
    for _ in 0..periods {
        let mut outs = vec![vec![0.0; 512]; k.len()];
        missed += rig.period(k, &mut outs);
        for (g, o) in got.iter_mut().zip(outs) {
            g.extend(o);
        }
    }
    (got, missed)
}

#[test]
fn sixteen_48k_stems_play_at_the_engine_rate_sample_aligned() {
    let dir = folder("rate-stems");
    let frames = 48_000usize;
    let input = sine(48_000, 440.0, frames, 2);
    let r = Rate::new(48_000, 44_100, frames as u64).expect("converts");
    let want = whole(&r, 2, &input);
    let mut rig = rig();
    rig.c.new_project();
    for s in 0..16u16 {
        let p = dir.join(format!("stem{s}.wav"));
        float_wav(&p, 48_000, 2, &input);
        rig.c.open_at_engine_rate(s, &p).expect("opens");
        assert_eq!(rig.c.frames(s), Some(44_100));
        assert_eq!(rig.c.layout(s).map(|l| l.frames), Some(48_000));
    }
    let mut ks: Vec<Cursor> = (0..16).map(|s| Cursor::new(s, s)).collect();
    {
        let mut refs: Vec<&mut Cursor> = ks.iter_mut().collect();
        rig.warm(&mut refs, 400);
    }
    let mut refs: Vec<&mut Cursor> = ks.iter_mut().collect();
    let (got, missed) = play(&mut rig, &mut refs, 100);
    assert_eq!(missed, 0);
    for g in &got {
        assert_eq!(g[..], want[..g.len()], "every stem, every frame");
    }
    // A seek and a loop, in engine frames, land on the same frames.
    let mut k = Cursor::new(3, 0);
    k.seek(30_000);
    k.set_loop(Some((30_000, 30_600)));
    rig.warm(&mut [&mut k], 400);
    let (got, missed) = play(&mut rig, &mut [&mut k], 8);
    assert_eq!(missed, 0);
    for (j, frame) in got[0].chunks_exact(2).enumerate() {
        let f = 30_000 + j % 600;
        assert_eq!(frame, &want[f * 2..f * 2 + 2], "frame {j}");
    }
}

#[test]
fn a_file_at_the_engine_rate_plays_its_own_samples() {
    let dir = folder("rate-same");
    let input = sine(44_100, 440.0, 20_000, 1);
    let p = dir.join("same.wav");
    float_wav(&p, 44_100, 1, &input);
    let mut rig = rig();
    rig.c.new_project();
    rig.c.open_at_engine_rate(0, &p).expect("opens");
    assert_eq!(rig.c.frames(0), Some(20_000));
    let mut k = Cursor::new(0, 0);
    rig.warm(&mut [&mut k], 200);
    let (got, missed) = play(&mut rig, &mut [&mut k], 20);
    assert_eq!(missed, 0);
    for (j, frame) in got[0].chunks_exact(2).enumerate() {
        assert_eq!(frame, &[input[j], input[j]], "frame {j}");
    }
}

/// A 44.1 kHz mono sine in a file, opened, and a cursor on it reading
/// `read` at `speed` from `from`: what it plays (left channel), after the
/// first period.
fn read_at(name: &str, hz: f64, read: Read, speed: f64, from: u64) -> (Vec<f32>, Vec<f32>) {
    let dir = folder(name);
    let input = sine(44_100, hz, 120_000, 1);
    let p = dir.join("tone.wav");
    float_wav(&p, 44_100, 1, &input);
    let mut rig = rig();
    rig.c.new_project();
    rig.c.open(0, &p).expect("opens");
    let mut k = Cursor::new(0, 0);
    k.set_read(read);
    k.set_speed(speed);
    k.seek(from);
    rig.warm(&mut [&mut k], 400);
    let (got, missed) = play(&mut rig, &mut [&mut k], 80);
    assert_eq!(missed, 0);
    (got[0].iter().step_by(2).copied().collect(), input)
}

#[test]
fn a_band_limited_read_keeps_its_whole_frames_and_its_pitch() {
    // Half speed: every other output lands on a whole frame, unstretched:
    // the file's own sample, bit for bit.
    let (got, input) = read_at("sinc-half", 440.0, Read::Sinc8, 0.5, 1_000);
    for (j, &x) in got.iter().enumerate().step_by(2) {
        assert_eq!(x, input[1_000 + j / 2], "output {j}");
    }
    // 1.5 times: 1 kHz comes out as 1.5 kHz, matching the tone read at
    // those places.
    for read in [Read::Sinc8, Read::Sinc16] {
        let (got, _) = read_at("sinc-pitch", 1_000.0, read, 1.5, 1_000);
        let err = got
            .iter()
            .enumerate()
            .map(|(j, &x)| {
                let at = 1_000.0 + 1.5 * j as f64;
                let want = (TAU * 1_000.0 * at / 44_100.0).sin() * 0.5;
                (f64::from(x) - want).abs()
            })
            .fold(0.0f64, f64::max);
        assert!(err < 2e-3, "{read:?}: {err}");
    }
}

#[test]
fn a_band_limited_read_twice_as_fast_doesnt_fold_back() {
    // 15 kHz read twice as fast is 30 kHz: past the output's 22.05 kHz,
    // it would fold back to 14.1 kHz. Hermite lets most of it through;
    // the band-limited reads cut it.
    let level = |read| {
        let (got, input) = read_at("sinc-fold", 15_000.0, read, 2.0, 1_000);
        20.0 * (rms(&got) / rms(&input)).log10()
    };
    let hermite = level(Read::Hermite);
    let sinc8 = level(Read::Sinc8);
    let sinc16 = level(Read::Sinc16);
    assert!(hermite > -12.0, "Hermite: {hermite} dB");
    assert!(sinc8 < -55.0, "Sinc8: {sinc8} dB");
    assert!(sinc16 < -75.0, "Sinc16: {sinc16} dB");
    // A tone below the output's Nyquist frequency after the speed (5 kHz,
    // read as 10 kHz) comes through at its level.
    let (got, input) = read_at("sinc-pass", 5_000.0, Read::Sinc8, 2.0, 1_000);
    let db = 20.0 * (rms(&got) / rms(&input)).log10();
    assert!(db.abs() < 0.5, "{db} dB");
}
