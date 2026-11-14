//! Renders a listening demo of the mono acid bass (Squelch) and writes
//! `~/mpfree-work/squelch-demo.wav` (or the path given).
//!
//! Each factory preset plays two bars of the same sixteen-step line at
//! 125 BPM, with accents (velocity 127) and slides (the next note played
//! before this one is let go), gates 3.5/6 of a step long as on the
//! original's sequencer. In the second bar the cutoff is swept up two
//! octaves and back, as a player turns the knob.
//!
//! It prints the timeline and each preset's peak. Everything runs through
//! `Node::process` in 128-frame blocks, as on the audio thread. The master
//! is trimmed to -1 dBFS peak.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::squelch_params::CUTOFF;
use mpfree_synth::Squelch;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
/// A sixteenth at 125 BPM.
const STEP: f64 = 0.12;
const STEPS: usize = 16;
const SECTION: f64 = 2.0 * STEPS as f64 * STEP + 0.4;

/// One bar: (semitones above C2, accent, slide into the next), or None
/// for a rest.
const LINE: [Option<(u8, bool, bool)>; STEPS] = [
    Some((0, true, false)),
    Some((0, false, false)),
    Some((12, false, true)),
    Some((0, false, false)),
    None,
    Some((3, true, false)),
    Some((0, false, false)),
    Some((7, false, true)),
    Some((10, false, false)),
    Some((0, true, false)),
    None,
    Some((12, false, true)),
    Some((10, false, false)),
    Some((7, true, false)),
    Some((5, false, false)),
    Some((3, false, true)),
];

fn at(s: f64) -> u64 {
    (s * f64::from(RATE)).round() as u64
}

fn phrase(t0: f64, cutoff: f32, ev: &mut Vec<(u64, NodeEvent)>) {
    for bar in 0..2 {
        for (i, step) in LINE.iter().enumerate() {
            let Some((n, accent, slide)) = *step else {
                continue;
            };
            let t = t0 + (bar * STEPS + i) as f64 * STEP;
            let note = 36 + n;
            let velocity = if accent { 127 } else { 90 };
            ev.push((at(t), NodeEvent::NoteOn { note, velocity }));
            // A slide holds the gate until just after the next note starts.
            let len = if slide { STEP + 0.01 } else { STEP * 3.5 / 6.0 };
            ev.push((at(t + len), NodeEvent::NoteOff { note }));
        }
    }
    // The second bar: the cutoff up two octaves and back down.
    let bar = STEPS as f64 * STEP;
    let n = 100;
    for k in 0..=n {
        let x = k as f64 / n as f64;
        let oct = 2.0 * (std::f64::consts::PI * x).sin();
        ev.push((
            at(t0 + bar + x * bar),
            NodeEvent::Param {
                id: CUTOFF,
                value: cutoff * oct.exp2() as f32,
            },
        ));
    }
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("squelch-demo.wav")
        });
    let presets = Squelch::presets();
    let total = at(presets.len() as f64 * SECTION + 0.5) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    println!(
        "Squelch demo, {} presets, {SECTION:.2} s each (bar 2 sweeps the cutoff):",
        presets.len()
    );
    for (i, p) in presets.iter().enumerate() {
        let t0 = i as f64 * SECTION;
        let mut s = Squelch::new(RATE);
        s.load_preset(p);
        let mut ev = Vec::new();
        phrase(t0, s.param(CUTOFF), &mut ev);
        ev.sort_by_key(|e| e.0);
        let from = at(t0) as usize;
        let to = (at(t0 + SECTION) as usize).min(total);
        let mut buf = Vec::new();
        let mut pos = from - from % BLOCK;
        while pos < to {
            buf.clear();
            for &(f, e) in &ev {
                if f >= pos as u64 && f < (pos + BLOCK) as u64 {
                    buf.push(Timed::new((f - pos as u64) as u32, e));
                }
            }
            s.process(
                &Ctx::new(RATE, pos as u64),
                &buf,
                &mut l[pos..pos + BLOCK],
                &mut r[pos..pos + BLOCK],
            );
            pos += BLOCK;
        }
        let pk = l[from..to]
            .iter()
            .chain(&r[from..to])
            .fold(0.0f32, |m, x| m.max(x.abs()));
        let m = (t0 / 60.0) as u32;
        let sec = t0 - f64::from(m) * 60.0;
        println!(
            "  {m}:{sec:04.1}  {:<12} peak {:.1} dBFS",
            p.name,
            20.0 * pk.max(1e-9).log10()
        );
    }
    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    let trim = 0.891 / peak.max(1e-6);
    let mut pcm = Vec::with_capacity(total * 4);
    for i in 0..total {
        for x in [l[i], r[i]] {
            let v = (x * trim).clamp(-1.0, 1.0);
            pcm.extend_from_slice(&((v * 32767.0).round() as i16).to_le_bytes());
        }
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("create output dir");
    }
    std::fs::write(&out, wav(&pcm)).expect("write wav");
    println!(
        "wrote {} ({:.1} s, master trim {:+.1} dB)",
        out.display(),
        total as f64 / f64::from(RATE),
        20.0 * trim.log10()
    );
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(44 + pcm.len());
    let data = pcm.len() as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.extend_from_slice(pcm);
    b
}
