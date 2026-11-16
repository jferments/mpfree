//! Plays the plugins in a folder the way the app does: the rack scans the
//! folder, and each plugin runs in its own process. A check for the plugin
//! pack (docs/dev/guides/plugin-pack.md) and for any folder of `.clap` files
//! or MPC plugins (docs/dev/guides/mpc-plugins-in-mpfree.md).
//!
//! ```text
//! cargo run --release -p mpfree-plughost --example rack_check -- DIR [--exe FILE [WORD ...] --]
//! ```
//!
//! `--exe` names the plugin process (and words before its own arguments);
//! by default it is `mpfree-plughost` beside this example. Under qemu-user a
//! process started from inside the emulator runs on the host CPU, so name the
//! emulator: `--exe qemu-arm -L /usr/arm-linux-gnueabihf .../mpfree-plughost --`.
//!
//! Each plugin is made through the rack, prepared at 44.1 kHz with 128-frame
//! blocks and played for two seconds (a chord for an instrument, a tone for an
//! effect), with a 5-second deadline per block so an emulator doesn't count as
//! late. Then its state is read back from its process and loaded into a second
//! one. For an effect it also says how far its output is from the dry tone
//! and how loud its tail is after the tone stops (information, not a
//! failure: an effect may be dry at its defaults). A plugin fails when the rack can't make it, a block's output isn't a
//! finite number, an instrument stays silent, its state can't be read or
//! loaded, or its process crashes. The exit code is 1 if any failed. Touches
//! nothing outside its own processes.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use mpfree_node::{Ctx, Kind, NodeEvent, Timed};
use mpfree_plughost::rack::Rack;
use mpfree_plughost::{Deadline, Program};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
const CHORD: [u8; 4] = [48, 55, 60, 64];

fn beside_me(name: &str) -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let dir = me.parent()?;
    [dir.join(name), dir.join("..").join(name)]
        .into_iter()
        .find(|p| p.exists())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(dir) = args.first().map(PathBuf::from) else {
        eprintln!("usage: rack_check DIR [--exe FILE [WORD ...] --]");
        return ExitCode::from(2);
    };
    let program = match args.iter().position(|a| a == "--exe") {
        Some(i) => {
            let rest: Vec<&str> = args[i + 1..]
                .iter()
                .take_while(|a| *a != "--")
                .map(String::as_str)
                .collect();
            let Some((exe, lead)) = rest.split_first() else {
                eprintln!("--exe needs a program");
                return ExitCode::from(2);
            };
            Program::with_lead(Path::new(exe), lead)
        }
        None => match beside_me("mpfree-plughost") {
            Some(p) => Program::new(&p),
            None => {
                eprintln!("no mpfree-plughost beside this example; name it with --exe");
                return ExitCode::from(2);
            }
        },
    };

    let (mut rack, notes) = Rack::scan(program, &dir, &dir.display().to_string());
    for n in &notes {
        println!("scan: {n}");
    }
    rack.set_deadline(Deadline::Fixed(Duration::from_secs(5)));
    let entries = rack.entries().to_vec();
    println!("{} plugins in {}", entries.len(), dir.display());

    let mut failed = 0;
    for e in &entries {
        let mut why = Vec::new();
        let mut node = match rack.make(e.node_id) {
            Ok(n) => n,
            Err(err) => {
                println!("FAIL {}: {err}", e.node_id);
                failed += 1;
                continue;
            }
        };
        node.prepare(RATE, BLOCK);
        let (mut l, mut r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
        let blocks = 2 * RATE as usize / BLOCK;
        let (mut peak, mut bad, mut phase) = (0.0f32, 0usize, 0.0f32);
        // For an effect: how far the output is from the dry tone, and what
        // rings on after the tone stops.
        let (mut dry_e, mut diff_e, mut tail) = (0.0f64, 0.0f64, 0.0f32);
        let mut dry = vec![0.0f32; BLOCK];
        let started = Instant::now();
        for b in 0..blocks {
            let mut events = Vec::new();
            if e.kind == Kind::Instrument {
                l.fill(0.0);
                r.fill(0.0);
                if b == 0 {
                    events.extend(CHORD.map(|n| {
                        Timed::new(
                            0,
                            NodeEvent::NoteOn {
                                note: n,
                                velocity: 100,
                            },
                        )
                    }));
                } else if b == blocks / 2 {
                    events.extend(CHORD.map(|n| Timed::new(0, NodeEvent::NoteOff { note: n })));
                }
            } else {
                for i in 0..BLOCK {
                    let v = if b < blocks / 2 {
                        0.25 * (phase * std::f32::consts::TAU).sin()
                    } else {
                        0.0
                    };
                    phase = (phase + 220.0 / RATE as f32).fract();
                    l[i] = v;
                    r[i] = v;
                    dry[i] = v;
                }
            }
            node.process(&Ctx::new(RATE, (b * BLOCK) as u64), &events, &mut l, &mut r);
            for &v in l.iter().chain(r.iter()) {
                if v.is_finite() {
                    peak = peak.max(v.abs());
                } else {
                    bad += 1;
                }
            }
            if e.kind == Kind::Effect {
                for i in 0..BLOCK {
                    if b < blocks / 2 {
                        dry_e += f64::from(dry[i] * dry[i]) * 2.0;
                        let (dl, dr) = (l[i] - dry[i], r[i] - dry[i]);
                        diff_e += f64::from(dl * dl + dr * dr);
                    } else if l[i].is_finite() && r[i].is_finite() {
                        tail = tail.max(l[i].abs().max(r[i].abs()));
                    }
                }
            }
        }
        let took = started.elapsed();
        if bad > 0 {
            why.push(format!("{bad} samples not finite"));
        }
        if peak <= 1e-6 && e.kind == Kind::Instrument {
            why.push("silent".to_string());
        }
        rack.refresh_states();
        let state = node.save_state();
        match &state {
            None => why.push("no state from its process".to_string()),
            Some(bytes) => match rack.make(e.node_id) {
                Ok(mut other) => {
                    if let Err(err) = other.load_state(bytes) {
                        why.push(format!("state didn't load: {err}"));
                    }
                }
                Err(err) => why.push(format!("a second copy failed: {err}")),
            },
        }
        rack.watch(Instant::now());
        let crashes = rack.summary().map_or(0, |s| s.crashes);
        if crashes > 0 {
            why.push(format!("{crashes} crashes so far"));
        }
        let what = if e.kind == Kind::Instrument {
            "instrument".to_string()
        } else {
            let db = |x: f64| {
                if x > 0.0 {
                    format!("{:.1} dB", 10.0 * x.log10())
                } else {
                    "none".to_string()
                }
            };
            let tail = if tail > 0.0 {
                format!("{:.1} dBFS", 20.0 * tail.log10())
            } else {
                "none".to_string()
            };
            format!(
                "effect, change from the dry tone {}, tail {tail}",
                db(diff_e / dry_e.max(1e-12))
            )
        };
        let line = format!(
            "{} ({what}, {}): peak {}, state {} bytes, 2 s played in {:.1} s",
            e.node_id,
            e.about,
            if peak > 0.0 {
                format!("{:.1} dBFS", 20.0 * peak.log10())
            } else {
                "silent".into()
            },
            state.as_ref().map_or(0, Vec::len),
            took.as_secs_f64(),
        );
        if why.is_empty() {
            println!("ok   {line}");
        } else {
            failed += 1;
            println!("FAIL {line}: {}", why.join(", "));
        }
        drop(node);
        rack.watch(Instant::now());
    }
    if let Some(lines) = rack.take_lines() {
        for l in lines {
            println!("rack: {l}");
        }
    }
    if let Some(s) = rack.summary().and_then(|s| s.line()) {
        println!("rack: {s}");
    }
    println!("{} checked, {failed} failed", entries.len());
    if failed > 0 || entries.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
