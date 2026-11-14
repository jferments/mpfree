//! Movement effects: [`Tremolo`], [`AutoPan`], [`RingMod`] and
//! [`Rotary`].
//!
//! Tremolo, auto-pan and the ring modulator's sweep share the LFO in
//! [`crate::kit`]: free in Hz or synced to the tempo (1/32 to 4 bars,
//! triplets and dotted), six shapes, and a restart in time with the bar
//! when the transport starts. Every level that moves glides, so nothing
//! clicks when a knob turns under a loop.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{
    self, node_impl, param, Choice, Glide, Hz, Lfo, Line, Linear, Log, Ms, Percent, NONE,
};
use crate::svf::{Svf, SvfMode};
use crate::{defaults, fast_exp2, fast_tanh, lerp, Preset};

const SQRT2: f32 = std::f32::consts::SQRT_2;

// ---------------------------------------------------------------- tremolo

/// [`Tremolo`] parameter ids.
pub mod tremolo {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const SHAPE: u16 = 2;
    pub const DEPTH: u16 = 3;
    pub const WIDTH: u16 = 4;
    pub const MODE: u16 = 5;
}

const TREMOLO_MODES: &[&str] = &["Volume", "Harmonic"];
/// Where Harmonic mode splits the sound (the 1960s amps' split).
const HARMONIC_HZ: f32 = 700.0;

static TREMOLO_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.tremolo",
    name: "Tremolo",
    kind: Kind::Effect,
};
static TREMOLO_PARAMS: [ParamInfo; 6] = [
    param(0, "Rate", "Tremolo", 0.05, 20.0, 4.0, Hz, Log),
    param(
        1,
        "Sync",
        "Tremolo",
        0.0,
        13.0,
        0.0,
        NONE,
        Choice(kit::SYNCS),
    ),
    param(
        2,
        "Shape",
        "Tremolo",
        0.0,
        5.0,
        0.0,
        NONE,
        Choice(kit::SHAPES),
    ),
    param(3, "Depth", "Tremolo", 0.0, 100.0, 50.0, Percent, Linear),
    param(4, "Width", "Tremolo", 0.0, 100.0, 0.0, Percent, Linear),
    param(
        5,
        "Mode",
        "Tremolo",
        0.0,
        1.0,
        0.0,
        NONE,
        Choice(TREMOLO_MODES),
    ),
];

/// Tremolo factory presets.
pub static TREMOLO_PRESETS: &[Preset] = &[
    Preset {
        name: "Amp wobble",
        values: &[(tremolo::RATE, 5.5), (tremolo::DEPTH, 45.0)],
    },
    Preset {
        name: "Eighth chop",
        values: &[
            (tremolo::SYNC, 5.0),
            (tremolo::SHAPE, 2.0),
            (tremolo::DEPTH, 100.0),
        ],
    },
    Preset {
        name: "Stereo shimmer",
        values: &[
            (tremolo::RATE, 7.0),
            (tremolo::DEPTH, 60.0),
            (tremolo::WIDTH, 100.0),
        ],
    },
    Preset {
        name: "Harmonic vibe",
        values: &[
            (tremolo::RATE, 3.5),
            (tremolo::DEPTH, 70.0),
            (tremolo::MODE, 1.0),
        ],
    },
    Preset {
        name: "Slow swell",
        values: &[
            (tremolo::SYNC, 11.0),
            (tremolo::SHAPE, 1.0),
            (tremolo::DEPTH, 80.0),
        ],
    },
];

/// Volume moved by the LFO: a soft wobble, a hard square chop, or (with
/// Width) a stereo shimmer as the two sides move apart. Harmonic mode
/// splits the sound at 700 Hz (a 12 dB low-pass and what it leaves, gentle
/// slopes like the amps' own RC filters, summing back to the input) and
/// moves the lows and highs in opposite phase, the swirl of early-1960s
/// amps, with little change in loudness.
pub struct Tremolo {
    v: [f32; 6],
    rate: u32,
    lfo: Lfo,
    /// Harmonic mode's low band; the high band is what it leaves, so the
    /// two always sum back to the input.
    lows: Svf,
    depth: Smooth,
    width: Smooth,
}

impl Default for Tremolo {
    fn default() -> Self {
        Self::new()
    }
}

impl Tremolo {
    pub fn new() -> Tremolo {
        let v = defaults(&TREMOLO_PARAMS);
        let mut t = Tremolo {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            lows: Svf::default(),
            depth: Smooth::new(v[3] / 100.0),
            width: Smooth::new(v[4] / 100.0),
        };
        t.setup(48_000);
        t
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            tremolo::DEPTH => self.depth.target = v / 100.0,
            tremolo::WIDTH => self.width.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, ctx: &Ctx) {
        self.lfo.begin(ctx, self.v[1] as usize);
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        let shape = self.v[2] as usize;
        let harmonic = self.v[5] >= 0.5;
        for i in 0..l.len() {
            let p = self.lfo.step();
            let depth = self.depth.glide();
            let off = 0.5 * self.width.glide();
            let ml = self.lfo.shape(shape, p, 0.0);
            let mr = self.lfo.shape(shape, p, off);
            if harmonic {
                // Lows and highs swap places: one is up while the other
                // is down, so the level stays nearly even.
                let g = |m: f32| (1.0 - depth * (0.5 - 0.5 * m), 1.0 - depth * (0.5 + 0.5 * m));
                for (ch, (x, m)) in [(&mut l[i], ml), (&mut r[i], mr)].into_iter().enumerate() {
                    let lo = self.lows.run(ch, *x);
                    let hi = *x - lo;
                    let (gl, gh) = g(m);
                    *x = lo * gl + hi * gh;
                }
            } else {
                l[i] *= 1.0 - depth * (0.5 - 0.5 * ml);
                r[i] *= 1.0 - depth * (0.5 - 0.5 * mr);
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let q = std::f32::consts::FRAC_1_SQRT_2;
        self.lows.set(SvfMode::LowPass, HARMONIC_HZ, q, 0.0, rate);
        self.depth.set_time(rate, 20.0);
        self.width.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.lows.reset();
        self.depth.snap(self.depth.target);
        self.width.snap(self.width.target);
    }
    fn tail(&self) -> u32 {
        0
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Tremolo, TREMOLO_INFO, TREMOLO_PARAMS);

// ---------------------------------------------------------------- auto-pan

/// [`AutoPan`] parameter ids.
pub mod autopan {
    pub const RATE: u16 = 0;
    pub const SYNC: u16 = 1;
    pub const SHAPE: u16 = 2;
    pub const DEPTH: u16 = 3;
    pub const CENTRE: u16 = 4;
}

static AUTOPAN_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.autopan",
    name: "Auto-pan",
    kind: Kind::Effect,
};
static AUTOPAN_PARAMS: [ParamInfo; 5] = [
    param(0, "Rate", "Pan", 0.05, 20.0, 0.5, Hz, Log),
    param(1, "Sync", "Pan", 0.0, 13.0, 8.0, NONE, Choice(kit::SYNCS)),
    param(2, "Shape", "Pan", 0.0, 5.0, 0.0, NONE, Choice(kit::SHAPES)),
    param(3, "Depth", "Pan", 0.0, 100.0, 70.0, Percent, Linear),
    param(4, "Centre", "Pan", -100.0, 100.0, 0.0, Percent, Linear),
];

/// Auto-pan factory presets.
pub static AUTOPAN_PRESETS: &[Preset] = &[
    Preset {
        name: "Gentle drift",
        values: &[(autopan::SYNC, 12.0), (autopan::DEPTH, 40.0)],
    },
    Preset {
        name: "Hard ping",
        values: &[
            (autopan::SYNC, 5.0),
            (autopan::SHAPE, 2.0),
            (autopan::DEPTH, 100.0),
        ],
    },
    Preset {
        name: "Random space",
        values: &[
            (autopan::SYNC, 3.0),
            (autopan::SHAPE, 5.0),
            (autopan::DEPTH, 80.0),
        ],
    },
];

/// The sound moved between the speakers by the LFO, at constant power, so
/// it keeps its loudness as it travels.
pub struct AutoPan {
    v: [f32; 5],
    rate: u32,
    lfo: Lfo,
    depth: Smooth,
    centre: Smooth,
}

impl Default for AutoPan {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoPan {
    pub fn new() -> AutoPan {
        let v = defaults(&AUTOPAN_PARAMS);
        let mut a = AutoPan {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            depth: Smooth::new(v[3] / 100.0),
            centre: Smooth::new(v[4] / 100.0),
        };
        a.setup(48_000);
        a
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            autopan::DEPTH => self.depth.target = v / 100.0,
            autopan::CENTRE => self.centre.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, ctx: &Ctx) {
        self.lfo.begin(ctx, self.v[1] as usize);
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        let shape = self.v[2] as usize;
        for i in 0..l.len() {
            let p = self.lfo.step();
            let pos = (self.centre.glide() + self.depth.glide() * self.lfo.shape(shape, p, 0.0))
                .clamp(-1.0, 1.0);
            // A quarter cycle from hard left (0) to hard right (0.25):
            // both sides at unity in the middle.
            let a = (pos + 1.0) * 0.125;
            l[i] *= kit::cosine(a) * SQRT2;
            r[i] *= kit::sine(a) * SQRT2;
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.depth.set_time(rate, 20.0);
        self.centre.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.depth.snap(self.depth.target);
        self.centre.snap(self.centre.target);
    }
    fn tail(&self) -> u32 {
        0
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(AutoPan, AUTOPAN_INFO, AUTOPAN_PARAMS);

// ---------------------------------------------------------------- ring mod

/// [`RingMod`] parameter ids.
pub mod ringmod {
    pub const FREQ: u16 = 0;
    pub const WAVE: u16 = 1;
    pub const SWEEP: u16 = 2;
    pub const LFO_RATE: u16 = 3;
    pub const LFO_SYNC: u16 = 4;
    pub const MIX: u16 = 5;
}

const WAVES: &[&str] = &["Sine", "Triangle", "Square"];

static RINGMOD_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.ringmod",
    name: "Ring mod",
    kind: Kind::Effect,
};
static RINGMOD_PARAMS: [ParamInfo; 6] = [
    param(0, "Freq", "Carrier", 20.0, 5000.0, 440.0, Hz, Log),
    param(1, "Wave", "Carrier", 0.0, 2.0, 0.0, NONE, Choice(WAVES)),
    param(
        2,
        "Sweep",
        "Carrier",
        0.0,
        4.0,
        0.0,
        mpfree_node::Unit::Octaves,
        Linear,
    ),
    param(3, "LFO rate", "Carrier", 0.05, 20.0, 0.5, Hz, Log),
    param(
        4,
        "LFO sync",
        "Carrier",
        0.0,
        13.0,
        0.0,
        NONE,
        Choice(kit::SYNCS),
    ),
    param(5, "Mix", "Carrier", 0.0, 100.0, 100.0, Percent, Linear),
];

/// Ring mod factory presets.
pub static RINGMOD_PRESETS: &[Preset] = &[
    Preset {
        name: "Robot voice",
        values: &[(ringmod::FREQ, 90.0)],
    },
    Preset {
        name: "Metal bell",
        values: &[(ringmod::FREQ, 1250.0)],
    },
    Preset {
        name: "Sci-fi sweep",
        values: &[
            (ringmod::FREQ, 300.0),
            (ringmod::SWEEP, 2.5),
            (ringmod::LFO_SYNC, 10.0),
        ],
    },
    Preset {
        name: "Square buzz",
        values: &[(ringmod::FREQ, 55.0), (ringmod::WAVE, 2.0)],
    },
];

/// The input multiplied by a carrier tone: clangs, bells and robot voices.
/// The carrier can sweep with an LFO (up to four octaves).
pub struct RingMod {
    v: [f32; 6],
    rate: u32,
    lfo: Lfo,
    /// The carrier frequency as log2 Hz, so a glide is even in pitch.
    pitch: Smooth,
    sweep: Smooth,
    mix: Smooth,
    phase: f32,
}

impl Default for RingMod {
    fn default() -> Self {
        Self::new()
    }
}

impl RingMod {
    pub fn new() -> RingMod {
        let v = defaults(&RINGMOD_PARAMS);
        let mut m = RingMod {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            pitch: Smooth::new(v[0].log2()),
            sweep: Smooth::new(v[2]),
            mix: Smooth::new(v[5] / 100.0),
            phase: 0.0,
        };
        m.setup(48_000);
        m
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            ringmod::FREQ => self.pitch.target = v.log2(),
            ringmod::SWEEP => self.sweep.target = v,
            ringmod::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, ctx: &Ctx) {
        self.lfo.begin(ctx, self.v[4] as usize);
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.set(self.v[3], self.v[4] as usize, self.rate);
        let wave = self.v[1] as usize;
        let per = 1.0 / self.rate as f32;
        let top = 0.45 * self.rate as f32;
        for i in 0..l.len() {
            let p = self.lfo.step();
            let sweep = self.sweep.glide() * (0.5 + 0.5 * kit::sine(p));
            let hz = fast_exp2(self.pitch.glide() + sweep).min(top);
            self.phase = kit::wrap(self.phase + hz * per);
            let dt = hz * per;
            let c = match wave {
                1 => triangle(self.phase, dt),
                2 => square(self.phase, dt),
                _ => kit::sine(self.phase),
            };
            let mix = self.mix.glide();
            l[i] = lerp(l[i], l[i] * c, mix);
            r[i] = lerp(r[i], r[i] * c, mix);
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.pitch.set_time(rate, 30.0);
        self.sweep.set_time(rate, 30.0);
        self.mix.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.phase = 0.0;
        self.pitch.snap(self.pitch.target);
        self.sweep.snap(self.sweep.target);
        self.mix.snap(self.mix.target);
    }
    fn tail(&self) -> u32 {
        0
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(RingMod, RINGMOD_INFO, RINGMOD_PARAMS);

/// The PolyBLEP step correction at phase `t` for a step of +2 at phase 0
/// (`dt` is the phase step per frame).
#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        2.0 * x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + 2.0 * x + 1.0
    } else {
        0.0
    }
}

/// The PolyBLAMP corner correction at phase `t` for a unit change of
/// slope (per frame) at phase 0.
#[inline]
fn blamp(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt - 1.0;
        -x * x * x / 3.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt + 1.0;
        x * x * x / 3.0
    } else {
        0.0
    }
}

/// A square wave (+1 then -1) with its steps band-limited (PolyBLEP), so a
/// high carrier doesn't alias.
#[inline]
pub(crate) fn square(t: f32, dt: f32) -> f32 {
    let dt = dt.clamp(1e-6, 0.5);
    let naive = if t < 0.5 { 1.0 } else { -1.0 };
    naive + blep(t, dt) - blep(kit::wrap(t + 0.5), dt)
}

/// A triangle wave (-1 at phase 0, +1 at a half) with its corners
/// band-limited (PolyBLAMP).
#[inline]
pub(crate) fn triangle(t: f32, dt: f32) -> f32 {
    let dt = dt.clamp(1e-6, 0.5);
    let naive = 1.0 - 4.0 * (t - 0.5).abs();
    // The slope turns from -4 to +4 at phase 0 and back at a half: a change
    // of 8 per cycle, 8 dt per frame.
    naive + 8.0 * dt * (blamp(t, dt) - blamp(kit::wrap(t + 0.5), dt))
}

// ---------------------------------------------------------------- rotary

/// [`Rotary`] parameter ids.
pub mod rotary {
    pub const SPEED: u16 = 0;
    pub const RAMP: u16 = 1;
    pub const BALANCE: u16 = 2;
    pub const WIDTH: u16 = 3;
    pub const DRIVE: u16 = 4;
    pub const MIX: u16 = 5;
}

const SPEEDS: &[&str] = &["Stop", "Slow", "Fast"];
/// Rotations per second for Stop, Slow and Fast.
/// (Horn about 48 and 400 rpm, drum about 40 and 340 rpm.)
const HORN_HZ: [f32; 3] = [0.0, 0.8, 6.7];
const DRUM_HZ: [f32; 3] = [0.0, 0.67, 5.7];
/// The horn spins up faster than the heavy drum.
const HORN_RAMP: f32 = 0.25;
const CROSSOVER_HZ: f32 = 800.0;

static ROTARY_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.rotary",
    name: "Rotary",
    kind: Kind::Effect,
};
static ROTARY_PARAMS: [ParamInfo; 6] = [
    param(0, "Speed", "Rotor", 0.0, 2.0, 1.0, NONE, Choice(SPEEDS)),
    param(1, "Ramp", "Rotor", 200.0, 8000.0, 4500.0, Ms, Log),
    param(2, "Balance", "Rotor", -100.0, 100.0, 0.0, Percent, Linear),
    param(3, "Width", "Rotor", 0.0, 100.0, 80.0, Percent, Linear),
    param(4, "Drive", "Rotor", 0.0, 100.0, 0.0, Percent, Linear),
    param(5, "Mix", "Rotor", 0.0, 100.0, 100.0, Percent, Linear),
];

/// Rotary factory presets.
pub static ROTARY_PRESETS: &[Preset] = &[
    Preset {
        name: "Chorale",
        values: &[(rotary::SPEED, 1.0), (rotary::WIDTH, 100.0)],
    },
    Preset {
        name: "Tremolo fast",
        values: &[(rotary::SPEED, 2.0)],
    },
    Preset {
        name: "Gritty organ",
        values: &[(rotary::SPEED, 2.0), (rotary::DRIVE, 60.0)],
    },
    Preset {
        name: "Quick ramp",
        values: &[(rotary::SPEED, 2.0), (rotary::RAMP, 1200.0)],
    },
];

/// A spinning speaker cabinet: the highs go to a horn and the lows to a
/// drum, each spinning at its own speed past two microphones, so the sound
/// moves in pitch (Doppler), level and space. Changing Speed ramps the
/// rotors up or down the way the real thing does, the light horn first.
pub struct Rotary {
    v: [f32; 6],
    rate: u32,
    horn_hz: f32,
    drum_hz: f32,
    horn_k: f32,
    drum_k: f32,
    horn_phase: f32,
    drum_phase: f32,
    xover: kit::Crossover,
    horn: Line,
    drum: Line,
    balance: Smooth,
    width: Smooth,
    drive: Smooth,
    mix: Smooth,
}

impl Default for Rotary {
    fn default() -> Self {
        Self::new()
    }
}

impl Rotary {
    pub fn new() -> Rotary {
        let v = defaults(&ROTARY_PARAMS);
        let s = v[0] as usize;
        let mut r = Rotary {
            v,
            rate: 48_000,
            horn_hz: HORN_HZ[s],
            drum_hz: DRUM_HZ[s],
            horn_k: 0.0,
            drum_k: 0.0,
            horn_phase: 0.0,
            drum_phase: 0.25,
            xover: kit::Crossover::default(),
            horn: Line::default(),
            drum: Line::default(),
            balance: Smooth::new(v[2] / 100.0),
            width: Smooth::new(v[3] / 100.0),
            drive: Smooth::new(v[4] / 100.0),
            mix: Smooth::new(v[5] / 100.0),
        };
        r.setup(48_000);
        r
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            rotary::RAMP => self.set_ramp(),
            rotary::BALANCE => self.balance.target = v / 100.0,
            rotary::WIDTH => self.width.target = v / 100.0,
            rotary::DRIVE => self.drive.target = v / 100.0,
            rotary::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }
    fn set_ramp(&mut self) {
        let ms = self.v[rotary::RAMP as usize];
        self.drum_k = kit::coef(ms / 3.0, self.rate);
        self.horn_k = kit::coef(ms * HORN_RAMP / 3.0, self.rate);
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let s = (self.v[0] as usize).min(2);
        let (horn_to, drum_to) = (HORN_HZ[s], DRUM_HZ[s]);
        let ms = self.rate as f32 * 0.001;
        let per = 1.0 / self.rate as f32;
        for i in 0..l.len() {
            self.horn_hz += (horn_to - self.horn_hz) * self.horn_k;
            self.drum_hz += (drum_to - self.drum_hz) * self.drum_k;
            self.horn_phase = kit::wrap(self.horn_phase + self.horn_hz * per);
            self.drum_phase = kit::wrap(self.drum_phase + self.drum_hz * per);

            let drive = self.drive.glide();
            let mut m = 0.5 * (l[i] + r[i]);
            if drive > 0.0 {
                let g = 1.0 + 15.0 * drive * drive;
                m = lerp(m, fast_tanh(m * g) / g.sqrt(), drive);
            }
            let (lo, hi) = self.xover.split(0, m);
            self.horn.push(hi);
            self.drum.push(lo);

            // The two microphones sit either side of the cabinet; Width
            // swings them up to 45 degrees apart from the front.
            let spread = 0.125 * self.width.glide();
            let mut out = [0.0f32; 2];
            for (k, a) in [-spread, spread].into_iter().enumerate() {
                let hp = self.horn_phase - a;
                // The horn's mouth swings about 17 cm: +-0.5 ms of
                // Doppler at the microphone.
                let hd = (1.0 + 0.5 * kit::sine(hp)) * ms;
                let ha = 1.0 + 0.5 * kit::cosine(hp);
                let dp = self.drum_phase - a;
                let dd = (1.0 + 0.12 * kit::sine(dp)) * ms;
                let da = 1.0 + 0.25 * kit::cosine(dp);
                let bal = self.balance.value;
                let (hg, dg) = ((1.0 + bal).min(1.0), (1.0 - bal).min(1.0));
                out[k] = self.horn.read(hd) * ha * 0.67 * hg + self.drum.read(dd) * da * 0.8 * dg;
            }
            self.balance.glide();
            let mix = self.mix.glide();
            l[i] = lerp(l[i], out[0], mix);
            r[i] = lerp(r[i], out[1], mix);
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.xover.set(CROSSOVER_HZ, rate);
        let frames = (rate as usize * 4) / 1000 + 8;
        self.horn = Line::new(frames);
        self.drum = Line::new(frames);
        for s in [
            &mut self.balance,
            &mut self.width,
            &mut self.drive,
            &mut self.mix,
        ] {
            s.set_time(rate, 20.0);
        }
        self.set_ramp();
    }
    fn clear(&mut self) {
        let s = (self.v[0] as usize).min(2);
        self.horn_hz = HORN_HZ[s];
        self.drum_hz = DRUM_HZ[s];
        self.horn_phase = 0.0;
        self.drum_phase = 0.25;
        self.xover.reset();
        self.horn.clear();
        self.drum.clear();
        for s in [
            &mut self.balance,
            &mut self.width,
            &mut self.drive,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
    }
    fn tail(&self) -> u32 {
        self.rate / 100
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Rotary, ROTARY_INFO, ROTARY_PARAMS);
