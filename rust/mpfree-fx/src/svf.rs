//! Zero-delay-feedback (trapezoidal) state-variable filters, after Andrew
//! Simper's "Linear Trapezoidal Integrated SVF" (Cytomic, 2013): stable at
//! any cutoff below Nyquist and under fast modulation, so a sweep never
//! blows up or zips. Coefficients are recomputed every frame while a cutoff
//! or resonance is moving and cached once it settles.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::adaa::{Adaa, Flat2x};
use crate::character::soft_big;
use crate::oversample::IirPair;
use crate::{db_to_gain, defaults, fast_tanh, flush, lerp, soft, FixedDelay};

use mpfree_filter::multi::{bend_for, SlopeStages, BEND_FULL_RES};
use mpfree_filter::svf::{hard, res_to_q};
pub use mpfree_filter::svf::{Svf, SvfCoefs, SvfMode};

const TYPES: &[&str] = &[
    "Low-pass",
    "High-pass",
    "Band-pass",
    "Notch",
    "Bell",
    "Low shelf",
    "High shelf",
];
const SLOPES: &[&str] = &["6 dB", "12 dB", "18 dB", "24 dB", "36 dB", "48 dB"];
/// Each Slope's poles (6 dB an octave each, [`mpfree_filter::multi::slope`]).
const SLOPE_POLES: [u8; 6] = [1, 2, 3, 4, 6, 8];
const MODELS: &[&str] = &["Multi", "Ladder"];
const RES_MODES: &[&str] = &["Soft", "Hard", "Clean"];

/// [`MultiFilter`] parameter ids.
pub mod multi {
    pub const TYPE: u16 = 0;
    pub const SLOPE: u16 = 1;
    pub const CUTOFF: u16 = 2;
    pub const RESONANCE: u16 = 3;
    pub const GAIN: u16 = 4;
    pub const DRIVE: u16 = 5;
    /// Multi (the state-variable filter) or Ladder (a Moog-style 24 dB
    /// low-pass).
    pub const MODEL: u16 = 6;
    /// What holds the resonance back on a loud sound: Soft (it bends),
    /// Hard (it clips, at twice the rate) or Clean (nothing: the linear
    /// filter).
    pub const RES_MODE: u16 = 7;

    /// The Slope's choices, in order (6 to 48 dB an octave). A band-pass
    /// plays 12, 24, 36 or 48 (6 dB goes to 12, 18 to 12), a notch 12 or
    /// 24 (36 and 48 to 24); the bell and shelves have one slope.
    pub const SLOPE_6: f32 = 0.0;
    pub const SLOPE_12: f32 = 1.0;
    pub const SLOPE_18: f32 = 2.0;
    pub const SLOPE_24: f32 = 3.0;
    pub const SLOPE_36: f32 = 4.0;
    pub const SLOPE_48: f32 = 5.0;
}

static MULTI_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.filter",
    name: "Multi filter",
    kind: Kind::Effect,
};
static MULTI_PARAMS: [ParamInfo; 8] = [
    ParamInfo {
        id: 0,
        name: "Type",
        group: "Filter",
        min: 0.0,
        max: 6.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Slope",
        group: "Filter",
        min: 0.0,
        max: 5.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(SLOPES),
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Cutoff",
        group: "Filter",
        min: 20.0,
        max: 20_000.0,
        default: 20_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Resonance",
        group: "Filter",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Gain",
        group: "Filter",
        min: -24.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Drive",
        group: "Filter",
        min: 0.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Model",
        group: "Filter",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(MODELS),
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Res mode",
        group: "Filter",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(RES_MODES),
        readonly: false,
    },
];

/// The product of two complex gains.
pub(crate) fn cmul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

/// A complex gain's level in dB (-240 dB for silence).
pub(crate) fn power_db(h: (f64, f64)) -> f32 {
    (10.0 * (h.0 * h.0 + h.1 * h.1).max(1e-24).log10()) as f32
}

/// A type or slope change crossfades from the old filter over this long.
const FADE_MS: f32 = 10.0;
/// The drive's 2x path: Drive's Eco half-bands (flat to 0.19 of the
/// doubled rate, 71 dB down from 0.31), and ADAA's compensation after the
/// curve, which together delay the sound by this many frames.
const OS_SECTIONS: usize = 5;
const OS_TBW: f64 = 0.06;
const ADAA_FRAMES: usize = Flat2x::STEPS / 2;
/// The Multi filter's latency, frames: the drive's path, so the drive can
/// come and go without moving the sound in time.
pub(crate) const MULTI_LATENCY: u32 = (2 + ADAA_FRAMES) as u32;
/// How long the dry side takes to move between the plain delay and the
/// half-bands' phase match as the drive comes in or leaves, frames.
const PATH_FADE: u32 = 256;
/// The frame grid the half-bands' states are flushed on.
const GRID: u32 = 64;

/// The drive for one channel, at twice the rate: the soft curve averaged
/// by first-order ADAA between polyphase IIR half-bands; and the dry
/// side's match (the half-bands' phase, then ADAA's delay), so the two
/// blend in phase.
#[derive(Clone, Debug)]
struct DriveOs {
    iir: IirPair<OS_SECTIONS>,
    adaa: Adaa,
    flat: Flat2x,
    dry_pad: FixedDelay,
}

impl DriveOs {
    fn new() -> DriveOs {
        DriveOs {
            iir: IirPair::new(OS_TBW),
            adaa: Adaa::default(),
            flat: Flat2x::default(),
            dry_pad: FixedDelay::new(ADAA_FRAMES),
        }
    }
    /// The driven and the dry (phase-matched) frame for input `x`, gain `g`.
    #[inline]
    fn run(&mut self, x: f32, g: f32) -> (f32, f32) {
        let [a, b] = self.iir.up(x);
        let ya = self.flat.run(self.adaa.run(g * a, soft, soft_big));
        let yb = self.flat.run(self.adaa.run(g * b, soft, soft_big));
        let wet = self.iir.down([ya, yb]);
        (wet, self.dry_pad.run(self.iir.phase(x)))
    }
    fn reset(&mut self) {
        self.iir.reset();
        self.adaa.reset();
        self.flat.reset();
        self.dry_pad.reset();
    }
}

/// The ladder's feedback at Resonance 100 %: past 4 it oscillates, so the
/// top few percent of the knob sing on their own (Soft and Hard).
const LADDER_K_MAX: f32 = 4.3;
/// The same with no ceiling (Clean): with nothing to hold it, a ladder past
/// 4 would grow without end, so Clean stops short of it and rings long
/// instead (about a quarter of a second at 100 Hz), never singing.
const LADDER_K_CLEAN: f32 = 3.9;
/// The ladder's output makeup per unit of feedback: a linear ladder loses
/// `1 + k` at low frequencies (14 dB at the edge of oscillation); this
/// gives back about two thirds of it, so the familiar bass loss is there
/// but gentle (4.5 dB at full Resonance).
const LADDER_MAKEUP: f32 = 0.5;
/// Hard's ceiling on the Multi model's band-pass state at Resonance 25 %
/// and up (it rises below, as Soft's knee does); its edge is rounded by
/// [`mpfree_filter::svf::HARD_ROUND`].
const HARD_LIM: f32 = 0.6;
/// Hard's ceiling on the ladder's input: where `tanh`'s knee is, so a Hard
/// ladder sings about as loud as a Soft one and passes a mix-level sound
/// about as cleanly, then clips flat where `tanh` would still be rounding.
const LADDER_HARD_LIM: f32 = 0.5;

/// What holds the resonance back on a loud sound (the Res mode).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResMode {
    /// It bends, as an analog filter's loop saturates (the default).
    Soft,
    /// It clips, at twice the rate: a harder, buzzier scream.
    Hard,
    /// Nothing: the linear filter.
    Clean,
}

impl ResMode {
    fn of(v: f32) -> ResMode {
        match v as usize {
            1 => ResMode::Hard,
            2 => ResMode::Clean,
            _ => ResMode::Soft,
        }
    }

    /// The ladder's curve at its input.
    #[inline]
    fn ladder_curve(self, a: f32) -> f32 {
        match self {
            ResMode::Soft => fast_tanh(a),
            ResMode::Hard => hard(a, LADDER_HARD_LIM),
            ResMode::Clean => a,
        }
    }

    /// The ladder's feedback at Resonance 100 %.
    fn ladder_k_max(self) -> f32 {
        if self == ResMode::Clean {
            LADDER_K_CLEAN
        } else {
            LADDER_K_MAX
        }
    }
}

/// The ladder's input curve: one Res mode's, or `w` of the way from
/// `from`'s to `to`'s while the Res mode changes.
#[derive(Clone, Copy, Debug)]
struct Shape {
    from: ResMode,
    to: ResMode,
    w: f32,
}

impl Shape {
    #[inline]
    fn curve(self, a: f32) -> f32 {
        let y = self.to.ladder_curve(a);
        if self.w >= 1.0 {
            y
        } else {
            lerp(self.from.ladder_curve(a), y, self.w)
        }
    }

    /// The feedback at Resonance 100 %, the same way.
    fn k_max(self) -> f32 {
        let k = self.to.ladder_k_max();
        if self.w >= 1.0 {
            k
        } else {
            lerp(self.from.ladder_k_max(), k, self.w)
        }
    }
}

/// The ladder's settings, at twice the rate.
#[derive(Clone, Copy, Debug)]
struct LadderCoefs {
    /// Each one-pole's `g / (1 + g)` and `1 / (1 + g)`.
    big_g: f32,
    inv: f32,
    /// The feedback `k`, the loop's solution `1 / (1 + k G^4)` and the
    /// output's makeup.
    k: f32,
    norm: f32,
    makeup: f32,
}

impl LadderCoefs {
    /// For cutoff `hz`, Resonance `res` (0 to 1) and the feedback at full
    /// Resonance `k_max`, at session rate `rate`.
    fn new(hz: f32, res: f32, k_max: f32, rate: u32) -> LadderCoefs {
        let fs2 = 2.0 * rate as f32;
        let g = (PI * hz.clamp(5.0, 0.45 * fs2) / fs2).tan();
        let big_g = g / (1.0 + g);
        let k = k_max * res.clamp(0.0, 1.0);
        let g4 = (big_g * big_g) * (big_g * big_g);
        LadderCoefs {
            big_g,
            inv: 1.0 / (1.0 + g),
            k,
            norm: 1.0 / (1.0 + k * g4),
            makeup: 1.0 + LADDER_MAKEUP * k,
        }
    }

    /// The linear ladder's gain at `hz` (exact for quiet sounds; the half-
    /// bands are flat over the audio band).
    fn response(&self, hz: f32, rate: u32) -> (f64, f64) {
        // One TPT one-pole: G (1 + z^-1) / (1 + (2G - 1) z^-1), at 2x.
        let w = std::f64::consts::PI * f64::from(hz) / f64::from(rate);
        let (c, sn) = (w.cos(), -w.sin());
        let g = f64::from(self.big_g);
        let num = (g * (1.0 + c), g * sn);
        let den = (1.0 + (2.0 * g - 1.0) * c, (2.0 * g - 1.0) * sn);
        let d2 = den.0 * den.0 + den.1 * den.1;
        let h1 = (
            (num.0 * den.0 + num.1 * den.1) / d2,
            (num.1 * den.0 - num.0 * den.1) / d2,
        );
        let h2 = cmul(h1, h1);
        let h4 = cmul(h2, h2);
        let k = f64::from(self.k);
        let den = (1.0 + k * h4.0, k * h4.1);
        let d2 = den.0 * den.0 + den.1 * den.1;
        let m = f64::from(self.makeup);
        (
            m * (h4.0 * den.0 + h4.1 * den.1) / d2,
            m * (h4.1 * den.0 - h4.0 * den.1) / d2,
        )
    }
}

/// A Moog-style ladder for one channel at twice the rate: four one-pole
/// low-passes in a loop, the sum at its input through the Res mode's curve
/// (`tanh`, a hard clip, or straight), solved without a delay in the loop
/// (Zavalishin's TPT ladder, the curve taken of the loop's linear
/// solution), so it stays in tune up to the top and self-oscillates at the
/// cutoff.
#[derive(Clone, Copy, Debug, Default)]
struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    #[inline]
    fn run(&mut self, x: f32, c: &LadderCoefs, curve: impl Fn(f32) -> f32) -> f32 {
        let g = c.big_g;
        let s = &mut self.s;
        // What the four stages' states alone put out at the last one: the
        // loop's output is G^4 u plus this.
        let held = (((s[0] * g + s[1]) * g + s[2]) * g + s[3]) * c.inv;
        let u = curve((x - c.k * held) * c.norm);
        let mut v = u;
        for st in s.iter_mut() {
            let w = (v - *st) * g;
            let y = w + *st;
            *st = flush(y + w);
            v = y;
        }
        v * c.makeup
    }
}

/// One channel at twice the rate, for the Hard and Ladder engines: up, the
/// drive (as `DriveOs`'s, blended in over its first 3 dB against a delay of
/// the same steps), a filter, down. Its latency is `MULTI_LATENCY`, as the
/// plain engine's.
#[derive(Clone, Debug)]
struct Os2x {
    iir: IirPair<OS_SECTIONS>,
    adaa: Adaa,
    flat: Flat2x,
    /// The straight side's delay at twice the rate, as long as ADAA's.
    pad: [f32; Flat2x::STEPS],
    at: usize,
}

impl Os2x {
    fn new() -> Os2x {
        Os2x {
            iir: IirPair::new(OS_TBW),
            adaa: Adaa::default(),
            flat: Flat2x::default(),
            pad: [0.0; Flat2x::STEPS],
            at: 0,
        }
    }

    /// One frame: `x` in; the drive's gain `g` and blend `amt` (0: the
    /// drive is off and not run); `filter` on each of the two samples.
    #[inline]
    fn run(&mut self, x: f32, g: f32, amt: f32, mut filter: impl FnMut(f32) -> f32) -> f32 {
        let up = self.iir.up(x);
        let mut y = [0.0f32; 2];
        for (j, &a) in up.iter().enumerate() {
            let straight = std::mem::replace(&mut self.pad[self.at], a);
            self.at = (self.at + 1) % Flat2x::STEPS;
            let v = if amt > 0.0 {
                let wet = self.flat.run(self.adaa.run(g * a, soft, soft_big));
                lerp(straight, wet, amt)
            } else {
                straight
            };
            y[j] = filter(v);
        }
        self.iir.down(y)
    }

    /// The drive starts again: its states from silence.
    fn drive_from_silence(&mut self) {
        self.adaa.reset();
        self.flat.reset();
    }

    fn reset(&mut self) {
        self.iir.reset();
        self.drive_from_silence();
        self.pad = [0.0; Flat2x::STEPS];
        self.at = 0;
    }
}

/// The Multi filter's engines: the state-variable filter at the session
/// rate (Res mode Soft or Clean), the same at twice the rate with the clip
/// (Hard), and the ladder.
const PLAIN: usize = 0;
const HARD: usize = 1;
const LADDER: usize = 2;

/// A smoothstep `n` frames of `len` into a fade: smooth at both ends, and
/// with the one leaving at `len - n` the two always add to 1.
#[inline]
fn rise(n: u32, len: u32) -> f32 {
    let t = n as f32 / len as f32;
    t * t * (3.0 - 2.0 * t)
}

/// A multi-mode filter: low-pass and high-pass at 6 to 48 dB an octave,
/// band-pass at 12 to 48 and notch at 12 or 24 (the pads' slopes,
/// [`SlopeStages`]), plus a bell and two shelves; or a Moog-style ladder
/// low-pass (the Model). Resonance up to a long ring, held back on a loud
/// sound as the Res mode says: it bends (Soft), it clips at twice the rate
/// (Hard), or nothing holds it (Clean). Optional drive into a soft
/// saturator before the filter (deliberate distortion, off at 0 dB), at
/// twice the rate with ADAA. A type, slope, model or Res mode change
/// crossfades over 10 ms. Latency: a fixed 4 frames.
pub struct MultiFilter {
    v: [f32; 8],
    rate: u32,
    log_hz: Smooth,
    res: Smooth,
    gain: Smooth,
    drive: Smooth,
    /// The plain engine: its stages; the filter being faded out after a
    /// type or slope change, and the frames of the fade left and in all;
    /// Soft's knee and room.
    st: SlopeStages,
    old: SlopeStages,
    fade_left: u32,
    fade_len: u32,
    knee: f32,
    room: f32,
    /// How far the plain engine is toward Soft from Clean (1 bends, 0 has
    /// no ceiling; Soft's knee and room scale by 1 over it), and where it
    /// is heading: it moves over 10 ms.
    bending: f32,
    bending_to: f32,
    os: [DriveOs; 2],
    /// The plain delay the sound takes while the drive is off.
    dly: [FixedDelay; 2],
    /// The drive's gain for the last drive in dB.
    drive_db: f32,
    drive_gain: f32,
    /// Where the dry side is between the plain delay (0) and the
    /// half-bands' phase match (1), and whether the 2x path is running.
    path: u32,
    os_on: bool,
    grid: u32,
    /// The Hard engine: its stages at twice the rate, the ones fading out
    /// after a type or slope change and the frames of that fade left, its
    /// ceiling, each channel's path, and whether its drive is running.
    st2: SlopeStages,
    old2: SlopeStages,
    fade2_left: u32,
    lim: f32,
    hard_os: [Os2x; 2],
    hard_drive: bool,
    /// The ladder: its settings, each channel's path and ladder, whether
    /// its drive is running, and the Res mode its curve is fading from and
    /// the frames of that fade left.
    lad_c: LadderCoefs,
    lad_os: [Os2x; 2],
    lad: [Ladder; 2],
    lad_drive: bool,
    lad_from: ResMode,
    lad_left: u32,
    /// Each engine's level in frames, 0 to `fade_len`: the one set rises
    /// and the others fall, so a Model or Res mode change crossfades.
    lv: [u32; 3],
}

impl Default for MultiFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiFilter {
    pub fn new() -> MultiFilter {
        let v = defaults(&MULTI_PARAMS);
        let st = SlopeStages::default();
        let mut f = MultiFilter {
            v,
            rate: 48_000,
            log_hz: Smooth::new(v[2].log2()),
            res: Smooth::new(v[3] / 100.0),
            gain: Smooth::new(v[4]),
            drive: Smooth::new(v[5]),
            st,
            old: st,
            fade_left: 0,
            fade_len: 1,
            knee: f32::INFINITY,
            room: 1.0,
            bending: 1.0,
            bending_to: 1.0,
            os: [DriveOs::new(), DriveOs::new()],
            dly: [
                FixedDelay::new(MULTI_LATENCY as usize),
                FixedDelay::new(MULTI_LATENCY as usize),
            ],
            drive_db: 0.0,
            drive_gain: 1.0,
            path: 0,
            os_on: false,
            grid: 0,
            st2: st,
            old2: st,
            fade2_left: 0,
            lim: f32::INFINITY,
            hard_os: [Os2x::new(), Os2x::new()],
            hard_drive: false,
            lad_c: LadderCoefs::new(v[2], v[3] / 100.0, LADDER_K_MAX, 48_000),
            lad_os: [Os2x::new(), Os2x::new()],
            lad: [Ladder::default(); 2],
            lad_drive: false,
            lad_from: ResMode::Soft,
            lad_left: 0,
            lv: [1, 0, 0],
        };
        f.prepare(48_000, mpfree_node::MAX_BLOCK);
        f
    }

    /// Whether the ladder is the model set.
    fn ladder(&self) -> bool {
        self.v[multi::MODEL as usize] >= 0.5
    }

    fn res_mode(&self) -> ResMode {
        ResMode::of(self.v[multi::RES_MODE as usize])
    }

    /// The engine the settings ask for.
    fn engine(&self) -> usize {
        if self.ladder() {
            LADDER
        } else if self.res_mode() == ResMode::Hard {
            HARD
        } else {
            PLAIN
        }
    }

    /// Whether engine `e` runs now: set, or fading in or out.
    fn runs(&self, e: usize) -> bool {
        self.lv[e] > 0 || self.engine() == e
    }

    /// The ladder's curve now.
    fn shape(&self) -> Shape {
        let w = if self.lad_left == 0 {
            1.0
        } else {
            rise(self.fade_len - self.lad_left, self.fade_len)
        };
        Shape {
            from: self.lad_from,
            to: self.res_mode(),
            w,
        }
    }

    fn ladder_coefs(&self, hz: f32, res: f32) -> LadderCoefs {
        LadderCoefs::new(hz, res, self.shape().k_max(), self.rate)
    }

    /// The plain engine's knee and room now: Soft's, scaled by 1 over how
    /// far it is toward Soft from Clean (at Clean nothing bends).
    #[inline]
    fn bend_now(&self) -> (f32, f32) {
        if self.bending >= 1.0 {
            (self.knee, self.room)
        } else if self.bending <= 0.0 {
            (f32::INFINITY, 1.0)
        } else {
            (self.knee / self.bending, self.room / self.bending)
        }
    }

    fn mode(&self) -> SvfMode {
        match self.v[0] as usize {
            1 => SvfMode::HighPass,
            2 => SvfMode::BandPass,
            3 => SvfMode::Notch,
            4 => SvfMode::Bell,
            5 => SvfMode::LowShelf,
            6 => SvfMode::HighShelf,
            _ => SvfMode::LowPass,
        }
    }

    /// The slope's poles (the bell and shelves ignore it).
    fn poles(&self) -> u8 {
        SLOPE_POLES[(self.v[multi::SLOPE as usize] as usize).min(SLOPE_POLES.len() - 1)]
    }

    fn update(&mut self) {
        let (mode, poles) = (self.mode(), self.poles());
        let hz = self.log_hz.value.exp2();
        let (res, gain, rate) = (self.res.value, self.gain.value, self.rate);
        // The resonant stage bends (the last; a notch, bell or shelf has
        // none).
        self.st.set(mode, poles, hz, res, gain, rate);
        let r = (res / BEND_FULL_RES).min(1.0);
        (self.knee, self.room) = bend_for(res);
        if self.runs(HARD) {
            // The same stages at twice the rate; the same ones clip.
            self.st2.set(mode, poles, hz, res, gain, 2 * rate);
            self.lim = if r > 0.0 { HARD_LIM / r } else { f32::INFINITY };
        }
        if self.runs(LADDER) {
            self.lad_c = self.ladder_coefs(hz, res);
        }
    }

    /// The response in dB at `hz` for the settings the filter is heading
    /// to, for drawing. Exact to the filter for quiet sounds; the drive is
    /// left out (it colours by level, it is not a curve).
    pub fn response_db(&self, hz: f32) -> f32 {
        let res_mode = self.res_mode();
        if self.ladder() {
            let k_max = res_mode.ladder_k_max();
            let c = LadderCoefs::new(self.v[2], self.v[3] / 100.0, k_max, self.rate);
            return power_db(c.response(hz, self.rate));
        }
        // Hard runs at twice the rate.
        let rate = if res_mode == ResMode::Hard {
            2 * self.rate
        } else {
            self.rate
        };
        let mut st = SlopeStages::default();
        let (cut, res, gain) = (self.v[2], self.v[3] / 100.0, self.v[4]);
        st.set(self.mode(), self.poles(), cut, res, gain, rate);
        power_db(st.response(hz, rate))
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = MULTI_PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        let (was_engine, was_mode) = (self.engine(), self.res_mode());
        let was = self.v[id as usize];
        self.v[id as usize] = v;
        match id {
            multi::CUTOFF => self.log_hz.target = v.log2(),
            multi::RESONANCE => self.res.target = v / 100.0,
            multi::GAIN => self.gain.target = v,
            multi::DRIVE => self.drive.target = v,
            multi::MODEL | multi::RES_MODE => {
                if v == was {
                    return;
                }
                let mode = self.res_mode();
                // Soft and Clean on the plain engine: its ceiling moves.
                self.bending_to = if mode == ResMode::Clean { 0.0 } else { 1.0 };
                if mode != was_mode && self.lv[LADDER] > 0 {
                    // The ladder's curve and feedback move over 10 ms.
                    if self.lad_left > 0 && mode == self.lad_from {
                        // Turned back during a fade: the same blend,
                        // heading the other way.
                        self.lad_from = was_mode;
                        self.lad_left = self.fade_len - self.lad_left;
                    } else {
                        // As a type change: in the first half of a fade,
                        // from the same curve; later, from the one coming in.
                        if self.lad_left == 0 || 2 * self.lad_left < self.fade_len {
                            self.lad_from = was_mode;
                        }
                        self.lad_left = self.fade_len;
                    }
                }
                let e = self.engine();
                if e != was_engine && self.lv[e] == 0 {
                    self.start(e);
                }
                self.update();
            }
            _ => {
                if v == was {
                    return;
                }
                // Type or slope: the new filter takes over over 10 ms. It
                // carries on from the old one's state (a 12 dB filter's
                // types share it); a stage that was idle starts from
                // silence. A change in the first half of a fade fades from
                // the same old filter; later, from the one that was coming
                // in. The ladder is a low-pass whatever the type.
                if self.runs(PLAIN) {
                    if self.fade_left == 0 || 2 * self.fade_left < self.fade_len {
                        self.old = self.st;
                    }
                    self.fade_left = self.fade_len;
                }
                if self.runs(HARD) {
                    if self.fade2_left == 0 || 2 * self.fade2_left < self.fade_len {
                        self.old2 = self.st2;
                    }
                    self.fade2_left = self.fade_len;
                }
                let (was, was2) = (self.st.layout(), self.st2.layout());
                self.update();
                self.st.clear_new(was);
                self.st2.clear_new(was2);
            }
        }
    }

    /// Engine `e` comes in. The plain and Hard engines carry on from the
    /// other's filter if it is running (a state-variable filter's states
    /// are its band-pass and low-pass, nearly the same at any rate);
    /// otherwise, and the ladder always, from silence.
    fn start(&mut self, e: usize) {
        match e {
            PLAIN => {
                self.clean_from_silence();
                if self.lv[HARD] > 0 {
                    self.st.take_states(&self.st2);
                }
                self.bending = self.bending_to;
            }
            HARD => {
                for p in &mut self.hard_os {
                    p.reset();
                }
                self.hard_drive = false;
                self.fade2_left = 0;
                if self.lv[PLAIN] > 0 {
                    self.st2.take_states(&self.st);
                } else {
                    self.st2.reset();
                }
            }
            _ => {
                for p in &mut self.lad_os {
                    p.reset();
                }
                self.lad = [Ladder::default(); 2];
                self.lad_drive = false;
                self.lad_left = 0;
            }
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let e = self.engine();
        // At rest on the plain engine with the drive off (most of the
        // time): the delay and the stages a block at a time, the same
        // numbers as the loop below.
        let resting = e == PLAIN
            && self.lv == [self.fade_len, 0, 0]
            && self.bending == self.bending_to
            && self.fade_left == 0
            && self.path == 0
            && self.drive.settled()
            && self.drive.value <= 0.0
            && self.log_hz.settled()
            && self.res.settled()
            && self.gain.settled();
        if resting {
            self.os_on = false;
            self.grid = (self.grid + l.len() as u32) % GRID;
            let (knee, room) = self.bend_now();
            self.dly[0].run_block(l);
            self.st.run_block(0, l, knee, room);
            self.dly[1].run_block(r);
            self.st.run_block(1, r, knee, room);
            return;
        }
        // One engine alone (the usual case): just it, as it is.
        if self.lv[e] == self.fade_len && self.lv.iter().sum::<u32>() == self.fade_len {
            match e {
                PLAIN => self.run_alone::<PLAIN>(l, r),
                HARD => self.run_alone::<HARD>(l, r),
                _ => self.run_alone::<LADDER>(l, r),
            }
            return;
        }
        // While one comes in and others leave: each at its own level.
        for i in 0..l.len() {
            let (d, driving) = self.frame_start();
            let x = [l[i], r[i]];
            let runs = [
                self.lv[PLAIN] > 0 || e == PLAIN,
                self.lv[HARD] > 0 || e == HARD,
                self.lv[LADDER] > 0 || e == LADDER,
            ];
            let mut out = [0.0f32; 2];
            for (k, &on) in runs.iter().enumerate() {
                if !on {
                    continue;
                }
                let y = match k {
                    PLAIN => {
                        let y = self.clean_drive(x, d, driving);
                        self.clean_stages(y)
                    }
                    HARD => self.hard_frame(x, d, driving),
                    _ => self.ladder_frame(x, d, driving),
                };
                let w = rise(self.lv[k], self.fade_len);
                out[0] += w * y[0];
                out[1] += w * y[1];
            }
            self.frame_end(runs[HARD], runs[LADDER]);
            for (k, lv) in self.lv.iter_mut().enumerate() {
                *lv = if k == e {
                    (*lv + 1).min(self.fade_len)
                } else {
                    lv.saturating_sub(1)
                };
            }
            l[i] = out[0];
            r[i] = out[1];
        }
    }

    /// Engine `E` alone, frame by frame.
    fn run_alone<const E: usize>(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let (d, driving) = self.frame_start();
            let x = [l[i], r[i]];
            let y = match E {
                PLAIN => {
                    let y = self.clean_drive(x, d, driving);
                    self.clean_stages(y)
                }
                HARD => self.hard_frame(x, d, driving),
                _ => self.ladder_frame(x, d, driving),
            };
            self.frame_end(E == HARD, E == LADDER);
            l[i] = y[0];
            r[i] = y[1];
        }
    }

    /// A frame's settings: the smoothed controls, Soft to Clean, and the
    /// drive in dB and whether it runs.
    #[inline(always)]
    fn frame_start(&mut self) -> (f32, bool) {
        let moving = !(self.log_hz.settled() && self.res.settled() && self.gain.settled());
        if moving {
            self.log_hz.next();
            self.res.next();
            self.gain.next();
            self.update();
        }
        if self.bending != self.bending_to {
            let step = 1.0 / self.fade_len as f32;
            self.bending = if self.bending_to > self.bending {
                (self.bending + step).min(self.bending_to)
            } else {
                (self.bending - step).max(self.bending_to)
            };
        }
        let d = self.drive.next();
        if d != self.drive_db {
            self.drive_db = d;
            self.drive_gain = db_to_gain(d);
        }
        (d, d > 0.0 || !self.drive.settled())
    }

    /// The end of a frame: the half-bands flushed on their grid.
    #[inline(always)]
    fn frame_end(&mut self, hard: bool, ladder: bool) {
        self.grid += 1;
        if self.grid == GRID {
            self.grid = 0;
            if self.os_on {
                for os in &mut self.os {
                    os.iir.settle();
                }
            }
            if hard {
                for p in &mut self.hard_os {
                    p.iir.settle();
                }
            }
            if ladder {
                for p in &mut self.lad_os {
                    p.iir.settle();
                }
            }
        }
    }

    /// The plain engine's drive for one frame: the plain delay, or the 2x
    /// drive against its phase-matched dry side, or between the two.
    #[inline]
    fn clean_drive(&mut self, x: [f32; 2], d: f32, driving: bool) -> [f32; 2] {
        // The plain delay always runs, so it is current when the drive
        // leaves.
        let plain = [self.dly[0].run(x[0]), self.dly[1].run(x[1])];
        let mut y = plain;
        if driving || self.path > 0 {
            if !self.os_on {
                for os in &mut self.os {
                    os.reset();
                }
                self.os_on = true;
            }
            // Deliberate: blend into the saturator over the first 3 dB,
            // against the dry side matched to the half-bands' phase.
            let amt = (d / 3.0).min(1.0);
            self.path = if driving {
                (self.path + 1).min(PATH_FADE)
            } else {
                self.path - 1
            };
            let p = self.path as f32 / PATH_FADE as f32;
            for c in 0..2 {
                let (wet, dry) = self.os[c].run(x[c], self.drive_gain);
                y[c] = lerp(lerp(plain[c], dry, p), wet, amt);
            }
        } else if self.os_on {
            self.os_on = false;
        }
        y
    }

    /// The plain engine's stages for one frame, with a type or slope
    /// change's crossfade.
    #[inline]
    fn clean_stages(&mut self, mut y: [f32; 2]) -> [f32; 2] {
        let (knee, room) = self.bend_now();
        // The 12 and 24 dB filters (only the last stages, old and new) run
        // them as before the steeper slopes; a steeper one runs out of line.
        if !(self.st.last_only() && (self.fade_left == 0 || self.old.last_only())) {
            return self.clean_steep(y, knee, room);
        }
        for c in 0..2 {
            let new = self.st.st.run(c, y[c], knee, room);
            y[c] = if self.fade_left > 0 {
                let old = self.old.st.run(c, y[c], knee, room);
                let t = 1.0 - self.fade_left as f32 / self.fade_len as f32;
                // A raised cosine, smooth at both ends.
                let w = t * t * (3.0 - 2.0 * t);
                lerp(old, new, w)
            } else {
                new
            };
        }
        self.fade_left = self.fade_left.saturating_sub(1);
        y
    }

    /// [`MultiFilter::clean_stages`] with a one-pole or plain stages before
    /// the last, or fading from such a filter.
    #[cold]
    #[inline(never)]
    fn clean_steep(&mut self, mut y: [f32; 2], knee: f32, room: f32) -> [f32; 2] {
        for c in 0..2 {
            let new = self.st.run(c, y[c], knee, room);
            y[c] = if self.fade_left > 0 {
                let old = self.old.run(c, y[c], knee, room);
                let t = 1.0 - self.fade_left as f32 / self.fade_len as f32;
                let w = t * t * (3.0 - 2.0 * t);
                lerp(old, new, w)
            } else {
                new
            };
        }
        self.fade_left = self.fade_left.saturating_sub(1);
        y
    }

    /// The Hard engine for one frame: the drive and the clipped stages at
    /// twice the rate, with a type or slope change's crossfade.
    #[inline]
    fn hard_frame(&mut self, x: [f32; 2], d: f32, driving: bool) -> [f32; 2] {
        if driving && !self.hard_drive {
            for p in &mut self.hard_os {
                p.drive_from_silence();
            }
        }
        self.hard_drive = driving;
        let amt = if driving { (d / 3.0).min(1.0) } else { 0.0 };
        let (g, lim) = (self.drive_gain, self.lim);
        let fade =
            (self.fade2_left > 0).then(|| rise(self.fade_len - self.fade2_left, self.fade_len));
        // The 12 and 24 dB filters (only the last stages, old and new) run
        // them as before the steeper slopes; a steeper one runs out of line
        // ([`MultiFilter::hard_steep`]), so this loop stays as lean as it
        // was.
        if !(self.st2.last_only() && (fade.is_none() || self.old2.last_only())) {
            return self.hard_steep(x, g, amt, lim, fade);
        }
        let (st, old, os) = (&mut self.st2.st, &mut self.old2.st, &mut self.hard_os);
        let mut y = [0.0f32; 2];
        for c in 0..2 {
            y[c] = os[c].run(x[c], g, amt, |v| {
                let new = st.run_clip(c, v, lim);
                match fade {
                    Some(w) => lerp(old.run_clip(c, v, lim), new, w),
                    None => new,
                }
            });
        }
        self.fade2_left = self.fade2_left.saturating_sub(1);
        y
    }

    /// [`MultiFilter::hard_frame`]'s stages with a one-pole or plain
    /// stages before the last (6, 18, 36 and 48 dB, a 24 dB band-pass), or
    /// fading from such a filter.
    #[cold]
    #[inline(never)]
    fn hard_steep(
        &mut self,
        x: [f32; 2],
        g: f32,
        amt: f32,
        lim: f32,
        fade: Option<f32>,
    ) -> [f32; 2] {
        let (st, old, os) = (&mut self.st2, &mut self.old2, &mut self.hard_os);
        let mut y = [0.0f32; 2];
        for c in 0..2 {
            y[c] = os[c].run(x[c], g, amt, |v| {
                let new = st.run_clip(c, v, lim);
                match fade {
                    Some(w) => lerp(old.run_clip(c, v, lim), new, w),
                    None => new,
                }
            });
        }
        self.fade2_left = self.fade2_left.saturating_sub(1);
        y
    }

    /// The ladder for one frame, with a Res mode change's blend of curves.
    #[inline]
    fn ladder_frame(&mut self, x: [f32; 2], d: f32, driving: bool) -> [f32; 2] {
        if driving && !self.lad_drive {
            for p in &mut self.lad_os {
                p.drive_from_silence();
            }
        }
        self.lad_drive = driving;
        let amt = if driving { (d / 3.0).min(1.0) } else { 0.0 };
        if self.lad_left > 0 {
            let sh = self.shape();
            let z = self.ladder_with(x, amt, |a| sh.curve(a));
            self.lad_left -= 1;
            // The next frame's feedback, on its way to the new Res mode's.
            self.lad_c = self.ladder_coefs(self.log_hz.value.exp2(), self.res.value);
            z
        } else {
            match self.res_mode() {
                ResMode::Soft => self.ladder_with(x, amt, fast_tanh),
                ResMode::Hard => self.ladder_with(x, amt, |a| hard(a, LADDER_HARD_LIM)),
                ResMode::Clean => self.ladder_with(x, amt, |a| a),
            }
        }
    }

    /// Both channels' ladders with input curve `curve`.
    #[inline(always)]
    fn ladder_with(
        &mut self,
        x: [f32; 2],
        amt: f32,
        curve: impl Fn(f32) -> f32 + Copy,
    ) -> [f32; 2] {
        let (g, coefs) = (self.drive_gain, self.lad_c);
        let (lad, os) = (&mut self.lad, &mut self.lad_os);
        let mut z = [0.0f32; 2];
        for c in 0..2 {
            let one = &mut lad[c];
            z[c] = os[c].run(x[c], g, amt, |v| one.run(v, &coefs, curve));
        }
        z
    }

    /// The plain engine starts again: from silence.
    fn clean_from_silence(&mut self) {
        self.st.reset();
        self.fade_left = 0;
        for d in &mut self.dly {
            d.reset();
        }
        for os in &mut self.os {
            os.reset();
        }
        self.os_on = false;
        self.path = if self.drive.target > 0.0 {
            PATH_FADE
        } else {
            0
        };
    }

    /// The engines' levels as they are with nothing fading: the one set
    /// heard alone.
    fn settle_levels(&mut self) {
        self.lv = [0; 3];
        self.lv[self.engine()] = self.fade_len;
        self.lad_left = 0;
        self.bending = self.bending_to;
    }
}

impl Node for MultiFilter {
    fn info(&self) -> &'static NodeInfo {
        &MULTI_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &MULTI_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        self.log_hz.set_time(self.rate, 10.0);
        self.res.set_time(self.rate, 10.0);
        self.gain.set_time(self.rate, 10.0);
        self.drive.set_time(self.rate, 10.0);
        self.fade_len = ((FADE_MS * 0.001 * self.rate as f32) as u32).max(1);
        self.fade_left = 0;
        self.fade2_left = 0;
        self.settle_levels();
        self.update();
    }
    fn reset(&mut self) {
        self.log_hz.snap(self.log_hz.target);
        self.res.snap(self.res.target);
        self.gain.snap(self.gain.target);
        self.drive.snap(self.drive.target);
        self.st.reset();
        self.fade_left = 0;
        for d in &mut self.dly {
            d.reset();
        }
        for os in &mut self.os {
            os.reset();
        }
        self.os_on = false;
        self.path = if self.drive.target > 0.0 {
            PATH_FADE
        } else {
            0
        };
        self.drive_db = f32::NAN;
        self.grid = 0;
        self.st2.reset();
        self.fade2_left = 0;
        for p in &mut self.hard_os {
            p.reset();
        }
        self.hard_drive = false;
        for p in &mut self.lad_os {
            p.reset();
        }
        self.lad = [Ladder::default(); 2];
        self.lad_drive = false;
        self.settle_levels();
        self.update();
    }
    fn latency(&self) -> u32 {
        MULTI_LATENCY
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
    fn rest_frames(&self) -> u32 {
        // What it holds is its tail: once rung out it is as good as reset.
        self.tail_frames()
    }
    fn tail_frames(&self) -> u32 {
        // A high-Q ring: about Q / (pi * f) seconds to fall 27 dB; allow 4x.
        let q = res_to_q(self.res.target);
        let hz = self.log_hz.target.exp2().max(20.0);
        let clean = MULTI_LATENCY + (4.0 * q / (PI * hz) * self.rate as f32) as u32;
        if !self.runs(LADDER) {
            return clean;
        }
        // The ladder's ring: its slowest poles decay at `sigma` times the
        // cutoff, `1 - k^(1/4) / sqrt 2` (the analog ladder's), and from k 4
        // up it sings on its own, so it never rests. While its curve
        // changes, the larger feedback of the two.
        let sh = self.shape();
        let k_max = if sh.w >= 1.0 {
            sh.to.ladder_k_max()
        } else {
            sh.from.ladder_k_max().max(sh.to.ladder_k_max())
        };
        let k = k_max * self.res.target.clamp(0.0, 1.0);
        let sigma = 1.0 - k.powf(0.25) * std::f32::consts::FRAC_1_SQRT_2;
        if sigma <= 1e-3 {
            return u32::MAX;
        }
        // 14 time constants: about 120 dB.
        let ladder = MULTI_LATENCY as f32 + 14.0 / (sigma * 2.0 * PI * hz) * self.rate as f32;
        clean.max(ladder.min(u32::MAX as f32 / 2.0) as u32)
    }
}

/// Walks a block's events and the pieces between them without borrowing
/// the node twice (the closure form of `mpfree_node::split` cannot call
/// `&mut self` from both closures). Same order as `split`: events at an offset apply
/// before the frames from that offset.
pub(crate) struct Segs<'e> {
    events: &'e [Timed],
    i: usize,
    at: usize,
    done: bool,
}

impl<'e> Segs<'e> {
    pub(crate) fn new(events: &'e [Timed]) -> Segs<'e> {
        Segs {
            events,
            i: 0,
            at: 0,
            done: false,
        }
    }

    /// Applies the events due at the current frame, then returns the next
    /// piece `(from, to)`, or `None` at the end (after applying any events
    /// at or past the end, so nothing is lost).
    pub(crate) fn next<F: FnMut(&NodeEvent)>(
        &mut self,
        len: usize,
        mut apply: F,
    ) -> Option<(usize, usize)> {
        if self.done {
            return None;
        }
        while self.at < len {
            while self.i < self.events.len() && (self.events[self.i].offset as usize) <= self.at {
                apply(&self.events[self.i].ev);
                self.i += 1;
            }
            let next = self
                .events
                .get(self.i)
                .map_or(len, |e| (e.offset as usize).min(len));
            let from = self.at;
            self.at = next;
            if next > from {
                return Some((from, next));
            }
        }
        for e in &self.events[self.i..] {
            apply(&e.ev);
        }
        self.i = self.events.len();
        self.done = true;
        None
    }
}

/// [`DjFilter`] parameter ids.
pub mod dj {
    pub const POSITION: u16 = 0;
    pub const RESONANCE: u16 = 1;
}

static DJ_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.dj_filter",
    name: "DJ filter",
    kind: Kind::Effect,
};
static DJ_PARAMS: [ParamInfo; 2] = [
    ParamInfo {
        id: 0,
        name: "Position",
        group: "DJ filter",
        min: -100.0,
        max: 100.0,
        default: 0.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Resonance",
        group: "DJ filter",
        min: 0.0,
        max: 100.0,
        default: 30.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// The DJ filter's corner at `pos` (-1 to 1): a low-pass from 20 kHz down
/// to 80 Hz to the left, a high-pass from 20 Hz up to 8 kHz to the right.
fn dj_corner(pos: f32) -> f32 {
    let a = pos.abs();
    if pos < 0.0 {
        20_000.0 * (80.0f32 / 20_000.0).powf(a)
    } else {
        20.0 * (8_000.0f32 / 20.0).powf(a)
    }
}

/// Sets the DJ filter's stage for `pos` and `res` (both -1 or 0 to 1).
/// Kept out of line: inlined (as the stage's `set` is since it moved to
/// `mpfree-filter`), it made the DJ filter's every frame, mostly with the
/// knob still, cost 9.5 % more.
#[inline(never)]
fn dj_set(f: &mut Svf, pos: f32, res: f32, rate: u32) {
    let mode = if pos < 0.0 {
        SvfMode::LowPass
    } else {
        SvfMode::HighPass
    };
    f.set(mode, dj_corner(pos), 0.707 + res * 6.0, 0.0, rate);
}

/// The one-knob DJ filter: position below 0 is a low-pass falling from
/// 20 kHz to 80 Hz, above 0 a high-pass rising from 20 Hz to 8 kHz, and the
/// middle is off. The filter fades in over the first 10 % each side, so the
/// knob can cross the middle without a click.
pub struct DjFilter {
    v: [f32; 2],
    rate: u32,
    pos: Smooth,
    res: Smooth,
    f: Svf,
    last_pos: f32,
    last_res: f32,
}

impl Default for DjFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl DjFilter {
    pub fn new() -> DjFilter {
        let v = defaults(&DJ_PARAMS);
        let mut d = DjFilter {
            v,
            rate: 48_000,
            pos: Smooth::new(0.0),
            res: Smooth::new(v[1] / 100.0),
            f: Svf::default(),
            last_pos: f32::NAN,
            last_res: f32::NAN,
        };
        d.prepare(48_000, mpfree_node::MAX_BLOCK);
        d
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = DJ_PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            dj::POSITION => self.pos.target = v / 100.0,
            _ => self.res.target = v / 100.0,
        }
    }

    fn coefs(&mut self, pos: f32, res: f32) {
        if pos == self.last_pos && res == self.last_res {
            return;
        }
        self.last_pos = pos;
        self.last_res = res;
        dj_set(&mut self.f, pos, res, self.rate);
    }

    /// The response in dB at `hz` for where the knob is heading, for
    /// drawing (exact, the fade-in near the middle included).
    pub fn response_db(&self, hz: f32) -> f32 {
        let (pos, res) = (self.v[0] / 100.0, self.v[1] / 100.0);
        let wet = f64::from((pos.abs() / 0.1).min(1.0));
        if wet == 0.0 {
            return 0.0;
        }
        let mut f = Svf::default();
        dj_set(&mut f, pos, res, self.rate);
        let h = f.response(hz, self.rate);
        power_db((1.0 - wet + wet * h.0, wet * h.1))
    }

    /// The filter's corner for where the knob is heading: `None` in the
    /// middle, else whether it is the low-pass, and its corner in Hz.
    pub fn corner(&self) -> Option<(bool, f32)> {
        let pos = self.v[0] / 100.0;
        if pos == 0.0 {
            return None;
        }
        Some((pos < 0.0, dj_corner(pos)))
    }

    /// One stereo frame (Perform FX runs the filter inside its own loop).
    #[inline]
    pub(crate) fn frame(&mut self, x: f32, y: f32) -> (f32, f32) {
        let p = self.pos.next();
        let q = self.res.next();
        let wet = (p.abs() / 0.1).min(1.0);
        if wet == 0.0 && self.pos.settled() {
            // Off: keep the filter's state quiet so a later turn starts clean.
            self.f.reset();
            return (x, y);
        }
        self.coefs(p, q);
        let fx = self.f.run(0, x);
        let fy = self.f.run(1, y);
        (crate::lerp(x, fx, wet), crate::lerp(y, fy, wet))
    }

    /// Off and staying off: the knob is in the middle and not moving.
    pub(crate) fn resting(&self) -> bool {
        self.pos.value == 0.0 && self.pos.settled()
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            (l[i], r[i]) = self.frame(l[i], r[i]);
        }
    }
}

impl Node for DjFilter {
    fn info(&self) -> &'static NodeInfo {
        &DJ_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &DJ_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        self.pos.set_time(self.rate, 15.0);
        self.res.set_time(self.rate, 15.0);
        self.last_pos = f32::NAN;
    }
    fn reset(&mut self) {
        self.pos.snap(self.pos.target);
        self.res.snap(self.res.target);
        self.f.reset();
        self.last_pos = f32::NAN;
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
    fn rest_frames(&self) -> u32 {
        // What it holds is its tail: once rung out it is as good as reset.
        self.tail_frames()
    }
    fn tail_frames(&self) -> u32 {
        self.rate / 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain for a sine, from RMS over the second half (a sampled peak can
    /// miss the true peak at high frequencies).
    fn sine_gain(f: &mut Svf, hz: f32, rate: u32) -> f32 {
        f.reset();
        let n = rate as usize;
        let (mut sx, mut sy) = (0.0f64, 0.0f64);
        for i in 0..n {
            let x = (2.0 * PI * hz * i as f32 / rate as f32).sin();
            let y = f.run(0, x);
            if i >= n / 2 {
                sx += (x * x) as f64;
                sy += (y * y) as f64;
            }
        }
        (sy / sx).sqrt() as f32
    }

    /// The response of `c` at `hz`, dB.
    fn coefs_db(c: SvfCoefs, hz: f32, rate: u32) -> f64 {
        let mut f = Svf::default();
        f.set_coefs(c);
        let (re, im) = f.response(hz, rate);
        10.0 * (re * re + im * im).log10()
    }

    #[test]
    fn matched_designs_are_the_bilinear_ones_down_low() {
        // Far below Nyquist the two designs are the same filter; the
        // other modes are bilinear either way.
        let rate = 48_000;
        for mode in [SvfMode::Bell, SvfMode::LowShelf, SvfMode::HighShelf] {
            for hz in [60.0f32, 250.0] {
                let (m, b) = (
                    SvfCoefs::matched(mode, hz, 0.9, 9.0, rate),
                    SvfCoefs::bilinear(mode, hz, 0.9, 9.0, rate),
                );
                for f in [20.0f32, hz, 4.0 * hz, 2000.0] {
                    let (dm, db) = (coefs_db(m, f, rate), coefs_db(b, f, rate));
                    assert!(
                        (dm - db).abs() < 0.02,
                        "{mode:?} {hz} Hz at {f}: {dm:.3} {db:.3}"
                    );
                }
            }
        }
        // The other modes fall through to the bilinear design, to the bit.
        // Its cutoff goes through `black_box`, or a release build works its
        // `tan` out while compiling (rounded correctly), while `matched`,
        // in mpfree-filter and not inlined here, runs the library's `tan`:
        // at 9 kHz the two are an ulp apart.
        let hz = std::hint::black_box(9000.0);
        assert_eq!(
            SvfCoefs::matched(SvfMode::LowPass, 9000.0, 2.0, 0.0, rate),
            SvfCoefs::bilinear(SvfMode::LowPass, hz, 2.0, 0.0, rate)
        );
    }

    #[test]
    fn a_glide_between_designs_ends_on_the_last() {
        let a = SvfCoefs::matched(SvfMode::Bell, 1000.0, 1.0, 6.0, 48_000);
        let b = SvfCoefs::matched(SvfMode::Bell, 9000.0, 3.0, -9.0, 48_000);
        assert_eq!(a.toward(b, 0.0), a);
        let mid = a.toward(b, 0.5);
        assert!(mid.g > 0.0 && mid.k > 0.0);
        let end = a.toward(b, 1.0);
        for (x, y) in [(end.g, b.g), (end.k, b.k), (end.m1, b.m1)] {
            assert!((x - y).abs() < 1e-6);
        }
    }

    #[test]
    fn low_pass_and_high_pass_slopes() {
        let mut f = Svf::default();
        f.set(SvfMode::LowPass, 1000.0, 0.707, 0.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 100.0, 48_000))).abs() < 0.2);
        let at10k = crate::gain_to_db(sine_gain(&mut f, 10_000.0, 48_000));
        assert!(at10k < -38.0 && at10k > -44.0, "{at10k}");
        f.set(SvfMode::HighPass, 1000.0, 0.707, 0.0, 48_000);
        assert!(crate::gain_to_db(sine_gain(&mut f, 10_000.0, 48_000)).abs() < 0.2);
        assert!(crate::gain_to_db(sine_gain(&mut f, 100.0, 48_000)) < -38.0);
    }

    #[test]
    fn bell_and_shelves_hit_their_gain() {
        let mut f = Svf::default();
        f.set(SvfMode::Bell, 1000.0, 1.0, 6.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 1000.0, 48_000)) - 6.0).abs() < 0.2);
        f.set(SvfMode::LowShelf, 300.0, 0.707, -9.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 30.0, 48_000)) + 9.0).abs() < 0.3);
        assert!(crate::gain_to_db(sine_gain(&mut f, 8000.0, 48_000)).abs() < 0.2);
        f.set(SvfMode::HighShelf, 3000.0, 0.707, 4.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 15_000.0, 48_000)) - 4.0).abs() < 0.3);
    }
}
