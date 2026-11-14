//! Renders a listening demo of the vintage poly (Analog) and writes
//! `~/mpfree-work/analog-demo.wav` (or the path given).
//!
//! Each factory preset plays what it was made for, over the same four
//! chords (Am7, Fmaj7, C, G, 1.2 s each): held chords for the pads and
//! keys, stabs for the brass, a legato line for the lead (so it glides),
//! an eighth-note line for the bass, an arpeggio for the bell. Worn Cards
//! plays one chord four times, so each lands on other cards and sounds a
//! little different.
//!
//! It prints the timeline and each preset's peak. Everything runs through
//! `Node::process` in 128-frame blocks, as on the audio thread. A preset's
//! tail rings 1.5 s into the next, faded over its last half second. The
//! master is trimmed to -1 dBFS peak.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::Analog;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const CHORD: f64 = 1.2;
const SECTION: f64 = 4.0 * CHORD + 0.6;
const TAIL: f64 = 1.5;

const CHORDS: [[u8; 4]; 4] = [
    [57, 60, 64, 67], // Am7
    [53, 57, 60, 64], // Fmaj7
    [48, 55, 60, 64], // C
    [50, 55, 59, 62], // G
];

#[derive(Clone, Copy)]
enum Play {
    /// Each chord held for this share of its length.
    Chords(f64),
    /// Each chord hit twice, short.
    Stabs,
    /// A line, each note held into the next.
    Lead,
    /// Eighth notes on each chord's root, two octaves down.
    Bass,
    /// Each chord's notes in eighths, an octave up.
    Arp,
    /// The first chord, four times.
    Repeat,
}

fn play_for(name: &str) -> Play {
    match name {
        "Poly Brass" => Play::Stabs,
        "Sync Lead" => Play::Lead,
        "Unison Bass" => Play::Bass,
        "Clang Bell" => Play::Arp,
        "Worn Cards" => Play::Repeat,
        "Square Keys" => Play::Chords(0.45),
        _ => Play::Chords(0.9),
    }
}

fn at(s: f64) -> u64 {
    (s * f64::from(RATE)).round() as u64
}

fn note(ev: &mut Vec<(u64, NodeEvent)>, t: f64, len: f64, note: u8, velocity: u8) {
    ev.push((at(t), NodeEvent::NoteOn { note, velocity }));
    ev.push((at(t + len), NodeEvent::NoteOff { note }));
}

fn phrase(t0: f64, play: Play, ev: &mut Vec<(u64, NodeEvent)>) {
    let eighth = CHORD / 4.0;
    for (c, chord) in CHORDS.iter().enumerate() {
        let t = t0 + c as f64 * CHORD;
        match play {
            Play::Chords(gate) => {
                for &n in chord {
                    note(ev, t, CHORD * gate, n, 100);
                }
            }
            Play::Stabs => {
                for k in 0..2 {
                    for &n in chord {
                        note(ev, t + k as f64 * CHORD / 2.0, 0.25, n, 110 - 20 * k);
                    }
                }
            }
            Play::Lead => {
                // The chord's top two notes an octave up, then its third,
                // each played before the last is let go.
                let line = [chord[3] + 12, chord[2] + 12, chord[1] + 12];
                let lens = [0.5, 0.3, 0.4];
                let mut s = t;
                for (n, l) in line.into_iter().zip(lens) {
                    note(ev, s, l * CHORD + 0.03, n, 100);
                    s += l * CHORD;
                }
            }
            Play::Bass => {
                let root = chord[0] - 24 + if c == 0 { 0 } else { 12 };
                for k in 0..4 {
                    let n = if k == 2 { root + 12 } else { root };
                    note(
                        ev,
                        t + k as f64 * eighth,
                        eighth * 0.6,
                        n,
                        90 + 15 * (k % 2) as u8,
                    );
                }
            }
            Play::Arp => {
                for (k, &n) in chord.iter().enumerate() {
                    note(ev, t + k as f64 * eighth, eighth * 1.5, n + 12, 100);
                }
            }
            Play::Repeat => {
                for &n in &CHORDS[0] {
                    note(ev, t, CHORD * 0.75, n, 100);
                }
            }
        }
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
                .join("analog-demo.wav")
        });
    let presets = Analog::presets();
    let total = at(presets.len() as f64 * SECTION + TAIL) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    let mut sl = [0.0f32; BLOCK];
    let mut sr = [0.0f32; BLOCK];
    println!(
        "Analog demo, {} presets, {SECTION:.2} s each:",
        presets.len()
    );
    for (i, p) in presets.iter().enumerate() {
        let t0 = i as f64 * SECTION;
        let mut s = Analog::new(RATE);
        s.load_preset(p);
        let mut ev = Vec::new();
        phrase(t0, play_for(p.name), &mut ev);
        ev.sort_by_key(|e| e.0);
        let from = at(t0) as usize;
        let end = at(t0 + SECTION) as usize;
        let to = (at(t0 + SECTION + TAIL) as usize).min(total);
        let fade_from = to - at(0.5) as usize;
        let mut buf = Vec::new();
        let mut pos = from - from % BLOCK;
        let mut pk = 0.0f32;
        while pos < to {
            buf.clear();
            for &(f, e) in &ev {
                if f >= pos as u64 && f < (pos + BLOCK) as u64 {
                    buf.push(Timed::new((f - pos as u64) as u32, e));
                }
            }
            sl.fill(0.0);
            sr.fill(0.0);
            s.process(&Ctx::new(RATE, pos as u64), &buf, &mut sl, &mut sr);
            for k in 0..BLOCK.min(total - pos) {
                let f = pos + k;
                let g = if f >= to {
                    0.0
                } else if f > fade_from {
                    (to - f) as f32 / (to - fade_from) as f32
                } else {
                    1.0
                };
                l[f] += sl[k] * g;
                r[f] += sr[k] * g;
                if f < end {
                    pk = pk.max(sl[k].abs()).max(sr[k].abs());
                }
            }
            pos += BLOCK;
        }
        let m = (t0 / 60.0) as u32;
        let sec = t0 - f64::from(m) * 60.0;
        println!(
            "  {m}:{sec:04.1}  {:<15} peak {:.1} dBFS",
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
