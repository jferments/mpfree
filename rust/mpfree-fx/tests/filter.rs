//! Multi filter: with the drive off it is the linear filter, late by the
//! reported latency; a loud sound bends the resonance instead of clipping
//! while a quiet one still rings up; Resonance 0 never bends; Res mode
//! Hard clips it and Clean leaves it linear; a type, slope, model or Res
//! mode change crossfades instead of clicking; the ladder sings in tune on
//! its own and plays as drawn, on Hard too, and on Clean never sings; and
//! any block size sounds the same.

use std::f32::consts::PI;

use mpfree_fx::ids::filter::*;
use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::{MultiFilter, Svf, SvfMode};

const RATE: u32 = 48_000;

fn filter(values: &[(u16, f32)]) -> MultiFilter {
    let mut f = MultiFilter::new();
    f.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        f.set_param(id, v);
    }
    f.reset();
    f
}

fn run(f: &mut MultiFilter, l: &mut [f32], r: &mut [f32]) {
    for (i, (a, b)) in l.chunks_mut(256).zip(r.chunks_mut(256)).enumerate() {
        f.process(&Ctx::new(RATE, (i * 256) as u64), &[], a, b);
    }
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}

/// The noise of a little LCG, white, in -1 to 1.
fn noise(n: usize, amp: f32) -> Vec<f32> {
    let mut s = 0x1234_5678u32;
    (0..n)
        .map(|_| {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            amp * ((s >> 8) as f32 / 8_388_608.0 - 1.0)
        })
        .collect()
}

#[test]
fn with_the_drive_off_it_is_the_linear_filter_late_by_the_latency() {
    let f0 = filter(&[]);
    let lat = f0.latency() as usize;
    assert_eq!(lat, 4);
    // Resonance 0 never bends, even at full scale.
    for (ty, slope, res) in [
        (0.0, 0.0, 0.0),
        (1.0, 1.0, 0.0),
        (3.0, 1.0, 80.0),
        (4.0, 0.0, 90.0),
    ] {
        let mut f = filter(&[
            (TYPE, ty),
            (SLOPE, slope),
            (CUTOFF, 1_000.0),
            (RESONANCE, res),
            (GAIN, 6.0),
        ]);
        let x = noise(RATE as usize / 2, 1.0);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        // The same filter, linear, on its own.
        let mode = [
            SvfMode::LowPass,
            SvfMode::HighPass,
            SvfMode::BandPass,
            SvfMode::Notch,
            SvfMode::Bell,
        ][ty as usize];
        let q = 0.707 * (40.0f32 / 0.707).powf(res / 100.0);
        let (mut a, mut b) = (Svf::default(), Svf::default());
        let cascaded = slope > 0.5 && ty < 4.0;
        if cascaded {
            let (q1, q2) = if ty < 2.0 {
                (0.541, 1.307 * (q / 0.707))
            } else {
                (q, q)
            };
            a.set(mode, 1_000.0, q1, 6.0, RATE);
            b.set(mode, 1_000.0, q2, 6.0, RATE);
        } else {
            a.set(mode, 1_000.0, q, 6.0, RATE);
        }
        let mut worst = 0.0f32;
        for i in lat..x.len() {
            let mut y = a.run(0, x[i - lat]);
            if cascaded {
                y = b.run(0, y);
            }
            worst = worst.max((y - l[i]).abs());
        }
        // Notch and bell never bend; at Resonance 0 nothing does.
        assert!(worst < 1e-6, "type {ty} slope {slope} res {res}: {worst}");
        assert!(l[..lat].iter().all(|&v| v == 0.0));
    }
}

#[test]
fn a_loud_sound_bends_the_resonance_and_a_quiet_one_rings_up() {
    let n = RATE as usize / 2;
    for slope in [0.0, 1.0] {
        let values = [(SLOPE, slope), (CUTOFF, 1_000.0), (RESONANCE, 100.0)];
        // -40 dBFS at the cutoff: at least +28 dB.
        let mut f = filter(&values);
        let x = sine(1_000.0, 0.01, n);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let gain = 20.0 * (peak(&l[n / 2..]) / 0.01).log10();
        assert!(gain >= 28.0, "slope {slope}: quiet gain {gain:.1} dB");
        // -6 dBFS at the cutoff, where a linear filter would put out +26
        // dBFS: under full scale.
        let mut f = filter(&values);
        let x = sine(1_000.0, 0.5, n);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let top = peak(&l);
        assert!(top < 1.0 && top > 0.3, "slope {slope}: loud peak {top}");
    }
}

/// The largest step from one sample to the next.
fn worst_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

#[test]
fn a_type_or_slope_change_crossfades_instead_of_clicking() {
    // A 100 Hz tone under a 1 kHz low-pass, switched to a high-pass (24 dB)
    // or from 12 to 24 dB: no step bigger than the tone's own.
    let n = RATE as usize / 2;
    let x = sine(100.0, 0.5, n);
    let tone = worst_step(&x);
    for (from, to) in [((TYPE, 0.0), (TYPE, 1.0)), ((SLOPE, 0.0), (SLOPE, 1.0))] {
        let mut f = filter(&[(SLOPE, 1.0), (CUTOFF, 1_000.0), from]);
        let (mut l, mut r) = (x.clone(), x.clone());
        let at = n / 2 + 37;
        for (i, (a, b)) in l.chunks_mut(64).zip(r.chunks_mut(64)).enumerate() {
            if i * 64 <= at && at < (i + 1) * 64 {
                f.set_param(to.0, to.1);
            }
            f.process(&Ctx::new(RATE, (i * 64) as u64), &[], a, b);
        }
        let step = worst_step(&l[n / 4..]);
        assert!(
            step <= 1.05 * tone,
            "{from:?} to {to:?}: step {step}, the tone's {tone}"
        );
    }
}

#[test]
fn any_block_size_sounds_the_same() {
    let values = [(CUTOFF, 700.0), (RESONANCE, 85.0), (DRIVE, 9.0)];
    let n = RATE as usize;
    let x = noise(n, 0.7);
    let mut outs = Vec::new();
    for block in [1usize, 37, 256] {
        let mut f = filter(&values);
        let (mut l, mut r) = (x.clone(), x.clone());
        for (i, (a, b)) in l.chunks_mut(block).zip(r.chunks_mut(block)).enumerate() {
            // A frame every block size starts a block on (37 x 256 x 2).
            if i * block == 18_944 {
                f.set_param(TYPE, 2.0);
                f.set_param(SLOPE, 1.0);
                f.set_param(DRIVE, 0.0);
            }
            // Hard, then Clean (37 x 256 x 3).
            if i * block == 18_944 {
                f.set_param(RES_MODE, 1.0);
            }
            if i * block == 28_416 {
                f.set_param(RES_MODE, 2.0);
            }
            // And the ladder, driven (37 x 256 x 4), then Hard (x 5).
            if i * block == 37_888 {
                f.set_param(MODEL, 1.0);
                f.set_param(DRIVE, 6.0);
            }
            if i * block == 47_360 {
                f.set_param(RES_MODE, 1.0);
            }
            f.process(&Ctx::new(RATE, (i * block) as u64), &[], a, b);
        }
        outs.push((l, r));
    }
    for o in &outs[1..] {
        assert!(o.0 == outs[0].0 && o.1 == outs[0].1);
    }
}

/// The pitch of `x` by its rising zero crossings, Hz, and its RMS level.
fn pitch_and_level(x: &[f32]) -> (f64, f64) {
    let mut first = None;
    let mut last = 0.0f64;
    let mut count = 0usize;
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            let t = (i - 1) as f64 + f64::from(-x[i - 1]) / f64::from(x[i] - x[i - 1]);
            if first.is_none() {
                first = Some(t);
            } else {
                count += 1;
            }
            last = t;
        }
    }
    let hz = count as f64 * f64::from(RATE) / (last - first.unwrap());
    let rms = (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt();
    (hz, rms)
}

#[test]
fn the_ladder_sings_in_tune_on_its_own() {
    // A click, then silence: at full Resonance the ladder rings up into a
    // steady oscillation at the cutoff.
    let n = 4 * RATE as usize;
    let mut levels = Vec::new();
    for hz in [100.0f32, 300.0, 1_000.0, 3_000.0, 5_000.0] {
        let mut f = filter(&[(MODEL, 1.0), (CUTOFF, hz), (RESONANCE, 100.0)]);
        let mut l = vec![0.0f32; n];
        l[0] = 0.1;
        let mut r = l.clone();
        run(&mut f, &mut l, &mut r);
        let (got, rms) = pitch_and_level(&l[3 * RATE as usize..]);
        let cents = 1200.0 * (got / f64::from(hz)).log2();
        assert!(
            cents.abs() < 5.0,
            "{hz} Hz: sings at {got:.2} ({cents:.2} cents)"
        );
        assert!(l == r);
        levels.push(20.0 * rms.log10());
    }
    let (lo, hi) = levels
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    assert!(hi - lo < 1.0, "levels {levels:?}");
    // About -12 dBFS RMS: loud enough to play, under full scale.
    assert!(hi < -6.0 && lo > -20.0, "levels {levels:?}");
}

#[test]
fn the_ladder_plays_as_drawn() {
    // Quiet sines (the tanh is straight there) against the drawing.
    for res in [0.0, 50.0, 85.0] {
        let f0 = filter(&[(MODEL, 1.0), (CUTOFF, 1_000.0), (RESONANCE, res)]);
        for hz in [100.0f32, 500.0, 900.0, 1_000.0, 1_300.0, 3_000.0, 5_000.0] {
            let mut f = filter(&[(MODEL, 1.0), (CUTOFF, 1_000.0), (RESONANCE, res)]);
            let n = RATE as usize;
            let x = sine(hz, 0.001, n);
            let (mut l, mut r) = (x.clone(), x.clone());
            run(&mut f, &mut l, &mut r);
            let heard = 20.0 * (peak(&l[n / 2..]) / 0.001).log10();
            let drawn = f0.response_db(hz);
            if drawn < -40.0 {
                // Far down the slope, a -60 dBFS sine is lost in rounding.
                continue;
            }
            assert!(
                (heard - drawn).abs() < 0.2,
                "res {res} at {hz} Hz: heard {heard:.2}, drawn {drawn:.2}"
            );
        }
    }
}

#[test]
fn the_ladder_holds_a_loud_sound_and_rings_up_a_quiet_one() {
    let n = RATE as usize / 2;
    let values = [(MODEL, 1.0), (CUTOFF, 1_000.0), (RESONANCE, 90.0)];
    let mut f = filter(&values);
    let x = sine(1_000.0, 0.01, n);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut f, &mut l, &mut r);
    let gain = 20.0 * (peak(&l[n / 2..]) / 0.01).log10();
    assert!(gain >= 20.0, "quiet gain {gain:.1} dB");
    let mut f = filter(&values);
    let x = sine(1_000.0, 0.5, n);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut f, &mut l, &mut r);
    let top = peak(&l);
    assert!(top < 1.0 && top > 0.2, "loud peak {top}");
}

#[test]
fn a_model_change_crossfades_instead_of_clicking() {
    // A 100 Hz tone under a 1 kHz low-pass, Clean to Ladder and back, and
    // back again within the fade.
    let n = RATE as usize / 2;
    let x = sine(100.0, 0.5, n);
    let tone = worst_step(&x);
    let mut f = filter(&[(CUTOFF, 1_000.0), (RESONANCE, 40.0)]);
    let (mut l, mut r) = (x.clone(), x.clone());
    let changes = [(n / 4 + 11, 1.0), (n / 2 + 5, 0.0), (n / 2 + 200, 1.0)];
    for (i, (a, b)) in l.chunks_mut(64).zip(r.chunks_mut(64)).enumerate() {
        for &(at, v) in &changes {
            if i * 64 <= at && at < (i + 1) * 64 {
                f.set_param(MODEL, v);
            }
        }
        f.process(&Ctx::new(RATE, (i * 64) as u64), &[], a, b);
    }
    let step = worst_step(&l[n / 8..]);
    assert!(step <= 1.05 * tone, "step {step}, the tone's {tone}");
}

#[test]
fn res_mode_clean_is_the_linear_filter_at_any_level() {
    // Full-scale noise through a full-Resonance low-pass and a 24 dB
    // band-pass, which Soft would bend: Clean is the linear filter exactly.
    for (ty, slope) in [(0.0, 0.0), (2.0, 1.0)] {
        let mut f = filter(&[
            (TYPE, ty),
            (SLOPE, slope),
            (CUTOFF, 1_000.0),
            (RESONANCE, 100.0),
            (RES_MODE, 2.0),
        ]);
        let lat = f.latency() as usize;
        let x = noise(RATE as usize / 2, 1.0);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let mode = [SvfMode::LowPass, SvfMode::HighPass, SvfMode::BandPass][ty as usize];
        let q = 0.707 * (40.0f32 / 0.707).powf(1.0);
        let (mut a, mut b) = (Svf::default(), Svf::default());
        a.set(mode, 1_000.0, q, 0.0, RATE);
        b.set(mode, 1_000.0, q, 0.0, RATE);
        let mut worst = 0.0f32;
        let mut top = 0.0f32;
        for i in lat..x.len() {
            let mut y = a.run(0, x[i - lat]);
            if slope > 0.5 {
                y = b.run(0, y);
            }
            worst = worst.max((y - l[i]).abs());
            top = top.max(y.abs());
        }
        assert!(top > 2.0, "type {ty}: the linear filter peaks at {top}");
        // Within rounding of the cutoff (set through its log), which a
        // ring this long magnifies.
        assert!(
            worst < 1e-4 * top,
            "type {ty} slope {slope}: {worst} of {top}"
        );
    }
}

#[test]
fn res_mode_hard_clips_a_loud_sound_and_rings_up_a_quiet_one() {
    let n = RATE as usize / 2;
    for slope in [0.0, 1.0] {
        let values = [
            (SLOPE, slope),
            (CUTOFF, 1_000.0),
            (RESONANCE, 100.0),
            (RES_MODE, 1.0),
        ];
        let mut f = filter(&values);
        let x = sine(1_000.0, 0.01, n);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let gain = 20.0 * (peak(&l[n / 2..]) / 0.01).log10();
        assert!(gain >= 28.0, "slope {slope}: quiet gain {gain:.1} dB");
        let mut f = filter(&values);
        let x = sine(1_000.0, 0.5, n);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut f, &mut l, &mut r);
        let top = peak(&l);
        assert!(top < 1.0 && top > 0.3, "slope {slope}: loud peak {top}");
    }
}

#[test]
fn a_res_mode_change_crossfades_instead_of_clicking() {
    // A 100 Hz tone under a 1 kHz low-pass, through every Res mode on both
    // models, and back within a fade.
    let n = RATE as usize / 2;
    let x = sine(100.0, 0.5, n);
    let tone = worst_step(&x);
    for model in [0.0, 1.0] {
        let mut f = filter(&[(MODEL, model), (CUTOFF, 1_000.0), (RESONANCE, 40.0)]);
        let (mut l, mut r) = (x.clone(), x.clone());
        let changes = [
            (n / 8 + 3, 1.0),
            (n / 4 + 11, 2.0),
            (3 * n / 8, 0.0),
            (n / 2 + 5, 1.0),
            (n / 2 + 200, 0.0),
            (5 * n / 8, 2.0),
            (5 * n / 8 + 150, 1.0),
        ];
        for (i, (a, b)) in l.chunks_mut(64).zip(r.chunks_mut(64)).enumerate() {
            for &(at, v) in &changes {
                if i * 64 <= at && at < (i + 1) * 64 {
                    f.set_param(RES_MODE, v);
                }
            }
            f.process(&Ctx::new(RATE, (i * 64) as u64), &[], a, b);
        }
        let step = worst_step(&l[n / 16..]);
        assert!(
            step <= 1.05 * tone,
            "model {model}: step {step}, the tone's {tone}"
        );
    }
}

#[test]
fn the_ladder_sings_on_hard_and_rings_out_on_clean() {
    let n = 4 * RATE as usize;
    let click = |mode: f32| {
        let mut f = filter(&[
            (MODEL, 1.0),
            (CUTOFF, 1_000.0),
            (RESONANCE, 100.0),
            (RES_MODE, mode),
        ]);
        let mut l = vec![0.0f32; n];
        l[0] = 0.1;
        let mut r = l.clone();
        run(&mut f, &mut l, &mut r);
        l
    };
    let l = click(1.0);
    let (got, rms) = pitch_and_level(&l[3 * RATE as usize..]);
    let cents = 1200.0 * (got / 1_000.0).log2();
    assert!(
        cents.abs() < 5.0,
        "Hard sings at {got:.2} ({cents:.2} cents)"
    );
    let db = 20.0 * rms.log10();
    assert!(db < -6.0 && db > -20.0, "Hard sings at {db:.1} dBFS");
    // Clean rings long, then is gone.
    let l = click(2.0);
    let ring = peak(&l[RATE as usize / 100..RATE as usize / 20]);
    let left = peak(&l[3 * RATE as usize..]);
    assert!(
        ring > 1e-3 && left < 1e-6,
        "Clean rings {ring}, then {left}"
    );
}
