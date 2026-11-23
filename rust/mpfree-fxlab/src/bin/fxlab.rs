//! fxlab's command line.
//!
//! ```text
//! fxlab cards [--rate HZ] [--quick] [--presets] [--effect ID]... [--out DIR]
//!     Report cards for every registry effect (or the ones named), at its
//!     defaults and, with --presets, at each factory preset. Prints a
//!     summary and writes DIR/cards.json (default ./fxlab-out).
//! fxlab programme [--rate HZ] [--short] [--out DIR]
//!     Writes the capture programme (programme.wav, or short.wav for
//!     setting sweeps) and its schedule (schedule.tsv / short.tsv) for
//!     recording a reference device.
//! fxlab cut RECORDING.wav SCHEDULE.tsv [--out DIR]
//!     Finds the programme in a recording and writes one WAV per segment.
//! fxlab analyse CAPTURE.wav... --schedule SCHEDULE.tsv [--gain DB] [--reverb] [--dynamics] [--out DIR]
//!     Report cards read from captures of the programme (an AIR effect
//!     recorded by FXProbe Tap); writes DIR/captures.json.
//! fxlab plan (--effect ID | --controls LIST.tsv) [--points N] [--grid N] [--random N]
//!            [--seed N] [--gains DB,DB...] [--programme FILE] [--out DIR]
//!     A sweep plan: defaults, each control across its range, pair grids and
//!     random settings, in every context. Writes DIR/plan.tsv.
//! fxlab sweep --effect ID [plan options] [--out DIR]
//!     Renders one of our effects at every setting of its plan through the
//!     short programme and measures it as a capture would be; writes
//!     DIR/plan.tsv and DIR/sweep.json.
//! fxlab quality [--class CLASS]... [--rate HZ] [--out DIR]
//!     CLASS: delay, eq, reverb, drive, crush, compressor, filter, lofi, amp,
//!     modulation, limiter, gate, deesser, multiband, ducker, transient,
//!     strip, movement, pitch or voice.
//!     The FX quality report: every target of each class (all of them when
//!     none is named), met or not; writes DIR/quality.json
//!     (docs/dev/design/fx-quality.md).
//! fxlab synth [--rate HZ] [--renders] [--engine NAME] [--out DIR]
//!     The synth library's baseline: every engine and factory preset
//!     against its targets, met or not, and a count by measure; writes
//!     DIR/synth.txt. With --renders, also each preset's listening phrase
//!     as DIR/renders/ENGINE/NN-PRESET.wav and each drum kit's tour
//!     (docs/dev/design/synth-quality.md). With --engine, only that
//!     engine's presets (and renders), for a quick look while working on it.
//! fxlab reference [--sixteen] [--out DIR]
//!     Y8 of the DSP quality plan: each aliasing case against a render at
//!     8 times the rate brought down to 44.1 kHz, and with --sixteen the
//!     8x reference against a 16x one; writes DIR/reference.txt
//!     (docs/dev/design/dsp-quality.md).
//! ```

use std::path::PathBuf;

use mpfree_fxlab::card::{self, Options};
use mpfree_fxlab::dut::{Dut, NodeDut};
use mpfree_fxlab::plan::{self, Context, ControlList, PlanOptions};
use mpfree_fxlab::programme::{self, Programme, Schedule};
use mpfree_fxlab::{capture, quality, reference, synth, wav};

fn usage() -> ! {
    eprintln!("usage: fxlab cards [--rate HZ] [--quick] [--presets] [--effect ID]... [--out DIR]");
    eprintln!("       fxlab programme [--rate HZ] [--short] [--out DIR]");
    eprintln!("       fxlab cut RECORDING.wav SCHEDULE.tsv [--out DIR]");
    eprintln!("       fxlab analyse CAPTURE.wav... --schedule SCHEDULE.tsv [--gain DB] [--reverb] [--dynamics] [--out DIR]");
    eprintln!("       fxlab plan (--effect ID | --controls LIST.tsv) [--points N] [--grid N] [--random N] [--seed N]");
    eprintln!("                  [--gains DB,DB...] [--programme FILE] [--out DIR]");
    eprintln!("       fxlab sweep --effect ID [plan options] [--out DIR]");
    eprintln!("       fxlab synth [--rate HZ] [--renders] [--engine NAME] [--out DIR]");
    eprintln!("       fxlab reference [--sixteen] [--out DIR]");
    eprintln!("       fxlab quality [--class delay|eq|reverb|drive|crush|compressor|filter|lofi|amp|modulation|limiter|gate|deesser|multiband|ducker|transient|strip|movement|pitch|voice]... [--rate HZ] [--out DIR]");
    std::process::exit(2);
}

struct Args {
    rate: u32,
    quick: bool,
    presets: bool,
    short: bool,
    effects: Vec<String>,
    classes: Vec<quality::Class>,
    out: PathBuf,
    rest: Vec<String>,
    schedule: Option<PathBuf>,
    gain: f64,
    reverb: bool,
    dynamics: bool,
    renders: bool,
    sixteen: bool,
    engine: Option<String>,
    controls: Option<PathBuf>,
    plan: PlanOptions,
}

fn parse(mut it: impl Iterator<Item = String>) -> Args {
    let mut a = Args {
        rate: 44_100,
        quick: false,
        presets: false,
        short: false,
        effects: Vec::new(),
        classes: Vec::new(),
        out: PathBuf::from("fxlab-out"),
        rest: Vec::new(),
        schedule: None,
        gain: 0.0,
        reverb: false,
        dynamics: false,
        renders: false,
        sixteen: false,
        engine: None,
        controls: None,
        plan: PlanOptions::default(),
    };
    let mut gains: Option<Vec<f64>> = None;
    let mut programme_file: Option<String> = None;
    while let Some(x) = it.next() {
        let mut num = || -> f64 {
            it.next()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| usage())
        };
        match x.as_str() {
            "--rate" => a.rate = num() as u32,
            "--quick" => a.quick = true,
            "--presets" => a.presets = true,
            "--short" => a.short = true,
            "--gain" => a.gain = num(),
            "--reverb" => a.reverb = true,
            "--dynamics" => a.dynamics = true,
            "--renders" => a.renders = true,
            "--sixteen" => a.sixteen = true,
            "--engine" => a.engine = Some(it.next().unwrap_or_else(|| usage())),
            "--points" => a.plan.sweep_points = num() as usize,
            "--grid" => a.plan.grid_points = num() as usize,
            "--random" => a.plan.random = num() as usize,
            "--seed" => a.plan.seed = num() as u64,
            "--schedule" => a.schedule = Some(PathBuf::from(it.next().unwrap_or_else(|| usage()))),
            "--controls" => a.controls = Some(PathBuf::from(it.next().unwrap_or_else(|| usage()))),
            "--programme" => programme_file = Some(it.next().unwrap_or_else(|| usage())),
            "--gains" => {
                let v = it.next().unwrap_or_else(|| usage());
                gains = Some(
                    v.split(',')
                        .map(|g| g.trim().parse().unwrap_or_else(|_| usage()))
                        .collect(),
                );
            }
            "--effect" => a.effects.push(it.next().unwrap_or_else(|| usage())),
            "--class" => a.classes.push(
                it.next()
                    .and_then(|c| quality::Class::parse(&c))
                    .unwrap_or_else(|| usage()),
            ),
            "--out" => a.out = PathBuf::from(it.next().unwrap_or_else(|| usage())),
            _ if x.starts_with("--") => usage(),
            _ => a.rest.push(x),
        }
    }
    let prog = programme_file.unwrap_or_else(|| "short.wav".into());
    a.plan.contexts = gains
        .unwrap_or_else(|| vec![0.0])
        .into_iter()
        .map(|gain_db| Context {
            programme: prog.clone(),
            gain_db,
        })
        .collect();
    a
}

/// The control list for --effect or --controls.
fn controls(a: &Args) -> Result<ControlList, Box<dyn std::error::Error>> {
    if let Some(path) = &a.controls {
        return Ok(ControlList::from_text(&std::fs::read_to_string(path)?)?);
    }
    let id = a
        .effects
        .first()
        .ok_or("--effect or --controls is needed")?;
    let e = mpfree_fx::EFFECTS
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("no effect {id}"))?;
    Ok(ControlList::from_registry(e))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_else(|| usage());
    let a = parse(args);
    std::fs::create_dir_all(&a.out)?;
    match cmd.as_str() {
        "cards" => {
            let opts = Options {
                rate: a.rate,
                quick: a.quick,
            };
            let mut cards = Vec::new();
            for e in mpfree_fx::EFFECTS.iter() {
                if !a.effects.is_empty() && !a.effects.iter().any(|x| x == e.id) {
                    continue;
                }
                let mut settings: Vec<Option<&mpfree_fx::Preset>> = vec![None];
                if a.presets {
                    settings.extend(e.presets.iter().map(Some));
                }
                for p in settings {
                    let c = card::measure(e, p, opts);
                    println!(
                        "{:<34} {:<22} gain1k {:>7} thd1k-18 {:>8} alias5k0 {:>8} lufsΔ {:>7} idle {:>8}",
                        c.name,
                        c.preset.as_deref().unwrap_or("(defaults)"),
                        fmt(c.get("resp_1k_db")),
                        fmt(c.get("thd_1k_-18_db")),
                        fmt(c.get("alias_5k_0_db")),
                        fmt(c.get("lufs_delta_pink")),
                        fmt(c.get("idle_db")),
                    );
                    cards.push(c);
                }
            }
            let path = a.out.join("cards.json");
            std::fs::write(&path, card::to_json(&cards))?;
            eprintln!("wrote {} cards to {}", cards.len(), path.display());
        }
        "programme" => {
            let (p, wav_name, tsv_name) = if a.short {
                (Programme::short(a.rate), "short.wav", "short.tsv")
            } else {
                (Programme::standard(a.rate), "programme.wav", "schedule.tsv")
            };
            let (audio, sched) = p.render();
            wav::write(&a.out.join(wav_name), &audio)?;
            std::fs::write(a.out.join(tsv_name), sched.to_text())?;
            eprintln!(
                "wrote {wav_name} ({:.1} s) and {tsv_name} to {}",
                audio.len() as f64 / a.rate as f64,
                a.out.display()
            );
        }
        "analyse" => {
            let sched_path = a.schedule.as_ref().unwrap_or_else(|| usage());
            let sched = Schedule::from_text(&std::fs::read_to_string(sched_path)?)?;
            if a.rest.is_empty() {
                usage();
            }
            let opts = capture::Options {
                gain_db: a.gain,
                reverb: a.reverb,
                dynamics: a.dynamics,
            };
            let mut cards = Vec::new();
            for path in &a.rest {
                let rec = wav::read(std::path::Path::new(path))?;
                if rec.rate != sched.rate {
                    return Err(format!(
                        "{path} is at {} Hz, the schedule at {}",
                        rec.rate, sched.rate
                    )
                    .into());
                }
                let name = std::path::Path::new(path)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let c = capture::analyse_recording("capture", &name, &rec, &sched, opts);
                println!(
                    "{:<40} gain1k {:>7} thd1k {:>8} alias5k {:>8} lufsΔ {:>7} idle {:>8}",
                    name,
                    fmt(c.get("resp_1k_db")),
                    fmt(c
                        .metrics
                        .iter()
                        .find(|m| m.0.starts_with("thd_1k_"))
                        .map(|m| m.1)),
                    fmt(c
                        .metrics
                        .iter()
                        .find(|m| m.0.starts_with("alias_5k_"))
                        .map(|m| m.1)),
                    fmt(c.get("lufs_delta_pink")),
                    fmt(c.get("idle_db")),
                );
                cards.push(c);
            }
            let path = a.out.join("captures.json");
            std::fs::write(&path, card::to_json(&cards))?;
            eprintln!("wrote {} cards to {}", cards.len(), path.display());
        }
        "plan" => {
            let list = controls(&a)?;
            let runs = plan::plan(&list, &a.plan);
            let path = a.out.join("plan.tsv");
            std::fs::write(&path, plan::to_tsv(&list, &runs))?;
            let secs = Programme::short(a.rate).render().0.len() as f64 / a.rate as f64;
            eprintln!(
                "wrote {} runs over {} controls to {} (about {:.1} h of capture at {:.0} s each, one slot)",
                runs.len(),
                list.controls.len(),
                path.display(),
                runs.len() as f64 * secs / 3600.0,
                secs
            );
        }
        "sweep" => {
            let list = controls(&a)?;
            let id = a.effects.first().ok_or("sweep needs --effect")?;
            let runs = plan::plan(&list, &a.plan);
            std::fs::write(a.out.join("plan.tsv"), plan::to_tsv(&list, &runs))?;
            let prog = Programme::short(a.rate);
            let cat = mpfree_fx::EFFECTS
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.category);
            let opts = |gain_db| capture::Options {
                gain_db,
                reverb: matches!(cat, Some(mpfree_fx::registry::Category::DelayReverb)),
                dynamics: matches!(cat, Some(mpfree_fx::registry::Category::Dynamics)),
            };
            let mut cards = Vec::new();
            for r in &runs {
                let mut d =
                    NodeDut::from_registry(id, a.rate).ok_or_else(|| format!("no effect {id}"))?;
                for (c, v) in list.controls.iter().zip(&r.values) {
                    if let Some(pid) = c.id {
                        d.set(pid, *v);
                    }
                }
                let (rec, sched) = capture::render(&mut d, &prog, r.context.gain_db);
                let pieces = programme::cut(&rec, &sched, 0);
                cards.push(capture::analyse(
                    id,
                    &format!("run {}", r.id),
                    &pieces,
                    a.rate,
                    opts(r.context.gain_db),
                ));
            }
            let path = a.out.join("sweep.json");
            std::fs::write(&path, card::to_json(&cards))?;
            eprintln!(
                "wrote {} runs to {} and plan.tsv",
                cards.len(),
                path.display()
            );
        }
        "quality" => {
            let classes = if a.classes.is_empty() {
                quality::Class::ALL.to_vec()
            } else {
                a.classes.clone()
            };
            let mut reports = Vec::new();
            for c in classes {
                let r = quality::report(c, a.rate);
                print!("{}", quality::to_text(&r));
                reports.push(r);
            }
            let path = a.out.join("quality.json");
            std::fs::write(&path, quality::to_json(&reports))?;
            eprintln!("wrote {}", path.display());
        }
        "synth" => {
            let only = a.engine.as_deref().map(|n| {
                synth::Engine::from_name(n).unwrap_or_else(|| {
                    eprintln!("fxlab: no engine called {n}");
                    std::process::exit(2)
                })
            });
            let rows = match only {
                Some(e) => synth::engine_report(a.rate, e),
                None => synth::report(a.rate),
            };
            let text = format!("{}\n{}", synth::to_text(&rows), synth::summary(&rows));
            print!("{text}");
            std::fs::create_dir_all(&a.out)?;
            std::fs::write(a.out.join("synth.txt"), &text)?;
            if a.renders {
                let engines = match only {
                    Some(e) => vec![e],
                    None => synth::Engine::ALL.to_vec(),
                };
                for e in engines {
                    let dir = a.out.join("renders").join(e.name());
                    std::fs::create_dir_all(&dir)?;
                    for (i, name) in e.presets().iter().enumerate() {
                        let file: String = name
                            .chars()
                            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                            .collect();
                        let path = dir.join(format!("{:02}-{file}.wav", i + 1));
                        wav::write(&path, &synth::phrase(e, i, a.rate))?;
                    }
                }
                if only.is_none() {
                    let dir = a.out.join("renders").join("Drums");
                    std::fs::create_dir_all(&dir)?;
                    for (i, name) in synth::kits().iter().enumerate() {
                        let file: String = name
                            .chars()
                            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                            .collect();
                        let path = dir.join(format!("{:02}-{file}.wav", i + 1));
                        wav::write(&path, &synth::kit_tour(i, a.rate))?;
                    }
                }
            }
            eprintln!("wrote {}", a.out.display());
        }
        "reference" => {
            let text = reference::to_text(&reference::y8(a.sixteen));
            print!("{text}");
            std::fs::create_dir_all(&a.out)?;
            std::fs::write(a.out.join("reference.txt"), &text)?;
            eprintln!("wrote {}", a.out.display());
        }
        "cut" => {
            if a.rest.len() != 2 {
                usage();
            }
            let rec = wav::read(std::path::Path::new(&a.rest[0]))?;
            let sched = Schedule::from_text(&std::fs::read_to_string(&a.rest[1])?)?;
            if sched.rate != rec.rate {
                return Err(
                    format!("recording at {} Hz, schedule at {}", rec.rate, sched.rate).into(),
                );
            }
            let at = programme::locate(&rec, 10.0);
            for (e, piece) in programme::cut(&rec, &sched, at) {
                let name: String = e
                    .name
                    .chars()
                    .map(|c| {
                        if c.is_ascii_alphanumeric() || c == '-' {
                            c
                        } else {
                            '_'
                        }
                    })
                    .collect();
                wav::write(&a.out.join(format!("{:02}-{name}.wav", e.id)), &piece)?;
            }
            eprintln!(
                "programme found at frame {at}; wrote {} segments to {}",
                sched.entries.len(),
                a.out.display()
            );
        }
        _ => usage(),
    }
    Ok(())
}

fn fmt(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.1}")).unwrap_or_else(|| "-".into())
}
