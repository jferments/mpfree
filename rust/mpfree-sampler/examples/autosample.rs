//! `autosample`: auto-sample one of MPFree's synths offline and hear the
//! result, on the workstation. Workstation only: no MPC, ALSA or network
//! access, and it writes only inside the output folder it is given:
//!
//! - `synth.wav`: a short phrase (an arpeggio, then a held chord) played
//!   by the synth itself.
//! - `sampled.wav`: the same phrase played by the sampler from the
//!   instrument it made, to compare with `synth.wav`.
//! - `takes/`: every take as trimmed (and normalized) for its pad.
//! - `autosample-report.txt`: the plan, its estimate, what was made and
//!   how long it took (and, with `--calibration`, an estimate on the MPC).
#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use mpfree_analysis::note_name;
use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_prof::model::{measure, Calibration, Model};
use mpfree_sampler::autosample::{Job, Plan, State, VelocityCurve};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{LoopMode, Project};
use mpfree_synth::{Drums, Fm, Pluck, Poly, Wavetable};

#[path = "../src/bin/shared/wav_out.rs"]
mod wav_out;
use wav_out::write_wav;

const RATE: u32 = 44_100;
const USAGE: &str = "\
Usage: cargo run --release -p mpfree-sampler --example autosample -- OUTPUT_DIR [options]

Options:
  --engine E          poly, fm, pluck, wavetable or drums (default poly)
  --preset NAME       the synth's preset (default: the one the app starts with)
  --list              list the engine's presets and stop
  --lo N --hi N       lowest and highest MIDI note (default 36 and 84, C2 to C6)
  --every N           semitones between sampled notes, 1 to 12 (default 3)
  --layers N          velocity layers, 1 to 8 (default 3: 40, 90, 127)
  --curve C           how the layers spread: linear, exp or log (default linear)
  --rr N              round robins, 1 to 4 (default 1)
  --hold S            seconds each note is held (default 2)
  --tail S            the longest each note rings after it (default 4)
  --loops             look for a forward loop in each note (--pingpong for ping-pong)
  --mono              record mono
  --calibration FILE  add an MPC time estimate from this calibration file
                      (docs/dev/perf/mpc-one-calibration.txt)

Workstation only; writes only inside OUTPUT_DIR.";

struct Options {
    out: PathBuf,
    engine: String,
    preset: Option<String>,
    list: bool,
    plan: Plan,
    calibration: Option<PathBuf>,
}

fn parse() -> Result<Options, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return Err(USAGE.into());
    }
    let mut o = Options {
        out: PathBuf::new(),
        engine: "poly".into(),
        preset: None,
        list: false,
        plan: Plan::default(),
        calibration: None,
    };
    let (mut layers, mut curve) = (None, VelocityCurve::Linear);
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = || {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"))
        };
        let mut took = true;
        match a {
            "--engine" => o.engine = value()?.to_lowercase(),
            "--preset" => o.preset = Some(value()?),
            "--lo" => o.plan.lo = value()?.parse()?,
            "--hi" => o.plan.hi = value()?.parse()?,
            "--every" => o.plan.every = value()?.parse()?,
            "--layers" => layers = Some(value()?.parse::<usize>()?),
            "--curve" => {
                curve = match value()?.as_str() {
                    "linear" => VelocityCurve::Linear,
                    "exp" => VelocityCurve::Exp,
                    "log" => VelocityCurve::Log,
                    c => return Err(format!("unknown curve {c}\n\n{USAGE}").into()),
                }
            }
            "--rr" => o.plan.round_robins = value()?.parse()?,
            "--hold" => o.plan.hold = value()?.parse()?,
            "--tail" => o.plan.tail = value()?.parse()?,
            "--calibration" => o.calibration = Some(PathBuf::from(value()?)),
            _ => {
                took = false;
                match a {
                    "--list" => o.list = true,
                    "--loops" => o.plan.loops = Some(LoopMode::Forward),
                    "--pingpong" => o.plan.loops = Some(LoopMode::PingPong),
                    "--mono" => o.plan.channels = 1,
                    _ if a.starts_with("--") => {
                        return Err(format!("unknown option {a}\n\n{USAGE}").into())
                    }
                    _ => pos.push(PathBuf::from(a)),
                }
            }
        }
        i += if took { 2 } else { 1 };
    }
    if let Some(n) = layers {
        if !(1..=8).contains(&n) {
            return Err("--layers is 1 to 8".into());
        }
        o.plan.velocities = if n == 3 && curve == VelocityCurve::Linear {
            Plan::default().velocities
        } else {
            Plan::spread_velocities(n, curve)
        };
    } else if curve != VelocityCurve::Linear {
        o.plan.velocities = Plan::spread_velocities(3, curve);
    }
    match (o.list, pos.len()) {
        (true, _) => {}
        (false, 1) => o.out = pos.remove(0),
        _ => return Err(USAGE.into()),
    }
    Ok(o)
}

/// A synth, its name with its preset's, and the engine's preset names.
type Synth = (Box<dyn Node>, String, Vec<String>);

/// A fresh synth with a preset loaded.
fn synth(engine: &str, preset: Option<&str>) -> Result<Synth, Box<dyn Error>> {
    fn pick(names: &[String], want: Option<&str>, start: &str) -> Result<usize, String> {
        let want = want.unwrap_or(start);
        names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(want))
            .ok_or_else(|| format!("no preset called {want:?}; --list shows them"))
    }
    let names =
        |ps: &[mpfree_synth::Preset]| ps.iter().map(|p| p.name.to_string()).collect::<Vec<_>>();
    Ok(match engine {
        "poly" => {
            let all = names(Poly::presets());
            let i = pick(&all, preset, "Pluck")?;
            let mut n = Poly::new(RATE);
            n.load_preset(&Poly::presets()[i]);
            (Box::new(n), format!("Poly {}", all[i]), all)
        }
        "pluck" => {
            let all = names(Pluck::presets());
            let i = pick(&all, preset, "Steel String")?;
            let mut n = Pluck::new(RATE);
            n.load_preset(&Pluck::presets()[i]);
            (Box::new(n), format!("Pluck {}", all[i]), all)
        }
        "wavetable" => {
            let all = names(Wavetable::presets());
            let i = pick(&all, preset, "Harmonic Pluck")?;
            let mut n = Wavetable::new(RATE);
            n.load_preset(&Wavetable::presets()[i]);
            (Box::new(n), format!("Wavetable {}", all[i]), all)
        }
        "drums" => {
            let all = names(Drums::presets());
            let i = pick(&all, preset, "Init Kit")?;
            let mut n = Drums::new(RATE);
            n.load_preset(&Drums::presets()[i]);
            (Box::new(n), format!("Drums {}", all[i]), all)
        }
        "fm" => {
            let presets = Fm::presets();
            let all: Vec<String> = presets.iter().map(|p| p.name.clone()).collect();
            let i = pick(&all, preset, "Glass Keys")?;
            let mut n = Fm::new(RATE);
            n.load_preset(&presets[i]);
            (Box::new(n), format!("FM {}", all[i]), all)
        }
        e => return Err(format!("unknown engine {e}\n\n{USAGE}").into()),
    })
}

/// The phrase: an arpeggio up two octaves from C3, then a held chord.
/// (start seconds, length seconds, note).
fn phrase() -> Vec<(f32, f32, u8)> {
    let mut out: Vec<(f32, f32, u8)> = [48u8, 52, 55, 60, 64, 67, 72]
        .iter()
        .enumerate()
        .map(|(k, &n)| (k as f32 * 0.3, 0.25, n))
        .collect();
    out.extend([48u8, 55, 64].map(|n| (2.4, 2.0, n)));
    out
}
const PHRASE_SECONDS: f32 = 6.5;

fn events_in(from: usize, to: usize) -> Vec<(usize, bool, u8)> {
    let at = |s: f32| (s * RATE as f32) as usize;
    let mut ev = Vec::new();
    for (start, len, note) in phrase() {
        for (f, on) in [(at(start), true), (at(start + len), false)] {
            if (from..to).contains(&f) {
                ev.push((f - from, on, note));
            }
        }
    }
    ev.sort_by_key(|e| (e.0, e.1));
    ev
}

fn play_synth(node: &mut dyn Node) -> (Vec<f32>, Vec<f32>) {
    node.prepare(RATE, 256);
    node.reset();
    let total = (PHRASE_SECONDS * RATE as f32) as usize;
    let (mut l, mut r) = (vec![0.0; total], vec![0.0; total]);
    for from in (0..total).step_by(256) {
        let to = (from + 256).min(total);
        let ev: Vec<Timed> = events_in(from, to)
            .into_iter()
            .map(|(at, on, note)| {
                let ev = if on {
                    NodeEvent::NoteOn {
                        note,
                        velocity: 100,
                    }
                } else {
                    NodeEvent::NoteOff { note }
                };
                Timed::new(at as u32, ev)
            })
            .collect();
        node.process(
            &Ctx::new(RATE, from as u64),
            &ev,
            &mut l[from..to],
            &mut r[from..to],
        );
    }
    (l, r)
}

fn play_sampler(project: &Project) -> (Vec<f32>, Vec<f32>) {
    let mut e = Engine::new(RATE, 32);
    e.set_view(EngineView::build(project, RATE));
    let total = (PHRASE_SECONDS * RATE as f32) as usize;
    let (mut l, mut r) = (vec![0.0; total], vec![0.0; total]);
    // Events land on block starts here: 1 ms blocks keep them close.
    for from in (0..total).step_by(44) {
        let to = (from + 44).min(total);
        for (_, on, note) in events_in(from, to) {
            e.apply(if on {
                Event::Key {
                    note,
                    velocity: 100,
                }
            } else {
                Event::KeyOff { note }
            });
        }
        e.render(&mut l[from..to], &mut r[from..to]);
    }
    (l, r)
}

fn run() -> Result<(), Box<dyn Error>> {
    let o = parse()?;
    let (node, name, presets) = synth(&o.engine, o.preset.as_deref())?;
    if o.list {
        println!("{} presets: {}", o.engine, presets.join(", "));
        return Ok(());
    }
    let plan = Plan {
        name: name.clone(),
        ..o.plan.clone()
    };
    plan.check()?;
    fs::create_dir_all(o.out.join("takes"))?;

    let mut report = String::new();
    let p = &plan;
    let est = p.estimate();
    writeln!(report, "autosample report")?;
    writeln!(report, "synth: {name}")?;
    writeln!(
        report,
        "plan: {} to {} every {} semitones ({} notes), velocities {:?}, {} round robin(s), \
         hold {} s, tail up to {} s, loops {}, {}",
        note_name(p.lo),
        note_name(p.hi),
        p.every,
        p.notes().len(),
        p.velocities,
        p.round_robins,
        p.hold,
        p.tail,
        match p.loops {
            None => "off",
            Some(LoopMode::Forward) => "forward",
            Some(LoopMode::PingPong) => "ping-pong",
        },
        if p.channels == 1 { "mono" } else { "stereo" },
    )?;
    writeln!(
        report,
        "estimate in real time: {} takes, {:.1} min (at most {:.1}), {:.0} MB as recorded (at most {:.0})",
        est.takes,
        est.seconds / 60.0,
        est.seconds_most / 60.0,
        est.bytes as f64 / 1e6,
        est.bytes_most as f64 / 1e6
    )?;

    let model = match &o.calibration {
        Some(path) => {
            let device = Calibration::parse(&fs::read_to_string(path)?)?;
            Some((Model::new(&measure(2000, 7), &device)?, device.machine))
        }
        None => None,
    };

    let job = Job::start(plan.clone(), node)?;
    while !job.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let (done, total) = job.progress();
        eprint!("\rtakes {done}/{total}");
    }
    eprintln!();
    let done = job.finish();
    if done.state != State::Done {
        return Err(format!("the session ended {:?}", done.state).into());
    }
    let inst = done.instrument?;
    writeln!(
        report,
        "made: {} pads, {} takes, {} looped, {:.1} MB, normalized {:+.1} dB{}",
        inst.pads.len(),
        inst.sources.len(),
        inst.looped,
        inst.bytes() as f64 / 1e6,
        inst.gain_db,
        if done.clipped {
            " (something reached full scale)"
        } else {
            ""
        }
    )?;
    let total = done.render_seconds + done.build_seconds;
    writeln!(
        report,
        "took here: render {:.2} s for {:.0} s of audio ({:.0}x real time), build {:.2} s",
        done.render_seconds,
        done.audio_seconds,
        done.audio_seconds / done.render_seconds,
        done.build_seconds
    )?;
    if let Some((m, machine)) = &model {
        let (e, lo, hi) = m.estimate(total * 1e9);
        writeln!(
            report,
            "MPC estimate ({machine}): {:.1} s (range {:.1} to {:.1} s), {:.0}x real time",
            e / 1e9,
            lo / 1e9,
            hi / 1e9,
            done.audio_seconds / (e / 1e9)
        )?;
    }

    for s in &inst.sources {
        let g = 10f32.powf(inst.gain_db / 20.0);
        let ch = usize::from(s.channels());
        let (l, r): (Vec<f32>, Vec<f32>) = s
            .data()
            .chunks(ch)
            .map(|f| (f[0] * g, f[ch - 1] * g))
            .unzip();
        let file: String = s
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '#' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        write_wav(
            &o.out.join("takes").join(format!("{file}.wav")),
            RATE,
            &l,
            &r,
        )?;
    }

    let (mut node, _, _) = synth(&o.engine, o.preset.as_deref())?;
    let (l, r) = play_synth(node.as_mut());
    write_wav(&o.out.join("synth.wav"), RATE, &l, &r)?;
    let takes = inst.sources.len();
    let mut project = Project::new();
    inst.put(&mut project, 0)?;
    let (l, r) = play_sampler(&project);
    write_wav(&o.out.join("sampled.wav"), RATE, &l, &r)?;
    writeln!(
        report,
        "synth.wav and sampled.wav: the same phrase (C3 to C5 arpeggio, then C3 G3 E4 held), \
         from the synth and from the sampled instrument"
    )?;

    let path = o.out.join("autosample-report.txt");
    fs::write(&path, &report)?;
    print!("{report}");
    println!("wrote {takes} takes in {}", o.out.join("takes").display());
    println!("wrote {}", path.display());
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("autosample: {err}");
        std::process::exit(1);
    }
}
