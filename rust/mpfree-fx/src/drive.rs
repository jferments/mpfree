//! Drive: deliberate distortion done cleanly. The shaper runs at 2x (Eco)
//! or 4x (HQ) the sample rate so its new harmonics are filtered before they
//! can fold back as aliasing. Eco resamples with polyphase IIR half-bands
//! (a few multiplies a frame); HQ with linear-phase FIR half-bands, so its
//! wet signal keeps the dry signal's phase at any Mix. A DC blocker removes the offset an
//! asymmetric curve makes; auto-gain keeps the level where it was, so more
//! drive means more grit, not just louder.

use std::f32::consts::FRAC_PI_2;

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::adaa::{log_cosh, Adaa, Flat2x, Flat4x};
use crate::oversample::{Down2, IirPair, Up2, TAPS_2X, TAPS_4X};
use crate::svf::{Segs, Svf, SvfMode};
use crate::{db_to_gain, defaults, fast_tanh, gain_to_db, FixedDelay};

/// [`Drive`] parameter ids.
pub mod ids {
    pub const MODE: u16 = 0;
    pub const DRIVE: u16 = 1;
    pub const TONE: u16 = 2;
    pub const BIAS: u16 = 3;
    pub const MIX: u16 = 4;
    pub const OUTPUT: u16 = 5;
    pub const AUTO_GAIN: u16 = 6;
    pub const QUALITY: u16 = 7;
    /// Meter: output level against input level, in dB.
    pub const LEVEL_CHANGE: u16 = 8;
}

const MODES: &[&str] = &["Soft", "Hard", "Tube", "Foldback", "Fuzz", "Rectify"];
const QUALITY: &[&str] = &["Eco 2x", "HQ 4x"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.drive",
    name: "Drive",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 9] = [
    ParamInfo {
        id: 0,
        name: "Mode",
        group: "Drive",
        min: 0.0,
        max: 5.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(MODES),
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Drive",
        group: "Drive",
        min: 0.0,
        max: 36.0,
        default: 12.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Tone",
        group: "Drive",
        min: 500.0,
        max: 20_000.0,
        default: 20_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Bias",
        group: "Drive",
        min: -100.0,
        max: 100.0,
        default: 20.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Mix",
        group: "Output",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Output",
        group: "Output",
        min: -24.0,
        max: 12.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Auto-gain",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Quality",
        group: "Output",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(QUALITY),
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Level change",
        group: "Meters",
        min: -48.0,
        max: 48.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: true,
    },
];

/// The delay in base frames. The inner 2x-to-4x pair delays by
/// `(TAPS_4X - 1) / 2` samples at the 2x rate, half a base frame short of a
/// whole one; ADAA and its [`Flat4x`] at 4x add the two 4x samples (one 2x
/// sample) that make it whole. Eco is padded to the same total, so
/// switching quality never moves the sound in time.
const LATENCY: usize = (TAPS_2X - 1) / 2 + (TAPS_4X - 1).div_ceil(4);
const _: () = assert!(
    (TAPS_4X - 1) / 2 + Flat4x::STEPS / 2 == 2 * ((TAPS_4X - 1).div_ceil(4)),
    "ADAA at 4x makes the inner pair a whole number of base frames"
);
/// Eco's half-bands: [`ECO_SECTIONS`] first-order all-pass sections, flat
/// to 0.19 of the 2x rate (18 kHz at 48 kHz) and at least 71 dB down from
/// 0.31 (30 kHz). A fifth of the cost of the 95-tap FIR pair on the MPC's
/// ARM.
const ECO_SECTIONS: usize = 5;
/// How often Eco's half-bands flush their states, frames.
const SETTLE: u32 = 32;
const ECO_TBW: f64 = 0.06;
/// The Eco pair's delay at low frequencies in base frames (2.02; 2.2 at
/// 10 kHz, 2.8 at 18 kHz), padded up to [`LATENCY`] so switching quality
/// never moves the sound in time.
const ECO_DELAY: usize = 2;
/// ADAA and its [`Flat2x`] at 2x, in base frames.
const ECO_ADAA: usize = Flat2x::STEPS / 2;
const ECO_PAD: usize = LATENCY - ECO_DELAY;
/// The level auto-gain holds: noise with a normal (Gaussian) spread of
/// levels at -18 dBFS RMS, as pink noise and a dense mix have. A sine at
/// a fixed level (what it held before) peaks far less often than music
/// does, so more drive made music louder.
const REF_RMS: f32 = 0.125_892_54;
/// Points of the Gaussian average auto-gain takes over the curve.
const GAUSS_N: usize = 128;
/// How far out (in standard deviations) the average looks.
const GAUSS_Z: f32 = 8.0;
/// A change of Mode or Quality crossfades over this long, ms.
const FADE_MS: f32 = 10.0;
/// A quality coming in runs this many frames unheard before its fade, so
/// its filters are full of the music when it is heard.
const WARM: u32 = 128;
/// The low-end filter on the output: fourth-order Butterworth (two
/// stages), here. The asymmetric curves turn every drum hit into a pulse of
/// DC, energy under 15 Hz that eats headroom and pumps a limiter; this
/// takes it to under -20 dB of the hit (G9) and is within 0.1 dB from
/// 25 Hz up. It is on the output, dry and wet together, so any Mix adds
/// the two in phase at the bottom too; Mix 0 is the input with nothing
/// under 17 Hz.
const LOW_HZ: f32 = 17.0;
const LOW_Q: [f32; 2] = [0.541_196_1, 1.306_563];

/// The curves without their bias: tanh (Soft and Tube), a clamp (Hard), a
/// sine fold (Foldback), Fuzz's exponential, and tanh of the magnitude
/// (Rectify).
#[inline]
fn base(mode: usize, v: f32) -> f32 {
    match mode {
        1 => v.clamp(-1.0, 1.0),
        3 => (FRAC_PI_2 * v).sin(),
        4 => v.signum() * (1.0 - (-2.0 * v.abs()).exp()),
        5 => fast_tanh(v.abs()),
        _ => fast_tanh(v),
    }
}

/// The antiderivative of [`base`], zero at 0, for ADAA.
#[inline]
fn base_big(mode: usize, v: f64) -> f64 {
    use std::f64::consts::FRAC_PI_4;
    let a = v.abs();
    match mode {
        1 => {
            if a <= 1.0 {
                0.5 * v * v
            } else {
                a - 0.5
            }
        }
        3 => {
            // (2 / pi) (1 - cos(pi v / 2)), without the cancellation.
            let s = (FRAC_PI_4 * v).sin();
            s * s / FRAC_PI_4
        }
        4 => a + 0.5 * (crate::adaa::exp(-2.0 * a) - 1.0),
        5 => v.signum() * log_cosh(a),
        _ => log_cosh(a),
    }
}

/// The bias mode `mode` adds before its curve: none in Soft (decision
/// 10.1: Soft with a bias would be Tube), Bias elsewhere.
#[inline]
fn bias_of(mode: usize, b: f32) -> f32 {
    if mode == 0 {
        0.0
    } else {
        b
    }
}

/// One mode's curve: `base(x + bias) - base(bias)`, so silence stays
/// silent, with the bias's DC taken off.
#[inline]
fn shape(mode: usize, x: f32, b: f32) -> f32 {
    let b = bias_of(mode, b);
    base(mode, x + b) - base(mode, b)
}

/// The Gaussian average's points and weights (the weights sum to 1).
fn gauss_table() -> [[f32; 2]; GAUSS_N] {
    let mut t = [[0.0f32; 2]; GAUSS_N];
    let h = 2.0 * GAUSS_Z / GAUSS_N as f32;
    let mut sum = 0.0f32;
    for (i, p) in t.iter_mut().enumerate() {
        let z = -GAUSS_Z + h * (i as f32 + 0.5);
        let w = (-0.5 * z * z).exp();
        *p = [z, w];
        sum += w;
    }
    for p in &mut t {
        p[1] /= sum;
    }
    t
}

/// Gain that brings Gaussian noise at [`REF_RMS`] back to its own RMS
/// after the curve (DC removed, as the low-end filter does): the curve's
/// output averaged over the noise's spread of levels. 128 curve points, so
/// it is worked out when Drive, Bias or Mode change, never per sample.
fn auto_gain(gauss: &[[f32; 2]; GAUSS_N], mode: usize, drive: f32, b: f32) -> f32 {
    let rms = if mode == 3 {
        // Foldback folds many times over the noise's spread at high drive,
        // more often than the points can follow; its average has a closed
        // form. For x normal with spread s, E sin(a (x + b)) is
        // sin(a b) exp(-a^2 s^2 / 2) and E sin^2 is
        // (1 - cos(2 a b) exp(-2 a^2 s^2)) / 2.
        let a = f64::from(FRAC_PI_2);
        let s = f64::from(REF_RMS * drive);
        let b = f64::from(b);
        let e2 = (-0.5 * a * a * s * s).exp();
        let mean = (a * b).sin() * e2;
        let sq = 0.5 * (1.0 - (2.0 * a * b).cos() * e2.powi(4));
        (sq - mean * mean).max(0.0).sqrt() as f32
    } else {
        let mut sum = 0.0f32;
        let mut sq = 0.0f32;
        for &[z, w] in gauss {
            let y = shape(mode, REF_RMS * drive * z, b);
            sum += w * y;
            sq += w * y * y;
        }
        (sq - sum * sum).max(0.0).sqrt()
    };
    if rms < 1e-7 {
        1.0
    } else {
        (REF_RMS / rms).min(64.0)
    }
}

/// `db_to_gain(db)`, recomputed only when `db` changed.
#[inline]
fn cached(c: &mut (f32, f32), db: f32) -> f32 {
    if c.0 != db {
        *c = (db, db_to_gain(db));
    }
    c.1
}

/// The curves one frame plays: the mode heard, and while a Mode change
/// fades, the mode going out with its weight.
#[derive(Clone, Copy)]
struct Shaping {
    mode: usize,
    b: f32,
    off: f32,
    from: usize,
    from_b: f32,
    from_off: f32,
    /// The weight of `from`, 0 once the fade is over.
    t: f32,
}

/// One oversampled stream's ADAA for the mode heard and the one fading
/// out.
#[derive(Clone, Copy, Default)]
struct Curves {
    cur: Adaa,
    from: Adaa,
}

impl Curves {
    #[inline]
    fn run(&mut self, s: &Shaping, u: f32) -> f32 {
        let (m, f) = (s.mode, s.from);
        let y = self.cur.run(u + s.b, |v| base(m, v), |v| base_big(m, v)) - s.off;
        if s.t > 0.0 {
            let o = self
                .from
                .run(u + s.from_b, |v| base(f, v), |v| base_big(f, v))
                - s.from_off;
            crate::lerp(y, o, s.t)
        } else {
            y
        }
    }

    /// A Mode change: the curve heard goes out, `to` comes in from the
    /// same last input (its bias `b_to` in place of `b_from`'s).
    fn switch(&mut self, to: usize, b_from: f32, b_to: f32) {
        self.from = self.cur;
        let u = self.cur.last() - b_from;
        self.cur.seed(u + b_to, |v| base_big(to, v));
    }

    fn reset(&mut self) {
        self.cur.reset();
        self.from.reset();
    }
}

/// Eco for one channel: polyphase IIR half-bands around ADAA at 2x.
struct Eco {
    /// The half-bands, and the dry side's phase match: the pair's
    /// all-pass phase, then the same pad as the wet path's ADAA and pad,
    /// so any Mix adds wet and dry in phase.
    iir: IirPair<ECO_SECTIONS>,
    curves: Curves,
    flat: Flat2x,
    pad: FixedDelay,
    dry_pad: FixedDelay,
}

impl Eco {
    fn new() -> Eco {
        Eco {
            iir: IirPair::new(ECO_TBW),
            curves: Curves::default(),
            flat: Flat2x::default(),
            pad: FixedDelay::new(ECO_PAD - ECO_ADAA),
            dry_pad: FixedDelay::new(ECO_PAD),
        }
    }

    /// The wet and dry of one frame: `pre` is the input with the drive's
    /// gain, `x` the input.
    #[inline]
    fn run(&mut self, s: &Shaping, pre: f32, x: f32) -> (f32, f32) {
        let [a, b] = self.iir.up(pre);
        let ya = self.flat.run(self.curves.run(s, a));
        let yb = self.flat.run(self.curves.run(s, b));
        let wet = self.pad.run(self.iir.down([ya, yb]));
        (wet, self.dry_pad.run(self.iir.phase(x)))
    }

    fn reset(&mut self) {
        self.iir.reset();
        self.curves.reset();
        self.flat.reset();
        self.pad.reset();
        self.dry_pad.reset();
    }
}

/// HQ for one channel: linear-phase FIR half-bands to 4x around ADAA, so
/// its wet signal keeps the dry signal's phase at any Mix.
struct Hq {
    up1: Up2,
    up2: Up2,
    down2: Down2,
    down1: Down2,
    curves: Curves,
    flat: Flat4x,
    dry: FixedDelay,
}

impl Hq {
    fn new() -> Hq {
        Hq {
            up1: Up2::new(TAPS_2X),
            up2: Up2::new(TAPS_4X),
            down2: Down2::new(TAPS_4X),
            down1: Down2::new(TAPS_2X),
            curves: Curves::default(),
            flat: Flat4x::default(),
            dry: FixedDelay::new(LATENCY),
        }
    }

    #[inline]
    fn run(&mut self, s: &Shaping, pre: f32, x: f32) -> (f32, f32) {
        let [a, b] = self.up1.run(pre);
        let [a0, a1] = self.up2.run(a);
        let [b0, b1] = self.up2.run(b);
        let mut c = |u: f32| self.flat.run(self.curves.run(s, u));
        let (a0, a1, b0, b1) = (c(a0), c(a1), c(b0), c(b1));
        let sa = self.down2.run([a0, a1]);
        let sb = self.down2.run([b0, b1]);
        (self.down1.run([sa, sb]), self.dry.run(x))
    }

    fn reset(&mut self) {
        self.up1.reset();
        self.up2.reset();
        self.down2.reset();
        self.down1.reset();
        self.curves.reset();
        self.flat.reset();
        self.dry.reset();
    }
}

struct Channel {
    eco: Eco,
    hq: Hq,
}

/// Oversampled drive with six curves, tone, bias, mix and auto-gain.
pub struct Drive {
    v: [f32; 9],
    rate: u32,
    drive_db: Smooth,
    bias: Smooth,
    log_tone: Smooth,
    mix: Smooth,
    out_db: Smooth,
    comp: Smooth,
    ch: [Channel; 2],
    tone: Svf,
    /// The low-end filter on the output, two stages.
    low: [Svf; 2],
    ms_in: f32,
    ms_out: f32,
    ms_coef: f32,
    /// The last drive and output levels in dB and their gains, so the
    /// gains are only recomputed while a level moves.
    gains: [(f32, f32); 2],
    /// The curve's value at its bias, and the mode and bias it is for.
    off_at: (usize, f32, f32),
    /// Frames since Eco's half-bands were last settled.
    settle: u32,
    gauss: [[f32; 2]; GAUSS_N],
    /// The mode heard, the one fading out, and the fade's frames left.
    mode: usize,
    from: usize,
    fade: u32,
    fade_len: u32,
    /// HQ's share of what is heard (0 Eco, 1 HQ), which paths run, and for
    /// how many frames each has run.
    hq_w: f32,
    running: [bool; 2],
    warm: [u32; 2],
}

impl Default for Drive {
    fn default() -> Self {
        Self::new()
    }
}

impl Drive {
    /// The shaper's curve at the mode, drive and bias set: what a sample
    /// at `x` becomes before the tone, mix, auto-gain and output, with the
    /// bias's DC taken off as the low-end filter does. For drawing.
    pub fn curve(&self, x: f32) -> f32 {
        let mode = self.v[ids::MODE as usize] as usize;
        let g = db_to_gain(self.v[ids::DRIVE as usize]);
        shape(mode, x * g, self.v[ids::BIAS as usize] / 100.0)
    }

    /// Allocates the oversamplers (control thread).
    pub fn new() -> Drive {
        let v = defaults(&PARAMS);
        let hq = v[ids::QUALITY as usize] >= 0.5;
        let mut d = Drive {
            v,
            rate: 48_000,
            drive_db: Smooth::new(v[1]),
            bias: Smooth::new(v[3] / 100.0),
            log_tone: Smooth::new(v[2].log2()),
            mix: Smooth::new(v[4] / 100.0),
            out_db: Smooth::new(v[5]),
            comp: Smooth::new(1.0),
            ch: [
                Channel {
                    eco: Eco::new(),
                    hq: Hq::new(),
                },
                Channel {
                    eco: Eco::new(),
                    hq: Hq::new(),
                },
            ],
            tone: Svf::default(),
            low: [Svf::default(); 2],
            ms_in: 0.0,
            ms_out: 0.0,
            ms_coef: 0.0,
            gains: [(0.0, 1.0); 2],
            off_at: (usize::MAX, 0.0, 0.0),
            settle: 0,
            gauss: gauss_table(),
            mode: v[ids::MODE as usize] as usize,
            from: v[ids::MODE as usize] as usize,
            fade: 0,
            fade_len: 480,
            hq_w: if hq { 1.0 } else { 0.0 },
            running: [!hq, hq],
            warm: [WARM; 2],
        };
        d.comp.snap(d.comp_target());
        d.prepare(48_000, mpfree_node::MAX_BLOCK);
        d
    }

    fn comp_target(&self) -> f32 {
        if self.v[6] < 0.5 {
            1.0
        } else {
            auto_gain(
                &self.gauss,
                self.v[0] as usize,
                db_to_gain(self.v[1]),
                self.v[3] / 100.0,
            )
        }
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        if p.readonly {
            return;
        }
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::MODE => {
                let m = v as usize;
                if m != self.mode {
                    // Fade from what is heard now; a change mid-fade fades
                    // from the mode that was coming in.
                    let b = self.bias.value;
                    let (bf, bt) = (bias_of(self.mode, b), bias_of(m, b));
                    for c in &mut self.ch {
                        c.eco.curves.switch(m, bf, bt);
                        c.hq.curves.switch(m, bf, bt);
                    }
                    self.from = self.mode;
                    self.mode = m;
                    self.fade = self.fade_len;
                }
            }
            ids::DRIVE => self.drive_db.target = v,
            ids::TONE => self.log_tone.target = v.log2(),
            ids::BIAS => self.bias.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            ids::OUTPUT => self.out_db.target = v,
            _ => {}
        }
        if matches!(id, ids::MODE | ids::DRIVE | ids::BIAS | ids::AUTO_GAIN) {
            self.comp.target = self.comp_target();
        }
    }

    fn set_tone(&mut self) {
        self.tone.set(
            SvfMode::LowPass,
            self.log_tone.value.exp2(),
            0.707,
            0.0,
            self.rate,
        );
    }

    /// One frame of the Quality crossfade: the quality asked for starts
    /// (from silence), runs [`WARM`] frames unheard, then fades in over
    /// [`FADE_MS`]; the one going out stops once it is silent.
    #[inline]
    fn step_quality(&mut self) {
        let want = usize::from(self.v[ids::QUALITY as usize] >= 0.5);
        let target = want as f32;
        if self.hq_w != target {
            if !self.running[want] {
                self.running[want] = true;
                self.warm[want] = 0;
            }
            if self.warm[want] >= WARM {
                let step = 1.0 / self.fade_len as f32;
                self.hq_w = if target > self.hq_w {
                    (self.hq_w + step).min(1.0)
                } else {
                    (self.hq_w - step).max(0.0)
                };
            }
        } else if self.running[1 - want] {
            self.running[1 - want] = false;
            let m = self.mode;
            let rest = bias_of(m, self.bias.value);
            for c in &mut self.ch {
                let curves = if want == 1 {
                    c.eco.reset();
                    &mut c.eco.curves
                } else {
                    c.hq.reset();
                    &mut c.hq.curves
                };
                curves.cur.seed(rest, |v| base_big(m, v));
            }
        }
        for (w, r) in self.warm.iter_mut().zip(self.running) {
            if r {
                *w = w.saturating_add(1);
            }
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            // Eco's half-bands drop what is too small to hear every SETTLE
            // frames from the stream's start (the same in any block size).
            if self.settle == 0 {
                for c in &mut self.ch {
                    c.eco.iir.settle();
                }
            }
            self.settle = (self.settle + 1) % SETTLE;
            self.step_quality();
            let g = cached(&mut self.gains[0], self.drive_db.next());
            let bias = self.bias.next();
            if !self.log_tone.settled() {
                self.log_tone.next();
                self.set_tone();
            }
            let mix = self.mix.next();
            let out = cached(&mut self.gains[1], self.out_db.next());
            let comp = self.comp.next();
            let (b, from_b) = (bias_of(self.mode, bias), bias_of(self.from, bias));
            let t = self.fade as f32 / self.fade_len as f32;
            let s = Shaping {
                mode: self.mode,
                b,
                off: if (self.off_at.0, self.off_at.1) == (self.mode, b) {
                    self.off_at.2
                } else {
                    let off = base(self.mode, b);
                    self.off_at = (self.mode, b, off);
                    off
                },
                from: self.from,
                from_b,
                from_off: if t > 0.0 {
                    base(self.from, from_b)
                } else {
                    0.0
                },
                t,
            };
            self.fade = self.fade.saturating_sub(1);
            let w = self.hq_w;
            let [run_eco, run_hq] = self.running;
            let x = [l[i], r[i]];
            let mut y = [0.0f32; 2];
            for c in 0..2 {
                let chn = &mut self.ch[c];
                let pre = x[c] * g;
                let (eco_wet, eco_dry) = if run_eco {
                    chn.eco.run(&s, pre, x[c])
                } else {
                    (0.0, 0.0)
                };
                let (hq_wet, hq_dry) = if run_hq {
                    chn.hq.run(&s, pre, x[c])
                } else {
                    (0.0, 0.0)
                };
                let wet = crate::lerp(eco_wet, hq_wet, w);
                let dry = crate::lerp(eco_dry, hq_dry, w);
                let wet = self.tone.run(c, wet * comp);
                let o = crate::lerp(dry, wet, mix);
                let o = self.low[0].run(c, o);
                y[c] = self.low[1].run(c, o) * out;
            }
            let k = self.ms_coef;
            self.ms_in += k * (0.5 * (x[0] * x[0] + x[1] * x[1]) - self.ms_in);
            self.ms_out += k * (0.5 * (y[0] * y[0] + y[1] * y[1]) - self.ms_out);
            self.ms_in = crate::flush(self.ms_in);
            self.ms_out = crate::flush(self.ms_out);
            l[i] = y[0];
            r[i] = y[1];
        }
        self.v[ids::LEVEL_CHANGE as usize] = if self.ms_in > 1e-10 {
            (gain_to_db((self.ms_out / self.ms_in).sqrt())).clamp(-48.0, 48.0)
        } else {
            0.0
        };
    }
}

impl Node for Drive {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        for s in [
            &mut self.drive_db,
            &mut self.bias,
            &mut self.log_tone,
            &mut self.mix,
            &mut self.out_db,
            &mut self.comp,
        ] {
            s.set_time(self.rate, 20.0);
        }
        for (f, q) in self.low.iter_mut().zip(LOW_Q) {
            f.set(SvfMode::HighPass, LOW_HZ, q, 0.0, self.rate);
        }
        self.fade_len = ((FADE_MS * 0.001 * self.rate as f32) as u32).max(1);
        self.ms_coef = 1.0 - (-1.0 / (0.3 * self.rate as f32)).exp();
        self.set_tone();
    }
    fn reset(&mut self) {
        self.settle = 0;
        for s in [
            &mut self.drive_db,
            &mut self.bias,
            &mut self.log_tone,
            &mut self.mix,
            &mut self.out_db,
            &mut self.comp,
        ] {
            s.snap(s.target);
        }
        // At rest the curves' input is the bias alone, as it is after any
        // silence, so a node woken from a reset plays what one that ran
        // through the silence would.
        let m = self.mode;
        let rest = bias_of(m, self.bias.value);
        for c in &mut self.ch {
            c.eco.reset();
            c.hq.reset();
            c.eco.curves.cur.seed(rest, |v| base_big(m, v));
            c.hq.curves.cur.seed(rest, |v| base_big(m, v));
        }
        let hq = self.v[ids::QUALITY as usize] >= 0.5;
        self.hq_w = if hq { 1.0 } else { 0.0 };
        self.running = [!hq, hq];
        self.warm = [WARM; 2];
        self.from = self.mode;
        self.fade = 0;
        self.tone.reset();
        self.set_tone();
        for f in &mut self.low {
            f.reset();
        }
        self.ms_in = 0.0;
        self.ms_out = 0.0;
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| {
            if let NodeEvent::Param { id, value } = *ev {
                self.apply(id, value)
            }
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn latency(&self) -> u32 {
        LATENCY as u32
    }
    fn rest_frames(&self) -> u32 {
        // What it holds is its tail: once rung out it is as good as reset.
        self.tail_frames()
    }
    fn tail_frames(&self) -> u32 {
        // The low-end filter rings longest: 100 dB down in 0.3 s.
        LATENCY as u32 + self.rate * 3 / 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_gain_holds_gaussian_noise_at_its_level() {
        let gauss = gauss_table();
        for mode in 0..6 {
            for drive_db in [0.0f32, 12.0, 36.0] {
                let g = auto_gain(&gauss, mode, db_to_gain(drive_db), 0.2);
                assert!(
                    g.is_finite() && g > 0.0 && g <= 64.0,
                    "mode {mode} drive {drive_db}: {g}"
                );
            }
        }
        // Soft at 0 dB drive and -18 dBFS RMS is nearly a straight line.
        assert!((auto_gain(&gauss, 0, 1.0, 0.0) - 1.0).abs() < 0.02);
        // Hard at 0 dB clips only the rare peaks over 18 dB above the RMS.
        assert!((auto_gain(&gauss, 1, 1.0, 0.0) - 1.0).abs() < 1e-3);
    }

    #[test]
    fn every_antiderivative_matches_its_curve() {
        // F(v + h) - F(v - h) over 2h is the curve at v, to the curve's
        // bend times h squared.
        let h = 1.0e-3f64;
        for mode in 0..6 {
            for i in -400..=400 {
                let v = i as f64 / 40.0 + 0.0123;
                let d = (base_big(mode, v + h) - base_big(mode, v - h)) / (2.0 * h);
                let f = f64::from(base(mode, v as f32));
                assert!((d - f).abs() < 2e-4, "mode {mode} at {v}: {d} against {f}");
            }
            assert_eq!(base_big(mode, 0.0), 0.0, "mode {mode}");
        }
    }
}
