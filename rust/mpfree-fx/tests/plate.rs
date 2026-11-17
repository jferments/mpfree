//! Plate reverb behaviour: its parameter ids hold (saved projects load),
//! Decay is the time to fall 60 dB at any rate and Size, Pre-delay moves
//! the whole wet signal by exactly its time, a source keeps its side, the
//! onset runs into the tail with no gap at any Size, the tail rings out to
//! silence, every control can sweep under noise without blowing up, and
//! the output is the same, bit for bit, at any block size. The node rules
//! (valid description, any block size, every extreme, no allocation) are
//! checked with the other effects in `fx.rs` and `no_alloc.rs`; the
//! measured targets (R1 to R12) in fxlab's quality ratchet.

use mpfree_fx::ids::plate::*;
use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::Plate;

const RATE: u32 = 48_000;

fn plate(rate: u32, values: &[(u16, f32)]) -> Plate {
    let mut p = Plate::new();
    p.prepare(rate, MAX_BLOCK);
    for &(id, v) in values {
        p.set_param(id, v);
    }
    p.reset();
    p
}

fn run_in(p: &mut Plate, rate: u32, block: usize, l: &mut [f32], r: &mut [f32]) {
    let mut at = 0;
    for (a, b) in l.chunks_mut(block).zip(r.chunks_mut(block)) {
        p.process(&Ctx::new(rate, at as u64), &[], a, b);
        at += a.len();
    }
}

fn run(p: &mut Plate, rate: u32, l: &mut [f32], r: &mut [f32]) {
    run_in(p, rate, 128, l, r);
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

/// The 60 dB decay time from the Schroeder curve's fall from -5 to -35 dB.
fn t60(l: &[f32], r: &[f32], rate: u32) -> f32 {
    let e: Vec<f64> = l
        .iter()
        .zip(r)
        .map(|(a, b)| (*a as f64).powi(2) + (*b as f64).powi(2))
        .collect();
    let mut tail = vec![0.0; e.len() + 1];
    for i in (0..e.len()).rev() {
        tail[i] = tail[i + 1] + e[i];
    }
    let level = |i: usize| 10.0 * (tail[i] / tail[0]).log10();
    let a = (0..e.len()).find(|&i| level(i) < -5.0).unwrap();
    let b = (0..e.len()).find(|&i| level(i) < -35.0).unwrap();
    2.0 * (b - a) as f32 / rate as f32
}

/// Wet only, flat (no damping, no tone filter, no low cut), no wobble.
const OPEN: [(u16, f32); 5] = [
    (MIX, 100.0),
    (DAMPING, 0.0),
    (TONE, 20_000.0),
    (LOW_CUT, 20.0),
    (MOD, 0.0),
];

fn impulse_response(rate: u32, values: &[(u16, f32)], secs: f32) -> (Vec<f32>, Vec<f32>) {
    let mut p = plate(rate, values);
    let n = (secs * rate as f32) as usize;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    l[0] = 1.0;
    r[0] = 1.0;
    run(&mut p, rate, &mut l, &mut r);
    (l, r)
}

#[test]
fn the_ids_saved_projects_use_hold() {
    let p = Plate::new();
    let names: Vec<(u16, &str)> = p.params().iter().map(|q| (q.id, q.name)).collect();
    let want = [
        (PREDELAY, "Pre-delay"),
        (SIZE, "Size"),
        (DECAY, "Decay"),
        (DAMPING, "Damping"),
        (TONE, "Tone"),
        (MOD, "Mod"),
        (LOW_CUT, "Low cut"),
        (WIDTH, "Width"),
        (MIX, "Mix"),
        (MIX_CURVE, "Mix curve"),
    ];
    for (id, name) in want {
        assert!(names.contains(&(id, name)), "{id} {name}: {names:?}");
    }
    assert_eq!(p.info().id, "mpfree.fx.plate");
}

#[test]
fn decay_is_the_time_to_fall_60_db_at_any_rate_and_size() {
    for rate in [44_100, 48_000, 96_000] {
        for (decay_ms, size) in [
            (500.0, 100.0),
            (2000.0, 100.0),
            (6000.0, 100.0),
            (1200.0, 50.0),
            (1200.0, 150.0),
        ] {
            let mut v = OPEN.to_vec();
            v.push((DECAY, decay_ms));
            v.push((SIZE, size));
            let (l, r) = impulse_response(rate, &v, decay_ms * 0.001 * 1.6 + 0.4);
            let got = t60(&l, &r, rate) * 1000.0;
            let err = (got - decay_ms) / decay_ms;
            assert!(
                err.abs() < 0.1,
                "{rate} Hz, {decay_ms} ms at Size {size}: {got:.0} ms"
            );
        }
    }
}

#[test]
fn pre_delay_moves_the_whole_wet_signal_by_its_time() {
    for ms in [40usize, 200] {
        let base = [(MIX, 100.0), (MOD, 0.0), (PREDELAY, 0.0)];
        let mut late = base.to_vec();
        late[2].1 = ms as f32;
        let (l0, r0) = impulse_response(RATE, &base, 0.6);
        let (l1, r1) = impulse_response(RATE, &late, 0.6);
        let shift = ms * RATE as usize / 1000;
        assert!(l1[..shift].iter().chain(&r1[..shift]).all(|v| *v == 0.0));
        for i in 0..l0.len() - shift {
            assert!((l1[i + shift] - l0[i]).abs() < 1e-6, "{ms}: left at {i}");
            assert!((r1[i + shift] - r0[i]).abs() < 1e-6, "{ms}: right at {i}");
        }
    }
}

#[test]
fn a_source_keeps_its_side() {
    let mut p = plate(RATE, &[(MIX, 100.0), (PREDELAY, 0.0)]);
    let n = RATE as usize / 20;
    let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
    l[0] = 1.0;
    run(&mut p, RATE, &mut l, &mut r);
    let side = 10.0 * (energy(&l) / energy(&r)).log10();
    assert!(
        side > 3.0,
        "the left's early energy is only {side:.1} dB stronger"
    );
}

#[test]
fn the_onset_runs_into_the_tail_with_no_gap_at_any_size() {
    // A plate has no walls: from the first ms to the tail, every 4 ms
    // holds at least the energy of the tail just after it, less 3 dB.
    for size in [50.0, 100.0, 150.0] {
        let mut v = OPEN.to_vec();
        v.extend([(SIZE, size), (DECAY, 2000.0), (PREDELAY, 0.0)]);
        let (l, r) = impulse_response(RATE, &v, 0.2);
        let bin = RATE as usize / 250;
        let db = |b: usize| {
            let s = b * bin..(b + 1) * bin;
            10.0 * (energy(&l[s.clone()]) + energy(&r[s])).log10()
        };
        let tail = (25..30).map(db).sum::<f64>() / 5.0;
        for b in 1..25 {
            assert!(
                db(b) > tail - 3.0,
                "Size {size}: {:.1} dB at {} ms, the tail after is {tail:.1}",
                db(b),
                4 * b
            );
        }
    }
}

#[test]
fn the_tail_rings_out_to_silence() {
    let mut p = plate(RATE, &[(MIX, 100.0), (DECAY, 1000.0), (MOD, 0.0)]);
    let n = RATE as usize;
    let mut l = noise(3, n, 0.5);
    let mut r = noise(4, n, 0.5);
    run(&mut p, RATE, &mut l, &mut r);
    let (mut l, mut r) = (vec![0.0; 8 * n], vec![0.0; 8 * n]);
    run(&mut p, RATE, &mut l, &mut r);
    assert!(l[7 * n..].iter().chain(&r[7 * n..]).all(|v| v.abs() < 1e-6));
    assert!(l.iter().chain(&r).all(|v| v.is_finite()));
    assert!(p.rest_frames() < u32::MAX, "no wobble: it can rest");
}

#[test]
fn every_control_sweeps_under_noise_without_blowing_up() {
    let mut p = plate(RATE, &[(MIX, 100.0), (DECAY, 18_000.0), (MOD, 100.0)]);
    let block = 128;
    let blocks = 3 * RATE as usize / block;
    let mut most = 0.0f32;
    for b in 0..blocks {
        let ph = b as f32 / blocks as f32;
        let s = |k: f32| 0.5 + 0.5 * (ph * k).sin();
        p.set_param(SIZE, 50.0 + 100.0 * s(37.0));
        p.set_param(DAMPING, 100.0 * s(23.0));
        p.set_param(TONE, 1000.0 + 19_000.0 * s(13.0));
        p.set_param(PREDELAY, 250.0 * s(7.0));
        p.set_param(DECAY, 200.0 + 19_800.0 * s(5.0));
        let mut l = noise(b as u64 + 1, block, 0.5);
        let mut r = noise(b as u64 + 9, block, 0.5);
        p.process(&Ctx::new(RATE, (b * block) as u64), &[], &mut l, &mut r);
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
        let mut p = plate(RATE, &[(MIX, 60.0), (PREDELAY, 12.0)]);
        let (mut l, mut r) = (noise(5, n, 0.5), noise(6, n, 0.5));
        run_in(&mut p, RATE, MAX_BLOCK, &mut l, &mut r);
        (l, r)
    };
    for block in [1, 7, 63, 64, 65, 100, 333] {
        let block = block.min(MAX_BLOCK);
        let mut p = plate(RATE, &[(MIX, 60.0), (PREDELAY, 12.0)]);
        let (mut l, mut r) = (noise(5, n, 0.5), noise(6, n, 0.5));
        run_in(&mut p, RATE, block, &mut l, &mut r);
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
