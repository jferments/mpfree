//! Listening renders for the MPC coverage effects and XY FX: one WAV per
//! effect in the folder given (default `~/mpfree-work/coverage`), at 120
//! BPM. Each starts with a bar of the source dry, then two bars of every
//! factory preset in turn. XY FX sweeps X up over the first bar of each
//! preset and down over the second, and closes Active for the last half
//! bar, so the release (or the echo's throw) is heard.
//!
//! Everything runs in 128-frame blocks through Master safety, as on the
//! audio thread. It prints each file's sections and peak, and fails if
//! anything is non-finite.

use std::f32::consts::TAU;
use std::path::PathBuf;

use mpfree_fx::ids::xyfx;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::{registry, MasterSafety};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
const BLOCK: usize = 128;

fn bar() -> usize {
    (4.0 * 60.0 / BPM * f64::from(RATE)).round() as usize
}
fn beat() -> usize {
    bar() / 4
}
fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

struct Noise(u32);
impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

fn kick_at(out: &mut [f32], start: usize, level: f32) {
    let mut ph = 0.0f32;
    for i in 0..(RATE as usize / 3).min(out.len().saturating_sub(start)) {
        let t = i as f32 / RATE as f32;
        ph += TAU * (45.0 + 110.0 * (-t * 30.0).exp()) / RATE as f32;
        out[start + i] += level * (-t * 9.0).exp() * ph.sin();
    }
}

/// Kick on 1 and 3, snare with a room tail on 2 and 4, hats on the 8ths.
fn drums(total: usize) -> Vec<f32> {
    let mut out = vec![0.0; total];
    let b = beat();
    let mut n = Noise(0xD2);
    for start in (0..total).step_by(b * 2) {
        kick_at(&mut out, start, 0.8);
    }
    for start in (b..total).step_by(b * 2) {
        for i in 0..(RATE as usize / 3).min(total - start) {
            let t = i as f32 / RATE as f32;
            let body = (TAU * 190.0 * t).sin() * (-t * 30.0).exp();
            let tail = n.next() * (0.6 * (-t * 40.0).exp() + 0.12 * (-t * 7.0).exp());
            out[start + i] += 0.5 * body + 0.5 * tail;
        }
    }
    for start in (0..total).step_by(b / 2) {
        for i in 0..(RATE as usize / 20).min(total - start) {
            let t = i as f32 / RATE as f32;
            out[start + i] += 0.12 * n.next() * (-t * 120.0).exp();
        }
    }
    out
}

/// A held minor chord of detuned saws, gently low-passed.
fn pad(total: usize) -> Vec<f32> {
    let notes = [57, 60, 64, 67];
    let mut ph = [0.0f32; 8];
    let mut lp = 0.0f32;
    (0..total)
        .map(|_| {
            let mut s = 0.0;
            for (k, p) in ph.iter_mut().enumerate() {
                let f = hz(notes[k / 2]) * if k % 2 == 0 { 0.997 } else { 1.003 };
                *p = (*p + f / RATE as f32).fract();
                s += 2.0 * *p - 1.0;
            }
            lp += 0.08 * (s * 0.15 - lp);
            lp
        })
        .collect()
}

/// Plucked notes: `every` frames apart, from `melody`, `octave` shifted.
fn pluck(total: usize, every: usize, octave: i32, level: f32) -> Vec<f32> {
    let melody = [69, 72, 76, 74, 72, 69, 67, 64];
    let mut out = vec![0.0; total];
    for (s, start) in (0..total).step_by(every).enumerate() {
        let f = hz(melody[s % melody.len()] + 12 * octave);
        let mut ph = 0.0f32;
        for i in 0..(beat() * 2).min(total - start) {
            let t = i as f32 / RATE as f32;
            ph = (ph + f / RATE as f32).fract();
            // A saw with a fast-closing brightness, like a picked string.
            let saw = 2.0 * ph - 1.0;
            let tri = 1.0 - 4.0 * (ph - 0.5).abs();
            let bright = (-t * 18.0).exp();
            out[start + i] += level * (bright * saw + (1.0 - bright) * tri) * (-t * 4.0).exp();
        }
    }
    out
}

/// A voice-like tone with an "s" and a "t" on each 8th: sibilance to
/// tame.
fn voice(total: usize) -> Vec<f32> {
    let mut out = pluck(total, beat() / 2, -1, 0.3);
    let mut n = Noise(0x5E55);
    let (mut hp, mut x1) = (0.0f32, 0.0f32);
    for start in (beat() / 4..total).step_by(beat() / 2) {
        for i in 0..(RATE as usize / 12).min(total - start) {
            let t = i as f32 / RATE as f32;
            let x = n.next();
            // A crude high-pass: the "s" lives above 5 kHz.
            hp = 0.55 * (hp + x - x1);
            x1 = x;
            out[start + i] += 0.45 * hp * (-t * 25.0).exp();
        }
    }
    out
}

fn mix(a: &[f32], b: &[f32], gb: f32) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + gb * y).collect()
}

/// The source each effect is heard on.
fn source(id: &str, total: usize) -> Vec<f32> {
    match id {
        "mpfree.fx.tremolo" | "mpfree.fx.autopan" | "mpfree.fx.rotary" => pad(total),
        "mpfree.fx.ringmod" | "mpfree.fx.exciter" => {
            mix(&pluck(total, beat() / 2, 0, 0.35), &pad(total), 0.6)
        }
        "mpfree.fx.limiter" => drums(total).iter().map(|v| v * 1.8).collect(),
        "mpfree.fx.gate" => {
            let mut n = Noise(9);
            drums(total).iter().map(|v| v + 0.01 * n.next()).collect()
        }
        "mpfree.fx.deesser" => voice(total),
        "mpfree.fx.multiband" | "mpfree.fx.xy" => mix(&drums(total), &pad(total), 0.7),
        "mpfree.fx.room" | "mpfree.fx.plate" | "mpfree.fx.spring" => {
            let mut d = drums(total);
            for (i, v) in d.iter_mut().enumerate() {
                // Drums on alternate bars only, so the tails are heard.
                if (i / bar()) % 2 == 1 {
                    *v *= 0.0;
                }
            }
            mix(&d, &pluck(total, beat(), 0, 0.35), 1.0)
        }
        "mpfree.fx.tape_delay" | "mpfree.fx.multitap" => pluck(total, beat() * 2, 0, 0.4),
        "mpfree.fx.amp" | "mpfree.fx.autowah" => pluck(total, beat() / 2, -2, 0.5),
        "mpfree.fx.freqshift" => mix(&drums(total), &pad(total), 0.7),
        "mpfree.fx.sample_delay" => drums(total),
        "mpfree.fx.channel_strip" => {
            let mut n = Noise(11);
            mix(&drums(total), &voice(total), 0.5)
                .iter()
                .map(|v| v + 0.01 * n.next())
                .collect()
        }
        "mpfree.fx.granulator" => pluck(total, beat(), 0, 0.35),
        _ => pad(total),
    }
}

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into()))
                .join("mpfree-work/coverage")
        });
    std::fs::create_dir_all(&dir).expect("make the output folder");
    assert_eq!(bar() % BLOCK, 0, "bars are whole blocks");
    let ids = [
        "mpfree.fx.tremolo",
        "mpfree.fx.autopan",
        "mpfree.fx.ringmod",
        "mpfree.fx.rotary",
        "mpfree.fx.limiter",
        "mpfree.fx.gate",
        "mpfree.fx.deesser",
        "mpfree.fx.multiband",
        "mpfree.fx.room",
        "mpfree.fx.plate",
        "mpfree.fx.spring",
        "mpfree.fx.tape_delay",
        "mpfree.fx.multitap",
        "mpfree.fx.amp",
        "mpfree.fx.exciter",
        "mpfree.fx.xy",
        "mpfree.fx.autowah",
        "mpfree.fx.talkbox",
        "mpfree.fx.freqshift",
        "mpfree.fx.width",
        "mpfree.fx.sample_delay",
        "mpfree.fx.channel_strip",
        "mpfree.fx.granulator",
    ];
    let mut index = String::from(
        "MPFree coverage effects: listening renders (120 BPM, 2 s a bar).\nEach file: bar 1 dry, then two bars per preset.\n\n",
    );
    for id in ids {
        let e = registry::entry(id).expect("registered");
        let sections = 1 + 2 * e.presets.len();
        let total = sections * bar() + bar(); // a bar to ring out
                                              // Headroom, so Master safety only catches what the effect adds
                                              // (the limiter's source is hot on purpose).
        let trim = if id == "mpfree.fx.limiter" { 1.0 } else { 0.6 };
        let input: Vec<f32> = source(id, total).iter().map(|v| v * trim).collect();
        let (mut l, mut r) = (input.clone(), input);
        let mut fx = (e.make)();
        fx.prepare(RATE, BLOCK);
        let mut safety = MasterSafety::new();
        safety.prepare(RATE, BLOCK);
        let mut start = 0;
        // The effect runs from the first preset to the end (the last bar
        // lets it ring out).
        let mut on = false;
        let mut events = Vec::new();
        while start < total {
            let end = (start + BLOCK).min(total);
            let section = start / bar();
            // Section 0 is dry; section 1 + 2k and 2 + 2k are preset k.
            let want = section >= 1 && section < sections;
            let k = section.saturating_sub(1) / 2;
            events.clear();
            // A bar is a whole number of blocks at 120 BPM and 48 kHz, so
            // each preset starts on a block.
            if want && start.is_multiple_of(bar()) && (section - 1).is_multiple_of(2) {
                // A new preset: a fresh effect at the preset's values.
                for p in fx.params().to_vec() {
                    if !p.readonly {
                        fx.set_param(p.id, p.default);
                    }
                }
                for &(pid, v) in e.presets[k].values {
                    fx.set_param(pid, v);
                }
                fx.reset();
            }
            on = on || want;
            if want && id == "mpfree.fx.xy" {
                // X up over the first bar, down over the second; Active
                // closes for the last half bar.
                let at = start - (1 + 2 * k) * bar();
                let pos = at as f32 / bar() as f32;
                let x = if pos < 1.0 { pos } else { 2.0 - pos };
                events.push(Timed {
                    offset: 0,
                    ev: NodeEvent::Param {
                        id: xyfx::X,
                        value: (x * 100.0).clamp(0.0, 100.0),
                    },
                });
                events.push(Timed {
                    offset: 0,
                    ev: NodeEvent::Param {
                        id: xyfx::ACTIVE,
                        value: if pos < 1.5 { 1.0 } else { 0.0 },
                    },
                });
            }
            let mut ctx = Ctx::new(RATE, start as u64);
            ctx.bpm = BPM;
            ctx.playing = true;
            ctx.beats = start as f64 / f64::from(RATE) * BPM / 60.0;
            if on {
                fx.process(&ctx, &events, &mut l[start..end], &mut r[start..end]);
            }
            safety.process(&ctx, &[], &mut l[start..end], &mut r[start..end]);
            start = end;
        }
        assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{id}");
        let peak = l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()));
        let file = format!("{}.wav", e.name.to_lowercase().replace([' ', '-'], "_"));
        std::fs::write(dir.join(&file), mpfree_fx::wav_f32(&l, &r, RATE)).expect("write");
        let list: Vec<String> = e
            .presets
            .iter()
            .enumerate()
            .map(|(k, p)| format!("{}s {}", 2 + 4 * k, p.name))
            .collect();
        let line = format!("{file}: 0s dry, {} (peak {:.2})\n", list.join(", "), peak);
        print!("{line}");
        index.push_str(&line);
    }
    std::fs::write(dir.join("README.txt"), index).expect("write the index");
}
