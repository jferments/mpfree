//! `mpfree-chop`: try the sampler's chop modes on a WAV file, on the
//! workstation. Workstation only: no MPC, ALSA or network access. It reads
//! one WAV and writes only inside the output folder it is given.
//!
//! It analyses the file, chops it, puts the slices on pads, plays them back
//! through the sampler engine, and writes:
//!
//! - `chop-report.txt`: tempo, root pitch, the slice table (with each
//!   slice's note) and the rebuild check.
//! - `rebuild.wav`: the pads hit in order at their slice starts. It should
//!   sound exactly like the input; the report says how close it is.
//! - `one-by-one.wav`: each slice alone, with a short gap after it.
//! - `reversed.wav`: the slices in reverse order on the original timing,
//!   each hit cutting the last (a first taste of rechopping).
//! - `slices/slice-NN.wav`: every slice as its own file.
#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use mpfree_analysis::{note_name, SilenceSettings};
use mpfree_sampler::chop::{demo_beat, SourceAnalysis};
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{ChopSet, Project, Source};
use mpfree_sampler::PreparedSample;

/// Rate of the --demo loop.
#[path = "shared/wav_out.rs"]
mod wav_out;
use wav_out::write_wav;

const DEMO_RATE: u32 = 44_100;
const USAGE: &str = "\
Usage: mpfree-chop INPUT.wav OUTPUT_DIR [options]
       mpfree-chop --demo OUTPUT_DIR [options]   (makes a 2-bar drum loop to chop)

Options:
  --mode transient|beat|grid|silence   chop mode (default transient)
  --sensitivity 0..1                   transient sensitivity (default 0.6)
  --slices N                           grid slices (default 16)
  --beats N                            beat mode: beats per slice (default 1; 0.5 = eighths)
  --bpm X                              beat mode: tempo to use instead of the detected one
  --threshold-db X                     silence mode: silence level (default -48)

Input: WAV, mono or stereo, 16/24/32-bit PCM or 32-bit float, 8-192 kHz.
Outputs are written at the input's rate.
Workstation only; writes only inside OUTPUT_DIR.";

struct Options {
    input: Option<PathBuf>,
    out: PathBuf,
    mode: String,
    sensitivity: f32,
    slices: usize,
    beats: f64,
    bpm: Option<f64>,
    threshold_db: f32,
}

fn parse() -> Result<Options, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") || args.len() < 2 {
        return Err(USAGE.into());
    }
    let (input, out, rest) = if args[0] == "--demo" {
        (None, PathBuf::from(&args[1]), &args[2..])
    } else {
        if args.len() < 2 || args[1].starts_with("--") {
            return Err(USAGE.into());
        }
        (
            Some(PathBuf::from(&args[0])),
            PathBuf::from(&args[1]),
            &args[2..],
        )
    };
    let mut o = Options {
        input,
        out,
        mode: "transient".into(),
        sensitivity: 0.6,
        slices: 16,
        beats: 1.0,
        bpm: None,
        threshold_db: -48.0,
    };
    let mut it = rest.iter();
    while let Some(flag) = it.next() {
        let value = it
            .next()
            .ok_or_else(|| format!("{flag} needs a value\n\n{USAGE}"))?;
        match flag.as_str() {
            "--mode" => o.mode = value.clone(),
            "--sensitivity" => o.sensitivity = value.parse()?,
            "--slices" => o.slices = value.parse()?,
            "--beats" => o.beats = value.parse()?,
            "--bpm" => o.bpm = Some(value.parse()?),
            "--threshold-db" => o.threshold_db = value.parse()?,
            _ => return Err(format!("unknown option {flag}\n\n{USAGE}").into()),
        }
    }
    if !["transient", "beat", "grid", "silence"].contains(&o.mode.as_str()) {
        return Err(format!("unknown mode {}\n\n{USAGE}", o.mode).into());
    }
    if !(0.0..=1.0).contains(&o.sensitivity) {
        return Err("--sensitivity must be between 0 and 1".into());
    }
    Ok(o)
}

/// Plays note-ons at the given frames through a fresh engine running at
/// the source's rate, so slices play back without resampling.
fn render(p: &Project, rate: u32, hits: &[(u8, u64)], frames: usize) -> (Vec<f32>, Vec<f32>) {
    let mut e = Engine::new(rate, 32);
    e.set_view(EngineView::build(p, rate));
    let mut l = vec![0.0f32; frames];
    let mut r = vec![0.0f32; frames];
    let mut next = 0;
    let mut pos = 0u64;
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        let end = pos + a.len() as u64;
        while next < hits.len() && hits[next].1 < end {
            e.schedule(
                (hits[next].1.saturating_sub(pos)) as u32,
                Event::NoteOn {
                    pad: hits[next].0,
                    velocity: 127,
                },
            );
            next += 1;
        }
        e.render(a, b);
        pos = end;
    }
    (l, r)
}

fn run() -> Result<(), Box<dyn Error>> {
    let o = parse()?;
    fs::create_dir_all(o.out.join("slices"))?;
    let (source, input_name) = match &o.input {
        Some(path) => {
            let p =
                PreparedSample::read_wav(path).map_err(|e| format!("{}: {e:?}", path.display()))?;
            (
                Source::from_prepared("input", &p),
                path.display().to_string(),
            )
        }
        None => {
            let (mono, _) = demo_beat(96.0, 2, 0, DEMO_RATE);
            let demo = o.out.join("demo-input.wav");
            write_wav(&demo, DEMO_RATE, &mono, &mono)?;
            (
                Source::new("demo", DEMO_RATE, 1, mono)?,
                format!("{} (made by --demo: 2 bars at 96 BPM)", demo.display()),
            )
        }
    };
    let analysis = SourceAnalysis::analyse(&source);
    let chop: ChopSet = match o.mode.as_str() {
        "transient" => analysis.transient("chop", &source, o.sensitivity)?,
        "beat" => analysis.beats("chop", &source, o.bpm, o.beats)?,
        "grid" => ChopSet::grid("chop", &source, o.slices)?,
        _ => analysis.silence(
            "chop",
            &source,
            SilenceSettings {
                threshold_db: o.threshold_db,
                ..SilenceSettings::default()
            },
        )?,
    };
    let n = chop.slices().min(128);

    let mut p = Project::new();
    p.add_source(source.clone());
    let c = p.add_chop_set(chop.clone())?;
    p.slices_to_pads(c, 0, n, 0)?;
    // +3 dB undoes the centre-pan law, so the rebuild matches the input level.
    for i in 0..n {
        p.edit_pad(i, None, |pad| pad.level_db = 3.0103)?;
    }

    let frames = source.frames() as usize;
    let rate = source.rate();
    let starts: Vec<u64> = (0..n).map(|i| chop.bounds(i).unwrap().0).collect();
    let in_order: Vec<(u8, u64)> = starts
        .iter()
        .enumerate()
        .map(|(i, &s)| (i as u8, s))
        .collect();
    let (rl, rr) = render(&p, rate, &in_order, frames);
    let data = source.data();
    let ch = usize::from(source.channels());
    let mut worst = 0.0f32;
    for f in 0..frames {
        let (x0, x1) = (data[f * ch], data[f * ch + ch - 1]);
        worst = worst.max((rl[f] - x0).abs()).max((rr[f] - x1).abs());
    }
    write_wav(&o.out.join("rebuild.wav"), rate, &rl, &rr)?;

    let gap = u64::from(rate) * 3 / 10;
    let mut t = 0u64;
    let mut one_by_one = Vec::with_capacity(n);
    for i in 0..n {
        one_by_one.push((i as u8, t));
        let (a, b) = chop.bounds(i).unwrap();
        t += b - a + gap;
    }
    let (l, r) = render(&p, rate, &one_by_one, t as usize);
    write_wav(&o.out.join("one-by-one.wav"), rate, &l, &r)?;

    let mut q = p.clone();
    for i in 0..n {
        q.edit_pad(i, None, |pad| pad.mute_group = Some(1))?;
    }
    let reversed: Vec<(u8, u64)> = starts
        .iter()
        .enumerate()
        .map(|(i, &s)| ((n - 1 - i) as u8, s))
        .collect();
    let (l, r) = render(&q, rate, &reversed, frames);
    write_wav(&o.out.join("reversed.wav"), rate, &l, &r)?;

    for i in 0..n {
        let (a, b) = chop.bounds(i).unwrap();
        let (l, r) = render(&p, rate, &[(i as u8, 0)], (b - a) as usize);
        write_wav(
            &o.out.join("slices").join(format!("slice-{:02}.wav", i + 1)),
            rate,
            &l,
            &r,
        )?;
    }

    let mut rep = String::new();
    let secs = |f: u64| f as f64 / f64::from(rate);
    writeln!(rep, "MPFREE_CHOP_V2")?;
    writeln!(rep, "input={input_name}")?;
    writeln!(
        rep,
        "frames={frames} seconds={:.3} channels={} rate={rate}",
        secs(frames as u64),
        source.channels()
    )?;
    match analysis.tempo() {
        Some(t) => writeln!(
            rep,
            "tempo_estimate={:.2} confidence={:.2}",
            t.bpm, t.confidence
        )?,
        None => writeln!(rep, "tempo_estimate=none")?,
    }
    match analysis.loop_tempo() {
        Some(b) => writeln!(
            rep,
            "loop_tempo={b:.3} (if the file is a loop cut to whole beats)"
        )?,
        None => writeln!(rep, "loop_tempo=none")?,
    }
    match analysis.pitch() {
        Some(p) => writeln!(
            rep,
            "root_pitch={} cents={:+.1} hz={:.2} confidence={:.2}",
            note_name(p.note),
            p.cents,
            p.hz,
            p.confidence
        )?,
        None => writeln!(rep, "root_pitch=none")?,
    }
    writeln!(
        rep,
        "mode={} sensitivity={} slices={}",
        o.mode,
        o.sensitivity,
        chop.slices()
    )?;
    if chop.slices() > n {
        writeln!(rep, "note=only the first {n} slices went on pads")?;
    }
    writeln!(
        rep,
        "{:>4} {:>10} {:>10} {:>10} {:>9} {:>11}",
        "pad", "start", "end", "start_s", "length_ms", "note"
    )?;
    for i in 0..n {
        let (a, b) = chop.bounds(i).unwrap();
        let note = analysis.pitch_of(a, b).map_or("-".to_string(), |p| {
            format!("{}{:+.0}", note_name(p.note), p.cents)
        });
        writeln!(
            rep,
            "{:>4} {:>10} {:>10} {:>10.4} {:>9.1} {:>11}",
            i + 1,
            a,
            b,
            secs(a),
            secs(b - a) * 1000.0,
            note
        )?;
    }
    let pass = worst < 1e-4;
    writeln!(rep, "rebuild_max_difference={worst:.7}")?;
    writeln!(
        rep,
        "rebuild={}",
        if pass {
            "PASS (the pads in order play the input back sample for sample)"
        } else {
            "FAIL"
        }
    )?;
    writeln!(rep, "files=chop-report.txt rebuild.wav one-by-one.wav reversed.wav slices/slice-01.wav..slice-{n:02}.wav")?;
    fs::write(o.out.join("chop-report.txt"), &rep)?;
    print!("{rep}");
    println!("output_dir={}", o.out.display());
    if !pass {
        return Err("rebuild check failed".into());
    }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("mpfree-chop: {err}");
        std::process::exit(1);
    }
}
