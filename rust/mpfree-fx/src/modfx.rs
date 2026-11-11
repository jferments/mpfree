//! Modulation effects: [`Chorus`], [`Flanger`] and [`Phaser`].
//!
//! They share an LFO that runs free (in Hz) or synced to the tempo (1/16
//! to 8 bars a cycle), and restarts in time with the bar when the transport
//! starts. Width offsets the right side's LFO phase, so 100 % is a wide
//! stereo sweep and 0 % keeps both sides moving together. Everything that
//! can move glides, so turning a knob under a loop never clicks or zips;
//! a change of voice or stage count crossfades.
//!
//! The LFO is a polynomial sine and the delay reads are cubic, so a render
//! is the same on every machine (the phaser's tangent aside) and a swept
//! delay stays bright.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{defaults, flush, lerp};

const SYNCS: &[&str] = &["Free", "1/16", "1/8", "1/4", "1/2", "1 bar", "2 bars", "4 bars", "8 bars"];
/// Beats per LFO cycle for each sync choice (0 is free).
const SYNC_BEATS: [f64; 9] = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0];

/// A sine of `phase` cycles (0 to 1): a parabola with a cubic correction
/// (error under 0.1 %), plain arithmetic, so it is the same everywhere.
#[inline]
fn sine(phase: f32) -> f32 {
    let x = if phase < 0.5 { phase } else { phase - 1.0 };
    let y = 8.0 * x - 16.0 * x * x.abs();
    0.225 * (y * y.abs() - y) + y
}

/// `phase + off` (never negative), wrapped into 0 to 1. Truncation is floor
/// here, with no library call.
#[inline]
fn wrap(p: f32) -> f32 { p - (p as u32) as f32 }

/// Cubic Hermite (Catmull-Rom) interpolation between `x1` (t = 0) and `x2`.
#[inline]
fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

/// The modulation LFO.
#[derive(Clone, Copy, Debug)]
struct Lfo {
    /// Cycles, 0 to 1 (f64, so a long free run never drifts or rounds).
    phase: f64,
    inc: f64,
    bpm: f64,
    playing: bool,
}

impl Default for Lfo {
    fn default() -> Lfo { Lfo { phase: 0.0, inc: 0.0, bpm: 120.0, playing: false } }
}

impl Lfo {
    /// At the start of a block: follow the tempo, and when synced, put the
    /// phase where the bar says as the transport starts.
    fn begin(&mut self, ctx: &Ctx, sync: usize) {
        if ctx.bpm > 0.0 { self.bpm = ctx.bpm; }
        if sync > 0 && ctx.playing && !self.playing {
            self.phase = (ctx.beats / SYNC_BEATS[sync]).rem_euclid(1.0);
        }
        self.playing = ctx.playing;
    }
    fn set(&mut self, hz: f32, sync: usize, rate: u32) {
        let cps = if sync > 0 { self.bpm / 60.0 / SYNC_BEATS[sync] } else { f64::from(hz) };
        self.inc = cps / f64::from(rate.max(1));
    }
    #[inline]
    fn next(&mut self) -> f32 {
        self.phase += self.inc;
        if self.phase >= 1.0 { self.phase -= 1.0; }
        self.phase as f32
    }
    fn reset(&mut self) {
        self.phase = 0.0;
        self.playing = false;
    }
}

/// A delay line read at a fractional, moving point.
#[derive(Clone, Debug, Default)]
struct ModLine {
    buf: Vec<f32>,
    mask: usize,
    w: usize,
}

impl ModLine {
    /// Room for `frames` of delay (control thread).
    fn new(frames: usize) -> ModLine {
        let n = (frames + 4).next_power_of_two();
        ModLine { buf: vec![0.0; n], mask: n - 1, w: 0 }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.w] = x;
        self.w = (self.w + 1) & self.mask;
    }
    /// The signal `d` frames before the last write (0 is the last write),
    /// `d` at least 1, interpolated.
    #[inline]
    fn read(&self, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.buf.len() - 4) as f32);
        let i = d as usize;
        let t = d - i as f32;
        let last = self.w.wrapping_sub(1);
        let at = |k: usize| self.buf[last.wrapping_sub(k) & self.mask];
        hermite(at(i - 1), at(i), at(i + 1), at(i + 2), t)
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

fn smooth_all(list: &mut [&mut Smooth], rate: u32, ms: f32) {
    for s in list { s.set_time(rate, ms); }
}

// ---------------------------------------------------------------- chorus

/// [`Chorus`] parameter ids.
pub mod chorus {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const DELAY: u16 = 3;
    pub const VOICES: u16 = 4;
    pub const WIDTH: u16 = 5;
    pub const TONE: u16 = 6;
    pub const MIX: u16 = 7;
}

static CHORUS_INFO: NodeInfo = NodeInfo { id: "mpfree.fx.chorus", name: "Chorus", kind: Kind::Effect };
static CHORUS_PARAMS: [ParamInfo; 8] = [
    ParamInfo { id: 0, name: "Rate", group: "LFO", min: 0.05, max: 10.0, default: 0.6, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 1, name: "Sync", group: "LFO", min: 0.0, max: 8.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(SYNCS), readonly: false },
    ParamInfo { id: 2, name: "Depth", group: "LFO", min: 0.0, max: 100.0, default: 40.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 3, name: "Delay", group: "Chorus", min: 2.0, max: 40.0, default: 12.0, unit: Unit::Ms, curve: Curve::Log, readonly: false },
    ParamInfo { id: 4, name: "Voices", group: "Chorus", min: 1.0, max: 4.0, default: 2.0, unit: Unit::None, curve: Curve::Stepped, readonly: false },
    ParamInfo { id: 5, name: "Width", group: "Chorus", min: 0.0, max: 100.0, default: 100.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 6, name: "Tone", group: "Output", min: 1_000.0, max: 20_000.0, default: 9_000.0, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 7, name: "Mix", group: "Output", min: 0.0, max: 100.0, default: 50.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
];

/// LFO phase offsets per voice: any count of voices starts from the same
/// ones, so adding a voice never moves the others.
const VOICE_PHASE: [f32; 4] = [0.0, 0.5, 0.25, 0.75];
const MAX_CHORUS_MS: f32 = 61.0;

/// A lush stereo chorus: up to four voices reading a delay swept by one
/// LFO at spread phases, the right side a quarter cycle on at full width,
/// and a tone control to keep the wet signal warm.
pub struct Chorus {
    v: [f32; 8],
    rate: u32,
    lfo: Lfo,
    lines: [ModLine; 2],
    depth: Smooth,
    log_delay: Smooth,
    width: Smooth,
    log_tone: Smooth,
    mix: Smooth,
    gains: [Smooth; 4],
    /// Cached while nothing moves: base delay and swing in frames, the
    /// tone coefficient.
    base: f32,
    swing: f32,
    tone_a: f32,
    tone: [f32; 2],
}

impl Default for Chorus {
    fn default() -> Self { Self::new() }
}

impl Chorus {
    pub fn new() -> Chorus {
        let v = defaults(&CHORUS_PARAMS);
        let mut c = Chorus {
            v,
            rate: 0,
            lfo: Lfo::default(),
            lines: [ModLine::default(), ModLine::default()],
            depth: Smooth::new(v[2] / 100.0),
            log_delay: Smooth::new(v[3].log2()),
            width: Smooth::new(v[5] / 100.0),
            log_tone: Smooth::new(v[6].log2()),
            mix: Smooth::new(v[7] / 100.0),
            gains: [Smooth::new(0.0); 4],
            base: 0.0,
            swing: 0.0,
            tone_a: 1.0,
            tone: [0.0; 2],
        };
        c.voices();
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c.reset();
        c
    }

    fn voices(&mut self) {
        let n = self.v[4] as usize;
        let g = 1.0 / (n as f32).sqrt();
        for (k, s) in self.gains.iter_mut().enumerate() { s.target = if k < n { g } else { 0.0 }; }
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = CHORUS_PARAMS.get(id as usize) else { return };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            chorus::DEPTH => self.depth.target = v / 100.0,
            chorus::DELAY => self.log_delay.target = v.log2(),
            chorus::VOICES => self.voices(),
            chorus::WIDTH => self.width.target = v / 100.0,
            chorus::TONE => self.log_tone.target = v.log2(),
            chorus::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
    }

    fn update(&mut self) {
        let fs = self.rate as f32;
        self.base = self.log_delay.value.exp2() * 0.001 * fs;
        self.swing = 0.5 * self.depth.value * self.base;
        self.tone_a = 1.0 - (-2.0 * PI * self.log_tone.value.exp2() / fs).exp();
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            if !(self.depth.settled() && self.log_delay.settled() && self.log_tone.settled()) {
                self.depth.next();
                self.log_delay.next();
                self.log_tone.next();
                self.update();
            }
            let ph = self.lfo.next();
            let off_r = 0.25 * self.width.next();
            let mix = self.mix.next();
            let mut g = [0.0f32; 4];
            for (g, s) in g.iter_mut().zip(&mut self.gains) { *g = s.next(); }
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                self.lines[ch].push(x[ch]);
                let off = if ch == 0 { 0.0 } else { off_r };
                let mut wet = 0.0;
                for k in 0..4 {
                    if g[k] == 0.0 { continue; }
                    let d = self.base + self.swing * sine(wrap(ph + VOICE_PHASE[k] + off));
                    wet += g[k] * self.lines[ch].read(d);
                }
                self.tone[ch] = flush(self.tone[ch] + self.tone_a * (wet - self.tone[ch]));
                y[ch] = lerp(x[ch], self.tone[ch], mix);
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }
}

impl Node for Chorus {
    fn info(&self) -> &'static NodeInfo { &CHORUS_INFO }
    fn params(&self) -> &'static [ParamInfo] { &CHORUS_PARAMS }
    fn set_param(&mut self, id: u16, value: f32) { self.apply(id, value); }
    fn param(&self, id: u16) -> f32 { self.v.get(id as usize).copied().unwrap_or(f32::NAN) }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        if rate != self.rate {
            let frames = (MAX_CHORUS_MS * 0.001 * rate as f32) as usize + 4;
            self.lines = [ModLine::new(frames), ModLine::new(frames)];
        }
        self.rate = rate;
        smooth_all(&mut [&mut self.depth, &mut self.log_delay, &mut self.width, &mut self.log_tone, &mut self.mix], rate, 30.0);
        for s in &mut self.gains { s.set_time(rate, 20.0); }
        self.lfo.set(self.v[0], self.v[1] as usize, rate);
        self.update();
    }
    fn reset(&mut self) {
        for s in [&mut self.depth, &mut self.log_delay, &mut self.width, &mut self.log_tone, &mut self.mix] { s.snap(s.target); }
        for s in &mut self.gains { s.snap(s.target); }
        for line in &mut self.lines { line.clear(); }
        self.tone = [0.0; 2];
        self.lfo.reset();
        self.update();
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.lfo.begin(ctx, self.v[1] as usize);
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| if let NodeEvent::Param { id, value } = *ev { self.apply(id, value) }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn tail_frames(&self) -> u32 {
        let ms = self.v[3] * (1.0 + 0.5 * self.v[2] / 100.0);
        // The delay, then the tone filter settling.
        (ms * 0.001 * self.rate as f32) as u32 + self.rate / 100
    }
}

// --------------------------------------------------------------- flanger

/// [`Flanger`] parameter ids.
pub mod flanger {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const DELAY: u16 = 3;
    pub const FEEDBACK: u16 = 4;
    pub const WIDTH: u16 = 5;
    pub const MIX: u16 = 6;
}

static FLANGER_INFO: NodeInfo = NodeInfo { id: "mpfree.fx.flanger", name: "Flanger", kind: Kind::Effect };
static FLANGER_PARAMS: [ParamInfo; 7] = [
    ParamInfo { id: 0, name: "Rate", group: "LFO", min: 0.02, max: 10.0, default: 0.2, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 1, name: "Sync", group: "LFO", min: 0.0, max: 8.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(SYNCS), readonly: false },
    ParamInfo { id: 2, name: "Depth", group: "LFO", min: 0.0, max: 100.0, default: 60.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 3, name: "Delay", group: "Flanger", min: 0.1, max: 10.0, default: 1.5, unit: Unit::Ms, curve: Curve::Log, readonly: false },
    ParamInfo { id: 4, name: "Feedback", group: "Flanger", min: -95.0, max: 95.0, default: 50.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 5, name: "Width", group: "Flanger", min: 0.0, max: 100.0, default: 100.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 6, name: "Mix", group: "Output", min: 0.0, max: 100.0, default: 50.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
];

/// The sweep at full depth: two octaves of delay each way.
const FLANGE_OCTAVES: f32 = 2.0;

/// A stereo flanger: a short delay swept in octaves (so the comb moves
/// evenly), with positive (metallic) or negative (hollow) feedback.
pub struct Flanger {
    v: [f32; 7],
    rate: u32,
    lfo: Lfo,
    lines: [ModLine; 2],
    depth: Smooth,
    log_delay: Smooth,
    feedback: Smooth,
    width: Smooth,
    mix: Smooth,
    /// The base delay in frames, cached while it holds still.
    base: f32,
}

impl Default for Flanger {
    fn default() -> Self { Self::new() }
}

impl Flanger {
    pub fn new() -> Flanger {
        let v = defaults(&FLANGER_PARAMS);
        let mut f = Flanger {
            v,
            rate: 0,
            lfo: Lfo::default(),
            lines: [ModLine::default(), ModLine::default()],
            depth: Smooth::new(v[2] / 100.0),
            log_delay: Smooth::new(v[3].log2()),
            feedback: Smooth::new(v[4] / 100.0),
            width: Smooth::new(v[5] / 100.0),
            mix: Smooth::new(v[6] / 100.0),
            base: 0.0,
        };
        f.prepare(48_000, mpfree_node::MAX_BLOCK);
        f.reset();
        f
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = FLANGER_PARAMS.get(id as usize) else { return };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            flanger::DEPTH => self.depth.target = v / 100.0,
            flanger::DELAY => self.log_delay.target = v.log2(),
            flanger::FEEDBACK => self.feedback.target = v / 100.0,
            flanger::WIDTH => self.width.target = v / 100.0,
            flanger::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            if !self.log_delay.settled() {
                self.base = self.log_delay.next().exp2() * 0.001 * self.rate as f32;
            }
            let depth = FLANGE_OCTAVES * self.depth.next();
            let fb = self.feedback.next();
            let off_r = 0.25 * self.width.next();
            let mix = self.mix.next();
            let ph = self.lfo.next();
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let off = if ch == 0 { 0.0 } else { off_r };
                let d = self.base * (depth * sine(wrap(ph + off))).exp2();
                // Read before the write: the last write is one frame back.
                let wet = self.lines[ch].read(d - 1.0);
                self.lines[ch].push(flush(x[ch] + fb * wet));
                y[ch] = lerp(x[ch], wet, mix);
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }
}

impl Node for Flanger {
    fn info(&self) -> &'static NodeInfo { &FLANGER_INFO }
    fn params(&self) -> &'static [ParamInfo] { &FLANGER_PARAMS }
    fn set_param(&mut self, id: u16, value: f32) { self.apply(id, value); }
    fn param(&self, id: u16) -> f32 { self.v.get(id as usize).copied().unwrap_or(f32::NAN) }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        if rate != self.rate {
            // The longest delay times the widest sweep.
            let frames = (10.0 * FLANGE_OCTAVES.exp2() * 0.001 * rate as f32) as usize + 8;
            self.lines = [ModLine::new(frames), ModLine::new(frames)];
        }
        self.rate = rate;
        smooth_all(&mut [&mut self.depth, &mut self.log_delay, &mut self.feedback, &mut self.width, &mut self.mix], rate, 30.0);
        self.lfo.set(self.v[0], self.v[1] as usize, rate);
        self.base = self.log_delay.value.exp2() * 0.001 * rate as f32;
    }
    fn reset(&mut self) {
        for s in [&mut self.depth, &mut self.log_delay, &mut self.feedback, &mut self.width, &mut self.mix] { s.snap(s.target); }
        for line in &mut self.lines { line.clear(); }
        self.lfo.reset();
        self.base = self.log_delay.value.exp2() * 0.001 * self.rate as f32;
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.lfo.begin(ctx, self.v[1] as usize);
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| if let NodeEvent::Param { id, value } = *ev { self.apply(id, value) }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn tail_frames(&self) -> u32 {
        // The longest loop, rung down 80 dB by the feedback.
        let longest = self.v[3] * 0.001 * (FLANGE_OCTAVES * self.v[2] / 100.0).exp2() * self.rate as f32;
        let fb = (self.v[4] / 100.0).abs();
        let rings = if fb > 0.0 { (1e-4f32).ln() / fb.ln() } else { 0.0 };
        (longest * (1.0 + rings)) as u32 + 2
    }
}

// ---------------------------------------------------------------- phaser

/// [`Phaser`] parameter ids.
pub mod phaser {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const CENTER: u16 = 3;
    pub const STAGES: u16 = 4;
    pub const FEEDBACK: u16 = 5;
    pub const WIDTH: u16 = 6;
    pub const MIX: u16 = 7;
}

const STAGE_NAMES: &[&str] = &["2", "4", "6", "8", "12"];
const STAGE_COUNTS: [usize; 5] = [2, 4, 6, 8, 12];
const MAX_STAGES: usize = 12;
/// The sweep at full depth: two and a half octaves each way.
const PHASE_OCTAVES: f32 = 2.5;

static PHASER_INFO: NodeInfo = NodeInfo { id: "mpfree.fx.phaser", name: "Phaser", kind: Kind::Effect };
static PHASER_PARAMS: [ParamInfo; 8] = [
    ParamInfo { id: 0, name: "Rate", group: "LFO", min: 0.02, max: 10.0, default: 0.3, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 1, name: "Sync", group: "LFO", min: 0.0, max: 8.0, default: 0.0, unit: Unit::None, curve: Curve::Choice(SYNCS), readonly: false },
    ParamInfo { id: 2, name: "Depth", group: "LFO", min: 0.0, max: 100.0, default: 70.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 3, name: "Center", group: "Phaser", min: 100.0, max: 8_000.0, default: 700.0, unit: Unit::Hz, curve: Curve::Log, readonly: false },
    ParamInfo { id: 4, name: "Stages", group: "Phaser", min: 0.0, max: 4.0, default: 1.0, unit: Unit::None, curve: Curve::Choice(STAGE_NAMES), readonly: false },
    ParamInfo { id: 5, name: "Feedback", group: "Phaser", min: -95.0, max: 95.0, default: 30.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 6, name: "Width", group: "Phaser", min: 0.0, max: 100.0, default: 100.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
    ParamInfo { id: 7, name: "Mix", group: "Output", min: 0.0, max: 100.0, default: 50.0, unit: Unit::Percent, curve: Curve::Linear, readonly: false },
];

/// A stereo phaser: 2 to 12 first-order all-pass stages (zero-delay
/// feedback, so a fast sweep stays clean) swept in octaves around a centre,
/// with feedback for resonance. Each pair of stages adds a notch.
pub struct Phaser {
    v: [f32; 8],
    rate: u32,
    lfo: Lfo,
    depth: Smooth,
    log_center: Smooth,
    feedback: Smooth,
    width: Smooth,
    mix: Smooth,
    /// Crossfade weights for the taps after 2, 4, 6, 8 and 12 stages.
    taps: [Smooth; 5],
    state: [[f32; MAX_STAGES]; 2],
    last: [f32; 2],
    /// The stages' coefficient for each side, worked out every [`CTL`]
    /// frames (an exp2 and a tan each) and glided to between.
    gg: [f32; 2],
    gg_step: [f32; 2],
    /// Frames until the next coefficient; whether it lands at once (after
    /// a reset) instead of gliding.
    ctl: u32,
    land: bool,
}

/// Frames between the Phaser's coefficient updates: every 0.36 ms at 44.1
/// kHz, far quicker than its sweep, with a straight glide between, so the
/// sweep stays smooth for a sixteenth of the cost.
const CTL: u32 = 16;

impl Default for Phaser {
    fn default() -> Self { Self::new() }
}

impl Phaser {
    pub fn new() -> Phaser {
        let v = defaults(&PHASER_PARAMS);
        let mut p = Phaser {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            depth: Smooth::new(v[2] / 100.0),
            log_center: Smooth::new(v[3].log2()),
            feedback: Smooth::new(v[5] / 100.0),
            width: Smooth::new(v[6] / 100.0),
            mix: Smooth::new(v[7] / 100.0),
            taps: [Smooth::new(0.0); 5],
            state: [[0.0; MAX_STAGES]; 2],
            last: [0.0; 2],
            gg: [0.0; 2],
            gg_step: [0.0; 2],
            ctl: 0,
            land: true,
        };
        p.stages();
        p.prepare(48_000, mpfree_node::MAX_BLOCK);
        p.reset();
        p
    }

    fn stages(&mut self) {
        let pick = self.v[4] as usize;
        for (k, t) in self.taps.iter_mut().enumerate() { t.target = if k == pick { 1.0 } else { 0.0 }; }
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PHASER_PARAMS.get(id as usize) else { return };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            phaser::DEPTH => self.depth.target = v / 100.0,
            phaser::CENTER => self.log_center.target = v.log2(),
            phaser::STAGES => self.stages(),
            phaser::FEEDBACK => self.feedback.target = v / 100.0,
            phaser::WIDTH => self.width.target = v / 100.0,
            phaser::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let fs = self.rate as f32;
        let top = 0.45 * fs;
        for i in 0..l.len() {
            let depth = PHASE_OCTAVES * self.depth.next();
            let center = self.log_center.next();
            let fb = self.feedback.next();
            let off_r = 0.25 * self.width.next();
            let mix = self.mix.next();
            let mut w = [0.0f32; 5];
            for (w, t) in w.iter_mut().zip(&mut self.taps) { *w = t.next(); }
            // Stages past the last tap in use rest, cleared.
            let used = (0..5).rev().find(|&k| w[k] != 0.0).map_or(0, |k| STAGE_COUNTS[k]);
            let ph = self.lfo.next();
            if self.ctl == 0 {
                // The coefficient now; the stages glide to it over the next
                // CTL frames (it lands at once after a reset). The counter
                // runs on across blocks, so any block size sounds the same.
                self.ctl = CTL;
                for ch in 0..2 {
                    let off = if ch == 0 { 0.0 } else { off_r };
                    let hz = (center + depth * sine(wrap(ph + off))).exp2().min(top);
                    let g = (PI * hz / fs).tan();
                    let to = g / (1.0 + g);
                    if self.land {
                        self.gg[ch] = to;
                    }
                    self.gg_step[ch] = (to - self.gg[ch]) / CTL as f32;
                }
                self.land = false;
            }
            self.ctl -= 1;
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for ch in 0..2 {
                let gg = self.gg[ch];
                self.gg[ch] += self.gg_step[ch];
                let mut u = x[ch] + fb * self.last[ch];
                let mut out = 0.0;
                let mut tap = 0;
                let st = &mut self.state[ch];
                for (k, s) in st.iter_mut().enumerate().take(used) {
                    // A trapezoidal one-pole low-pass; all-pass = 2 lp - in.
                    let v = (u - *s) * gg;
                    let lp = v + *s;
                    *s = flush(lp + v);
                    u = 2.0 * lp - u;
                    if k + 1 == STAGE_COUNTS[tap] {
                        out += w[tap] * u;
                        tap += 1;
                    }
                }
                for s in &mut st[used..] { *s = 0.0; }
                self.last[ch] = flush(out);
                y[ch] = lerp(x[ch], out, mix);
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }
}

impl Node for Phaser {
    fn info(&self) -> &'static NodeInfo { &PHASER_INFO }
    fn params(&self) -> &'static [ParamInfo] { &PHASER_PARAMS }
    fn set_param(&mut self, id: u16, value: f32) { self.apply(id, value); }
    fn param(&self, id: u16) -> f32 { self.v.get(id as usize).copied().unwrap_or(f32::NAN) }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        smooth_all(&mut [&mut self.depth, &mut self.log_center, &mut self.feedback, &mut self.width, &mut self.mix], self.rate, 30.0);
        for t in &mut self.taps { t.set_time(self.rate, 20.0); }
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
    }
    fn reset(&mut self) {
        for s in [&mut self.depth, &mut self.log_center, &mut self.feedback, &mut self.width, &mut self.mix] { s.snap(s.target); }
        for t in &mut self.taps { t.snap(t.target); }
        self.state = [[0.0; MAX_STAGES]; 2];
        self.last = [0.0; 2];
        self.gg_step = [0.0; 2];
        self.ctl = 0;
        self.land = true;
        self.lfo.reset();
    }
    fn process(&mut self, ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        self.lfo.begin(ctx, self.v[1] as usize);
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| if let NodeEvent::Param { id, value } = *ev { self.apply(id, value) }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn tail_frames(&self) -> u32 {
        // Each stage holds about 1 / (2 pi f) seconds at the lowest point
        // of the sweep; the feedback rings that loop down 80 dB.
        let low = (self.v[3] * (-PHASE_OCTAVES * self.v[2] / 100.0).exp2()).max(10.0);
        let stages = STAGE_COUNTS[self.v[4] as usize] as f32;
        let lp = stages / (2.0 * PI * low);
        let fb = (self.v[5] / 100.0).abs();
        let rings = if fb > 0.0 { (1e-4f32).ln() / fb.ln() } else { 0.0 };
        // At least 8 time constants of the slowest stage, plus the rings.
        ((8.0 / (2.0 * PI * low) + lp * rings) * self.rate as f32) as u32 + 2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lfo_sine_is_a_sine() {
        for i in 0..1000 {
            let p = i as f32 / 1000.0;
            assert!((sine(p) - (2.0 * PI * p).sin()).abs() < 0.002, "{p}");
        }
    }

    #[test]
    fn a_mod_line_reads_whole_and_fractional_delays() {
        let mut m = ModLine::new(16);
        for k in 0..40 { m.push(k as f32); }
        assert_eq!(m.read(1.0), 38.0);
        assert_eq!(m.read(5.0), 34.0);
        // A ramp interpolates exactly.
        assert!((m.read(2.25) - 36.75).abs() < 1e-5);
    }

    #[test]
    fn a_synced_lfo_lands_on_the_bar_when_the_transport_starts() {
        let mut lfo = Lfo::default();
        let ctx = Ctx { bpm: 120.0, beats: 6.0, playing: true, ..Ctx::new(48_000, 0) };
        // One cycle a bar, starting two beats into the bar: half a cycle.
        lfo.begin(&ctx, 5);
        assert!((lfo.phase - 0.5).abs() < 1e-12);
        lfo.set(1.0, 5, 48_000);
        // A bar at 120 BPM is 2 s.
        assert!((lfo.inc * 96_000.0 - 1.0).abs() < 1e-9);
        // Already playing: no jump.
        lfo.phase = 0.1;
        lfo.begin(&ctx, 5);
        assert_eq!(lfo.phase, 0.1);
    }
}
