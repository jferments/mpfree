//! Renders a demo of the EQ, the modulation effects and Lo-fi in a heavy
//! 90 BPM beat and writes `~/mpfree-work/mod-demo.wav` (or the path given).
//!
//! Each effect is an insert that is always running; its Mix (or gain) is
//! automated with sample-timed events, so it fades in and out without a
//! click:
//! - bars 1-2: the beat dry, for reference
//! - bars 3-4: **Chorus** (three voices, full width) on held chords
//! - bars 5-6: **Flanger** on the drums and the chords, one sweep a bar,
//!   synced to the transport, 70 % feedback
//! - bars 7-8: **Phaser** (8 stages, one sweep every 2 bars, synced) on
//!   the fuzz bass and the chords
//! - bars 9-10: **Lo-fi** ("Dusty vinyl") on the master, its Amount rising
//!   from nothing to full: wow, crackle and hiss creep in
//! - bars 11-12: the **EQ** on the master builds up: a 24 dB low cut sweeps
//!   from 20 Hz to 700 Hz while a resonant bell rises at 2 kHz
//! - bar 13: the drop: the EQ snaps open and every effect plays at once
//! - bars 14-15: a last hit rings out through a Cloud reverb send.
//!
//! The mix runs hot into Master safety, as in the other demos. It prints
//! the level of every bar, what Master safety caught and how many frames a
//! Raw master would have clipped. Nothing fails because of level; the
//! self-check fails only if the converter would clip, a sample is
//! non-finite or the end is not quiet.

use std::f32::consts::TAU;

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::{gain_to_db, Chorus, Cloud, Drive, Eq, Flanger, LoFi, MasterSafety, Phaser, CLOUD_PRESETS, LOFI_PRESETS};

const RATE: u32 = 48_000;
const BPM: f64 = 90.0;
const BLOCK: usize = 128;
const BARS: usize = 15;

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

const DRUMS: usize = 0;
const BASS: usize = 1;
const CHORDS: usize = 2;

fn kick(st: &mut Stem, at: usize, len: f32, gain: f32) {
    let rate = RATE as f32;
    let mut ph = 0.0f32;
    add(st, at, (len * rate) as usize, 0.0, &mut |k| {
        let t = k as f32 / rate;
        ph += (46.0 + 130.0 * (-t * 30.0).exp()) / rate;
        ((ph * TAU).sin() * 3.0).tanh() * (-t * 6.0).exp() * gain
    });
}

fn chord(st: &mut Stem, at: usize, len: usize, notes: &[i32], decay: f32) {
    let rate = RATE as f32;
    for (v, &n) in notes.iter().enumerate() {
        let f = hz(n);
        let mut ph = v as f32 * 0.13;
        let pan = (v as f32 - (notes.len() - 1) as f32 / 2.0) * 0.25;
        add(st, at, len, pan, &mut |k| {
            ph = (ph + f / rate).fract();
            let t = k as f32 / rate;
            let env = (t / 0.03).min(1.0) * (-t * decay).exp();
            (saw(ph, f) * 0.55 + (TAU * ph).sin() * 0.45) * env * 0.11
        });
    }
}

fn stems(total: usize) -> [Stem; 3] {
    let mut st: [Stem; 3] = std::array::from_fn(|_| (vec![0.0; total], vec![0.0; total]));
    let mut noise = Noise(11);
    let rate = RATE as f32;
    for b in 1..14 {
        let s0 = (b as f64 - 1.0) * 16.0;
        for &s in &[0.0, 7.0, 10.0] { kick(&mut st[DRUMS], step(s0 + s), 0.4, 0.6); }
        for &s in &[4.0, 12.0] {
            let mut ph = 0.0f32;
            add(&mut st[DRUMS], step(s0 + s), (0.22 * rate) as usize, 0.05, &mut |k| {
                let t = k as f32 / rate;
                ph += 185.0 / rate;
                noise.next() * (-t * 18.0).exp() * 0.42 + (ph * TAU).sin() * (-t * 30.0).exp() * 0.3
            });
        }
        for h in 0..16 {
            let s = h as f64 + if h % 2 == 1 { 0.2 } else { 0.0 };
            let vel = if h % 4 == 2 { 0.2 } else if h % 2 == 0 { 0.14 } else { 0.08 };
            let mut prev = 0.0f32;
            add(&mut st[DRUMS], step(s0 + s), (0.05 * rate) as usize, -0.35, &mut |k| {
                let n = noise.next();
                let x = n - prev;
                prev = n;
                x * (-(k as f32) / rate * 90.0).exp() * vel
            });
        }
        let riff: [(f64, f64, i32); 6] = [(0.0, 2.5, 28), (3.0, 1.0, 28), (5.0, 1.0, 40), (6.0, 2.0, 31), (8.0, 3.5, 28), (12.0, 3.0, 26)];
        for &(s, len, n) in &riff {
            let f = hz(n);
            let mut ph = 0.0f32;
            add(&mut st[BASS], step(s0 + s), step(len), 0.0, &mut |k| {
                ph = (ph + f / rate).fract();
                saw(ph, f) * (k as f32 / (0.004 * rate)).min(1.0) * 0.42
            });
        }
        // E minor, C, G, D, held for the bar.
        let notes: [i32; 4] = match (b - 1) % 4 { 0 => [52, 59, 64, 67], 1 => [48, 55, 64, 67], 2 => [50, 55, 62, 71], _ => [50, 57, 62, 66] };
        chord(&mut st[CHORDS], step(s0), step(16.0), &notes, 0.25);
    }
    // The last hit at bar 14, into the reverb.
    let at = bar(14);
    kick(&mut st[DRUMS], at, 0.7, 0.7);
    let mut ph = 0.0f32;
    add(&mut st[BASS], at, rate as usize, 0.0, &mut |k| {
        ph = (ph + 41.2 / rate).fract();
        saw(ph, 41.2) * (-(k as f32) / rate * 3.0).exp() * 0.45
    });
    chord(&mut st[CHORDS], at, step(10.0), &[64, 67, 71, 76], 1.6);
    st
}

/// A node and the values it starts from.
type Setup<'a> = (&'a mut dyn Node, &'a [(u16, f32)]);

/// Parameter moves at absolute frames for one insert.
type Moves = Vec<(usize, u16, f32)>;

fn events_in(moves: &Moves, at: usize, len: usize, out: &mut Vec<Timed>) {
    out.clear();
    for &(f, id, v) in moves {
        if f >= at && f < at + len { out.push(Timed::new((f - at) as u32, NodeEvent::Param { id, value: v })); }
    }
}

fn main() {
    let out = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::Path::new(&home).join("mpfree-work").join("mod-demo.wav")
    });
    let total = bar(BARS + 1);
    let st = stems(total);

    let mut fuzz = Drive::new();
    let mut chorus = Chorus::new();
    let mut flanger = Flanger::new();
    let mut flanger_chords = Flanger::new();
    let mut phaser = Phaser::new();
    let mut phaser_chords = Phaser::new();
    let mut eq = Eq::new();
    let mut lofi = LoFi::new();
    let mut hall = Cloud::new();
    let mut safety = MasterSafety::new();
    let mut raw = MasterSafety::new();
    let flange: &[(u16, f32)] = &[(ids::flanger::SYNC, 5.0), (ids::flanger::DEPTH, 80.0), (ids::flanger::DELAY, 1.2), (ids::flanger::FEEDBACK, 70.0), (ids::flanger::MIX, 0.0)];
    let setups: [Setup; 11] = [
        (&mut fuzz, &[(ids::drive::MODE, 4.0), (ids::drive::DRIVE, 20.0), (ids::drive::TONE, 4000.0), (ids::drive::MIX, 80.0)]),
        (&mut chorus, &[(ids::chorus::VOICES, 3.0), (ids::chorus::DEPTH, 60.0), (ids::chorus::RATE, 0.5), (ids::chorus::MIX, 0.0)]),
        (&mut flanger, flange),
        (&mut flanger_chords, flange),
        (&mut phaser, &[(ids::phaser::SYNC, 6.0), (ids::phaser::STAGES, 3.0), (ids::phaser::FEEDBACK, 60.0), (ids::phaser::CENTER, 600.0), (ids::phaser::DEPTH, 85.0), (ids::phaser::MIX, 0.0)]),
        (&mut phaser_chords, &[(ids::phaser::SYNC, 6.0), (ids::phaser::STAGES, 2.0), (ids::phaser::FEEDBACK, 40.0), (ids::phaser::CENTER, 900.0), (ids::phaser::MIX, 0.0)]),
        (&mut eq, &[(ids::eq::LOW_CUT, 20.0), (ids::eq::freq(2), 2000.0), (ids::eq::q(2), 6.0)]),
        (&mut lofi, LOFI_PRESETS.iter().find(|p| p.name == "Dusty vinyl").expect("preset").values),
        (&mut hall, CLOUD_PRESETS.iter().find(|p| p.name == "Big hall").expect("preset").values),
        (&mut safety, &[]),
        (&mut raw, &[(ids::master::MODE, 2.0)]),
    ];
    for (node, values) in setups {
        node.prepare(RATE, BLOCK);
        for &(id, v) in values { node.set_param(id, v); }
        node.reset();
    }
    hall.set_param(ids::cloud::MIX, 100.0);
    lofi.set_param(ids::lofi::AMOUNT, 0.0);
    lofi.set_param(ids::lofi::MIX, 0.0);
    lofi.reset();

    // Mix automation: each effect fades in for its section and again for
    // the drop.
    let on_off = |id: u16, level: f32, spans: &[(usize, usize)]| -> Moves {
        spans.iter().flat_map(|&(a, b)| [(bar(a), id, level), (bar(b), id, 0.0)]).collect()
    };
    let chorus_moves = on_off(ids::chorus::MIX, 55.0, &[(3, 5), (13, 15)]);
    let flanger_moves = on_off(ids::flanger::MIX, 50.0, &[(5, 7), (13, 15)]);
    let phaser_moves = on_off(ids::phaser::MIX, 60.0, &[(7, 9), (13, 15)]);
    let phaser_chord_moves = on_off(ids::phaser::MIX, 50.0, &[(7, 9)]);
    // Lo-fi creeps in: one Amount move a block, then off at bar 11.
    let mut lofi_moves: Moves = vec![(bar(9), ids::lofi::MIX, 100.0), (bar(11), ids::lofi::MIX, 0.0)];
    let (a, b) = (bar(9), bar(11));
    for f in (a..b).step_by(BLOCK) { lofi_moves.push((f, ids::lofi::AMOUNT, 100.0 * ((f - a) as f32 / (b - a) as f32).sqrt())); }
    // The build: one move a block, glided by the EQ's own smoothing.
    let mut eq_moves: Moves = vec![(bar(11), ids::eq::LOW_SLOPE, 2.0)];
    let (a, b) = (bar(11), bar(13));
    for f in (a..b).step_by(BLOCK) {
        let t = (f - a) as f32 / (b - a) as f32;
        eq_moves.push((f, ids::eq::LOW_CUT, 20.0 * 35f32.powf(t * t)));
        eq_moves.push((f, ids::eq::gain(2), 12.0 * t));
    }
    eq_moves.push((b, ids::eq::LOW_SLOPE, 0.0));
    eq_moves.push((b, ids::eq::gain(2), 0.0));

    let (mut l, mut r) = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut pre = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut ev = Vec::new();
    let beats_per_frame = BPM / 60.0 / f64::from(RATE);
    let mut at = 0;
    while at < total {
        let len = BLOCK.min(total - at);
        let ctx = Ctx { bpm: BPM, beats: at as f64 * beats_per_frame, playing: true, ..Ctx::new(RATE, at as u64) };
        let range = at..at + len;
        let slice = |s: &Stem| (s.0[range.clone()].to_vec(), s.1[range.clone()].to_vec());
        let (mut dl, mut dr) = slice(&st[DRUMS]);
        let (mut bl, mut br) = slice(&st[BASS]);
        let (mut cl, mut cr) = slice(&st[CHORDS]);
        fuzz.process(&ctx, &[], &mut bl, &mut br);
        events_in(&phaser_moves, at, len, &mut ev);
        phaser.process(&ctx, &ev, &mut bl, &mut br);
        events_in(&flanger_moves, at, len, &mut ev);
        flanger.process(&ctx, &ev, &mut dl, &mut dr);
        events_in(&chorus_moves, at, len, &mut ev);
        chorus.process(&ctx, &ev, &mut cl, &mut cr);
        events_in(&phaser_chord_moves, at, len, &mut ev);
        phaser_chords.process(&ctx, &ev, &mut cl, &mut cr);
        events_in(&flanger_moves, at, len, &mut ev);
        flanger_chords.process(&ctx, &ev, &mut cl, &mut cr);
        let mut ml: Vec<f32> = (0..len).map(|k| 1.25 * dl[k] + 1.2 * bl[k] + cl[k]).collect();
        let mut mr: Vec<f32> = (0..len).map(|k| 1.25 * dr[k] + 1.2 * br[k] + cr[k]).collect();
        // The reverb send opens for the last hit only.
        let open = |i: usize| if i >= bar(14) { 0.8 } else { 0.0 };
        let mut sl: Vec<f32> = (0..len).map(|k| open(at + k) * (0.5 * dl[k] + cl[k])).collect();
        let mut sr: Vec<f32> = (0..len).map(|k| open(at + k) * (0.5 * dr[k] + cr[k])).collect();
        hall.process(&ctx, &[], &mut sl, &mut sr);
        for k in 0..len {
            ml[k] += sl[k];
            mr[k] += sr[k];
        }
        events_in(&lofi_moves, at, len, &mut ev);
        lofi.process(&ctx, &ev, &mut ml, &mut mr);
        events_in(&eq_moves, at, len, &mut ev);
        eq.process(&ctx, &ev, &mut ml, &mut mr);
        pre.0[range.clone()].copy_from_slice(&ml);
        pre.1[range.clone()].copy_from_slice(&mr);
        let (mut rl, mut rr) = (ml.clone(), mr.clone());
        raw.process(&ctx, &[], &mut rl, &mut rr);
        safety.process(&ctx, &[], &mut ml, &mut mr);
        l[range.clone()].copy_from_slice(&ml);
        r[range].copy_from_slice(&mr);
        at += len;
    }

    let sections = ["dry", "dry", "chorus", "chorus", "flanger", "flanger", "phaser", "phaser", "lo-fi rising", "lo-fi full", "eq build", "eq build", "drop, all", "last hit", "ring out"];
    println!("Mod FX demo: {BARS} bars at {BPM} BPM, {RATE} Hz, {BLOCK}-frame blocks\n");
    println!("  bar  {:<15} {:>9} {:>9} {:>9}", "effect", "in peak", "out peak", "out RMS");
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
    println!("last 100 ms peak {:.1} dBFS", db(last));
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
