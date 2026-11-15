//! Loads a CLAP file and plays every plugin in it for a few seconds, the way
//! the plugin pack's builds are checked (docs/dev/guides/plugin-pack.md).
//!
//! ```text
//! cargo run --release -p mpfree-clap --example clap_check -- FILE [--only TEXT]
//! ```
//!
//! For each plugin: make it, prepare it at 44.1 kHz with 128-frame blocks, play
//! a chord (an instrument) or a tone (an effect) for a second and let it ring
//! for another, then move a parameter, save the state and load it into a
//! second copy, which must come back with every value. One line
//! per plugin; the exit code is 1 if any failed. A plugin fails when it can't be
//! made or activated, a block fails, its output isn't a finite number, an
//! instrument stays silent, or its state doesn't load back. An effect that
//! stays silent is said but passes (a gate may be shut).
//!
//! The time is the share of real time its blocks took on this machine; under
//! an emulator it says nothing about the MPC.

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use mpfree_clap::Library;
use mpfree_node::{Ctx, Kind, Node, NodeEvent, Timed};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
const CHORD: [u8; 4] = [48, 55, 60, 64];

struct Played {
    peak: f32,
    bad: usize,
    errors: u64,
    busy: f64,
}

fn play(node: &mut dyn Node, kind: Kind) -> Played {
    let blocks = (2 * RATE as usize) / BLOCK;
    let on_blocks = blocks / 2;
    let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let mut peak = 0.0f32;
    let mut bad = 0;
    let mut spent = 0.0f64;
    let mut phase = 0.0f32;
    for b in 0..blocks {
        let mut events = Vec::new();
        if kind == Kind::Instrument {
            if b == 0 {
                for n in CHORD {
                    events.push(Timed::new(
                        0,
                        NodeEvent::NoteOn {
                            note: n,
                            velocity: 100,
                        },
                    ));
                }
            } else if b == on_blocks {
                for n in CHORD {
                    events.push(Timed::new(0, NodeEvent::NoteOff { note: n }));
                }
            }
            l.fill(0.0);
            r.fill(0.0);
        } else {
            for i in 0..BLOCK {
                let v = if b < on_blocks {
                    0.25 * (phase * std::f32::consts::TAU).sin()
                } else {
                    0.0
                };
                phase = (phase + 220.0 / RATE as f32).fract();
                l[i] = v;
                r[i] = v;
            }
        }
        let ctx = Ctx {
            rate: RATE,
            frame: (b * BLOCK) as u64,
            bpm: 120.0,
            beats: (b * BLOCK) as f64 / RATE as f64 * 2.0,
            playing: true,
            sidechain: None,
        };
        let t = Instant::now();
        node.process(&ctx, &events, &mut l, &mut r);
        spent += t.elapsed().as_secs_f64();
        for &v in l.iter().chain(r.iter()) {
            if v.is_finite() {
                peak = peak.max(v.abs());
            } else {
                bad += 1;
            }
        }
    }
    let real = blocks as f64 * BLOCK as f64 / RATE as f64;
    Played {
        peak,
        bad,
        errors: 0,
        busy: spent / real,
    }
}

fn db(v: f32) -> String {
    if v <= 0.0 {
        "silent".into()
    } else {
        format!("{:.1} dBFS", 20.0 * v.log10())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(file) = args.first() else {
        eprintln!("usage: clap_check FILE [--only TEXT]");
        return ExitCode::from(2);
    };
    let only = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|i| args.get(i + 1));
    let lib = match Library::open(Path::new(file)) {
        Ok(l) => l,
        Err(e) => {
            println!("FAIL {file}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let plugins = lib.plugins();
    println!("{file}: {} plugins", plugins.len());
    let mut failed = 0;
    let mut checked = 0;
    for p in &plugins {
        if only.is_some_and(|o| !p.id.contains(o.as_str()) && !p.name.contains(o.as_str())) {
            continue;
        }
        checked += 1;
        let mut node = match lib.create(&p.id) {
            Ok(n) => n,
            Err(e) => {
                println!("FAIL {}: {e}", p.id);
                failed += 1;
                continue;
            }
        };
        let kind = node.kind();
        let params = node.params().len();
        node.prepare(RATE, BLOCK);
        if !node.is_active() {
            println!("FAIL {}: won't activate", p.id);
            failed += 1;
            continue;
        }
        let mut played = play(&mut node, kind);
        // Move one parameter off its default, so the state saved has
        // something to carry; the next block takes it to the plugin.
        let moved = node
            .params()
            .iter()
            .find(|i| !i.readonly && i.max > i.min)
            .map(|i| {
                let v = i.clamp(i.min + 0.37 * (i.max - i.min));
                let v = if (v - i.default).abs() < 1e-6 {
                    i.clamp(i.min + 0.71 * (i.max - i.min))
                } else {
                    v
                };
                (i.id, v)
            });
        if let Some((id, v)) = moved {
            node.set_param(id, v);
            let ctx = Ctx {
                rate: RATE,
                frame: 0,
                bpm: 120.0,
                beats: 0.0,
                playing: false,
                sidechain: None,
            };
            let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
            node.process(&ctx, &[], &mut l, &mut r);
        }
        played.errors = node.stats().errors;
        let control = node.control();
        let text = if params > 0 {
            control.param_text(0, node.param(0))
        } else {
            None
        };
        let state = control.save_state();
        let reloaded = match &state {
            Some(bytes) => match lib.create(&p.id) {
                Ok(other) => {
                    let ok = other.control().load_state(bytes);
                    let same =
                        (0..params as u16).all(|i| (other.param(i) - node.param(i)).abs() < 1e-4);
                    let carried = moved.is_none_or(|(id, v)| (other.param(id) - v).abs() < 1e-3);
                    ok && same && carried
                }
                Err(_) => false,
            },
            None => false,
        };
        let silent = played.peak <= 1e-6;
        let mut why = Vec::new();
        if played.bad > 0 {
            why.push(format!("{} samples not finite", played.bad));
        }
        if played.errors > 0 {
            why.push(format!("{} blocks failed", played.errors));
        }
        if silent && kind == Kind::Instrument {
            why.push("silent".into());
        }
        if !reloaded {
            why.push(if state.is_none() {
                "no state".into()
            } else {
                "state didn't load back".to_string()
            });
        }
        let what = if kind == Kind::Instrument {
            "instrument"
        } else {
            "effect"
        };
        let line = format!(
            "{} ({what}, {params} params{}): peak {}, state {} bytes, {:.0}% of real time",
            p.id,
            text.map(|t| format!(", first reads \"{t}\""))
                .unwrap_or_default(),
            db(played.peak),
            state.as_ref().map_or(0, Vec::len),
            played.busy * 100.0,
        );
        if why.is_empty() {
            println!(
                "ok   {line}{}",
                if silent {
                    " (silent: an effect may be)"
                } else {
                    ""
                }
            );
        } else {
            failed += 1;
            println!("FAIL {line}: {}", why.join(", "));
        }
    }
    println!("{checked} checked, {failed} failed");
    if failed > 0 || checked == 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
