//! Renders a demo of Perform FX on a 120 BPM beat and writes
//! `~/mpfree-work/perform-demo.wav` (or the path given).
//!
//! Perform FX sits on the master before Master safety, with entry on the
//! next 1/16. The pads are pressed a little off the grid on purpose (a
//! human finger), so you hear them land on it:
//! - bar 1: dry
//! - bar 2: **Stutter 1/16** on beats 3-4
//! - bar 3: **Beat repeat 1/8**, then a **1/32 roll** on top for the last
//!   beat, let go back to the 1/8 repeat
//! - bar 4: **Tape stop** on beat 4
//! - bar 5: **Vinyl brake** over beats 3-4
//! - bar 6: **Reverse** on beats 2-3
//! - bar 7: **Half speed** for the bar
//! - bar 8: **Pitch drop** over beats 3-4
//! - bar 9: **Trance gate 1/16** with an **HP sweep**
//! - bar 10: **Echo throw 1/8 dotted** on the snare of beat 2, ringing out
//! - bar 11: **LP sweep** for the bar
//! - bar 12: **Crush** on beats 1-2, a **Wash throw** on beat 4, ringing
//!   into bar 13
//! - bar 13: **Noise riser** into a **Beat repeat 1/4** on beat 4
//! - bar 14: dry, then a last hit that rings out to the end of bar 15.
//!
//! It prints each bar's level and what Master safety caught. The
//! self-check fails only if the converter would clip, a sample is
//! non-finite or the end is not quiet.

use std::f32::consts::TAU;

use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::{gain_to_db, ids, MasterSafety, Perform, PERFORM_LENGTHS};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
const BLOCK: usize = 128;
const BARS: usize = 15;

fn beat(b: f64) -> usize {
    (b * 60.0 / BPM * f64::from(RATE)).round() as usize
}
fn bar(b: usize) -> usize {
    beat((b as f64 - 1.0) * 4.0)
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32
}
fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, v| m.max(v.abs()))
}
fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

fn saw(ph: f32, hz: f32) -> f32 {
    let n = ((12_000.0 / hz) as usize).clamp(1, 24);
    (1..=n)
        .map(|k| (TAU * ph * k as f32).sin() / k as f32)
        .sum::<f32>()
        * 0.55
}

type Stereo = (Vec<f32>, Vec<f32>);

fn add(s: &mut Stereo, at: usize, len: usize, pan: f32, f: &mut dyn FnMut(usize) -> f32) {
    let fade = (0.003 * RATE as f32) as usize;
    for k in 0..len {
        let i = at + k;
        if i >= s.0.len() {
            break;
        }
        let x = f(k) * ((len - k) as f32 / fade as f32).min(1.0);
        s.0[i] += x * (1.0 - pan);
        s.1[i] += x * (1.0 + pan);
    }
}

/// The beat: four on the floor, snare on 2 and 4, open and closed hats, a
/// bass line and chord stabs, bars 1 to 14, and a last hit at bar 14.5.
fn beat_mix(total: usize) -> Stereo {
    let mut m: Stereo = (vec![0.0; total], vec![0.0; total]);
    let mut noise = Noise(9);
    let rate = RATE as f32;
    for b in 1..=13 {
        let b0 = (b as f64 - 1.0) * 4.0;
        for k in 0..4 {
            let mut ph = 0.0f32;
            add(
                &mut m,
                beat(b0 + k as f64),
                (0.35 * rate) as usize,
                0.0,
                &mut |n| {
                    let t = n as f32 / rate;
                    ph += (48.0 + 140.0 * (-t * 32.0).exp()) / rate;
                    ((ph * TAU).sin() * 2.5).tanh() * (-t * 7.0).exp() * 0.6
                },
            );
        }
        for k in [1.0, 3.0] {
            let mut ph = 0.0f32;
            add(
                &mut m,
                beat(b0 + k),
                (0.2 * rate) as usize,
                0.05,
                &mut |n| {
                    let t = n as f32 / rate;
                    ph += 190.0 / rate;
                    noise.next() * (-t * 16.0).exp() * 0.32
                        + (ph * TAU).sin() * (-t * 28.0).exp() * 0.25
                },
            );
        }
        for h in 0..8 {
            let open = h % 2 == 1;
            let mut prev = 0.0f32;
            let decay = if open { 18.0 } else { 90.0 };
            add(
                &mut m,
                beat(b0 + h as f64 * 0.5),
                (if open { 0.18 } else { 0.05 } * rate) as usize,
                -0.3,
                &mut |n| {
                    let x = noise.next();
                    let y = x - prev;
                    prev = x;
                    y * (-(n as f32) / rate * decay).exp() * if open { 0.07 } else { 0.06 }
                },
            );
        }
        let root = [33, 33, 36, 31][(b - 1) % 4];
        for (s, len, step) in [
            (0.0, 0.4, 0),
            (0.75, 0.2, 12),
            (1.5, 0.4, 0),
            (2.5, 0.4, 7),
            (3.25, 0.4, 0),
        ] {
            let f = hz(root + step);
            let mut ph = 0.0f32;
            add(&mut m, beat(b0 + s), beat(len), 0.0, &mut |n| {
                ph = (ph + f / rate).fract();
                saw(ph, f) * (n as f32 / (0.004 * rate)).min(1.0) * 0.3
            });
        }
        let chord: [i32; 3] = match (b - 1) % 4 {
            0 => [57, 60, 64],
            1 => [57, 60, 64],
            2 => [60, 64, 67],
            _ => [55, 59, 62],
        };
        for s in [0.5, 1.75, 3.0] {
            for (v, &n) in chord.iter().enumerate() {
                let f = hz(n);
                let mut ph = 0.0f32;
                add(
                    &mut m,
                    beat(b0 + s),
                    beat(0.4),
                    (v as f32 - 1.0) * 0.4,
                    &mut |k| {
                        ph = (ph + f / rate).fract();
                        let t = k as f32 / rate;
                        saw(ph, f) * (t / 0.004).min(1.0) * (-t * 7.0).exp() * 0.12
                    },
                );
            }
        }
    }
    // The last hit: kick, low A and an A minor chord, ringing 1.5 bars.
    let at = beat(53.0);
    let mut ph = 0.0f32;
    add(&mut m, at, (0.5 * rate) as usize, 0.0, &mut |n| {
        let t = n as f32 / rate;
        ph += (48.0 + 140.0 * (-t * 32.0).exp()) / rate;
        ((ph * TAU).sin() * 2.5).tanh() * (-t * 5.0).exp() * 0.6
    });
    for (v, &n) in [45, 57, 60, 64].iter().enumerate() {
        let f = hz(n);
        let mut ph = 0.0f32;
        add(&mut m, at, beat(5.0), (v as f32 - 1.5) * 0.3, &mut |k| {
            ph = (ph + f / rate).fract();
            let t = k as f32 / rate;
            saw(ph, f) * (t / 0.004).min(1.0) * (-t * 1.6).exp() * 0.14
        });
    }
    // Some headroom, so the effects are heard rather than the limiter.
    for x in m.0.iter_mut().chain(m.1.iter_mut()) {
        *x *= 0.6;
    }
    m
}

fn length(name: &str) -> f32 {
    PERFORM_LENGTHS.iter().position(|n| *n == name).unwrap() as f32
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("perform-demo.wav")
        });
    let total = bar(BARS + 1);
    let dry = beat_mix(total);

    let mut perform = Perform::new();
    perform.prepare(RATE, BLOCK);
    // The default bank, with the riser shortened to one bar.
    perform.set_param(
        ids::perform::slot(12) + ids::perform::LENGTH,
        length("1 bar"),
    );
    perform.reset();
    let mut safety = MasterSafety::new();
    safety.prepare(RATE, BLOCK);

    // (pad, pressed on beat, let go on beat), beats counted from 0. Pads
    // go down up to 1/32 early or late, as a finger does.
    let pads: [(u8, f64, f64); 17] = [
        (0, 6.0 - 0.06, 8.0 - 0.04),    // bar 2: stutter 1/16
        (1, 10.0 + 0.05, 12.0),         // bar 3: beat repeat 1/8...
        (13, 11.0 - 0.03, 11.9),        // ...with a 1/32 roll on top
        (2, 15.0 + 0.02, 16.0),         // bar 4: tape stop
        (3, 18.0 - 0.05, 20.0 - 0.02),  // bar 5: vinyl brake
        (4, 21.0 + 0.04, 23.0),         // bar 6: reverse
        (5, 24.0 - 0.02, 28.0 - 0.03),  // bar 7: half speed
        (6, 30.0 + 0.03, 32.0 - 0.02),  // bar 8: pitch drop
        (7, 32.0 - 0.04, 36.0 - 0.01),  // bar 9: trance gate...
        (8, 32.0 + 0.02, 36.0 - 0.01),  // ...and an HP sweep
        (9, 37.0 - 0.02, 37.3),         // bar 10: echo throw 1/8D
        (10, 40.0 - 0.03, 44.0 - 0.02), // bar 11: LP sweep
        (11, 44.0 + 0.04, 46.0),        // bar 12: crush
        (15, 47.0 - 0.02, 47.6),        // wash throw
        (12, 48.0 - 0.02, 51.0),        // bar 13: noise riser...
        (14, 51.0 - 0.03, 52.0 - 0.01), // ...into a beat repeat 1/4
        (0, 52.0 + 0.5, 52.0 + 0.5),    // bar 14: a tap shorter than the grid: nothing
    ];
    let mut events: Vec<(usize, NodeEvent)> = Vec::new();
    for &(pad, down, up) in &pads {
        events.push((
            beat(down),
            NodeEvent::NoteOn {
                note: pad,
                velocity: 100,
            },
        ));
        events.push((beat(up), NodeEvent::NoteOff { note: pad }));
    }
    events.sort_by_key(|e| e.0);

    let (mut l, mut r) = dry.clone();
    let mut evs = Vec::new();
    let (mut at, mut k) = (0, 0);
    while at < total {
        let len = BLOCK.min(total - at);
        evs.clear();
        while k < events.len() && events[k].0 < at + len {
            evs.push(Timed::new((events[k].0 - at) as u32, events[k].1));
            k += 1;
        }
        let ctx = Ctx {
            bpm: BPM,
            beats: at as f64 * BPM / 60.0 / f64::from(RATE),
            playing: true,
            ..Ctx::new(RATE, at as u64)
        };
        perform.process(&ctx, &evs, &mut l[at..at + len], &mut r[at..at + len]);
        safety.process(&ctx, &[], &mut l[at..at + len], &mut r[at..at + len]);
        at += len;
    }

    let sections = [
        "dry",
        "stutter 1/16",
        "repeat 1/8 + roll",
        "tape stop",
        "vinyl brake",
        "reverse",
        "half speed",
        "pitch drop",
        "gate + HP sweep",
        "echo throw",
        "LP sweep",
        "crush + wash",
        "riser + repeat",
        "dry, last hit",
        "ring out",
    ];
    println!("Perform FX demo: {BARS} bars at {BPM} BPM, {RATE} Hz, {BLOCK}-frame blocks\n");
    println!(
        "  bar  {:<18} {:>9} {:>9} {:>9}",
        "pads", "dry RMS", "out peak", "out RMS"
    );
    for (b, section) in sections.iter().enumerate() {
        let (r0, r1) = (bar(b + 1), bar(b + 2).min(total));
        println!(
            "  {:>3}  {:<18} {:>6.1} dB {:>6.1} dB {:>6.1} dB",
            b + 1,
            section,
            gain_to_db(rms(&dry.0[r0..r1])),
            gain_to_db(peak(&l[r0..r1]).max(peak(&r[r0..r1]))),
            gain_to_db(rms(&l[r0..r1]))
        );
    }
    let s = safety.stats();
    let last = peak(&l[total - 4800..]).max(peak(&r[total - 4800..]));
    println!(
        "\nMaster safety: caught {} overs, most limiting {:.1} dB, peak out {:.2} dBFS, clipped frames {}",
        s.overs, s.max_reduction_db, s.peak_out_db, s.clipped
    );
    println!(
        "Perform FX memory: {:.1} MB; last 100 ms peak {:.1} dBFS",
        perform.memory_bytes() as f32 / 1e6,
        gain_to_db(last)
    );
    let mut ok = true;
    if s.clipped > 0 || peak(&l).max(peak(&r)) > 1.0 {
        println!("FAIL: the converter would clip");
        ok = false;
    }
    if !l.iter().chain(&r).all(|v| v.is_finite()) {
        println!("FAIL: non-finite output");
        ok = false;
    }
    if last > 1e-3 {
        println!("FAIL: the end is not quiet");
        ok = false;
    }
    let mut pcm = Vec::with_capacity(total * 4);
    for i in 0..total {
        for x in [l[i], r[i]] {
            pcm.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes());
        }
    }
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).expect("create output dir");
    }
    std::fs::write(&out, wav(&pcm)).expect("write wav");
    println!(
        "wrote {} ({} frames, {:.1} s), PCM hash {:016x}",
        out.display(),
        total,
        total as f32 / RATE as f32,
        fnv(&pcm)
    );
    if !ok {
        eprintln!("self-check FAILED");
        std::process::exit(1);
    }
    println!("self-check ok");
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

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}
