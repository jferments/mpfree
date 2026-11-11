//! Drums: a drum synth with a model per pad. Sixteen pads, each one of
//! eleven models (kick, snare, clap, closed and open hat, tom, rim, cowbell,
//! cymbal, zap, FM perc) with the same fourteen knobs: tune, decay, tone,
//! snap, noise, pitch sweep and its time, drive, level, pan, velocity to
//! tone and to level, and a choke group.
//!
//! Notes play pads: pad 1 is the Pad 1 note parameter (36 by default) and
//! each pad after it a semitone higher. Other notes are ignored, and so are
//! note-offs: every hit is a one-shot that rings for its decay. A new hit on
//! a pad fades its last hit out over 3 ms, and a hit in a choke group fades
//! out the group's other pads (an open hat closed by the closed hat).
//!
//! A hit takes its pad's settings when it starts, as drum machines do, so a
//! knob turn changes the next hit. The kit Level moves at once (smoothed).
//!
//! Every part of a voice runs every frame, so the output does not depend on
//! how the host cuts blocks. Nothing here allocates after `prepare`.

use mpfree_node::{Ctx, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed};

use crate::drum_params::*;
use crate::dsp::{drive_sq, osc, Rng, Wave};
use crate::params::Preset;

/// Hit slots: one per pad plus room for fading hits.
pub const POOL: usize = 32;
/// A replaced or choked hit fades out over this long.
const KILL_MS: f32 = 3.0;
/// AllOff fades every hit out over this long.
const ALL_OFF_MS: f32 = 30.0;
/// A hit ends when every envelope is below this (-80 dB).
const SILENT: f32 = 1e-4;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.drums",
    name: "Drums",
    kind: Kind::Instrument,
};

/// The 808's six hat oscillators, in Hz at Tune 0.
const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
/// Clap: the handclaps' starts after the hit, in seconds, then the tail.
const CLAP_BURSTS: [f32; 4] = [0.0, 0.0105, 0.0215, 0.0305];

/// A sine of `ph` turns (any value), within about 0.001.
#[inline]
pub fn sin_turns(ph: f32) -> f32 {
    // sin(2 pi x) = -sin(pi s) with s = 2x - 1 in -1..1; a parabola for
    // sin(pi s), refined.
    let x = ph - floor(ph);
    let s = 2.0 * x - 1.0;
    let p = 4.0 * s * (1.0 - s.abs());
    -(0.225 * (p * p.abs() - p) + p)
}

/// `x.floor()`, the same value for every input: a positive value below
/// 2^23 (every phase here in practice) truncates through an i32, which is
/// its floor, instead of the library call `floorf` is on ARMv7.
#[inline]
fn floor(x: f32) -> f32 {
    if x > 0.0 && x < 8_388_608.0 {
        x as i32 as f32
    } else {
        x.floor()
    }
}

/// A per-frame multiplier that falls 60 dB in `secs`.
#[inline]
fn t60(secs: f32, rate: f32) -> f32 {
    (-6.907_755 / (secs.max(1e-4) * rate)).exp()
}

/// A trapezoidal state-variable filter with every output (Simper).
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum Out {
    #[default]
    Off,
    Lp,
    Bp,
    Hp,
}

impl Svf {
    fn set(&mut self, fc: f32, q: f32, rate: f32) {
        let fc = fc.clamp(20.0, 0.45 * rate);
        let g = (std::f32::consts::PI * fc / rate).tan();
        self.k = 1.0 / q.max(0.3);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    #[inline]
    fn run(&mut self, out: Out, x: f32) -> f32 {
        if out == Out::Off {
            return x;
        }
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match out {
            Out::Lp => v2,
            Out::Bp => v1 * self.k,
            _ => x - self.k * v1 - v2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
enum State {
    #[default]
    Free,
    On,
    /// Fading out (replaced, choked or AllOff), then free.
    Kill,
}

/// One hit. Everything a model needs; a model leaves what it doesn't use
/// at zero.
#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    state: State,
    pad: u8,
    choke: u8,
    age: u64,
    gain_l: f32,
    gain_r: f32,
    kill: f32,
    kill_step: f32,
    // Body: up to two sines with a pitch sweep, and FM.
    f0: f32,
    ph_a: f32,
    ph_b: f32,
    ratio_b: f32,
    lvl_a: f32,
    lvl_b: f32,
    body: f32,
    body_mul: f32,
    sweep: f32,
    pitch: f32,
    pitch_mul: f32,
    shape: f32,
    fm_ph: f32,
    fm_ratio: f32,
    fm_index: f32,
    // Metal: square oscillators.
    n_metal: usize,
    ph_m: [f32; 6],
    dt_m: [f32; 6],
    metal_lvl: f32,
    // Noise and metal share the filters and the noise envelope.
    rng: Rng,
    noise_lvl: f32,
    noise: f32,
    noise_mul: f32,
    f1: Svf,
    o1: Out,
    f2: Svf,
    o2: Out,
    // Clap: frames since the hit, the bursts, the tail.
    clap: bool,
    t: u32,
    burst: f32,
    burst_mul: f32,
    next_burst: usize,
    // Snap: a short noise click.
    click: f32,
    click_mul: f32,
    click_lvl: f32,
    drive: f32,
    /// What the drive stage takes, from `drive`: the gain, its square root
    /// and the amount.
    drive_gain: f32,
    drive_sq: f32,
    drive_amt: f32,
    /// Frames since the hit: the silence check runs every 64, so when a
    /// hit ends doesn't depend on the block size.
    frames: u32,
}

impl Voice {
    #[inline]
    fn tick(&mut self, rate: f32) -> f32 {
        let inv = 1.0 / rate;
        let mut out = 0.0;
        if self.lvl_a > 0.0 {
            // No sweep: exp2(0) is exactly 1, so the pitch is f0 as it is.
            let hz = if self.sweep == 0.0 && self.pitch.is_finite() {
                self.f0
            } else {
                self.f0 * (self.sweep * self.pitch * (1.0 / 12.0)).exp2()
            };
            self.pitch *= self.pitch_mul;
            self.ph_a += hz * inv;
            if self.ph_a >= 1.0 {
                self.ph_a -= 1.0;
            }
            let mut s = if self.fm_index > 0.0 {
                self.fm_ph += hz * self.fm_ratio * inv;
                if self.fm_ph >= 1.0 {
                    self.fm_ph -= floor(self.fm_ph);
                }
                let ix = self.fm_index * (0.15 + 0.85 * self.pitch);
                sin_turns(self.ph_a + ix * sin_turns(self.fm_ph) * 0.159_155)
            } else {
                sin_turns(self.ph_a)
            };
            if self.shape > 0.0 {
                s = s * (1.0 + self.shape) / (1.0 + self.shape * s.abs());
            }
            let mut b = self.lvl_a * s;
            if self.lvl_b > 0.0 {
                self.ph_b += hz * self.ratio_b * inv;
                if self.ph_b >= 1.0 {
                    self.ph_b -= floor(self.ph_b);
                }
                b += self.lvl_b * sin_turns(self.ph_b);
            }
            out += b * self.body;
            self.body *= self.body_mul;
        }
        if self.noise_lvl > 0.0 || self.n_metal > 0 {
            let mut x = 0.0;
            if self.n_metal > 0 {
                let mut m = 0.0;
                for k in 0..self.n_metal {
                    m += osc(Wave::Pulse, &mut self.ph_m[k], self.dt_m[k], 0.5);
                }
                x += m * self.metal_lvl;
            }
            if self.noise_lvl > 0.0 {
                x += self.rng.bipolar() * self.noise_lvl;
            }
            let mut env = self.noise;
            self.noise *= self.noise_mul;
            if self.clap {
                if self.next_burst < CLAP_BURSTS.len()
                    && self.t as f32 >= CLAP_BURSTS[self.next_burst] * rate
                {
                    self.burst = 1.0;
                    self.next_burst += 1;
                }
                env = env * 0.35 + self.burst;
                self.burst *= self.burst_mul;
                self.t = self.t.saturating_add(1);
            }
            let y = self.f2.run(self.o2, self.f1.run(self.o1, x));
            out += y * env;
        }
        if self.click > SILENT {
            out += self.rng.bipolar() * self.click * self.click_lvl;
            self.click *= self.click_mul;
        }
        if self.drive > 0.0 {
            // drive(out, 1 + 9 drive, sqrt(drive)) with its square roots
            // worked out at the hit.
            out = drive_sq(out, self.drive_gain, self.drive_sq, self.drive_amt);
        }
        if self.state == State::Kill {
            out *= self.kill;
            self.kill -= self.kill_step;
            if self.kill <= 0.0 {
                self.state = State::Free;
            }
        }
        self.frames = self.frames.wrapping_add(1);
        if self.frames.is_multiple_of(64) && self.state == State::On && self.silent() {
            self.state = State::Free;
        }
        out
    }

    fn silent(&self) -> bool {
        self.body * self.lvl_a.max(self.lvl_b) < SILENT
            && self.noise * (self.noise_lvl + self.metal_lvl) < SILENT
            && self.click * self.click_lvl < SILENT
            && (!self.clap || (self.next_burst >= CLAP_BURSTS.len() && self.burst < SILENT))
    }
}

/// A hit of `pad` from its fields (`g` reads one field of the pad), at
/// `rate`: what [`Drums`] plays and [`hit_shape`] draws.
fn build(
    g: impl Fn(u16) -> f32,
    rate: f32,
    pad: usize,
    velocity: u8,
    seed: u64,
    clock: u64,
) -> Voice {
    use crate::drum_params::model::*;
    let vel = f32::from(velocity.min(127)) / 127.0;
    let model = g(field::MODEL) as u8;
    let ratio = (g(field::TUNE) / 12.0).exp2();
    let decay = g(field::DECAY) * 0.001;
    let tone =
        (g(field::TONE) / 100.0 - g(field::VEL_TONE) / 100.0 * (1.0 - vel) * 0.6).clamp(0.0, 1.0);
    let snap = g(field::SNAP) / 100.0;
    let nz = g(field::NOISE) / 100.0;
    let vl = g(field::VEL_LEVEL) / 100.0;
    let amp = (1.0 - vl + vl * vel * vel)
        * if g(field::LEVEL) <= -60.0 {
            0.0
        } else {
            10f32.powf(g(field::LEVEL) / 20.0)
        };
    let pan = g(field::PAN) / 100.0;

    let mut v = Voice {
        state: State::On,
        pad: pad as u8,
        choke: g(field::CHOKE) as u8,
        age: clock,
        kill: 1.0,
        rng: Rng::new(seed),
        body: 1.0,
        body_mul: t60(decay, rate),
        noise: 1.0,
        noise_mul: t60(decay, rate),
        pitch: 1.0,
        pitch_mul: (-4.6 / (g(field::SWEEP_TIME) * 0.001 * rate).max(1.0)).exp(),
        sweep: g(field::SWEEP),
        click: 1.0,
        click_mul: t60(0.004, rate),
        click_lvl: snap * 0.5,
        drive: g(field::DRIVE) / 100.0,
        ..Voice::default()
    };
    v.drive_gain = 1.0 + v.drive * 9.0;
    v.drive_sq = v.drive_gain.sqrt();
    v.drive_amt = v.drive.sqrt();
    // How loud each model is at full velocity, so the defaults peak
    // near -12 dBFS.
    let norm;
    match model {
        KICK => {
            v.f0 = 50.0 * ratio;
            v.lvl_a = 1.0;
            v.shape = tone * 4.0;
            v.noise_lvl = nz * 0.3;
            v.noise_mul = t60(decay * 0.3, rate);
            v.f1.set(200.0 + tone * 4000.0, 0.7, rate);
            v.o1 = Out::Lp;
            v.click_lvl = snap * 0.6;
            v.click_mul = t60(0.003, rate);
            norm = 0.56;
        }
        SNARE => {
            v.f0 = 185.0 * ratio;
            v.lvl_a = 0.65 * (1.0 - nz * 0.4);
            v.ratio_b = 1.56;
            v.lvl_b = 0.45 * (1.0 - nz * 0.4);
            v.body_mul = t60(decay * 0.6, rate);
            v.noise_lvl = nz;
            v.f1.set(1500.0 + tone * 6000.0, 0.6, rate);
            v.o1 = Out::Bp;
            v.f2.set(700.0, 0.7, rate);
            v.o2 = Out::Hp;
            norm = 0.38;
        }
        CLAP => {
            v.clap = true;
            v.noise_lvl = 1.0;
            v.burst_mul = t60(0.004 + (1.0 - snap) * 0.012, rate);
            v.f1.set(800.0 + tone * 2400.0, 1.2, rate);
            v.o1 = Out::Bp;
            v.f2.set(400.0, 0.7, rate);
            v.o2 = Out::Hp;
            v.click_lvl = 0.0;
            norm = 0.75;
        }
        CLOSED_HAT | OPEN_HAT | CYMBAL => {
            v.n_metal = 6;
            for (k, hz) in METAL.iter().enumerate() {
                v.dt_m[k] = hz * ratio / rate;
                v.ph_m[k] = (k as f32 * 0.37) % 1.0;
            }
            v.metal_lvl = (1.0 - nz) / 3.0;
            v.noise_lvl = nz * 0.6;
            let (bp, hp) = if model == CYMBAL {
                (5000.0 + tone * 5000.0, 3500.0)
            } else {
                (7000.0 + tone * 5000.0, 6000.0 + tone * 2000.0)
            };
            v.f1.set(bp * ratio.sqrt(), 0.9, rate);
            v.o1 = Out::Bp;
            v.f2.set(hp, 0.7, rate);
            v.o2 = Out::Hp;
            v.click_lvl = snap * 0.25;
            norm = if model == CYMBAL { 0.9 } else { 1.6 };
        }
        TOM => {
            v.f0 = 120.0 * ratio;
            v.lvl_a = 1.0;
            v.shape = tone * 1.5;
            v.noise_lvl = nz * 0.5;
            v.noise_mul = t60(decay * 0.25, rate);
            v.f1.set(v.f0 * 3.0, 1.0, rate);
            v.o1 = Out::Bp;
            norm = 0.3;
        }
        RIM => {
            v.f0 = 500.0 * ratio;
            v.lvl_a = 0.6;
            v.ratio_b = 3.3;
            v.lvl_b = 0.4;
            v.noise_lvl = nz * 0.8;
            v.f1.set(2000.0 + tone * 3000.0, 2.0, rate);
            v.o1 = Out::Bp;
            v.click_lvl = snap * 0.8;
            v.click_mul = t60(0.002, rate);
            norm = 0.36;
        }
        COWBELL => {
            v.n_metal = 2;
            v.dt_m[0] = 540.0 * ratio / rate;
            v.dt_m[1] = 800.0 * ratio / rate;
            v.metal_lvl = 0.5;
            v.noise_lvl = nz * 0.3;
            v.f1.set(2640.0 * ratio * (0.7 + 0.6 * tone), 1.5, rate);
            v.o1 = Out::Bp;
            norm = 0.55;
        }
        ZAP => {
            v.f0 = 90.0 * ratio;
            v.lvl_a = 1.0;
            v.shape = tone * 3.0;
            v.noise_lvl = nz * 0.3;
            v.f1.set(3000.0, 0.7, rate);
            v.o1 = Out::Hp;
            norm = 0.25;
        }
        _ => {
            // FM perc: a sine modulated at an inharmonic ratio; Tone sets
            // the index, which falls with the sweep time.
            v.f0 = 220.0 * ratio;
            v.lvl_a = 1.0;
            v.fm_ratio = 1.41;
            v.fm_index = 0.5 + tone * 8.0 + snap * 2.0;
            v.click_lvl = 0.0;
            v.noise_lvl = nz * 0.3;
            v.f1.set(5000.0, 0.7, rate);
            v.o1 = Out::Hp;
            norm = 0.25;
        }
    }
    let a = amp * norm;
    v.gain_l = a * (1.0 - pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
    v.gain_r = a * (1.0 + pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
    v
}

/// What a full-velocity hit of one pad does over time, for drawing it
/// (`mpfree-synth-ui`). Each part is (its level in the mix before the
/// filters, the share of it left after each frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitShape {
    /// The pad's model ("Kick").
    pub model: &'static str,
    /// The body's pitch once the sweep has settled, in Hz; 0 for a model
    /// with no body (hats, clap, cowbell, cymbal).
    pub hz: f32,
    /// The body: the tuned sines (or FM).
    pub body: (f32, f32),
    /// The noise and metal, through the pad's filters.
    pub noise: (f32, f32),
    /// The snap's click.
    pub click: (f32, f32),
    /// The pitch sweep: semitones above `hz` at the start, and the share
    /// of it left after each frame.
    pub sweep: (f32, f32),
    /// A clap: when each handclap starts, in seconds, and the share of a
    /// burst left after each frame. The noise envelope carries the tail
    /// at a third of the level.
    pub bursts: Option<(&'static [f32], f32)>,
    /// Drive, 0 to 1.
    pub drive: f32,
}

/// The shape of a full-velocity hit of `pad` (0 to 15) at `rate`, with the
/// values `value` gives by parameter id. Built by the same code as the
/// hits [`Drums`] plays.
pub fn hit_shape(value: impl Fn(u16) -> f32, pad: usize, rate: u32) -> HitShape {
    let pad = pad.min(PADS - 1);
    let get = |f: u16| value(pad_id(pad, f));
    let model = MODELS[get(field::MODEL) as usize % MODELS.len()];
    let v = build(get, rate as f32, pad, 127, 0, 0);
    let body = v.lvl_a > 0.0;
    HitShape {
        model,
        hz: if body { v.f0 } else { 0.0 },
        body: (if body { v.lvl_a + v.lvl_b } else { 0.0 }, v.body_mul),
        noise: (
            v.noise_lvl + v.metal_lvl * (v.n_metal as f32).sqrt(),
            v.noise_mul,
        ),
        click: (v.click_lvl, v.click_mul),
        sweep: (if body { v.sweep } else { 0.0 }, v.pitch_mul),
        bursts: v.clap.then_some((&CLAP_BURSTS[..], v.burst_mul)),
        drive: v.drive,
    }
}

/// The drum synth. See the module docs.
pub struct Drums {
    rate: u32,
    values: [f32; ID_SPACE],
    voices: [Voice; POOL],
    clock: u64,
    rng: Rng,
    level: Smooth,
    gains: Vec<f32>,
    hits: u64,
}

impl Drums {
    pub fn new(rate: u32) -> Drums {
        let mut values = [0.0; ID_SPACE];
        for p in PARAMS.iter() {
            values[p.id as usize] = p.default;
        }
        let mut d = Drums {
            rate,
            values,
            voices: [Voice::default(); POOL],
            clock: 0,
            rng: Rng::new(0x5EED_D2B1),
            level: Smooth::new(1.0),
            gains: Vec::new(),
            hits: 0,
        };
        d.prepare(rate, mpfree_node::MAX_BLOCK);
        d
    }

    /// Loads a factory kit: the Init kit, then the kit's values. Hits ring on.
    pub fn load_preset(&mut self, kit: &Preset) {
        for p in PARAMS.iter() {
            if !p.readonly {
                self.set_param(p.id, p.default);
            }
        }
        for &(id, v) in kit.values {
            self.set_param(id, v);
        }
    }

    pub fn presets() -> &'static [Preset] {
        KITS
    }

    /// Hits sounding or fading.
    pub fn active_voices(&self) -> usize {
        self.voices
            .iter()
            .filter(|v| v.state != State::Free)
            .count()
    }

    /// The model name on pad `pad` (0 to 15), for pad labels.
    pub fn pad_name(&self, pad: usize) -> &'static str {
        MODELS[self.v(pad_id(pad.min(PADS - 1), field::MODEL)) as usize % MODELS.len()]
    }

    /// The pad a note plays, if any.
    pub fn pad_of(&self, note: u8) -> Option<usize> {
        let p = i32::from(note) - self.v(BASE_NOTE) as i32;
        (0..PADS as i32).contains(&p).then_some(p as usize)
    }

    #[inline]
    fn v(&self, id: u16) -> f32 {
        self.values[id as usize]
    }
    fn pv(&self, pad: usize, f: u16) -> f32 {
        self.v(pad_id(pad, f))
    }

    fn level_target(&self) -> f32 {
        let db = self.v(LEVEL);
        if db <= -60.0 {
            0.0
        } else {
            10f32.powf(db / 20.0)
        }
    }

    fn kill_where(&mut self, ms: f32, mut which: impl FnMut(&Voice) -> bool) {
        let step = 1.0 / (ms * 0.001 * self.rate as f32).max(1.0);
        for v in self.voices.iter_mut() {
            if v.state == State::On && which(v) {
                v.state = State::Kill;
                v.kill = 1.0;
                v.kill_step = step;
            }
        }
    }

    fn hit(&mut self, pad: usize, velocity: u8) {
        if velocity == 0 {
            return;
        }
        let choke = self.pv(pad, field::CHOKE) as u8;
        self.kill_where(KILL_MS, |v| {
            v.pad as usize == pad || (choke > 0 && v.choke == choke)
        });
        self.clock += 1;
        self.hits += 1;
        let slot = self
            .voices
            .iter()
            .position(|v| v.state == State::Free)
            .or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.state == State::Kill)
                    .min_by_key(|(_, v)| v.age)
                    .map(|(i, _)| i)
            })
            .unwrap_or_else(|| {
                self.voices
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, v)| v.age)
                    .map_or(0, |(i, _)| i)
            });
        let seed = self.rng.next_u64();
        let v = self.setup(pad, velocity, seed);
        self.voices[slot] = v;
    }

    /// Builds a hit of `pad` from its settings.
    fn setup(&self, pad: usize, velocity: u8, seed: u64) -> Voice {
        build(
            |f| self.pv(pad, f),
            self.rate as f32,
            pad,
            velocity,
            seed,
            self.clock,
        )
    }

    fn apply(&mut self, ev: &NodeEvent) {
        match *ev {
            NodeEvent::NoteOn { note, velocity } => {
                if let Some(p) = self.pad_of(note) {
                    self.hit(p, velocity);
                }
            }
            NodeEvent::AllOff => self.kill_where(ALL_OFF_MS, |_| true),
            NodeEvent::Param { id, value } => self.set_param(id, value),
            NodeEvent::NoteOff { .. } | NodeEvent::Pressure { .. } | NodeEvent::Bend { .. } => {}
        }
    }

    fn run(&mut self, from: usize, to: usize, left: &mut [f32], right: &mut [f32]) {
        let rate = self.rate as f32;
        for g in &mut self.gains[from..to] {
            *g = self.level.next();
        }
        for v in self.voices.iter_mut() {
            if v.state == State::Free {
                continue;
            }
            for i in from..to {
                let x = v.tick(rate) * self.gains[i];
                left[i] += x * v.gain_l;
                right[i] += x * v.gain_r;
                if v.state == State::Free {
                    break;
                }
            }
        }
    }
}

impl Node for Drums {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }

    fn set_param(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.iter().find(|p| p.id == id) else {
            return;
        };
        if p.readonly {
            return;
        }
        self.values[id as usize] = p.clamp(value);
        if id == LEVEL {
            self.level.target = self.level_target();
        }
    }

    fn param(&self, id: u16) -> f32 {
        if id == METER_VOICES {
            return self.active_voices() as f32;
        }
        match PARAMS.iter().find(|p| p.id == id) {
            Some(_) => self.values[id as usize],
            None => f32::NAN,
        }
    }

    fn prepare(&mut self, rate: u32, max_block: usize) {
        self.rate = rate;
        self.gains = vec![0.0; max_block.max(1)];
        self.level.set_time(rate, 10.0);
        self.level.snap(self.level_target());
        self.reset();
    }

    fn reset(&mut self) {
        for v in self.voices.iter_mut() {
            v.state = State::Free;
        }
        self.level.snap(self.level_target());
    }

    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len()).min(self.gains.len());
        let mut at = 0usize;
        let mut i = 0usize;
        while at < len {
            while i < events.len() && (events[i].offset as usize) <= at {
                let ev = events[i].ev;
                self.apply(&ev);
                i += 1;
            }
            let next = events.get(i).map_or(len, |e| (e.offset as usize).min(len));
            if next > at {
                self.run(at, next, left, right);
            }
            at = next;
        }
        for e in &events[i..] {
            let ev = e.ev;
            self.apply(&ev);
        }
    }

    fn tail_frames(&self) -> u32 {
        (5.2 * self.rate as f32) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::floor;

    #[test]
    fn the_quick_floor_matches_floorf_for_every_input() {
        let same = |x: f32| floor(x).to_bits() == x.floor().to_bits() || x.is_nan();
        for x in [
            0.0,
            -0.0,
            0.5,
            -0.5,
            1.0,
            8_388_607.5,
            8_388_608.0,
            f32::MIN_POSITIVE,
        ] {
            assert!(same(x) && same(-x), "{x:e}");
        }
        for bits in (0..=u32::MAX).step_by(997) {
            let x = f32::from_bits(bits);
            assert!(same(x), "{bits:#x}");
        }
    }
}
