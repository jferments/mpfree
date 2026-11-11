//! Renders a demo of the F1 effects in a heavy 90 BPM beat and writes
//! `~/mpfree-work/fx-demo.wav` (or the path given).
//!
//! Every effect gets its own two bars, switched in and out through
//! [`Chain`]s exactly as on the audio thread (128-frame blocks, sample-timed
//! parameter events, click-free bypass):
//! - bars 1-2: the beat dry, for reference
//! - bars 3-4: **Drive** (Fuzz, HQ 4x) on the bass into a resonant 24 dB
//!   **Multi filter** whose cutoff sweeps up by automation
//! - bars 5-6: **Crush** on the drums (6 bits, 5 kHz); sustained chords
//!   ducked by the kick through the **Compressor**'s sidechain
//! - bars 7-8: chord stabs into a ping-pong **Delay** (1/8 dotted, dirty
//!   feedback)
//! - bars 9-10: the delay is bypassed and keeps ringing (spillover) while
//!   the **DJ filter** on the master sweeps high-pass, then low-pass
//! - bar 11: a last hit echoes out to the bar line at the end of bar 12.
//!
//! The mix is driven hot on purpose into **Master safety** (Safe limiter):
//! it prints the overs it caught, the most limiting, the peak, and how many
//! frames a Raw master would have clipped. Nothing here fails because of
//! level; the self-check fails only if the converter would clip, a step is
//! non-finite or the end is not quiet.

use std::f32::consts::TAU;

use mpfree_fx::ids;
use mpfree_fx::mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_fx::{gain_to_db, Chain, Compressor, Crush, Delay, DjFilter, Drive, MasterSafety, MultiFilter};

const RATE: u32 = 48_000;
const BPM: f64 = 90.0;
const BLOCK: usize = 128;
const BARS: usize = 12;

fn bar_frames() -> usize { (4.0 * 60.0 / BPM * f64::from(RATE)).round() as usize }
/// The frame of a 16th step from the start (bar 1 = steps 0..16).
fn step(s: f64) -> usize { (s * 60.0 / BPM * f64::from(RATE) / 4.0).round() as usize }
fn bar(b: usize) -> usize { step((b as f64 - 1.0) * 16.0) }

fn db(x: f32) -> f32 { gain_to_db(x) }
fn rms(x: &[f32]) -> f32 { (x.iter().map(|v| (v * v) as f64).sum::<f64>() / x.len().max(1) as f64).sqrt() as f32 }
fn peak(x: &[f32]) -> f32 { x.iter().fold(0.0f32, |m, v| m.max(v.abs())) }

struct Noise(u64);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 40) as f32 / (1u64 << 23) as f32) - 1.0
    }
}

/// Adds `len` frames of `f` at `at`, panned, with a 3 ms fade at the end.
fn add(l: &mut [f32], r: &mut [f32], at: usize, len: usize, pan: f32, f: &mut dyn FnMut(usize) -> f32) {
    let fade = (0.003 * RATE as f32) as usize;
    for k in 0..len {
        let i = at + k;
        if i >= l.len() { break; }
        let x = f(k) * ((len - k) as f32 / fade as f32).min(1.0);
        l[i] += x * (1.0 - pan);
        r[i] += x * (1.0 + pan);
    }
}

/// Band-limited enough: a saw from its first `n` harmonics (all below 12 kHz).
fn saw(ph: f32, hz: f32) -> f32 {
    let n = ((12_000.0 / hz) as usize).clamp(1, 24);
    (1..=n).map(|k| (TAU * ph * k as f32).sin() / k as f32).sum::<f32>() * 0.55
}

fn hz(midi: i32) -> f32 { 440.0 * 2f32.powf((midi - 69) as f32 / 12.0) }

struct Stems { kick: (Vec<f32>, Vec<f32>), drums: (Vec<f32>, Vec<f32>), bass: (Vec<f32>, Vec<f32>), chords: (Vec<f32>, Vec<f32>) }

fn stems(total: usize) -> Stems {
    let z = || (vec![0.0f32; total], vec![0.0f32; total]);
    let (mut kick, mut drums, mut bass, mut chords) = (z(), z(), z(), z());
    let mut noise = Noise(11);
    let rate = RATE as f32;
    let last = 11; // the bar of the final hit
    for b in 1..last {
        let s0 = (b as f64 - 1.0) * 16.0;
        // Kick: a pitch-dropping sine, driven.
        for &s in &[0.0, 6.0, 10.0] {
            let mut ph = 0.0f32;
            add(&mut kick.0, &mut kick.1, step(s0 + s), (0.4 * rate) as usize, 0.0, &mut |k| {
                let t = k as f32 / rate;
                ph += (48.0 + 120.0 * (-t * 30.0).exp()) / rate;
                ((ph * TAU).sin() * 2.5).tanh() * (-t * 6.5).exp() * 0.55
            });
        }
        // Snare: noise and a tone.
        for &s in &[4.0, 12.0] {
            let mut ph = 0.0f32;
            add(&mut drums.0, &mut drums.1, step(s0 + s), (0.25 * rate) as usize, 0.05, &mut |k| {
                let t = k as f32 / rate;
                ph += 190.0 / rate;
                noise.next() * (-t * 16.0).exp() * 0.35 + (ph * TAU).sin() * (-t * 28.0).exp() * 0.3
            });
        }
        // Hats: swung 16ths of high-passed noise.
        for h in 0..16 {
            let s = h as f64 + if h % 2 == 1 { 0.2 } else { 0.0 };
            let vel = if h % 4 == 2 { 0.12 } else if h % 2 == 0 { 0.08 } else { 0.05 };
            let mut prev = 0.0f32;
            add(&mut drums.0, &mut drums.1, step(s0 + s), (0.04 * rate) as usize, -0.35, &mut |k| {
                let n = noise.next();
                let x = n - prev;
                prev = n;
                x * (-(k as f32) / rate * 110.0).exp() * vel
            });
        }
        // Bass: A minor riff, a clean saw (so the drive has work to do).
        let riff: [(f64, f64, i32); 6] = [(0.0, 2.5, 33), (3.0, 1.5, 33), (6.0, 2.0, 36), (8.0, 2.5, 33), (11.0, 1.5, 31), (14.0, 2.0, 28)];
        for &(s, len, n) in &riff {
            let f = hz(n);
            let mut ph = 0.0f32;
            let frames = step(len);
            add(&mut bass.0, &mut bass.1, step(s0 + s), frames, 0.0, &mut |k| {
                ph = (ph + f / rate).fract();
                let att = (k as f32 / (0.004 * rate)).min(1.0);
                saw(ph, f) * att * (0.75 + 0.25 * (-(k as f32) / rate * 8.0).exp()) * 0.45
            });
        }
        // Chords: Am F C G. Stabs, sustained pads in bars 5-6, sparse stabs
        // (room for echoes) in bars 7-8.
        let chord: [i32; 3] = match (b - 1) % 4 { 0 => [57, 60, 64], 1 => [53, 57, 60], 2 => [55, 60, 64], _ => [55, 59, 62] };
        let hits: &[(f64, f64)] = match b { 5 | 6 => &[(0.0, 15.5)], 7 | 8 => &[(0.0, 1.0), (7.0, 1.0)], _ => &[(2.0, 1.0), (6.0, 1.0), (10.0, 1.0), (14.0, 1.0)] };
        for &(s, len) in hits {
            for (v, &n) in chord.iter().enumerate() {
                let f = hz(n);
                let mut ph = 0.0f32;
                let frames = step(len);
                let pan = (v as f32 - 1.0) * 0.4;
                add(&mut chords.0, &mut chords.1, step(s0 + s), frames, pan, &mut |k| {
                    ph = (ph + f / rate).fract();
                    let t = k as f32 / rate;
                    let env = (t / 0.005).min(1.0) * if len > 4.0 { 0.8 } else { (-t * 5.0).exp() };
                    (saw(ph, f) * 0.6 + (TAU * ph).sin() * 0.4) * env * 0.16
                });
            }
        }
    }
    // The last hit: kick, a low A and an A minor chord that rings.
    let at = bar(last);
    let mut ph = 0.0f32;
    add(&mut kick.0, &mut kick.1, at, (0.6 * rate) as usize, 0.0, &mut |k| {
        let t = k as f32 / rate;
        ph += (45.0 + 120.0 * (-t * 25.0).exp()) / rate;
        ((ph * TAU).sin() * 2.5).tanh() * (-t * 5.0).exp() * 0.6
    });
    let mut ph = 0.0f32;
    add(&mut bass.0, &mut bass.1, at, rate as usize, 0.0, &mut |k| {
        ph = (ph + 55.0 / rate).fract();
        saw(ph, 55.0) * (-(k as f32) / rate * 3.0).exp() * 0.45
    });
    for (v, &n) in [57, 60, 64, 69].iter().enumerate() {
        let f = hz(n);
        let mut ph = 0.0f32;
        add(&mut chords.0, &mut chords.1, at, step(3.0), (v as f32 - 1.5) * 0.3, &mut |k| {
            ph = (ph + f / rate).fract();
            let t = k as f32 / rate;
            (saw(ph, f) * 0.6 + (TAU * ph).sin() * 0.4) * (t / 0.005).min(1.0) * (-t * 3.0).exp() * 0.16
        });
    }
    Stems { kick, drums, bass, chords }
}

/// A bypass switch or a parameter value at a frame.
enum Cue { On(usize, bool), Param(usize, u16, f32) }

fn main() {
    let out = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        std::path::Path::new(&home).join("mpfree-work").join("fx-demo.wav")
    });
    let total = bar(BARS + 1);
    let s = stems(total);

    // ---- chains ----
    let mut bass = Chain::new();
    bass.prepare(RATE, BLOCK);
    let drive = bass.push(Box::new(Drive::new()));
    let filt = bass.push(Box::new(MultiFilter::new()));
    use ids::drive as dr;
    for (id, v) in [(dr::MODE, 4.0), (dr::DRIVE, 26.0), (dr::BIAS, 35.0), (dr::TONE, 6000.0), (dr::QUALITY, 1.0)] { bass.set_param(drive, id, v); }
    use ids::filter as fl;
    for (id, v) in [(fl::TYPE, 0.0), (fl::SLOPE, 1.0), (fl::CUTOFF, 120.0), (fl::RESONANCE, 65.0)] { bass.set_param(filt, id, v); }

    let mut drums = Chain::new();
    drums.prepare(RATE, BLOCK);
    let crush = drums.push(Box::new(Crush::new()));
    let glue = drums.push(Box::new(Compressor::new()));
    use ids::crush as cr;
    for (id, v) in [(cr::BITS, 6.0), (cr::RATE, 5000.0), (cr::MIX, 85.0)] { drums.set_param(crush, id, v); }
    use ids::compressor as co;
    for (id, v) in [(co::THRESHOLD, -20.0), (co::RATIO, 3.0), (co::ATTACK, 8.0), (co::RELEASE, 120.0), (co::MAKEUP, 3.0)] { drums.set_param(glue, id, v); }

    let mut chords = Chain::new();
    chords.prepare(RATE, BLOCK);
    let duck = chords.push(Box::new(Compressor::new()));
    let echo = chords.push(Box::new(Delay::new()));
    for (id, v) in [(co::THRESHOLD, -36.0), (co::RATIO, 10.0), (co::ATTACK, 1.0), (co::RELEASE, 200.0), (co::KNEE, 3.0), (co::SIDECHAIN, 1.0)] { chords.set_param(duck, id, v); }
    use ids::delay as de;
    for (id, v) in [(de::SYNC, 7.0), (de::FEEDBACK, 65.0), (de::PING_PONG, 1.0), (de::TONE, 3500.0), (de::SATURATION, 60.0), (de::MIX, 45.0)] { chords.set_param(echo, id, v); }

    let mut master = Chain::new();
    master.prepare(RATE, BLOCK);
    let dj = master.push(Box::new(DjFilter::new()));
    master.set_param(dj, ids::dj_filter::RESONANCE, 45.0);
    let mut safety = MasterSafety::new();
    safety.prepare(RATE, BLOCK);
    let mut raw = MasterSafety::new();
    raw.prepare(RATE, BLOCK);
    raw.set_param(ids::master::MODE, 2.0);
    raw.reset();

    // Everything starts bypassed; the cues switch effects per section.
    for slot in [drive, filt] { bass.set_on_now(slot, false); }
    drums.set_on_now(crush, false);
    for slot in [duck, echo] { chords.set_on_now(slot, false); }
    // Cues per chain: (frame, cue).
    let mut cues: [Vec<(usize, Cue)>; 4] = Default::default();
    let [bass_cues, drum_cues, chord_cues, master_cues] = &mut cues;
    bass_cues.push((bar(3), Cue::On(drive, true)));
    bass_cues.push((bar(3), Cue::On(filt, true)));
    bass_cues.push((bar(5), Cue::On(filt, false)));
    bass_cues.push((bar(7), Cue::On(drive, false)));
    drum_cues.push((bar(5), Cue::On(crush, true)));
    drum_cues.push((bar(7), Cue::On(crush, false)));
    chord_cues.push((bar(5), Cue::On(duck, true)));
    chord_cues.push((bar(7), Cue::On(duck, false)));
    chord_cues.push((bar(7), Cue::On(echo, true)));
    chord_cues.push((bar(9), Cue::On(echo, false)));
    chord_cues.push((bar(10) + step(12.0), Cue::Param(echo, de::FEEDBACK, 45.0)));
    chord_cues.push((bar(10) + step(12.0), Cue::On(echo, true)));
    // Sweeps: a value every 64 frames, as sample-timed events.
    let sweep = |cues: &mut Vec<(usize, Cue)>, slot: usize, id: u16, from: usize, to: usize, f: &dyn Fn(f32) -> f32| {
        let mut i = from;
        while i < to {
            cues.push((i, Cue::Param(slot, id, f((i - from) as f32 / (to - from) as f32))));
            i += 64;
        }
    };
    sweep(bass_cues, filt, fl::CUTOFF, bar(3), bar(5), &|t| 120.0 * (5000.0f32 / 120.0).powf(t));
    sweep(master_cues, dj, ids::dj_filter::POSITION, bar(9), bar(10), &|t| 75.0 * (std::f32::consts::PI * t).sin());
    sweep(master_cues, dj, ids::dj_filter::POSITION, bar(10), bar(11), &|t| -85.0 * (std::f32::consts::PI * t).sin());
    master_cues.push((bar(11), Cue::Param(dj, ids::dj_filter::POSITION, 0.0)));
    for c in cues.iter_mut() { c.sort_by_key(|c| c.0); }

    // ---- render, block by block ----
    let gains = [1.2f32, 1.3, 1.0, 0.85]; // kick+drums, bass, chords, hot master
    let (mut l, mut r) = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut pre_master = (vec![0.0f32; total], vec![0.0f32; total]);
    let mut next = [0usize; 4];
    let mut limiting = [0.0f32; BARS + 1];
    let mut at = 0;
    while at < total {
        let len = BLOCK.min(total - at);
        let ctx = Ctx { bpm: BPM, playing: true, beats: at as f64 / (60.0 / BPM * f64::from(RATE)), ..Ctx::new(RATE, at as u64) };
        let mut chains = [&mut bass, &mut drums, &mut chords, &mut master];
        for (k, chain) in chains.iter_mut().enumerate() {
            while next[k] < cues[k].len() && cues[k][next[k]].0 < at + len {
                let (f, ref cue) = cues[k][next[k]];
                match *cue {
                    // Switches land on the block they fall in.
                    Cue::On(slot, on) => chain.set_on(slot, on),
                    Cue::Param(slot, id, v) => { chain.queue(slot, Timed::new(f.saturating_sub(at) as u32, NodeEvent::Param { id, value: v })); }
                }
                next[k] += 1;
            }
        }
        let range = at..at + len;
        let (mut dl, mut drr): (Vec<f32>, Vec<f32>) = (range.clone().map(|i| s.drums.0[i] + s.kick.0[i]).collect(), range.clone().map(|i| s.drums.1[i] + s.kick.1[i]).collect());
        drums.process(&ctx, &mut dl, &mut drr);
        let (mut bl, mut br) = (s.bass.0[range.clone()].to_vec(), s.bass.1[range.clone()].to_vec());
        bass.process(&ctx, &mut bl, &mut br);
        let (mut cl, mut cr) = (s.chords.0[range.clone()].to_vec(), s.chords.1[range.clone()].to_vec());
        let key = (&s.kick.0[range.clone()], &s.kick.1[range.clone()]);
        chords.process(&Ctx { sidechain: Some(key), ..ctx }, &mut cl, &mut cr);
        let mut ml: Vec<f32> = (0..len).map(|i| gains[3] * (gains[0] * dl[i] + gains[1] * bl[i] + gains[2] * cl[i])).collect();
        let mut mr: Vec<f32> = (0..len).map(|i| gains[3] * (gains[0] * drr[i] + gains[1] * br[i] + gains[2] * cr[i])).collect();
        master.process(&ctx, &mut ml, &mut mr);
        pre_master.0[range.clone()].copy_from_slice(&ml);
        pre_master.1[range.clone()].copy_from_slice(&mr);
        let (mut rl, mut rr) = (ml.clone(), mr.clone());
        raw.process(&ctx, &[], &mut rl, &mut rr);
        safety.process(&ctx, &[], &mut ml, &mut mr);
        l[range.clone()].copy_from_slice(&ml);
        r[range].copy_from_slice(&mr);
        let b = (at / bar_frames()).min(BARS);
        limiting[b] = limiting[b].min(safety.param(ids::master::REDUCTION));
        at += len;
    }

    // ---- report ----
    let sections = ["dry", "dry", "drive+filter sweep", "drive+filter sweep", "crush, ducking", "crush, ducking", "ping-pong delay", "ping-pong delay", "delay tail, DJ HP", "delay tail, DJ LP", "last hit echoes", "ring out"];
    println!("FX demo: {BARS} bars at {BPM} BPM, {RATE} Hz, chains in {BLOCK}-frame blocks\n");
    println!("  bar  {:<20} {:>9} {:>9} {:>9} {:>10}", "effects", "in peak", "out peak", "out RMS", "limiting");
    let mut ok = true;
    for b in 0..BARS {
        let r0 = b * bar_frames();
        let r1 = ((b + 1) * bar_frames()).min(total);
        let pin = peak(&pre_master.0[r0..r1]).max(peak(&pre_master.1[r0..r1]));
        let pout = peak(&l[r0..r1]).max(peak(&r[r0..r1]));
        println!("  {:>3}  {:<20} {:>6.1} dB {:>6.1} dB {:>6.1} dB {:>7.1} dB", b + 1, sections[b], db(pin), db(pout), db(rms(&l[r0..r1])), limiting[b]);
    }
    let st = safety.stats();
    let rs = raw.stats();
    let final_peak = peak(&l).max(peak(&r));
    let last = peak(&l[total - 4800..]).max(peak(&r[total - 4800..]));
    println!("\nMaster safety (Safe limiter, ceiling -0.3 dBFS): caught {} overs, most limiting {:.1} dB, peak out {:.2} dBFS, clipped frames {}", st.overs, st.max_reduction_db, st.peak_out_db, st.clipped);
    println!("A Raw master on the same mix would have clipped {} frames (peak {:+.1} dBFS)", rs.clipped, rs.peak_out_db);
    println!("Chain counters: NaN resets {}, dropped events {}", [&bass, &drums, &chords, &master].iter().map(|c| c.stats().nan_resets).sum::<u64>(), [&bass, &drums, &chords, &master].iter().map(|c| c.stats().dropped_events).sum::<u64>());
    println!("last 100 ms peak {:.1} dBFS", db(last));
    if st.clipped > 0 || final_peak > 1.0 { println!("FAIL: the converter would clip"); ok = false; }
    if !l.iter().chain(&r).all(|v| v.is_finite()) { println!("FAIL: non-finite output"); ok = false; }
    if last > 1e-3 { println!("FAIL: the end is not quiet"); ok = false; }

    // ---- write ----
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
