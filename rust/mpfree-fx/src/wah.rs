//! Moving filters: the [`AutoWah`] and the [`TalkBox`].
//!
//! - The auto wah sweeps one resonant filter between a low and a high
//!   frequency, moved by the sound's own level (an envelope follower with
//!   its attack and release, as the MPC's Auto Wah), by an LFO (free or in
//!   time with the tempo), or by hand (a pedal on a Q-Link). The sweep is
//!   even in pitch. Its Wah shape is the band-pass with some low end
//!   leaking through, as a pedal's does. The band's gain rises with the
//!   square root of its Q, so a sharper peak keeps the effect's level and
//!   the defaults go in at the dry's. Resonance, Low and High glide, a
//!   Filter change crossfades the shapes and a Source change glides the
//!   sweep from where it was, so nothing clicks.
//! - The talk box is the MPC's AIR Talk Box idea: three formant filters
//!   tuned to a vowel (average adult formants from Peterson and Barney,
//!   1952), so the sound seems to say "ah" or "ee". They are Klatt's
//!   parallel bank with alternating signs, so they add between formants
//!   instead of cutting notches, with levels fitted so a saw comes out at
//!   a sung vowel's and each vowel at the dry's loudness. The envelope and
//!   the LFO move it along the row of vowels (back to front: OO, OU, AW,
//!   AH, UH, AE, EH, IH, EE), so it talks as the level moves or in time;
//!   the formants glide between vowels in pitch, and Formant shifts them
//!   all for a smaller or bigger mouth. See `docs/dev/design/fx-voice.md`.
//!
//! Both are trapezoidal state-variable filters (Simper), retuned every
//! [`CTRL`] frames from a smoothed position, so a sweep costs a few
//! tangents a block rather than one a frame; the coefficients glide from
//! one retune to the next, so a fast sweep doesn't buzz at the retune
//! rate.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Kind, NodeInfo, ParamInfo, Smooth, Unit};

use crate::kit::{
    self, node_impl, param, Choice, Follower, Glide, Hz, Lfo, Linear, Log, Ms, Percent, NONE,
};
use crate::{defaults, fast_db_to_gain, fast_exp2, fast_log2, flush, lerp, Preset};

/// Frames between retunes.
pub(crate) const CTRL: usize = 16;

/// One stereo trapezoidal state-variable stage, tuned by [`Sv::tune`], or
/// gliding to a tuning a frame at a time ([`Sv::glide_to`], [`Sv::advance`]).
#[derive(Clone, Copy, Debug, Default)]
struct Sv {
    k: f32,
    a1: f32,
    a2: f32,
    a3: f32,
    /// The coefficients' steps a frame while gliding.
    d: [f32; 3],
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl Sv {
    /// `k`, `a1`, `a2` and `a3` for a peak at `hz` of sharpness `q`.
    fn coefs(hz: f32, q: f32, rate: u32) -> [f32; 4] {
        let fs = rate as f32;
        let g = (PI * hz.clamp(10.0, 0.45 * fs) / fs).tan();
        let k = 1.0 / q.max(0.1);
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        [k, a1, a2, g * a2]
    }
    fn tune(&mut self, hz: f32, q: f32, rate: u32) {
        [self.k, self.a1, self.a2, self.a3] = Sv::coefs(hz, q, rate);
        self.d = [0.0; 3];
    }
    /// Glides the coefficients to the tuning over `frames` calls of
    /// [`Sv::advance`], so a sweep has no steps to buzz with (FX quality
    /// W18). `k` is set at once; the caller ramps what it scales.
    fn glide_to(&mut self, hz: f32, q: f32, rate: u32, frames: usize) {
        let [k, a1, a2, a3] = Sv::coefs(hz, q, rate);
        let per = 1.0 / frames as f32;
        self.k = k;
        self.d = [
            (a1 - self.a1) * per,
            (a2 - self.a2) * per,
            (a3 - self.a3) * per,
        ];
    }
    #[inline]
    fn advance(&mut self) {
        self.a1 += self.d[0];
        self.a2 += self.d[1];
        self.a3 += self.d[2];
    }
    /// `(low, band)` for one sample of channel `ch`.
    #[inline]
    fn run(&mut self, ch: usize, v0: f32) -> (f32, f32) {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = v0 - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;
        self.ic1[ch] = flush(2.0 * v1 - ic1);
        self.ic2[ch] = flush(2.0 * v2 - ic2);
        (v2, v1)
    }
    fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
    }
}

/// How much louder the envelope reads at full Sensitivity, in dB.
const SENS_DB: f32 = 42.0;

/// The envelope follower's level (0 up) as a sweep amount, 0 to 1: soft
/// at the top, so loud notes don't all hit the end stop.
#[inline]
fn env_amount(e: f32) -> f32 {
    e / (1.0 + e)
}

// ---------------------------------------------------------------- auto wah

/// [`AutoWah`] parameter ids.
pub mod autowah {
    pub const SOURCE: u16 = 0;
    pub const CENTER: u16 = 1;
    pub const DEPTH: u16 = 2;
    pub const SENSITIVITY: u16 = 3;
    pub const ATTACK: u16 = 4;
    pub const RELEASE: u16 = 5;
    pub const RATE: u16 = 6;
    pub const SYNC: u16 = 7;
    pub const LOW: u16 = 8;
    pub const HIGH: u16 = 9;
    pub const RESONANCE: u16 = 10;
    pub const FILTER: u16 = 11;
    pub const MIX: u16 = 12;
}

const SOURCES: &[&str] = &["Envelope", "LFO", "Pedal"];
/// The filter shapes: Wah is the band-pass with some of the low end
/// leaking through, as a wah pedal's does, so a bass line keeps its body.
const FILTERS: &[&str] = &["Wah", "Band-pass", "Low-pass", "High-pass"];
/// How much low end the Wah shape lets through (about -9 dB).
const LEAK: f32 = 0.35;
/// The band's gain is this times the square root of its Q: a band-pass
/// passes about 1/Q of a broad sound's power, so the effect keeps its
/// level as Resonance sharpens the peak (FX quality W17), and the defaults
/// go in at the dry's level on pink noise (W16).
const BAND_GAIN: f32 = 1.95;
/// How long Resonance, Low and High glide, and the Filter shapes
/// crossfade, in ms (FX quality W18).
const GLIDE_MS: f32 = 20.0;
/// How long the sweep glides to where a new Source puts it, in ms.
const SOURCE_MS: f32 = 30.0;

/// The filter's Q at Resonance `res` (0 to 1).
#[inline]
fn wah_q(res: f32) -> f32 {
    0.7 + res * res * 11.3
}

static AUTOWAH_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.autowah",
    name: "Auto wah",
    kind: Kind::Effect,
};
static AUTOWAH_PARAMS: [ParamInfo; 13] = [
    param(0, "Source", "Wah", 0.0, 2.0, 0.0, NONE, Choice(SOURCES)),
    param(1, "Center", "Wah", 0.0, 100.0, 0.0, Percent, Linear),
    param(2, "Depth", "Wah", -100.0, 100.0, 100.0, Percent, Linear),
    param(
        3,
        "Sensitivity",
        "Envelope",
        0.0,
        100.0,
        50.0,
        Percent,
        Linear,
    ),
    param(4, "Attack", "Envelope", 1.0, 200.0, 8.0, Ms, Log),
    param(5, "Release", "Envelope", 10.0, 2000.0, 150.0, Ms, Log),
    param(6, "Rate", "LFO", 0.05, 10.0, 1.0, Hz, Log),
    param(7, "Sync", "LFO", 0.0, 13.0, 0.0, NONE, Choice(kit::SYNCS)),
    param(8, "Low", "Filter", 100.0, 1500.0, 300.0, Hz, Log),
    param(9, "High", "Filter", 500.0, 6000.0, 2500.0, Hz, Log),
    param(10, "Resonance", "Filter", 0.0, 100.0, 60.0, Percent, Linear),
    param(11, "Filter", "Filter", 0.0, 3.0, 0.0, NONE, Choice(FILTERS)),
    param(12, "Mix", "Wah", 0.0, 100.0, 100.0, Percent, Linear),
];

/// Auto wah factory presets.
pub static AUTOWAH_PRESETS: &[Preset] = &[
    Preset {
        name: "Funk quack",
        values: &[
            (autowah::SENSITIVITY, 65.0),
            (autowah::RELEASE, 90.0),
            (autowah::RESONANCE, 75.0),
        ],
    },
    Preset {
        name: "Slow bloom",
        values: &[
            (autowah::ATTACK, 60.0),
            (autowah::RELEASE, 600.0),
            (autowah::LOW, 200.0),
            (autowah::HIGH, 1800.0),
            (autowah::FILTER, 2.0),
        ],
    },
    Preset {
        name: "Down wah",
        values: &[
            (autowah::CENTER, 100.0),
            (autowah::DEPTH, -100.0),
            (autowah::SENSITIVITY, 60.0),
        ],
    },
    Preset {
        name: "Eighth sweep",
        values: &[
            (autowah::SOURCE, 1.0),
            (autowah::SYNC, 5.0),
            (autowah::CENTER, 50.0),
            (autowah::DEPTH, 50.0),
        ],
    },
    Preset {
        name: "Pedal",
        values: &[
            (autowah::SOURCE, 2.0),
            (autowah::CENTER, 50.0),
            (autowah::RESONANCE, 80.0),
        ],
    },
];

/// A wah moved by the sound's level, an LFO or by hand.
pub struct AutoWah {
    v: [f32; 13],
    rate: u32,
    lfo: Lfo,
    env: Follower,
    filter: Sv,
    /// The sweep position, 0 to 1 (glides between retunes).
    pos: Smooth,
    /// Resonance (0 to 1), and Low and High as log2 Hz, gliding a retune
    /// at a time.
    res: Smooth,
    lo: Smooth,
    hi: Smooth,
    /// The band's output gain (`k` times [`BAND_GAIN`]·√Q) and the
    /// high-pass's `k`, each with its step a frame, so they ramp from one
    /// retune to the next instead of stepping.
    band: f32,
    band_step: f32,
    k: f32,
    k_step: f32,
    /// The Filter shape, and while it changes, how much of each shape is
    /// heard: each share glides toward 1 for the shape and 0 for the rest
    /// through two one-poles (`pull`, then `shares`), so they always add
    /// up to 1, start and stop gently, and a change mid-way glides on from
    /// where it is.
    shape: usize,
    fading: bool,
    pull: [f32; 4],
    shares: [f32; 4],
    share_k: f32,
    /// The Source in use; after a change, the gap between where the old
    /// one had the sweep and where the new one has it, faded out over
    /// [`SOURCE_MS`] (retunes left of how many).
    source: usize,
    jump: f32,
    jump_left: u32,
    jump_len: u32,
    mix: Smooth,
    /// Frames until the next retune.
    ctrl: usize,
    sens: f32,
    lfo_now: f32,
}

impl Default for AutoWah {
    fn default() -> Self {
        Self::new()
    }
}

impl AutoWah {
    pub fn new() -> AutoWah {
        let v = defaults(&AUTOWAH_PARAMS);
        let mut w = AutoWah {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            env: Follower::default(),
            filter: Sv::default(),
            pos: Smooth::new(v[1] / 100.0),
            res: Smooth::new(v[10] / 100.0),
            lo: Smooth::new(v[8].log2()),
            hi: Smooth::new(v[9].log2()),
            band: 0.0,
            band_step: 0.0,
            k: 1.0,
            k_step: 0.0,
            shape: v[11] as usize,
            fading: false,
            pull: [0.0; 4],
            shares: [0.0; 4],
            share_k: 1.0,
            source: v[0] as usize,
            jump: 0.0,
            jump_left: 0,
            jump_len: 1,
            mix: Smooth::new(v[12] / 100.0),
            ctrl: 0,
            sens: 1.0,
            lfo_now: 0.0,
        };
        w.setup(48_000);
        w.clear();
        w
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            autowah::SENSITIVITY => self.sens = fast_db_to_gain(v / 100.0 * SENS_DB),
            autowah::ATTACK | autowah::RELEASE => {
                self.env.set(self.v[4], self.v[5], self.rate);
            }
            autowah::MIX => self.mix.target = v / 100.0,
            autowah::RESONANCE => self.res.target = v / 100.0,
            autowah::LOW => self.lo.target = v.log2(),
            autowah::HIGH => self.hi.target = v.log2(),
            autowah::FILTER => {
                let shape = (v as usize).min(FILTERS.len() - 1);
                if shape != self.shape {
                    self.shape = shape;
                    self.fading = true;
                }
            }
            autowah::SOURCE => {
                // The sweep glides from where the old Source had it.
                let source = (v as usize).min(SOURCES.len() - 1);
                if source != self.source {
                    self.source = source;
                    self.jump = self.pos.target - self.target();
                    self.jump_left = self.jump_len;
                }
            }
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    /// The position the controls and the modulation ask for, 0 to 1.
    fn target(&self) -> f32 {
        let m = match self.source {
            0 => env_amount(self.env.env),
            1 => 0.5 + 0.5 * self.lfo_now,
            _ => 0.0,
        };
        (self.v[1] / 100.0 + self.v[2] / 100.0 * m).clamp(0.0, 1.0)
    }
    /// Every [`CTRL`] frames: glide the controls a step, move the sweep
    /// and retune, and set the gains' ramps to land on the new ones.
    fn retune(&mut self) {
        let [m] = self.lfo.read([0.0]);
        self.lfo_now = m;
        let mut to = self.target();
        if self.jump_left > 0 {
            // A raised cosine from the whole gap down to none.
            let w = 0.5 - 0.5 * kit::cosine(0.5 * self.jump_left as f32 / self.jump_len as f32);
            to = (to + self.jump * w).clamp(0.0, 1.0);
            self.jump_left -= 1;
        }
        self.pos.target = to;
        let (lo, hi) = (self.lo.glide(), self.hi.glide());
        let q = wah_q(self.res.glide());
        let hz = fast_exp2(lo + (hi - lo) * self.pos.glide());
        self.filter.glide_to(hz, q, self.rate, CTRL);
        let per = 1.0 / CTRL as f32;
        self.band_step = (self.filter.k * BAND_GAIN * q.sqrt() - self.band) * per;
        self.k_step = (self.filter.k - self.k) * per;
    }
    /// The output of filter shape `shape` from the input and the filter's
    /// low and band.
    #[inline(always)]
    fn shaped(&self, shape: usize, x: f32, low: f32, band: f32) -> f32 {
        match shape {
            1 => self.band * band,
            2 => low,
            3 => x - self.k * band - low,
            _ => self.band * band + LEAK * low,
        }
    }
    /// A frame of a Filter change: each shape's share a step toward where
    /// it is going, until the new shape is all there is.
    #[cold]
    #[inline(never)]
    fn fade_shares(&mut self) {
        let c = self.share_k;
        for (k, (p, w)) in self.pull.iter_mut().zip(&mut self.shares).enumerate() {
            let to = if k == self.shape { 1.0 } else { 0.0 };
            *p += (to - *p) * c;
            *w += (*p - *w) * c;
        }
        if 1.0 - self.shares[self.shape] < 1e-5 {
            self.settle_shares();
        }
    }
    fn settle_shares(&mut self) {
        self.shares = [0.0; 4];
        self.shares[self.shape] = 1.0;
        self.pull = self.shares;
        self.fading = false;
    }
    fn run(&mut self, ctx: &Ctx, from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.follow(ctx, from, self.v[7] as usize);
        self.lfo.set(self.v[6], self.v[7] as usize, self.rate);
        for i in 0..l.len() {
            self.lfo.tick();
            self.env.run(0.5 * (l[i].abs() + r[i].abs()) * self.sens);
            if self.ctrl == 0 {
                self.ctrl = CTRL;
                self.retune();
            } else {
                self.pos.glide();
            }
            self.ctrl -= 1;
            self.filter.advance();
            self.band += self.band_step;
            self.k += self.k_step;
            let mix = self.mix.glide();
            let shape = self.shape;
            if self.fading {
                self.fade_shares();
                let w = self.shares;
                for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                    let x = *s;
                    let (low, band) = self.filter.run(ch, x);
                    let mut y = 0.0;
                    for (k, w) in w.iter().enumerate() {
                        y += w * self.shaped(k, x, low, band);
                    }
                    *s = lerp(x, y, mix);
                }
            } else {
                for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                    let x = *s;
                    let (low, band) = self.filter.run(ch, x);
                    *s = lerp(x, self.shaped(shape, x, low, band), mix);
                }
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        // The position glides per frame between retunes; the follower
        // sets the pace, this only smooths the steps.
        self.pos.set_time(rate, 4.0);
        self.mix.set_time(rate, 20.0);
        let retunes = (rate / CTRL as u32).max(1);
        self.res.set_time(retunes, GLIDE_MS);
        self.lo.set_time(retunes, GLIDE_MS);
        self.hi.set_time(retunes, GLIDE_MS);
        // Each one-pole an eighth of the glide time: half way in about
        // half of it, and within 1e-5 of the new shape in under twice it.
        self.share_k = 1.0 - (-8.0 / (GLIDE_MS * 0.001 * rate as f32).max(1.0)).exp();
        self.jump_len = ((SOURCE_MS * 0.001 * retunes as f32) as u32).max(1);
        self.env.set(self.v[4], self.v[5], rate);
        self.sens = fast_db_to_gain(self.v[3] / 100.0 * SENS_DB);
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.env.reset();
        self.filter.reset();
        self.ctrl = 0;
        self.lfo_now = 0.0;
        self.jump_left = 0;
        self.settle_shares();
        self.pos.snap(self.target());
        self.res.snap(self.res.target);
        self.lo.snap(self.lo.target);
        self.hi.snap(self.hi.target);
        self.mix.snap(self.mix.target);
        // The filter and the gains start where the first retune puts them.
        let q = wah_q(self.res.value);
        let (lo, hi) = (self.lo.value, self.hi.value);
        let hz = fast_exp2(lo + (hi - lo) * self.pos.value);
        self.filter.tune(hz, q, self.rate);
        self.k = 1.0 / q;
        self.band = self.k * BAND_GAIN * q.sqrt();
        self.band_step = 0.0;
        self.k_step = 0.0;
    }
    fn tail(&self) -> u32 {
        // The filter rings for a few periods of its lowest note.
        self.rate / 20
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(AutoWah, AUTOWAH_INFO, AUTOWAH_PARAMS);

// ---------------------------------------------------------------- talk box

/// [`TalkBox`] parameter ids.
pub mod talkbox {
    pub const VOWEL: u16 = 0;
    pub const FORMANT: u16 = 1;
    pub const RESONANCE: u16 = 2;
    pub const ENV_DEPTH: u16 = 3;
    pub const SENSITIVITY: u16 = 4;
    pub const ATTACK: u16 = 5;
    pub const RELEASE: u16 = 6;
    pub const LFO_RATE: u16 = 7;
    pub const LFO_SYNC: u16 = 8;
    pub const LFO_SHAPE: u16 = 9;
    pub const LFO_DEPTH: u16 = 10;
    pub const MIX: u16 = 11;
}

/// The vowels, back to front, so a sweep passes through its neighbours.
pub const VOWELS: &[&str] = &["OO", "OU", "AW", "AH", "UH", "AE", "EH", "IH", "EE"];

/// Each vowel's three formants in Hz (Peterson and Barney's adult
/// averages), the second and third formants' gains in dB against the
/// first's, and the vowel's make-up gain in dB.
///
/// The bank is Klatt's parallel one with alternating signs (F1 +, F2 -,
/// F3 +), so the band-passes add between formants instead of cancelling
/// (same-sign band-passes cut notches 7 to 16 dB deeper than a mouth's
/// there). The gains are fitted so a 110 Hz saw comes out with Peterson
/// and Barney's sung levels under F1 (FX quality V3), and the make-up
/// puts each vowel at the dry's loudness, halfway between a pink noise's
/// and a saw's (V4). The fit is mpfree-claude
/// `fx/quality/tools/talkbox_proto.py`.
const FORMANTS: [([f32; 3], [f32; 2], f32); 9] = [
    ([300.0, 870.0, 2240.0], [-8.60, -23.98], 12.13), // OO "who'd"
    ([440.0, 1020.0, 2240.0], [-3.26, -17.76], 13.16), // OU "hood"
    ([570.0, 840.0, 2410.0], [-1.08, -22.51], 14.31), // AW "hawed"
    ([730.0, 1090.0, 2440.0], [-2.58, -18.91], 16.23), // AH "father"
    ([640.0, 1190.0, 2390.0], [-3.39, -14.69], 15.47), // UH "hud"
    ([660.0, 1720.0, 2410.0], [-0.13, -9.39], 14.62), // AE "had"
    ([530.0, 1840.0, 2480.0], [-3.81, -7.47], 14.52), // EH "head"
    ([390.0, 1990.0, 2550.0], [-10.51, -12.13], 13.94), // IH "hid"
    ([270.0, 2290.0, 3010.0], [-3.61, -4.97], 11.34), // EE "heed"
];
/// The formants' bandwidths in Hz at Resonance 50 % and Formant 0; they
/// scale with Formant (a smaller mouth's are wider), so each formant's Q
/// stays as Resonance sets it.
const BANDWIDTH: [f32; 3] = [80.0, 100.0, 140.0];
/// The bank's signs: alternating, so neighbouring formants add between
/// their peaks.
const SIGNS: [f32; 3] = [1.0, -1.0, 1.0];
/// How many vowels the envelope and the LFO move it at full depth.
const REACH: f32 = 4.0;

static TALKBOX_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.talkbox",
    name: "Talk box",
    kind: Kind::Effect,
};
static TALKBOX_PARAMS: [ParamInfo; 12] = [
    param(0, "Vowel", "Voice", 0.0, 8.0, 3.0, NONE, Choice(VOWELS)),
    param(
        1,
        "Formant",
        "Voice",
        -12.0,
        12.0,
        0.0,
        Unit::Semitones,
        Linear,
    ),
    param(2, "Resonance", "Voice", 0.0, 100.0, 50.0, Percent, Linear),
    param(
        3,
        "Env depth",
        "Envelope",
        -100.0,
        100.0,
        0.0,
        Percent,
        Linear,
    ),
    param(
        4,
        "Sensitivity",
        "Envelope",
        0.0,
        100.0,
        50.0,
        Percent,
        Linear,
    ),
    param(5, "Attack", "Envelope", 1.0, 2000.0, 30.0, Ms, Log),
    param(6, "Release", "Envelope", 10.0, 5000.0, 300.0, Ms, Log),
    param(7, "LFO rate", "LFO", 0.05, 10.0, 1.0, Hz, Log),
    param(
        8,
        "LFO sync",
        "LFO",
        0.0,
        13.0,
        0.0,
        NONE,
        Choice(kit::SYNCS),
    ),
    param(
        9,
        "LFO shape",
        "LFO",
        0.0,
        5.0,
        0.0,
        NONE,
        Choice(kit::SHAPES),
    ),
    param(10, "LFO depth", "LFO", -100.0, 100.0, 0.0, Percent, Linear),
    param(11, "Mix", "Voice", 0.0, 100.0, 100.0, Percent, Linear),
];

/// Talk box factory presets.
pub static TALKBOX_PRESETS: &[Preset] = &[
    Preset {
        name: "Yeah",
        values: &[
            (talkbox::VOWEL, 8.0),
            (talkbox::ENV_DEPTH, -100.0),
            (talkbox::SENSITIVITY, 60.0),
        ],
    },
    Preset {
        name: "Wow wow",
        values: &[
            (talkbox::VOWEL, 0.0),
            (talkbox::LFO_SYNC, 8.0),
            (talkbox::LFO_SHAPE, 1.0),
            (talkbox::LFO_DEPTH, 100.0),
        ],
    },
    Preset {
        name: "Robot choir",
        values: &[
            (talkbox::VOWEL, 3.0),
            (talkbox::RESONANCE, 85.0),
            (talkbox::LFO_RATE, 0.2),
            (talkbox::LFO_DEPTH, 40.0),
        ],
    },
    Preset {
        name: "Small mouth",
        values: &[(talkbox::VOWEL, 6.0), (talkbox::FORMANT, 5.0)],
    },
];

/// The formants (log2 Hz, before Formant's shift) and their signed gains,
/// make-up included, at vowel position `v` (0 to 8): the frequencies glide
/// in pitch and the gains in dB from one vowel to the next.
fn vowel_at(v: f32) -> ([f32; 3], [f32; 3]) {
    let v = v.clamp(0.0, (FORMANTS.len() - 1) as f32);
    let i = (v as usize).min(FORMANTS.len() - 2);
    let t = v - i as f32;
    let (a, b) = (&FORMANTS[i], &FORMANTS[i + 1]);
    let mut f = [0.0; 3];
    for k in 0..3 {
        f[k] = lerp(fast_log2(a.0[k]), fast_log2(b.0[k]), t);
    }
    let up = lerp(a.2, b.2, t);
    let g = [
        SIGNS[0] * fast_db_to_gain(up),
        SIGNS[1] * fast_db_to_gain(lerp(a.1[0], b.1[0], t) + up),
        SIGNS[2] * fast_db_to_gain(lerp(a.1[1], b.1[1], t) + up),
    ];
    (f, g)
}

/// Vowel formants that talk, moved by the level or an LFO.
pub struct TalkBox {
    v: [f32; 12],
    rate: u32,
    lfo: Lfo,
    env: Follower,
    bank: [Sv; 3],
    /// Each formant's output gain (sign, level, make-up and its band's
    /// `k`), ramped a frame at a time to the next retune's.
    gains: [f32; 3],
    steps: [f32; 3],
    /// The vowel position, 0 to 8 (glides between retunes).
    pos: Smooth,
    /// The Vowel control, gliding a step a retune, so stepping through
    /// the vowels doesn't click (V5); the envelope and the LFO move the
    /// position from here at their own pace.
    vowel: Smooth,
    /// Formant in octaves and Resonance's sharpness in octaves (-1 to 1),
    /// gliding a step a retune.
    shift: Smooth,
    sharp: Smooth,
    mix: Smooth,
    ctrl: usize,
    sens: f32,
    lfo_now: f32,
}

impl Default for TalkBox {
    fn default() -> Self {
        Self::new()
    }
}

impl TalkBox {
    pub fn new() -> TalkBox {
        let v = defaults(&TALKBOX_PARAMS);
        let mut t = TalkBox {
            v,
            rate: 48_000,
            lfo: Lfo::default(),
            env: Follower::default(),
            bank: [Sv::default(); 3],
            gains: [0.0; 3],
            steps: [0.0; 3],
            pos: Smooth::new(v[0]),
            vowel: Smooth::new(v[0]),
            shift: Smooth::new(v[1] / 12.0),
            sharp: Smooth::new(v[2] / 50.0 - 1.0),
            mix: Smooth::new(v[11] / 100.0),
            ctrl: 0,
            sens: 1.0,
            lfo_now: 0.0,
        };
        t.setup(48_000);
        t.clear();
        t
    }
    fn apply(&mut self, id: u16, v: f32) {
        match id {
            talkbox::SENSITIVITY => self.sens = fast_db_to_gain(v / 100.0 * SENS_DB),
            talkbox::ATTACK | talkbox::RELEASE => {
                self.env.set(self.v[5], self.v[6], self.rate);
            }
            talkbox::VOWEL => self.vowel.target = v,
            talkbox::FORMANT => self.shift.target = v / 12.0,
            talkbox::RESONANCE => self.sharp.target = v / 50.0 - 1.0,
            talkbox::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }
    fn begin(&mut self, _ctx: &Ctx) {}
    fn target(&self) -> f32 {
        let env = env_amount(self.env.env) * self.v[3] / 100.0;
        let lfo = self.lfo_now * self.v[10] / 100.0;
        self.vowel.value + REACH * (env + lfo)
    }
    /// Each formant's frequency, Q and output gain where the controls are
    /// now. Resonance 0 to 100 % is half to twice as sharp as speech; a
    /// sharper band passes less, so its gain rises with the square root of
    /// the sharpness and the level stays put (V4).
    fn tuning(&self) -> [(f32, f32, f32); 3] {
        let (f, g) = vowel_at(self.pos.value);
        let sharp = fast_exp2(self.sharp.value);
        let shift = self.shift.value;
        let up = sharp.sqrt();
        std::array::from_fn(|k| {
            let hz = fast_exp2(f[k]);
            let q = hz / BANDWIDTH[k] * sharp;
            (hz * fast_exp2(shift), q, g[k] * up / q)
        })
    }
    /// Every [`CTRL`] frames: move the vowel, glide the controls a step,
    /// and set the filters and gains gliding to where they land.
    fn retune(&mut self) {
        let [m] = self.lfo.read([0.0]);
        self.lfo_now = m;
        self.vowel.glide();
        self.pos.target = self.target();
        self.pos.glide();
        self.shift.glide();
        self.sharp.glide();
        let per = 1.0 / CTRL as f32;
        for (k, (hz, q, g)) in self.tuning().into_iter().enumerate() {
            self.bank[k].glide_to(hz, q, self.rate, CTRL);
            self.steps[k] = (g - self.gains[k]) * per;
        }
    }
    fn run(&mut self, ctx: &Ctx, from: usize, l: &mut [f32], r: &mut [f32]) {
        self.lfo.follow(ctx, from, self.v[8] as usize);
        self.lfo.set(self.v[7], self.v[8] as usize, self.rate);
        self.lfo.set_shape(self.v[9] as usize);
        for i in 0..l.len() {
            self.lfo.tick();
            self.env.run(0.5 * (l[i].abs() + r[i].abs()) * self.sens);
            if self.ctrl == 0 {
                self.ctrl = CTRL;
                self.retune();
            } else {
                self.pos.glide();
            }
            self.ctrl -= 1;
            for k in 0..3 {
                self.bank[k].advance();
                self.gains[k] += self.steps[k];
            }
            let mix = self.mix.glide();
            for (ch, s) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let x = *s;
                let mut y = 0.0;
                for k in 0..3 {
                    let (_, band) = self.bank[k].run(ch, x);
                    y += self.gains[k] * band;
                }
                *s = lerp(x, y, mix);
            }
        }
    }
    fn setup(&mut self, rate: u32) {
        self.rate = rate;
        self.pos.set_time(rate, 6.0);
        self.mix.set_time(rate, 20.0);
        let retunes = (rate / CTRL as u32).max(1);
        self.vowel.set_time(retunes, GLIDE_MS);
        self.shift.set_time(retunes, GLIDE_MS);
        self.sharp.set_time(retunes, GLIDE_MS);
        self.env.set(self.v[5], self.v[6], rate);
        self.sens = fast_db_to_gain(self.v[4] / 100.0 * SENS_DB);
    }
    fn clear(&mut self) {
        self.lfo.reset();
        self.env.reset();
        for b in &mut self.bank {
            b.reset();
        }
        self.ctrl = 0;
        self.lfo_now = 0.0;
        self.vowel.snap(self.vowel.target);
        self.pos.snap(self.target());
        self.shift.snap(self.shift.target);
        self.sharp.snap(self.sharp.target);
        self.mix.snap(self.mix.target);
        // The filters and gains start where the first retune puts them.
        for (k, (hz, q, g)) in self.tuning().into_iter().enumerate() {
            self.bank[k].tune(hz, q, self.rate);
            self.gains[k] = g;
            self.steps[k] = 0.0;
        }
    }
    fn tail(&self) -> u32 {
        self.rate / 20
    }
    fn lat(&self) -> u32 {
        0
    }
}

node_impl!(TalkBox, TALKBOX_INFO, TALKBOX_PARAMS);

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;

    fn sine_level(fx: &mut dyn Node, hz: f32, amp: f32) -> f32 {
        fx.prepare(48_000, 128);
        fx.reset();
        let n = 48_000;
        let mut l: Vec<f32> = (0..n)
            .map(|i| amp * (i as f32 * hz / 48_000.0 * std::f32::consts::TAU).sin())
            .collect();
        let mut r = l.clone();
        let ctx = Ctx::new(48_000, 0);
        for k in (0..n).step_by(128) {
            fx.process(&ctx, &[], &mut l[k..k + 128], &mut r[k..k + 128]);
        }
        let tail = &l[n / 2..];
        (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt() / (amp / 2f32.sqrt())
    }

    #[test]
    fn the_pedal_puts_the_peak_where_it_points() {
        // Pedal down: the low end passes a low tone and cuts a high one;
        // pedal up the other way round.
        let mut w = AutoWah::new();
        w.set_param(autowah::SOURCE, 2.0);
        w.set_param(autowah::FILTER, 1.0);
        w.set_param(autowah::CENTER, 0.0);
        let low = sine_level(&mut w, 300.0, 0.1);
        let high = sine_level(&mut w, 2500.0, 0.1);
        assert!(low > 0.7 && high < 0.3, "down: {low} {high}");
        w.set_param(autowah::CENTER, 100.0);
        let low = sine_level(&mut w, 300.0, 0.1);
        let high = sine_level(&mut w, 2500.0, 0.1);
        assert!(high > 0.7 && low < 0.3, "up: {low} {high}");
        // The Wah shape keeps some of the low end, pedal up.
        w.set_param(autowah::FILTER, 0.0);
        let low = sine_level(&mut w, 110.0, 0.1);
        assert!(low > 0.25 && low < 0.5, "wah leak: {low}");
    }

    #[test]
    fn a_louder_sound_opens_the_wah_further() {
        // On the envelope, a 2 kHz tone passes more when it is loud.
        let mut w = AutoWah::new();
        let quiet = sine_level(&mut w, 2000.0, 0.003);
        let loud = sine_level(&mut w, 2000.0, 0.5);
        assert!(loud > 2.0 * quiet, "{quiet} {loud}");
    }

    /// The output's RMS against the input's on white noise, after half a
    /// second.
    fn noise_level(fx: &mut dyn Node) -> f32 {
        fx.prepare(48_000, 128);
        fx.reset();
        let n = 48_000;
        let mut seed = 0x9e37_79b9u32;
        let input: Vec<f32> = (0..n)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                0.1 * (seed as f32 / u32::MAX as f32 * 2.0 - 1.0)
            })
            .collect();
        let (mut l, mut r) = (input.clone(), input.clone());
        let ctx = Ctx::new(48_000, 0);
        for k in (0..n).step_by(128) {
            fx.process(&ctx, &[], &mut l[k..k + 128], &mut r[k..k + 128]);
        }
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        rms(&l[n / 2..]) / rms(&input[n / 2..])
    }

    #[test]
    fn a_sharper_band_pass_keeps_its_level() {
        // FX quality W17: on noise, the band-pass at Resonance 0 and 100
        // is within 3 dB of its level at 60.
        let level = |res: f32| {
            let mut w = AutoWah::new();
            w.set_param(autowah::SOURCE, 2.0);
            w.set_param(autowah::CENTER, 50.0);
            w.set_param(autowah::FILTER, 1.0);
            w.set_param(autowah::RESONANCE, res);
            20.0 * noise_level(&mut w).log10()
        };
        let mid = level(60.0);
        for res in [0.0, 100.0] {
            let db = level(res) - mid;
            assert!(db.abs() < 3.0, "Resonance {res}: {db} dB");
        }
    }

    /// The largest frame-to-frame change of the output on a 997 Hz tone,
    /// steady (before and well after) and around a change of `id` to `to`
    /// half a second in.
    fn steps_around(w: &mut AutoWah, id: u16, to: f32) -> (f32, f32) {
        use mpfree_node::{NodeEvent, Timed};
        w.prepare(48_000, 128);
        w.reset();
        let n = 48_000;
        let tone = |i: usize| 0.25 * (i as f32 * 997.0 / 48_000.0 * std::f32::consts::TAU).sin();
        let mut l: Vec<f32> = (0..n).map(tone).collect();
        let mut r = l.clone();
        let ctx = Ctx::new(48_000, 0);
        let at = n / 2;
        for k in (0..n).step_by(128) {
            let ev = if (k..k + 128).contains(&at) {
                vec![Timed {
                    offset: (at - k) as u32,
                    ev: NodeEvent::Param { id, value: to },
                }]
            } else {
                Vec::new()
            };
            w.process(&ctx, &ev, &mut l[k..k + 128], &mut r[k..k + 128]);
        }
        let step = |a: usize, b: usize| {
            l[a..b]
                .windows(2)
                .map(|p| (p[1] - p[0]).abs())
                .fold(0.0f32, f32::max)
        };
        let steady = step(at - 4_800, at).max(step(at + 9_600, n));
        (steady, step(at, at + 9_600))
    }

    #[test]
    fn a_filter_change_glides() {
        // FX quality W18: from the Wah shape to each other, the output
        // moves no faster than the tone does on its own either side.
        for to in [1.0, 2.0, 3.0] {
            let (steady, around) = steps_around(&mut AutoWah::new(), autowah::FILTER, to);
            assert!(
                around < 1.25 * steady,
                "Filter to {to}: {around} against {steady}"
            );
        }
    }

    #[test]
    fn a_source_change_glides_the_sweep() {
        // FX quality W18: from the envelope to the pedal at its lowest,
        // the sweep takes at least 10 ms to get half way (the position's
        // own glide alone would take 3) and is there within 60 ms.
        let mut w = AutoWah::new();
        w.prepare(48_000, 128);
        w.reset();
        let ctx = Ctx::new(48_000, 0);
        let tone = |i: usize| 0.25 * (i as f32 * 997.0 / 48_000.0 * std::f32::consts::TAU).sin();
        let (mut l, mut r) = ([0.0f32; 1], [0.0f32; 1]);
        for i in 0..24_000 {
            (l[0], r[0]) = (tone(i), tone(i));
            w.process(&ctx, &[], &mut l, &mut r);
        }
        let from = w.pos.value;
        assert!(from > 0.2, "{from}");
        w.set_param(autowah::SOURCE, 2.0);
        let mut half = None;
        for i in 0..2_880 {
            (l[0], r[0]) = (tone(i), tone(i));
            w.process(&ctx, &[], &mut l, &mut r);
            if half.is_none() && w.pos.value < 0.5 * from {
                half = Some(i);
            }
        }
        let half = half.expect("never got half way");
        assert!(half >= 480, "half way in {half} frames");
        assert!(w.pos.value < 0.01 * from, "{} after 60 ms", w.pos.value);
    }

    #[test]
    fn vowels_have_their_formants() {
        // "AH" passes 730 Hz (its first formant) far better than "EE"
        // does, and "EE" passes 2.29 kHz (its second) better than "AH".
        let mut t = TalkBox::new();
        t.set_param(talkbox::VOWEL, 3.0);
        let ah_low = sine_level(&mut t, 730.0, 0.1);
        let ah_high = sine_level(&mut t, 2290.0, 0.1);
        t.set_param(talkbox::VOWEL, 8.0);
        let ee_low = sine_level(&mut t, 730.0, 0.1);
        let ee_high = sine_level(&mut t, 2290.0, 0.1);
        assert!(ah_low > 4.0 * ee_low, "{ah_low} {ee_low}");
        assert!(ee_high > 2.0 * ah_high, "{ee_high} {ah_high}");
    }

    #[test]
    fn the_formant_shift_moves_the_vowel_up_and_keeps_its_q() {
        let at = |formant: f32| {
            let mut t = TalkBox::new();
            t.set_param(talkbox::FORMANT, formant);
            t.reset();
            t.tuning()
        };
        let (a, b) = (at(0.0), at(12.0));
        for k in 0..3 {
            assert!((b[k].0 / a[k].0 - 2.0).abs() < 1e-3, "{k}: {a:?} {b:?}");
            assert!((b[k].1 - a[k].1).abs() < 1e-4, "{k}: {a:?} {b:?}");
        }
        // Halfway between two vowels is between them in pitch.
        let (a, _) = vowel_at(0.0);
        let (m, _) = vowel_at(0.5);
        let (b, _) = vowel_at(1.0);
        assert!(m[1] > a[1] && m[1] < b[1]);
    }

    #[test]
    fn no_hollow_notch_between_the_first_two_formants() {
        // FX quality V2: "AE" (660 and 1720 Hz) dips 31 dB under F2 with
        // same-sign band-passes; a mouth (and the alternating bank) dips
        // about 15.
        let mut t = TalkBox::new();
        t.set_param(talkbox::VOWEL, 5.0);
        let f2 = sine_level(&mut t, 1720.0, 0.1);
        let low = (0..12)
            .map(|i| 700.0 * (1650.0f32 / 700.0).powf(i as f32 / 11.0))
            .map(|hz| sine_level(&mut t, hz, 0.1))
            .fold(f32::INFINITY, f32::min);
        let dip = 20.0 * (low / f2).log10();
        assert!(dip > -20.0, "dip {dip} dB");
    }

    #[test]
    fn the_vowels_and_resonance_keep_the_level() {
        // FX quality V4 and W17's rule: on white noise, every vowel within
        // 6 dB of the defaults' level (white noise favours the front
        // vowels' higher formants; fxlab reads V4 on pink, K-weighted) and
        // Resonance 0 and 100 within 3 dB.
        let level = |id: u16, v: f32| {
            let mut t = TalkBox::new();
            t.set_param(id, v);
            20.0 * noise_level(&mut t).log10()
        };
        let mid = level(talkbox::VOWEL, 3.0);
        for v in 0..9 {
            let db = level(talkbox::VOWEL, v as f32) - mid;
            assert!(db.abs() < 6.0, "vowel {v}: {db} dB");
        }
        for res in [0.0, 100.0] {
            let db = level(talkbox::RESONANCE, res) - mid;
            assert!(db.abs() < 3.0, "Resonance {res}: {db} dB");
        }
    }
}
