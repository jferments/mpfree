//! Hear clean pitching against Vintage on the workstation: writes WAVs of
//! the same sounds played pitched up, once clean and once Vintage.
//!
//! ```text
//! cargo run --release -p mpfree-sampler --example clean_pitch -- OUT
//! ```
//!
//! - `sweep-up-12-*.wav`: a sine sweeping from 100 Hz to 20 kHz, an octave
//!   up. Vintage's sweep turns round past 11 kHz and comes back down as a
//!   second, falling tone (the alias); the clean one just goes quiet.
//! - `saw-up-12-*.wav`: a bright 440 Hz saw (every harmonic up to the
//!   sound's Nyquist) an octave up. Vintage's harmonics past 22 kHz fold
//!   back between the real ones as a metallic, out-of-tune haze.
//! - `saw-96k-*.wav`: the same saw recorded at 96 kHz (harmonics up to
//!   48 kHz), played at its own pitch.
//!
//! It only reads and computes, and writes those files into OUT.

#[path = "../src/bin/shared/wav_out.rs"]
mod wav_out;

use std::error::Error;
use std::path::PathBuf;

use mpfree_sampler::engine::{Engine, EngineView, Event};
use mpfree_sampler::model::*;
use wav_out::write_wav;

const RATE: u32 = 44_100;

/// A mono sound from `f(t)`, `t` in seconds.
fn sound(name: &str, rate: u32, seconds: f64, f: impl Fn(f64) -> f64) -> Source {
    let n = (seconds * f64::from(rate)) as usize;
    let v = (0..n)
        .map(|i| (0.5 * f(i as f64 / f64::from(rate))) as f32)
        .collect();
    Source::new(name, rate, 1, v).expect("source")
}

/// A sine sweeping exponentially from `a` to `b` Hz over `seconds`.
fn sweep(rate: u32, seconds: f64, a: f64, b: f64) -> Source {
    let k = (b / a).ln() / seconds;
    sound("sweep", rate, seconds, |t| {
        (std::f64::consts::TAU * a * ((k * t).exp() - 1.0) / k).sin()
    })
}

/// A saw at `hz` holding every harmonic below the rate's Nyquist.
fn saw(rate: u32, seconds: f64, hz: f64) -> Source {
    let top = (f64::from(rate) / 2.0 / hz) as usize;
    sound("saw", rate, seconds, |t| {
        (1..=top)
            .map(|h| (std::f64::consts::TAU * hz * h as f64 * t).sin() / h as f64)
            .sum::<f64>()
            * 0.5
    })
}

/// `s` on a gated pad tuned `cents`, held for its whole length played at
/// that speed.
fn play(s: &Source, cents: i32, vintage: bool) -> (Vec<f32>, Vec<f32>) {
    let mut p = Project::new();
    p.add_source(s.clone());
    let mut r = Region::whole(s);
    r.mode = PlayMode::Gate;
    let mut pad = Pad::new("tone", Layer::new(r));
    pad.tune_cents = cents;
    pad.vintage = vintage;
    p.set_pad(0, Some(pad), None).expect("pad");
    let speed = f64::from(s.rate()) / f64::from(RATE) * (f64::from(cents) / 1200.0).exp2();
    let frames = (s.frames() as f64 / speed) as usize;
    let mut e = Engine::new(RATE, 4);
    e.set_view(EngineView::build(&p, RATE));
    e.apply(Event::NoteOn {
        pad: 0,
        velocity: 127,
    });
    let (mut l, mut r) = (vec![0.0; frames], vec![0.0; frames]);
    for (a, b) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
        e.render(a, b);
    }
    (l, r)
}

fn main() -> Result<(), Box<dyn Error>> {
    let out: PathBuf = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .ok_or("usage: clean_pitch OUT")?;
    std::fs::create_dir_all(&out)?;
    let cases = [
        ("sweep-up-12", sweep(RATE, 6.0, 100.0, 20_000.0), 1200),
        ("saw-up-12", saw(RATE, 3.0, 440.0), 1200),
        ("saw-96k", saw(96_000, 2.0, 440.0), 0),
    ];
    for (name, s, cents) in &cases {
        for (look, vintage) in [("clean", false), ("vintage", true)] {
            let (l, r) = play(s, *cents, vintage);
            let path = out.join(format!("{name}-{look}.wav"));
            write_wav(&path, RATE, &l, &r)?;
            println!("{}", path.display());
        }
    }
    Ok(())
}
