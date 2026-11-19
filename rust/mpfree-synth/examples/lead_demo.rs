//! Renders the ten lead presets (stage 7 of the synth library), each on the
//! same phrase, and writes one WAV per preset into
//! `~/mpfree-work/lead-demo/` (or the folder given): `Poly Portamento
//! Lead.wav` and so on.
//!
//! The phrase shows what the leads are for (100 BPM, 10 s with the tail):
//! - 0.0 to 2.4 s: a rising line, each note held over the next (legato:
//!   the glide and Legato's no-retrigger);
//! - 2.4 to 5.0 s: a long C5 leaned into (pad pressure up and down), slid
//!   up a tone and back by its own slide (MPE), its tone swept, then the
//!   mod wheel up (vibrato);
//! - 5.2 to 6.2 s: a two-octave leap, C3 to C5, held over (glide by rate
//!   takes longer for it than for the line's small steps);
//! - 6.5 to 8.0 s: C4 held while G4 and E4 are played over it (Priority:
//!   Low stays on C4, High and Last move to the new key).
//!
//! The synths run through `mpfree_node::Node::process` in 128-frame blocks,
//! as on the audio thread. Each file is trimmed to a -3 dBFS peak; the
//! trim is printed.

use mpfree_node::{Ctl, Ctx, Expr, Node, NodeEvent, Timed};
use mpfree_synth::{Analog, Bass, Poly};

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const LENGTH: f64 = 10.0;

fn at(t: f64) -> u64 {
    (t * f64::from(RATE)).round() as u64
}

fn on(ev: &mut Vec<(u64, NodeEvent)>, t: f64, note: u8, velocity: u8) {
    ev.push((at(t), NodeEvent::NoteOn { note, velocity }));
}

fn off(ev: &mut Vec<(u64, NodeEvent)>, t: f64, note: u8) {
    ev.push((at(t), NodeEvent::NoteOff { note }));
}

/// `f(x)` for x from 0 to 1 as events every 20 ms from `t0` to `t1`.
fn ramp(ev: &mut Vec<(u64, NodeEvent)>, t0: f64, t1: f64, f: impl Fn(f64) -> NodeEvent) {
    let steps = ((t1 - t0) / 0.02).ceil() as usize;
    for i in 0..=steps {
        let x = i as f64 / steps as f64;
        ev.push((at(t0 + x * (t1 - t0)), f(x)));
    }
}

fn phrase() -> Vec<(u64, NodeEvent)> {
    let mut ev = Vec::new();
    // The rising line, each note over the next.
    for (i, n) in [60u8, 63, 67, 70].into_iter().enumerate() {
        let t = i as f64 * 0.6;
        on(&mut ev, t, n, 96 + 8 * (i as u8 % 2));
        off(&mut ev, t + 0.66, n);
    }
    // The long C5: pressure, its own slide and tone, then the wheel.
    on(&mut ev, 2.4, 72, 110);
    ramp(&mut ev, 2.8, 4.0, |x| NodeEvent::Pressure {
        note: 72,
        value: (127.0 * (std::f64::consts::PI * x).sin()).round() as u8,
    });
    ramp(&mut ev, 3.2, 4.2, |x| NodeEvent::Expression {
        note: 72,
        kind: Expr::Slide,
        value: (2.0 * (std::f64::consts::PI * x).sin()) as f32,
    });
    ramp(&mut ev, 3.0, 4.6, |x| NodeEvent::Expression {
        note: 72,
        kind: Expr::Tone,
        value: (0.5 + 0.5 * (std::f64::consts::PI * x).sin()) as f32,
    });
    ramp(&mut ev, 4.2, 5.0, |x| NodeEvent::Control {
        ctl: Ctl::ModWheel,
        value: (std::f64::consts::PI * x).sin() as f32,
    });
    off(&mut ev, 5.0, 72);
    // The leap.
    on(&mut ev, 5.2, 48, 100);
    on(&mut ev, 5.6, 72, 100);
    off(&mut ev, 5.7, 48);
    off(&mut ev, 6.2, 72);
    // Priority: C4 held, G4 and E4 over it.
    on(&mut ev, 6.5, 60, 100);
    on(&mut ev, 6.8, 67, 100);
    off(&mut ev, 7.1, 67);
    on(&mut ev, 7.3, 64, 100);
    off(&mut ev, 7.6, 64);
    off(&mut ev, 8.0, 60);
    ev.sort_by_key(|e| e.0);
    ev
}

fn render(n: &mut dyn Node, ev: &[(u64, NodeEvent)]) -> (Vec<f32>, Vec<f32>) {
    let total = (at(LENGTH) as usize).div_ceil(BLOCK) * BLOCK;
    let (mut l, mut r) = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut buf = Vec::new();
    for pos in (0..total).step_by(BLOCK) {
        buf.clear();
        for &(f, e) in ev {
            if f >= pos as u64 && f < (pos + BLOCK) as u64 {
                buf.push(Timed::new((f - pos as u64) as u32, e));
            }
        }
        n.process(
            &Ctx::new(RATE, pos as u64),
            &buf,
            &mut l[pos..pos + BLOCK],
            &mut r[pos..pos + BLOCK],
        );
    }
    (l, r)
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("lead-demo")
        });
    std::fs::create_dir_all(&dir).expect("create output dir");
    let ev = phrase();
    let mut sounds: Vec<(String, Box<dyn Node>)> = Vec::new();
    for name in [
        "Portamento Lead",
        "Low Note Lead",
        "Breath Lead",
        "Slide Lead",
        "Unison Lead",
    ] {
        let p = Poly::presets().iter().find(|p| p.name == name).expect(name);
        let mut s = Poly::new(RATE);
        s.load_preset(p);
        sounds.push((format!("Poly {name}"), Box::new(s)));
    }
    for name in ["Solo Lead", "Glide Lead", "Fat Lead"] {
        let p = Analog::presets()
            .iter()
            .find(|p| p.name == name)
            .expect(name);
        let mut s = Analog::new(RATE);
        s.load_preset(p);
        sounds.push((format!("Analog {name}"), Box::new(s)));
    }
    for name in ["Rate Glide Bass", "High Note Lead"] {
        let p = Bass::presets().iter().find(|p| p.name == name).expect(name);
        let mut s = Bass::new(RATE);
        s.load_preset(p);
        sounds.push((format!("Bass {name}"), Box::new(s)));
    }
    for (name, mut s) in sounds {
        let (l, r) = render(s.as_mut(), &ev);
        let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
        let trim = 0.708 / peak.max(1e-6);
        let mut pcm = Vec::with_capacity(l.len() * 4);
        for (a, b) in l.iter().zip(&r) {
            for x in [a, b] {
                let v = (x * trim).clamp(-1.0, 1.0);
                pcm.extend_from_slice(&((v * 32767.0).round() as i16).to_le_bytes());
            }
        }
        let out = dir.join(format!("{name}.wav"));
        std::fs::write(&out, wav(&pcm)).expect("write wav");
        println!(
            "{name:<22} peak {:6.1} dBFS, trim {:+5.1} dB -> {}",
            20.0 * peak.max(1e-9).log10(),
            20.0 * trim.log10(),
            out.display()
        );
    }
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
