//! Renders a before-and-after demo of pad pressure (poly aftertouch) and
//! writes `~/mpfree-work/pressure-demo.wav` (or the path given).
//!
//! The same 4-bar phrase plays twice over a grimy 90 BPM beat, with the same
//! notes and the same pad pressure both times:
//! - bars 1-4, **before**: pressure Off on every synth, so the sounds stand
//!   still.
//! - bars 5-8, **after**: pressure on, through the shaper and each preset's
//!   slots:
//!   - bars 5-6: a **Brass** (Poly) chord held for two bars. Only its top
//!     note is pressed: it swells up 9 dB and opens 1.5 octaves, twice,
//!     while the other notes rest. The pad spikes to full as it's hit; the
//!     30 ms hold-off ignores the spike.
//!   - bars 5-8: an **Acid** (Poly) line. Pressure opens its filter (up to 2
//!     octaves) and adds resonance as the phrase builds.
//!   - bar 7: a **Brass Section** (FM) chord, every note pressed: a 9 dB
//!     swell with 12 steps of brightness.
//!   - bar 8: a **Dust Pad** (Poly) chord with its top note pressed: vibrato
//!     and a 6 dB swell on that note alone.
//! - bar 9: a last Brass chord swells and rings out.
//!
//! Every synth runs through `mpfree_node::Node::process` in 128-frame
//! blocks, as on the audio thread; the drums are synthesised here. It prints
//! a timeline and measures how much each part moves before and after (level
//! swing, brightness). Where only one note is pressed, it measures that
//! note alone (rendered again on its own). It checks the after pass moves
//! clearly more, and trims the master to -1 dBFS peak (no clipping).

use mpfree_node::{Node, NodeEvent, Timed};
use mpfree_synth::{fm_params as fp, params as pp, Fm, Poly};

const RATE: u32 = 48_000;
const BPM: f64 = 90.0;
const BLOCK: usize = 128;
/// Pressure messages arrive every 10 ms while a pad is held.
const PRESS_EVERY: u64 = 480;

fn frames_per_beat() -> f64 { 60.0 / BPM * f64::from(RATE) }
/// The frame of a 16th step from the start (bar 1 = steps 0..16).
fn step(s: f64) -> u64 { (s * frames_per_beat() / 4.0).round() as u64 }

struct Part {
    name: &'static str,
    make: fn(&str) -> Box<dyn Node>,
    synth: Box<dyn Node>,
    /// The one pressed note, measured on its own (none: the whole part).
    probe: Option<u8>,
    /// The pressure mode parameter, to switch it off for the before pass.
    mode_id: u16,
    events: Vec<(u64, NodeEvent)>,
    gain: f32,
    pan: f32,
    /// Steps where the part holds notes in the before pass (the after pass
    /// is 64 steps later), for the measurements.
    held: (f64, f64),
    left: Vec<f32>,
    right: Vec<f32>,
}

fn make_poly(name: &str) -> Box<dyn Node> {
    let mut s = Poly::new(RATE);
    s.load_preset(Poly::presets().iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no Poly preset {name}")));
    Box::new(s)
}

fn make_fm(name: &str) -> Box<dyn Node> {
    let mut s = Fm::new(RATE);
    s.load_preset(&Fm::presets().into_iter().find(|p| p.name == name).unwrap_or_else(|| panic!("no FM preset {name}")));
    Box::new(s)
}

fn poly(name: &'static str, gain: f32, pan: f32, held: (f64, f64), probe: Option<u8>) -> Part {
    Part { name, make: make_poly, synth: make_poly(name), probe, mode_id: pp::PRESS_MODE, events: Vec::new(), gain, pan, held, left: Vec::new(), right: Vec::new() }
}

fn fm(name: &'static str, gain: f32, pan: f32, held: (f64, f64), probe: Option<u8>) -> Part {
    Part { name, make: make_fm, synth: make_fm(name), probe, mode_id: fp::PRESS_MODE, events: Vec::new(), gain, pan, held, left: Vec::new(), right: Vec::new() }
}

/// Renders `events` on `synth` in 128-frame blocks, as the audio thread does.
fn run(synth: &mut dyn Node, events: &[(u64, NodeEvent)], total: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![0.0; total], vec![0.0; total]);
    let mut buf: Vec<Timed> = Vec::with_capacity(256);
    let mut e = 0usize;
    for at in (0..total).step_by(BLOCK) {
        buf.clear();
        while e < events.len() && events[e].0 < (at + BLOCK) as u64 {
            buf.push(Timed::new((events[e].0 - at as u64) as u32, events[e].1));
            e += 1;
        }
        let ctx = mpfree_node::Ctx { rate: RATE, frame: at as u64, bpm: BPM, beats: at as f64 / frames_per_beat(), playing: true, sidechain: None };
        synth.process(&ctx, &buf, &mut l[at..at + BLOCK], &mut r[at..at + BLOCK]);
    }
    (l, r)
}

/// A note from step `at` lasting `len` steps.
fn note(p: &mut Part, at: f64, len: f64, n: u8, vel: u8) {
    p.events.push((step(at), NodeEvent::NoteOn { note: n, velocity: vel }));
    p.events.push((step(at + len), NodeEvent::NoteOff { note: n }));
}

/// Pressure on note `n` from step `from` to `to`, every 10 ms, following
/// `curve(t)` (t from 0 to 1 across the span, value 0 to 1). The first 10 ms
/// spike to full, as a pad does when it's struck.
fn press(p: &mut Part, from: f64, to: f64, n: u8, curve: impl Fn(f64) -> f64) {
    let (a, b) = (step(from), step(to));
    p.events.push((a, NodeEvent::Pressure { note: n, value: 127 }));
    let mut f = a + PRESS_EVERY;
    while f < b {
        let t = (f - a) as f64 / (b - a) as f64;
        let v = (curve(t).clamp(0.0, 1.0) * 127.0).round() as u8;
        p.events.push((f, NodeEvent::Pressure { note: n, value: v }));
        f += PRESS_EVERY;
    }
}

/// A piecewise-linear curve through `(t, value)` points.
fn shape(pts: &'static [(f64, f64)]) -> impl Fn(f64) -> f64 {
    move |t| {
        for w in pts.windows(2) {
            if t <= w[1].0 {
                let k = ((t - w[0].0) / (w[1].0 - w[0].0)).clamp(0.0, 1.0);
                return w[0].1 + (w[1].1 - w[0].1) * k;
            }
        }
        pts.last().map_or(0.0, |p| p.1)
    }
}

/// Two swells on one held note, lifting before the note-off.
const SWELLS: &[(f64, f64)] = &[(0.0, 0.0), (0.08, 0.05), (0.25, 1.0), (0.42, 0.15), (0.68, 1.0), (0.84, 1.0), (0.97, 0.0), (1.0, 0.0)];
/// One long swell across two bars.
const SWELL: &[(f64, f64)] = &[(0.0, 0.0), (0.1, 0.0), (0.55, 1.0), (0.85, 1.0), (0.97, 0.0), (1.0, 0.0)];

fn main() {
    let out = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::Path::new(&home).join("mpfree-work").join("pressure-demo.wav")
    });
    let bars = 9.0;
    let tail = 3.0;
    let total = (step(bars * 16.0) as f64 + tail * f64::from(RATE)) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;

    // ---- the phrase, written once for steps 0..64 and played twice ----
    let mut brass = poly("Brass", 1.3, -0.2, (0.0, 31.0), Some(63));
    let mut acid = poly("Acid", 1.3, 0.0, (0.0, 63.0), None);
    let mut fm_brass = fm("Brass Section", 1.5, 0.2, (32.0, 47.0), None);
    let mut pad = poly("Dust Pad", 1.8, 0.3, (48.0, 63.0), Some(82));
    for pass in 0..2 {
        let o = pass as f64 * 64.0;
        // Brass: C minor, top note pressed (two swells).
        for &n in &[48u8, 55, 60, 63] {
            note(&mut brass, o, 31.0, n, 110);
        }
        press(&mut brass, o, o + 31.0, 63, shape(SWELLS));
        // Acid: a 16th line; pressure builds across the phrase.
        let riff: [(f64, f64, u8); 10] = [(0.0, 1.0, 36), (2.0, 1.0, 36), (3.0, 1.0, 48), (4.0, 2.0, 39), (6.0, 1.0, 36),
                                          (8.0, 1.0, 43), (9.0, 1.0, 36), (11.0, 1.0, 46), (12.0, 2.0, 48), (14.0, 1.5, 34)];
        let build = shape(&[(0.0, 0.0), (0.12, 0.05), (0.45, 1.0), (0.7, 0.75), (0.82, 1.0), (1.0, 0.1)]);
        for bar in 0..4 {
            for &(at, len, n) in &riff {
                let s = o + bar as f64 * 16.0 + at;
                note(&mut acid, s, len, n, 112);
                let (a, b) = ((s - o) / 64.0, (s + len - o) / 64.0);
                press(&mut acid, s, s + len, n, |t| build(a + (b - a) * t));
            }
        }
        // FM brass: an E flat major chord for a bar, every note pressed, one
        // long swell.
        for &n in &[51u8, 55, 58, 63] {
            note(&mut fm_brass, o + 32.0, 15.5, n, 115);
            press(&mut fm_brass, o + 32.0, o + 47.5, n, shape(SWELL));
        }
        // Pad: a bar, its top note (an octave above the rest) pressed after
        // the slow attack. Its release rings into the next bar.
        for &n in &[63u8, 67, 70, 82] {
            note(&mut pad, o + 48.0, 15.0, n, 100);
        }
        press(&mut pad, o + 48.0, o + 63.0, 82, shape(&[(0.0, 0.0), (0.25, 0.0), (0.4, 1.0), (0.9, 1.0), (1.0, 0.7)]));
    }
    // Bar 9: a last Brass chord, every note pressed, rings out.
    for &n in &[48u8, 55, 60, 63, 67] {
        note(&mut brass, 128.0, 14.0, n, 110);
        press(&mut brass, 128.0, 142.0, n, shape(&[(0.0, 0.0), (0.1, 0.0), (0.6, 1.0), (1.0, 1.0)]));
    }
    let mut parts = vec![brass, acid, fm_brass, pad];
    // Pressure Off for the before pass, on from bar 5.
    for p in &mut parts {
        p.events.push((0, NodeEvent::Param { id: p.mode_id, value: 2.0 }));
        p.events.push((step(64.0), NodeEvent::Param { id: p.mode_id, value: 0.0 }));
    }

    // ---- render every part through the node interface ----
    let started = std::time::Instant::now();
    for p in &mut parts {
        p.events.sort_by_key(|e| e.0);
        (p.left, p.right) = run(&mut *p.synth, &p.events, total);
        assert!(p.left.iter().chain(&p.right).all(|x| x.is_finite()), "{}: non-finite output", p.name);
    }
    let render_secs = started.elapsed().as_secs_f64();

    // ---- drums and mix ----
    let (dl, dr) = drums(total, 8);
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

    // ---- measure: how much each part moves, before and after ----
    let db = |x: f32| 20.0 * x.max(1e-9).log10();
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
    // Brightness: the share of energy in the sample-to-sample change.
    let bright = |x: &[f32]| x.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>() / x.iter().map(|v| v * v).sum::<f32>().max(1e-12);
    // What pressure does to the level: the same notes play in both passes,
    // so the difference between them, 100 ms at a time, is pressure's doing
    // alone (envelopes and LFOs cancel out). Its range is the swell.
    let swell = |a: &[f32], b: &[f32]| {
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for (ca, cb) in a.chunks(4_800).zip(b.chunks(4_800)).filter(|(c, _)| c.len() == 4_800) {
            let (la, lb) = (db(rms(ca)), db(rms(cb)));
            // Gaps between notes say nothing.
            if la > -60.0 && lb > -60.0 {
                lo = lo.min(lb - la);
                hi = hi.max(lb - la);
            }
        }
        if hi >= lo { hi - lo } else { 0.0 }
    };
    let secs = total as f64 / f64::from(RATE);
    println!("Pressure demo: {BPM} BPM, 9 bars + {tail} s tail, {secs:.1} s, master trim {:+.1} dB", db(trim));
    println!("rendered {} parts in {render_secs:.2} s on this computer", parts.len());
    println!("timeline:");
    println!("  bars 1-4  BEFORE (pressure off): Brass chord (bars 1-2), Acid line, FM Brass Section (bar 3), Dust Pad (bar 4)");
    println!("  bars 5-8  AFTER (pressure on, same notes and pressure): Brass top note swells twice (5-6), Acid opens,");
    println!("            FM brass swells and brightens (7), the pad's top note gets vibrato and swells (8)");
    println!("  bar  9    a last Brass chord swells under pressure and rings out");
    println!("  {:<24} {:>12} {:>10} {:>10}", "part (A before, B after)", "swell B-A", "bright A", "bright B");
    let mut ok = true;
    for p in &parts {
        // The pressed note alone, where only one is pressed.
        let (l, r) = match p.probe {
            Some(n) => {
                let only: Vec<(u64, NodeEvent)> = p.events.iter().copied().filter(|(_, e)| match *e {
                    NodeEvent::NoteOn { note, .. } | NodeEvent::NoteOff { note } | NodeEvent::Pressure { note, .. } => note == n,
                    _ => true,
                }).collect();
                run(&mut *(p.make)(p.name), &only, total)
            }
            None => (p.left.clone(), p.right.clone()),
        };
        let mono: Vec<f32> = l.iter().zip(&r).map(|(a, b)| 0.5 * (a + b)).collect();
        let span = |o: f64| (step(o + p.held.0) as usize + 19_200)..(step(o + p.held.1) as usize);
        let (a, b) = (&mono[span(0.0)], &mono[span(64.0)]);
        let (sw, ba, bb) = (swell(a, b), bright(a), bright(b));
        let label = match p.probe {
            Some(n) => format!("{} (note {n} alone)", p.name),
            None => p.name.to_string(),
        };
        println!("  {label:<24} {sw:>9.1} dB {ba:>10.4} {bb:>10.4}");
        // Pressure must clearly move the after pass: a swell of 4 dB or
        // more, or 40 % more brightness.
        if !(sw >= 4.0 || bb > ba * 1.4) {
            println!("  ! {} does not move more with pressure", p.name);
            ok = false;
        }
    }
    let final_peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    let last = l[total - 4800..].iter().chain(&r[total - 4800..]).fold(0.0f32, |m, x| m.max(x.abs()));
    println!("peak {:.2} dBFS, last 100 ms peak {:.1} dBFS", db(final_peak), db(last));
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
    println!("wrote {} ({} frames), PCM hash {:016x}", out.display(), total, fnv(&pcm));
    println!("(the PCM hash repeats on the same computer; other computers' maths libraries can differ in the last bit)");
    if !ok {
        eprintln!("self-check FAILED");
        std::process::exit(1);
    }
    println!("self-check ok");
}

/// Heavy boom-bap drums: a driven kick, a crushed snare, swung hats, vinyl
/// dust, `bars` bars from the start, and one last kick on bar 9.
fn drums(total: usize, bars: u32) -> (Vec<f32>, Vec<f32>) {
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    let mut seed = 11u64;
    let mut noise = move || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    };
    let rate = RATE as f32;
    let add = |l: &mut Vec<f32>, r: &mut Vec<f32>, at: u64, len: usize, pan: f32, f: &mut dyn FnMut(usize) -> f32| {
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
    let kick = |l: &mut Vec<f32>, r: &mut Vec<f32>, at: u64| {
        let mut ph = 0.0f32;
        add(l, r, at, (0.45 * rate) as usize, 0.0, &mut |k| {
            let t = k as f32 / rate;
            ph += (42.0 + 120.0 * (-t * 26.0).exp()) / rate;
            ((ph * std::f32::consts::TAU).sin() * 2.6).tanh() * (-t * 6.5).exp() * 0.5
        });
    };
    for bar in 0..bars {
        let b = bar as f64 * 16.0;
        for &s in &[0.0, 7.0, 10.0] {
            kick(&mut l, &mut r, step(b + s));
        }
        for &s in &[4.0, 12.0] {
            let mut ph = 0.0f32;
            let mut held = 0.0f32;
            add(&mut l, &mut r, step(b + s), (0.3 * rate) as usize, 0.05, &mut |k| {
                let t = k as f32 / rate;
                ph += 180.0 / rate;
                if k % 4 == 0 {
                    held = noise();
                }
                let x = held * (-t * 13.0).exp() * 0.8 + (ph * std::f32::consts::TAU).sin() * (-t * 30.0).exp() * 0.5;
                ((x * 8.0).round() / 8.0) * 0.3
            });
        }
        for h in 0..8 {
            let s = h as f64 * 2.0 + if h % 2 == 1 { 0.33 } else { 0.0 };
            let mut prev = 0.0f32;
            let vel = if h % 2 == 0 { 0.1 } else { 0.06 };
            add(&mut l, &mut r, step(b + s), (0.05 * rate) as usize, -0.3, &mut |k| {
                let t = k as f32 / rate;
                let n = noise();
                let hp = n - prev;
                prev = n;
                hp * (-t * 90.0).exp() * vel
            });
        }
    }
    kick(&mut l, &mut r, step(f64::from(bars) * 16.0));
    // Vinyl dust: sparse clicks over the beat.
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
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}
