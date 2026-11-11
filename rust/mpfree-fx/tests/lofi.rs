//! Lo-fi: Mix 0 is the input delayed by the reported latency, Amount 0 is
//! clean, wow bends the pitch by about what it says, crackle and hiss ride
//! the music and rest in silence, and the presets play.

use std::f32::consts::PI;

use mpfree_fx::ids::lofi::*;
use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::{LoFi, LOFI_PRESETS};

const RATE: u32 = 48_000;

fn lofi(values: &[(u16, f32)]) -> LoFi {
    let mut f = LoFi::new();
    f.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values { f.set_param(id, v); }
    f.reset();
    f
}

fn run(f: &mut LoFi, l: &mut [f32], r: &mut [f32]) {
    for (i, (a, b)) in l.chunks_mut(256).zip(r.chunks_mut(256)).enumerate() {
        f.process(&Ctx::new(RATE, (i * 256) as u64), &[], a, b);
    }
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> { (0..frames).map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin()).collect() }

fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32 }

fn peak(x: &[f32]) -> f32 { x.iter().fold(0.0f32, |m, v| m.max(v.abs())) }

#[test]
fn mix_zero_is_the_input_delayed_by_the_latency() {
    let mut f = lofi(&[(MIX, 0.0), (AMOUNT, 100.0), (CRACKLE, 100.0), (NOISE, 100.0)]);
    let lat = f.latency() as usize;
    assert_eq!(lat, 96, "2 ms at 48 kHz");
    let x = sine(440.0, 0.5, 9600);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut f, &mut l, &mut r);
    assert!(l[..lat].iter().all(|&v| v == 0.0));
    assert!(l[lat..] == x[..x.len() - lat], "the input, {lat} frames late");
}

#[test]
fn amount_zero_is_clean_and_on_time() {
    let mut f = lofi(&[(AMOUNT, 0.0), (WOW, 100.0), (FLUTTER, 100.0), (SATURATION, 100.0), (CRACKLE, 100.0), (NOISE, 100.0), (WIDTH, 0.0)]);
    let lat = f.latency() as usize;
    let x = sine(1000.0, 0.5, 24_000);
    let y: Vec<f32> = x.iter().map(|v| -v).collect();
    let (mut l, mut r) = (x.clone(), y.clone());
    run(&mut f, &mut l, &mut r);
    // Only the open cuts (20 Hz and 20 kHz) touch it: well under 0.1 dB and
    // in time with the reported latency.
    let err: Vec<f32> = (12_000..24_000).map(|i| l[i] - x[i - lat]).collect();
    assert!(rms(&err) < 0.01 * rms(&x), "error {}", rms(&err));
    assert!((rms(&r[12_000..]) / rms(&y[12_000..]) - 1.0).abs() < 0.01, "full width at Amount 0");
}

/// Mean frequency over windows from zero-crossing spacing.
fn pitch_track(x: &[f32], window: usize) -> Vec<f32> {
    let mut out = Vec::new();
    for w in x.chunks(window) {
        let ups: Vec<f32> = (1..w.len()).filter(|&i| w[i - 1] < 0.0 && w[i] >= 0.0).map(|i| (i - 1) as f32 + w[i - 1] / (w[i - 1] - w[i])).collect();
        if ups.len() > 2 { out.push(RATE as f32 * (ups.len() - 1) as f32 / (ups[ups.len() - 1] - ups[0])); }
    }
    out
}

#[test]
fn wow_bends_the_pitch_by_about_what_it_says() {
    let x = sine(1000.0, 0.3, RATE as usize * 4);
    for (wow, lo, hi) in [(0.0, 0.0, 0.0002), (100.0, 0.002, 0.006)] {
        let mut f = lofi(&[(AMOUNT, 100.0), (WOW, wow), (FLUTTER, 0.0), (SATURATION, 0.0), (CRACKLE, 0.0), (NOISE, 0.0), (LOW_CUT, 20.0), (HIGH_CUT, 20_000.0)]);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let track = pitch_track(&l[4800..], 960);
        let dev = track.iter().fold(0.0f32, |m, &p| m.max((p / 1000.0 - 1.0).abs()));
        assert!(dev >= lo && dev <= hi, "wow {wow}: peak deviation {:.3} %", dev * 100.0);
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
    assert!(l[..RATE as usize].iter().chain(&r[..RATE as usize]).all(|&v| v == 0.0), "silent before the music");
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
            assert!(v >= info.min && v <= info.max, "{}: {} out of range", p.name, info.name);
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
