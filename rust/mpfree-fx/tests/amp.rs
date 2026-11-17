//! Amp (FX quality round 2, plan-2 section 12): silence stays silent with
//! every model, boost and the gate (A9); Model, Cab and Boost changes and
//! thrown tone knobs don't click (A6, A8); the gate shuts hiss, opens at
//! once and shuts once on a decaying note (A9); Sag compresses and
//! recovers; Depth lifts the lows; and any block size sounds the same
//! while the knobs glide.

use std::f32::consts::PI;

use mpfree_fx::ids::amp::*;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::Amp;

const RATE: u32 = 48_000;

fn amp(values: &[(u16, f32)]) -> Amp {
    let mut a = Amp::new();
    a.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        a.set_param(id, v);
    }
    a.reset();
    a
}

fn sine(hz: f32, level: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| level * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

/// White noise, a fixed sequence.
fn hiss(level: f32, frames: usize) -> Vec<f32> {
    let mut s = 0x2545_f491_4f6c_dd1du64;
    (0..frames)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            level * ((s >> 40) as f32 / (1u64 << 23) as f32 - 1.0)
        })
        .collect()
}

/// `x` through `a` in blocks of `block` frames, with `events` (frame, id,
/// value).
fn run_in(a: &mut Amp, x: &[f32], events: &[(usize, u16, f32)], block: usize) -> Vec<f32> {
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut at = 0;
    while at < l.len() {
        let len = block.min(l.len() - at);
        let ev: Vec<Timed> = events
            .iter()
            .filter(|(f, _, _)| (at..at + len).contains(f))
            .map(|&(f, id, value)| Timed::new((f - at) as u32, NodeEvent::Param { id, value }))
            .collect();
        a.process(
            &Ctx::new(RATE, at as u64),
            &ev,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    assert_eq!(l, r, "the amp is mono");
    l
}

fn run(a: &mut Amp, x: &[f32], events: &[(usize, u16, f32)]) -> Vec<f32> {
    run_in(a, x, events, 128)
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}

/// The largest second difference in `x`: a click's sharpest corner.
fn corner(x: &[f32]) -> f32 {
    x.windows(3)
        .map(|w| (w[2] - 2.0 * w[1] + w[0]).abs())
        .fold(0.0f32, f32::max)
}

#[test]
fn silence_stays_silent_with_every_model_boost_and_the_gate() {
    let x = vec![0.0f32; RATE as usize / 2];
    for model in 0..4 {
        for boost in 0..3 {
            for gate in [0.0, 8.0] {
                let mut a = amp(&[
                    (MODEL, model as f32),
                    (GAIN, 10.0),
                    (BOOST, boost as f32),
                    (GATE, gate),
                    (SAG, 10.0),
                    (DEPTH, 10.0),
                ]);
                let y = run(&mut a, &x, &[]);
                let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                assert!(
                    peak < 1e-6,
                    "model {model} boost {boost} gate {gate}: {peak}"
                );
            }
        }
    }
}

#[test]
fn changes_and_thrown_knobs_dont_click() {
    // A 220 Hz sine at -12 dBFS; halfway through, one control changes.
    // The sharpest corner around the change stays within 3 dB of the
    // steady sound's, before or after (A6, A8).
    let n = RATE as usize;
    let x = sine(220.0, 0.25, n);
    let half = n / 2;
    for model in 0..4 {
        let to_model = if model == 2 { 1.0 } else { 2.0 };
        for (id, from, to) in [
            (MODEL, model as f32, to_model),
            (CAB, 3.0, 0.0),
            (CAB, 0.0, 4.0),
            (CAB, 1.0, 3.0),
            (BOOST, 0.0, 1.0),
            (BOOST, 1.0, 2.0),
            (BOOST, 2.0, 0.0),
            (BASS, 0.0, 10.0),
            (MID, 0.0, 10.0),
            (TREBLE, 0.0, 10.0),
            (TREBLE, 10.0, 0.0),
            (DEPTH, 0.0, 10.0),
        ] {
            let mut a = amp(&[(MODEL, model as f32), (id, from)]);
            let y = run(&mut a, &x, &[(half, id, to)]);
            let steady = corner(&y[n / 4..half - 10]).max(corner(&y[half + 9600..n]));
            let at = corner(&y[half - 10..half + 9600]);
            assert!(
                db(at / steady) <= 3.0,
                "model {model}: {id} {from} to {to}: {:.1} dB",
                db(at / steady)
            );
        }
    }
}

#[test]
fn the_gate_shuts_hiss_opens_at_once_and_shuts_once() {
    // Gate 8: a -45 dBFS threshold.
    let n = RATE as usize;
    // A note, then hiss at about -60 dBFS RMS: the rest comes out silent
    // even at Gain 10 on the lead model.
    let mut x = sine(220.0, 0.25, n / 2);
    x.extend(hiss(0.0017, 3 * n / 2));
    let mut a = amp(&[(MODEL, 2.0), (GAIN, 10.0), (GATE, 8.0)]);
    let y = run(&mut a, &x, &[]);
    assert!(rms(&y[n / 4..n / 2]) > 0.01, "the note plays");
    assert!(
        db(rms(&y[3 * n / 2..])) < -90.0,
        "{}",
        db(rms(&y[3 * n / 2..]))
    );

    // A note at -24 dBFS after silence: from 2 ms on the gated output is
    // the ungated one within 1 % (at Gain 0, where the amp is nearly
    // straight; what is left is the filters' memory of the half
    // millisecond the gate took to open).
    let mut x = vec![0.0f32; n / 2];
    x.extend(sine(220.0, 0.063, n / 2));
    let mut g = amp(&[(MODEL, 2.0), (GAIN, 0.0), (GATE, 8.0)]);
    let mut o = amp(&[(MODEL, 2.0), (GAIN, 0.0)]);
    let (yg, yo) = (run(&mut g, &x, &[]), run(&mut o, &x, &[]));
    let after = n / 2 + (0.002 * RATE as f32) as usize;
    let diff: Vec<f32> = yg[after..]
        .iter()
        .zip(&yo[after..])
        .map(|(a, b)| a - b)
        .collect();
    assert!(
        db(rms(&diff) / rms(&yo[after..])) < -40.0,
        "{}",
        db(rms(&diff) / rms(&yo[after..]))
    );

    // A note decaying 40 dB a second through the threshold: the gate's
    // gain (gated over ungated, 5 ms at a time) falls under a half once.
    let x: Vec<f32> = sine(220.0, 0.25, 2 * n)
        .iter()
        .enumerate()
        .map(|(i, v)| v * 10f32.powf(-2.0 * i as f32 / RATE as f32))
        .collect();
    let (yg, yo) = (run(&mut g, &x, &[]), run(&mut o, &x, &[]));
    let w = (0.005 * RATE as f32) as usize;
    let mut shut = 0;
    let mut open = false;
    for (a, b) in yg.chunks(w).zip(yo.chunks(w)) {
        let r = rms(a) / rms(b).max(1e-12);
        if r > 0.9 {
            open = true;
        } else if r < 0.5 && open {
            shut += 1;
            open = false;
        }
    }
    assert_eq!(shut, 1);
}

#[test]
fn sag_compresses_a_loud_chord_and_recovers() {
    // A loud low chord through Crunch at Gain 5: with Sag 10 the held
    // chord comes out at least 1 dB quieter than with none; once the
    // chord stops and a quiet note follows, the two match again.
    let n = RATE as usize;
    let mut x: Vec<f32> = sine(110.0, 0.35, n)
        .iter()
        .zip(sine(165.0, 0.3, n))
        .map(|(a, b)| a + b)
        .collect();
    x.extend(vec![0.0; n]);
    x.extend(sine(440.0, 0.01, n / 2));
    let mut flat = amp(&[(MODEL, 1.0)]);
    let mut sag = amp(&[(MODEL, 1.0), (SAG, 10.0)]);
    let (a, b) = (run(&mut flat, &x, &[]), run(&mut sag, &x, &[]));
    let held = n / 2..n;
    let drop = db(rms(&a[held.clone()]) / rms(&b[held]));
    assert!(drop > 1.0, "{drop}");
    let quiet = 2 * n + n / 4..2 * n + n / 2;
    let left = db(rms(&a[quiet.clone()]) / rms(&b[quiet]));
    assert!(left.abs() < 0.1, "{left}");
}

#[test]
fn depth_lifts_the_lows() {
    // A quiet 80 Hz sine through Bass at Gain 0 with the cab off: Depth 10
    // lifts it at least 6 dB; a 1 kHz sine moves under 1 dB.
    let n = RATE as usize / 2;
    for (hz, lo, hi) in [(80.0, 6.0, 12.0), (1000.0, -1.0, 1.0)] {
        let x = sine(hz, 0.01, n);
        let mut a = amp(&[(MODEL, 3.0), (GAIN, 0.0), (CAB, 0.0)]);
        let mut d = amp(&[(MODEL, 3.0), (GAIN, 0.0), (CAB, 0.0), (DEPTH, 10.0)]);
        let (ya, yd) = (run(&mut a, &x, &[]), run(&mut d, &x, &[]));
        let lift = db(rms(&yd[n / 2..]) / rms(&ya[n / 2..]));
        assert!((lo..=hi).contains(&lift), "{hz} Hz: {lift}");
    }
}

#[test]
fn any_block_size_sounds_the_same_while_knobs_glide() {
    let n = RATE as usize / 2;
    let x = sine(220.0, 0.25, n);
    let events = [
        (1000, TREBLE, 9.0),
        (3000, GAIN, 9.0),
        (5000, MODEL, 2.0),
        (7000, CAB, 1.0),
        (9000, BOOST, 1.0),
        (11000, BASS, 1.0),
    ];
    let mut a = amp(&[]);
    let mut b = amp(&[]);
    let ya = run_in(&mut a, &x, &events, 128);
    let yb = run_in(&mut b, &x, &events, 37);
    assert_eq!(ya, yb);
}
