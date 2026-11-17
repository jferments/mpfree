//! Multi filter: with the drive off it is the linear filter, late by the
//! reported latency; a loud sound bends the resonance instead of clipping
//! while a quiet one still rings up; Resonance 0 never bends; a type or
//! slope change crossfades instead of clicking; and any block size sounds
//! the same.

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
            f.process(&Ctx::new(RATE, (i * block) as u64), &[], a, b);
        }
        outs.push((l, r));
    }
    for o in &outs[1..] {
        assert!(o.0 == outs[0].0 && o.1 == outs[0].1);
    }
}
