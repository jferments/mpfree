//! Renders the Ducker, Transient shaper and Pitch shifter, each dry and
//! then with its effect, at 120 BPM, and writes
//! `~/mpfree-work/new-fx-demo.wav` (or the path given).
//!
//! - bars 1-4: a held pad chord under a kick. Bars 1-2 dry; bars 3-4 the
//!   pad goes through the **Ducker** keyed by the kick ("Kick duck").
//! - bars 5-8: the same pad alone. Bars 5-6 the Ducker in Tempo mode
//!   ("Big pump", quarters); bars 7-8 "Sixteenth gate".
//! - bars 9-12: a drum loop. Bars 9-10 dry; bar 11 the **Transient shaper**
//!   at "Punchy kick"; bar 12 at "Tight room".
//! - bars 13-16: a pluck melody. Bar 13 dry; then the **Pitch shifter**:
//!   bar 14 an octave down, bar 15 a fifth up (mixed in for harmony), bar
//!   16 "Wide doubler".
//!
//! Everything runs in 128-frame blocks through Master safety, as on the
//! audio thread. It prints each bar's level and checks that nothing is
//! non-finite and the converter would not clip.

use std::f32::consts::TAU;

use mpfree_fx::ids::{ducker, pitch, transient};
use mpfree_fx::mpfree_node::{Ctx, Node};
use mpfree_fx::{gain_to_db, presets, Ducker, MasterSafety, PitchShifter, TransientShaper};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
const BLOCK: usize = 128;
const BARS: usize = 16;

fn bar_frames() -> usize {
    (4.0 * 60.0 / BPM * f64::from(RATE)).round() as usize
}
fn beat_frames() -> usize {
    bar_frames() / 4
}
fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
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

/// A kick on every beat: a falling sine with a click.
fn kick(total: usize) -> Vec<f32> {
    let mut out = vec![0.0; total];
    let b = beat_frames();
    for start in (0..total).step_by(b) {
        let mut ph = 0.0f32;
        for i in 0..(RATE as usize / 3).min(total - start) {
            let t = i as f32 / RATE as f32;
            let f = 45.0 + 110.0 * (-t * 30.0).exp();
            ph += TAU * f / RATE as f32;
            out[start + i] += 0.9 * (-t * 9.0).exp() * ph.sin();
        }
    }
    out
}

/// A held minor chord of detuned saws, gently low-passed.
fn pad(total: usize) -> Vec<f32> {
    let notes = [57, 60, 64, 67];
    let mut phases = [0.0f32; 8];
    let mut lp = 0.0f32;
    (0..total)
        .map(|_| {
            let mut s = 0.0;
            for (k, ph) in phases.iter_mut().enumerate() {
                let f = hz(notes[k / 2]) * if k % 2 == 0 { 0.997 } else { 1.003 };
                *ph = (*ph + f / RATE as f32).fract();
                s += 2.0 * *ph - 1.0;
            }
            lp += 0.08 * (s * 0.15 - lp);
            lp
        })
        .collect()
}

/// A drum loop: kick on 1 and 3, snare on 2 and 4, hats on the 8ths, in a
/// small room.
fn drums(total: usize) -> Vec<f32> {
    let mut out = vec![0.0; total];
    let b = beat_frames();
    let mut n = Noise(0xD2);
    let k = kick(b * 2);
    for start in (0..total).step_by(b * 2) {
        for i in 0..b.min(total - start) {
            out[start + i] += k[i];
        }
    }
    for start in (b..total).step_by(b * 2) {
        for i in 0..(RATE as usize / 4).min(total - start) {
            let t = i as f32 / RATE as f32;
            let body = (TAU * 190.0 * t).sin() * (-t * 30.0).exp();
            // The room: a slower tail under the crack.
            let tail = n.next() * (0.6 * (-t * 40.0).exp() + 0.15 * (-t * 8.0).exp());
            out[start + i] += 0.5 * body + 0.5 * tail;
        }
    }
    for start in (0..total).step_by(b / 2) {
        for i in 0..(RATE as usize / 20).min(total - start) {
            let t = i as f32 / RATE as f32;
            out[start + i] += 0.15 * n.next() * (-t * 120.0).exp();
        }
    }
    out
}

/// A plucked melody, one note per 8th.
fn pluck(total: usize) -> Vec<f32> {
    let melody = [69, 72, 76, 74, 72, 69, 67, 64];
    let mut out = vec![0.0; total];
    let step = beat_frames() / 2;
    for (s, start) in (0..total).step_by(step).enumerate() {
        let f = hz(melody[s % melody.len()]);
        let mut ph = 0.0f32;
        for i in 0..(step * 2).min(total - start) {
            let t = i as f32 / RATE as f32;
            ph = (ph + f / RATE as f32).fract();
            let tri = 1.0 - 4.0 * (ph - 0.5).abs();
            out[start + i] += 0.35 * tri * (-t * 6.0).exp();
        }
    }
    out
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

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let dir = format!("{home}/mpfree-work");
        let _ = std::fs::create_dir_all(&dir);
        format!("{dir}/new-fx-demo.wav")
    });
    let bf = bar_frames();
    let total = BARS * bf;
    let bar = |b: usize| (b - 1) * bf;

    let k = kick(total);
    let pd = pad(total);
    let dr = drums(total);
    let pl = pluck(total);

    let mut duck = Ducker::new();
    let mut shaper = TransientShaper::new();
    let mut shift = PitchShifter::new();
    let mut master = MasterSafety::new();
    for n in [
        &mut duck as &mut dyn Node,
        &mut shaper,
        &mut shift,
        &mut master,
    ] {
        n.prepare(RATE, BLOCK);
        n.reset();
    }

    let (mut out_l, mut out_r) = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut at = 0;
    let (mut pl_l, mut pl_r) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    let (mut sl, mut sr) = (vec![0.0f32; BLOCK], vec![0.0f32; BLOCK]);
    while at < total {
        let len = BLOCK.min(total - at);
        let b = at / bf + 1;
        let at_bar_start = at % bf < BLOCK;
        let ctx = Ctx {
            bpm: BPM,
            beats: at as f64 * BPM / 60.0 / f64::from(RATE),
            playing: true,
            ..Ctx::new(RATE, at as u64)
        };
        // Section changes on the bar line.
        if at_bar_start && at % bf == 0 {
            match b {
                3 => {
                    set_preset(&mut duck, "Kick duck");
                    duck.reset();
                }
                5 => {
                    duck = Ducker::new();
                    set_preset(&mut duck, "Big pump");
                    duck.reset();
                }
                7 => {
                    duck = Ducker::new();
                    set_preset(&mut duck, "Sixteenth gate");
                    duck.reset();
                }
                11 => {
                    set_preset(&mut shaper, "Punchy kick");
                    shaper.reset();
                }
                12 => {
                    shaper = TransientShaper::new();
                    set_preset(&mut shaper, "Tight room");
                    shaper.reset();
                }
                14 => {
                    shift.set_param(pitch::PITCH, -12.0);
                    shift.set_param(pitch::WINDOW, 80.0);
                    shift.reset();
                }
                15 => {
                    shift = PitchShifter::new();
                    shift.set_param(pitch::PITCH, 7.0);
                    shift.set_param(pitch::MIX, 45.0);
                    shift.reset();
                }
                16 => {
                    shift = PitchShifter::new();
                    set_preset(&mut shift, "Wide doubler");
                    shift.reset();
                }
                _ => {}
            }
        }
        let (l, r) = (&mut out_l[at..at + len], &mut out_r[at..at + len]);
        match b {
            1..=4 => {
                sl[..len].copy_from_slice(&pd[at..at + len]);
                sr[..len].copy_from_slice(&pd[at..at + len]);
                if b >= 3 {
                    let key = &k[at..at + len];
                    let c = Ctx {
                        sidechain: Some((key, key)),
                        ..ctx
                    };
                    duck.process(&c, &[], &mut sl[..len], &mut sr[..len]);
                }
                for i in 0..len {
                    l[i] = sl[i] + 0.8 * k[at + i];
                    r[i] = sr[i] + 0.8 * k[at + i];
                }
            }
            5..=8 => {
                sl[..len].copy_from_slice(&pd[at..at + len]);
                sr[..len].copy_from_slice(&pd[at..at + len]);
                duck.process(&ctx, &[], &mut sl[..len], &mut sr[..len]);
                l.copy_from_slice(&sl[..len]);
                r.copy_from_slice(&sr[..len]);
            }
            9..=12 => {
                // Headroom for the shaper's lifted fronts.
                for i in 0..len {
                    l[i] = 0.4 * dr[at + i];
                    r[i] = 0.4 * dr[at + i];
                }
                if b >= 11 {
                    shaper.process(&ctx, &[], l, r);
                }
            }
            _ => {
                pl_l[..len].copy_from_slice(&pl[at..at + len]);
                pl_r[..len].copy_from_slice(&pl[at..at + len]);
                if b >= 14 {
                    shift.process(&ctx, &[], &mut pl_l[..len], &mut pl_r[..len]);
                }
                l.copy_from_slice(&pl_l[..len]);
                r.copy_from_slice(&pl_r[..len]);
            }
        }
        master.process(&ctx, &[], l, r);
        at += len;
    }

    println!("bar  level (dBFS rms)");
    for b in 1..=BARS {
        let s = &out_l[bar(b)..bar(b) + bf];
        println!("{b:>3}  {:>6.1}", gain_to_db(rms(s)));
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
    let _ = (ducker::DEPTH, transient::ATTACK);
    std::fs::write(&path, mpfree_fx::wav_f32(&out_l, &out_r, RATE)).expect("write the WAV");
    println!("wrote {path}");
}
