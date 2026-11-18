//! Movement effects: [`Tremolo`], [`AutoPan`], [`RingMod`] and
//! [`Rotary`].
//!
//! Tremolo, auto-pan and the ring modulator's sweep share the LFO in
//! [`crate::kit`]: free in Hz or synced to the tempo (1/32 to 4 bars,
//! triplets and dotted), six shapes, on the bar when the transport starts
//! and following the song through a loop wrap or a locate. Every level
//! that moves glides, so nothing clicks when a knob turns under a loop.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::adaa::{Adaa, Flat2x};
use crate::character::soft_big;
use crate::kit::{
    self, node_impl, param, Choice, Glide, Hz, Lfo, Line, Linear, Log, Ms, Percent, NONE,
};
use crate::oversample::IirPair;
use crate::svf::{Svf, SvfMode};
use crate::{defaults, fast_exp2, flush, lerp, soft, Preset};

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

/// The Mode control's value as Volume (0) or Harmonic (1).
fn harmonic(mode: f32) -> f32 {
    if mode >= 0.5 {
        1.0
    } else {
        0.0
    }
}
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
    /// Volume (0) to Harmonic (1), gliding on a Mode change.
    harmonic: Smooth,
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
            harmonic: Smooth::new(harmonic(v[5])),
        };
        t.setup(48_000);
        t
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            tremolo::DEPTH => self.depth.target = v / 100.0,
            tremolo::WIDTH => self.width.target = v / 100.0,
            tremolo::MODE => {
                // The split has been idle in Volume: start it clean.
                if self.harmonic.value == 0.0 {
                    self.lows.reset();
                }
                self.harmonic.target = harmonic(v);
            }
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, ctx: &Ctx, from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.follow(ctx, from, self.v[1] as usize);
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        self.lfo.set_shape(self.v[2] as usize);
        let still = |s: &Smooth| s.value == 0.0 && s.target == 0.0;
        if still(&self.harmonic) && still(&self.width) {
            // The usual case, Volume with both sides together: one value
            // of the LFO a frame.
            for i in 0..l.len() {
                let depth = self.depth.glide();
                let [m] = self.lfo.step([0.0]);
                let g = 1.0 - depth * (0.5 - 0.5 * m);
                l[i] *= g;
                r[i] *= g;
            }
            return;
        }
        for i in 0..l.len() {
            let depth = self.depth.glide();
            let off = 0.5 * self.width.glide();
            let [ml, mr] = self.lfo.step([0.0, off]);
            let harmonic = self.harmonic.glide();
            if harmonic > 0.0 {
                // Lows and highs swap places: one is up while the other
                // is down, so the level stays nearly even. A Mode change
                // moves between that and Volume's one gain (the lows'
                // here) over 20 ms.
                let g = |m: f32| {
                    let lo = 1.0 - depth * (0.5 - 0.5 * m);
                    let hi = 1.0 - depth * (0.5 + 0.5 * m);
                    (lo, lerp(lo, hi, harmonic))
                };
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
        self.harmonic.set_time(rate, 20.0);
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.lows.reset();
        self.depth.snap(self.depth.target);
        self.width.snap(self.width.target);
        self.harmonic.snap(self.harmonic.target);
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
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, ctx: &Ctx, from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.follow(ctx, from, self.v[1] as usize);
        self.lfo.set(self.v[0], self.v[1] as usize, self.rate);
        self.lfo.set_shape(self.v[2] as usize);
        for i in 0..l.len() {
            let [m] = self.lfo.step([0.0]);
            let pos = (self.centre.glide() + self.depth.glide() * m).clamp(-1.0, 1.0);
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
/// Triangle and Square scaled to the sine's RMS, so a change of Wave
/// changes the tone and not the level (FX quality Z4).
const TRI_GAIN: f32 = 1.224_744_9;
const SQUARE_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;
/// The ring mod's half-bands for Triangle and Square, which run at twice
/// the rate: nine sections, flat to 0.46 of the base rate and 92 dB down
/// from 0.52, so a product above Nyquist is gone before it can fold back
/// (FX quality Z3).
const RING_SECTIONS: usize = 9;
const RING_TBW: f64 = 0.02;

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
    /// The carrier's phase in cycles, double precision so its error never
    /// shows as spurs.
    phase: f64,
    /// How much of each Wave is heard, and the first of the two one-poles
    /// that glide it after a change (so a change mid-way carries on).
    shares: [f32; 3],
    pull: [f32; 3],
    fading: bool,
    share_k: f32,
    /// Triangle and Square: each side's half-bands for the product at
    /// twice the rate, and the carrier's, which keep it under Nyquist.
    os: [IirPair<RING_SECTIONS>; 2],
    carrier: IirPair<RING_SECTIONS>,
    /// Frames since the stream started, modulo [`SETTLE`].
    settle: u32,
}

/// Each Wave's share when `wave` is chosen.
fn wave_shares(wave: usize) -> [f32; 3] {
    let mut w = [0.0; 3];
    w[wave.min(2)] = 1.0;
    w
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
            shares: wave_shares(v[1] as usize),
            pull: wave_shares(v[1] as usize),
            fading: false,
            share_k: 1.0,
            os: [IirPair::new(RING_TBW), IirPair::new(RING_TBW)],
            carrier: IirPair::new(RING_TBW),
            settle: 0,
        };
        m.setup(48_000);
        m
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            ringmod::FREQ => self.pitch.target = v.log2(),
            ringmod::SWEEP => self.sweep.target = v,
            ringmod::MIX => self.mix.target = v / 100.0,
            ringmod::WAVE => self.fading = true,
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    /// A frame of a Wave change: the shares a step toward the new Wave's,
    /// through two one-poles, until they are there.
    #[cold]
    #[inline(never)]
    fn fade_shares(&mut self) {
        let to = wave_shares(self.v[1] as usize);
        let c = self.share_k;
        let mut gap = 0.0f32;
        for k in 0..3 {
            self.pull[k] += (to[k] - self.pull[k]) * c;
            self.shares[k] += (self.pull[k] - self.shares[k]) * c;
            gap = gap.max((to[k] - self.shares[k]).abs());
        }
        if gap < 1e-5 {
            self.shares = to;
            self.pull = to;
            self.fading = false;
            if to[0] == 1.0 {
                // Back to the Sine alone: the half-bands stop, so they
                // start from rest next time rather than from what they
                // held.
                self.carrier.reset();
                for os in &mut self.os {
                    os.reset();
                }
            }
        }
    }
    /// Triangle's and Square's carrier at twice the rate for the frame
    /// whose phase went from `from` by `dt`: band-limited steps and
    /// corners, then through a half-band pair so nothing is left above the
    /// base rate's Nyquist.
    #[inline]
    fn carrier_2x(&mut self, from: f64, dt: f32) -> [f32; 2] {
        let [_, tri, square_w] = self.shares;
        let d2 = 0.5 * dt;
        let at = |p: f64| {
            let p = kit::wrap(p as f32);
            tri * TRI_GAIN * triangle(p, d2) + square_w * SQUARE_GAIN * square(p, d2)
        };
        let raw = [at(from + f64::from(d2)), at(from + f64::from(dt))];
        let base = self.carrier.down(raw);
        self.carrier.up(base)
    }
    fn run(&mut self, ctx: &Ctx, from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.follow(ctx, from, self.v[4] as usize);
        self.lfo.set(self.v[3], self.v[4] as usize, self.rate);
        let per = 1.0 / self.rate as f32;
        let top = 0.45 * self.rate as f32;
        // With Sweep at 0 the LFO only keeps time.
        let swept = self.sweep.value != 0.0 || self.sweep.target != 0.0;
        if !self.fading && self.shares[0] == 1.0 {
            self.run_sine(l, r, swept, per, top);
            return;
        }
        for i in 0..l.len() {
            let sweep = if swept {
                let [m] = self.lfo.step([0.0]);
                self.sweep.glide() * (0.5 + 0.5 * m)
            } else {
                self.lfo.tick();
                0.0
            };
            let hz = fast_exp2(self.pitch.glide() + sweep).min(top);
            let dt = hz * per;
            let was = self.phase;
            self.phase += f64::from(dt);
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            // The half-bands drop what is too small to hear every SETTLE
            // frames from the stream's start (the same in any block size).
            self.settle = (self.settle + 1) % SETTLE;
            if self.settle == 0 {
                self.carrier.settle();
                for os in &mut self.os {
                    os.settle();
                }
            }
            if self.fading {
                self.fade_shares();
            }
            let sine = self.shares[0];
            let c = if sine != 0.0 {
                sine * kit::sine_fine(self.phase as f32)
            } else {
                0.0
            };
            let twice = sine != 1.0;
            let c2 = if twice {
                self.carrier_2x(was, dt)
            } else {
                [0.0; 2]
            };
            let mix = self.mix.glide();
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let x = *s;
                let mut y = x * c;
                if twice {
                    let u = self.os[ch].up(x);
                    y += self.os[ch].down([u[0] * c2[0], u[1] * c2[1]]);
                }
                *s = lerp(x, y, mix);
            }
        }
    }
    /// The Sine alone (the default): nothing runs at twice the rate, and
    /// with the pitch still and no Sweep, the step is worked out once.
    /// The same samples as [`RingMod::run`]'s loop would give.
    fn run_sine(&mut self, l: &mut [f32], r: &mut [f32], swept: bool, per: f32, top: f32) {
        let still = !swept && self.pitch.settled();
        let still_dt = fast_exp2(self.pitch.value + 0.0).min(top) * per;
        for i in 0..l.len() {
            let dt = if still {
                self.lfo.tick();
                still_dt
            } else {
                let sweep = if swept {
                    let [m] = self.lfo.step([0.0]);
                    self.sweep.glide() * (0.5 + 0.5 * m)
                } else {
                    self.lfo.tick();
                    0.0
                };
                fast_exp2(self.pitch.glide() + sweep).min(top) * per
            };
            self.phase += f64::from(dt);
            if self.phase >= 1.0 {
                self.phase -= 1.0;
            }
            let c = kit::sine_fine(self.phase as f32);
            let mix = self.mix.glide();
            l[i] = lerp(l[i], l[i] * c, mix);
            r[i] = lerp(r[i], r[i] * c, mix);
        }
        // The half-bands rest, but the count goes on.
        self.settle = (self.settle + (l.len() % SETTLE as usize) as u32) % SETTLE;
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.pitch.set_time(rate, 30.0);
        self.sweep.set_time(rate, 30.0);
        self.mix.set_time(rate, 20.0);
        // Each one-pole 2.5 ms: a Wave change glides over about 20 ms.
        self.share_k = 1.0 - (-1.0 / (0.0025 * rate as f32).max(1.0)).exp();
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.phase = 0.0;
        self.shares = wave_shares(self.v[1] as usize);
        self.pull = self.shares;
        self.fading = false;
        self.carrier.reset();
        for os in &mut self.os {
            os.reset();
        }
        self.settle = 0;
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

/// The step correction at phase `t` for a step of +2 at phase 0 (`dt` is
/// the phase step per frame): the step smoothed by a cubic B-spline across
/// four frames, less the step itself. Its aliases fall as sinc to the
/// fourth, about 25 dB below the two-frame PolyBLEP's where a carrier's
/// harmonics fold back (FX quality Z3).
#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < 2.0 * dt {
        // After the step, `x` frames on.
        let x = t / dt;
        if x < 1.0 {
            let x2 = x * x;
            -1.0 + x * (4.0 / 3.0) + x2 * (x * (-2.0 / 3.0) + x * x * 0.25)
        } else {
            let u = 2.0 - x;
            let u2 = u * u;
            -u2 * u2 / 12.0
        }
    } else if t > 1.0 - 2.0 * dt {
        // Before it, `x` frames back (negative).
        let x = (t - 1.0) / dt;
        if x > -1.0 {
            let x2 = x * x;
            1.0 + x * (4.0 / 3.0) + x2 * (x * (-2.0 / 3.0) - x * x * 0.25)
        } else {
            let u = 2.0 + x;
            let u2 = u * u;
            u2 * u2 / 12.0
        }
    } else {
        0.0
    }
}

/// The corner correction at phase `t` for a unit change of slope (per
/// frame) at phase 0: the corner smoothed by the same cubic B-spline as
/// [`blep`], less the corner itself. It is the same either side.
#[inline]
fn blamp(t: f32, dt: f32) -> f32 {
    let x = if t < 2.0 * dt {
        t / dt
    } else if t > 1.0 - 2.0 * dt {
        (1.0 - t) / dt
    } else {
        return 0.0;
    };
    if x < 1.0 {
        let x2 = x * x;
        7.0 / 30.0 - 0.5 * x + x2 * (1.0 / 3.0 + x2 * (x / 40.0 - 1.0 / 12.0))
    } else {
        let u = 2.0 - x;
        let u2 = u * u;
        u2 * u2 * u / 120.0
    }
}

/// A square wave (+1 then -1) with its steps band-limited across four
/// frames, so a high carrier doesn't alias.
#[inline]
pub(crate) fn square(t: f32, dt: f32) -> f32 {
    let dt = dt.clamp(1e-6, 0.25);
    let naive = if t < 0.5 { 1.0 } else { -1.0 };
    naive + blep(t, dt) - blep(kit::wrap(t + 0.5), dt)
}

/// A triangle wave (-1 at phase 0, +1 at a half) with its corners
/// band-limited across four frames.
#[inline]
pub(crate) fn triangle(t: f32, dt: f32) -> f32 {
    let dt = dt.clamp(1e-6, 0.25);
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
/// Ramp is the time the drum takes from Slow to within 10 % of Fast: a
/// one-pole covers that gap (5.03 Hz, to 0.57) in ln(5.03 / 0.57) = 2.18
/// time constants. The horn takes [`HORN_RAMP`] of it (1.1 s at the
/// default 4.5 s), as a Leslie's light horn does.
const RAMP_TAUS: f32 = 2.18;
/// Both rotors coast down a quarter more slowly than the motor speeds
/// them up (Smith, Serafin, Abel and Berners: the two differ).
const COAST: f32 = 1.25;
const CROSSOVER_HZ: f32 = 800.0;
/// The horn's treble beam: facing away from a microphone, it heard
/// through a one-pole low-pass at this corner, so the highs swirl more
/// than the mids (after Smith et al.'s angle-dependent filtering).
const BEAM_HZ: f32 = 2000.0;
/// The horn's and the drum's levels at the microphones, so the cabinet
/// goes in at the dry's level (fxlab W11) with the beam and the level
/// swings it adds.
const HORN_GAIN: f32 = 1.07;
const DRUM_GAIN: f32 = 1.28;
/// The drive's half-band filters: Drive's Eco design (flat to 0.19 of the
/// doubled rate, 71 dB down from 0.31), and the frames it and ADAA's flat
/// filter delay the sound by.
const OS_SECTIONS: usize = 5;
const OS_TBW: f64 = 0.06;
const OS_DELAY: usize = 2 + Flat2x::STEPS / 2;
/// Drive 100 % drives the input this much more (21 dB) into [`soft`].
const DRIVE_GAIN: f32 = 11.0;
/// What [`soft`]'s output settles at in size on loud noise, against the
/// -18 dBFS reference.
const SOFT_TOP: f32 = 0.14;
/// How often the drive's half-bands flush their states, frames.
const SETTLE: u32 = 32;

/// The level after the drive's gain `g` and [`soft`] that keeps noise at
/// -18 dBFS RMS as loud as it went in (within 0.3 dB from Drive 0 to 100,
/// a Gaussian's numbers): `1 / g` while the curve is straight, and the
/// curve's ceiling once it saturates.
#[inline]
fn undrive(g: f32) -> f32 {
    (1.0 / (g * g) + SOFT_TOP * SOFT_TOP).sqrt()
}

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

/// The rotary's drive at twice the rate: [`soft`] averaged by first-order
/// ADAA between Drive's Eco half-bands, as Lo-fi's saturation (FX quality
/// W13). [`OS_DELAY`] frames late.
#[derive(Clone, Debug)]
struct Sat {
    iir: IirPair<OS_SECTIONS>,
    adaa: Adaa,
    flat: Flat2x,
    /// The last input at twice the rate, for the straight line's average.
    v1: f32,
}

impl Sat {
    fn new() -> Sat {
        Sat {
            iir: IirPair::new(OS_TBW),
            adaa: Adaa::default(),
            flat: Flat2x::default(),
            v1: 0.0,
        }
    }
    /// One base frame: `soft(g x) * undrive`, faded in from the straight
    /// line by `blend`.
    #[inline]
    fn run(&mut self, x: f32, g: f32, undrive: f32, blend: f32) -> f32 {
        let up = self.iir.up(x);
        let mut out = [0.0f32; 2];
        for (o, &v) in out.iter_mut().zip(&up) {
            // In its straight part ADAA is the two-point mean.
            let line = 0.5 * (v + self.v1);
            let c = self.adaa.run(g * v, soft, soft_big) * undrive;
            self.v1 = v;
            *o = self.flat.run(line + (c - line) * blend);
        }
        self.iir.down(out)
    }
    fn reset(&mut self) {
        self.iir.reset();
        self.adaa.reset();
        self.flat.reset();
        self.v1 = 0.0;
    }
}

/// A spinning speaker cabinet: the highs go to a horn and the lows to a
/// drum, each spinning at its own speed past two microphones, so the sound
/// moves in pitch (Doppler), level and space. Changing Speed ramps the
/// rotors up or down the way the real thing does, the light horn first,
/// and they coast down more slowly than they speed up. The drum turns the
/// other way from the horn, and the horn's treble beams: its highs swing
/// more than its mids as it turns.
pub struct Rotary {
    v: [f32; 6],
    rate: u32,
    horn_hz: f32,
    drum_hz: f32,
    /// One-pole steps toward the speed: speeding up, coasting down.
    horn_k: [f32; 2],
    drum_k: [f32; 2],
    horn_phase: f32,
    drum_phase: f32,
    xover: kit::Crossover,
    horn: Line,
    drum: Line,
    /// The beam's low-pass, one a microphone, and its step.
    beam: [f32; 2],
    beam_a: f32,
    /// The drive at twice the rate while Drive is up (or fading out), and
    /// the input [`OS_DELAY`] frames late for when it isn't, so the
    /// cabinet's timing never changes; `driven` crossfades the two.
    sat: Sat,
    driving: bool,
    driven: Smooth,
    late: [f32; OS_DELAY],
    late_at: usize,
    /// Frames since the stream started, modulo [`SETTLE`].
    settle: u32,
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
            horn_k: [0.0; 2],
            drum_k: [0.0; 2],
            horn_phase: 0.0,
            drum_phase: 0.25,
            xover: kit::Crossover::default(),
            horn: Line::default(),
            drum: Line::default(),
            beam: [0.0; 2],
            beam_a: 0.0,
            sat: Sat::new(),
            driving: false,
            driven: Smooth::new(0.0),
            late: [0.0; OS_DELAY],
            late_at: 0,
            settle: 0,
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
        let tau = self.v[rotary::RAMP as usize] / RAMP_TAUS;
        let rate = self.rate;
        self.drum_k = [kit::coef(tau, rate), kit::coef(tau * COAST, rate)];
        let tau = tau * HORN_RAMP;
        self.horn_k = [kit::coef(tau, rate), kit::coef(tau * COAST, rate)];
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    /// The input, driven or not, [`OS_DELAY`] frames late.
    #[inline]
    fn input(&mut self, m: f32) -> f32 {
        // A ring of OS_DELAY frames: what is read is what went in that
        // long ago.
        let late = std::mem::replace(&mut self.late[self.late_at], m);
        self.late_at = (self.late_at + 1) % OS_DELAY;
        // The half-bands drop what is too small to hear every SETTLE
        // frames from the stream's start (the same in any block size).
        self.settle = (self.settle + 1) % SETTLE;
        if !self.driving {
            return late;
        }
        if self.settle == 0 {
            self.sat.iir.settle();
        }
        let d = self.drive.glide();
        let g = 1.0 + DRIVE_GAIN * d * d;
        let y = self.sat.run(m, g, undrive(g), d);
        let w = self.driven.glide();
        if w == 0.0 && self.driven.target == 0.0 {
            self.driving = false;
        }
        late + (y - late) * w
    }
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let s = (self.v[0] as usize).min(2);
        let (horn_to, drum_to) = (HORN_HZ[s], DRUM_HZ[s]);
        let hk = self.horn_k[usize::from(horn_to < self.horn_hz)];
        let dk = self.drum_k[usize::from(drum_to < self.drum_hz)];
        let ms = self.rate as f32 * 0.001;
        let per = 1.0 / self.rate as f32;
        // Drive comes in through the oversampled stage, which starts from
        // rest, and goes out the same way once it is back at 0.
        let wants = self.drive.target > 0.0 || self.drive.value > 0.0;
        if wants && !self.driving {
            self.sat.reset();
            self.driving = true;
        }
        self.driven.target = if wants { 1.0 } else { 0.0 };
        for i in 0..l.len() {
            self.horn_hz += (horn_to - self.horn_hz) * hk;
            self.drum_hz += (drum_to - self.drum_hz) * dk;
            self.horn_phase = kit::wrap(self.horn_phase + self.horn_hz * per);
            // The drum turns the other way.
            self.drum_phase = kit::wrap(self.drum_phase - self.drum_hz * per);

            let m = self.input(0.5 * (l[i] + r[i]));
            let (lo, hi) = self.xover.split(0, m);
            self.horn.push(hi);
            self.drum.push(lo);

            // The two microphones sit either side of the cabinet; Width
            // swings them up to 45 degrees apart from the front.
            let spread = 0.125 * self.width.glide();
            let bal = self.balance.value;
            let (hg, dg) = (
                (1.0 + bal).min(1.0) * HORN_GAIN,
                (1.0 - bal).min(1.0) * DRUM_GAIN,
            );
            // Each rotor's angle at each microphone, from the rotor's and
            // the spread's sines by the angle sums (six sines, not eight).
            let (ss, cs) = (kit::sine(spread), kit::cosine(spread));
            let (sh, ch) = (kit::sine(self.horn_phase), kit::cosine(self.horn_phase));
            let (sd, cd) = (kit::sine(self.drum_phase), kit::cosine(self.drum_phase));
            let mut out = [0.0f32; 2];
            // The left microphone sees each rotor `spread` ahead, the
            // right `spread` behind.
            for (k, side) in [ss, -ss].into_iter().enumerate() {
                // The horn's mouth swings about 17 cm: +-0.5 ms of
                // Doppler at the microphone.
                let hd = (1.0 + 0.5 * (sh * cs + ch * side)) * ms;
                let facing = ch * cs - sh * side;
                let ha = 1.0 + 0.5 * facing;
                // Facing away, the horn is heard through the beam's
                // low-pass; facing the microphone, whole.
                let h = self.horn.read(hd);
                let b = self.beam[k] + (h - self.beam[k]) * self.beam_a;
                self.beam[k] = b;
                let h = b + (h - b) * (0.5 + 0.5 * facing);
                let dd = (1.0 + 0.12 * (sd * cs + cd * side)) * ms;
                let da = 1.0 + 0.25 * (cd * cs - sd * side);
                out[k] = h * ha * hg + self.drum.read(dd) * da * dg;
            }
            self.balance.glide();
            let mix = self.mix.glide();
            l[i] = lerp(l[i], out[0], mix);
            r[i] = lerp(r[i], out[1], mix);
        }
        // The beam's state can fall away to nothing in silence.
        self.beam = self.beam.map(flush);
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.xover.set(CROSSOVER_HZ, rate);
        let frames = (rate as usize * 4) / 1000 + 8;
        self.horn = Line::new(frames);
        self.drum = Line::new(frames);
        self.beam_a = 1.0 - (-std::f32::consts::TAU * BEAM_HZ / rate as f32).exp();
        for s in [
            &mut self.balance,
            &mut self.width,
            &mut self.drive,
            &mut self.mix,
            &mut self.driven,
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
        self.beam = [0.0; 2];
        self.sat.reset();
        self.late = [0.0; OS_DELAY];
        self.late_at = 0;
        self.settle = 0;
        for s in [
            &mut self.balance,
            &mut self.width,
            &mut self.drive,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
        self.driving = self.drive.target > 0.0;
        self.driven.snap(if self.driving { 1.0 } else { 0.0 });
    }
    fn tail(&self) -> u32 {
        self.rate / 100
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Rotary, ROTARY_INFO, ROTARY_PARAMS);

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    /// Seconds for the horn's or the drum's speed to get within `near`
    /// (a share of the gap, or of the target when `of_target`) of the
    /// speed `to`, from `from`, run a block at a time.
    fn ramp_s(from: f32, to: f32, horn: bool, near: f32, of_target: bool) -> f32 {
        let rate = 48_000;
        let mut r = Rotary::new();
        r.setup(rate);
        r.set_param(rotary::SPEED, from);
        r.reset();
        r.set_param(rotary::SPEED, to);
        let speed = |r: &Rotary| if horn { r.horn_hz } else { r.drum_hz };
        let want = if horn { HORN_HZ } else { DRUM_HZ }[to as usize];
        let within = near
            * if of_target {
                want
            } else {
                (speed(&r) - want).abs()
            };
        let ctx = Ctx::new(rate, 0);
        let (mut l, mut rr) = (vec![0.0f32; 128], vec![0.0f32; 128]);
        for n in 0..20 * rate as usize / 128 {
            r.run(&ctx, 0, &mut l, &mut rr);
            if (speed(&r) - want).abs() <= within {
                return (n + 1) as f32 * 128.0 / rate as f32;
            }
        }
        f32::MAX
    }

    #[test]
    fn the_rotors_ramp_as_a_leslie_and_coast_down_slower() {
        // From Slow to within 10 % of Fast at the default Ramp: the drum
        // in the Ramp's 4.5 s, the horn in about a quarter of it.
        let drum = ramp_s(1.0, 2.0, false, 0.1, true);
        let horn = ramp_s(1.0, 2.0, true, 0.1, true);
        assert!((drum - 4.5).abs() < 0.2, "drum {drum} s");
        assert!((horn - 1.1).abs() < 0.15, "horn {horn} s");
        // Coasting down 90 % of the way takes a quarter longer than
        // speeding up 90 % of the way.
        for horn in [false, true] {
            let up = ramp_s(1.0, 2.0, horn, 0.1, false);
            let down = ramp_s(2.0, 1.0, horn, 0.1, false);
            assert!(
                (down / up - COAST).abs() < 0.05,
                "{horn}: {down} against {up}"
            );
        }
    }

    #[test]
    fn the_drum_turns_the_other_way_from_the_horn() {
        let rate = 48_000;
        let mut r = Rotary::new();
        r.setup(rate);
        r.set_param(rotary::SPEED, 2.0);
        r.reset();
        let ctx = Ctx::new(rate, 0);
        let (mut l, mut rr) = (vec![0.0f32; 16], vec![0.0f32; 16]);
        let (h0, d0) = (r.horn_phase, r.drum_phase);
        r.run(&ctx, 0, &mut l, &mut rr);
        let turn = |a: f32, b: f32| kit::wrap(b - a + 0.5) - 0.5;
        assert!(turn(h0, r.horn_phase) > 0.0);
        assert!(turn(d0, r.drum_phase) < 0.0);
    }

    #[test]
    fn drive_comes_in_and_goes_out_without_moving_the_sound_in_time() {
        // A low tone through Drive 0, then Drive up to 30 % and back: the
        // driven stage is as late as the plain one, so the tone never
        // steps (no frame-to-frame change more than twice the tone's own).
        let rate = 48_000;
        let mut r = Rotary::new();
        r.setup(rate);
        r.set_param(rotary::SPEED, 0.0);
        r.reset();
        let ctx = Ctx::new(rate, 0);
        let hz = 100.0;
        let amp = 0.1;
        let own = amp * std::f32::consts::TAU * hz / rate as f32;
        let mut last = 0.0f32;
        let mut worst = 0.0f32;
        let mut t = 0usize;
        for block in 0..(3 * rate as usize / 128) {
            if block == rate as usize / 128 {
                r.set_param(rotary::DRIVE, 30.0);
            }
            if block == 2 * rate as usize / 128 {
                r.set_param(rotary::DRIVE, 0.0);
            }
            let mut l: Vec<f32> = (0..128)
                .map(|i| amp * (std::f32::consts::TAU * hz * (t + i) as f32 / rate as f32).sin())
                .collect();
            let mut rr = l.clone();
            r.run(&ctx, 0, &mut l, &mut rr);
            for &v in &l {
                if t > 4_800 {
                    worst = worst.max((v - last).abs());
                }
                last = v;
                t += 1;
            }
        }
        assert!(worst < 3.0 * own, "{worst} against {own}");
        assert!(!r.driving, "back to the plain stage");
    }

    #[test]
    fn the_band_limited_steps_and_corners_join_up() {
        // Smoothed by the B-spline, the square has no step and the
        // triangle no corner: fine steps of the phase move them only a
        // little, where the plain waves jump by 2 and turn by 8.
        let fine = 1e-5f32;
        for dt in [0.01f32, 0.1, 0.2] {
            let (mut sq, mut tri, mut turn) = (0.0f32, 0.0f32, 0.0f32);
            let mut mean = 0.0f64;
            let n = (1.0 / fine) as usize;
            let mut was = (square(1.0 - fine, dt), triangle(1.0 - fine, dt));
            let mut slope = (triangle(1.0 - fine, dt) - triangle(1.0 - 2.0 * fine, dt)) / fine;
            for i in 0..n {
                let t = i as f32 * fine;
                let now = (square(t, dt), triangle(t, dt));
                sq = sq.max((now.0 - was.0).abs());
                tri = tri.max((now.1 - was.1).abs());
                let s = (now.1 - was.1) / fine;
                turn = turn.max((s - slope).abs());
                slope = s;
                mean += f64::from(now.0);
                was = now;
            }
            assert!(sq < 0.01, "dt {dt}: square steps {sq}");
            assert!(tri < 4.1 * fine, "dt {dt}: triangle steps {tri}");
            assert!(turn < 0.5, "dt {dt}: the slope turns {turn}");
            assert!((mean / n as f64).abs() < 1e-3, "dt {dt}: mean {mean}");
        }
    }

    /// The RMS of the ring mod's output over its last half second, a
    /// steady 0.5 in, the carrier at 440 Hz in `wave`.
    fn ring_rms(wave: f32) -> f32 {
        let rate = 44_100;
        let mut m = RingMod::new();
        m.set_param(ringmod::FREQ, 440.0);
        m.set_param(ringmod::WAVE, wave);
        m.prepare(rate, 128);
        m.reset();
        let ctx = Ctx::new(rate, 0);
        let mut sum = 0.0f64;
        for k in 0..(rate as usize / 128) {
            let (mut l, mut r) = ([0.5f32; 128], [0.5f32; 128]);
            m.process(&ctx, &[], &mut l, &mut r);
            if k >= rate as usize / 256 {
                sum += l.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
            }
        }
        (sum / ((rate as usize / 128 - rate as usize / 256) * 128) as f64).sqrt() as f32
    }

    #[test]
    fn every_wave_is_as_loud() {
        let sine = ring_rms(0.0);
        assert!((sine - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 0.005);
        for wave in [1.0, 2.0] {
            let db = 20.0 * (ring_rms(wave) / sine).log10();
            assert!(db.abs() < 0.2, "wave {wave}: {db} dB");
        }
    }

    #[test]
    fn a_wave_change_glides() {
        let rate = 48_000;
        let mut m = RingMod::new();
        m.prepare(rate, 128);
        m.reset();
        let ctx = Ctx::new(rate, 0);
        m.set_param(ringmod::WAVE, 2.0);
        let (mut l, mut r) = ([0.1f32; 1], [0.1f32; 1]);
        m.process(&ctx, &[], &mut l, &mut r);
        // A frame on, still nearly all Sine; 60 ms on, all Square.
        assert!(m.shares[0] > 0.99 && m.fading, "{:?}", m.shares);
        let (mut l, mut r) = (vec![0.1f32; 2880], vec![0.1f32; 2880]);
        m.process(&ctx, &[], &mut l, &mut r);
        assert_eq!(m.shares, wave_shares(2));
        assert!(!m.fading);
    }
}
