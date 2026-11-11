//! Renders Poly presets the same way every run and prints a hash of the
//! output and the time taken, so a change to Poly's code can be checked for
//! identical output and timed.
//!
//! `cargo run --release -p mpfree-synth --example poly_cost [PRESET] [VOICES] [SECONDS]`.
//! With no preset it renders all of them plus a few edge settings.

use std::time::Instant;

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::params::*;
use mpfree_synth::{Poly, PRESETS};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let only = args.get(1).cloned();
    let voices: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(8);
    let secs: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let mut total = 0u64;
    for (name, extra) in cases() {
        if only.as_deref().is_some_and(|o| o != name) {
            continue;
        }
        let preset = PRESETS.iter().find(|p| name.starts_with(p.name)).unwrap();
        let mut s = Poly::new(RATE);
        for &(id, v) in preset.values {
            s.set_param(id, v);
        }
        for &(id, v) in extra {
            s.set_param(id, v);
        }
        s.set_param(POLYPHONY, 16.0);
        let t = Instant::now();
        let h = render(&mut s, voices, secs);
        let us = t.elapsed().as_secs_f64() * 1e6 / (secs * f64::from(RATE) / BLOCK as f64);
        total ^= h;
        println!("{name:<28} {h:016x}  {us:7.1} us/block");
    }
    println!("{:<28} {total:016x}", "all");
}

fn cases() -> Vec<(&'static str, &'static [(u16, f32)])> {
    let mut v: Vec<(&'static str, &'static [(u16, f32)])> =
        PRESETS.iter().map(|p| (p.name, &[][..])).collect();
    v.push(("Init unison 4", &[(UNISON, 4.0)]));
    v.push((
        "Init unison 3 drive",
        &[(UNISON, 3.0), (DRIVE, 6.0), (FILTER_MODE, 2.0)],
    ));
    v.push((
        "Init unison 2 spread 0",
        &[(UNISON, 2.0), (UNISON_SPREAD, 0.0), (FILTER_MODE, 3.0)],
    ));
    v.push((
        "Init sine tri sub noise",
        &[
            (OSC1_WAVE, 3.0),
            (OSC2_WAVE, 2.0),
            (SUB_LEVEL, 50.0),
            (NOISE_LEVEL, 20.0),
        ],
    ));
    v
}

/// Chords of `voices` notes changing every half second, with pressure,
/// bend and a spread change mid-way, into 128-frame blocks.
fn render(s: &mut Poly, voices: usize, secs: f64) -> u64 {
    let blocks = (secs * f64::from(RATE) / BLOCK as f64) as u64;
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let chord_blocks = (f64::from(RATE) * 0.5 / BLOCK as f64) as u64;
    let mut held: Vec<u8> = Vec::new();
    for b in 0..blocks {
        let mut ev = Vec::new();
        if b % chord_blocks == 0 {
            for &n in &held {
                ev.push(Timed::new(3, NodeEvent::NoteOff { note: n }));
            }
            held.clear();
            let root = 40 + ((b / chord_blocks) * 5 % 17) as u8;
            for i in 0..voices {
                let n = root + (i as u8) * 4 % 24 + (i as u8 / 6) * 12;
                if !held.contains(&n) {
                    held.push(n);
                    ev.push(Timed::new(
                        7,
                        NodeEvent::NoteOn {
                            note: n,
                            velocity: 60 + (i as u8 * 9) % 60,
                        },
                    ));
                }
            }
        }
        if b % 7 == 3 {
            if let Some(&n) = held.first() {
                ev.push(Timed::new(
                    50,
                    NodeEvent::Pressure {
                        note: n,
                        value: ((b % 40) * 3) as u8,
                    },
                ));
            }
        }
        if b == blocks / 2 {
            ev.push(Timed::new(20, NodeEvent::Bend { value: 0.3 }));
        }
        // Spread off then back on, so stereo stacks go mono and back.
        if b == blocks / 3 {
            ev.push(Timed::new(
                33,
                NodeEvent::Param {
                    id: UNISON_SPREAD,
                    value: 0.0,
                },
            ));
        }
        if b == blocks * 2 / 3 {
            ev.push(Timed::new(
                90,
                NodeEvent::Param {
                    id: UNISON_SPREAD,
                    value: 75.0,
                },
            ));
        }
        l.fill(0.0);
        r.fill(0.0);
        s.process(&Ctx::new(RATE, b * BLOCK as u64), &ev, &mut l, &mut r);
        for k in 0..BLOCK {
            h = (h ^ u64::from(l[k].to_bits())).wrapping_mul(0x100_0000_01b3);
            h = (h ^ u64::from(r[k].to_bits())).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}
