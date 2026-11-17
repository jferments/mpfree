//! Lo-fi: Mix 0 and Amount 0 are the input delayed by the reported
//! latency, wow bends the pitch by about what it says, crackle and hiss
//! ride the music and rest in silence (or run on with Follow at 0),
//! dropouts dip by their depth along ramps, Stereo moves the sides apart,
//! hum sits on the mains, any block size sounds the same, and the presets
//! play.

use std::f32::consts::PI;

use mpfree_fx::ids::lofi::*;
use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::{LoFi, LOFI_PRESETS};

const RATE: u32 = 48_000;

fn lofi(values: &[(u16, f32)]) -> LoFi {
    let mut f = LoFi::new();
    f.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        f.set_param(id, v);
    }
    f.reset();
    f
}

fn run(f: &mut LoFi, l: &mut [f32], r: &mut [f32]) {
    for (i, (a, b)) in l.chunks_mut(256).zip(r.chunks_mut(256)).enumerate() {
        f.process(&Ctx::new(RATE, (i * 256) as u64), &[], a, b);
    }
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

#[test]
fn mix_zero_is_the_input_delayed_by_the_latency() {
    let mut f = lofi(&[
        (MIX, 0.0),
        (AMOUNT, 100.0),
        (CRACKLE, 100.0),
        (NOISE, 100.0),
    ]);
    let lat = f.latency() as usize;
    assert_eq!(
        lat, 484,
        "10 ms of tape and 4 frames of oversampling at 48 kHz"
    );
    let x = sine(440.0, 0.5, 9600);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut f, &mut l, &mut r);
    assert!(l[..lat].iter().all(|&v| v == 0.0));
    assert!(
        l[lat..] == x[..x.len() - lat],
        "the input, {lat} frames late"
    );
}

#[test]
fn amount_zero_is_the_input_to_the_bit_and_on_time() {
    let mut f = lofi(&[
        (AMOUNT, 0.0),
        (WOW, 100.0),
        (FLUTTER, 100.0),
        (SATURATION, 100.0),
        (CRACKLE, 100.0),
        (NOISE, 100.0),
        (WIDTH, 0.0),
        (DROPOUTS, 100.0),
        (HUM, 100.0),
        (STEREO, 100.0),
        (FOLLOW, 0.0),
    ]);
    let lat = f.latency() as usize;
    let x = sine(1000.0, 0.5, 24_000);
    let y: Vec<f32> = x.iter().map(|v| -0.7 * v).collect();
    let (mut l, mut r) = (x.clone(), y.clone());
    run(&mut f, &mut l, &mut r);
    assert!(l[..lat].iter().chain(&r[..lat]).all(|&v| v == 0.0));
    assert!(
        l[lat..] == x[..x.len() - lat],
        "left: the input, {lat} late"
    );
    assert!(
        r[lat..] == y[..y.len() - lat],
        "right: the input, {lat} late"
    );
}

/// Mean frequency over windows from zero-crossing spacing.
fn pitch_track(x: &[f32], window: usize) -> Vec<f32> {
    let mut out = Vec::new();
    for w in x.chunks(window) {
        let ups: Vec<f32> = (1..w.len())
            .filter(|&i| w[i - 1] < 0.0 && w[i] >= 0.0)
            .map(|i| (i - 1) as f32 + w[i - 1] / (w[i - 1] - w[i]))
            .collect();
        if ups.len() > 2 {
            out.push(RATE as f32 * (ups.len() - 1) as f32 / (ups[ups.len() - 1] - ups[0]));
        }
    }
    out
}

#[test]
fn wow_bends_the_pitch_by_about_what_it_says() {
    let x = sine(1000.0, 0.3, RATE as usize * 4);
    for (wow, lo, hi) in [(0.0, 0.0, 0.0002), (100.0, 0.012, 0.03)] {
        let mut f = lofi(&[
            (AMOUNT, 100.0),
            (WOW, wow),
            (FLUTTER, 0.0),
            (SATURATION, 0.0),
            (CRACKLE, 0.0),
            (NOISE, 0.0),
            (LOW_CUT, 20.0),
            (HIGH_CUT, 20_000.0),
        ]);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let track = pitch_track(&l[4800..], 960);
        let dev = track
            .iter()
            .fold(0.0f32, |m, &p| m.max((p / 1000.0 - 1.0).abs()));
        assert!(
            dev >= lo && dev <= hi,
            "wow {wow}: peak deviation {:.3} %",
            dev * 100.0
        );
    }
}

#[test]
fn crackle_and_hiss_ride_the_music_and_rest_in_silence() {
    let mut f = lofi(&[(AMOUNT, 100.0), (CRACKLE, 100.0), (NOISE, 100.0)]);
    let n = RATE as usize * 9;
    let mut x = vec![0.0f32; n];
    // Silence, two seconds of music, then silence again.
    x[RATE as usize..RATE as usize * 3].copy_from_slice(&sine(220.0, 0.5, RATE as usize * 2));
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut f, &mut l, &mut r);
    assert!(
        l[..RATE as usize]
            .iter()
            .chain(&r[..RATE as usize])
            .all(|&v| v == 0.0),
        "silent before the music"
    );
    // During the music there are clicks: frames far off the clean sine.
    let lat = f.latency() as usize;
    let mut quiet = lofi(&[(AMOUNT, 100.0), (CRACKLE, 0.0), (NOISE, 0.0)]);
    let (mut ql, mut qr) = (x.clone(), x.clone());
    run(&mut quiet, &mut ql, &mut qr);
    let span = RATE as usize * 3 / 2..RATE as usize * 3;
    let dust: Vec<f32> = span.clone().map(|i| l[i] - ql[i]).collect();
    assert!(peak(&dust) > 0.02, "crackle plays: {}", peak(&dust));
    assert!(rms(&dust) > 5e-4, "hiss plays: {}", rms(&dust));
    // After the music, it fades below -80 dB within the tail.
    let end = RATE as usize * 3 + lat + f.tail_frames() as usize;
    assert!(end < n);
    assert!(peak(&l[end..]) < 1e-4 * 0.5, "rested: {}", peak(&l[end..]));
}

#[test]
fn presets_are_in_range_and_play() {
    let x = sine(330.0, 0.5, 48_000);
    for p in LOFI_PRESETS {
        let mut f = LoFi::new();
        for &(id, v) in p.values {
            let info = &f.params()[id as usize];
            assert!(
                v >= info.min && v <= info.max,
                "{}: {} out of range",
                p.name,
                info.name
            );
            f.set_param(id, v);
        }
        f.reset();
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{}", p.name);
        let g = rms(&l[24_000..]) / rms(&x[24_000..]);
        assert!(g > 0.3 && g < 1.5, "{}: level {g}", p.name);
    }
}

/// RMS of each window of `n` frames, dB.
fn windows_db(x: &[f32], n: usize) -> Vec<f32> {
    x.chunks_exact(n)
        .map(|w| 20.0 * rms(w).max(1e-12).log10())
        .collect()
}

#[test]
fn follow_zero_keeps_the_noise_running_in_silence() {
    let mut f = lofi(&[
        (AMOUNT, 100.0),
        (CRACKLE, 0.0),
        (NOISE, 100.0),
        (HUM, 50.0),
        (FOLLOW, 0.0),
    ]);
    assert_eq!(f.rest_frames(), u32::MAX, "never at rest");
    let n = RATE as usize * 6;
    let mut x = vec![0.0f32; n];
    x[..RATE as usize * 2].copy_from_slice(&sine(220.0, 0.5, RATE as usize * 2));
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut f, &mut l, &mut r);
    // After the music: the same bed of noise in each second that follows.
    let s = RATE as usize;
    let a = 20.0 * rms(&l[3 * s..4 * s]).log10();
    let b = 20.0 * rms(&l[5 * s..6 * s]).log10();
    assert!(a > -70.0, "noise plays on in silence: {a} dBFS");
    assert!((a - b).abs() < 1.0, "steady: {a} and {b} dBFS");
    // With Follow at 100 it rests.
    let mut g = lofi(&[(AMOUNT, 100.0), (NOISE, 100.0), (HUM, 50.0)]);
    assert!(g.rest_frames() < u32::MAX);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut g, &mut l, &mut r);
    assert!(
        rms(&l[5 * s..6 * s]) < 1e-5,
        "Follow 100 stops with the music"
    );
}

#[test]
fn dropouts_dip_by_their_depth_along_ramps() {
    let mut f = lofi(&[
        (AMOUNT, 100.0),
        (WOW, 0.0),
        (FLUTTER, 0.0),
        (SATURATION, 0.0),
        (CRACKLE, 0.0),
        (NOISE, 0.0),
        (LOW_CUT, 20.0),
        (HIGH_CUT, 20_000.0),
        (DROPOUTS, 100.0),
    ]);
    let depth = f.dropout_db();
    assert!((depth - 24.0).abs() < 1e-3, "{depth}");
    let n = RATE as usize * 12;
    let x = sine(1000.0, 0.25, n);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut f, &mut l, &mut r);
    // 1 ms windows: one whole cycle of the tone each.
    let lat = f.latency() as usize;
    let db = windows_db(&l[lat + RATE as usize..], 48);
    // The level between dropouts (makeup included): the median window.
    let mut sorted = db.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let steady = sorted[sorted.len() / 2];
    let rel: Vec<f32> = db.iter().map(|v| v - steady).collect();
    let deepest = rel.iter().fold(0.0f32, |m, &v| m.min(v));
    assert!(
        (deepest + depth).abs() < 1.0,
        "dips by its depth: {deepest} dB"
    );
    let dips = rel
        .windows(2)
        .filter(|w| w[0] > -1.0 && w[1] <= -1.0)
        .count();
    assert!(dips >= 10, "about 4 a second: {dips} in 11 s");
    // Every fall from -1 dB to within 1 dB of the depth takes 5 ms or more.
    let mut i = 0;
    while i < rel.len() {
        if rel[i] <= -1.0 && (i == 0 || rel[i - 1] > -1.0) {
            let k = (i..rel.len()).find(|&k| rel[k] <= 1.0 - depth);
            if let Some(k) = k {
                assert!(k - i + 1 >= 5, "a ramp of {} ms at {i} ms", k - i + 1);
            }
        }
        i += 1;
    }
}

#[test]
fn stereo_moves_the_sides_apart_and_zero_keeps_them_together() {
    let x = sine(1000.0, 0.3, RATE as usize * 4);
    let mut spread = Vec::new();
    for stereo in [0.0, 100.0] {
        let mut f = lofi(&[
            (AMOUNT, 100.0),
            (WOW, 80.0),
            (FLUTTER, 0.0),
            (SATURATION, 0.0),
            (CRACKLE, 0.0),
            (NOISE, 0.0),
            (WIDTH, 100.0),
            (STEREO, stereo),
        ]);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let d: Vec<f32> = l.iter().zip(&r).map(|(a, b)| a - b).collect();
        spread.push(rms(&d[RATE as usize..]) / rms(&l[RATE as usize..]));
    }
    assert!(spread[0] == 0.0, "Stereo 0: both sides the same");
    assert!(spread[1] > 0.1, "Stereo 100: apart, {}", spread[1]);
}

#[test]
fn hum_sits_on_the_mains() {
    for (mains, hz, other) in [(0.0, 50.0, 60.0), (1.0, 60.0, 50.0)] {
        let mut f = lofi(&[
            (AMOUNT, 100.0),
            (CRACKLE, 0.0),
            (NOISE, 0.0),
            (HUM, 100.0),
            (FOLLOW, 0.0),
            (MAINS, mains),
        ]);
        let n = RATE as usize * 2;
        let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
        run(&mut f, &mut l, &mut r);
        let y = &l[RATE as usize..];
        let at = |f: f32| {
            let (mut c, mut s) = (0.0f64, 0.0f64);
            for (i, v) in y.iter().enumerate() {
                let p = 2.0 * std::f64::consts::PI * f as f64 * i as f64 / RATE as f64;
                c += *v as f64 * p.cos();
                s += *v as f64 * p.sin();
            }
            (c * c + s * s).sqrt() as f32
        };
        let level = 20.0 * rms(y).log10();
        assert!((level + 40.0).abs() < 3.0, "about -40 dBFS: {level}");
        assert!(at(hz) > 30.0 * at(other), "{hz} Hz, not {other}");
    }
}

#[test]
fn any_block_size_sounds_the_same() {
    let values = [
        (AMOUNT, 90.0),
        (WOW, 70.0),
        (SATURATION, 60.0),
        (DROPOUTS, 60.0),
        (STEREO, 50.0),
        (HUM, 40.0),
        (FOLLOW, 50.0),
    ];
    let n = RATE as usize * 3;
    let x = sine(330.0, 0.5, n);
    let mut outs = Vec::new();
    for block in [1usize, 37, 256] {
        let mut f = lofi(&values);
        let (mut l, mut r) = (x.clone(), x.clone());
        for (i, (a, b)) in l.chunks_mut(block).zip(r.chunks_mut(block)).enumerate() {
            // A frame every block size starts a block on (37 x 256 x 5).
            if i * block == 47_360 {
                f.set_param(SATURATION, 10.0);
                f.set_param(mpfree_fx::ids::lofi::RATE, 2.0);
            }
            f.process(&Ctx::new(RATE, (i * block) as u64), &[], a, b);
        }
        outs.push((l, r));
    }
    for o in &outs[1..] {
        assert!(o.0 == outs[0].0 && o.1 == outs[0].1);
    }
}
