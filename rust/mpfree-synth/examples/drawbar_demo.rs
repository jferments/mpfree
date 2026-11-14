//! Renders a listening demo of the tonewheel organ (Drawbar) and writes
//! `~/mpfree-work/drawbar-demo.wav` (or the path given).
//!
//! Each factory preset plays, in turn, 9 seconds:
//! - four chords (C7, F9, Bb6/9, G7#9 voicings) with a bass note under
//!   each, played detached so the percussion fires on every one;
//! - a staccato riff on top, for the click and the percussion;
//! - a held chord where the rotary speaker goes to Fast and back to the
//!   preset's speed, so the horn's quick ramp and the drum's slow one can
//!   be heard.
//!
//! It prints the timeline, so a sound can be found by its time, and each
//! preset's peak. All parts run through `Node::process` in 128-frame
//! blocks, as on the audio thread. The master is trimmed to -1 dBFS peak.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::drawbar_params::{SPEED, SPEED_FAST};
use mpfree_synth::Drawbar;

const RATE: u32 = 48_000;
const BLOCK: usize = 128;
const SECTION: f64 = 9.0;

fn at(s: f64) -> u64 {
    (s * f64::from(RATE)).round() as u64
}

/// One preset's phrase, starting at `t0` seconds; `speed` is the preset's
/// own rotor speed, which the held chord returns to.
fn phrase(t0: f64, speed: f32, ev: &mut Vec<(u64, NodeEvent)>) {
    let mut note = |t: f64, len: f64, n: u8| {
        ev.push((
            at(t0 + t),
            NodeEvent::NoteOn {
                note: n,
                velocity: 100,
            },
        ));
        ev.push((at(t0 + t + len), NodeEvent::NoteOff { note: n }));
    };
    let chords: [(&[u8], u8); 4] = [
        (&[58, 64, 67, 72], 36),
        (&[57, 63, 67, 70], 41),
        (&[55, 60, 62, 67], 46),
        (&[59, 65, 70, 74], 43),
    ];
    for (i, (voicing, bass)) in chords.iter().enumerate() {
        let t = i as f64 * 0.9;
        for &n in voicing.iter() {
            note(t, 0.75, n);
        }
        note(t, 0.8, *bass);
    }
    for (i, n) in [72u8, 75, 77, 79, 82, 84, 82, 79].into_iter().enumerate() {
        note(3.8 + i as f64 * 0.22, 0.12, n);
    }
    for &n in &[48u8, 60, 64, 67, 70] {
        note(5.8, 3.0, n);
    }
    let set = |t: f64, value: f32| (at(t0 + t), NodeEvent::Param { id: SPEED, value });
    ev.push(set(6.0, SPEED_FAST));
    ev.push(set(7.6, speed));
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("drawbar-demo.wav")
        });
    let presets = Drawbar::presets();
    let total = at(presets.len() as f64 * SECTION + 1.0) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    println!("Drawbar demo, {} presets, {SECTION} s each:", presets.len());
    for (i, p) in presets.iter().enumerate() {
        let t0 = i as f64 * SECTION;
        let mut s = Drawbar::new(RATE);
        s.load_preset(p);
        let mut ev = Vec::new();
        phrase(t0, s.param(SPEED), &mut ev);
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
            "  {m}:{sec:04.1}  {:<12} chords 0:00-0:03.6, riff 0:03.8, held chord 0:05.8 (Fast 0:06.0, back 0:07.6)   peak {:.1} dBFS",
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
