//! Renders a demo of the Cloud reverb's five characters in a heavy 90 BPM
//! beat and writes `~/mpfree-work/cloud-demo.wav` (or the path given;
//! `light` after it renders every reverb at its Light quality).
//!
//! Each character sits on its own send (Mix 100 %, a factory preset), and
//! the stems are sent to it for two bars at a time:
//! - bars 1-2: the beat dry, for reference
//! - bars 3-4: **Small room** on the kick, snare and hats
//! - bars 5-6: **Dark plate** on the snare and the chord stabs
//! - bars 7-8: **Big hall** on sparse chord stabs and the snare
//! - bars 9-10: **Endless cloud** on a sustained chord, frozen at bar 10
//!   while the beat plays on dry over it, released at bar 11
//! - bar 11: a last hit into **Echo chamber**, ringing out to the bar line
//!   at the end of bar 12.
//!
//! The bass runs through a fuzz Drive and the mix is driven hot into
//! Master safety, as in the F1 demo. It prints the level of every bar, what
//! Master safety caught and how many frames a Raw master would have
//! clipped. Nothing fails because of level; the self-check fails only if
//! the converter would clip, a sample is non-finite or the end is not
//! quiet.

use std::f32::consts::TAU;

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::{gain_to_db, Cloud, Drive, MasterSafety, CLOUD_PRESETS};

const RATE: u32 = 48_000;
const BPM: f64 = 90.0;
const BLOCK: usize = 128;
const BARS: usize = 12;

fn step(s: f64) -> usize { (s * 60.0 / BPM * f64::from(RATE) / 4.0).round() as usize }
fn bar(b: usize) -> usize { step((b as f64 - 1.0) * 16.0) }
fn db(x: f32) -> f32 { gain_to_db(x) }
fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32 }
fn peak(x: &[f32]) -> f32 { x.iter().fold(0.0f32, |m, v| m.max(v.abs())) }
fn hz(midi: i32) -> f32 { 440.0 * 2f32.powf((midi - 69) as f32 / 12.0) }

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

/// A saw from its harmonics below 12 kHz.
fn saw(ph: f32, hz: f32) -> f32 {
    let n = ((12_000.0 / hz) as usize).clamp(1, 24);
    (1..=n).map(|k| (TAU * ph * k as f32).sin() / k as f32).sum::<f32>() * 0.55
}

/// One mono-ish stem in stereo.
type Stem = (Vec<f32>, Vec<f32>);

/// Adds `len` frames of `f` at `at`, panned, with a 3 ms fade at the end.
fn add(s: &mut Stem, at: usize, len: usize, pan: f32, f: &mut dyn FnMut(usize) -> f32) {
    let fade = (0.003 * RATE as f32) as usize;
    for k in 0..len {
        let i = at + k;
        if i >= s.0.len() { break; }
        let x = f(k) * ((len - k) as f32 / fade as f32).min(1.0);
        s.0[i] += x * (1.0 - pan);
        s.1[i] += x * (1.0 + pan);
    }
}

const KICK: usize = 0;
const SNARE: usize = 1;
const HATS: usize = 2;
const BASS: usize = 3;
const CHORDS: usize = 4;

fn stems(total: usize) -> [Stem; 5] {
    let mut st: [Stem; 5] = std::array::from_fn(|_| (vec![0.0; total], vec![0.0; total]));
    let mut noise = Noise(5);
    let rate = RATE as f32;
    for b in 1..11 {
        let s0 = (b as f64 - 1.0) * 16.0;
        for &s in &[0.0, 7.0, 10.0] {
            let mut ph = 0.0f32;
            add(&mut st[KICK], step(s0 + s), (0.4 * rate) as usize, 0.0, &mut |k| {
                let t = k as f32 / rate;
                ph += (46.0 + 130.0 * (-t * 30.0).exp()) / rate;
                ((ph * TAU).sin() * 3.0).tanh() * (-t * 6.0).exp() * 0.55
            });
        }
        for &s in &[4.0, 12.0] {
            let mut ph = 0.0f32;
            add(&mut st[SNARE], step(s0 + s), (0.22 * rate) as usize, 0.05, &mut |k| {
                let t = k as f32 / rate;
                ph += 185.0 / rate;
                noise.next() * (-t * 18.0).exp() * 0.4 + (ph * TAU).sin() * (-t * 30.0).exp() * 0.3
            });
        }
        for h in 0..16 {
            let s = h as f64 + if h % 2 == 1 { 0.2 } else { 0.0 };
            let vel = if h % 4 == 2 { 0.12 } else if h % 2 == 0 { 0.08 } else { 0.05 };
            let mut prev = 0.0f32;
            add(&mut st[HATS], step(s0 + s), (0.04 * rate) as usize, -0.35, &mut |k| {
                let n = noise.next();
                let x = n - prev;
                prev = n;
                x * (-(k as f32) / rate * 110.0).exp() * vel
            });
        }
        let riff: [(f64, f64, i32); 5] = [(0.0, 3.0, 28), (3.0, 1.0, 28), (6.0, 2.0, 31), (8.0, 3.5, 28), (12.0, 3.0, 26)];
        for &(s, len, n) in &riff {
            let f = hz(n);
            let mut ph = 0.0f32;
            add(&mut st[BASS], step(s0 + s), step(len), 0.0, &mut |k| {
                ph = (ph + f / rate).fract();
                saw(ph, f) * (k as f32 / (0.004 * rate)).min(1.0) * 0.45
            });
        }
        // E minor, C, G, D: stabs, sparse stabs in the hall, a held chord
        // for the cloud.
        let chord: [i32; 3] = match (b - 1) % 4 { 0 => [52, 55, 59], 1 => [48, 52, 55], 2 => [50, 55, 59], _ => [50, 54, 57] };
        let hits: &[(f64, f64)] = match b { 7 | 8 => &[(0.0, 1.0), (8.0, 1.0)], 9 => &[(0.0, 15.0)], 10 => &[], _ => &[(2.0, 1.0), (6.0, 1.0), (10.0, 1.0), (14.0, 1.0)] };
        for &(s, len) in hits {
            for (v, &n) in chord.iter().enumerate() {
                let f = hz(n + 12);
                let mut ph = 0.0f32;
                let pan = (v as f32 - 1.0) * 0.4;
                add(&mut st[CHORDS], step(s0 + s), step(len), pan, &mut |k| {
                    ph = (ph + f / rate).fract();
                    let t = k as f32 / rate;
                    let env = (t / 0.005).min(1.0) * if len > 4.0 { 0.9 } else { (-t * 6.0).exp() };
                    (saw(ph, f) * 0.6 + (TAU * ph).sin() * 0.4) * env * 0.15
                });
            }
        }
    }
    // The last hit at bar 11: kick, low E and an E minor chord.
    let at = bar(11);
    let mut ph = 0.0f32;
    add(&mut st[KICK], at, (0.6 * rate) as usize, 0.0, &mut |k| {
        let t = k as f32 / rate;
        ph += (44.0 + 130.0 * (-t * 25.0).exp()) / rate;
        ((ph * TAU).sin() * 3.0).tanh() * (-t * 5.0).exp() * 0.6
    });
    let mut ph = 0.0f32;
    add(&mut st[BASS], at, rate as usize, 0.0, &mut |k| {
        ph = (ph + 41.2 / rate).fract();
        saw(ph, 41.2) * (-(k as f32) / rate * 3.0).exp() * 0.45
    });
    for (v, &n) in [64, 67, 71, 76].iter().enumerate() {
        let f = hz(n);
        let mut ph = 0.0f32;
        add(&mut st[CHORDS], at, step(2.0), (v as f32 - 1.5) * 0.3, &mut |k| {
            ph = (ph + f / rate).fract();
            let t = k as f32 / rate;
            (saw(ph, f) * 0.6 + (TAU * ph).sin() * 0.4) * (t / 0.005).min(1.0) * (-t * 5.0).exp() * 0.15
        });
    }
    st
}

fn main() {
    let light = std::env::args().nth(2).is_some_and(|a| a == "light");
    let out = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::Path::new(&home).join("mpfree-work").join("cloud-demo.wav")
    });
    let total = bar(BARS + 1);
    let st = stems(total);

    // One reverb per character, wet only, from the factory presets.
    let names = ["Small room", "Dark plate", "Big hall", "Endless cloud", "Echo chamber"];
    let mut verbs: Vec<Cloud> = names
        .iter()
        .map(|name| {
            let p = CLOUD_PRESETS.iter().find(|p| p.name == *name).expect("preset");
            let mut c = Cloud::new();
            c.prepare(RATE, BLOCK);
            for &(id, v) in p.values { c.set_param(id, v); }
            c.set_param(ids::cloud::MIX, 100.0);
            c.set_param(ids::cloud::QUALITY, if light { 1.0 } else { 0.0 });
            c
        })
        .collect();
    // The cloud decays in 2.5 s once released, so the end rings out in time.
    verbs[3].set_param(ids::cloud::DECAY, 2500.0);
    for v in &mut verbs { v.reset(); }
    // Send levels per stem (kick, snare, hats, bass, chords) for each reverb,
    // and the bars they are open.
    let sends: [([f32; 5], usize, usize); 5] = [
        ([0.35, 0.9, 0.6, 0.0, 0.0], 3, 5),
        ([0.0, 1.0, 0.0, 0.0, 0.7], 5, 7),
        ([0.0, 0.5, 0.0, 0.0, 1.0], 7, 9),
        ([0.0, 0.0, 0.0, 0.0, 1.2], 9, 10),
        ([0.5, 0.0, 0.0, 0.3, 1.0], 11, 12),
    ];
    let returns = [0.9f32, 1.0, 1.0, 1.1, 0.9];
    let freeze_at = bar(10);
    let release_at = bar(11);

    let mut fuzz = Drive::new();
    fuzz.prepare(RATE, BLOCK);
    for (id, v) in [(ids::drive::MODE, 4.0), (ids::drive::DRIVE, 22.0), (ids::drive::TONE, 4500.0), (ids::drive::MIX, 80.0)] { fuzz.set_param(id, v); }
    fuzz.reset();
    let mut safety = MasterSafety::new();
    safety.prepare(RATE, BLOCK);
    let mut raw = MasterSafety::new();
    raw.prepare(RATE, BLOCK);
    raw.set_param(ids::master::MODE, 2.0);
    raw.reset();

    let ramp = (0.01 * RATE as f32) as usize;
    let open = |i: usize, from: usize, to: usize| -> f32 {
        let (a, b) = (bar(from), bar(to));
        if i < a || i >= b + ramp { return 0.0; }
        ((i - a) as f32 / ramp as f32).min(1.0) * if i >= b { 1.0 - (i - b) as f32 / ramp as f32 } else { 1.0 }
    };
    let (mut l, mut r) = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut pre = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut tails = [0.0f32; 5];
    let mut at = 0;
    while at < total {
        let len = BLOCK.min(total - at);
        let ctx = Ctx { bpm: BPM, playing: true, ..Ctx::new(RATE, at as u64) };
        let range = at..at + len;
        let (mut bl, mut br) = (st[BASS].0[range.clone()].to_vec(), st[BASS].1[range.clone()].to_vec());
        fuzz.process(&ctx, &[], &mut bl, &mut br);
        let dry = |k: usize, ch: usize| -> f32 {
            let i = at + k;
            let s = |n: usize| if ch == 0 { st[n].0[i] } else { st[n].1[i] };
            1.3 * s(KICK) + s(SNARE) + s(HATS) + 1.2 * if ch == 0 { bl[k] } else { br[k] } + s(CHORDS)
        };
        let (mut ml, mut mr): (Vec<f32>, Vec<f32>) = ((0..len).map(|k| dry(k, 0)).collect(), (0..len).map(|k| dry(k, 1)).collect());
        for (v, verb) in verbs.iter_mut().enumerate() {
            let (gains, from, to) = sends[v];
            let mut sl = vec![0.0f32; len];
            let mut sr = vec![0.0f32; len];
            for k in 0..len {
                let i = at + k;
                let g = open(i, from, to);
                if g == 0.0 { continue; }
                for n in 0..5 {
                    let (x, y) = if n == BASS { (bl[k], br[k]) } else { (st[n].0[i], st[n].1[i]) };
                    sl[k] += g * gains[n] * x;
                    sr[k] += g * gains[n] * y;
                }
            }
            let mut events = Vec::new();
            if v == 3 {
                for (f, value) in [(freeze_at, 1.0), (release_at, 0.0)] {
                    if range.contains(&f) { events.push(Timed::new((f - at) as u32, NodeEvent::Param { id: ids::cloud::FREEZE, value })); }
                }
            }
            verb.process(&ctx, &events, &mut sl, &mut sr);
            for k in 0..len {
                ml[k] += returns[v] * sl[k];
                mr[k] += returns[v] * sr[k];
                if at + k >= total - 4800 { tails[v] = tails[v].max(sl[k].abs()).max(sr[k].abs()); }
            }
        }
        // Hot on purpose: Master safety catches it.
        for x in ml.iter_mut().chain(mr.iter_mut()) { *x *= 0.95; }
        pre.0[range.clone()].copy_from_slice(&ml);
        pre.1[range.clone()].copy_from_slice(&mr);
        let (mut rl, mut rr) = (ml.clone(), mr.clone());
        raw.process(&ctx, &[], &mut rl, &mut rr);
        safety.process(&ctx, &[], &mut ml, &mut mr);
        l[range.clone()].copy_from_slice(&ml);
        r[range].copy_from_slice(&mr);
        at += len;
    }

    let sections = ["dry", "dry", "small room", "small room", "dark plate", "dark plate", "big hall", "big hall", "endless cloud", "cloud frozen", "echo chamber", "ring out"];
    println!("Cloud reverb demo: {BARS} bars at {BPM} BPM, {RATE} Hz, {BLOCK}-frame blocks\n");
    println!("  bar  {:<15} {:>9} {:>9} {:>9}", "reverb", "in peak", "out peak", "out RMS");
    for (b, section) in sections.iter().enumerate() {
        let (r0, r1) = (bar(b + 1), bar(b + 2).min(total));
        let pin = peak(&pre.0[r0..r1]).max(peak(&pre.1[r0..r1]));
        println!("  {:>3}  {:<15} {:>6.1} dB {:>6.1} dB {:>6.1} dB", b + 1, section, db(pin), db(peak(&l[r0..r1]).max(peak(&r[r0..r1]))), db(rms(&l[r0..r1])));
    }
    let s = safety.stats();
    let rs = raw.stats();
    let last = peak(&l[total - 4800..]).max(peak(&r[total - 4800..]));
    println!("\nMaster safety (Safe limiter, ceiling -0.3 dBFS): caught {} overs, most limiting {:.1} dB, peak out {:.2} dBFS, clipped frames {}", s.overs, s.max_reduction_db, s.peak_out_db, s.clipped);
    println!("A Raw master on the same mix would have clipped {} frames (peak {:+.1} dBFS)", rs.clipped, rs.peak_out_db);
    println!("last 100 ms peak {:.1} dBFS; reverb returns there: {}", db(last), names.iter().zip(tails).map(|(n, t)| format!("{n} {:.0} dB", db(t))).collect::<Vec<_>>().join(", "));
    let mut ok = true;
    if s.clipped > 0 || peak(&l).max(peak(&r)) > 1.0 { println!("FAIL: the converter would clip"); ok = false; }
    if !l.iter().chain(&r).all(|v| v.is_finite()) { println!("FAIL: non-finite output"); ok = false; }
    if last > 1e-3 { println!("FAIL: the end is not quiet"); ok = false; }

    let mut pcm = Vec::with_capacity(total * 4);
    for i in 0..total {
        for x in [l[i], r[i]] { pcm.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes()); }
    }
    if let Some(dir) = out.parent() { std::fs::create_dir_all(dir).expect("create output dir"); }
    std::fs::write(&out, wav(&pcm)).expect("write wav");
    println!("wrote {} ({} frames, {:.1} s), PCM hash {:016x}", out.display(), total, total as f32 / RATE as f32, fnv(&pcm));
    println!("(the PCM hash repeats on the same computer; other computers' maths libraries can differ in the last bit)");
    if !ok { eprintln!("self-check FAILED"); std::process::exit(1); }
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
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}
