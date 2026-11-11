//! Renders a listening demo of tunings and per-note slides and writes
//! `~/mpfree-work/tuning-demo.wav` (or the path given).
//!
//! 1. **Chords in three tunings** (0 to 27 s): C, F, G, C on the FM
//!    "Drawbars" organ, three times: 12-TET, then 5-limit just intonation
//!    on C, then quarter-comma meantone on C. Listen to the thirds: in
//!    12-TET they beat (a slow wobble), in just intonation they lock still,
//!    in meantone they lock too while the fifths turn slightly sour.
//! 2. **One note slides inside a chord** (27 to 34 s): the Poly "Dust Pad"
//!    holds C, E and G; only the E slides up to F and back (a per-note
//!    slide), while C and G stay put.
//! 3. **Quarter tones** (34 to 44 s): the Pluck "Koto" plays a C major
//!    scale in 12-TET, then maqam Rast on C in 24-TET, whose E and B sit a
//!    quarter tone flat.
//!
//! Every synth runs through `mpfree_node::Node::process` in 128-frame
//! blocks, as on the audio thread, with its tuning set through
//! `Node::set_tuning` and slides sent as `NodeEvent::Expression`. The
//! master is trimmed to -1 dBFS peak.

use mpfree_node::{Expr, Node, NodeEvent, Timed, Tuning};
use mpfree_synth::{Fm, Pluck, Poly};
use mpfree_tuning::Builtin;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;

fn secs(s: f64) -> u64 {
    (s * f64::from(RATE)).round() as u64
}

fn poly(name: &str) -> Box<dyn Node> {
    let mut s = Poly::new(RATE);
    s.load_preset(Poly::presets().iter().find(|p| p.name == name).unwrap());
    Box::new(s)
}

fn fm(name: &str) -> Box<dyn Node> {
    let mut s = Fm::new(RATE);
    s.load_preset(&Fm::presets().into_iter().find(|p| p.name == name).unwrap());
    Box::new(s)
}

fn pluck(name: &str) -> Box<dyn Node> {
    let mut s = Pluck::new(RATE);
    s.load_preset(Pluck::presets().iter().find(|p| p.name == name).unwrap());
    Box::new(s)
}

/// One synth playing `events` (frames from `start`) for `len` frames.
struct Clip {
    what: String,
    start: u64,
    len: u64,
    node: Box<dyn Node>,
    tuning: Tuning,
    events: Vec<(u64, NodeEvent)>,
    gain: f32,
}

impl Clip {
    fn render(mut self, l: &mut [f32], r: &mut [f32]) {
        self.node.set_tuning(&self.tuning);
        self.events.sort_by_key(|e| e.0);
        let mut bl = [0.0f32; BLOCK];
        let mut br = [0.0f32; BLOCK];
        let mut buf = Vec::new();
        let mut at = 0u64;
        while at < self.len {
            buf.clear();
            for &(f, ev) in &self.events {
                if f >= at && f < at + BLOCK as u64 {
                    buf.push(Timed::new((f - at) as u32, ev));
                }
            }
            bl.fill(0.0);
            br.fill(0.0);
            let ctx = mpfree_node::Ctx::new(RATE, at);
            self.node.process(&ctx, &buf, &mut bl, &mut br);
            for i in 0..BLOCK {
                let o = (self.start + at) as usize + i;
                if o < l.len() {
                    l[o] += bl[i] * self.gain;
                    r[o] += br[i] * self.gain;
                }
            }
            at += BLOCK as u64;
        }
    }
}

fn chord(events: &mut Vec<(u64, NodeEvent)>, at: f64, len: f64, notes: &[u8]) {
    for &n in notes {
        events.push((
            secs(at),
            NodeEvent::NoteOn {
                note: n,
                velocity: 96,
            },
        ));
        events.push((secs(at + len), NodeEvent::NoteOff { note: n }));
    }
}

/// A slide on `note` from `from` to `to` semitones over `len` seconds, a
/// value every 5 ms (the synth smooths between them).
fn slide(events: &mut Vec<(u64, NodeEvent)>, note: u8, at: f64, len: f64, from: f32, to: f32) {
    let steps = (len / 0.005).round() as usize;
    for k in 0..=steps {
        let t = k as f32 / steps as f32;
        // Ease in and out, like a finger.
        let e = 0.5 - 0.5 * (std::f32::consts::PI * t).cos();
        events.push((
            secs(at + len * f64::from(t)),
            NodeEvent::Expression {
                note,
                kind: Expr::Slide,
                value: from + (to - from) * e,
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
                .join("tuning-demo.wav")
        });
    let mut clips = Vec::new();

    // 1. Chords in three tunings.
    let progression: [&[u8]; 4] = [
        &[48, 60, 64, 67],
        &[41, 60, 65, 69],
        &[43, 59, 62, 67],
        &[48, 60, 64, 67, 72],
    ];
    let tunings = [
        ("12-TET", Tuning::standard()),
        ("just intonation on C", Builtin::Just.tuning(0, 440.0)),
        (
            "quarter-comma meantone on C",
            Builtin::Meantone.tuning(0, 440.0),
        ),
    ];
    for (k, (name, tuning)) in tunings.into_iter().enumerate() {
        let start = 9.0 * k as f64;
        let mut events = Vec::new();
        for (i, c) in progression.iter().enumerate() {
            chord(&mut events, 2.0 * i as f64, 1.9, c);
        }
        clips.push(Clip {
            what: format!("Drawbars (FM): C F G C in {name}"),
            start: secs(start),
            len: secs(9.0),
            node: fm("Drawbars"),
            tuning,
            events,
            gain: 0.9,
        });
    }

    // 2. One note slides inside a chord.
    let mut events = Vec::new();
    chord(&mut events, 0.0, 5.5, &[48, 60, 64, 67]);
    slide(&mut events, 64, 1.5, 0.35, 0.0, 1.0);
    slide(&mut events, 64, 3.5, 0.35, 1.0, 0.0);
    clips.push(Clip {
        what: "Dust Pad (Poly): C E G held; only E slides to F and back".to_string(),
        start: secs(27.0),
        len: secs(7.0),
        node: poly("Dust Pad"),
        tuning: Tuning::standard(),
        events,
        gain: 1.0,
    });

    // 3. Quarter tones: C major in 12-TET, then Rast on C in 24-TET (each
    // key a quarter tone, A4 on key 69, so C4 is key 51).
    let major = [60u8, 62, 64, 65, 67, 69, 71, 72];
    let rast = [51u8, 55, 58, 61, 65, 69, 72, 75];
    for (k, (name, scale, tuning)) in [
        ("C major in 12-TET", major, Tuning::standard()),
        (
            "maqam Rast on C in 24-TET",
            rast,
            Builtin::Equal24.tuning(0, 440.0),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut events = Vec::new();
        let run: Vec<u8> = scale
            .iter()
            .chain(scale.iter().rev().skip(1))
            .copied()
            .collect();
        for (i, &n) in run.iter().enumerate() {
            let at = 0.3 * i as f64;
            events.push((
                secs(at),
                NodeEvent::NoteOn {
                    note: n,
                    velocity: 100,
                },
            ));
            events.push((secs(at + 0.28), NodeEvent::NoteOff { note: n }));
        }
        clips.push(Clip {
            what: format!("Koto (Pluck): {name}"),
            start: secs(34.0 + 5.0 * k as f64),
            len: secs(5.0),
            node: pluck("Koto"),
            tuning,
            events,
            gain: 1.2,
        });
    }

    let total = secs(44.5) as usize;
    let (mut l, mut r) = (vec![0.0f32; total], vec![0.0f32; total]);
    for c in clips {
        println!(
            "{:5.1} s to {:5.1} s  {}",
            c.start as f64 / f64::from(RATE),
            (c.start + c.len) as f64 / f64::from(RATE),
            c.what
        );
        c.render(&mut l, &mut r);
    }
    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak.is_finite() && peak > 0.01, "silent or not finite");
    let trim = 0.891 / peak;
    println!(
        "peak {:.1} dBFS before the trim to -1 dBFS",
        20.0 * peak.log10()
    );
    let mut pcm = Vec::with_capacity(total * 4);
    for i in 0..total {
        for x in [l[i], r[i]] {
            let s = (x * trim).clamp(-1.0, 1.0);
            pcm.extend_from_slice(&((s * 32767.0).round() as i16).to_le_bytes());
        }
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("create output dir");
    }
    std::fs::write(&out, wav(&pcm)).expect("write wav");
    println!(
        "wrote {} ({:.1} s)",
        out.display(),
        total as f64 / f64::from(RATE)
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
