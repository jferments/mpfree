//! Renders every preset of FM, Drums, Pluck and Wavetable the same way
//! every run and prints a hash of the output and the time taken, so a
//! change to an engine's code can be checked for identical output and
//! timed (Poly has its own, `poly_cost`).
//!
//! `cargo run --release -p mpfree-synth --example engine_cost [ENGINE [PRESET [SECONDS]]]`.
//! ENGINE is fm, drums, pluck or wt; with none it renders all of them.

use std::time::Instant;

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::{fm_params, pluck_params, wavetable, Drums, Fm, Pluck, Wavetable};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let engine = args.get(1).cloned();
    let only = args.get(2).cloned();
    let secs: f64 = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let want = |e: &str| engine.as_deref().is_none_or(|x| x == e);
    let pick = |name: &str| only.as_deref().is_none_or(|o| o == name);
    let mut total = 0u64;
    let mut run = |label: String, node: &mut dyn Node, drums: bool, toggle: Toggle| {
        let t = Instant::now();
        let h = render(node, drums, secs, toggle);
        let us = t.elapsed().as_secs_f64() * 1e6 / (secs * f64::from(RATE) / BLOCK as f64);
        total ^= h;
        println!("{label:<28} {h:016x}  {us:7.1} us/block");
    };
    if want("fm") {
        for p in Fm::presets().iter().filter(|p| pick(&p.name)) {
            let mut s = Fm::new(RATE);
            s.load_preset(p);
            s.set_param(fm_params::POLYPHONY, 16.0);
            run(format!("FM {}", p.name), &mut s, false, None);
        }
    }
    if want("drums") {
        for p in Drums::presets().iter().filter(|p| pick(p.name)) {
            let mut s = Drums::new(RATE);
            s.load_preset(p);
            run(format!("Drums {}", p.name), &mut s, true, None);
        }
    }
    if want("pluck") {
        for p in Pluck::presets().iter().filter(|p| pick(p.name)) {
            let mut s = Pluck::new(RATE);
            s.load_preset(p);
            s.set_param(pluck_params::POLYPHONY, 16.0);
            run(format!("Pluck {}", p.name), &mut s, false, None);
        }
    }
    if want("wt") {
        for p in Wavetable::presets().iter().filter(|p| pick(p.name)) {
            let mut s = Wavetable::new(RATE);
            s.load_preset(p);
            s.set_param(wavetable::params::POLYPHONY, 16.0);
            run(format!("WT {}", p.name), &mut s, false, None);
        }
        // Unison with no spread (both sides the same), and a spread that
        // comes and goes.
        use wavetable::params::{UNISON, UNISON_SPREAD};
        let presets = Wavetable::presets();
        let edge: [(&str, &str, f32, f32, Toggle); 2] = [
            ("Morph Pad spread 0", "Morph Pad", 3.0, 0.0, None),
            (
                "Init unison 3 spread on/off",
                "Init",
                3.0,
                0.0,
                Some((UNISON_SPREAD, 60.0, 0.0)),
            ),
        ];
        for (label, base, unison, spread, toggle) in edge {
            if !pick(label) {
                continue;
            }
            let mut s = Wavetable::new(RATE);
            if let Some(p) = presets.iter().find(|p| p.name == base) {
                s.load_preset(p);
            }
            s.set_param(wavetable::params::POLYPHONY, 16.0);
            s.set_param(UNISON, unison);
            s.set_param(UNISON_SPREAD, spread);
            run(format!("WT {label}"), &mut s, false, toggle);
        }
    }
    println!("{:<28} {total:016x}", "all");
}

/// Keyboards: chords of 8 notes changing every half second, with pressure
/// and a bend. Drums: a busy 16th-note pattern over the kit's 16 pads
/// (pad 1 is note 36 in every factory kit).
/// A parameter set to one value a third of the way through and another
/// at two thirds.
type Toggle = Option<(u16, f32, f32)>;

fn render(s: &mut dyn Node, drums: bool, secs: f64, toggle: Toggle) -> u64 {
    let blocks = (secs * f64::from(RATE) / BLOCK as f64) as u64;
    let mut l = vec![0.0f32; BLOCK];
    let mut r = vec![0.0f32; BLOCK];
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let chord_blocks = (f64::from(RATE) * 0.5 / BLOCK as f64) as u64;
    let mut held: Vec<u8> = Vec::new();
    for b in 0..blocks {
        let mut ev = Vec::new();
        if drums {
            // A 16th at 120 BPM is about 10 blocks.
            if b % 10 == 0 {
                let step = (b / 10) as u8;
                for (k, pad) in [step % 16, (step * 5 + 3) % 16].into_iter().enumerate() {
                    if k == 0 || step.is_multiple_of(3) {
                        let velocity = 50 + (step * 13 + pad) % 77;
                        ev.push(Timed::new(
                            5 + 9 * k as u32,
                            NodeEvent::NoteOn {
                                note: 36 + pad,
                                velocity,
                            },
                        ));
                    }
                }
            }
        } else {
            if b % chord_blocks == 0 {
                for &n in &held {
                    ev.push(Timed::new(3, NodeEvent::NoteOff { note: n }));
                }
                held.clear();
                let root = 40 + ((b / chord_blocks) * 5 % 17) as u8;
                for i in 0..8u8 {
                    let n = root + i * 4 % 24 + (i / 6) * 12;
                    if !held.contains(&n) {
                        held.push(n);
                        ev.push(Timed::new(
                            7,
                            NodeEvent::NoteOn {
                                note: n,
                                velocity: 60 + (i * 9) % 60,
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
        }
        if let Some((id, a, z)) = toggle {
            if b == blocks / 3 {
                s.set_param(id, a);
            } else if b == blocks * 2 / 3 {
                s.set_param(id, z);
            }
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
