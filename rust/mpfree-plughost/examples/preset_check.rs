//! Plays a plugin's own presets the way the app does: the rack scans a
//! folder, lists each instrument's presets in a process of their own, makes
//! the plugin in its own process and loads presets into it, then plays a
//! chord on each. A check for a plugin pack, not a benchmark.
//!
//! ```text
//! preset_check DIR [--home DIR] [--count N] [--wait SECS] [--settle SECS] [--exe FILE [WORDS] --]
//! ```
//!
//! `--home` is the plugins' home folder (default: a fresh one in the temp
//! folder, never the real home). `--count` presets per instrument, spread
//! over its list (default 4). `--wait`: seconds of audio to wait for a
//! preset to sound (default 10); `--settle`: seconds of quiet first, as
//! when a musician picks a preset and plays a moment later (default 0.5).
//! A plugin that loads in the background (Surge XT) is silent while it
//! does, and drops the notes played meanwhile; a chord played at once can
//! still sound from the old preset as it fades out, which the settle
//! keeps from counting as the new one.
//! Under qemu-user, name the emulator as the
//! plugin process: `--exe qemu-arm -L /usr/arm-linux-gnueabihf
//! .../mpfree-plughost --`. Exits 1 if a list or a load fails, or a
//! plugin stays silent after every preset.

use mpfree_node::{Ctx, Kind, NodeEvent, Timed};
use mpfree_plughost::rack::Rack;
use mpfree_plughost::{Deadline, Program};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const RATE: u32 = 44_100;
const FRAMES: usize = 128;

fn usage() -> ! {
    eprintln!(
        "usage: preset_check DIR [--home DIR] [--count N] [--wait SECS] [--settle SECS] [--exe FILE [WORDS] --]"
    );
    std::process::exit(64);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(dir) = args.next().map(PathBuf::from) else {
        usage()
    };
    let mut home = None;
    let mut count = 4usize;
    let mut wait = 10.0f32;
    let mut settle = 0.5f32;
    let mut program = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--home" => home = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--count" => {
                count = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--settle" => {
                settle = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--wait" => {
                wait = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or_else(|| usage())
            }
            "--exe" => {
                let exe = PathBuf::from(args.next().unwrap_or_else(|| usage()));
                let mut lead = Vec::new();
                for w in args.by_ref() {
                    if w == "--" {
                        break;
                    }
                    lead.push(w);
                }
                let lead: Vec<&str> = lead.iter().map(String::as_str).collect();
                program = Some(Program::with_lead(&exe, &lead));
            }
            _ => usage(),
        }
    }
    let program = program.unwrap_or_else(|| {
        let here = std::env::current_exe().expect("exe");
        let exe = here
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("mpfree-plughost"));
        Program::new(&exe.unwrap_or_else(|| PathBuf::from("mpfree-plughost")))
    });
    let home = home.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("mpfree-preset-check-{}", std::process::id()))
    });
    let (mut rack, notes) = Rack::scan(program.with_home(&home), &dir, "the folder");
    for n in notes {
        println!("{n}");
    }
    rack.set_deadline(Deadline::Fixed(Duration::from_secs(5)));
    let mut failed = 0;
    let instruments: Vec<_> = rack.instruments().cloned().collect();
    for e in &instruments {
        let t = Instant::now();
        let list = match rack.presets(e.node_id) {
            Ok(l) => l,
            Err(err) => {
                println!("{}: presets not listed: {err}", e.name);
                failed += 1;
                continue;
            }
        };
        println!(
            "{}: {} preset(s), listed in {:.1} s",
            e.name,
            list.len(),
            t.elapsed().as_secs_f32()
        );
        for l in rack.take_lines().unwrap_or_default() {
            println!("  {l}");
        }
        if list.is_empty() {
            continue;
        }
        let (instance, mut node) = match rack.make_instance(e.node_id) {
            Ok(m) => m,
            Err(err) => {
                println!("  not made: {err}");
                failed += 1;
                continue;
            }
        };
        node.prepare(RATE, FRAMES);
        let mut frame = 0u64;
        let mut run = |node: &mut Box<dyn mpfree_node::Node>, secs: f32, events: &[Timed]| {
            let (mut l, mut r) = (vec![0f32; FRAMES], vec![0f32; FRAMES]);
            let mut peak = 0f32;
            let blocks = (secs * RATE as f32 / FRAMES as f32) as usize;
            for b in 0..blocks {
                l.fill(0.0);
                r.fill(0.0);
                let ev: &[Timed] = if b == 0 { events } else { &[] };
                node.process(&Ctx::new(RATE, frame), ev, &mut l, &mut r);
                frame += FRAMES as u64;
                for s in l.iter().chain(&r) {
                    if s.is_finite() {
                        peak = peak.max(s.abs());
                    }
                }
            }
            peak
        };
        let chord: Vec<Timed> = [48u8, 55, 60, 64]
            .iter()
            .map(|&note| Timed {
                offset: 0,
                ev: NodeEvent::NoteOn {
                    note,
                    velocity: 100,
                },
            })
            .collect();
        let off = [Timed {
            offset: 0,
            ev: NodeEvent::AllOff,
        }];
        let first = run(&mut node, 0.5, &chord);
        run(&mut node, 0.2, &off);
        println!(
            "  before any preset: chord peak {}",
            if first > 0.0 {
                format!("{:.1} dBFS", 20.0 * first.log10())
            } else {
                "silent".into()
            }
        );
        let mut heard = 0;
        let step = (list.len() / count.max(1)).max(1);
        for p in list.iter().step_by(step).take(count) {
            let before: Vec<f32> = (0..node.params().len())
                .map(|i| node.param(i as u16))
                .collect();
            let t = Instant::now();
            if let Err(err) = rack.load_preset(instance, p) {
                println!("  {}: didn't load: {err}", p.name);
                failed += 1;
                continue;
            }
            let asked = t.elapsed();
            // Some plugins load in the background, silent while they do:
            // play the chord every quarter second until it sounds.
            let started = Instant::now();
            if settle > 0.0 {
                run(&mut node, settle, &[]);
            }
            let (mut waited, mut peak) = (settle, 0.0f32);
            while waited < wait {
                peak = run(&mut node, 0.25, &chord);
                run(&mut node, 0.05, &off);
                waited += 0.3;
                if peak > 1e-4 {
                    break;
                }
            }
            let changed = (0..node.params().len())
                .filter(|&i| (node.param(i as u16) - before[i]).abs() > 1e-6)
                .count();
            let db = if peak > 0.0 {
                20.0 * peak.log10()
            } else {
                f32::NEG_INFINITY
            };
            let sounded = if peak > 1e-4 {
                heard += 1;
                format!(
                    "sounded after {waited:.2} s of audio ({:.1} s here)",
                    started.elapsed().as_secs_f32()
                )
            } else {
                format!("silent for {waited:.1} s of audio")
            };
            println!(
                "  {} ({}): asked in {} ms, {sounded}, chord peak {db:.1} dBFS, {changed} parameter(s) changed",
                p.name,
                if p.category.is_empty() { "-" } else { &p.category },
                asked.as_millis()
            );
        }
        if heard == 0 && e.kind == Kind::Instrument {
            println!("  silent after every preset");
            failed += 1;
        }
        for l in rack.take_lines().unwrap_or_default() {
            println!("  {l}");
        }
    }
    if let Some(s) = rack.summary() {
        println!("{}", s.line().unwrap_or_default());
    }
    std::process::exit(i32::from(failed > 0));
}
