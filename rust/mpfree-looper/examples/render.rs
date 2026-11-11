//! WORKSTATION ONLY: a looper session through the transport and the real
//! looper engine, written to a WAV file, so looping can be heard without
//! the MPC. It never touches any device; it only writes one WAV file.
//!
//! ```text
//! cargo run --release -p mpfree-looper --example render -- [--period N] [--out FILE]
//! ```
//!
//! A player plays into the line input at 96 BPM, and what they play reaches
//! the looper 640 frames late (about 14.5 ms, the output buffer plus the
//! input buffer), which the looper makes up for. One bar of count-in clicks,
//! then a two-bar loop builds up:
//!
//! - Bars 1-2: a distorted saw bass riff, recorded as the first pass.
//! - Bars 3-4: overdub crushed drums (kick, snare, hats).
//! - Bars 7-8: overdub a saturated pulse lead.
//! - Bar 9, beat 2: Undo, and the lead fades out. Bar 11: Redo, it is back.
//! - Bars 13-14: overdub distorted power-chord stabs on the off-beats.
//! - Bar 17: stop; the loop fades out under a final live hit.
//!
//! The player is heard live (direct monitoring) while they play; the loop
//! plays everything back. The distortion is deliberate, in the sounds; the
//! master is plain gain with headroom, and the tool reports any clipped
//! samples. It also checks that the first pass lines up with what was
//! played, to the frame, despite the input latency.

use mpfree_looper::{Command as L, Looper, Phase, FADE};
use mpfree_transport::{Block, Command as T, Transport};

const RATE: u32 = 44_100;
const BPM: f64 = 96.0;
const BAR_TICKS: f64 = 3_840.0;
/// Frames from the line input's jack to the looper (simulated).
const LATENCY: u64 = 640;
/// The transport starts here, a bar before bar 1 (the count-in).
const PLAY: u64 = 2_205;
/// Bars after bar 1 starts, then the tail.
const BARS: f64 = 16.0;
const TAIL: f64 = 3.0;
const MASTER: f32 = 0.62;

fn main() {
    let mut period = 128u32;
    let mut out: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--period" => period = args.next().and_then(|v| v.parse().ok()).filter(|p| (1..=8192).contains(p)).unwrap_or_else(|| fail("--period takes 1 to 8192")),
            "--out" => out = Some(args.next().unwrap_or_else(|| fail("--out takes a file name"))),
            "-h" | "--help" => {
                println!("render [--period N] [--out FILE]   (default FILE: ~/mpfree-work/looper-demo.wav)");
                return;
            }
            other => fail(&format!("unknown option {other}")),
        }
    }
    let out = out.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        format!("{home}/mpfree-work/looper-demo.wav")
    });

    let mut t = Transport::new(RATE, BPM).unwrap_or_else(|e| fail(&format!("transport: {e:?}")));
    let mut looper = Looper::new(RATE, 60);
    let mut block = Block::new();
    t.command(0, T::Locate(-(BAR_TICKS as i64)));
    t.command(PLAY, T::Play);
    t.command(frame_at(BARS * BAR_TICKS), T::Stop);
    looper.command(L::SetBars(Some(2)));
    looper.command(L::SetLatency(LATENCY as u32));
    looper.command(L::Record);
    println!("looper: armed, 2 bars, latency {LATENCY} frames");

    // The presses, at their song positions (a 1/16 early to punch in).
    let presses: [(f64, L, &str); 9] = [
        (2.0 * BAR_TICKS - 240.0, L::Overdub, "overdub drums"),
        (4.0 * BAR_TICKS - 60.0, L::Overdub, "punch out"),
        (6.0 * BAR_TICKS - 240.0, L::Overdub, "overdub lead"),
        (8.0 * BAR_TICKS - 60.0, L::Overdub, "punch out"),
        (8.0 * BAR_TICKS + 960.0, L::Undo, "undo (the lead fades out)"),
        (10.0 * BAR_TICKS, L::Redo, "redo (the lead comes back)"),
        (12.0 * BAR_TICKS - 240.0, L::Overdub, "overdub stabs"),
        (14.0 * BAR_TICKS - 60.0, L::Overdub, "punch out"),
        (16.0 * BAR_TICKS + 1_920.0, L::Clear, "clear (after the stop)"),
    ];
    let mut next = 0;

    let frames = frame_at(BARS * BAR_TICKS) + (TAIL * f64::from(RATE)) as u64;
    let n_max = period as usize;
    let mut input = vec![0.0f32; 2 * n_max];
    let mut mix = vec![0.0f32; 2 * n_max];
    let mut pcm = Vec::with_capacity(frames as usize * 4);
    let mut peak = 0.0f32;
    let mut clipped = 0usize;
    let mut first_pass_checked = None;
    let mut last_phase = Phase::Empty;
    let mut f = 0u64;
    while f < frames {
        let n = (u64::from(period)).min(frames - f) as usize;
        while next < presses.len() && frame_at(presses[next].0) <= f {
            let (tick, c, what) = presses[next];
            looper.command(c);
            println!("looper: {} at {}", what, bar_beat(tick));
            next += 1;
        }
        for i in 0..n {
            let g = f + i as u64;
            // What the player plays reaches the line input LATENCY late.
            let x = if g >= LATENCY { player(g - LATENCY) } else { [0.0; 2] };
            input[2 * i] = x[0];
            input[2 * i + 1] = x[1];
            // They hear themselves at once (direct monitoring), and the
            // count-in clicks.
            let live = player(g);
            let c = click(g);
            mix[2 * i] = live[0] + c;
            mix[2 * i + 1] = live[1] + c;
        }
        t.process(n as u32, &mut block);
        looper.process(&block, &input[..2 * n], &mut mix[..2 * n]);
        let s = looper.status();
        if s.phase != last_phase {
            // Seen at the end of the period it happened in.
            let tick = tick_at(f + n as u64);
            let when = if tick < -BAR_TICKS { "before Play".to_string() } else { format!("by {}", bar_beat(tick)) };
            println!("looper: {:?} {when} ({} layers)", s.phase, s.layers);
            last_phase = s.phase;
        }
        if first_pass_checked.is_none() && s.layers >= 1 && f > frame_at(2.0 * BAR_TICKS) + LATENCY + 2 * FADE as u64 {
            first_pass_checked = Some(check_first_pass(&looper));
        }
        for v in &mix[..2 * n] {
            let v = v * MASTER;
            peak = peak.max(v.abs());
            clipped += usize::from(v.abs() > 1.0);
            let v = (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            pcm.extend_from_slice(&v.to_le_bytes());
        }
        f += n as u64;
    }

    let wav = wav(&pcm);
    if let Some(dir) = std::path::Path::new(&out).parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).unwrap_or_else(|e| fail(&format!("cannot make {}: {e}", dir.display())));
        }
    }
    std::fs::write(&out, &wav).unwrap_or_else(|e| fail(&format!("cannot write {out}: {e}")));
    let st = looper.stats();
    println!("wrote {out}: {:.1} s, 44.1 kHz stereo 16-bit, period {period} frames", frames as f64 / f64::from(RATE));
    println!("peak: {:.1} dBFS  clipped samples: {clipped}", 20.0 * peak.log10());
    println!("looper stats: {} commands dropped, {} overdubs refused, {} first passes thrown away", st.dropped_commands, st.refused, st.aborted);
    let err = first_pass_checked.unwrap_or(f32::INFINITY);
    println!("first pass lines up with what was played (latency {LATENCY} made up for): max error {err:.6} ({})", if err < 1e-3 { "yes" } else { "NO" });
    println!("audio  {:016x}  (repeats on this computer at this period size)", fnv(&wav));
    if err >= 1e-3 || clipped > 0 {
        std::process::exit(1);
    }
}

/// The first pass against what the player played in bars 1 and 2, away
/// from the loop point's crossfade.
fn check_first_pass(looper: &Looper) -> f32 {
    let layer = looper.layer(0).expect("the first pass");
    let start = frame_at(0.0);
    let len = layer.len() / 2;
    (FADE..len - FADE)
        .map(|j| {
            let want = player(start + j as u64);
            (layer[2 * j] - want[0]).abs().max((layer[2 * j + 1] - want[1]).abs())
        })
        .fold(0.0, f32::max)
}

fn frames_per_tick() -> f64 {
    f64::from(RATE) * 60.0 / (BPM * 960.0)
}

/// The frame of a song position (bar 1 is tick 0).
fn frame_at(tick: f64) -> u64 {
    PLAY + ((tick + BAR_TICKS) * frames_per_tick()).round() as u64
}

fn tick_at(f: u64) -> f64 {
    (f as f64 - PLAY as f64) / frames_per_tick() - BAR_TICKS
}

fn bar_beat(tick: f64) -> String {
    let bar = (tick / BAR_TICKS).floor();
    let beat = ((tick - bar * BAR_TICKS) / 960.0).floor();
    format!("bar {}.{}", bar as i64 + 1, beat as i64 + 1)
}

/// Seconds since a song position, or None before it.
fn since(f: u64, tick: f64) -> Option<f32> {
    let s = f as f64 - frame_at(tick) as f64;
    (s >= 0.0).then(|| (s / f64::from(RATE)) as f32)
}

const TAU: f32 = std::f32::consts::TAU;

fn hz(semitones_from_e1: i32) -> f32 {
    41.203 * 2f32.powf(semitones_from_e1 as f32 / 12.0)
}

/// White noise from the frame number: the same every run.
fn noise(f: u64, seed: u64) -> f32 {
    let mut x = f.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ seed.wrapping_mul(0xD6E8_FEB8_6659_FD93);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

/// A saw from its first `n` harmonics (no aliasing).
fn saw(phase: f32, n: usize) -> f32 {
    (1..=n).map(|k| (TAU * phase * k as f32).sin() / k as f32).sum::<f32>() * 0.6
}

fn pulse(phase: f32, n: usize) -> f32 {
    (1..=n).step_by(2).map(|k| (TAU * phase * k as f32).sin() / k as f32).sum::<f32>() * 0.9
}

/// Attack, then exponential decay, then a 10 ms release after `gate` s.
fn env(t: f32, decay: f32, gate: f32) -> f32 {
    let a = (t / 0.003).min(1.0);
    let r = if t > gate { (1.0 - (t - gate) / 0.01).max(0.0) } else { 1.0 };
    a * (-t / decay).exp() * r
}

/// Everything the player plays, at frame `f` of their own timeline.
fn player(f: u64) -> [f32; 2] {
    let tick = tick_at(f);
    let mut l = 0.0;
    let mut r = 0.0;
    // Bars 1-2: the bass riff in eighths.
    if (0.0..2.0 * BAR_TICKS + 960.0).contains(&tick) {
        const RIFF: [i32; 16] = [0, 0, 12, 0, 3, 0, 5, 7, 0, 0, 12, 0, 10, 7, 5, 3];
        let k = ((tick / 480.0).floor() as i64).clamp(0, 15) as usize;
        if let Some(t) = since(f, k as f64 * 480.0) {
            let e = env(t, 0.45, 0.2);
            let v = (2.6 * saw(t * hz(RIFF[k]), 14) * e).tanh() * 0.34;
            l += v;
            r += v;
        }
    }
    // Bars 3-4: drums.
    if (2.0 * BAR_TICKS..4.0 * BAR_TICKS + 960.0).contains(&tick) {
        let rel = tick - 2.0 * BAR_TICKS;
        const KICKS: [f64; 5] = [0.0, 1_920.0, 3_840.0, 5_520.0, 5_760.0];
        const SNARES: [f64; 4] = [960.0, 2_880.0, 4_800.0, 6_720.0];
        for &k in KICKS.iter().filter(|&&k| k <= rel) {
            if let Some(t) = since(f, 2.0 * BAR_TICKS + k).filter(|&t| t < 1.2) {
                let ph = 45.0 * t + 90.0 * 0.03 * (1.0 - (-t / 0.03).exp());
                // Decays to nothing at 1.2 s, so it never stops with a step.
                let e = (-t / 0.25).exp() * (1.0 - t / 1.2);
                let v = ((TAU * ph).sin() * e * 4.0).tanh() * 0.42;
                l += v;
                r += v;
            }
        }
        for &s in SNARES.iter().filter(|&&s| s <= rel) {
            if let Some(t) = since(f, 2.0 * BAR_TICKS + s).filter(|&t| t < 0.5) {
                let body = (TAU * 185.0 * t).sin() * (-t / 0.05).exp();
                let v = (noise(f, 7) * (-t / 0.11).exp() + body) * (1.0 - t / 0.5) * 0.24;
                l += v * 0.9;
                r += v;
            }
        }
        let h = (rel / 480.0).floor().max(0.0);
        if h < 16.0 {
            if let Some(t) = since(f, 2.0 * BAR_TICKS + h * 480.0) {
                // High-passed noise, crushed to 16 levels: grit on purpose.
                let n = (noise(f, 3) - noise(f.saturating_sub(1), 3)) * 0.5;
                let v = (n * 8.0).round() / 8.0 * (-t / 0.03).exp() * if h % 2.0 == 1.0 { 0.09 } else { 0.06 };
                l += v;
                r += v * 0.7;
            }
        }
    }
    // Bars 7-8: the lead.
    if (6.0 * BAR_TICKS..8.0 * BAR_TICKS + 960.0).contains(&tick) {
        // (start in eighths from bar 7, length in eighths, semitones from E1)
        const LEAD: [(u32, u32, i32); 9] = [(0, 2, 31), (2, 1, 34), (3, 1, 36), (4, 3, 38), (7, 1, 36), (8, 2, 34), (10, 2, 31), (12, 1, 29), (13, 3, 31)];
        let rel = tick - 6.0 * BAR_TICKS;
        if let Some(&(s, len, note)) = LEAD.iter().rev().find(|n| f64::from(n.0) * 480.0 <= rel) {
            if let Some(t) = since(f, 6.0 * BAR_TICKS + f64::from(s) * 480.0) {
                let gate = len as f32 * 0.3125 * 0.92;
                // Vibrato as a phase wobble that fades in from 0.15 s to
                // 0.4 s (0.6 % deep), so the pitch never jumps.
                let depth = ((t - 0.15) / 0.25).clamp(0.0, 1.0);
                let wobble = depth * depth * (3.0 - 2.0 * depth) * 0.006 * hz(note) / (TAU * 5.5);
                let phase = t * hz(note) + wobble * (TAU * 5.5 * t).sin();
                let e = env(t, 1.6, gate);
                let v = (2.2 * pulse(phase, 15) * e).tanh() * 0.17;
                l += v * 0.8;
                r += v;
            }
        }
    }
    // Bars 13-14: power-chord stabs on the off-beats.
    if (12.0 * BAR_TICKS..14.0 * BAR_TICKS + 960.0).contains(&tick) {
        let rel = tick - 12.0 * BAR_TICKS;
        let b = ((rel - 480.0) / 960.0).floor();
        if (0.0..8.0).contains(&b) {
            if let Some(t) = since(f, 12.0 * BAR_TICKS + b * 960.0 + 480.0) {
                let root = if b < 4.0 { 12 } else { 15 };
                let chord: f32 = [root, root + 7, root + 12].iter().map(|&n| saw(t * hz(n), 10)).sum();
                let v = (5.0 * chord * env(t, 0.5, 0.14)).tanh() * 0.13;
                l += v;
                r += v * 0.85;
            }
        }
    }
    // Bar 17: a final live hit (the transport has stopped).
    if let Some(t) = since(f, 16.0 * BAR_TICKS) {
        let ph = 45.0 * t + 90.0 * 0.03 * (1.0 - (-t / 0.03).exp());
        let kick = ((TAU * ph).sin() * (-t / 0.3).exp() * 4.0).tanh() * 0.42;
        let chord: f32 = [12, 19, 24].iter().map(|&n| saw(t * hz(n), 10)).sum();
        let v = (4.0 * chord * env(t, 1.2, 2.5)).tanh() * 0.16;
        l += kick + v;
        r += kick + v * 0.85;
    }
    [l, r]
}

/// The count-in: four clicks in the bar before bar 1.
fn click(f: u64) -> f32 {
    let tick = tick_at(f);
    if !(-BAR_TICKS..0.0).contains(&tick) {
        return 0.0;
    }
    let beat = ((tick + BAR_TICKS) / 960.0).floor();
    since(f, beat * 960.0 - BAR_TICKS).map_or(0.0, |t| {
        let hz = if beat == 0.0 { 1_760.0 } else { 1_320.0 };
        (TAU * hz * t).sin() * (-t / 0.02).exp() * 0.25
    })
}

fn wav(pcm: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(44 + pcm.len());
    let data = pcm.len() as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes()); // PCM
    b.extend_from_slice(&2u16.to_le_bytes()); // stereo
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

fn fail(msg: &str) -> ! {
    eprintln!("render: {msg}");
    std::process::exit(2);
}
