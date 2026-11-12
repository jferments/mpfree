//! Renders a demo of the Poly synth's presets in a grimy 92 BPM beat and
//! writes `~/mpfree-work/synth-demo.wav` (or the path given).
//!
//! Each preset enters on its own so it can be heard:
//! - bars 1-2: drums and **Grit Bass** (legato, glide, drive)
//! - bars 3-4: plus **Acid** (resonant, a cutoff sweep automated by
//!   parameter events, slides)
//! - bars 5-6: Acid out; **Dust Pad** chords and **Supersaw** stabs (unison)
//! - bars 7-8: plus **Lead** (glide and a pitch bend)
//! - bar 9: a final chord rings out, then a tail of silence-bound release.
//!
//! The synths run through `mpfree_node::Node::process` in 128-frame blocks,
//! exactly as on the audio thread. The drums are synthesised here. It prints
//! a timeline, each stem's level, checks each part sounds only where it
//! should, and trims the master to -1 dBFS peak (no clipping).

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::params::*;
use mpfree_synth::{Poly, PRESETS};

const RATE: u32 = 48_000;
const BPM: f64 = 92.0;
const BLOCK: usize = 128;

fn frames_per_beat() -> f64 {
    60.0 / BPM * f64::from(RATE)
}
/// The frame of a 16th step from the start (bar 1 = steps 0..16).
fn step(s: f64) -> u64 {
    (s * frames_per_beat() / 4.0).round() as u64
}

struct Part {
    name: &'static str,
    synth: Poly,
    events: Vec<(u64, NodeEvent)>,
    gain: f32,
    pan: f32,
    /// Bars (1-based, inclusive) where the part should be heard.
    bars: (u32, u32),
    left: Vec<f32>,
    right: Vec<f32>,
}

fn preset(name: &str) -> &'static Preset {
    PRESETS
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"))
}

fn part(name: &'static str, preset_name: &str, gain: f32, pan: f32, bars: (u32, u32)) -> Part {
    let mut synth = Poly::new(RATE);
    synth.load_preset(preset(preset_name));
    Part {
        name,
        synth,
        events: Vec::new(),
        gain,
        pan,
        bars,
        left: Vec::new(),
        right: Vec::new(),
    }
}

/// A note from step `at` lasting `len` steps.
fn note(p: &mut Part, at: f64, len: f64, n: u8, vel: u8) {
    p.events.push((
        step(at),
        NodeEvent::NoteOn {
            note: n,
            velocity: vel,
        },
    ));
    p.events
        .push((step(at + len), NodeEvent::NoteOff { note: n }));
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("synth-demo.wav")
        });
    let bars = 9.0;
    let tail = 2.5;
    let total = (step(bars * 16.0) as f64 + tail * f64::from(RATE)) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;

    // ---- the parts ----
    let mut bass = part("Grit Bass", "Grit Bass", 2.0, 0.0, (1, 9));
    // E minor riff, 2 bars, played 4 times; overlapping notes slide (legato).
    let riff: [(f64, f64, u8); 9] = [
        (0.0, 3.0, 40),
        (3.0, 1.5, 40),
        (4.0, 2.0, 43),
        (6.0, 2.0, 40),
        (10.0, 2.0, 38),
        (12.0, 3.5, 35),
        (16.0, 3.0, 40),
        (22.0, 3.0, 47),
        (24.5, 7.0, 45),
    ];
    for rep in 0..4 {
        for &(at, len, n) in &riff {
            note(&mut bass, rep as f64 * 32.0 + at, len, n, 110);
        }
    }
    note(&mut bass, 128.0, 14.0, 40, 120);

    let mut acid = part("Acid", "Acid", 1.6, -0.25, (3, 4));
    let line: [u8; 16] = [
        52, 52, 64, 52, 55, 52, 62, 64, 52, 50, 52, 64, 59, 52, 62, 57,
    ];
    for bar in 0..2 {
        for (i, &n) in line.iter().enumerate() {
            let at = 32.0 + bar as f64 * 16.0 + i as f64;
            let slide = i % 4 == 3;
            let accent = i % 4 == 0 || i == 6;
            note(
                &mut acid,
                at,
                if slide { 1.2 } else { 0.55 },
                n,
                if accent { 127 } else { 80 },
            );
        }
    }
    // The cutoff opens over the two bars: automation on exact frames.
    for k in 0..=32 {
        let hz = 250.0 * (2400.0f32 / 250.0).powf(k as f32 / 32.0);
        acid.events.push((
            step(32.0 + k as f64),
            NodeEvent::Param {
                id: CUTOFF,
                value: hz,
            },
        ));
    }

    let mut pad = part("Dust Pad", "Dust Pad", 0.7, 0.0, (5, 9));
    let chords: [[u8; 4]; 4] = [
        [52, 55, 59, 66],
        [48, 52, 55, 59],
        [45, 52, 55, 60],
        [47, 51, 54, 57],
    ];
    for (i, c) in chords.iter().enumerate() {
        for &n in c {
            note(&mut pad, 64.0 + i as f64 * 16.0, 15.5, n, 100);
        }
    }
    for &n in &[52u8, 55, 59, 62, 66] {
        note(&mut pad, 128.0, 14.0, n, 105);
    }

    let mut saw = part("Supersaw", "Supersaw", 1.3, 0.0, (5, 6));
    for (bar, c) in chords.iter().take(2).enumerate() {
        for &s in &[2.0, 6.0, 10.0, 13.0] {
            for &n in c {
                note(&mut saw, 64.0 + bar as f64 * 16.0 + s, 1.0, n + 12, 110);
            }
        }
    }

    let mut lead = part("Lead", "Lead", 1.3, 0.2, (7, 8));
    let melody: [(f64, f64, u8); 7] = [
        (96.0, 3.5, 71),
        (99.0, 1.5, 74),
        (100.0, 4.0, 76),
        (106.0, 2.0, 74),
        (108.0, 6.0, 71),
        (116.0, 4.0, 69),
        (119.5, 8.0, 67),
    ];
    for &(at, len, n) in &melody {
        note(&mut lead, at, len, n, 115);
    }
    // A bend up a whole step and back in bar 8 (range 12: 2/12 = 0.1667).
    for k in 0..=8 {
        let v = (k as f32 / 8.0) * (2.0 / 12.0);
        lead.events
            .push((step(122.0 + k as f64 * 0.25), NodeEvent::Bend { value: v }));
        lead.events.push((
            step(125.0 + k as f64 * 0.25),
            NodeEvent::Bend {
                value: (2.0 / 12.0) - v,
            },
        ));
    }

    let mut parts = vec![bass, acid, pad, saw, lead];

    // ---- render every part through the node interface ----
    for p in &mut parts {
        p.events.sort_by_key(|e| e.0);
        p.left = vec![0.0; total];
        p.right = vec![0.0; total];
        let mut buf: Vec<Timed> = Vec::with_capacity(256);
        let mut e = 0usize;
        for at in (0..total).step_by(BLOCK) {
            buf.clear();
            while e < p.events.len() && p.events[e].0 < (at + BLOCK) as u64 {
                buf.push(Timed::new(
                    (p.events[e].0 - at as u64) as u32,
                    p.events[e].1,
                ));
                e += 1;
            }
            let ctx = Ctx {
                rate: RATE,
                frame: at as u64,
                bpm: BPM,
                beats: at as f64 / frames_per_beat(),
                playing: true,
                sidechain: None,
            };
            p.synth.process(
                &ctx,
                &buf,
                &mut p.left[at..at + BLOCK],
                &mut p.right[at..at + BLOCK],
            );
        }
        assert!(
            p.left.iter().chain(&p.right).all(|x| x.is_finite()),
            "{}: non-finite output",
            p.name
        );
    }

    // ---- drums ----
    let (dl, dr) = drums(total, 8);

    // ---- mix ----
    let mut l = dl.clone();
    let mut r = dr.clone();
    for p in &parts {
        let (gl, gr) = (p.gain * (1.0 - p.pan).sqrt(), p.gain * (1.0 + p.pan).sqrt());
        for i in 0..total {
            l[i] += p.left[i] * gl;
            r[i] += p.right[i] * gr;
        }
    }
    let raw_peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    let trim = 10f32.powf(-1.0 / 20.0) / raw_peak;
    for x in l.iter_mut().chain(r.iter_mut()) {
        *x *= trim;
    }

    // ---- report ----
    let bar_frames = step(16.0) as usize;
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
    let db = |x: f32| 20.0 * x.max(1e-9).log10();
    println!(
        "synth demo: {BPM} BPM, 9 bars + {tail} s tail, {:.1} s, master trim {:+.1} dB",
        total as f64 / f64::from(RATE),
        db(trim)
    );
    println!("timeline:");
    println!("  bars 1-2  drums + Grit Bass (legato slides, drive)");
    println!("  bars 3-4  + Acid (resonance, cutoff sweep 250 Hz -> 2.4 kHz by automation)");
    println!("  bars 5-6  Acid out; + Dust Pad chords, + Supersaw stabs (4-voice unison)");
    println!("  bars 7-8  + Lead (glide, pitch bend up a tone and back in bar 8)");
    println!("  bar  9    final chord rings out");
    let mut ok = true;
    for p in &parts {
        let mono: Vec<f32> = p
            .left
            .iter()
            .zip(&p.right)
            .map(|(a, b)| 0.5 * (a + b) * p.gain * trim)
            .collect();
        let mut line = format!("  {:<10}", p.name);
        for bar in 1..=9u32 {
            let s = (bar as usize - 1) * bar_frames;
            let level = db(rms(&mono[s..(s + bar_frames).min(total)]));
            let inside = bar >= p.bars.0 && bar <= p.bars.1;
            line.push_str(&format!(" {:>6.1}", level));
            // A part must be clearly there in its bars, and gone (bar the
            // release of its last note) two bars after.
            if inside && level < -35.0 {
                ok = false;
                line.push('!');
            }
            if bar >= p.bars.1 + 2 && level > -60.0 {
                ok = false;
                line.push('?');
            }
        }
        println!("{line}");
    }
    {
        let mono: Vec<f32> = dl
            .iter()
            .zip(&dr)
            .map(|(a, b)| 0.5 * (a + b) * trim)
            .collect();
        let mut line = format!("  {:<10}", "Drums");
        for bar in 1..=9usize {
            let s = (bar - 1) * bar_frames;
            line.push_str(&format!(
                " {:>6.1}",
                db(rms(&mono[s..(s + bar_frames).min(total)]))
            ));
        }
        println!("{line}");
    }
    println!("  (dBFS RMS per bar after the master trim, bars 1..9)");
    let final_peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    let last = l[total - 4800..]
        .iter()
        .chain(&r[total - 4800..])
        .fold(0.0f32, |m, x| m.max(x.abs()));
    println!(
        "peak {:.2} dBFS, last 100 ms peak {:.1} dBFS",
        db(final_peak),
        db(last)
    );
    if final_peak > 1.0 {
        println!("WARNING: master clips");
        ok = false;
    }
    if last > 1e-3 {
        println!("WARNING: the end is not quiet");
        ok = false;
    }

    // ---- write ----
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
        "wrote {} ({} frames), PCM hash {:016x}",
        out.display(),
        total,
        fnv(&pcm)
    );
    println!("(the PCM hash repeats on the same computer; other computers' maths libraries can differ in the last bit)");
    if !ok {
        eprintln!("self-check FAILED");
        std::process::exit(1);
    }
    println!("self-check ok");
}

/// Boom-bap drums with grit: a driven kick, a crushed snare, swung hats,
/// vinyl dust. `bars` bars from the start.
fn drums(total: usize, bars: u32) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    let mut seed = 7u64;
    let mut noise = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((seed >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    };
    let rate = RATE as f32;
    let add = |l: &mut Vec<f32>,
               r: &mut Vec<f32>,
               at: u64,
               len: usize,
               pan: f32,
               f: &mut dyn FnMut(usize) -> f32| {
        for k in 0..len {
            let i = at as usize + k;
            if i >= total {
                break;
            }
            // A 3 ms fade at the end of every hit.
            let fade = ((len - k) as f32 / (0.003 * rate)).min(1.0);
            let x = f(k) * fade;
            l[i] += x * (1.0 - pan);
            r[i] += x * (1.0 + pan);
        }
    };
    for bar in 0..bars {
        let b = bar as f64 * 16.0;
        for &s in &[0.0, 7.0, 10.0] {
            let mut ph = 0.0f32;
            add(
                &mut l,
                &mut r,
                step(b + s),
                (0.45 * rate) as usize,
                0.0,
                &mut |k| {
                    let t = k as f32 / rate;
                    ph += (45.0 + 110.0 * (-t * 28.0).exp()) / rate;
                    ((ph * std::f32::consts::TAU).sin() * 2.2).tanh() * (-t * 7.0).exp() * 0.5
                },
            );
        }
        for &s in &[4.0, 12.0] {
            let mut ph = 0.0f32;
            let mut held = 0.0f32;
            add(
                &mut l,
                &mut r,
                step(b + s),
                (0.3 * rate) as usize,
                0.05,
                &mut |k| {
                    let t = k as f32 / rate;
                    ph += 185.0 / rate;
                    if k % 4 == 0 {
                        held = noise();
                    }
                    let x = held * (-t * 14.0).exp() * 0.8
                        + (ph * std::f32::consts::TAU).sin() * (-t * 30.0).exp() * 0.5;
                    ((x * 8.0).round() / 8.0) * 0.32
                },
            );
        }
        for h in 0..8 {
            let s = h as f64 * 2.0 + if h % 2 == 1 { 0.33 } else { 0.0 };
            let mut prev = 0.0f32;
            let vel = if h % 2 == 0 { 0.11 } else { 0.07 };
            add(
                &mut l,
                &mut r,
                step(b + s),
                (0.05 * rate) as usize,
                -0.3,
                &mut |k| {
                    let t = k as f32 / rate;
                    let n = noise();
                    let hp = n - prev;
                    prev = n;
                    hp * (-t * 90.0).exp() * vel
                },
            );
        }
    }
    // Vinyl dust: sparse clicks over the whole beat.
    let beat_end = step(f64::from(bars + 1) * 16.0) as usize;
    let mut i = 0usize;
    while i < beat_end.min(total) {
        let gap = 1500 + ((noise() + 1.0) * 6000.0) as usize;
        i += gap;
        if i + 30 < total {
            let a = noise() * 0.04;
            for k in 0..30 {
                l[i + k] += a * (-(k as f32) / 6.0).exp();
                r[i + k] += a * 0.8 * (-(k as f32) / 6.0).exp();
            }
        }
    }
    (l, r)
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
