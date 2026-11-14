//! Guitar and bass colour: [`Amp`] (preamp stages, a tone stack modelled
//! from the circuit, a power stage, presence and a speaker cabinet) and
//! [`Exciter`] (harmonics added to the top end).
//!
//! The amp runs mono (an amp is), with its clipping stages at four times
//! the rate between polyphase IIR half-bands, so high gain doesn't alias.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::kit::{node_impl, param, Choice, Db, Glide, Hz, Linear, Percent, NONE};
use crate::oversample::{IirDown2, IirUp2};
use crate::svf::{Svf, SvfMode};
use crate::{db_to_gain, defaults, fast_tanh, flush, lerp, soft, Preset};

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

/// [`Amp`] parameter ids.
pub mod amp {
    pub const MODEL: u16 = 0;
    pub const GAIN: u16 = 1;
    pub const BASS: u16 = 2;
    pub const MID: u16 = 3;
    pub const TREBLE: u16 = 4;
    pub const PRESENCE: u16 = 5;
    pub const CAB: u16 = 6;
    pub const LEVEL: u16 = 7;
}

const MODELS: &[&str] = &["Clean", "Crunch", "Lead", "Bass"];
const CABS: &[&str] = &["Off", "1x12 open", "2x12", "4x12", "1x15 bass"];

/// What each model changes: the input high-pass (tightness), the gain at
/// the top of the Gain knob, how many clipping stages, their bias (even
/// harmonics), the stack and the cab a new amp starts with.
struct Model {
    tight_hz: f32,
    max_gain_db: f32,
    stages: usize,
    bias: f32,
    stack: Stack,
}

const MODEL_DATA: [Model; 4] = [
    Model {
        tight_hz: 60.0,
        max_gain_db: 24.0,
        stages: 1,
        bias: 0.15,
        stack: STACK_TWEED,
    },
    Model {
        tight_hz: 100.0,
        max_gain_db: 40.0,
        stages: 2,
        bias: 0.25,
        stack: STACK_BRIT,
    },
    Model {
        tight_hz: 300.0,
        max_gain_db: 54.0,
        stages: 3,
        bias: 0.3,
        stack: STACK_CAL,
    },
    Model {
        tight_hz: 30.0,
        max_gain_db: 30.0,
        stages: 2,
        bias: 0.2,
        stack: STACK_TWEED,
    },
];

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
static AMP_PARAMS: [ParamInfo; 8] = [
    param(0, "Model", "Amp", 0.0, 3.0, 1.0, NONE, Choice(MODELS)),
    param(1, "Gain", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(2, "Bass", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(3, "Mid", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(4, "Treble", "Amp", 0.0, 10.0, 5.0, NONE, Linear),
    param(5, "Presence", "Amp", 0.0, 10.0, 3.0, NONE, Linear),
    param(6, "Cab", "Amp", 0.0, 4.0, 3.0, NONE, Choice(CABS)),
    param(7, "Level", "Amp", -24.0, 12.0, 0.0, Db, Linear),
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
        ],
    },
    Preset {
        name: "Bass grit",
        values: &[
            (amp::MODEL, 3.0),
            (amp::GAIN, 4.0),
            (amp::BASS, 6.0),
            (amp::CAB, 4.0),
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
        ],
    },
];

/// The clipping stages at four times the rate.
#[derive(Clone, Debug)]
struct Pre {
    up: [IirUp2; 2],
    down: [IirDown2; 2],
    /// Between stages: a coupling high-pass and a Miller low-pass, per
    /// stage (one-pole each, at the 4x rate).
    hp: [f32; 3],
    hp_x: [f32; 3],
    lp: [f32; 3],
    /// A 12 kHz low-pass after the last stage, at 4x: what the stages
    /// made above hearing goes before the rate comes down, so it can't
    /// fold back.
    guard: Svf,
}

const HB1: (usize, f64) = (5, 0.06);
const HB2: (usize, f64) = (4, 0.12);

impl Pre {
    fn new() -> Pre {
        Pre {
            up: [IirUp2::new(HB1.0, HB1.1), IirUp2::new(HB2.0, HB2.1)],
            down: [IirDown2::new(HB2.0, HB2.1), IirDown2::new(HB1.0, HB1.1)],
            hp: [0.0; 3],
            hp_x: [0.0; 3],
            lp: [0.0; 3],
            guard: Svf::default(),
        }
    }
    fn reset(&mut self) {
        for u in &mut self.up {
            u.reset();
        }
        for d in &mut self.down {
            d.reset();
        }
        self.hp = [0.0; 3];
        self.hp_x = [0.0; 3];
        self.lp = [0.0; 3];
        self.guard.reset();
    }
}

/// A guitar or bass amp: a tightening high-pass, one to three tube-like
/// gain stages (asymmetric, so they add even harmonics as tubes do) at four
/// times the rate, the treble, mid and bass tone stack worked out from the
/// circuit itself, a power stage that rounds the peaks, presence, and a
/// speaker cabinet. Mono in, the same sound on both sides out.
pub struct Amp {
    v: [f32; 8],
    rate: u32,
    pre: Pre,
    tight: Svf,
    stack: ToneStack,
    presence: Svf,
    cab: [Svf; 6],
    gain: Smooth,
    level: Smooth,
    hp_k: f32,
    lp_k: f32,
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
            pre: Pre::new(),
            tight: Svf::default(),
            stack: ToneStack::default(),
            presence: Svf::default(),
            cab: [Svf::default(); 6],
            gain: Smooth::new(1.0),
            level: Smooth::new(1.0),
            hp_k: 0.0,
            lp_k: 0.0,
        };
        a.setup(48_000);
        a
    }

    fn model(&self) -> &'static Model {
        &MODEL_DATA[(self.v[amp::MODEL as usize] as usize).min(3)]
    }

    fn gain_target(&self) -> f32 {
        let g = self.v[amp::GAIN as usize] / 10.0;
        db_to_gain(self.model().max_gain_db * g)
    }

    fn set_stack(&mut self) {
        let m = self.model();
        self.stack.set(
            &m.stack,
            self.v[amp::TREBLE as usize] / 10.0,
            self.v[amp::MID as usize] / 10.0,
            self.v[amp::BASS as usize] / 10.0,
            self.rate,
        );
    }

    fn set_cab(&mut self) {
        let c = self.v[amp::CAB as usize] as usize;
        if c == 0 {
            return;
        }
        for (s, &(mode, hz, q, db)) in self.cab.iter_mut().zip(&CAB_DATA[c - 1]) {
            s.set(mode, hz, q, db, self.rate);
        }
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            amp::MODEL => {
                let m = self.model();
                self.tight
                    .set(SvfMode::HighPass, m.tight_hz, 0.707, 0.0, self.rate);
                self.gain.target = self.gain_target();
                self.set_stack();
            }
            amp::GAIN => self.gain.target = self.gain_target(),
            amp::BASS | amp::MID | amp::TREBLE => self.set_stack(),
            amp::PRESENCE => self
                .presence
                .set(SvfMode::HighShelf, 3500.0, 0.7, 0.8 * v, self.rate),
            amp::CAB => self.set_cab(),
            amp::LEVEL => self.level.target = db_to_gain(v),
            _ => {}
        }
    }

    /// One tube-like stage at the 4x rate: gain, an asymmetric tanh (a
    /// shifted curve, so the two halves clip differently), a Miller
    /// low-pass and a coupling high-pass that removes the offset.
    #[inline]
    fn stage(pre: &mut Pre, k: usize, x: f32, g: f32, bias: f32, hp_k: f32, lp_k: f32) -> f32 {
        let y = fast_tanh(x * g + bias) - fast_tanh(bias);
        pre.lp[k] = flush(pre.lp[k] + (y - pre.lp[k]) * lp_k);
        let z = pre.lp[k];
        // One-pole high-pass: y = a (y1 + x - x1).
        pre.hp[k] = flush(hp_k * (pre.hp[k] + z - pre.hp_x[k]));
        pre.hp_x[k] = z;
        pre.hp[k]
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let m = self.model();
        let (stages, bias) = (m.stages, m.bias);
        let cab_on = self.v[amp::CAB as usize] >= 1.0;
        let (hp_k, lp_k) = (self.hp_k, self.lp_k);
        // Later stages gain 1 to 3 times with the knob; the first takes
        // the model's range.
        let later = 1.0 + 2.0 * self.v[amp::GAIN as usize] / 10.0;
        for i in 0..l.len() {
            let g = self.gain.glide();
            let x = self.tight.run(0, 0.5 * (l[i] + r[i]));
            let mut quad = [0.0f32; 4];
            let [a, b] = self.pre.up[0].run(x);
            for (h, v) in [a, b].into_iter().enumerate() {
                let [c, d] = self.pre.up[1].run(v);
                quad[2 * h] = c;
                quad[2 * h + 1] = d;
            }
            for q in &mut quad {
                let mut y = Self::stage(&mut self.pre, 0, *q, g, bias, hp_k, lp_k);
                for k in 1..stages {
                    y = Self::stage(&mut self.pre, k, y, later, bias, hp_k, lp_k);
                }
                *q = self.pre.guard.run(0, y);
            }
            let h0 = self.pre.down[0].run([quad[0], quad[1]]);
            let h1 = self.pre.down[0].run([quad[2], quad[3]]);
            let y = self.pre.down[1].run([h0, h1]);
            // The stack loses about 10 to 20 dB, as the real one does; the
            // power stage gets it back and rounds the peaks.
            let mut y = self.stack.run(y) * 4.0;
            // Gentle: it rounds the loudest peaks only (hard clipping at
            // the base rate would alias).
            y = soft(y * 0.3) * (1.0 / 0.3);
            y = self.presence.run(0, y);
            if cab_on {
                for s in &mut self.cab {
                    y = s.run(0, y);
                }
            }
            let out = y * self.level.glide() * 0.5;
            l[i] = out;
            r[i] = out;
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let fs4 = 4.0 * rate as f32;
        // A 30 Hz coupling high-pass and a 9 kHz Miller pole, at 4x.
        self.hp_k = (-std::f32::consts::TAU * 30.0 / fs4).exp();
        self.lp_k = 1.0 - (-std::f32::consts::TAU * 9000.0 / fs4).exp();
        self.pre
            .guard
            .set(SvfMode::LowPass, 12_000.0, 0.707, 0.0, 4 * rate);
        self.gain.set_time(rate, 20.0);
        self.level.set_time(rate, 20.0);
        for id in 0..AMP_PARAMS.len() as u16 {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        self.pre.reset();
        self.tight.reset();
        self.stack.reset();
        self.presence.reset();
        for s in &mut self.cab {
            s.reset();
        }
        self.gain.snap(self.gain.target);
        self.level.snap(self.level.target);
    }

    fn tail(&self) -> u32 {
        self.rate / 20
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Amp, AMP_INFO, AMP_PARAMS);

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
    up: [IirUp2; 2],
    down: [IirDown2; 2],
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
            up: [IirUp2::new(HB1.0, HB1.1), IirUp2::new(HB1.0, HB1.1)],
            down: [IirDown2::new(HB1.0, HB1.1), IirDown2::new(HB1.0, HB1.1)],
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
            let drive = 1.0 + 9.0 * self.drive.glide();
            let undo = 1.0 / drive;
            let colour = self.colour.glide();
            let amount = self.amount.glide();
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let band = self.hp_in.run(ch, *x);
                let two = self.up[ch].run(band * drive);
                let shaped = two.map(|v| {
                    // Odd: a symmetric curve less its straight part. Even:
                    // a rectified one (its DC goes in the high-pass after),
                    // and tanh(|v|) is |tanh(v)|, so one curve serves both.
                    let t = fast_tanh(v);
                    lerp(t - v * 0.5, t.abs(), colour)
                });
                let h = self.down[ch].run(shaped) * undo;
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
        for u in &mut self.up {
            u.reset();
        }
        for d in &mut self.down {
            d.reset();
        }
        for s in [&mut self.drive, &mut self.colour, &mut self.amount] {
            s.snap(s.target);
        }
    }
    fn tail(&self) -> u32 {
        0
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Exciter, EXCITER_INFO, EXCITER_PARAMS);

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
}
