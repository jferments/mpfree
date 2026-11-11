//! Renders a demo of the Wavetable synth in a grimy 90 BPM beat and writes
//! `~/mpfree-work/wavetable-demo.wav` (or the path given).
//!
//! Each sound enters on its own so it can be heard:
//! - bars 1-2: drums and **Growl Bass** (Resonance table, its position
//!   wobbling on a 1/16 LFO, bend warp, legato slides)
//! - bars 3-4: plus **Morph Pad** chords (Vowels and Drift tables drifting
//!   on a slow LFO, unison 3)
//! - bars 5-6: pad out; **Vowel Lead**: one long note per bar with pad
//!   pressure rising and falling, which scans the vowel table (A-E-I-O-U)
//! - bars 7-8: lead out; **Sync Sweep** stabs (the mod envelope sweeps the
//!   sync table) and a **pad sample turned into a table**: a sung "ah-oh"
//!   tone made here, cut into 16 cycles by `import::table_from_sample`,
//!   scanned by the LFO
//! - bar 9: drums out; a final Morph Pad chord rings out over the bass's
//!   last note, then the releases end.
//!
//! The synths run through `mpfree_node::Node::process` in 128-frame blocks,
//! exactly as on the audio thread. It prints a timeline, each stem's level
//! per bar, checks each part sounds only where it should, and trims the
//! master to -1 dBFS peak (no clipping).

use std::sync::Arc;

use mpfree_node::{Ctx, Node, NodeEvent, Timed};
use mpfree_synth::params::Preset;
use mpfree_synth::wavetable::import::table_from_sample;
use mpfree_synth::wavetable::params::{self as wt, osc_id};
use mpfree_synth::Wavetable;

const RATE: u32 = 48_000;
const BPM: f64 = 90.0;
const BLOCK: usize = 128;

fn frames_per_beat() -> f64 {
    60.0 / BPM * f64::from(RATE)
}
fn step(s: f64) -> u64 {
    (s * frames_per_beat() / 4.0).round() as u64
}

struct Part {
    name: &'static str,
    synth: Wavetable,
    events: Vec<(u64, NodeEvent)>,
    gain: f32,
    pan: f32,
    bars: (u32, u32),
    left: Vec<f32>,
    right: Vec<f32>,
}

fn preset(name: &str) -> &'static Preset {
    Wavetable::presets()
        .iter()
        .find(|p| p.name == name)
        .unwrap_or_else(|| panic!("no preset {name}"))
}

fn part(name: &'static str, preset_name: &str, gain: f32, pan: f32, bars: (u32, u32)) -> Part {
    let mut synth = Wavetable::new(RATE);
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

/// A sung "ah" sliding to "oh" at 196 Hz with vibrato, as a pad sample
/// would hold it: three formants on a pulse train, two seconds.
fn sung_sample() -> Vec<f32> {
    let n = RATE as usize * 2;
    let mut out = Vec::with_capacity(n);
    let mut ph = 0.0f64;
    // Two-pole resonators per formant.
    let mut st = [[0.0f64; 2]; 3];
    for i in 0..n {
        let t = i as f64 / f64::from(RATE);
        let k = (t / 2.0).min(1.0);
        let f = [
            800.0 + (450.0 - 800.0) * k,
            1150.0 + (800.0 - 1150.0) * k,
            2900.0 + (2830.0 - 2900.0) * k,
        ];
        let hz = 196.0 * (1.0 + 0.004 * (std::f64::consts::TAU * 5.0 * t).sin());
        ph += hz / f64::from(RATE);
        let pulse = if ph >= 1.0 {
            ph -= 1.0;
            1.0
        } else {
            0.0
        };
        let mut y = 0.0;
        for (q, s) in st.iter_mut().enumerate() {
            let bw = [80.0, 100.0, 150.0][q];
            let r = (-std::f64::consts::PI * bw / f64::from(RATE)).exp();
            let c = 2.0 * r * (std::f64::consts::TAU * f[q] / f64::from(RATE)).cos();
            let v = pulse + c * s[0] - r * r * s[1];
            s[1] = s[0];
            s[0] = v;
            y += v * [1.0, 0.5, 0.25][q];
        }
        out.push(y as f32);
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    out.iter().map(|v| v / peak * 0.8).collect()
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            std::path::Path::new(&home)
                .join("mpfree-work")
                .join("wavetable-demo.wav")
        });
    let bars = 9.0;
    let tail = 3.0;
    let total = (step(bars * 16.0) as f64 + tail * f64::from(RATE)) as usize;
    let total = total.div_ceil(BLOCK) * BLOCK;

    // ---- the parts ----
    let mut bass = part("Growl Bass", "Growl Bass", 1.5, 0.0, (1, 9));
    let riff: [(f64, f64, u8); 8] = [
        (0.0, 3.0, 29),
        (3.0, 1.0, 29),
        (4.0, 4.0, 32),
        (10.0, 2.0, 27),
        (12.0, 4.0, 29),
        (16.0, 6.0, 29),
        (22.0, 2.0, 36),
        (24.0, 7.5, 34),
    ];
    for rep in 0..4 {
        for &(at, len, n) in &riff {
            note(&mut bass, rep as f64 * 32.0 + at, len, n, 115);
        }
    }
    note(&mut bass, 128.0, 12.0, 29, 120);

    let chords: [[u8; 4]; 4] = [
        [53, 56, 60, 63],
        [49, 53, 56, 60],
        [51, 55, 58, 62],
        [48, 51, 55, 58],
    ];
    let mut pad = part("Morph Pad", "Morph Pad", 2.2, 0.2, (3, 4));
    for (i, c) in chords.iter().enumerate() {
        for &n in c {
            note(&mut pad, 32.0 + i as f64 * 8.0, 7.5, n, 100);
        }
    }
    let mut last = part("Final chord", "Morph Pad", 1.4, 0.0, (9, 9));
    for &n in &[53u8, 60, 63, 67, 72] {
        note(&mut last, 128.0, 10.0, n, 105);
    }

    // The lead: pressure rises over each bar and falls, scanning the vowels.
    let mut lead = part("Vowel Lead", "Vowel Lead", 1.2, -0.2, (5, 6));
    lead.synth.set_param(wt::press_depth(0), 72.0);
    for (b, n) in [(0.0, 72u8), (16.0, 75)] {
        note(&mut lead, 64.0 + b, 15.0, n, 110);
        for k in 0..=60 {
            let x = k as f64 / 60.0;
            let p = (127.0 * (std::f64::consts::PI * x).sin()) as u8;
            lead.events.push((
                step(64.0 + b + x * 14.0),
                NodeEvent::Pressure { note: n, value: p },
            ));
        }
    }

    let mut stabs = part("Sync Sweep", "Sync Sweep", 1.0, 0.25, (7, 8));
    for (bar, c) in chords.iter().take(2).enumerate() {
        for &s in &[0.0, 3.0, 6.0, 10.0, 14.0] {
            for &n in &c[..3] {
                note(&mut stabs, 96.0 + bar as f64 * 16.0 + s, 1.5, n + 12, 115);
            }
        }
    }

    // A pad sample becomes a table: 16 cycles of the sung tone.
    let sample = sung_sample();
    let table =
        table_from_sample("Sung ah-oh", &sample, RATE, 16).expect("the sung sample is pitched");
    println!(
        "pad sample -> table: {} frames, {:.2} MB",
        table.frames(),
        table.bytes() as f64 / 1e6
    );
    let mut voice = part("Sung table", "Init", 1.6, -0.25, (7, 8));
    let _ = voice.synth.set_user_table(0, Arc::new(table));
    for (id, v) in [
        (wt::TABLE, wt::USER_TABLE),
        (wt::LFO_POS_A, 100.0),
        (wt::LFO_RATE, 0.5),
        (wt::LFO_SHAPE, 1.0),
        (wt::AMP_A, 60.0),
        (wt::AMP_R, 500.0),
        (wt::UNISON, 2.0),
        (wt::UNISON_DETUNE, 8.0),
        (osc_id(1, wt::OSC_LEVEL), 0.0),
        (wt::LFO_PITCH, 0.15),
        (wt::LFO_RETRIG, 1.0),
    ] {
        voice.synth.set_param(id, v);
    }
    let tune: [(f64, f64, u8); 6] = [
        (96.0, 6.0, 67),
        (102.0, 2.0, 68),
        (104.0, 8.0, 70),
        (112.0, 6.0, 72),
        (118.0, 2.0, 70),
        (120.0, 7.5, 67),
    ];
    for &(at, len, n) in &tune {
        note(&mut voice, at, len, n, 105);
    }

    let mut parts = vec![bass, pad, lead, stabs, voice, last];

    // ---- timeline ----
    println!(
        "Wavetable demo, {BPM} BPM, {} bars + {tail} s tail, {} frames at {RATE} Hz",
        bars, total
    );
    for p in &parts {
        let ons: Vec<f64> = p
            .events
            .iter()
            .filter_map(|&(f, e)| {
                matches!(e, NodeEvent::NoteOn { .. })
                    .then_some(f as f64 / frames_per_beat() / 4.0 + 1.0)
            })
            .collect();
        let first = ons.iter().cloned().fold(f64::MAX, f64::min);
        let last = ons.iter().cloned().fold(0.0, f64::max);
        println!(
            "  {:<11} {:>3} notes, 16th steps {:.0} to {:.0} (bars {}-{})",
            p.name,
            ons.len(),
            first,
            last,
            p.bars.0,
            p.bars.1
        );
    }

    // ---- render ----
    // Where the lead reads in its table, block by block.
    let mut lead_pos: Vec<f32> = Vec::new();
    for p in &mut parts {
        p.events.sort_by_key(|e| e.0);
        p.left = vec![0.0; total];
        p.right = vec![0.0; total];
        let mut buf = Vec::new();
        let mut at = 0usize;
        while at < total {
            buf.clear();
            for &(f, ev) in &p.events {
                if f >= at as u64 && f < (at + BLOCK) as u64 {
                    buf.push(Timed::new((f - at as u64) as u32, ev));
                }
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
            if p.name == "Vowel Lead" {
                lead_pos.push(p.synth.live_positions()[0]);
            }
            at += BLOCK;
        }
    }
    let (dl, dr) = drums(total, 8);

    // ---- mix ----
    let mut l = vec![0.0f32; total];
    let mut r = vec![0.0f32; total];
    for p in &parts {
        let (gl, gr) = ((1.0 - p.pan).sqrt() * p.gain, (1.0 + p.pan).sqrt() * p.gain);
        for i in 0..total {
            l[i] += p.left[i] * gl;
            r[i] += p.right[i] * gr;
        }
    }
    for i in 0..total {
        l[i] += dl[i];
        r[i] += dr[i];
    }
    let peak = l.iter().chain(&r).fold(0.0f32, |m, x| m.max(x.abs()));
    let trim = 10f32.powf(-1.0 / 20.0) / peak.max(1e-9);
    for x in l.iter_mut().chain(r.iter_mut()) {
        *x *= trim;
    }
    println!(
        "master trim {:.1} dB (mix peak {:.2} before)",
        20.0 * trim.log10(),
        peak
    );

    // ---- self-check ----
    let bar_frames = step(16.0) as usize;
    let db = |x: f32| 20.0 * x.max(1e-9).log10();
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt();
    println!("stem levels:");
    let mut ok = true;
    for p in &parts {
        let mono: Vec<f32> = p
            .left
            .iter()
            .zip(&p.right)
            .map(|(a, b)| 0.5 * (a + b) * p.gain * trim)
            .collect();
        let mut line = format!("  {:<11}", p.name);
        for bar in 1..=9u32 {
            let s = (bar as usize - 1) * bar_frames;
            let level = db(rms(&mono[s..(s + bar_frames).min(total)]));
            let inside = bar >= p.bars.0 && bar <= p.bars.1;
            line.push_str(&format!(" {:>6.1}", level));
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
        let mut line = format!("  {:<11}", "Drums");
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
    // The lead's pressure scan: where it reads in the vowel table (0 = A,
    // 1 = U) at each beat of bars 5-6. It rises and falls twice, with the
    // pressure.
    {
        let beat = frames_per_beat();
        let mut line = String::from("  Vowel Lead table position per beat, bars 5-6:");
        for b in 0..8 {
            let at = (step(64.0) as f64 + (b as f64 + 0.5) * beat) as usize / BLOCK;
            line.push_str(&format!(" {:.2}", lead_pos[at]));
        }
        println!("{line}");
    }
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
    let left_on = parts.iter().map(|p| p.synth.active_voices()).sum::<usize>();
    if left_on > 0 {
        println!("WARNING: {left_on} voices still on at the end");
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
