//! Renders a listening demo of the electric piano (Keys) and writes
//! `~/mpfree-work/keys-demo.wav` (or the path given).
//!
//! Each factory preset plays, in turn, about 8 seconds:
//! - four chords (Dm9, G13, Cmaj9, A7#9 voicings around middle C) at
//!   velocities 40, 70, 100 and 127, with a bass note under each, so the
//!   tone can be heard going from soft and round to barking;
//! - a velocity ladder on C2 and C4 (30, 60, 90, 127): the low register
//!   barks sooner, as the tine swings wider;
//! - a short melody in the top octaves, for the bell of the attack.
//!
//! It prints the timeline, so a sound can be found by its time, and each
//! preset's peak. All parts run through `Node::process` in 128-frame
//! blocks, as on the audio thread. The master is trimmed to -1 dBFS peak.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::Keys;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const SECTION: f64 = 8.5;

fn at(s: f64) -> u64 {
    (s * f64::from(RATE)).round() as u64
}

/// One preset's phrase, starting at `t0` seconds.
fn phrase(t0: f64, ev: &mut Vec<(u64, NodeEvent)>) {
    let mut note = |t: f64, len: f64, n: u8, vel: u8| {
        ev.push((
            at(t0 + t),
            NodeEvent::NoteOn {
                note: n,
                velocity: vel,
            },
        ));
        ev.push((at(t0 + t + len), NodeEvent::NoteOff { note: n }));
    };
    let chords: [(&[u8], u8); 4] = [
        (&[53, 57, 60, 64], 38),
        (&[53, 57, 59, 64], 43),
        (&[52, 55, 59, 62], 36),
        (&[55, 60, 61, 67], 45),
    ];
    for (i, ((voicing, bass), vel)) in chords.iter().zip([40u8, 70, 100, 127]).enumerate() {
        let t = i as f64 * 0.9;
        for (k, &n) in voicing.iter().enumerate() {
            // A slight roll, as fingers land.
            note(t + k as f64 * 0.008, 0.8, n, vel);
        }
        note(t, 0.85, *bass, vel);
    }
    for (i, vel) in [30u8, 60, 90, 127].into_iter().enumerate() {
        note(3.8 + i as f64 * 0.45, 0.4, 36, vel);
        note(3.8 + i as f64 * 0.45 + 0.2, 0.2, 60, vel);
    }
    for (i, n) in [79u8, 84, 86, 88, 91, 88, 96].into_iter().enumerate() {
        note(5.8 + i as f64 * 0.18, 0.16, n, 96);
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
                .join("keys-demo.wav")
        });
    let presets = Keys::presets();
    let total = at(presets.len() as f64 * SECTION + 1.0) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    println!("Keys demo, {} presets, {SECTION} s each:", presets.len());
    for (i, p) in presets.iter().enumerate() {
        let t0 = i as f64 * SECTION;
        let mut ev = Vec::new();
        phrase(t0, &mut ev);
        ev.sort_by_key(|e| e.0);
        let mut s = Keys::new(RATE);
        s.load_preset(p);
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
            "  {m}:{sec:04.1}  {:<14} chords 0:00-0:03.6, C2/C4 ladder 0:03.8, melody 0:05.8   peak {:.1} dBFS",
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
