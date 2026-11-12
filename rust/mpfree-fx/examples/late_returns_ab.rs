//! Renders A/B pairs for the "returns a block late" choice
//! ([`Mixer::set_pipelined_returns`]): the same sounds through the same
//! mixer and return effects, once with the returns on time (A) and once
//! with them one block late (B), at the MPC's period (128 frames at
//! 44.1 kHz, 2.9 ms).
//!
//! The lane runs inline (`threaded: false`), so nothing is ever late and B
//! is A with every return one block later; the self-check proves that for
//! each case (B's return output, moved back one block, equals A's bit for
//! bit, and the dry paths are identical).
//!
//! Writes into the folder given (default `~/mpfree-work/late-returns-ab`):
//! for each case `NN-name/` with `A-now.wav`, `B-late.wav` (24-bit, both
//! with the same gain so their loudness matches), `A-then-B.wav` (A, a
//! second of silence, B) and `diff.wav` (B minus A at that same gain), and
//! `results.tsv` with the measures. Run with
//! `cargo run --release -p mpfree-fx --example late_returns_ab -- DIR`.

use std::f32::consts::TAU;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use mpfree_fx::ids::{self, mixer as mx};
use mpfree_fx::mpfree_node::Ctx;
use mpfree_fx::{
    CapturePoint, Cloud, Compressor, Delay, Drive, Mixer, MixerConfig, StripId, CLOUD_PRESETS,
};

const RATE: u32 = 44_100;
const BLOCK: usize = 128;
const BPM: f64 = 96.0;
/// Loudness every pair is set to (A's), in LUFS.
const TARGET_LUFS: f64 = -18.0;
/// The highest sample peak allowed after that gain, dBFS.
const CEILING_DB: f64 = -1.0;

/// Cases also rendered late with the reverb's pre-delay one block shorter.
const COMPENSATED: &[&str] = &[
    "drums-room",
    "plucks-room",
    "plucks-hall",
    "voice-hall",
    "pad-hall-wet",
];

const RET_A: StripId = StripId::Return(0);
const RET_B: StripId = StripId::Return(1);

type Stem = (Vec<f32>, Vec<f32>);

fn secs(s: f64) -> usize {
    (s * f64::from(RATE)) as usize
}
fn beat(b: f64) -> usize {
    secs(b * 60.0 / BPM)
}
fn hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
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

/// A saw from its harmonics below 12 kHz.
fn saw(ph: f32, hz: f32) -> f32 {
    let n = ((12_000.0 / hz) as usize).clamp(1, 48);
    (1..=n)
        .map(|k| (TAU * ph * k as f32).sin() / k as f32)
        .sum::<f32>()
        * 0.55
}

/// Adds `len` frames of `f` at `at`, panned (-1 to 1), with a 3 ms fade out.
fn add(s: &mut Stem, at: usize, len: usize, pan: f32, f: &mut dyn FnMut(usize) -> f32) {
    let fade = secs(0.003);
    let (gl, gr) = ((1.0 - pan).min(1.0), (1.0 + pan).min(1.0));
    for k in 0..len {
        let i = at + k;
        if i >= s.0.len() {
            break;
        }
        let x = f(k) * ((len - k) as f32 / fade as f32).min(1.0);
        s.0[i] += x * gl;
        s.1[i] += x * gr;
    }
}

fn zero(total: usize) -> Stem {
    (vec![0.0; total], vec![0.0; total])
}

fn kick(s: &mut Stem, at: usize, vel: f32) {
    let mut ph = 0.0f32;
    let r = RATE as f32;
    add(s, at, secs(0.4), 0.0, &mut |i| {
        let t = i as f32 / r;
        ph += (50.0 + 130.0 * (-t * 30.0).exp()) / r;
        ((ph * TAU).sin() * 2.2).tanh() * (-t * 7.0).exp() * 0.6 * vel
    });
}
fn snare(s: &mut Stem, at: usize, vel: f32, n: &mut Noise) {
    let mut ph = 0.0f32;
    let r = RATE as f32;
    add(s, at, secs(0.3), 0.05, &mut |i| {
        let t = i as f32 / r;
        ph += 185.0 / r;
        (n.next() * (-t * 15.0).exp() * 0.4 + (ph * TAU).sin() * (-t * 26.0).exp() * 0.3) * vel
    });
}
fn hat(s: &mut Stem, at: usize, vel: f32, open: bool, n: &mut Noise) {
    let r = RATE as f32;
    let mut prev = 0.0f32;
    let decay = if open { 9.0 } else { 60.0 };
    add(
        s,
        at,
        secs(if open { 0.35 } else { 0.06 }),
        -0.3,
        &mut |i| {
            let t = i as f32 / r;
            let x = n.next();
            let hp = x - prev;
            prev = x;
            hp * (-t * decay).exp() * vel
        },
    );
}
/// A rim/clap: a short bright crack, the most exposing hit for a reverb's
/// onset.
fn rim(s: &mut Stem, at: usize, pan: f32, n: &mut Noise) {
    let r = RATE as f32;
    let mut ph = 0.0f32;
    let mut prev = 0.0f32;
    add(s, at, secs(0.08), pan, &mut |i| {
        let t = i as f32 / r;
        ph += 1_700.0 / r;
        let x = n.next();
        let hp = x - prev;
        prev = x;
        ((ph * TAU).sin() * 0.5 + hp * 0.5) * (-t * 90.0).exp() * 0.8
    });
}

/// Two bars of a beat, twice, then 3 s of nothing for the tails.
fn drums() -> Stem {
    let total = beat(16.0) + secs(3.0);
    let mut s = zero(total);
    let mut n = Noise(7);
    for b in 0..4 {
        let o = b as f64 * 4.0;
        for k in [0.0, 1.75, 2.5] {
            kick(&mut s, beat(o + k), if k == 0.0 { 1.0 } else { 0.8 });
        }
        snare(&mut s, beat(o + 1.0), 1.0, &mut n);
        snare(&mut s, beat(o + 3.0), 1.0, &mut n);
        if b % 2 == 1 {
            snare(&mut s, beat(o + 3.75), 0.4, &mut n);
        }
        for h in 0..8 {
            let open = h == 7 && b % 2 == 1;
            hat(
                &mut s,
                beat(o + h as f64 * 0.5),
                if h % 2 == 1 { 0.22 } else { 0.13 },
                open,
                &mut n,
            );
        }
    }
    s
}

/// Sparse rimshots with silence between: every reverb onset in the clear.
fn rims() -> Stem {
    let total = secs(11.0);
    let mut s = zero(total);
    let mut n = Noise(3);
    for &(t, pan) in &[
        (0.2, 0.0),
        (1.4, -0.4),
        (2.6, 0.4),
        (3.2, 0.0),
        (3.5, 0.0),
        (5.0, 0.0),
        (7.0, 0.0),
    ] {
        rim(&mut s, secs(t), pan, &mut n);
    }
    s
}

/// A plucked arpeggio (Karplus-Strong), sixteenths, alternating sides.
fn plucks() -> Stem {
    let total = beat(16.0) + secs(3.0);
    let mut s = zero(total);
    let mut n = Noise(11);
    let chords: [[f32; 4]; 4] = [
        [57.0, 64.0, 69.0, 72.0],
        [53.0, 60.0, 65.0, 69.0],
        [55.0, 62.0, 67.0, 71.0],
        [52.0, 59.0, 64.0, 67.0],
    ];
    let order = [0, 1, 2, 3, 2, 1, 3, 2];
    for (bar, chord) in chords.iter().enumerate() {
        for st in 0..16 {
            let note = chord[order[st % 8]] + if st >= 8 { 12.0 } else { 0.0 };
            let f = hz(note);
            let p = (RATE as f32 / f).round() as usize;
            let mut buf: Vec<f32> = (0..p).map(|_| n.next() * 0.5).collect();
            let mut idx = 0;
            let mut prev = 0.0f32;
            let pan = if st % 2 == 0 { -0.35 } else { 0.35 };
            add(
                &mut s,
                beat(bar as f64 * 4.0 + st as f64 * 0.25),
                secs(0.9),
                pan,
                &mut |_| {
                    let x = buf[idx];
                    let y = 0.5 * (x + prev) * 0.996;
                    prev = x;
                    buf[idx] = y;
                    idx = (idx + 1) % p;
                    x * 0.5
                },
            );
        }
    }
    s
}

/// Slow saw chords, two bars each.
fn pads() -> Stem {
    let total = secs(13.0);
    let mut s = zero(total);
    let r = RATE as f32;
    let chords: [[f32; 4]; 3] = [
        [48.0, 55.0, 64.0, 71.0],
        [45.0, 57.0, 60.0, 67.0],
        [53.0, 60.0, 64.0, 69.0],
    ];
    for (c, ch) in chords.iter().enumerate() {
        let at = secs(c as f64 * 3.2);
        let len = secs(3.4);
        for (v, &m) in ch.iter().enumerate() {
            for (d, det) in [(-0.07f32, -0.5f32), (0.07, 0.5)] {
                let f = hz(m + d);
                let mut ph = v as f32 * 0.17 + d;
                let mut lp = 0.0f32;
                add(&mut s, at, len, det * 0.6, &mut |i| {
                    ph = (ph + f / r).fract();
                    let t = i as f32 / r;
                    let rem = (len - i) as f32 / r;
                    let env = (t / 0.6).min(1.0) * (rem / 0.8).min(1.0);
                    lp += (saw(ph, f) - lp) * 0.18;
                    lp * env * 0.07
                });
            }
        }
    }
    s
}

/// A sung-vowel line: a glottal pulse through three formant resonators,
/// with vibrato, gliding between vowels, phrased with breaths.
fn voice() -> Stem {
    let total = secs(12.0);
    let mut s = zero(total);
    let r = RATE as f32;
    // (start s, length s, midi, vowel a->b)
    let notes: [(f64, f64, f32, usize, usize); 7] = [
        (0.2, 0.7, 64.0, 0, 1),
        (0.95, 0.45, 67.0, 1, 2),
        (1.45, 1.6, 69.0, 2, 0),
        (3.6, 0.5, 67.0, 0, 0),
        (4.15, 0.5, 64.0, 3, 1),
        (4.7, 2.2, 62.0, 1, 2),
        (7.6, 0.9, 64.0, 0, 3),
    ];
    // Formants (Hz) of a, e, o, i.
    let vowels: [[f32; 3]; 4] = [
        [800.0, 1150.0, 2900.0],
        [400.0, 1600.0, 2700.0],
        [450.0, 800.0, 2830.0],
        [325.0, 2000.0, 2800.0],
    ];
    let mut n = Noise(23);
    for &(t0, dur, m, va, vb) in &notes {
        let f0 = hz(m);
        let len = secs(dur);
        let mut ph = 0.0f32;
        let mut vph = 0.0f32;
        let mut res = [[0.0f32; 2]; 3];
        add(&mut s, secs(t0), len + secs(0.05), 0.0, &mut |i| {
            let t = i as f32 / r;
            let k = (i as f32 / len as f32).min(1.0);
            vph += 5.5 / r;
            let vib = 1.0 + 0.012 * (vph * TAU).sin() * (t / 0.3).min(1.0);
            ph += f0 * vib / r;
            let pulse = if ph >= 1.0 {
                ph -= 1.0;
                1.0
            } else {
                0.0
            };
            let src = pulse + n.next() * 0.02;
            let mut y = 0.0;
            for (j, st) in res.iter_mut().enumerate() {
                let fr = vowels[va][j] + (vowels[vb][j] - vowels[va][j]) * k;
                let bw = 80.0 + 40.0 * j as f32;
                let rr = (-std::f32::consts::PI * bw / r).exp();
                let c = 2.0 * rr * (TAU * fr / r).cos();
                let out = src * (1.0 - rr) + c * st[0] - rr * rr * st[1];
                st[1] = st[0];
                st[0] = out;
                y += out * [1.0, 0.6, 0.3][j];
            }
            let env = (t / 0.04).min(1.0) * (((len as f32 / r) - t + 0.05) / 0.12).clamp(0.0, 1.0);
            y * env * 1.6
        });
    }
    s
}

/// What one return holds.
#[derive(Clone, Copy)]
enum Fx {
    /// A Cloud preset, 100 % wet, with optional pre-delay and early
    /// overrides (ms, %).
    Verb {
        preset: &'static str,
        pre: Option<f32>,
        early: Option<f32>,
    },
    /// A dotted-eighth ping-pong delay, 100 % wet.
    Echo,
    /// Parallel compression: a hard, fast compressor, 100 % wet.
    Squash,
    /// Parallel saturation: a drive, 100 % wet.
    Grit,
}

impl Fx {
    fn label(&self) -> String {
        match self {
            Fx::Verb { preset, pre, early } => {
                let mut s = format!("Cloud {preset}");
                if let Some(p) = pre {
                    let _ = write!(s, ", pre-delay {p} ms");
                }
                if let Some(e) = early {
                    let _ = write!(s, ", early {e} %");
                }
                s
            }
            Fx::Echo => "Delay 1/8D ping-pong".into(),
            Fx::Squash => "Compressor (parallel)".into(),
            Fx::Grit => "Drive (parallel)".into(),
        }
    }
}

/// `trim_ms` comes off a reverb's pre-delay (the compensated renders).
fn load(m: &mut Mixer, ret: StripId, fx: Fx, trim_ms: f32) {
    let c = m.chain_mut(ret).unwrap();
    match fx {
        Fx::Verb { preset, pre, early } => {
            c.push(Box::new(Cloud::new()));
            let p = CLOUD_PRESETS
                .iter()
                .find(|p| p.name == preset)
                .unwrap_or_else(|| panic!("no Cloud preset {preset}"));
            for &(id, v) in p.values {
                c.set_param(0, id, v);
            }
            let p0 = p
                .values
                .iter()
                .find(|(id, _)| *id == ids::cloud::PRE_DELAY)
                .map_or(0.0, |&(_, v)| v);
            let pre_ms = pre.unwrap_or(p0) - trim_ms;
            assert!(pre_ms >= 0.0, "{preset}: pre-delay too short to compensate");
            c.set_param(0, ids::cloud::PRE_DELAY, pre_ms);

            if let Some(v) = early {
                c.set_param(0, ids::cloud::EARLY, v);
            }
            c.set_param(0, ids::cloud::MIX, 100.0);
        }
        Fx::Echo => {
            c.push(Box::new(Delay::new()));
            c.set_param(0, ids::delay::SYNC, 7.0);
            c.set_param(0, ids::delay::FEEDBACK, 40.0);
            c.set_param(0, ids::delay::PING_PONG, 1.0);
            c.set_param(0, ids::delay::TONE, 5_000.0);
            c.set_param(0, ids::delay::MIX, 100.0);
        }
        Fx::Squash => {
            c.push(Box::new(Compressor::new()));
            c.set_param(0, ids::compressor::THRESHOLD, -32.0);
            c.set_param(0, ids::compressor::RATIO, 10.0);
            c.set_param(0, ids::compressor::ATTACK, 1.0);
            c.set_param(0, ids::compressor::RELEASE, 90.0);
            c.set_param(0, ids::compressor::MAKEUP, 14.0);
            c.set_param(0, ids::compressor::MIX, 100.0);
        }
        Fx::Grit => {
            c.push(Box::new(Drive::new()));
            c.set_param(0, ids::drive::DRIVE, 20.0);
            c.set_param(0, ids::drive::AUTO_GAIN, 1.0);
            c.set_param(0, ids::drive::MIX, 100.0);
        }
    }
}

/// A source on a track with its sends.
struct Feed<'a> {
    stem: &'a Stem,
    /// Send level to return A and B, dB (None: off).
    send: [Option<f32>; 2],
}

struct Case<'a> {
    name: &'static str,
    sound: &'static str,
    feeds: Vec<Feed<'a>>,
    fx: [Option<Fx>; 2],
    /// Return faders, dB.
    ret_db: [f32; 2],
}

struct Render {
    mix: Stem,
    /// The master's sum before Master safety.
    sum: Stem,
    wet: [Stem; 2],
    /// Frames Master safety caught.
    safety: u64,
    posts: u64,
    late: u64,
}

/// `comp`: take one block off each reverb's pre-delay (with `late`).
fn render(case: &Case, late: bool, comp: bool) -> Render {
    let total = case.feeds.iter().map(|f| f.stem.0.len()).max().unwrap();
    let mut m = Mixer::new(MixerConfig::default(), RATE, BLOCK);
    for (t, f) in case.feeds.iter().enumerate() {
        let s = StripId::Track(t);
        m.set_now(s, mx::OUTPUT, mx::output::MASTER);
        // Headroom, so Master safety never acts on either render.
        m.set_now(s, mx::VOLUME, -8.0);
        for (r, lvl) in f.send.iter().enumerate() {
            m.set_now(s, mx::send(r), lvl.unwrap_or(mpfree_fx::SILENT_DB));
        }
    }
    for (r, (ret, fx)) in [RET_A, RET_B].into_iter().zip(case.fx).enumerate() {
        if let Some(fx) = fx {
            let trim = if comp {
                BLOCK as f32 * 1e3 / RATE as f32
            } else {
                0.0
            };
            load(&mut m, ret, fx, trim);
        }
        m.set_now(ret, mx::VOLUME, case.ret_db[r]);
        assert!(m.set_capture(r, Some((ret, CapturePoint::AfterFader))));
    }
    // The master's sum before Master safety, for the dry check.
    assert!(m.set_capture(2, Some((StripId::Master, CapturePoint::BeforeFader))));
    if late {
        let cfg = mpfree_lanes::Config {
            threaded: false,
            ..Default::default()
        };
        m.set_pipelined_returns(Some(cfg), Duration::from_secs(1));
        assert!(m.pipelined_returns());
    }
    let mut mix = zero(total);
    let mut wet = [zero(total), zero(total)];
    let mut sum = zero(total);
    let mut at = 0;
    while at < total {
        // Fixed 128-frame blocks, as the app's period; a short last block
        // would drop a block of the late returns, so pad to a whole block.
        let len = BLOCK;
        for (t, f) in case.feeds.iter().enumerate() {
            let (l, r) = m.strip_input(StripId::Track(t)).unwrap();
            for k in 0..len {
                l[k] = f.stem.0.get(at + k).copied().unwrap_or(0.0);
                r[k] = f.stem.1.get(at + k).copied().unwrap_or(0.0);
            }
        }
        let ctx = Ctx {
            bpm: BPM,
            beats: at as f64 / f64::from(RATE) * BPM / 60.0,
            playing: true,
            ..Ctx::new(RATE, at as u64)
        };
        let (mut l, mut r) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
        m.process(&ctx, &mut l, &mut r);
        let n = len.min(total - at);
        mix.0[at..at + n].copy_from_slice(&l[..n]);
        mix.1[at..at + n].copy_from_slice(&r[..n]);
        for (slot, dst) in wet.iter_mut().enumerate() {
            let (cl, cr) = m.capture(slot).unwrap();
            dst.0[at..at + n].copy_from_slice(&cl[..n]);
            dst.1[at..at + n].copy_from_slice(&cr[..n]);
        }
        let (cl, cr) = m.capture(2).unwrap();
        sum.0[at..at + n].copy_from_slice(&cl[..n]);
        sum.1[at..at + n].copy_from_slice(&cr[..n]);
        at += len;
    }
    let st = m.stats();
    let safety = m.safety().stats().overs;
    Render {
        mix,
        sum,
        wet,
        safety,
        posts: st.pipe_posts,
        late: st.pipe_late,
    }
}

// ---- Loudness (ITU-R BS.1770-4, gated) ----

/// A biquad from the BS.1770 K-weighting, recomputed for the rate.
fn k_weight(x: &[f32]) -> Vec<f64> {
    let fs = f64::from(RATE);
    // Stage 1: high shelf.
    let (f0, g, q) = (
        1_681.974_450_955_533,
        3.999_843_853_973_347,
        0.707_175_236_955_419_6,
    );
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let vh = 10f64.powf(g / 20.0);
    let vb = vh.powf(0.499_666_774_154_541_6);
    let a0 = 1.0 + k / q + k * k;
    let s1 = (
        [
            (vh + vb * k / q + k * k) / a0,
            2.0 * (k * k - vh) / a0,
            (vh - vb * k / q + k * k) / a0,
        ],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    );
    // Stage 2: high pass.
    let (f0, q) = (38.135_470_876_024_44, 0.500_327_037_323_877_3);
    let k = (std::f64::consts::PI * f0 / fs).tan();
    let a0 = 1.0 + k / q + k * k;
    let s2 = (
        [1.0, -2.0, 1.0],
        [2.0 * (k * k - 1.0) / a0, (1.0 - k / q + k * k) / a0],
    );
    let run = |x: &[f64], (b, a): ([f64; 3], [f64; 2])| -> Vec<f64> {
        let (mut x1, mut x2, mut y1, mut y2) = (0.0, 0.0, 0.0, 0.0);
        x.iter()
            .map(|&v| {
                let y = b[0] * v + b[1] * x1 + b[2] * x2 - a[0] * y1 - a[1] * y2;
                x2 = x1;
                x1 = v;
                y2 = y1;
                y1 = y;
                y
            })
            .collect()
    };
    let x: Vec<f64> = x.iter().map(|&v| f64::from(v)).collect();
    run(&run(&x, s1), s2)
}

fn lufs(s: &Stem) -> f64 {
    let (l, r) = (k_weight(&s.0), k_weight(&s.1));
    let win = secs(0.4);
    let hop = secs(0.1);
    let mut blocks = Vec::new();
    let mut i = 0;
    while i + win <= l.len() {
        let e: f64 = (i..i + win).map(|k| l[k] * l[k] + r[k] * r[k]).sum::<f64>() / win as f64;
        blocks.push(e);
        i += hop;
    }
    let loud = |e: f64| -0.691 + 10.0 * e.max(1e-20).log10();
    let abs: Vec<f64> = blocks
        .iter()
        .copied()
        .filter(|&e| loud(e) > -70.0)
        .collect();
    if abs.is_empty() {
        return -70.0;
    }
    let mean = abs.iter().sum::<f64>() / abs.len() as f64;
    let rel = loud(mean) - 10.0;
    let gated: Vec<f64> = abs.into_iter().filter(|&e| loud(e) > rel).collect();
    loud(gated.iter().sum::<f64>() / gated.len() as f64)
}

fn peak(s: &Stem) -> f32 {
    s.0.iter().chain(&s.1).fold(0.0f32, |m, v| m.max(v.abs()))
}
fn energy(s: &Stem) -> f64 {
    s.0.iter()
        .chain(&s.1)
        .map(|&v| f64::from(v) * f64::from(v))
        .sum()
}
fn db(x: f64) -> f64 {
    10.0 * x.max(1e-30).log10()
}

fn wav24(path: &Path, s: &Stem, gain: f32) {
    let mut pcm = Vec::with_capacity(s.0.len() * 6);
    for (&l, &r) in s.0.iter().zip(&s.1) {
        for v in [l, r] {
            let q = ((v * gain).clamp(-1.0, 1.0) * 8_388_607.0).round() as i32;
            pcm.extend_from_slice(&q.to_le_bytes()[..3]);
        }
    }
    let data = pcm.len() as u32;
    let mut b = Vec::with_capacity(44 + pcm.len());
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&RATE.to_le_bytes());
    b.extend_from_slice(&(RATE * 6).to_le_bytes());
    b.extend_from_slice(&6u16.to_le_bytes());
    b.extend_from_slice(&24u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    b.extend_from_slice(&pcm);
    std::fs::write(path, b).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Raw float stereo, for the spectral check outside.
fn raw_f32(path: &Path, s: &Stem, gain: f32) {
    let mut b = Vec::with_capacity(s.0.len() * 8);
    for (&l, &r) in s.0.iter().zip(&s.1) {
        b.extend_from_slice(&(l * gain).to_le_bytes());
        b.extend_from_slice(&(r * gain).to_le_bytes());
    }
    std::fs::write(path, b).unwrap();
}

fn main() {
    let out: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            Path::new(&home).join("mpfree-work").join("late-returns-ab")
        });
    std::fs::create_dir_all(&out).unwrap();
    let raw = std::env::var_os("LATE_AB_RAW").map(PathBuf::from);
    if let Some(r) = &raw {
        std::fs::create_dir_all(r).unwrap();
    }

    let (dr, rm, pl, pd, vo) = (drums(), rims(), plucks(), pads(), voice());
    let verb = |preset, pre, early| Some(Fx::Verb { preset, pre, early });
    let one = |stem, send: f32| {
        vec![Feed {
            stem,
            send: [Some(send), None],
        }]
    };
    let cases: Vec<Case> = vec![
        Case {
            name: "drums-room",
            sound: "drum loop",
            feeds: one(&dr, -8.0),
            fx: [verb("Small room", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "drums-plate",
            sound: "drum loop",
            feeds: one(&dr, -12.0),
            fx: [verb("Dark plate", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "drums-hall-wet",
            sound: "drum loop",
            feeds: one(&dr, 0.0),
            fx: [verb("Big hall", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "rims-room-close",
            sound: "sparse rimshots",
            feeds: one(&rm, -4.0),
            fx: [verb("Small room", Some(0.0), Some(100.0)), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "rims-plate-predelay40",
            sound: "sparse rimshots",
            feeds: one(&rm, -8.0),
            fx: [verb("Dark plate", Some(40.0), None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "rims-hall-low",
            sound: "sparse rimshots",
            feeds: one(&rm, -20.0),
            fx: [verb("Big hall", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "rims-chamber",
            sound: "sparse rimshots",
            feeds: one(&rm, -6.0),
            fx: [verb("Echo chamber", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "plucks-room",
            sound: "plucked arpeggio",
            feeds: one(&pl, -8.0),
            fx: [verb("Small room", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "plucks-hall",
            sound: "plucked arpeggio",
            feeds: one(&pl, -10.0),
            fx: [verb("Big hall", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "plucks-delay",
            sound: "plucked arpeggio",
            feeds: one(&pl, -6.0),
            fx: [Some(Fx::Echo), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "pad-hall-wet",
            sound: "saw pad",
            feeds: one(&pd, 0.0),
            fx: [verb("Big hall", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "pad-cloud",
            sound: "saw pad",
            feeds: one(&pd, -10.0),
            fx: [verb("Endless cloud", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "voice-plate",
            sound: "sung vowels",
            feeds: one(&vo, -10.0),
            fx: [verb("Dark plate", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "voice-room-close",
            sound: "sung vowels",
            feeds: one(&vo, -4.0),
            fx: [verb("Small room", Some(0.0), Some(100.0)), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "voice-hall",
            sound: "sung vowels",
            feeds: one(&vo, -10.0),
            fx: [verb("Big hall", None, None), None],
            ret_db: [0.0; 2],
        },
        Case {
            name: "song-plate-delay",
            sound: "drums + plucks + pad + voice",
            feeds: vec![
                Feed {
                    stem: &dr,
                    send: [Some(-14.0), None],
                },
                Feed {
                    stem: &pl,
                    send: [Some(-12.0), Some(-10.0)],
                },
                Feed {
                    stem: &pd,
                    send: [Some(-8.0), None],
                },
                Feed {
                    stem: &vo,
                    send: [Some(-8.0), Some(-14.0)],
                },
            ],
            fx: [verb("Dark plate", None, None), Some(Fx::Echo)],
            ret_db: [0.0; 2],
        },
        Case {
            name: "WORST-drums-parallel-comp",
            sound: "drum loop",
            feeds: one(&dr, 0.0),
            fx: [Some(Fx::Squash), None],
            ret_db: [-4.0, 0.0],
        },
        Case {
            name: "WORST-voice-parallel-drive",
            sound: "sung vowels",
            feeds: one(&vo, 0.0),
            fx: [Some(Fx::Grit), None],
            ret_db: [-3.0, 0.0],
        },
    ];

    let mut tsv = String::from(
        "case\tsound\treturns\tsends_dB\tA_LUFS\tB_LUFS\tB_minus_A_LU\tgain_dB\twet_vs_dry_dB\tdiff_vs_mix_dB\twet_exact_one_block_late\tlate_joins\tsafety_overs_A\tsafety_overs_B\n",
    );
    println!(
        "Late returns A/B: {RATE} Hz, {BLOCK}-frame blocks ({:.2} ms)",
        BLOCK as f64 * 1e3 / f64::from(RATE)
    );
    for (i, case) in cases.iter().enumerate() {
        let a = render(case, false, false);
        let b = render(case, true, false);
        // Self-check: B's returns are A's one block later, bit for bit
        // (the first block of B's returns is silent), and nothing was late.
        let mut exact = b.late == 0;
        for r in 0..2 {
            let (wa, wb) = (&a.wet[r], &b.wet[r]);
            let n = wa.0.len();
            exact &= wb.0[..BLOCK]
                .iter()
                .chain(&wb.1[..BLOCK])
                .all(|&v| v == 0.0);
            exact &= (0..n - BLOCK).all(|k| {
                wb.0[k + BLOCK].to_bits() == wa.0[k].to_bits()
                    && wb.1[k + BLOCK].to_bits() == wa.1[k].to_bits()
            });
        }
        if case.fx.iter().flatten().count() > 0 {
            exact &= b.posts > 0;
        }
        // The dry part is the mix minus the returns; the same in both.
        let dry = |x: &Render| -> Stem {
            let n = x.mix.0.len();
            let f = |c: usize, k: usize| {
                let m = if c == 0 { x.sum.0[k] } else { x.sum.1[k] };
                let w: f32 = x
                    .wet
                    .iter()
                    .map(|w| if c == 0 { w.0[k] } else { w.1[k] })
                    .sum();
                m - w
            };
            (
                (0..n).map(|k| f(0, k)).collect(),
                (0..n).map(|k| f(1, k)).collect(),
            )
        };
        let (da, dbb) = (dry(&a), dry(&b));
        let dry_err =
            da.0.iter()
                .chain(&da.1)
                .zip(dbb.0.iter().chain(&dbb.1))
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max);
        exact &= dry_err < 1e-5;

        let la = lufs(&a.mix);
        let lb = lufs(&b.mix);
        // A to the target loudness; B on its own gain to the same loudness,
        // so a level change does not sway the listening (the change is in
        // the table). Both come down together if a peak would pass the
        // ceiling.
        let mut gain_db = TARGET_LUFS - la;
        let gain_b_db = la - lb;
        let pk = (20.0 * f64::from(peak(&a.mix)).log10())
            .max(20.0 * f64::from(peak(&b.mix)).log10() + gain_b_db);
        if pk + gain_db > CEILING_DB {
            gain_db = CEILING_DB - pk;
        }
        let g = 10f64.powf(gain_db / 20.0) as f32;
        let gb = g * 10f64.powf(gain_b_db / 20.0) as f32;
        let diff: Stem = (
            b.mix.0.iter().zip(&a.mix.0).map(|(x, y)| x - y).collect(),
            b.mix.1.iter().zip(&a.mix.1).map(|(x, y)| x - y).collect(),
        );
        let wet_e: f64 = a.wet.iter().map(energy).sum();
        let wet_vs_dry = db(wet_e) - db(energy(&da));
        let diff_vs_mix = db(energy(&diff)) - db(energy(&a.mix));

        let dir = out.join(format!("{:02}-{}", i + 1, case.name));
        std::fs::create_dir_all(&dir).unwrap();
        wav24(&dir.join("A-now.wav"), &a.mix, g);
        wav24(&dir.join("B-late.wav"), &b.mix, gb);
        wav24(&dir.join("diff.wav"), &diff, g);
        let gap = secs(1.0);
        let ab: Stem = (
            a.mix
                .0
                .iter()
                .copied()
                .chain(std::iter::repeat_n(0.0, gap))
                .chain(b.mix.0.iter().map(|v| v * gb / g))
                .collect(),
            a.mix
                .1
                .iter()
                .copied()
                .chain(std::iter::repeat_n(0.0, gap))
                .chain(b.mix.1.iter().map(|v| v * gb / g))
                .collect(),
        );
        wav24(&dir.join("A-then-B.wav"), &ab, g);
        if let Some(r) = &raw {
            raw_f32(
                &r.join(format!("{:02}-{}-A.f32", i + 1, case.name)),
                &a.mix,
                g,
            );
            raw_f32(
                &r.join(format!("{:02}-{}-B.f32", i + 1, case.name)),
                &b.mix,
                g,
            );
            raw_f32(
                &r.join(format!("{:02}-{}-dry.f32", i + 1, case.name)),
                &da,
                g,
            );
        }
        // Late, with a block taken off the reverb's pre-delay: should sound
        // like A again.
        if COMPENSATED.contains(&case.name) {
            let c = render(case, true, true);
            let lc = lufs(&c.mix);
            let gc = g * 10f64.powf((la - lc) / 20.0) as f32;
            wav24(&dir.join("C-late-predelay-compensated.wav"), &c.mix, gc);
            if let Some(r) = &raw {
                raw_f32(
                    &r.join(format!("{:02}-{}-C.f32", i + 1, case.name)),
                    &c.mix,
                    g,
                );
            }
        }
        let rets: Vec<String> = case.fx.iter().flatten().map(Fx::label).collect();
        let sends: Vec<String> = case
            .feeds
            .iter()
            .map(|f| {
                f.send
                    .iter()
                    .map(|s| s.map_or("-".into(), |v| format!("{v}")))
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .collect();
        let _ = writeln!(
            tsv,
            "{:02}-{}\t{}\t{}\t{}\t{:.2}\t{:.2}\t{:+.3}\t{:+.1}\t{:+.1}\t{:+.1}\t{}\t{}\t{}\t{}",
            i + 1,
            case.name,
            case.sound,
            rets.join(" + "),
            sends.join(" "),
            la,
            lb,
            lb - la,
            gain_db,
            wet_vs_dry,
            diff_vs_mix,
            exact,
            b.late,
            a.safety,
            b.safety
        );
        println!(
            "  {:02} {:<28} A {:>6.2} LUFS  B-A {:+.3} LU  wet/dry {:+5.1} dB  diff/mix {:+6.1} dB  exact {}  safety {}/{}",
            i + 1,
            case.name,
            la,
            lb - la,
            wet_vs_dry,
            diff_vs_mix,
            exact,
            a.safety,
            b.safety
        );
        assert!(
            exact,
            "{}: B is not A with the returns one block late",
            case.name
        );
        assert!(a.mix.0.iter().chain(&b.mix.0).all(|v| v.is_finite()));
    }
    std::fs::write(out.join("results.tsv"), tsv).unwrap();
    println!("wrote {}", out.display());
}
