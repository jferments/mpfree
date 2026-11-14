//! Renders a sung phrase (a synthetic voice, a little out of tune, with
//! vibrato) dry and through the vocal effects, and writes
//! `~/mpfree-work/vocal-demo.wav` (or the path given). Four sections of
//! two bars at 100 BPM, the same phrase each time:
//!
//! - bars 1-2: dry.
//! - bars 3-4: **Pitch correct**, "Natural" in A minor (a gentle pull that
//!   keeps the vibrato).
//! - bars 5-6: **Pitch correct**, "A minor snap" (the hard, stepped sound).
//! - bars 7-8: the **Harmonizer**, "Close harmony" in A minor (a 3rd above
//!   and below that follow the key).
//!
//! It prints how far each section sings from the key's notes (the median
//! over 40 ms looks, in cents), checks that nothing is non-finite and that
//! the converter would not clip. Everything runs in 128-frame blocks
//! through Master safety, as on the audio thread.

use std::f32::consts::{PI, TAU};

use mpfree_fx::ids::{harmonizer as harm, pitch_correct as tune};
use mpfree_fx::mpfree_node::{Ctx, Node};
use mpfree_fx::{gain_to_db, nearest_note, presets, Harmonizer, MasterSafety, PitchCorrect};

const RATE: u32 = 48_000;
const BPM: f64 = 100.0;
const BLOCK: usize = 128;
const KEY_A: f32 = 9.0;
const MINOR: f32 = 2.0;

fn bar_frames() -> usize {
    (4.0 * 60.0 / BPM * f64::from(RATE)).round() as usize
}

/// A two-pole resonator (one formant).
struct Formant {
    a1: f32,
    a2: f32,
    g: f32,
    y: [f32; 2],
}

impl Formant {
    fn new(hz: f32, bw: f32) -> Formant {
        let r = (-PI * bw / RATE as f32).exp();
        Formant {
            a1: 2.0 * r * (TAU * hz / RATE as f32).cos(),
            a2: -r * r,
            g: 1.0 - r,
            y: [0.0; 2],
        }
    }
    fn run(&mut self, x: f32) -> f32 {
        let y = self.g * x + self.a1 * self.y[0] + self.a2 * self.y[1];
        self.y = [y, self.y[0]];
        y
    }
}

/// The phrase: (MIDI note, beats, how far off the singer is in cents).
const PHRASE: [(i32, f32, f32); 8] = [
    (57, 1.0, -28.0),
    (60, 0.5, 22.0),
    (62, 0.5, -18.0),
    (64, 1.5, 30.0),
    (67, 0.5, -24.0),
    (64, 1.0, 16.0),
    (62, 1.0, -30.0),
    (60, 2.0, 20.0),
];

/// Two bars of a singer on "ah": a soft glottal pulse at the phrase's
/// pitch (gliding 40 ms between notes, 5.5 Hz vibrato of 25 cents that
/// starts after 300 ms) through three formants, with a breath at the end.
fn phrase() -> Vec<f32> {
    let beat = 60.0 / BPM as f32 * RATE as f32;
    let total = bar_frames() * 2;
    let mut f1 = Formant::new(700.0, 110.0);
    let mut f2 = Formant::new(1220.0, 120.0);
    let mut f3 = Formant::new(2600.0, 160.0);
    let mut out = vec![0.0f32; total];
    let (mut phase, mut pitch, mut seed) = (0.0f32, 57.0f32, 7u32);
    let (mut start, mut lp) = (0usize, 0.0f32);
    for &(note, beats, off) in &PHRASE {
        let len = (beats * beat) as usize;
        let target = note as f32 + off / 100.0;
        for i in 0..len.min(total - start) {
            let t = i as f32 / RATE as f32;
            pitch += (target - pitch) * (1.0 - (-1.0 / (0.04 * RATE as f32)).exp());
            let vib = if t > 0.3 {
                0.25 * (TAU * 5.5 * t).sin() * ((t - 0.3) * 4.0).min(1.0)
            } else {
                0.0
            };
            let hz = 440.0 * 2f32.powf((pitch + vib - 69.0) / 12.0);
            phase += hz / RATE as f32;
            let pulse = if phase >= 1.0 {
                phase -= 1.0;
                1.0
            } else {
                0.0
            };
            // A one-pole low-pass makes the pulse a softer glottal one.
            lp += 0.3 * (pulse - lp);
            let env = (t / 0.03).min(1.0) * ((len as f32 / RATE as f32 - t) / 0.05).clamp(0.0, 1.0);
            let x = env * (lp - hz / RATE as f32);
            let y = 1.0 * f1.run(x) + 0.6 * f2.run(x) + 0.25 * f3.run(x);
            out[start + i] = y;
        }
        start += len;
    }
    // A breath in what is left.
    for v in &mut out[start..] {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *v = 0.02 * ((seed >> 8) as f32 / (1 << 23) as f32 - 1.0);
    }
    let top = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    out.iter().map(|v| v * 0.35 / top).collect()
}

/// The pitch of `x` (its difference function's first dip under 0.1),
/// None where it isn't clearly pitched.
fn pitch_hz(x: &[f32]) -> Option<f32> {
    let (lo, hi) = (RATE as usize / 1000, RATE as usize / 70);
    let win = x.len() - hi - 1;
    let d: Vec<f32> = (0..=hi)
        .map(|tau| (0..win).map(|j| (x[j] - x[j + tau]).powi(2)).sum())
        .collect();
    let mut sum = 0.0;
    let mut norm = vec![1.0f32; hi + 1];
    for tau in 1..=hi {
        sum += d[tau];
        norm[tau] = d[tau] * tau as f32 / sum.max(1e-12);
    }
    let mut tau = lo;
    while tau < hi - 1 && norm[tau] >= 0.1 {
        tau += 1;
    }
    if tau >= hi - 1 {
        return None;
    }
    while tau < hi - 1 && norm[tau + 1] < norm[tau] {
        tau += 1;
    }
    let (a, b, c) = (norm[tau - 1], norm[tau], norm[tau + 1]);
    Some(RATE as f32 / (tau as f32 + 0.5 * (a - c) / (a - 2.0 * b + c)))
}

/// The median distance from A minor's notes over 40 ms looks, in cents.
fn cents_off_key(x: &[f32]) -> f32 {
    let w = RATE as usize / 25;
    let mut offs: Vec<f32> = (0..x.len() / w - 1)
        .filter_map(|k| pitch_hz(&x[k * w..(k + 1) * w + RATE as usize / 70 + 1]))
        .map(|hz| {
            let midi = 69.0 + 12.0 * (hz / 440.0).log2();
            100.0 * (midi - nearest_note(midi, 9, 2) as f32).abs()
        })
        .collect();
    offs.sort_by(f32::total_cmp);
    offs.get(offs.len() / 2).copied().unwrap_or(f32::NAN)
}

fn set_preset(n: &mut dyn Node, name: &str) {
    let p = presets(n.info().id)
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"));
    for &(id, v) in p.values {
        n.set_param(id, v);
    }
}

fn run(n: &mut dyn Node, x: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (x.to_vec(), x.to_vec());
    let mut at = 0;
    while at < x.len() {
        let len = BLOCK.min(x.len() - at);
        n.process(
            &Ctx::new(RATE, at as u64),
            &[],
            &mut l[at..at + len],
            &mut r[at..at + len],
        );
        at += len;
    }
    (l, r)
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let dir = format!("{home}/mpfree-work");
        let _ = std::fs::create_dir_all(&dir);
        format!("{dir}/vocal-demo.wav")
    });
    let voice = phrase();

    let mut natural = PitchCorrect::new();
    natural.set_param(tune::KEY, KEY_A);
    natural.set_param(tune::SCALE, MINOR);
    set_preset(&mut natural, "Natural");
    let mut snap = PitchCorrect::new();
    set_preset(&mut snap, "A minor snap");
    let mut harmony = Harmonizer::new();
    set_preset(&mut harmony, "Close harmony");
    harmony.set_param(harm::KEY, KEY_A);
    harmony.set_param(harm::SCALE, MINOR);
    harmony.set_param(harm::DRY, 80.0);

    let mut sections: Vec<(&str, Vec<f32>, Vec<f32>)> = vec![("dry", voice.clone(), voice.clone())];
    for (name, node) in [
        ("Pitch correct, Natural", &mut natural as &mut dyn Node),
        ("Pitch correct, A minor snap", &mut snap),
        ("Harmonizer, Close harmony", &mut harmony),
    ] {
        node.prepare(RATE, BLOCK);
        node.reset();
        // The effect's latency comes off the front, as the mixer's
        // compensation would take it.
        let lat = node.latency() as usize;
        let mut x = voice.clone();
        x.resize(x.len() + lat, 0.0);
        let (l, r) = run(node, &x);
        sections.push((name, l[lat..].to_vec(), r[lat..].to_vec()));
    }

    let (mut out_l, mut out_r) = (Vec::new(), Vec::new());
    println!("bars  section                        cents off A minor (median)");
    for (k, (name, l, r)) in sections.iter().enumerate() {
        // The harmony is several voices: its pitch is not one number.
        let off = if k < 3 {
            format!("{:.0}", cents_off_key(l))
        } else {
            "-".to_string()
        };
        println!("{}-{}   {name:<30} {off}", 2 * k + 1, 2 * k + 2);
        out_l.extend_from_slice(l);
        out_r.extend_from_slice(r);
    }
    let mut master = MasterSafety::new();
    master.prepare(RATE, BLOCK);
    master.reset();
    let mut at = 0;
    while at < out_l.len() {
        let len = BLOCK.min(out_l.len() - at);
        master.process(
            &Ctx::new(RATE, at as u64),
            &[],
            &mut out_l[at..at + len],
            &mut out_r[at..at + len],
        );
        at += len;
    }
    let peak = out_l
        .iter()
        .chain(&out_r)
        .fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(
        out_l.iter().chain(&out_r).all(|v| v.is_finite()),
        "non-finite output"
    );
    assert!(peak <= 1.0, "the converter would clip: {peak}");
    println!("peak {:.1} dBFS", gain_to_db(peak));
    std::fs::write(&path, mpfree_fx::wav_f32(&out_l, &out_r, RATE)).expect("write the WAV");
    println!("wrote {path}");
}
