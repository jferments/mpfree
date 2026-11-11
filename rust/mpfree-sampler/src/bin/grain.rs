//! `mpfree-grain`: hear the grain engine on a WAV file, on the workstation.
//! Workstation only: no MPC, ALSA or network access. It reads one WAV and
//! writes only inside the output folder it is given.
//!
//! It renders eight presets that show what the engine does, each to its own
//! WAV, and writes `grain-report.txt` with the grain counts and how fast
//! each render ran (and, with `--demo`, `demo-input.wav`, the source):
//!
//! - `cloud.wav`: a soft cloud around one spot.
//! - `stretch.wav`: the whole input four times slower, pitch unchanged.
//! - `shimmer.wav`: octaves and fifths above, spread wide.
//! - `reverse-spray.wav`: mostly backwards grains from all over.
//! - `stutter-120.wav`: grains locked to sixteenths at 120 BPM.
//! - `freeze-live.wav`: the input played through as a live signal, with the
//!   granular effect frozen on it partway through.
//! - `pressure-pad.wav`: the input on four granular pads in the sampler
//!   engine, one after another, each pressed harder then lighter: pressure
//!   on grain density, then grain size, then position, then pitch.
//! - `master-freeze.wav`: the input on a pad in the sampler engine, under
//!   the master granular effect half wet. Freeze at 30 % holds that moment
//!   while the input carries on; at 60 % the input stops and the frozen
//!   cloud plays on alone; at 85 % freeze is let go and the cloud fades as
//!   the silence reaches it.
#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use mpfree_grain::{Budget, Clip, GrainVoice, LiveBuffer, Params, TempoSync, Window};
use mpfree_sampler::chop::demo_beat;
use mpfree_sampler::dsp::Ahdsr;
use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::{GrainParams, Layer, MasterFx, Pad, PlayMode, PressureRoute, PressureTarget, Project, Region, Source};
use mpfree_sampler::PreparedSample;

#[path = "shared/wav_out.rs"]
mod wav_out;
use wav_out::write_wav;

const DEMO_RATE: u32 = 44_100;
const BLOCK: usize = 128;
const BUDGET: usize = 256;
const USAGE: &str = "\
Usage: mpfree-grain INPUT.wav OUTPUT_DIR [--seconds N] [--seed N]
       mpfree-grain --demo OUTPUT_DIR [--seconds N] [--seed N]   (makes a chord and a drum loop to use)

Options:
  --seconds N   length of each preset (default 8; stretch is 4x the input, at most 60)
  --seed N      random seed (default 1); the same seed gives the same files

Input: WAV, mono or stereo, 16/24/32-bit PCM or 32-bit float, 8-192 kHz.
Outputs are 16-bit stereo at the input's rate.
Workstation only; writes only inside OUTPUT_DIR.";

struct Options {
    input: Option<PathBuf>,
    out: PathBuf,
    seconds: f64,
    seed: u64,
}

fn parse() -> Result<Options, Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return Err(USAGE.into());
    }
    let mut pos = Vec::new();
    let mut demo = false;
    let mut seconds = 8.0;
    let mut seed = 1;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let value = || args.get(i + 1).ok_or_else(|| format!("{a} needs a value\n\n{USAGE}"));
        match a {
            "--demo" => demo = true,
            "--seconds" => { seconds = value()?.parse()?; i += 1; }
            "--seed" => { seed = value()?.parse()?; i += 1; }
            _ if a.starts_with("--") => return Err(format!("unknown option {a}\n\n{USAGE}").into()),
            _ => pos.push(PathBuf::from(a)),
        }
        i += 1;
    }
    if !(0.5..=60.0).contains(&seconds) { return Err("--seconds must be between 0.5 and 60".into()); }
    match (demo, pos.len()) {
        (true, 1) => Ok(Options { input: None, out: pos.remove(0), seconds, seed }),
        (false, 2) => Ok(Options { out: pos.remove(1), input: Some(pos.remove(0)), seconds, seed }),
        _ => Err(USAGE.into()),
    }
}

/// A slow A minor chord, slightly detuned between left and right, then a
/// two-bar drum loop: something tonal and something percussive to granulate.
fn demo_source(rate: u32) -> Vec<f32> {
    let r = rate as f32;
    let chord_frames = rate as usize * 4;
    let mut out = Vec::with_capacity(chord_frames * 2);
    for i in 0..chord_frames {
        let t = i as f32 / r;
        let env = (t / 0.3).min(1.0) * ((4.0 - t) / 0.5).clamp(0.0, 1.0);
        let tone = |f: f32| (t * f * std::f32::consts::TAU).sin();
        let l = 0.25 * env * (tone(220.0) + 0.7 * tone(261.63) + 0.6 * tone(329.63));
        let rr = 0.25 * env * (tone(220.0) + 0.7 * tone(261.63 * 1.002) + 0.6 * tone(329.63 * 0.998));
        out.extend([l, rr]);
    }
    let (beat, _) = demo_beat(96.0, 2, rate as usize / 4, rate);
    out.extend(beat.iter().flat_map(|&x| [x, x]));
    out
}

struct Rendered {
    name: &'static str,
    left: Vec<f32>,
    right: Vec<f32>,
    /// Grains started, where the render can count them.
    spawned: Option<u64>,
    skipped: u64,
    peak_grains: usize,
    millis: f64,
}

fn render(name: &'static str, data: &[f32], rate: u32, p: Params, frames: usize, seed: u64) -> Rendered {
    let clip = Clip::new(data, 2, rate).expect("stereo input");
    let mut v = GrainVoice::new(rate, seed);
    v.set_params(p);
    v.start();
    let mut budget = Budget::new(BUDGET);
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    let t = Instant::now();
    let mut at = 0;
    while at < frames {
        let n = BLOCK.min(frames - at);
        v.render(&clip, &mut budget, at as u64, &mut left[at..at + n], &mut right[at..at + n]);
        at += n;
    }
    let millis = t.elapsed().as_secs_f64() * 1000.0;
    let s = v.stats();
    Rendered { name, left, right, spawned: Some(s.spawned), skipped: s.skipped, peak_grains: s.peak_active, millis }
}

/// The input streams through as if live, and the granular effect plays on
/// it: dry, then frozen from 40 % to 80 % of the way through while the dry
/// signal fades out, then live again.
fn render_live(data: &[f32], rate: u32, frames: usize, seed: u64) -> Rendered {
    let mut live = LiveBuffer::new(2, rate, rate as usize * 4).expect("buffer");
    let mut v = GrainVoice::new(rate, seed);
    v.set_params(Params { position: 0.85, spray: 0.15, size_ms: 90.0, density: 35.0, pitch_spray: 0.2, spread: 0.7, gain: 0.8, ..Params::default() });
    v.start();
    let mut budget = Budget::new(BUDGET);
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    let input_frames = data.len() / 2;
    let t = Instant::now();
    let mut at = 0;
    while at < frames {
        let n = BLOCK.min(frames - at);
        let frac = at as f64 / frames as f64;
        let frozen = (0.4..0.8).contains(&frac);
        live.set_frozen(frozen);
        let dry = if frozen { 0.0 } else { 0.6 };
        for k in 0..n {
            let f = (at + k) % input_frames;
            let (l, r) = (data[f * 2], data[f * 2 + 1]);
            live.write(&[l, r]);
            left[at + k] = dry * l;
            right[at + k] = dry * r;
        }
        v.render(&live, &mut budget, at as u64, &mut left[at..at + n], &mut right[at..at + n]);
        at += n;
    }
    let millis = t.elapsed().as_secs_f64() * 1000.0;
    let s = v.stats();
    Rendered { name: "freeze-live", left, right, spawned: Some(s.spawned), skipped: s.skipped, peak_grains: s.peak_active, millis }
}

/// Four granular pads on the input, played one after another through the
/// sampler engine. Each is held for most of its quarter while its pressure
/// swells from nothing to full and back, then released.
fn render_pressure(data: &[f32], rate: u32, frames: usize) -> Result<Rendered, Box<dyn Error>> {
    let source = Source::new("input", rate, 2, data.to_vec())?;
    let mut project = Project::new();
    project.add_source(source.clone());
    let routes = [
        (PressureTarget::GrainDensity, 1.0), // 12 grains a second up to 192
        (PressureTarget::GrainSize, 1.0),    // 80 ms grains up to 640 ms
        (PressureTarget::GrainPosition, 0.6), // reads from 30 % of the way in up to 90 %
        (PressureTarget::Pitch, 1.0),        // up an octave
    ];
    for (i, &(target, depth)) in routes.iter().enumerate() {
        let mut r = Region::whole(&source);
        r.mode = PlayMode::Granular;
        r.grain = GrainParams { position: 0.3, spray: 0.05, size_ms: 80.0, density: 12.0, spread: 0.6, ..GrainParams::default() };
        let mut pad = Pad::new("pressure", Layer::new(r));
        pad.amp = Ahdsr { attack_ms: 20.0, hold_ms: 0.0, decay_ms: 0.0, sustain: 1.0, release_ms: 300.0 };
        pad.pressure = PressureRoute { target, depth };
        project.set_pad(i, Some(pad), None)?;
    }
    let mut e = Engine::new(rate, 8);
    e.set_view(EngineView::build(&project, rate));
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    let quarter = frames / 4;
    let held = quarter * 85 / 100;
    let mut peak_grains = 0;
    let t = Instant::now();
    let mut at = 0;
    while at < frames {
        let n = BLOCK.min(frames - at);
        let pad = (at / quarter).min(3);
        let into = at - pad * quarter;
        let pad = pad as u8;
        if into < n { e.apply(Event::NoteOn { pad, velocity: 110 }); }
        if into < held {
            let swell = (std::f64::consts::PI * into as f64 / held as f64).sin();
            e.apply(Event::Pressure { pad, value: (127.0 * swell).round() as u8 });
        } else if into < held + n {
            e.apply(Event::NoteOff { pad });
        }
        e.render(&mut left[at..at + n], &mut right[at..at + n]);
        peak_grains = peak_grains.max(e.live_grains());
        at += n;
    }
    let millis = t.elapsed().as_secs_f64() * 1000.0;
    Ok(Rendered { name: "pressure-pad", left, right, spawned: None, skipped: e.stats().skipped_grains, peak_grains, millis })
}

/// The input on a pad through the sampler engine, under the master granular
/// effect half wet, frozen partway through (see the list at the top).
fn render_master(data: &[f32], rate: u32, frames: usize) -> Result<Rendered, Box<dyn Error>> {
    let source = Source::new("input", rate, 2, data.to_vec())?;
    let mut project = Project::new();
    project.add_source(source.clone());
    let mut pad = Pad::new("input", Layer::new(Region::whole(&source)));
    pad.amp = Ahdsr { release_ms: 80.0, ..Ahdsr::OPEN };
    project.set_pad(0, Some(pad), None)?;
    let grain = GrainParams { position: 0.8, spray: 0.15, size_ms: 140.0, density: 25.0, pitch_spray: 0.15, spread: 0.8, ..GrainParams::default() };
    project.set_master(MasterFx { mix: 0.5, grain }, None)?;
    let mut e = Engine::new(rate, 8);
    e.set_view(EngineView::build(&project, rate));
    e.set_master_fx(Engine::make_master_fx(rate));
    // The input starts again each time it ends, until it stops at 60 %.
    let (freeze, stop, thaw) = (frames * 3 / 10, frames * 6 / 10, frames * 85 / 100);
    let mut events = vec![(freeze, Event::Freeze { on: true }), (stop, Event::AllOff), (thaw, Event::Freeze { on: false })];
    events.extend((0..stop).step_by(data.len() / 2).map(|f| (f, Event::NoteOn { pad: 0, velocity: 127 })));
    events.sort_by_key(|&(f, _)| f);
    let (mut left, mut right) = (vec![0.0; frames], vec![0.0; frames]);
    let mut peak_grains = 0;
    let t = Instant::now();
    let mut at = 0;
    while at < frames {
        let n = BLOCK.min(frames - at);
        for &(f, ev) in events.iter().filter(|(f, _)| (at..at + n).contains(f)) {
            e.schedule((f - at) as u32, ev);
        }
        e.render(&mut left[at..at + n], &mut right[at..at + n]);
        peak_grains = peak_grains.max(e.live_grains());
        at += n;
    }
    let millis = t.elapsed().as_secs_f64() * 1000.0;
    let spawned = e.master_fx().map(|fx| fx.stats().spawned);
    Ok(Rendered { name: "master-freeze", left, right, spawned, skipped: e.stats().skipped_grains, peak_grains, millis })
}

fn run() -> Result<(), Box<dyn Error>> {
    let o = parse()?;
    let (data, rate, label) = match &o.input {
        Some(path) => {
            let s = PreparedSample::read_wav(path)?;
            let data: Vec<f32> = s.frame_slice().iter().flat_map(|f| [f[0], f[1]]).collect();
            (data, s.sample_rate(), path.display().to_string())
        }
        None => (demo_source(DEMO_RATE), DEMO_RATE, "demo chord and drum loop".to_string()),
    };
    let input_frames = data.len() / 2;
    if input_frames < rate as usize / 10 { return Err("the input is shorter than 0.1 s".into()); }
    fs::create_dir_all(&o.out)?;
    if o.input.is_none() {
        let (l, r): (Vec<f32>, Vec<f32>) = data.chunks_exact(2).map(|f| (f[0], f[1])).unzip();
        write_wav(&o.out.join("demo-input.wav"), rate, &l, &r)?;
    }
    let frames = (o.seconds * f64::from(rate)) as usize;
    let stretch_frames = (input_frames * 4).min(rate as usize * 60);

    let presets: [(&'static str, Params, usize); 5] = [
        ("cloud", Params { position: 0.3, spray: 0.15, size_ms: 120.0, density: 40.0, pitch_spray: 0.15, spread: 0.8, ..Params::default() }, frames),
        ("stretch", Params { scan: 0.25, size_ms: 80.0, density: 50.0, spray: 0.002, spread: 0.2, ..Params::default() }, stretch_frames),
        ("shimmer", Params { position: 0.1, spray: 0.1, size_ms: 220.0, density: 30.0, pitch: 12.0, pitch_spray: 12.0, scale: 1 | 1 << 7, scan: 0.5, spread: 1.0, gain: 0.8, ..Params::default() }, frames),
        ("reverse-spray", Params { spray: 1.0, size_ms: 70.0, density: 25.0, reverse: 0.7, spread: 0.6, ..Params::default() }, frames),
        ("stutter-120", Params {
            position: 0.0,
            scan: 1.0,
            window: Window::Tukey { taper: 0.15 },
            sync: Some(TempoSync { frames_per_beat: 60.0 * f64::from(rate) / 120.0, grains_per_beat: Some(4.0), size_beats: Some(0.2) }),
            ..Params::default()
        }, frames),
    ];
    let mut outs: Vec<Rendered> = presets.iter().map(|&(name, p, n)| render(name, &data, rate, p, n, o.seed)).collect();
    outs.push(render_live(&data, rate, frames, o.seed));
    outs.push(render_pressure(&data, rate, frames)?);
    outs.push(render_master(&data, rate, frames)?);

    let mut rep = String::new();
    writeln!(rep, "mpfree-grain report")?;
    writeln!(rep, "input: {label}")?;
    writeln!(rep, "rate: {rate} Hz, {:.2} s, seed {}, block {BLOCK} frames, budget {BUDGET} grains", input_frames as f64 / f64::from(rate), o.seed)?;
    writeln!(rep)?;
    writeln!(rep, "{:<14} {:>7} {:>8} {:>7} {:>6} {:>9} {:>10} {:>9}", "preset", "seconds", "grains", "skipped", "peak", "level", "render ms", "x realtime")?;
    let mut all_ok = true;
    for r in &mut outs {
        let peak = r.left.iter().chain(&r.right).fold(0.0f32, |m, &x| m.max(x.abs()));
        all_ok &= peak.is_finite() && peak > 0.0;
        // Bring loud renders down to -1 dBFS rather than clip.
        let level = if peak > 0.891 { 0.891 / peak } else { 1.0 };
        for x in r.left.iter_mut().chain(r.right.iter_mut()) { *x *= level; }
        let secs = r.left.len() as f64 / f64::from(rate);
        let spawned = r.spawned.map_or("-".to_string(), |n| n.to_string());
        writeln!(rep, "{:<14} {:>7.2} {:>8} {:>7} {:>6} {:>8.1}dB {:>10.1} {:>9.0}",
            r.name, secs, spawned, r.skipped, r.peak_grains, 20.0 * peak.max(1e-9).log10(), r.millis, secs * 1000.0 / r.millis.max(1e-6))?;
        write_wav(&o.out.join(format!("{}.wav", r.name)), rate, &r.left, &r.right)?;
    }
    writeln!(rep)?;
    writeln!(rep, "level is the peak before any gain change; renders over -1 dBFS were turned down to -1 dBFS.")?;
    writeln!(rep, "x realtime is how many times faster than real time this workstation rendered it (one voice).")?;
    writeln!(rep, "pressure-pad plays through the sampler engine, which does not count grains started (-).")?;
    writeln!(rep, "result: {}", if all_ok { "PASS (every preset made sound)" } else { "FAIL (a preset was silent or not finite)" })?;
    fs::write(o.out.join("grain-report.txt"), &rep)?;
    print!("{rep}");
    println!("output_dir={}", o.out.display());
    if !all_ok { return Err("a preset was silent or not finite".into()); }
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("mpfree-grain: {err}");
        std::process::exit(1);
    }
}
