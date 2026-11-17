//! [`Fuzz`]: a fuzz pedal (FX quality round 2, decision 10.6). Three
//! models: **Face** (two transistors with an asymmetric bias: one half
//! clips first on small signals, both squarer as the fuzz rises, and it
//! cleans up as the input drops), **Muff** (two clipping stages with
//! filters between them, then the scooped tone control: thick and
//! sustaining) and **Octave** (a rectifier into the fuzz: an octave up on
//! single notes, ring-modulator clusters on chords). **Gate** starves the
//! first transistor so quiet notes sputter and cut out; **Low keep** leaves
//! the lows under a crossover clean, for 808s and bass: the fuzz still
//! hears everything (so a bare sub still drives it), but only what it makes
//! above the crossover is kept, and under it the dry lows play.
//!
//! Stereo, each side at four times the rate between polyphase IIR
//! half-bands, the first curve averaged by first-order ADAA (as the Amp's
//! first stage). Each model's output follows a trim over Fuzz fitted so
//! pink noise at -18 dBFS RMS comes out as loud as it went in: the knob
//! changes the fuzz, not the loudness; Level sets the rest.

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth};

use crate::adaa::{log_cosh, Adaa};
use crate::kit::{node_impl, param, Choice, Crossover, Db, Glide, Linear, Percent, NONE};
use crate::oversample::{IirDown2, IirUp2};
use crate::svf::{Svf, SvfMode};
use crate::{db_to_gain, defaults, fast_tanh, flush, Preset};

/// [`Fuzz`] parameter ids.
pub mod fuzz_ids {
    pub const MODEL: u16 = 0;
    pub const FUZZ: u16 = 1;
    pub const TONE: u16 = 2;
    pub const GATE: u16 = 3;
    pub const LOW_KEEP: u16 = 4;
    pub const LEVEL: u16 = 5;
}
use fuzz_ids as fz;

const MODELS: &[&str] = &["Face", "Muff", "Octave"];
const KEEPS: &[&str] = &["Off", "60 Hz", "100 Hz", "150 Hz", "250 Hz"];
const KEEP_HZ: [f32; 5] = [0.0, 60.0, 100.0, 150.0, 250.0];

static FUZZ_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.fuzz",
    name: "Fuzz",
    kind: Kind::Effect,
};
static FUZZ_PARAMS: [ParamInfo; 6] = [
    param(0, "Model", "Fuzz", 0.0, 2.0, 0.0, NONE, Choice(MODELS)),
    param(1, "Fuzz", "Fuzz", 0.0, 100.0, 60.0, Percent, Linear),
    param(2, "Tone", "Fuzz", 0.0, 100.0, 50.0, Percent, Linear),
    param(3, "Gate", "Fuzz", 0.0, 100.0, 0.0, Percent, Linear),
    param(4, "Low keep", "Fuzz", 0.0, 4.0, 0.0, NONE, Choice(KEEPS)),
    param(5, "Level", "Fuzz", -24.0, 12.0, 0.0, Db, Linear),
];

/// Fuzz factory presets.
pub static FUZZ_PRESETS: &[Preset] = &[
    Preset {
        name: "Vintage face",
        values: &[
            (fz::MODEL, 0.0),
            (fz::FUZZ, 70.0),
            (fz::TONE, 55.0),
            (fz::LEVEL, -0.2),
        ],
    },
    Preset {
        name: "Thick muff",
        values: &[
            (fz::MODEL, 1.0),
            (fz::FUZZ, 75.0),
            (fz::TONE, 45.0),
            (fz::LEVEL, -0.6),
        ],
    },
    Preset {
        name: "Octave lead",
        values: &[
            (fz::MODEL, 2.0),
            (fz::FUZZ, 70.0),
            (fz::TONE, 65.0),
            (fz::LEVEL, -0.7),
        ],
    },
    Preset {
        name: "Sputter",
        values: &[
            (fz::MODEL, 0.0),
            (fz::FUZZ, 85.0),
            (fz::TONE, 50.0),
            (fz::GATE, 60.0),
            (fz::LEVEL, 0.4),
        ],
    },
    Preset {
        name: "808 fuzz",
        values: &[
            (fz::MODEL, 1.0),
            (fz::FUZZ, 60.0),
            (fz::TONE, 60.0),
            (fz::LOW_KEEP, 3.0),
            (fz::LEVEL, 1.4),
        ],
    },
    Preset {
        name: "Synth grit",
        values: &[
            (fz::MODEL, 0.0),
            (fz::FUZZ, 35.0),
            (fz::TONE, 70.0),
            (fz::LEVEL, -0.7),
        ],
    },
];

/// What each model does with the Fuzz knob, and its output trims.
struct Model {
    /// The first curve's gain at Fuzz 0 and 100, dB.
    gain_db: (f32, f32),
    /// Its bias (Face: asymmetric; Muff: none; Octave: after the
    /// rectifier).
    bias: f32,
    /// The trim in dB at Fuzz 0, 10, 20 and so on to 100 (straight lines
    /// between), fitted on pink noise at -18 dBFS RMS at Tone 50, Gate 0,
    /// Low keep off.
    trim_db: [f32; 11],
}

const MODEL_DATA: [Model; 3] = [
    Model {
        gain_db: (6.0, 42.0),
        bias: -0.35,
        trim_db: [
            -2.14, -5.48, -8.51, -11.05, -12.95, -14.23, -15.00, -15.43, -15.66, -15.78, -15.84,
        ],
    },
    Model {
        gain_db: (6.0, 30.0),
        bias: 0.0,
        trim_db: [
            -0.06, -3.54, -6.45, -8.63, -10.09, -10.97, -11.48, -11.75, -11.89, -11.96, -12.00,
        ],
    },
    Model {
        gain_db: (6.0, 44.0),
        bias: -0.2,
        trim_db: [
            0.33, -3.28, -6.60, -9.46, -11.72, -13.35, -14.39, -14.98, -15.29, -15.44, -15.52,
        ],
    },
];

fn trim_at(t: &[f32; 11], k: f32) -> f32 {
    let at = (k * 10.0).clamp(0.0, 10.0);
    let i = (at as usize).min(9);
    t[i] + (t[i + 1] - t[i]) * (at - i as f32)
}

/// The starved curve `tanh(dz(u) + b)`, where `dz` takes `d` off the
/// size of `u` (nothing passes under it), and its antiderivative. ADAA
/// averages it, so the dead zone's corners don't alias.
#[inline]
fn starved(u: f32, d: f32, b: f32) -> f32 {
    let z = if u > d {
        u - d
    } else if u < -d {
        u + d
    } else {
        0.0
    };
    fast_tanh(z + b)
}

#[inline]
fn starved_big(u: f64, d: f64, b: f64, tb: f64, lcb: f64) -> f64 {
    if u > d {
        tb * d + log_cosh((u - d + b).abs()) - lcb
    } else if u < -d {
        -tb * d + log_cosh((u + d + b).abs()) - lcb
    } else {
        tb * u
    }
}

/// The octave's rectifier `|u| + u / 4` (mostly the octave, a little of
/// the note) and its antiderivative.
#[inline]
fn rectify(u: f32) -> f32 {
    u.abs() + 0.25 * u
}

#[inline]
fn rectify_big(u: f64) -> f64 {
    0.5 * u * u.abs() + 0.125 * u * u
}

/// One side's state at 4x for one model.
#[derive(Clone, Copy, Debug, Default)]
struct Side {
    /// The first curve's ADAA (and the octave's rectifier's).
    first: Adaa,
    rect: Adaa,
    /// One-pole filters: the coupling high-passes before each stage, the
    /// Muff's low-pass after each stage, the tone's low-pass and
    /// high-pass.
    hp: [f32; 2],
    hp_x: [f32; 2],
    lp: [f32; 2],
    tone_lp: f32,
    tone_hp: f32,
    tone_hp_x: f32,
}

/// The coefficients at 4x, shared.
#[derive(Clone, Copy, Debug, Default)]
struct Coefs {
    /// Coupling high-passes: 30 Hz (Face, Octave), 100 Hz (Muff).
    hp30: f32,
    hp100: f32,
    /// The Muff's 1.2 kHz low-pass after each clipping stage.
    lp_muff: f32,
    /// The Muff's tone: a 400 Hz low-pass and a 1.6 kHz high-pass.
    tone_lo: f32,
    tone_hi: f32,
}

/// The gains and curve settings a frame uses, from the knobs.
#[derive(Clone, Copy, Debug, Default)]
struct Drive {
    g1: f32,
    g2: f32,
    /// The first curve's dead zone (Gate) and bias, `tanh` of the bias
    /// and `ln cosh` of it, for the curve and its antiderivative.
    dead: f32,
    b: f32,
    tb: f32,
    lcb: f64,
    /// Face and Octave: the tone low-pass's step; Muff: the blend.
    tone: f32,
    trim: f32,
}

impl Drive {
    #[inline]
    fn big(&self, u: f64) -> f64 {
        starved_big(
            u,
            f64::from(self.dead),
            f64::from(self.b),
            f64::from(self.tb),
            self.lcb,
        )
    }
}

impl Side {
    fn reset(&mut self, d: &Drive) {
        *self = Side::default();
        self.first.seed(0.0, |u| d.big(u));
        self.rect.seed(0.0, rectify_big);
    }

    /// Keeps the first curve's last input when its dead zone moves, so
    /// the next average is over the new curve.
    fn reseed(&mut self, d: &Drive) {
        let v = self.first.last();
        self.first.seed(v, |u| d.big(u));
    }

    /// One sample at 4x of `model`.
    #[inline]
    fn run(&mut self, model: usize, x: f32, d: &Drive, c: &Coefs) -> f32 {
        let tb32 = d.tb;
        let big = |u: f64| d.big(u);
        let curve = |u: f32| starved(u, d.dead, d.b);
        match model {
            0 => {
                // Face: coupling high-pass, the starved, biased curve,
                // then a one-pole low-pass for Tone.
                let v = self.hp1(0, x, c.hp30);
                let y = self.first.run(v * d.g1, curve, big) - tb32;
                self.tone_lp = flush(self.tone_lp + (y - self.tone_lp) * d.tone);
                self.tone_lp
            }
            1 => {
                // Muff: two stages, each a 100 Hz high-pass, gain, a
                // symmetric clip and a 1.2 kHz low-pass.
                let v = self.hp1(0, x, c.hp100);
                let y = self.first.run(v * d.g1, curve, big) - tb32;
                self.lp[0] = flush(self.lp[0] + (y - self.lp[0]) * c.lp_muff);
                let v = self.hp1(1, self.lp[0], c.hp100);
                let y = fast_tanh(v * d.g2);
                self.lp[1] = flush(self.lp[1] + (y - self.lp[1]) * c.lp_muff);
                let y = self.lp[1];
                // The tone: the low-pass and the high-pass blended, which
                // scoops the mids at noon.
                self.tone_lp = flush(self.tone_lp + (y - self.tone_lp) * c.tone_lo);
                self.tone_hp = flush(c.tone_hi * (self.tone_hp + y - self.tone_hp_x));
                self.tone_hp_x = y;
                2.0 * (self.tone_lp + (self.tone_hp - self.tone_lp) * d.tone)
            }
            _ => {
                // Octave: the rectifier (averaged), its offset taken out,
                // then Face's curve and Tone.
                let r = self.rect.run(x, rectify, rectify_big);
                let v = self.hp1(0, r, c.hp30);
                let y = self.first.run(v * d.g1, curve, big) - tb32;
                self.tone_lp = flush(self.tone_lp + (y - self.tone_lp) * d.tone);
                self.tone_lp
            }
        }
    }

    #[inline]
    fn hp1(&mut self, k: usize, x: f32, a: f32) -> f32 {
        self.hp[k] = flush(a * (self.hp[k] + x - self.hp_x[k]));
        self.hp_x[k] = x;
        self.hp[k]
    }
}

/// Model changes and Low keep turning on or off crossfade over 10 ms.
const FADE_MS: f32 = 10.0;

/// A fuzz pedal: Face, Muff or Octave, with Fuzz, Tone, Gate (starve),
/// Low keep (a clean sub under a crossover) and Level.
pub struct Fuzz {
    v: [f32; 6],
    rate: u32,
    c: Coefs,
    up: [[IirUp2; 2]; 2],
    down: [[IirDown2; 2]; 2],
    guard: Svf,
    /// Each side's state for each model.
    sides: [[Side; 3]; 2],
    model: usize,
    old: usize,
    fade: u32,
    fade_len: u32,
    /// Low keep's crossover: its low band is the dry lows kept.
    split: Crossover,
    /// The same crossover after the fuzz: its high band, so the fuzz adds
    /// nothing under the kept lows.
    post: Crossover,
    keep: Smooth,
    /// The crossover's frequency, gliding from one Low keep to another.
    keep_hz: Smooth,
    knob: Smooth,
    tone: Smooth,
    gate: Smooth,
    level: Smooth,
    /// The knob positions the drive was last worked out at.
    at: [f32; 3],
    drive: [Drive; 3],
}

const HB1: (usize, f64) = (5, 0.06);
const HB2: (usize, f64) = (4, 0.12);

impl Default for Fuzz {
    fn default() -> Self {
        Self::new()
    }
}

impl Fuzz {
    pub fn new() -> Fuzz {
        let v = defaults(&FUZZ_PARAMS);
        let up = || [IirUp2::new(HB1.0, HB1.1), IirUp2::new(HB2.0, HB2.1)];
        let down = || [IirDown2::new(HB2.0, HB2.1), IirDown2::new(HB1.0, HB1.1)];
        let mut f = Fuzz {
            v,
            rate: 48_000,
            c: Coefs::default(),
            up: [up(), up()],
            down: [down(), down()],
            guard: Svf::default(),
            sides: [[Side::default(); 3]; 2],
            model: 0,
            old: 0,
            fade: 0,
            fade_len: 1,
            split: Crossover::default(),
            post: Crossover::default(),
            keep: Smooth::new(0.0),
            keep_hz: Smooth::new(KEEP_HZ[1]),
            knob: Smooth::new(0.6),
            tone: Smooth::new(0.5),
            gate: Smooth::new(0.0),
            level: Smooth::new(1.0),
            at: [f32::NAN; 3],
            drive: [Drive::default(); 3],
        };
        f.setup(48_000);
        f
    }

    /// The knobs' targets (fuzz, tone, gate; 0 to 1).
    fn at_targets(&self) -> [f32; 3] {
        [self.knob.target, self.tone.target, self.gate.target]
    }

    fn model_index(&self) -> usize {
        (self.v[fz::MODEL as usize] as usize).min(MODEL_DATA.len() - 1)
    }

    fn keep_index(&self) -> usize {
        (self.v[fz::LOW_KEEP as usize] as usize).min(KEEP_HZ.len() - 1)
    }

    /// The drive for `model` at knob positions fuzz, tone and gate (0 to
    /// 1).
    fn drive_for(&self, model: usize, k: [f32; 3]) -> Drive {
        let m = &MODEL_DATA[model];
        let (lo, hi) = m.gain_db;
        let g1 = db_to_gain(lo + (hi - lo) * k[0]);
        // The Muff's second stage takes most of its range from the first
        // knob too, a little less.
        let g2 = db_to_gain(6.0 + 18.0 * k[0]);
        // Gate starves the first stage: a dead zone of up to 0.05 (-26
        // dBFS) of what comes in, in the curve's units after the gain.
        let dead = 0.05 * k[2] * k[2] * g1;
        // Face and Octave: Tone sweeps a one-pole low-pass from 1.5 to
        // 12 kHz; Muff: the blend.
        let tone = if model == 1 {
            k[1]
        } else {
            let hz = 1500.0 * 8f32.powf(k[1]);
            1.0 - (-std::f32::consts::TAU * hz / (4 * self.rate) as f32).exp()
        };
        let tb = fast_tanh(m.bias);
        Drive {
            g1,
            g2,
            dead,
            b: m.bias,
            tb,
            lcb: log_cosh(f64::from(m.bias).abs()),
            tone,
            trim: db_to_gain(trim_at(&m.trim_db, k[0])),
        }
    }

    fn apply(&mut self, id: u16, v: f32) {
        match id {
            fz::MODEL => {
                let m = self.model_index();
                if m == self.model {
                    return;
                }
                let d = self.drive_for(m, self.at_targets());
                self.drive[m] = d;
                for side in &mut self.sides {
                    side[m].reset(&d);
                }
                self.old = self.model;
                self.model = m;
                self.at = [f32::NAN; 3];
                self.fade = self.fade_len;
            }
            fz::FUZZ => self.knob.target = v / 100.0,
            fz::TONE => self.tone.target = v / 100.0,
            fz::GATE => self.gate.target = v / 100.0,
            fz::LOW_KEEP => {
                let k = self.keep_index();
                if k > 0 {
                    if self.keep.value == 0.0 {
                        // Coming on from off: the crossovers start clean,
                        // at the new frequency.
                        self.split.reset();
                        self.post.reset();
                        self.keep_hz.snap(KEEP_HZ[k]);
                        self.split.set(KEEP_HZ[k], self.rate);
                        self.post.set(KEEP_HZ[k], self.rate);
                    } else {
                        // From one frequency to another: it glides.
                        self.keep_hz.target = KEEP_HZ[k];
                    }
                }
                self.keep.target = if k > 0 { 1.0 } else { 0.0 };
            }
            fz::LEVEL => self.level.target = db_to_gain(v),
            _ => {}
        }
    }

    fn begin(&mut self, _ctx: &Ctx) {}
    fn run(&mut self, _ctx: &Ctx, _from: usize, l: &mut [f32], r: &mut [f32]) {
        let c = self.c;
        let inv_fade = 1.0 / self.fade_len as f32;
        for i in 0..l.len() {
            let k = [self.knob.glide(), self.tone.glide(), self.gate.glide()];
            if k != self.at {
                self.at = k;
                let mut live = [self.model, self.old];
                if self.fade == 0 {
                    live[1] = self.model;
                }
                for m in live {
                    let d = self.drive_for(m, k);
                    self.drive[m] = d;
                    for side in &mut self.sides {
                        side[m].reseed(&d);
                    }
                }
            }
            let keep = self.keep.glide();
            if self.keep_hz.value != self.keep_hz.target {
                let hz = self.keep_hz.glide();
                self.split.set(hz, self.rate);
                self.post.set(hz, self.rate);
            }
            let level = self.level.glide();
            let w = if self.fade > 0 {
                let w = 1.0 - self.fade as f32 * inv_fade;
                self.fade -= 1;
                Some(w)
            } else {
                None
            };
            let (model, old) = (self.model, self.old);
            let (dn, dold) = (self.drive[model], self.drive[old]);
            for (ch, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let lo = if keep > 0.0 {
                    self.split.split(ch, *x).0 * keep
                } else {
                    0.0
                };
                let mut quad = [0.0f32; 4];
                let [a, b] = self.up[ch][0].run(*x);
                for (h, v) in [a, b].into_iter().enumerate() {
                    let [c0, c1] = self.up[ch][1].run(v);
                    quad[2 * h] = c0;
                    quad[2 * h + 1] = c1;
                }
                let side = &mut self.sides[ch];
                for q in &mut quad {
                    let n = side[model].run(model, *q, &dn, &c) * dn.trim;
                    let y = match w {
                        Some(w) => {
                            let o = side[old].run(old, *q, &dold, &c) * dold.trim;
                            o + (n - o) * w
                        }
                        None => n,
                    };
                    *q = self.guard.run(ch, y);
                }
                let h0 = self.down[ch][0].run([quad[0], quad[1]]);
                let h1 = self.down[ch][0].run([quad[2], quad[3]]);
                let mut y = self.down[ch][1].run([h0, h1]);
                if keep > 0.0 {
                    let (_, hp) = self.post.split(ch, y);
                    y += (hp - y) * keep;
                }
                *x = (lo + y) * level;
            }
        }
    }

    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        let r4 = (4 * rate) as f32;
        let hp = |hz: f32| (-std::f32::consts::TAU * hz / r4).exp();
        let lp = |hz: f32| 1.0 - (-std::f32::consts::TAU * hz / r4).exp();
        self.c = Coefs {
            hp30: hp(30.0),
            hp100: hp(100.0),
            lp_muff: lp(1200.0),
            tone_lo: lp(400.0),
            tone_hi: hp(1600.0),
        };
        self.guard
            .set(SvfMode::LowPass, 12_000.0, 0.707, 0.0, 4 * rate);
        self.fade_len = ((FADE_MS * 0.001 * rate as f32) as u32).max(1);
        for s in [
            &mut self.keep,
            &mut self.keep_hz,
            &mut self.knob,
            &mut self.tone,
            &mut self.gate,
            &mut self.level,
        ] {
            s.set_time(rate, 20.0);
        }
        self.keep.set_time(rate, FADE_MS * 0.5);
        self.model = self.model_index();
        for id in [fz::FUZZ, fz::TONE, fz::GATE, fz::LOW_KEEP, fz::LEVEL] {
            self.apply(id, self.v[id as usize]);
        }
        self.clear();
    }

    fn clear(&mut self) {
        for ch in 0..2 {
            for u in &mut self.up[ch] {
                u.reset();
            }
            for d in &mut self.down[ch] {
                d.reset();
            }
        }
        for s in [
            &mut self.keep,
            &mut self.keep_hz,
            &mut self.knob,
            &mut self.tone,
            &mut self.gate,
            &mut self.level,
        ] {
            s.snap(s.target);
        }
        let k = self.at_targets();
        for m in 0..MODEL_DATA.len() {
            let d = self.drive_for(m, k);
            self.drive[m] = d;
            for side in &mut self.sides {
                side[m].reset(&d);
            }
        }
        self.at = k;
        self.guard.reset();
        self.split.set(self.keep_hz.value, self.rate);
        self.post.set(self.keep_hz.value, self.rate);
        self.split.reset();
        self.post.reset();
        self.old = self.model;
        self.fade = 0;
    }

    fn tail(&self) -> u32 {
        self.rate / 50
    }
    fn rest(&self) -> u32 {
        // Its filters and half-bands settle within a few ms.
        self.rate / 20
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(Fuzz, FUZZ_INFO, FUZZ_PARAMS, rest);
