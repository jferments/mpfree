//! Renders a demo of the drum synth and the pluck synth in a grimy 90 BPM
//! beat and writes `~/mpfree-work/drum-pluck-demo.wav` (or the path given).
//!
//! Every two bars something new comes in, so each part can be heard:
//! - bars 1-2: **Init Kit** alone (kick, snare, clap, hats where the closed
//!   hat chokes the open hat, a tom fill)
//! - bars 3-4: **Grit Kit** (everything driven) and the **Muted Bass** pluck
//! - bars 5-6: **Boom Kit** (long 808-style booms), bass, and a **Koto**
//!   line (Karplus-Strong, plucked near the bridge)
//! - bars 7-8: **Electro Kit** (zaps, FM perc), bass, and a **Marimba** riff
//!   (the modal bank)
//! - bars 9-10: Grit Kit, bass, **Bell** melody and **Kalimba** arpeggio
//! - bar 11: a **Glass** chord and a last boom ring out
//!
//! All parts run through `mpfree_node::Node::process` in 128-frame blocks,
//! as on the audio thread. It prints a timeline and each part's level per
//! bar, checks each part sounds only where it should, and trims the master
//! to -1 dBFS peak.

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::{Drums, Pluck};

const RATE: u32 = 48_000;
const BPM: f64 = 90.0;
const BLOCK: usize = 128;
const BARS: u32 = 11;

fn step(s: f64) -> u64 {
    (s * 60.0 / BPM * f64::from(RATE) / 4.0).round() as u64
}

struct Part {
    name: &'static str,
    node: Box<dyn Node>,
    events: Vec<(u64, NodeEvent)>,
    /// Kit changes: (frame, kit index), applied between blocks.
    kits: Vec<(u64, usize)>,
    gain: f32,
    bars: (u32, u32),
    left: Vec<f32>,
    right: Vec<f32>,
}

fn pluck(name: &'static str, preset: &str, gain: f32, bars: (u32, u32)) -> Part {
    let mut p = Pluck::new(RATE);
    let pr = Pluck::presets()
        .iter()
        .find(|p| p.name == preset)
        .unwrap_or_else(|| panic!("no preset {preset}"));
    p.load_preset(pr);
    Part {
        name,
        node: Box::new(p),
        events: Vec::new(),
        kits: Vec::new(),
        gain,
        bars,
        left: Vec::new(),
        right: Vec::new(),
    }
}

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

/// A drum hit on pad `pad` (1 to 16) at step `at`.
fn hit(p: &mut Part, at: f64, pad: u8, vel: u8) {
    p.events.push((
        step(at),
        NodeEvent::NoteOn {
            note: 35 + pad,
            velocity: vel,
        },
    ));
}

fn kit_index(name: &str) -> usize {
    Drums::presets()
        .iter()
        .position(|k| k.name == name)
        .unwrap_or_else(|| panic!("no kit {name}"))
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("drum-pluck-demo.wav")
        });
    let tail = 3.5;
    let total = (step(f64::from(BARS) * 16.0) as f64 + tail * f64::from(RATE)) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;

    // ---- drums ----
    let mut drums = Part {
        name: "Drums",
        node: Box::new(Drums::new(RATE)),
        events: Vec::new(),
        kits: Vec::new(),
        gain: 1.0,
        bars: (1, 11),
        left: Vec::new(),
        right: Vec::new(),
    };
    let kits = [
        "Init Kit",
        "Init Kit",
        "Grit Kit",
        "Grit Kit",
        "Boom Kit",
        "Boom Kit",
        "Electro Kit",
        "Electro Kit",
        "Grit Kit",
        "Grit Kit",
        "Boom Kit",
    ];
    for (bar, k) in kits.iter().enumerate() {
        // Kits change just before the bar line, after the last hit's start.
        drums
            .kits
            .push((step(bar as f64 * 16.0).saturating_sub(64), kit_index(k)));
    }
    for bar in 0..10u32 {
        let b = f64::from(bar) * 16.0;
        // Kick (pad 1): boom-bap with a pickup.
        for &s in &[0.0, 7.0, 10.0] {
            hit(&mut drums, b + s, 1, 120);
        }
        if bar % 2 == 1 {
            hit(&mut drums, b + 15.0, 1, 90);
        }
        // Snare (pad 2) on 2 and 4, a clap (pad 3) doubling the 4.
        hit(&mut drums, b + 4.0, 2, 118);
        hit(&mut drums, b + 12.0, 2, 125);
        hit(&mut drums, b + 12.0, 3, 100);
        // Swung closed hats (pad 4); an open hat (pad 5) on the "and" of 4,
        // choked by the next bar's first closed hat.
        for h in 0..8 {
            if h == 7 {
                continue;
            }
            let s = f64::from(h) * 2.0 + if h % 2 == 1 { 0.4 } else { 0.0 };
            hit(&mut drums, b + s, 4, if h % 2 == 0 { 105 } else { 70 });
        }
        hit(&mut drums, b + 14.4, 5, 100);
        // Percussion: rim (9) and cowbell (10) in the Init kit bars, zaps
        // and FM perc (pads 6-8, 13) in the Electro bars.
        if bar < 2 {
            hit(&mut drums, b + 6.0, 9, 90);
            hit(&mut drums, b + 11.0, 10, 70);
        }
        if (6..8).contains(&bar) {
            hit(&mut drums, b + 3.0, 13, 110);
            hit(&mut drums, b + 9.0, 13, 80);
        }
        // A tom fill at the end of every second bar.
        if bar % 2 == 1 {
            for (i, pad) in [8u8, 7, 6].iter().enumerate() {
                hit(&mut drums, b + 13.0 + i as f64, *pad, 110);
            }
        }
        if (6..8).contains(&bar) && bar % 2 == 0 {
            hit(&mut drums, b + 14.0, 7, 120);
        }
    }
    // Bar 11: a last boom (pad 14) and a cymbal.
    hit(&mut drums, 160.0, 14, 127);
    hit(&mut drums, 160.0, 11, 100);

    // ---- plucks ----
    let mut bass = pluck("Muted Bass", "Muted Bass", 2.4, (3, 10));
    let riff: [(f64, f64, u8); 7] = [
        (0.0, 2.0, 40),
        (3.0, 1.0, 40),
        (6.0, 2.0, 43),
        (10.0, 1.5, 38),
        (12.0, 2.0, 40),
        (14.5, 1.0, 47),
        (16.0, 1.0, 45),
    ];
    for rep in 0..4 {
        for &(at, len, n) in &riff[..6] {
            note(&mut bass, 32.0 + f64::from(rep) * 32.0 + at, len, n, 115);
        }
        for &(at, len, n) in &riff {
            note(
                &mut bass,
                32.0 + f64::from(rep) * 32.0 + 16.0 + at.min(15.0),
                len,
                n.saturating_sub(2),
                105,
            );
        }
    }
    let mut koto = pluck("Koto", "Koto", 1.2, (5, 6));
    let line: [(f64, u8); 9] = [
        (0.0, 64),
        (2.0, 67),
        (3.0, 69),
        (6.0, 71),
        (8.0, 74),
        (11.0, 71),
        (14.0, 69),
        (16.0, 67),
        (20.0, 64),
    ];
    for &(at, n) in &line {
        note(&mut koto, 64.0 + at, 2.0, n, 115);
    }
    for &(at, n) in &line[..5] {
        note(&mut koto, 88.0 + at / 2.0, 1.0, n + 12, 100);
    }
    let mut marimba = pluck("Marimba", "Marimba", 1.3, (7, 8));
    let riff2: [(f64, u8, u8); 8] = [
        (0.0, 64, 120),
        (2.0, 71, 90),
        (3.0, 67, 100),
        (6.0, 76, 120),
        (8.0, 74, 90),
        (10.0, 71, 110),
        (11.0, 67, 80),
        (14.0, 64, 100),
    ];
    for rep in 0..2 {
        for &(at, n, v) in &riff2 {
            note(&mut marimba, 96.0 + f64::from(rep) * 16.0 + at, 1.0, n, v);
        }
    }
    let mut bell = pluck("Bell", "Bell", 1.0, (9, 10));
    for &(at, n) in &[
        (0.0, 76u8),
        (6.0, 79),
        (8.0, 83),
        (16.0, 81),
        (22.0, 79),
        (24.0, 76),
    ] {
        note(&mut bell, 128.0 + at, 4.0, n, 110);
    }
    let mut kalimba = pluck("Kalimba", "Kalimba", 1.2, (9, 10));
    for k in 0..16 {
        note(
            &mut kalimba,
            128.0 + f64::from(k) * 2.0,
            1.5,
            [64u8, 67, 71, 74, 76, 74, 71, 67][k as usize % 8],
            90 + (k % 3) as u8 * 15,
        );
    }
    let mut glass = pluck("Glass", "Glass", 1.0, (11, 11));
    for &n in &[64u8, 71, 76, 79, 83] {
        note(&mut glass, 160.0, 12.0, n, 110);
    }

    let mut parts = vec![drums, bass, koto, marimba, bell, kalimba, glass];

    // ---- render ----
    let mut buf = Vec::new();
    for p in parts.iter_mut() {
        p.events.sort_by_key(|e| e.0);
        p.left = vec![0.0; total];
        p.right = vec![0.0; total];
        let mut ev_i = 0;
        let mut kit_i = 0;
        for at in (0..total).step_by(BLOCK) {
            while kit_i < p.kits.len() && p.kits[kit_i].0 < (at + BLOCK) as u64 {
                // Kits load between blocks; hits ring on with the old kit.
                let k = &Drums::presets()[p.kits[kit_i].1];
                for q in mpfree_synth::drum_params::PARAMS
                    .iter()
                    .filter(|q| !q.readonly)
                {
                    p.node.set_param(q.id, q.default);
                }
                for &(id, v) in k.values {
                    p.node.set_param(id, v);
                }
                kit_i += 1;
            }
            buf.clear();
            while ev_i < p.events.len() && p.events[ev_i].0 < (at + BLOCK) as u64 {
                let (f, e) = p.events[ev_i];
                buf.push(Timed::new((f.max(at as u64) - at as u64) as u32, e));
                ev_i += 1;
            }
            p.node.process(
                &Ctx::new(RATE, at as u64),
                &buf,
                &mut p.left[at..at + BLOCK],
                &mut p.right[at..at + BLOCK],
            );
        }
    }

    // ---- mix and check ----
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    for p in &parts {
        for i in 0..total {
            l[i] += p.left[i] * p.gain;
            r[i] += p.right[i] * p.gain;
        }
    }
    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    let trim = 10f32.powf(-1.0 / 20.0) / peak.max(1e-9);
    for x in l.iter_mut().chain(r.iter_mut()) {
        *x *= trim;
    }
    println!(
        "mix peak before trim {:.2} dBFS, trimmed by {:.1} dB",
        db(peak),
        db(trim)
    );
    println!("timeline (bar: what is new):");
    for (bar, what) in [
        (1, "Init Kit"),
        (3, "Grit Kit, Muted Bass"),
        (5, "Boom Kit, Koto"),
        (7, "Electro Kit, Marimba"),
        (9, "Grit Kit, Bell, Kalimba"),
        (11, "Glass chord, last boom, ring-out"),
    ] {
        println!(
            "  bar {bar:>2} ({:5.2} s): {what}",
            step(f64::from(bar - 1) * 16.0) as f64 / f64::from(RATE)
        );
    }
    let bar_frames = step(16.0) as usize;
    let mut ok = true;
    println!("level per bar (dBFS RMS after the trim):");
    for p in &parts {
        let mono: Vec<f32> = p
            .left
            .iter()
            .zip(&p.right)
            .map(|(a, b)| 0.5 * (a + b) * p.gain * trim)
            .collect();
        let mut line = format!("  {:<11}", p.name);
        for bar in 1..=BARS {
            let s = (bar as usize - 1) * bar_frames;
            let level = db(rms(&mono[s..(s + bar_frames).min(total)]));
            line.push_str(&format!(" {:>6.1}", level));
            let inside = bar >= p.bars.0 && bar <= p.bars.1;
            if inside && level < -40.0 {
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
    let last = l[total - 4800..]
        .iter()
        .chain(&r[total - 4800..])
        .fold(0.0f32, |m, x| m.max(x.abs()));
    println!("last 100 ms peak {:.1} dBFS", db(last));
    if last > 1e-3 {
        println!("WARNING: the end is not quiet");
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
        "wrote {} ({:.1} s)",
        out.display(),
        total as f64 / f64::from(RATE)
    );
    if !ok {
        eprintln!("self-check FAILED");
        std::process::exit(1);
    }
    println!("self-check ok");
}

fn db(x: f32) -> f32 {
    20.0 * x.max(1e-12).log10()
}
fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
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
