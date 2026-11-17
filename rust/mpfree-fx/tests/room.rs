//! Room reverb behaviour: its parameter ids hold (saved projects load),
//! Decay is the time to fall 60 dB at any rate, Pre-delay moves the whole
//! wet signal by exactly its time, a source keeps its side, the tail rings
//! out to silence, Size and Damping can sweep under noise without blowing
//! up, and the output is the same, bit for bit, at any block size (the
//! Room works in chunks of its own, on a grid of its own). The node rules
//! (valid description, any block size, every extreme, no allocation) are
//! checked with the other effects in `fx.rs` and `no_alloc.rs`; the
//! measured targets (R1 to R12) in fxlab's quality ratchet.

use mpfree_fx::ids::room::*;
use mpfree_fx::mpfree_node::{Ctx, Node, MAX_BLOCK};
use mpfree_fx::Room;

const RATE: u32 = 48_000;

fn room(rate: u32, values: &[(u16, f32)]) -> Room {
    let mut r = Room::new();
    r.prepare(rate, MAX_BLOCK);
    for &(id, v) in values {
        r.set_param(id, v);
    }
    r.reset();
    r
}

fn run_in(r: &mut Room, rate: u32, block: usize, l: &mut [f32], rr: &mut [f32]) {
    let mut at = 0;
    for (a, b) in l.chunks_mut(block).zip(rr.chunks_mut(block)) {
        r.process(&Ctx::new(rate, at as u64), &[], a, b);
        at += a.len();
    }
}

fn run(r: &mut Room, rate: u32, l: &mut [f32], rr: &mut [f32]) {
    run_in(r, rate, 128, l, rr);
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

/// Wet only, flat (no damping, no low cut), no wobble.
const OPEN: [(u16, f32); 5] = [
    (MIX, 100.0),
    (DAMPING, 0.0),
    (LOW_CUT, 20.0),
    (MOD, 0.0),
    (EARLY, 0.0),
];

fn impulse_response(rate: u32, values: &[(u16, f32)], secs: f32) -> (Vec<f32>, Vec<f32>) {
    let mut r = room(rate, values);
    let n = (secs * rate as f32) as usize;
    let (mut l, mut rr) = (vec![0.0; n], vec![0.0; n]);
    l[0] = 1.0;
    rr[0] = 1.0;
    run(&mut r, rate, &mut l, &mut rr);
    (l, rr)
}

#[test]
fn the_ids_saved_projects_use_hold() {
    let r = Room::new();
    let names: Vec<(u16, &str)> = r.params().iter().map(|p| (p.id, p.name)).collect();
    let want = [
        (PREDELAY, "Pre-delay"),
        (SIZE, "Size"),
        (DECAY, "Decay"),
        (DAMPING, "Damping"),
        (EARLY, "Early"),
        (MOD, "Mod"),
        (GATE, "Gate"),
        (LOW_CUT, "Low cut"),
        (WIDTH, "Width"),
        (MIX, "Mix"),
        (MIX_CURVE, "Mix curve"),
        (DIFFUSION, "Diffusion"),
    ];
    for (id, name) in want {
        assert!(names.contains(&(id, name)), "{id} {name}: {names:?}");
    }
    assert_eq!(r.info().id, "mpfree.fx.room");
}

#[test]
fn decay_is_the_time_to_fall_60_db_at_any_rate() {
    for rate in [44_100, 48_000, 96_000] {
        for (decay_ms, size) in [(500.0, 45.0), (1500.0, 45.0), (4000.0, 80.0), (800.0, 10.0)] {
            let mut v = OPEN.to_vec();
            v.push((DECAY, decay_ms));
            v.push((SIZE, size));
            let (l, r) = impulse_response(rate, &v, decay_ms * 0.001 * 1.6 + 0.3);
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
    let base = [(MIX, 100.0), (EARLY, 60.0), (MOD, 0.0), (PREDELAY, 0.0)];
    let mut late = base.to_vec();
    late[3].1 = 40.0;
    let (l0, r0) = impulse_response(RATE, &base, 0.5);
    let (l1, r1) = impulse_response(RATE, &late, 0.5);
    let shift = 40 * RATE as usize / 1000;
    assert!(l1[..shift].iter().chain(&r1[..shift]).all(|v| *v == 0.0));
    for i in 0..l0.len() - shift {
        assert!((l1[i + shift] - l0[i]).abs() < 1e-6, "left at {i}");
        assert!((r1[i + shift] - r0[i]).abs() < 1e-6, "right at {i}");
    }
}

#[test]
fn a_source_keeps_its_side() {
    let mut r = room(RATE, &[(MIX, 100.0)]);
    let n = RATE as usize / 20;
    let (mut l, mut rr) = (vec![0.0; n], vec![0.0; n]);
    l[0] = 1.0;
    run(&mut r, RATE, &mut l, &mut rr);
    let side = 10.0 * (energy(&l) / energy(&rr)).log10();
    assert!(
        side > 3.0,
        "the left's early energy is only {side:.1} dB stronger"
    );
}

#[test]
fn the_tail_rings_out_to_silence() {
    let mut r = room(RATE, &[(MIX, 100.0), (DECAY, 1000.0), (MOD, 0.0)]);
    let n = RATE as usize;
    let mut l = noise(3, n, 0.5);
    let mut rr = noise(4, n, 0.5);
    run(&mut r, RATE, &mut l, &mut rr);
    let (mut l, mut rr) = (vec![0.0; 8 * n], vec![0.0; 8 * n]);
    run(&mut r, RATE, &mut l, &mut rr);
    let last = &l[7 * n..];
    assert!(last.iter().chain(&rr[7 * n..]).all(|v| v.abs() < 1e-6));
    assert!(l.iter().chain(&rr).all(|v| v.is_finite()));
    assert!(
        r.rest_frames() < u32::MAX,
        "no wobble, no gate: it can rest"
    );
}

#[test]
fn size_damping_and_diffusion_sweep_under_noise_without_blowing_up() {
    let mut r = room(RATE, &[(MIX, 100.0), (DECAY, 8000.0)]);
    let block = 128;
    let blocks = 3 * RATE as usize / block;
    let mut most = 0.0f32;
    for b in 0..blocks {
        let ph = b as f32 / blocks as f32;
        r.set_param(SIZE, 100.0 * (0.5 + 0.5 * (ph * 37.0).sin()));
        r.set_param(DAMPING, 100.0 * (0.5 + 0.5 * (ph * 23.0).cos()));
        r.set_param(DIFFUSION, 100.0 * (0.5 + 0.5 * (ph * 11.0).sin()));
        r.set_param(DECAY, 200.0 + 9000.0 * (0.5 + 0.5 * (ph * 5.0).sin()));
        let mut l = noise(b as u64 + 1, block, 0.5);
        let mut rr = noise(b as u64 + 9, block, 0.5);
        r.process(&Ctx::new(RATE, (b * block) as u64), &[], &mut l, &mut rr);
        for v in l.iter().chain(&rr) {
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
        let mut r = room(RATE, &[(MIX, 60.0), (PREDELAY, 12.0)]);
        let (mut l, mut rr) = (noise(5, n, 0.5), noise(6, n, 0.5));
        run_in(&mut r, RATE, MAX_BLOCK, &mut l, &mut rr);
        (l, rr)
    };
    for block in [1, 7, 63, 64, 65, 100, 333] {
        let block = block.min(MAX_BLOCK);
        let mut r = room(RATE, &[(MIX, 60.0), (PREDELAY, 12.0)]);
        let (mut l, mut rr) = (noise(5, n, 0.5), noise(6, n, 0.5));
        run_in(&mut r, RATE, block, &mut l, &mut rr);
        for i in 0..n {
            assert_eq!(
                l[i].to_bits(),
                want.0[i].to_bits(),
                "block {block}, left at {i}"
            );
            assert_eq!(
                rr[i].to_bits(),
                want.1[i].to_bits(),
                "block {block}, right at {i}"
            );
        }
    }
}
