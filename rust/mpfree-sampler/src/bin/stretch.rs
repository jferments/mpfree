//! `mpfree-stretch`: hear the time stretch and pitch shift on a WAV file,
//! on the workstation. Workstation only: no MPC, ALSA or network access. It
//! reads one WAV and writes only inside the output folder it is given.
//!
//! With no `--ratio` or `--semitones` it renders eight examples, each to
//! its own WAV, and writes `stretch-report.txt` with how long each took
//! (and, with `--demo`, `demo-input.wav`, the source):
//!
//! - `slower-150.wav`: half as long again, pitch unchanged.
//! - `faster-75.wav`: three quarters of the length.
//! - `twice-as-long.wav`: twice the length.
//! - `slower-150-no-hits.wav`: as `slower-150` with the hits left to the
//!   stretch, to hear what keeping them sharp does.
//! - `up-5.wav`, `down-7.wav`, `octave-up.wav`: the pitch moved, the timing
//!   kept.
//! - `warped.wav`: warp markers: the first half twice as long, the second
//!   at two thirds.
//!
//! With `--ratio R` and/or `--semitones S` it renders just `custom.wav`.
//! With `--calibration FILE` (the MPC's, `docs/perf/mpc-one-calibration.txt`)
//! the report adds an estimate of each render's time on the MPC.
#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use mpfree_prof::model::{measure, Calibration, Model};
use mpfree_sampler::chop::demo_beat;
use mpfree_sampler::PreparedSample;
use mpfree_stretch::{Job, Settings, TimeMap};

#[path = "shared/wav_out.rs"]
mod wav_out;
use wav_out::write_wav;

const DEMO_RATE: u32 = 44_100;
/// Output frames per step, as the worker thread will run it.
const STEP: usize = 4096;
const USAGE: &str = "\
Usage: mpfree-stretch INPUT.wav OUTPUT_DIR [options]
       mpfree-stretch --demo OUTPUT_DIR [options]   (makes a chord and a drum loop to use)

Options:
  --ratio R           output length over input length, 0.1 to 10 (renders only custom.wav)
  --semitones S       pitch shift, -24 to 24 (renders only custom.wav)
  --window N          analysis window in frames, a power of two from 256 to 16384
                      (default about 46 ms)
  --no-hits           don't keep hits sharp
  --sensitivity X     how keen the hit finder is, 0 to 1 (default 0.6)
  --calibration FILE  add an MPC time estimate from this calibration file

Input: WAV, mono or stereo, 16/24/32-bit PCM or 32-bit float, 8-192 kHz.
Outputs are 16-bit stereo at the input's rate.
Workstation only; writes only inside OUTPUT_DIR.";

struct Options {
    input: Option<PathBuf>,
    out: PathBuf,
    ratio: Option<f64>,
    semitones: Option<f32>,
    window: usize,
    hits: bool,
    sensitivity: f32,
    calibration: Option<PathBuf>,
}

fn parse() -> Result<Options, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return Err(USAGE.into());
    }
    let mut pos = Vec::new();
    let mut demo = false;
    let mut o = Options {
        input: None,
        out: PathBuf::new(),
        ratio: None,
        semitones: None,
        window: 0,
        hits: true,
        sensitivity: Settings::default().sensitivity,
        calibration: None,
    };
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = || {
            args.get(i + 1)
                .ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"))
        };
        match a {
            "--demo" => demo = true,
            "--no-hits" => o.hits = false,
            "--ratio" => {
                o.ratio = Some(value()?.parse()?);
                i += 1;
            }
            "--semitones" => {
                o.semitones = Some(value()?.parse()?);
                i += 1;
            }
            "--window" => {
                o.window = value()?.parse()?;
                i += 1;
            }
            "--sensitivity" => {
                o.sensitivity = value()?.parse()?;
                i += 1;
            }
            "--calibration" => {
                o.calibration = Some(PathBuf::from(value()?));
                i += 1;
            }
            _ if a.starts_with("--") => return Err(format!("unknown option {a}\n\n{USAGE}").into()),
            _ => pos.push(PathBuf::from(a)),
        }
        i += 1;
    }
    match (demo, pos.len()) {
        (true, 1) => o.out = pos.remove(0),
        (false, 2) => {
            o.out = pos.remove(1);
            o.input = Some(pos.remove(0));
        }
        _ => return Err(USAGE.into()),
    }
    Ok(o)
}

/// A slow A minor chord, slightly detuned between left and right, then a
/// two-bar drum loop: something tonal and something percussive.
fn demo_source(rate: u32) -> Vec<f32> {
    let r = rate as f32;
    let chord_frames = rate as usize * 4;
    let mut out = Vec::with_capacity(chord_frames * 2);
    for i in 0..chord_frames {
        let t = i as f32 / r;
        let env = (t / 0.3).min(1.0) * ((4.0 - t) / 0.5).clamp(0.0, 1.0);
        let tone = |f: f32| (t * f * std::f32::consts::TAU).sin();
        let l = 0.25 * env * (tone(220.0) + 0.7 * tone(261.63) + 0.6 * tone(329.63));
        let rr =
            0.25 * env * (tone(220.0) + 0.7 * tone(261.63 * 1.002) + 0.6 * tone(329.63 * 0.998));
        out.extend([l, rr]);
    }
    let (beat, _) = demo_beat(96.0, 2, rate as usize / 4, rate);
    out.extend(beat.iter().flat_map(|&x| [x, x]));
    out
}

struct Render {
    name: String,
    what: String,
    map: TimeMap,
    settings: Settings,
}

fn run() -> Result<(), Box<dyn Error>> {
    let o = parse()?;
    let (data, rate, label) = match &o.input {
        Some(path) => {
            let s = PreparedSample::read_wav(path)?;
            let data: Vec<f32> = s.frame_slice().iter().flat_map(|f| [f[0], f[1]]).collect();
            (data, s.sample_rate(), path.display().to_string())
        }
        None => (
            demo_source(DEMO_RATE),
            DEMO_RATE,
            "demo chord and drum loop".to_string(),
        ),
    };
    let frames = (data.len() / 2) as u64;
    if frames < u64::from(rate) / 10 {
        return Err("the input is shorter than 0.1 s".into());
    }
    let model = match &o.calibration {
        Some(path) => {
            let device = Calibration::parse(&fs::read_to_string(path)?)?;
            Some((Model::new(&measure(2000, 7), &device)?, device.machine))
        }
        None => None,
    };
    fs::create_dir_all(&o.out)?;
    if o.input.is_none() {
        let (l, r): (Vec<f32>, Vec<f32>) = data.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
        write_wav(&o.out.join("demo-input.wav"), rate, &l, &r)?;
    }

    let base = Settings {
        window: o.window,
        transients: o.hits,
        sensitivity: o.sensitivity,
        ..Settings::default()
    };
    let plain =
        |name: &str, what: &str, ratio: f64, semitones: f32| -> Result<Render, Box<dyn Error>> {
            Ok(Render {
                name: name.to_string(),
                what: what.to_string(),
                map: TimeMap::ratio(frames, ratio)?,
                settings: Settings { semitones, ..base },
            })
        };
    let renders = if o.ratio.is_some() || o.semitones.is_some() {
        let (r, s) = (o.ratio.unwrap_or(1.0), o.semitones.unwrap_or(0.0));
        vec![plain(
            "custom",
            &format!("ratio {r}, {s:+} semitones"),
            r,
            s,
        )?]
    } else {
        let n = frames as f64;
        vec![
            plain("slower-150", "1.5 times as long", 1.5, 0.0)?,
            plain("faster-75", "0.75 times as long", 0.75, 0.0)?,
            plain("twice-as-long", "2 times as long", 2.0, 0.0)?,
            Render {
                settings: Settings {
                    transients: false,
                    ..base
                },
                ..plain(
                    "slower-150-no-hits",
                    "1.5 times as long, hits not kept sharp",
                    1.5,
                    0.0,
                )?
            },
            plain("up-5", "5 semitones up", 1.0, 5.0)?,
            plain("down-7", "7 semitones down", 1.0, -7.0)?,
            plain("octave-up", "12 semitones up", 1.0, 12.0)?,
            Render {
                name: "warped".into(),
                what: "first half 2 times as long, second half 2/3".into(),
                map: TimeMap::new(vec![(0.0, 0.0), (n / 2.0, n), (n, n + n / 3.0)])?,
                settings: base,
            },
        ]
    };

    let input: std::sync::Arc<[f32]> = data.into();
    let secs_in = frames as f64 / f64::from(rate);
    let mut report = String::new();
    writeln!(report, "mpfree-stretch report")?;
    writeln!(report, "input: {label}, {secs_in:.2} s at {rate} Hz")?;
    if let Some((_, machine)) = &model {
        writeln!(report, "MPC estimate from the calibration of: {machine}")?;
    }
    writeln!(report)?;
    for r in renders {
        let t = Instant::now();
        let mut job = Job::new(input.clone(), 2, rate, r.map, r.settings)?;
        let setup = t.elapsed().as_secs_f64();
        let mut steps = 0u32;
        while !job.run(STEP).finished {
            steps += 1;
        }
        let secs = t.elapsed().as_secs_f64();
        let (found, restarts) = job.hits();
        let out = job.into_output().ok_or("unfinished job")?;
        let (l, rr): (Vec<f32>, Vec<f32>) = out.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
        let path = o.out.join(format!("{}.wav", r.name));
        write_wav(&path, rate, &l, &rr)?;
        let secs_out = l.len() as f64 / f64::from(rate);
        writeln!(report, "{}.wav: {}", r.name, r.what)?;
        writeln!(
            report,
            "  {secs_out:.2} s out; {found} hits found, {restarts} kept sharp; \
             {:.0} ms here ({:.0} ms finding hits, {steps} steps), {:.0}x real time",
            secs * 1000.0,
            setup * 1000.0,
            secs_out / secs
        )?;
        if let Some((m, _)) = &model {
            let (est, lo, hi) = m.estimate(secs * 1e9);
            writeln!(
                report,
                "  MPC estimate {:.1} s (range {:.1} to {:.1} s), {:.1}x real time",
                est / 1e9,
                lo / 1e9,
                hi / 1e9,
                secs_out / (est / 1e9)
            )?;
        }
        println!("wrote {}", path.display());
    }
    let report_path = o.out.join("stretch-report.txt");
    fs::write(&report_path, &report)?;
    print!("{report}");
    println!("wrote {}", report_path.display());
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("mpfree-stretch: {err}");
        std::process::exit(1);
    }
}
