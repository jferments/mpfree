//! The Character effect: quiet signals keep their level in every mode;
//! Console holds loud peaks with odd harmonics only and Iron adds even
//! ones; Tape lifts its head bump and loses a little top; Soften takes the
//! edge off loud highs and leaves quiet ones and lows alone; Mix 0 is the
//! input exactly and Output scales; a change of mode does not click.

use std::f32::consts::PI;

use mpfree_fx::ids::character::*;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed, MAX_BLOCK};
use mpfree_fx::Character;

const RATE: u32 = 48_000;

fn character(values: &[(u16, f32)]) -> Character {
    let mut c = Character::new();
    c.prepare(RATE, MAX_BLOCK);
    for &(id, v) in values {
        c.set_param(id, v);
    }
    c.reset();
    c
}

fn sine(hz: f32, amp: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (2.0 * PI * hz * i as f32 / RATE as f32).sin())
        .collect()
}

fn run(c: &mut Character, l: &mut [f32], r: &mut [f32], events: &[(usize, u16, f32)]) {
    let mut at = 0;
    while at < l.len() {
        let len = 128.min(l.len() - at);
        let ev: Vec<Timed> = events
            .iter()
            .filter(|(f, _, _)| (at..at + len).contains(f))
            .map(|&(f, id, value)| Timed::new((f - at) as u32, NodeEvent::Param { id, value }))
            .collect();
        c.process(
            &Ctx::new(RATE, at as u64),
            &ev,
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
}

/// A sine through the effect: the output, after a settling half second.
fn through(values: &[(u16, f32)], hz: f32, amp: f32) -> Vec<f32> {
    let mut c = character(values);
    let n = RATE as usize;
    let (mut l, mut r) = (sine(hz, amp, n), sine(hz, amp, n));
    run(&mut c, &mut l, &mut r, &[]);
    l[n / 2..].to_vec()
}

/// The level of harmonic `k` of `hz` in `x`, against the fundamental, dB.
fn harmonic_db(x: &[f32], hz: f32, k: f32) -> f32 {
    let at = |f: f32| {
        let (mut s, mut c) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let p = 2.0 * std::f64::consts::PI * f as f64 * i as f64 / RATE as f64;
            s += v as f64 * p.sin();
            c += v as f64 * p.cos();
        }
        (s * s + c * c).sqrt()
    };
    (20.0 * (at(hz * k) / at(hz)).log10()) as f32
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn db(x: f32) -> f32 {
    20.0 * x.log10()
}

#[test]
fn quiet_signals_keep_their_level_in_every_mode() {
    let amp = 0.01; // -40 dBFS
    for mode in [CONSOLE, TAPE, IRON, SOFTEN] {
        // Tape's head bump reaches a little past 100 Hz.
        let low = if mode == TAPE { 500.0 } else { 200.0 };
        for hz in [low, 1000.0, 3000.0] {
            let y = through(&[(MODE, mode), (AMOUNT, 100.0)], hz, amp);
            let level = db(rms(&y) / (amp / 2f32.sqrt()));
            println!("mode {mode}, {hz} Hz at -40 dBFS: {level:+.2} dB");
            assert!(level.abs() < 0.6, "mode {mode}, {hz} Hz: {level:+.2} dB");
        }
    }
}

#[test]
fn console_holds_loud_peaks_with_odd_harmonics_and_iron_adds_even_ones() {
    let hz = 100.0;
    let console = through(&[(MODE, CONSOLE), (AMOUNT, 100.0)], hz, 0.5);
    let peak = console.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let (h2, h3) = (
        harmonic_db(&console, hz, 2.0),
        harmonic_db(&console, hz, 3.0),
    );
    println!("console: peak {peak:.3}, 2nd {h2:.1} dB, 3rd {h3:.1} dB");
    assert!(peak < 0.3, "a -6 dBFS sine is held: peak {peak}");
    assert!(h3 > -30.0, "3rd harmonic {h3:.1} dB");
    assert!(h2 < -90.0, "no 2nd harmonic: {h2:.1} dB");
    let gentle = through(&[(MODE, CONSOLE), (AMOUNT, 20.0)], hz, 0.5);
    let g3 = harmonic_db(&gentle, hz, 3.0);
    println!("console at 20 %: 3rd {g3:.1} dB");
    assert!(g3 < h3 - 10.0, "less Amount, less colour");

    let iron = through(&[(MODE, IRON), (AMOUNT, 100.0)], 60.0, 0.5);
    let i2 = harmonic_db(&iron, 60.0, 2.0);
    let i3 = harmonic_db(&iron, 60.0, 3.0);
    println!("iron at 60 Hz: 2nd {i2:.1} dB, 3rd {i3:.1} dB");
    assert!(i2 > -40.0, "iron's 2nd harmonic {i2:.1} dB");
    // Iron saturates its lows first: a 2 kHz tone at the same level comes
    // through far cleaner.
    let high = through(&[(MODE, IRON), (AMOUNT, 100.0)], 2000.0, 0.5);
    let h3 = harmonic_db(&high, 2000.0, 3.0);
    println!("iron at 2 kHz: 3rd {h3:.1} dB");
    assert!(
        h3 < i3 - 15.0,
        "iron leaves highs cleaner: {h3:.1} dB against {i3:.1}"
    );
}

#[test]
fn tape_lifts_its_head_bump_and_loses_a_little_top() {
    let amp = 0.01;
    let gain = |hz: f32| {
        db(rms(&through(&[(MODE, TAPE), (AMOUNT, 100.0)], hz, amp)) / (amp / 2f32.sqrt()))
    };
    let (bump, mid, top) = (gain(75.0), gain(1000.0), gain(15_000.0));
    println!("tape at 100 %: 75 Hz {bump:+.2} dB, 1 kHz {mid:+.2} dB, 15 kHz {top:+.2} dB");
    assert!((bump - 3.0).abs() < 0.5, "head bump {bump:+.2} dB");
    assert!(mid.abs() < 0.6, "1 kHz {mid:+.2} dB");
    assert!(top < -2.0 && top > -8.0, "15 kHz {top:+.2} dB");
    let none = through(&[(MODE, TAPE), (AMOUNT, 0.0)], 75.0, amp);
    let flat = db(rms(&none) / (amp / 2f32.sqrt()));
    assert!(flat.abs() < 0.1, "no bump at Amount 0: {flat:+.2} dB");
}

#[test]
fn soften_takes_the_edge_off_loud_highs_only() {
    let set = [(MODE, SOFTEN), (AMOUNT, 100.0)];
    let level = |hz: f32, amp: f32| db(rms(&through(&set, hz, amp)) / (amp / 2f32.sqrt()));
    let (loud_high, quiet_high, loud_low) = (
        level(10_000.0, 0.7),
        level(10_000.0, 0.02),
        level(200.0, 0.7),
    );
    println!(
        "soften at 100 %: 10 kHz loud {loud_high:+.2} dB, quiet {quiet_high:+.2} dB; 200 Hz loud {loud_low:+.2} dB"
    );
    assert!(
        loud_high < -3.0,
        "loud highs are softened: {loud_high:+.2} dB"
    );
    assert!(
        quiet_high.abs() < 0.3,
        "quiet highs pass: {quiet_high:+.2} dB"
    );
    assert!(loud_low.abs() < 0.1, "loud lows pass: {loud_low:+.2} dB");
}

#[test]
fn mix_zero_is_the_input_exactly_and_output_scales() {
    let x = sine(440.0, 0.8, 4800);
    for mode in [CONSOLE, TAPE, IRON, SOFTEN] {
        let mut c = character(&[(MODE, mode), (AMOUNT, 100.0), (MIX, 0.0)]);
        let (mut l, mut r) = (x.clone(), x.clone());
        run(&mut c, &mut l, &mut r, &[]);
        assert!(
            l.iter().zip(&x).all(|(a, b)| a.to_bits() == b.to_bits()),
            "mode {mode}"
        );
    }
    let quiet = sine(1000.0, 0.01, RATE as usize);
    let mut c = character(&[(AMOUNT, 0.0), (OUTPUT, -6.0)]);
    let (mut l, mut r) = (quiet.clone(), quiet.clone());
    run(&mut c, &mut l, &mut r, &[]);
    let g = db(rms(&l[4800..]) / rms(&quiet[4800..]));
    assert!((g + 6.0).abs() < 0.05, "Output -6 dB gives {g:+.2} dB");
}

#[test]
fn a_change_of_mode_crossfades_without_a_click() {
    // A loud 100 Hz sine, with the mode stepping through all four every
    // 100 ms: no step between frames is larger than the sine's own
    // steepest step plus a little.
    let n = RATE as usize / 2;
    let x = sine(100.0, 0.7, n);
    let steep = 0.7 * 2.0 * PI * 100.0 / RATE as f32;
    let changes: Vec<(usize, u16, f32)> = (1..5)
        .map(|k| (k * RATE as usize / 10 + 37, MODE, (k % 4) as f32))
        .collect();
    let mut c = character(&[(AMOUNT, 100.0)]);
    let (mut l, mut r) = (x.clone(), x.clone());
    run(&mut c, &mut l, &mut r, &changes);
    let worst = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    println!("worst step {worst:.4}, the sine's steepest {steep:.4}");
    assert!(worst < 2.0 * steep, "worst step {worst}");
}
