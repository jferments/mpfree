//! Spring reverb behaviour: its parameter ids hold (saved projects load),
//! Decay is the time for the middle to fall 60 dB at any rate, Length,
//! Tension, Drip and Tone, a longer Decay rings longer rather than
//! louder, a source keeps its side, the tail rings out to silence, every
//! control can sweep under noise without blowing up, and the output is
//! the same, bit for bit, at any block size. The node rules (valid
//! description, any block size, every extreme, no allocation) are checked
//! with the other effects in `coverage.rs` and `coverage_alloc.rs`, the
//! chirp there too; the measured targets (R1 to R12) in fxlab's quality
//! ratchet.

use mpfree_fx::ids::spring::*;
use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::Spring;

const RATE: u32 = 48_000;

fn spring(rate: u32, values: &[(u16, f32)]) -> Spring {
    let mut s = Spring::new();
    s.prepare(rate, MAX_BLOCK);
    for &(id, v) in values {
        s.set_param(id, v);
    }
    s.reset();
    s
}

fn run_in(s: &mut Spring, rate: u32, block: usize, l: &mut [f32], r: &mut [f32]) {
    let mut at = 0;
    for (a, b) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
        s.process(&Ctx::new(rate, at as u64), &[], a, b);
        at += a.len();
    }
}

fn run(s: &mut Spring, rate: u32, l: &mut [f32], r: &mut [f32]) {
    run_in(s, rate, 128, l, r);
}

fn noise(seed: u64, frames: usize, amp: f32) -> Vec<f32> {
    let mut x = seed.max(1);
    (0..frames)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            amp * ((x >> 40) as f32 / (1u64 << 23) as f32 - 1.0)
        })
        .collect()
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|v| (*v as f64).powi(2)).sum()
}

/// `x` through a band-pass an octave wide at `hz` (two RBJ band-passes).
fn band(x: &[f32], hz: f64, rate: u32) -> Vec<f64> {
    let w = std::f64::consts::TAU * hz / rate as f64;
    let alpha = w.sin() / (2.0 * 1.41);
    let a0 = 1.0 + alpha;
    let (b0, a1, a2) = (alpha / a0, -2.0 * w.cos() / a0, (1.0 - alpha) / a0);
    let mut y: Vec<f64> = x.iter().map(|v| *v as f64).collect();
    for _ in 0..2 {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        for v in &mut y {
            let out = b0 * *v - b0 * x2 - a1 * y1 - a2 * y2;
            (x2, x1, y2, y1) = (x1, *v, y1, out);
            *v = out;
        }
    }
    y
}

/// The 60 dB decay time of the 500 Hz and 1 kHz octaves (their mean),
/// each from its Schroeder curve's fall from -5 to -25 dB.
fn t60_mid(l: &[f32], r: &[f32], rate: u32) -> f32 {
    let t = |hz: f64| {
        let (bl, br) = (band(l, hz, rate), band(r, hz, rate));
        let e: Vec<f64> = bl.iter().zip(&br).map(|(a, b)| a * a + b * b).collect();
        let mut tail = vec![0.0; e.len() + 1];
        for i in (0..e.len()).rev() {
            tail[i] = tail[i + 1] + e[i];
        }
        let level = |i: usize| 10.0 * (tail[i] / tail[0]).log10();
        let a = (0..e.len()).find(|&i| level(i) < -5.0).unwrap();
        let b = (0..e.len()).find(|&i| level(i) < -25.0).unwrap();
        3.0 * (b - a) as f32 / rate as f32
    };
    0.5 * (t(500.0) + t(1000.0))
}

fn impulse_response(rate: u32, values: &[(u16, f32)], secs: f32) -> (Vec<f32>, Vec<f32>) {
    let mut s = spring(rate, values);
    let n = (secs * rate as f32) as usize;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    l[0] = 1.0;
    r[0] = 1.0;
    run(&mut s, rate, &mut l, &mut r);
    (l, r)
}

#[test]
fn the_ids_saved_projects_use_hold() {
    let s = Spring::new();
    let names: Vec<(u16, &str)> = s.params().iter().map(|q| (q.id, q.name)).collect();
    let want = [
        (LENGTH, "Length"),
        (DECAY, "Decay"),
        (TENSION, "Tension"),
        (DRIP, "Drip"),
        (TONE, "Tone"),
        (LOW_CUT, "Low cut"),
        (MIX, "Mix"),
        (MIX_CURVE, "Mix curve"),
    ];
    for (id, name) in want {
        assert!(names.contains(&(id, name)), "{id} {name}: {names:?}");
    }
    assert_eq!(s.info().id, "mpfree.fx.spring");
}

#[test]
fn decay_is_the_time_for_the_middle_to_fall_60_db() {
    let cases: [&[(u16, f32)]; 9] = [
        &[(DECAY, 1000.0)],
        &[(DECAY, 2500.0)],
        &[(DECAY, 6000.0)],
        &[(DECAY, 2500.0), (LENGTH, 20.0)],
        &[(DECAY, 2500.0), (LENGTH, 80.0)],
        &[(DECAY, 2500.0), (TENSION, 0.0), (DRIP, 0.0)],
        &[(DECAY, 2500.0), (TENSION, 100.0), (DRIP, 100.0)],
        &[(DECAY, 2500.0), (TONE, 1000.0)],
        &[(DECAY, 2500.0), (TONE, 8000.0)],
    ];
    for rate in [44_100, 48_000, 96_000] {
        for case in cases {
            let mut v = case.to_vec();
            v.extend([(MIX, 100.0), (LOW_CUT, 20.0)]);
            let decay = v[0].1 * 0.001;
            let (l, r) = impulse_response(rate, &v, decay * 0.7 + 0.3);
            let got = t60_mid(&l, &r, rate);
            let err = (got - decay) / decay;
            assert!(err.abs() < 0.12, "{rate} Hz, {case:?}: {got:.2} s");
        }
    }
}

#[test]
fn a_longer_decay_rings_longer_not_louder() {
    let level = |decay: f32| {
        let mut s = spring(RATE, &[(MIX, 100.0), (DECAY, decay)]);
        let n = 4 * RATE as usize;
        let (mut l, mut r) = (noise(1, n, 0.5), noise(2, n, 0.5));
        run(&mut s, RATE, &mut l, &mut r);
        let from = 3 * RATE as usize;
        10.0 * ((energy(&l[from..]) + energy(&r[from..])) / (2 * (n - from)) as f64).log10()
    };
    let (short, long) = (level(500.0), level(8000.0));
    assert!(
        (long - short).abs() < 2.0,
        "{short:.1} dB at 0.5 s, {long:.1} dB at 8 s"
    );
}

#[test]
fn a_source_keeps_its_side() {
    let mut s = spring(RATE, &[(MIX, 100.0)]);
    let n = RATE as usize / 20;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    l[0] = 1.0;
    run(&mut s, RATE, &mut l, &mut r);
    let side = 10.0 * (energy(&l) / energy(&r)).log10();
    assert!(
        side > 6.0,
        "the left's early energy is only {side:.1} dB stronger"
    );
}

#[test]
fn the_tail_rings_out_to_silence() {
    let mut s = spring(RATE, &[(MIX, 100.0), (DECAY, 1000.0), (TENSION, 100.0)]);
    let n = RATE as usize;
    let mut l = noise(3, n, 0.5);
    let mut r = noise(4, n, 0.5);
    run(&mut s, RATE, &mut l, &mut r);
    let rest = s.rest_frames() as usize;
    let (mut l, mut r) = (vec![0.0; rest], vec![0.0; rest]);
    run(&mut s, RATE, &mut l, &mut r);
    let last = rest - RATE as usize / 10;
    let peak = l[last..]
        .iter()
        .chain(&r[last..])
        .fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak < 1e-5, "{peak} after its rest");
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
}

#[test]
fn every_control_sweeps_under_noise_without_blowing_up() {
    let mut s = spring(RATE, &[(MIX, 100.0), (DECAY, 8000.0), (DRIP, 100.0)]);
    let block = 128;
    let blocks = 3 * RATE as usize / block;
    let mut most = 0.0f32;
    for b in 0..blocks {
        let ph = b as f32 / blocks as f32;
        let w = |k: f32| 0.5 + 0.5 * (ph * k).sin();
        s.set_param(LENGTH, 20.0 + 60.0 * w(37.0));
        s.set_param(TENSION, 100.0 * w(23.0));
        s.set_param(DRIP, 100.0 * w(17.0));
        s.set_param(TONE, 1000.0 + 7000.0 * w(13.0));
        s.set_param(LOW_CUT, 20.0 + 980.0 * w(11.0));
        s.set_param(DECAY, 500.0 + 7500.0 * w(5.0));
        let mut l = noise(b as u64 + 1, block, 0.5);
        let mut r = noise(b as u64 + 9, block, 0.5);
        s.process(&Ctx::new(RATE, (b * block) as u64), &[], &mut l, &mut r);
        for v in l.iter().chain(&r) {
            assert!(v.is_finite());
            most = most.max(v.abs());
        }
    }
    assert!(most < 8.0, "peaked at {most}");
}

#[test]
fn the_output_is_the_same_at_any_block_size() {
    let n = RATE as usize / 2;
    let want = {
        let mut s = spring(RATE, &[(MIX, 60.0), (DRIP, 80.0)]);
        let (mut l, mut r) = (noise(5, n, 0.5), noise(6, n, 0.5));
        run_in(&mut s, RATE, MAX_BLOCK, &mut l, &mut r);
        (l, r)
    };
    for block in [1, 7, 31, 32, 33, 100, 333] {
        let block = block.min(MAX_BLOCK);
        let mut s = spring(RATE, &[(MIX, 60.0), (DRIP, 80.0)]);
        let (mut l, mut r) = (noise(5, n, 0.5), noise(6, n, 0.5));
        run_in(&mut s, RATE, block, &mut l, &mut r);
        for i in 0..n {
            assert_eq!(
                l[i].to_bits(),
                want.0[i].to_bits(),
                "block {block}, left at {i}"
            );
            assert_eq!(
                r[i].to_bits(),
                want.1[i].to_bits(),
                "block {block}, right at {i}"
            );
        }
    }
}
