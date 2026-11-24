//! Guitar and bass colour: [`Amp`] (preamp stages, a tone stack modelled
//! from the circuit, a power stage, presence and a speaker cabinet) and
//! [`Exciter`] (harmonics added to the top end).
//!
//! The amp runs mono (an amp is), with everything from its boost to its
//! power stage at four times the rate between polyphase IIR half-bands and
//! its first stage's curve averaged by ADAA, so high gain doesn't alias.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::adaa::{log_cosh, Adaa};
use crate::kit::{node_impl, param, Choice, Db, Glide, Hz, Linear, Log, Ms, Percent, NONE};
use crate::oversample::IirPair;
use crate::svf::{Svf, SvfMode};
use crate::{db_to_gain, defaults, fast_tanh, flush, lerp, Preset};

// ---------------------------------------------------------------- tone stack

/// The values of one "FMV" tone stack (the Fender, Marshall and Vox
/// family's treble, mid and bass network): pots R1 (treble), R2 (bass),
/// R3 (mid), slope resistor R4, capacitors C1 to C3.
#[derive(Clone, Copy, Debug)]
struct Stack {
    r1: f64,
    r2: f64,
    r3: f64,
    r4: f64,
    c1: f64,
    c2: f64,
    c3: f64,
}

/// A 1959 tweed bass amp's stack (Yeh and Smith's reference circuit).
const STACK_TWEED: Stack = Stack {
    r1: 250e3,
    r2: 1e6,
    r3: 25e3,
    r4: 56e3,
    c1: 250e-12,
    c2: 20e-9,
    c3: 20e-9,
};
/// A British lead amp's stack (more mid, tighter bass).
const STACK_BRIT: Stack = Stack {
    r1: 220e3,
    r2: 1e6,
    r3: 22e3,
    r4: 33e3,
    c1: 470e-12,
    c2: 22e-9,
    c3: 22e-9,
};
/// A Californian high-gain stack (bigger bass caps).
const STACK_CAL: Stack = Stack {
    r1: 250e3,
    r2: 250e3,
    r3: 25e3,
    r4: 100e3,
    c1: 250e-12,
    c2: 100e-9,
    c3: 47e-9,
};

/// The tone stack's analog transfer function for treble `t`, mid `m` and
/// bass `l` (0 to 1, kept off the ends), from nodal analysis of the
/// circuit with the mid pot's wiper on C3 (it matches Yeh and Smith's
/// published first-order terms): `(b1 s + b2 s^2 + b3 s^3) / (1 + a1 s +
/// a2 s^2 + a3 s^3)`.
fn stack_analog(k: &Stack, t: f64, m: f64, l: f64) -> ([f64; 3], [f64; 3]) {
    let Stack {
        r1,
        r2,
        r3,
        r4,
        c1,
        c2,
        c3,
    } = *k;
    let b1 = c1 * r1 * t + c1 * r2 * l + c1 * r3 + c2 * r2 * l + c2 * r3 + c3 * r3 * m;
    let b2 = c1 * c2 * r1 * r2 * l
        + c1 * c2 * r1 * r3
        + c1 * c2 * r1 * r4 * t
        + c1 * c2 * r2 * r4 * l
        + c1 * c2 * r3 * r4
        + c1 * c3 * r1 * r3 * m
        + c1 * c3 * r1 * r4 * t
        + c1 * c3 * r2 * r3 * l * m
        + c1 * c3 * r2 * r4 * l
        - c1 * c3 * r3 * r3 * m * m
        + c1 * c3 * r3 * r3 * m
        + c1 * c3 * r3 * r4
        + c2 * c3 * r2 * r3 * l * m
        - c2 * c3 * r3 * r3 * m * m
        + c2 * c3 * r3 * r3 * m;
    let c123 = c1 * c2 * c3;
    let b3 = c123
        * (r1 * r2 * r3 * l * m + r1 * r2 * r4 * l * t - r1 * r3 * r3 * m * m + r1 * r3 * r3 * m
            - r1 * r3 * r4 * m * t
            + r1 * r3 * r4 * t
            + r2 * r3 * r4 * l * m
            - r3 * r3 * r4 * m * m
            + r3 * r3 * r4 * m);
    let a1 =
        c1 * r1 + c1 * r2 * l + c1 * r3 + c2 * r2 * l + c2 * r3 + c2 * r4 + c3 * r3 * m + c3 * r4;
    let a2 = c1 * c2 * r1 * r2 * l
        + c1 * c2 * r1 * r3
        + c1 * c2 * r1 * r4
        + c1 * c2 * r2 * r4 * l
        + c1 * c2 * r3 * r4
        + c1 * c3 * r1 * r3 * m
        + c1 * c3 * r1 * r4
        + c1 * c3 * r2 * r3 * l * m
        + c1 * c3 * r2 * r4 * l
        - c1 * c3 * r3 * r3 * m * m
        + c1 * c3 * r3 * r3 * m
        + c1 * c3 * r3 * r4
        + c2 * c3 * r2 * r3 * l * m
        + c2 * c3 * r2 * r4 * l
        - c2 * c3 * r3 * r3 * m * m
        + c2 * c3 * r3 * r3 * m
        - c2 * c3 * r3 * r4 * m
        + c2 * c3 * r3 * r4;
    let a3 = c123
        * (r1 * r2 * r3 * l * m + r1 * r2 * r4 * l - r1 * r3 * r3 * m * m + r1 * r3 * r3 * m
            - r1 * r3 * r4 * m
            + r1 * r3 * r4
            + r2 * r3 * r4 * l * m
            - r3 * r3 * r4 * m * m
            + r3 * r3 * r4 * m);
    ([b1, b2, b3], [a1, a2, a3])
}

/// The tone stack as a third-order digital filter (bilinear transform),
/// run in f64 because its poles sit close to the unit circle at low
/// frequencies.
#[derive(Clone, Copy, Debug, Default)]
struct ToneStack {
    b: [f64; 4],
    a: [f64; 3],
    s: [f64; 3],
}

impl ToneStack {
    fn set(&mut self, k: &Stack, treble: f32, mid: f32, bass: f32, rate: u32) {
        let t = f64::from(treble).clamp(0.001, 0.999);
        let m = f64::from(mid).clamp(0.001, 0.999);
        // The bass pot is an audio (log) taper.
        let x = f64::from(bass).clamp(0.0, 1.0);
        let l = ((10f64.powf(2.0 * x) - 1.0) / 99.0).clamp(0.001, 0.999);
        let ([b1, b2, b3], [a1, a2, a3]) = stack_analog(k, t, m, l);
        let c = 2.0 * f64::from(rate);
        let (c2, c3) = (c * c, c * c * c);
        let bb = [
            b1 * c + b2 * c2 + b3 * c3,
            b1 * c - b2 * c2 - 3.0 * b3 * c3,
            -b1 * c - b2 * c2 + 3.0 * b3 * c3,
            -b1 * c + b2 * c2 - b3 * c3,
        ];
        let aa = [
            1.0 + a1 * c + a2 * c2 + a3 * c3,
            3.0 + a1 * c - a2 * c2 - 3.0 * a3 * c3,
            3.0 - a1 * c - a2 * c2 + 3.0 * a3 * c3,
            1.0 - a1 * c + a2 * c2 - a3 * c3,
        ];
        let n = 1.0 / aa[0];
        self.b = bb.map(|v| v * n);
        self.a = [aa[1] * n, aa[2] * n, aa[3] * n];
    }
    /// One sample (transposed direct form II).
    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        let x = f64::from(x);
        let y = self.b[0] * x + self.s[0];
        self.s[0] = self.b[1] * x - self.a[0] * y + self.s[1];
        self.s[1] = self.b[2] * x - self.a[1] * y + self.s[2];
        self.s[2] = self.b[3] * x - self.a[2] * y;
        y as f32
    }
    /// The gain in dB at `hz` (for tests and drawings).
    #[cfg(test)]
    fn response_db(&self, hz: f64, rate: u32) -> f64 {
        let w = std::f64::consts::TAU * hz / f64::from(rate);
        let z = |k: f64| (-(w * k)).sin_cos();
        let (mut nr, mut ni, mut dr, mut di) = (0.0, 0.0, 1.0, 0.0);
        for k in 0..4 {
            let (s, c) = z(k as f64);
            nr += self.b[k] * c;
            ni += self.b[k] * s;
            if k > 0 {
                dr += self.a[k - 1] * c;
                di += self.a[k - 1] * s;
            }
        }
        10.0 * ((nr * nr + ni * ni) / (dr * dr + di * di)).log10()
    }
    fn reset(&mut self) {
        self.s = [0.0; 3];
    }
}

// ---------------------------------------------------------------- amp

/// The antiderivative of `tanh`, for the stages' antialiasing.
#[inline]
fn tanh_big(u: f64) -> f64 {
    log_cosh(u.abs())
}

/// [`Amp`] parameter ids. 0 to 7 are the first version's, so saved
/// projects keep their settings.
pub mod amp_ids {
    pub const MODEL: u16 = 0;
    pub const GAIN: u16 = 1;
    pub const BASS: u16 = 2;
    pub const MID: u16 = 3;
    pub const TREBLE: u16 = 4;
    pub const PRESENCE: u16 = 5;
    pub const CAB: u16 = 6;
    pub const LEVEL: u16 = 7;
    pub const BOOST: u16 = 8;
    pub const GATE: u16 = 9;
    pub const RELEASE: u16 = 10;
    pub const SAG: u16 = 11;
    pub const DEPTH: u16 = 12;
}
use amp_ids as amp;

const MODELS: &[&str] = &["Clean", "Crunch", "Lead", "Bass"];
const CABS: &[&str] = &["Off", "1x12 open", "2x12", "4x12", "1x15 bass"];
const BOOSTS: &[&str] = &["Off", "Tight", "Treble"];

/// What each model changes: the input high-pass (tightness), the gain at
/// the top of the Gain knob, how many clipping stages, their bias (even
/// harmonics), the stack, Depth's low resonance and the output trim.
struct Model {
    tight_hz: f32,
    max_gain_db: f32,
    stages: usize,
    bias: f32,
    stack: Stack,
    depth_hz: f32,
    /// The output trim in dB at Gain 0, 1, 2 and so on to 10 (straight
    /// lines between), fitted so pink noise at -18 dBFS RMS comes out as
    /// loud as it went in, at the default tone and cab with Boost, Sag and
    /// Depth off: Gain changes the grit, not the loudness.
    trim_db: [f32; 11],
}

const MODEL_DATA: [Model; 4] = [
    Model {
        tight_hz: 60.0,
        max_gain_db: 24.0,
        stages: 1,
        bias: 0.15,
        stack: STACK_TWEED,
        depth_hz: 100.0,
        trim_db: [
            1.76, -0.58, -2.86, -5.08, -7.18, -9.12, -10.84, -12.32, -13.52, -14.45, -15.15,
        ],
    },
    Model {
        tight_hz: 100.0,
        max_gain_db: 40.0,
        stages: 2,
        bias: 0.25,
        stack: STACK_BRIT,
        depth_hz: 90.0,
        trim_db: [
            1.51, -3.88, -8.65, -12.50, -15.19, -16.83, -17.71, -18.15, -18.35, -18.43, -18.47,
        ],
    },
    Model {
        tight_hz: 300.0,
        max_gain_db: 54.0,
        stages: 3,
        bias: 0.3,
        stack: STACK_CAL,
        depth_hz: 80.0,
        trim_db: [
            7.11, -1.29, -8.31, -12.77, -14.88, -15.73, -16.06, -16.20, -16.29, -16.35, -16.40,
        ],
    },
    Model {
        tight_hz: 30.0,
        max_gain_db: 30.0,
        stages: 2,
        bias: 0.2,
        stack: STACK_TWEED,
        depth_hz: 60.0,
        trim_db: [
            2.55, -1.84, -5.77, -9.11, -11.74, -13.65, -14.94, -15.75, -16.23, -16.50, -16.65,
        ],
    },
];

/// The trim in dB at knob position `k` (Gain over 10).
fn trim_at(t: &[f32; 11], k: f32) -> f32 {
    let at = (k * 10.0).clamp(0.0, 10.0);
    let i = (at as usize).min(9);
    t[i] + (t[i + 1] - t[i]) * (at - i as f32)
}

/// One cab: six filter stages (mode, Hz, Q, dB), fitted by ear and to the
/// shape of published speaker responses: the low resonance, the
/// cone's mid bumps, the steep fall above 5 kHz and the dip near 7 kHz.
type CabShape = [(SvfMode, f32, f32, f32); 6];
const CAB_DATA: [CabShape; 4] = [
    [
        (SvfMode::HighPass, 80.0, 0.8, 0.0),
        (SvfMode::Bell, 130.0, 1.0, 2.5),
        (SvfMode::Bell, 2500.0, 1.2, 4.0),
        (SvfMode::LowPass, 5500.0, 0.8, 0.0),
        (SvfMode::Bell, 7000.0, 2.0, -9.0),
        (SvfMode::LowPass, 8000.0, 0.7, 0.0),
    ],
    [
        (SvfMode::HighPass, 75.0, 0.9, 0.0),
        (SvfMode::Bell, 115.0, 1.0, 2.0),
        (SvfMode::Bell, 1800.0, 1.0, 3.0),
        (SvfMode::LowPass, 5000.0, 0.8, 0.0),
        (SvfMode::Bell, 6500.0, 2.0, -8.0),
        (SvfMode::LowPass, 7500.0, 0.7, 0.0),
    ],
    [
        (SvfMode::HighPass, 70.0, 1.1, 0.0),
        (SvfMode::Bell, 100.0, 1.0, 3.5),
        (SvfMode::Bell, 450.0, 1.0, -3.0),
        (SvfMode::Bell, 2200.0, 1.2, 4.0),
        (SvfMode::LowPass, 4500.0, 0.9, 0.0),
        (SvfMode::LowPass, 6000.0, 0.7, 0.0),
    ],
    [
        (SvfMode::HighPass, 40.0, 1.0, 0.0),
        (SvfMode::Bell, 80.0, 1.0, 3.0),
        (SvfMode::Bell, 700.0, 1.0, -3.0),
        (SvfMode::Bell, 1200.0, 1.2, 2.0),
        (SvfMode::LowPass, 3500.0, 0.8, 0.0),
        (SvfMode::LowPass, 4500.0, 0.7, 0.0),
    ],
];

static AMP_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.amp",
    name: "Amp",
    kind: Kind::Effect,
};
static AMP_PARAMS: [ParamInfo; 13] = [
    param(0, "Model", "Amp", 0.0, 3.0, 1.0, NONE, Choice(MODELS)),
    param(1, "Gain", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(2, "Bass", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(3, "Mid", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(4, "Treble", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(5, "Presence", "Amp", 0.0, 10.0, 3.0, NONE, Linear),
    param(6, "Cab", "Amp", 0.0, 4.0, 3.0, NONE, Choice(CABS)),
    param(7, "Level", "Amp", -24.0, 12.0, 0.0, Db, Linear),
    param(8, "Boost", "Amp", 0.0, 2.0, 0.0, NONE, Choice(BOOSTS)),
    param(9, "Gate", "Amp", 0.0, 10.0, 0.0, NONE, Linear),
    param(10, "Release", "Amp", 20.0, 2000.0, 150.0, Ms, Log),
    param(11, "Sag", "Amp", 0.0, 10.0, 0.0, NONE, Linear),
    param(12, "Depth", "Amp", 0.0, 10.0, 0.0, NONE, Linear),
];

/// Amp factory presets.
pub static AMP_PRESETS: &[Preset] = &[
    Preset {
        name: "Sparkle clean",
        values: &[
            (amp::MODEL, 0.0),
            (amp::GAIN, 3.0),
            (amp::TREBLE, 7.0),
            (amp::CAB, 1.0),
            (amp::LEVEL, -0.7),
        ],
    },
    Preset {
        name: "Edge of breakup",
        values: &[
            (amp::MODEL, 0.0),
            (amp::GAIN, 7.0),
            (amp::MID, 6.0),
            (amp::TREBLE, 6.0),
            (amp::SAG, 5.0),
            (amp::CAB, 2.0),
            (amp::LEVEL, 0.7),
        ],
    },
    Preset {
        name: "Classic crunch",
        values: &[(amp::MODEL, 1.0), (amp::GAIN, 6.0), (amp::CAB, 3.0)],
    },
    Preset {
        name: "Modern lead",
        values: &[
            (amp::MODEL, 2.0),
            (amp::GAIN, 8.0),
            (amp::MID, 3.0),
            (amp::CAB, 3.0),
            (amp::LEVEL, 0.6),
        ],
    },
    Preset {
        name: "Tight rhythm",
        values: &[
            (amp::MODEL, 2.0),
            (amp::GAIN, 7.0),
            (amp::MID, 4.0),
            (amp::BOOST, 1.0),
            (amp::GATE, 5.0),
            (amp::DEPTH, 4.0),
            (amp::CAB, 3.0),
            (amp::LEVEL, -1.0),
        ],
    },
    Preset {
        name: "Treble lead",
        values: &[
            (amp::MODEL, 1.0),
            (amp::GAIN, 6.0),
            (amp::MID, 6.0),
            (amp::BOOST, 2.0),
            (amp::CAB, 2.0),
            (amp::LEVEL, 0.6),
        ],
    },
    Preset {
        name: "Bass grit",
        values: &[
            (amp::MODEL, 3.0),
            (amp::GAIN, 4.0),
            (amp::BASS, 6.0),
            (amp::CAB, 4.0),
            (amp::LEVEL, 2.0),
        ],
    },
    Preset {
        name: "Synth grit",
        values: &[
            (amp::MODEL, 1.0),
            (amp::GAIN, 4.0),
            (amp::BASS, 6.0),
            (amp::TREBLE, 6.0),
            (amp::PRESENCE, 5.0),
            (amp::CAB, 1.0),
            (amp::LEVEL, -0.7),
        ],
    },
    Preset {
        name: "Lo-fi radio",
        values: &[
            (amp::MODEL, 1.0),
            (amp::GAIN, 7.0),
            (amp::BASS, 0.0),
            (amp::TREBLE, 3.0),
            (amp::CAB, 1.0),
            (amp::LEVEL, 1.9),
        ],
    },
];

/// The half-bands: the first stage (the base rate to twice it) five
/// sections with a transition half-width of 0.06, the second (twice to four
/// times) four with 0.12. Each [`IirPair`] holds one design's up-sampler
/// and its down-sampler.
const HB1_N: usize = 5;
const HB1_TBW: f64 = 0.06;
const HB2_N: usize = 4;
const HB2_TBW: f64 = 0.12;

/// The half-bands drop what is too small to hear every this many frames
/// from the stream's start (the same in any block size).
const SETTLE: u32 = 32;

/// A one-pole coefficient at `hz` for a stream at `rate`: the low-pass's
/// step `1 - e^(-2 pi f / fs)` and the high-pass's `e^(-2 pi f / fs)`.
fn one_pole(hz: f32, rate: u32) -> (f32, f32) {
    let p = (-std::f32::consts::TAU * hz / rate as f32).exp();
    (1.0 - p, p)
}

/// The coefficients every [`Core`] and [`Boost`] shares, at 4x.
#[derive(Clone, Copy, Debug, Default)]
struct Coefs {
    /// The stages' coupling high-pass (30 Hz) and Miller low-pass (9 kHz).
    hp_k: f32,
    lp_k: f32,
    /// Tight: the clipping branch's 720 Hz high-pass, the 3 kHz tone
    /// low-pass, and the 120 Hz cut of its output.
    ts_hp: f32,
    ts_lp: f32,
    ts_cut: f32,
    /// Treble: the input's 1.5 kHz high-pass.
    tb_hp: f32,
}

/// What a model runs at four times the rate: the tightening high-pass,
/// one to three clipping stages, the tone stack, Depth and the power
/// stage with its sag. A Model change runs two of these for 10 ms and
/// crossfades.
#[derive(Clone, Debug)]
struct Core {
    model: usize,
    tight: Svf,
    adaa: Adaa,
    /// `tanh` of the bias, taken off each stage so silence stays silence.
    tb: f32,
    lp: [f32; 3],
    hp: [f32; 3],
    hp_x: [f32; 3],
    stack: ToneStack,
    depth: Svf,
    depth_on: bool,
    /// The power stage's load, followed (Sag), and the supply it leaves.
    load: f32,
    supply: f32,
    /// The gains at Gain knob position `knob` (0 to 1): the first stage's,
    /// the later stages', and the output trim.
    knob: f32,
    g1: f32,
    later: f32,
    trim: f32,
}

impl Core {
    fn new() -> Core {
        Core {
            model: 1,
            tight: Svf::default(),
            adaa: Adaa::default(),
            tb: 0.0,
            lp: [0.0; 3],
            hp: [0.0; 3],
            hp_x: [0.0; 3],
            stack: ToneStack::default(),
            depth: Svf::default(),
            depth_on: false,
            load: 0.0,
            supply: 1.0,
            knob: f32::NAN,
            g1: 1.0,
            later: 1.0,
            trim: 1.0,
        }
    }

    fn data(&self) -> &'static Model {
        &MODEL_DATA[self.model]
    }

    fn reset(&mut self) {
        self.tight.reset();
        self.lp = [0.0; 3];
        self.hp = [0.0; 3];
        self.hp_x = [0.0; 3];
        self.stack.reset();
        self.depth.reset();
        self.load = 0.0;
        self.supply = 1.0;
        // At rest the first stage sees its bias.
        self.adaa.seed(self.data().bias, tanh_big);
    }

    /// Switches to `model`, keeping the filters' memories.
    fn set_model(&mut self, model: usize, rate4: u32) {
        let old = self.data().bias;
        self.model = model.min(MODEL_DATA.len() - 1);
        let m = self.data();
        self.tight
            .set(SvfMode::HighPass, m.tight_hz, 0.707, 0.0, rate4);
        self.tb = fast_tanh(m.bias);
        // The first stage's last input moves with the bias, so the next
        // average is over the signal's own step, not the jump in bias.
        let u = self.adaa.last() - old + m.bias;
        self.adaa.seed(u, tanh_big);
        self.knob = f32::NAN;
    }

    /// The stack at knob positions `tone` (treble, mid, bass; 0 to 1).
    fn set_stack(&mut self, tone: [f32; 3], rate4: u32) {
        let m = self.data();
        self.stack.set(&m.stack, tone[0], tone[1], tone[2], rate4);
    }

    /// Depth: a low bell at the model's resonance, 0 to 10 dB.
    fn set_depth(&mut self, depth: f32, rate4: u32) {
        let hz = self.data().depth_hz;
        self.depth_on = depth > 0.0;
        self.depth.set(SvfMode::Bell, hz, 0.9, depth, rate4);
    }

    /// Takes the Gain knob's position `k` (0 to 1) when it has moved.
    #[inline]
    fn set_knob(&mut self, k: f32) {
        if k == self.knob {
            return;
        }
        self.knob = k;
        let m = self.data();
        self.g1 = db_to_gain(m.max_gain_db * k);
        self.later = 1.0 + 2.0 * k;
        self.trim = db_to_gain(trim_at(&m.trim_db, k));
    }

    /// One tube-like stage: gain, an asymmetric tanh (a shifted curve, so
    /// the two halves clip differently), a Miller low-pass and a coupling
    /// high-pass that removes the offset. The first stage's curve is
    /// averaged by ADAA: it takes the most gain and squares the wave off,
    /// and what it makes above the 4x Nyquist would fold back. The later
    /// stages clip what the Miller pole has already smoothed, and ADAA on
    /// them changed the worst alias by under 0.5 dB for half again the
    /// cost, so they take the plain curve.
    #[inline]
    fn stage(&mut self, k: usize, x: f32, g: f32, bias: f32, c: &Coefs) -> f32 {
        let u = x * g + bias;
        let y = if k == 0 {
            self.adaa.run(u, fast_tanh, tanh_big)
        } else {
            fast_tanh(u)
        } - self.tb;
        self.lp[k] = flush(self.lp[k] + (y - self.lp[k]) * c.lp_k);
        let z = self.lp[k];
        // One-pole high-pass: y = a (y1 + x - x1).
        self.hp[k] = flush(c.hp_k * (self.hp[k] + z - self.hp_x[k]));
        self.hp_x[k] = z;
        self.hp[k]
    }

    /// One sample at 4x.
    #[inline]
    fn run(&mut self, x: f32, c: &Coefs) -> f32 {
        let m = self.data();
        let x = self.tight.run(0, x);
        let mut y = self.stage(0, x, self.g1, m.bias, c);
        for k in 1..m.stages {
            y = self.stage(k, y, self.later, m.bias, c);
        }
        // The stack loses about 10 to 20 dB, as the real one does; the
        // power stage gets it back and rounds the loudest peaks.
        let mut y = self.stack.run(y) * 4.0;
        if self.depth_on {
            y = self.depth.run(0, y);
        }
        // The supply's sag lowers the power stage's ceiling. It clips
        // gently and at 4x, so its plain curve aliases under the stages'
        // (ADAA on it moved no alias row).
        let s = self.supply;
        let p = fast_tanh(y * (0.3 / s)) * (s * (1.0 / 0.3));
        p * self.trim
    }

    /// Once a base frame, after its four samples: the power stage's load
    /// `level` (its output's mean size), followed fast up and slowly down,
    /// lowers the supply by up to `sag` (0 to 1).
    #[inline]
    fn follow(&mut self, level: f32, sag: f32, up: f32, down: f32) {
        let k = if level > self.load { up } else { down };
        self.load = flush(self.load + (level - self.load) * k);
        self.supply = 1.0 / (1.0 + 1.5 * sag * self.load);
    }
}

/// A pedal in front of the amp, at 4x: Tight (a Tube Screamer style mid
/// hump, its drive low and level up, with the low end cut, as metal
/// players use it to tighten a high-gain amp) or Treble (a treble
/// booster's thin, hot top end).
#[derive(Clone, Copy, Debug, Default)]
struct Boost {
    hp: f32,
    hp_x: f32,
    lp: f32,
    cut: f32,
    clip: Adaa,
}

/// The Treble booster's bias: its transistor clips one side first.
const TREBLE_BIAS: f32 = 0.3;

impl Boost {
    fn reset(&mut self, kind: usize) {
        *self = Boost::default();
        let rest = if kind == 2 { TREBLE_BIAS } else { 0.0 };
        self.clip.seed(rest, tanh_big);
    }

    #[inline]
    fn run(&mut self, kind: usize, x: f32, c: &Coefs) -> f32 {
        match kind {
            1 => {
                // The clipping branch: a one-pole high-pass at 720 Hz,
                // about 21 dB of gain, soft symmetric clipping.
                self.hp = flush(c.ts_hp * (self.hp + x - self.hp_x));
                self.hp_x = x;
                let clip = 0.5 * self.clip.run(24.0 * self.hp, fast_tanh, tanh_big);
                // The sum loses its lows under 120 Hz, the clipping's
                // difference tones with them.
                let y = x + clip;
                self.cut = flush(self.cut + (y - self.cut) * c.ts_cut);
                let y = y - self.cut;
                self.lp = flush(self.lp + (y - self.lp) * c.ts_lp);
                1.4 * self.lp
            }
            2 => {
                self.hp = flush(c.tb_hp * (self.hp + x - self.hp_x));
                self.hp_x = x;
                let u = 10.0 / 0.6 * self.hp + TREBLE_BIAS;
                0.6 * (self.clip.run(u, fast_tanh, tanh_big) - fast_tanh(TREBLE_BIAS))
            }
            _ => x,
        }
    }
}

/// The gate in front: it opens at once when the input's peak passes the
/// threshold, holds 30 ms after it falls 6 dB under, then fades out over
/// Release.
#[derive(Clone, Copy, Debug, Default)]
struct InGate {
    on: bool,
    env: f32,
    fall: f32,
    open_at: f32,
    shut_at: f32,
    open: bool,
    held: u32,
    hold: u32,
    g: f32,
    att: f32,
    rel: f32,
}

const GATE_HOLD_MS: f32 = 30.0;
const GATE_HYSTERESIS_DB: f32 = 6.0;

impl InGate {
    /// Gate 0 is off; 1 to 10 put the threshold at -80 to -35 dBFS.
    fn set(&mut self, gate: f32, release_ms: f32, rate: u32) {
        let fs = rate as f32;
        self.on = gate > 0.0;
        let thr = -85.0 + 5.0 * gate;
        self.open_at = db_to_gain(thr);
        self.shut_at = db_to_gain(thr - GATE_HYSTERESIS_DB);
        // The detector falls 60 dB in 20 ms; it opens over half a ms and
        // fades 80 dB over Release.
        self.fall = db_to_gain(-60.0 / (0.02 * fs));
        self.att = 1.0 / (0.0005 * fs).max(1.0);
        self.rel = db_to_gain(-80.0 / (release_ms * 0.001 * fs).max(1.0));
        self.hold = (GATE_HOLD_MS * 0.001 * fs) as u32;
    }

    fn reset(&mut self) {
        self.env = 0.0;
        self.open = false;
        self.held = 0;
        self.g = 0.0;
    }

    #[inline]
    fn run(&mut self, x: f32) -> f32 {
        if !self.on {
            return x;
        }
        let level = x.abs();
        self.env = if level > self.env {
            level
        } else {
            flush(self.env * self.fall)
        };
        if self.env >= self.open_at {
            self.open = true;
            self.held = self.hold;
        } else if self.env < self.shut_at {
            if self.held > 0 {
                self.held -= 1;
            } else {
                self.open = false;
            }
        }
        if self.open {
            self.g = (self.g + self.att).min(1.0);
        } else if self.g > 0.0 {
            self.g *= self.rel;
            if self.g < 1e-4 {
                self.g = 0.0;
            }
        }
        x * self.g
    }
}

/// Crossfades last 10 ms.
const FADE_MS: f32 = 10.0;
/// The tone stack's values are worked out again every 4 frames while its
/// knobs glide, on a grid counted from the start, so any block size
/// sounds the same.
const STACK_EVERY: u32 = 4;

/// A guitar or bass amp: a gate, a boost pedal, a tightening high-pass,
/// one to three tube-like gain stages (asymmetric, so they add even
/// harmonics as tubes do), the treble, mid and bass tone stack worked out
/// from the circuit itself, Depth, a power stage that rounds the peaks and
/// sags under load, presence, and a speaker cabinet. Everything from the
/// boost to the power stage runs at four times the rate, the first stage's
/// curve antialiased (ADAA). Mono in, the same sound on both sides out.
pub struct Amp {
    v: [f32; 13],
    rate: u32,
    c: Coefs,
    gate: InGate,
    /// The first stage's half-bands (up first, down last) and the
    /// second's.
    hb1: IirPair<HB1_N>,
    hb2: IirPair<HB2_N>,
    boost: usize,
    boosts: [Boost; 2],
    boost_old: usize,
    boost_fade: u32,
    core: Core,
    old: Core,
    model_fade: u32,
    /// A 12 kHz low-pass at 4x before the rate comes down, so what the
    /// stages made above hearing can't fold back.
    guard: Svf,
    presence: Svf,
    cab: [Svf; 6],
    cab_old: [Svf; 6],
    cab_idx: usize,
    cab_old_on: bool,
    cab_fade: u32,
    fade_len: u32,
    /// The Gain knob (0 to 1), the tone stack's knobs (treble, mid, bass;
    /// 0 to 1) and Level, gliding.
    knob: Smooth,
    tone: [Smooth; 3],
    tone_set: [f32; 3],
    level: Smooth,
    /// Sag (0 to 1) and its follower's coefficients.
    sag: f32,
    sag_up: f32,
    sag_down: f32,
    frame: u32,
}

impl Default for Amp {
    fn default() -> Self {
        Self::new()
    }
}

impl Amp {
    pub fn new() -> Amp {
        let v = defaults(&AMP_PARAMS);
        let mut a = Amp {
            v,
            rate: 48_000,
            c: Coefs::default(),
            gate: InGate::default(),
            hb1: IirPair::new(HB1_TBW),
            hb2: IirPair::new(HB2_TBW),
            boost: 0,
            boosts: [Boost::default(); 2],
            boost_old: 0,
            boost_fade: 0,
            core: Core::new(),
            old: Core::new(),
            model_fade: 0,
            guard: Svf::default(),
            presence: Svf::default(),
            cab: [Svf::default(); 6],
            cab_old: [Svf::default(); 6],
            cab_idx: 3,
            cab_old_on: false,
            cab_fade: 0,
            fade_len: 1,
            knob: Smooth::new(0.5),
            tone: [Smooth::new(0.5); 3],
            tone_set: [0.5; 3],
            level: Smooth::new(1.0),
            sag: 0.0,
            sag_up: 1.0,
            sag_down: 1.0,
            frame: 0,
        };
        a.setup(48_000);
        a
    }

    fn rate4(&self) -> u32 {
        4 * self.rate
    }

    fn model_index(&self) -> usize {
        (self.v[amp::MODEL as usize] as usize).min(MODEL_DATA.len() - 1)
    }

    fn boost_index(&self) -> usize {
        (self.v[amp::BOOST as usize] as usize).min(2)
    }

    fn tone_targets(&self) -> [f32; 3] {
        [
            self.v[amp::TREBLE as usize] / 10.0,
            self.v[amp::MID as usize] / 10.0,
            self.v[amp::BASS as usize] / 10.0,
        ]
    }

    fn set_cab(&mut self) {
        let c = self.cab_idx;
        if c == 0 {
            return;
        }
        for (s, &(mode, hz, q, db)) in self.cab.iter_mut().zip(&CAB_DATA[c - 1]) {
            s.set(mode, hz, q, db, self.rate);
        }
    }

    /// Sets up a model on `core` from scratch (setup and reset).
    fn fresh_core(&mut self) {
        let (m, r4) = (self.model_index(), self.rate4());
        self.core.set_model(m, r4);
        self.core.set_stack(self.tone_set, r4);
        self.core.set_depth(self.v[amp::DEPTH as usize], r4);
    }

    fn apply(&mut self, id: u16, v: f32) {
        let r4 = self.rate4();
        match id {
            amp::MODEL => {
                let m = self.model_index();
                if m == self.core.model {
                    return;
                }
                // The old model plays on for 10 ms while the new one,
                // starting from its memories, fades in.
                self.old = self.core.clone();
                self.core.set_model(m, r4);
                self.core.set_stack(self.tone_set, r4);
                self.core.set_depth(self.v[amp::DEPTH as usize], r4);
                self.model_fade = self.fade_len;
            }
            amp::GAIN => self.knob.target = v / 10.0,
            amp::BASS | amp::MID | amp::TREBLE => {
                let t = self.tone_targets();
                for (s, t) in self.tone.iter_mut().zip(t) {
                    s.target = t;
                }
            }
            amp::PRESENCE => self
                .presence
                .set(SvfMode::HighShelf, 3500.0, 0.7, 0.8 * v, self.rate),
            amp::CAB => {
                let c = (v as usize).min(CAB_DATA.len());
                if c == self.cab_idx {
                    return;
                }
                // The old cab, its memory kept, fades out over 10 ms; the
                // new one starts from that memory (or from rest, after
                // Off).
                self.cab_old = self.cab;
                self.cab_old_on = self.cab_idx > 0;
                if self.cab_idx == 0 {
                    for s in &mut self.cab {
                        s.reset();
                    }
                }
                self.cab_idx = c;
                self.set_cab();
                self.cab_fade = self.fade_len;
            }
            amp::LEVEL => self.level.target = db_to_gain(v),
            amp::BOOST => {
                let b = self.boost_index();
                if b == self.boost {
                    return;
                }
                if b > 0 {
                    self.boosts[b - 1].reset(b);
                }
                self.boost_old = self.boost;
                self.boost = b;
                self.boost_fade = self.fade_len;
            }
            amp::GATE | amp::RELEASE => self.gate.set(
                self.v[amp::GATE as usize],
                self.v[amp::RELEASE as usize],
                self.rate,
            ),
            amp::SAG => self.sag = v / 10.0,
            amp::DEPTH => {
                self.core.set_depth(v, r4);
                self.old.set_depth(v, r4);
            }
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let c = self.c;
        let inv_fade = 1.0 / self.fade_len as f32;
        for i in 0..l.len() {
            // The knobs glide; the stack follows on its grid.
            let k = self.knob.glide();
            self.core.set_knob(k);
            let mut t = [0.0f32; 3];
            for (t, s) in t.iter_mut().zip(&mut self.tone) {
                *t = s.glide();
            }
            if self.frame.is_multiple_of(STACK_EVERY) && t != self.tone_set {
                self.tone_set = t;
                let r4 = self.rate4();
                self.core.set_stack(t, r4);
                if self.model_fade > 0 {
                    self.old.set_stack(t, r4);
                }
            }
            self.frame = self.frame.wrapping_add(1);

            if self.frame.is_multiple_of(SETTLE) {
                self.hb1.settle();
                self.hb2.settle();
            }
            let x = self.gate.run(0.5 * (l[i] + r[i]));
            let mut quad = [0.0f32; 4];
            let [a, b] = self.hb1.up(x);
            for (h, v) in [a, b].into_iter().enumerate() {
                let [c0, c1] = self.hb2.up(v);
                quad[2 * h] = c0;
                quad[2 * h + 1] = c1;
            }
            // The boost, crossfaded for 10 ms after a change.
            if self.boost_fade > 0 {
                let w = 1.0 - self.boost_fade as f32 * inv_fade;
                self.boost_fade -= 1;
                let (new, old) = (self.boost, self.boost_old);
                for q in &mut quad {
                    let n = self.run_boost(new, *q);
                    let o = self.run_boost(old, *q);
                    *q = o + (n - o) * w;
                }
            } else if self.boost > 0 {
                let b = self.boost;
                for q in &mut quad {
                    *q = self.boosts[b - 1].run(b, *q, &c);
                }
            }
            // The model, crossfaded for 10 ms after a change.
            let mut size = 0.0f32;
            if self.model_fade > 0 {
                let w = 1.0 - self.model_fade as f32 * inv_fade;
                self.model_fade -= 1;
                self.old.set_knob(k);
                let mut old_size = 0.0f32;
                for q in &mut quad {
                    let n = self.core.run(*q, &c);
                    let o = self.old.run(*q, &c);
                    size += n.abs();
                    old_size += o.abs();
                    *q = self.guard.run(0, o + (n - o) * w);
                }
                self.old.follow(
                    old_size * 0.25 * 0.3 / self.old.trim,
                    self.sag,
                    self.sag_up,
                    self.sag_down,
                );
            } else {
                for q in &mut quad {
                    let n = self.core.run(*q, &c);
                    size += n.abs();
                    *q = self.guard.run(0, n);
                }
            }
            self.core.follow(
                size * 0.25 * 0.3 / self.core.trim,
                self.sag,
                self.sag_up,
                self.sag_down,
            );
            let h0 = self.hb2.down([quad[0], quad[1]]);
            let h1 = self.hb2.down([quad[2], quad[3]]);
            let mut y = self.hb1.down([h0, h1]);
            y = self.presence.run(0, y);
            // The cab, crossfaded for 10 ms after a change.
            let mut wet = y;
            if self.cab_idx > 0 {
                for s in &mut self.cab {
                    wet = s.run(0, wet);
                }
            }
            if self.cab_fade > 0 {
                let w = 1.0 - self.cab_fade as f32 * inv_fade;
                self.cab_fade -= 1;
                let mut o = y;
                if self.cab_old_on {
                    for s in &mut self.cab_old {
                        o = s.run(0, o);
                    }
                }
                wet = o + (wet - o) * w;
            }
            let out = wet * self.level.glide() * 0.5;
            l[i] = out;
            r[i] = out;
        }
    }

    /// The boost `kind` (0 is Off) on one 4x sample.
    #[inline]
    fn run_boost(&mut self, kind: usize, x: f32) -> f32 {
        if kind == 0 {
            x
        } else {
            let c = self.c;
            self.boosts[kind - 1].run(kind, x, &c)
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let r4 = 4 * rate;
        let (lp_k, _) = one_pole(9000.0, r4);
        let (_, hp_k) = one_pole(30.0, r4);
        let (ts_lp, _) = one_pole(3000.0, r4);
        let (ts_cut, _) = one_pole(120.0, r4);
        let (_, ts_hp) = one_pole(720.0, r4);
        let (_, tb_hp) = one_pole(1500.0, r4);
        self.c = Coefs {
            hp_k,
            lp_k,
            ts_hp,
            ts_lp,
            ts_cut,
            tb_hp,
        };
        // Sag: the supply drops within about 10 ms and recovers over 150.
        self.sag_up = 1.0 - (-1.0 / (0.010 * rate as f32)).exp();
        self.sag_down = 1.0 - (-1.0 / (0.150 * rate as f32)).exp();
        self.guard.set(SvfMode::LowPass, 12_000.0, 0.707, 0.0, r4);
        self.fade_len = ((FADE_MS * 0.001 * rate as f32) as u32).max(1);
        self.knob.set_time(rate, 20.0);
        for s in &mut self.tone {
            s.set_time(rate, 20.0);
        }
        self.level.set_time(rate, 20.0);
        self.knob.target = self.v[amp::GAIN as usize] / 10.0;
        let t = self.tone_targets();
        for (s, t) in self.tone.iter_mut().zip(t) {
            s.target = t;
        }
        self.level.target = db_to_gain(self.v[amp::LEVEL as usize]);
        self.tone_set = t;
        self.fresh_core();
        self.cab_idx = (self.v[amp::CAB as usize] as usize).min(CAB_DATA.len());
        self.set_cab();
        self.boost = self.boost_index();
        for id in [amp::PRESENCE, amp::GATE, amp::SAG] {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.gate.reset();
        self.hb1.reset();
        self.hb2.reset();
        for (k, b) in self.boosts.iter_mut().enumerate() {
            b.reset(k + 1);
        }
        self.boost_old = self.boost;
        self.boost_fade = 0;
        self.model_fade = 0;
        self.cab_fade = 0;
        self.knob.snap(self.knob.target);
        for s in &mut self.tone {
            s.snap(s.target);
        }
        self.level.snap(self.level.target);
        let t = self.tone_targets();
        if t != self.tone_set {
            self.tone_set = t;
            self.core.set_stack(t, self.rate4());
        }
        self.core.reset();
        self.core.set_knob(self.knob.target);
        self.guard.reset();
        self.presence.reset();
        for s in &mut self.cab {
            s.reset();
        }
        self.frame = 0;
    }

    fn tail(&self) -> u32 {
        self.rate / 20
    }
    fn rest(&self) -> u32 {
        // Its filters ring out within the tail; the gate's fade and the
        // sag's recovery take at most Release and about a second.
        let ms = self.v[amp::RELEASE as usize] + 1000.0;
        self.tail() + (ms * 0.001 * self.rate as f32) as u32
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Amp, AMP_INFO, AMP_PARAMS, rest);

// ---------------------------------------------------------------- exciter

/// [`Exciter`] parameter ids.
pub mod exciter {
    pub const FREQ: u16 = 0;
    pub const DRIVE: u16 = 1;
    pub const COLOUR: u16 = 2;
    pub const AMOUNT: u16 = 3;
}

static EXCITER_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.exciter",
    name: "Exciter",
    kind: Kind::Effect,
};
static EXCITER_PARAMS: [ParamInfo; 4] = [
    param(
        0,
        "Freq",
        "Exciter",
        1000.0,
        10_000.0,
        3000.0,
        Hz,
        crate::kit::Log,
    ),
    param(1, "Drive", "Exciter", 0.0, 100.0, 50.0, Percent, Linear),
    param(2, "Colour", "Exciter", 0.0, 100.0, 50.0, Percent, Linear),
    param(3, "Amount", "Exciter", 0.0, 100.0, 30.0, Percent, Linear),
];

/// Exciter factory presets.
pub static EXCITER_PRESETS: &[Preset] = &[
    Preset {
        name: "Vocal air",
        values: &[(exciter::FREQ, 5000.0), (exciter::AMOUNT, 25.0)],
    },
    Preset {
        name: "Hat sparkle",
        values: &[
            (exciter::FREQ, 7000.0),
            (exciter::COLOUR, 20.0),
            (exciter::AMOUNT, 35.0),
        ],
    },
    Preset {
        name: "Warm presence",
        values: &[
            (exciter::FREQ, 2000.0),
            (exciter::COLOUR, 90.0),
            (exciter::DRIVE, 70.0),
        ],
    },
];

/// An exciter: the band above Freq is driven through a waveshaper and the
/// harmonics it makes (filtered again, so only new top end is added) are
/// mixed in under the dry sound. Colour turns from odd harmonics (crisp)
/// to even (warm). The shaper runs at twice the rate.
pub struct Exciter {
    v: [f32; 4],
    rate: u32,
    hp_in: Svf,
    hp_out: Svf,
    /// Each side's half-bands round the shaper (the first stage's design).
    hb: [IirPair<HB1_N>; 2],
    /// Frames since the stream started, modulo [`SETTLE`].
    settle: u32,
    drive: Smooth,
    colour: Smooth,
    amount: Smooth,
}

impl Default for Exciter {
    fn default() -> Self {
        Self::new()
    }
}

impl Exciter {
    pub fn new() -> Exciter {
        let v = defaults(&EXCITER_PARAMS);
        let mut e = Exciter {
            v,
            rate: 48_000,
            hp_in: Svf::default(),
            hp_out: Svf::default(),
            hb: [IirPair::new(HB1_TBW), IirPair::new(HB1_TBW)],
            settle: 0,
            drive: Smooth::new(0.5),
            colour: Smooth::new(0.5),
            amount: Smooth::new(0.3),
        };
        e.setup(48_000);
        e
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            exciter::FREQ => {
                // Fourth-order cuts in: 12 dB each, on the way in and out.
                self.hp_in.set(SvfMode::HighPass, v, 0.707, 0.0, self.rate);
                self.hp_out.set(SvfMode::HighPass, v, 0.707, 0.0, self.rate);
            }
            exciter::DRIVE => self.drive.target = v / 100.0,
            exciter::COLOUR => self.colour.target = v / 100.0,
            exciter::AMOUNT => self.amount.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            if self.settle == 0 {
                for h in &mut self.hb {
                    h.settle();
                }
            }
            self.settle = (self.settle + 1) % SETTLE;
            let drive = 1.0 + 9.0 * self.drive.glide();
            let undo = 1.0 / drive;
            let colour = self.colour.glide();
            let amount = self.amount.glide();
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let band = self.hp_in.run(ch, *x);
                let two = self.hb[ch].up(band * drive);
                let shaped = two.map(|v| {
                    // Odd: a symmetric curve less its straight part. Even:
                    // a rectified one (its DC goes in the high-pass after),
                    // and tanh(|v|) is |tanh(v)|, so one curve serves both.
                    let t = fast_tanh(v);
                    lerp(t - v * 0.5, t.abs(), colour)
                });
                let h = self.hb[ch].down(shaped) * undo;
                let h = self.hp_out.run(ch, h);
                *x += amount * h;
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        for s in [&mut self.drive, &mut self.colour, &mut self.amount] {
            s.set_time(rate, 20.0);
        }
        for id in 0..EXCITER_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }
    fn clear(&mut self) {
        self.hp_in.reset();
        self.hp_out.reset();
        for h in &mut self.hb {
            h.reset();
        }
        self.settle = 0;
        for s in [&mut self.drive, &mut self.colour, &mut self.amount] {
            s.snap(s.target);
        }
    }
    fn tail(&self) -> u32 {
        0
    }
    fn rest(&self) -> u32 {
        // Its high-passes and the oversampler's half-bands settle within
        // a few ms.
        self.rate / 20
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Exciter, EXCITER_INFO, EXCITER_PARAMS, rest);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_stack_matches_the_circuit_at_noon() {
        // Yeh and Smith's tweed stack at noon has its mid scoop near
        // 400 Hz to 1 kHz, about 12 dB under the lows (the circuit solved
        // numerically gives -1.6 dB at 50 Hz and -11.7 dB at 1 kHz).
        let mut s = ToneStack::default();
        s.set(&STACK_TWEED, 0.5, 0.5, 0.0, 48_000);
        // Bass at 0 on the knob is l near 0; check the knob mapping at
        // noon separately: (10^1 - 1) / 99 is about 0.09.
        let mut noon = ToneStack::default();
        // Bass knob giving l = 0.5 exactly: 10^(2x) = 50.5.
        let x = (50.5f64.log10() / 2.0) as f32;
        noon.set(&STACK_TWEED, 0.5, 0.5, x, 48_000);
        let at = |hz| noon.response_db(hz, 48_000);
        assert!((at(50.0) + 1.64).abs() < 0.3, "{}", at(50.0));
        assert!((at(1000.0) + 11.75).abs() < 0.3, "{}", at(1000.0));
        assert!((at(5000.0) + 5.29).abs() < 0.5, "{}", at(5000.0));
        // Less bass knob, less bass.
        assert!(s.response_db(60.0, 48_000) < at(60.0) - 3.0);
    }

    #[test]
    fn the_first_stage_follows_the_plain_curve_on_slow_signals() {
        // ADAA averages the curve between two inputs: on a slow 50 Hz sine
        // at the 4x rate that is the plain curve at their midpoint, within
        // rounding, at every model's top gain.
        let rate4 = 4 * 48_000;
        let c = Coefs {
            hp_k: 1.0,
            lp_k: 1.0,
            ..Coefs::default()
        };
        for model in 0..MODEL_DATA.len() {
            let mut core = Core::new();
            core.set_model(model, rate4);
            core.reset();
            let m = &MODEL_DATA[model];
            let g = db_to_gain(m.max_gain_db);
            let mut last = 0.0f32;
            let mut worst = 0.0f32;
            for i in 0..rate4 / 25 {
                let x = 0.1 * (std::f32::consts::TAU * 50.0 * i as f32 / rate4 as f32).sin();
                // With no filters around it (hp_k 1 passes, lp_k 1 holds
                // nothing back), the stage is the curve less its offset.
                core.hp = [0.0; 3];
                core.hp_x = [0.0; 3];
                let y = core.stage(0, x, g, m.bias, &c);
                let mid = 0.5 * (x + last);
                let want = fast_tanh(mid * g + m.bias) - fast_tanh(m.bias);
                if i > 0 {
                    worst = worst.max((y - want).abs());
                }
                last = x;
            }
            assert!(worst < 2e-3, "model {model}: {worst}");
        }
    }
}
