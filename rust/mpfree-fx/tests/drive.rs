//! Drive (FX quality round 2, plan-2 section 4): Bias acts in every mode
//! but Soft (G6); a change of Mode or Quality does not click (G7); quiet
//! signals pass flat (G8); and the antiderivative antialiasing holds still
//! when its input does, with no spike and nothing non-finite.

use std::f32::consts::PI;

use mpfree_fx::ids::drive::*;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::Drive;

const RATE: u32 = 48_000;

fn drive(values: &[(u16, f32)]) -> Drive {
    let mut d = Drive::new();
    d.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        d.set_param(id, v);
    }
    d.reset();
    d
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

/// `x` through `d` in 128-frame blocks, with `events` (frame, id, value).
fn run(d: &mut Drive, x: &[f32], events: &[(usize, u16, f32)]) -> Vec<f32> {
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut at = 0;
    while at < l.len() {
        let len = 128.min(l.len() - at);
        let ev: Vec<Timed> = events
            .iter()
            .filter(|(f, _, _)| (at..at + len).contains(f))
            .map(|&(f, id, value)| Timed::new((f - at) as u32, NodeEvent::Param { id, value }))
            .collect();
        d.process(
            &Ctx::new(RATE, at as u64),
            &ev,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    l
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
fn bias_acts_in_every_mode_but_soft() {
    // A 1 kHz sine at -12 dBFS at the default Drive: from Bias 0 to 100
    // the output changes by at least -20 dB of itself where Bias acts
    // (decision 10.1), and not at all in Soft (whose page dims Bias).
    let x = sine(1000.0, 0.25, RATE as usize / 2);
    for (mode, name) in [
        (0.0, "Soft"),
        (1.0, "Hard"),
        (2.0, "Tube"),
        (3.0, "Foldback"),
        (4.0, "Fuzz"),
        (5.0, "Rectify"),
    ] {
        let out = |bias: f32| {
            let mut d = drive(&[(MODE, mode), (BIAS, bias), (AUTO_GAIN, 0.0)]);
            run(&mut d, &x, &[])
        };
        let (a, b) = (out(0.0), out(100.0));
        let diff: Vec<f32> = a.iter().zip(&b).map(|(p, q)| p - q).collect();
        let change = db(rms(&diff[RATE as usize / 4..]) / rms(&a[RATE as usize / 4..]));
        if mode == 0.0 {
            assert!(change < -120.0, "{name}: Bias changed it by {change:.1} dB");
        } else {
            assert!(
                change > -20.0,
                "{name}: Bias changed it by only {change:.1} dB"
            );
        }
    }
}

#[test]
fn a_change_of_mode_or_quality_does_not_click() {
    // A 100 Hz sine at -12 dBFS; the sharpest corner around each change is
    // within 3 dB of the steady sound's either side (G7).
    let n = RATE as usize;
    let x = sine(100.0, 0.25, n);
    for (id, from, to, what) in [
        (MODE, 0.0, 3.0, "Soft to Foldback"),
        (MODE, 0.0, 4.0, "Soft to Fuzz"),
        (MODE, 2.0, 5.0, "Tube to Rectify"),
        (QUALITY, 0.0, 1.0, "Eco to HQ"),
        (QUALITY, 1.0, 0.0, "HQ to Eco"),
    ] {
        let mut d = drive(&[(id, from)]);
        let y = run(&mut d, &x, &[(n / 2, id, to)]);
        let steady = corner(&y[n / 4..n / 2 - 200]).max(corner(&y[n / 2 + 8000..3 * n / 4]));
        let at = corner(&y[n / 2 - 100..n / 2 + 4000]);
        let rise = db(at / steady);
        assert!(rise < 3.0, "{what}: {rise:.1} dB over the steady sound");
    }
}

#[test]
fn quiet_signals_pass_flat_in_both_qualities() {
    // At -40 dBFS the curves are straight lines: 50 Hz to 16 kHz within
    // 0.5 dB of 1 kHz in every mode but Rectify (which folds even a quiet
    // sine), at Mix 100 and at Mix 50 with no drive, where wet and dry
    // are the same level and must add in phase (G8). At Mix 50 the check
    // stops at 12 kHz: the Tone low-pass on the wet alone, at its top
    // (20 kHz), turns the wet 40 degrees by 16 kHz, so the sum dips
    // 0.64 dB there at 48 kHz (as before this round).
    for mode in [0.0, 1.0, 2.0, 3.0, 4.0] {
        for quality in [0.0, 1.0] {
            for mix in [100.0, 50.0] {
                let level = |hz: f32| {
                    let drive_db = if mix < 100.0 { 0.0 } else { 12.0 };
                    let mut d = drive(&[
                        (MODE, mode),
                        (QUALITY, quality),
                        (MIX, mix),
                        (DRIVE, drive_db),
                    ]);
                    let y = run(&mut d, &sine(hz, 0.01, RATE as usize / 2), &[]);
                    db(rms(&y[RATE as usize / 4..]))
                };
                let at1k = level(1000.0);
                let top = if mix < 100.0 { 12_000.0 } else { 16_000.0 };
                for hz in [50.0, 200.0, 5000.0, 12_000.0, 16_000.0] {
                    if hz > top {
                        continue;
                    }
                    let off = level(hz) - at1k;
                    assert!(
                        off.abs() < 0.5,
                        "mode {mode}, quality {quality}, Mix {mix}: {hz} Hz is {off:+.2} dB"
                    );
                }
            }
        }
    }
}

#[test]
fn a_held_input_holds_still() {
    // A held level, a held loud one and silence, in every mode and both
    // qualities: the antialiasing takes its midpoint where two inputs are
    // equal, so nothing spikes, nothing is non-finite, and once the
    // low-end filter has taken the step's DC away the output is silent.
    let mut x = vec![0.0f32; RATE as usize];
    for (i, v) in x.iter_mut().enumerate() {
        *v = match i * 4 / RATE as usize {
            0 => 0.3,
            1 => 4.0,
            _ => 0.0,
        };
    }
    for mode in 0..6 {
        for quality in [0.0, 1.0] {
            let mut d = drive(&[(MODE, mode as f32), (QUALITY, quality), (DRIVE, 36.0)]);
            let y = run(&mut d, &x, &[]);
            assert!(y.iter().all(|v| v.is_finite()), "mode {mode}");
            let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak < 8.0, "mode {mode}, quality {quality}: peak {peak}");
            let end = y[y.len() - 2000..]
                .iter()
                .fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(
                end < 1e-4,
                "mode {mode}, quality {quality}: {end} at the end"
            );
        }
    }
}
