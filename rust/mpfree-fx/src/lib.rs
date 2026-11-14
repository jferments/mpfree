//! MPFree's effects, built on the shared [`mpfree_node::Node`] interface.
//!
//! - [`MultiFilter`]: zero-delay-feedback state-variable filter (low-pass,
//!   high-pass, band-pass, notch, bell, shelves; 12 or 24 dB) with drive.
//! - [`DjFilter`]: one knob, low-pass to the left, high-pass to the right,
//!   off in the middle.
//! - [`Drive`]: soft, hard, tube, foldback, fuzz and rectify shapers inside
//!   2x or 4x oversampling, with tone, bias, mix and auto-gain.
//! - [`Comb`]: a tuned feedback comb with damping, stereo spread and key
//!   follow (notes play its pitch).
//! - [`Crush`]: bit depth and sample-rate reduction (aliasing on purpose).
//! - [`Delay`]: free or tempo-synced echo with ping-pong, a filter and
//!   saturation in the feedback, tape-style time changes and freeze.
//! - [`Eq`]: four parametric bands (bell or shelf) with low and high cuts,
//!   and the exact response curve for drawing.
//! - [`Chorus`], [`Flanger`], [`Phaser`]: modulation effects with a free
//!   or tempo-synced LFO and stereo width.
//! - [`LoFi`]: tape wow, flutter and saturation, vinyl crackle and hiss
//!   that ride the music, tone and width, all under one Amount.
//! - [`Compressor`]: soft-knee, log-domain compressor with parallel mix and
//!   a sidechain input.
//! - [`Ducker`]: the pump: a volume envelope (attack, hold, shaped
//!   release) triggered by a sidechain key or by the tempo.
//! - [`TransientShaper`]: more or less attack and sustain on each hit, with
//!   no threshold.
//! - [`PitchShifter`]: two octaves down to two up without changing speed,
//!   with fine tune, stereo spread and feedback.
//! - [`Cloud`]: a lush algorithmic reverb ported from Cloud Seed, with
//!   five characters (plate, room, hall, cloud, echoes) and freeze.
//! - [`MasterSafety`]: the last stage of every output: DC block, trim, a
//!   look-ahead peak limiter that counts overs, and Soft clip and Raw modes.
//! - [`Perform`]: sixteen pads of performance effects (stutter, beat
//!   repeat, reverse, half speed, tape stop, vinyl brake, pitch drop,
//!   trance gate, filter sweeps, crush, echo throws, a noise riser) that
//!   start on the next grid line and snap back when let go, with a DJ
//!   filter and an echo knob for the XY pad and the DJ knob.
//! - [`XyPad`]: the XY pad's logic (two parameters, snap back or hold).
//! - [`Chain`]: serial inserts with click-free bypass, tails that keep
//!   ringing after bypass, latency-matched dry paths and a NaN guard.
//!
//! [`registry`] lists every effect in one table (id, name, a line about
//! it, its [`Category`], a factory and its presets): the one place a new
//! effect joins.
//!
//! # Clipping
//!
//! Deliberate clipping is a feature; clipping nobody asked for is not.
//! Nothing here clamps a signal unless that is the effect's job (drive,
//! crush, the delay's feedback saturation, the Master safety stage), and no
//! effect ever fails, stops or mutes because a level is high. Every output
//! is meant to end in [`MasterSafety`], which keeps the converter from
//! clipping unless the user picks its Raw mode.
//!
//! # Real-time rules
//!
//! Every node follows the [`mpfree_node`] rules: `process` never allocates,
//! locks or panics; memory comes from `prepare`; parameters are smoothed per
//! sample; and the output is the same, sample for sample, however a stream
//! is cut into blocks. The tests in `tests/` check each of these.
#![forbid(unsafe_code)]

mod chain;
mod character;
mod cloud;
mod comb;
mod comp;
mod crush;
mod delay;
mod drive;
mod ducker;
mod dynamics;
mod eq;
mod kit;
mod lofi;
mod master;
mod mixer;
mod modfx;
mod motion;
mod oversample;
mod perform;
mod pitch;
mod presets;
pub mod print;
pub mod registry;
mod svf;
mod transient;
mod xy;

pub use chain::{Chain, ChainStats, Insert, MAX_SLOT_EVENTS};
pub use character::Character;
pub use cloud::{Cloud, PRESETS as CLOUD_PRESETS};
pub use comb::Comb;
pub use comp::Compressor;
pub use crush::Crush;
pub use delay::Delay;
pub use drive::Drive;
pub use ducker::{
    Ducker, RATES as DUCKER_RATES, RATE_BEATS as DUCKER_RATE_BEATS, SOURCES as DUCKER_SOURCES,
};
pub use dynamics::{DeEsser, Gate, Limiter, MultibandCompressor};
pub use eq::Eq;
pub use lofi::{LoFi, PRESETS as LOFI_PRESETS};
pub use master::{MasterSafety, MasterStats};
pub use mixer::{
    param_applies as mixer_param_applies, strip_params, CapturePoint, Edit, FreezeState, Mixer,
    MixerConfig, MixerStats, StripId, BUSES, CAPTURES, FREEZE_FADE_MS, MAX_INPUTS, MAX_PADS,
    MAX_SLOTS, MAX_TRACKS, RETURNS, SILENT_DB,
};
pub use modfx::{Chorus, Flanger, Phaser};
pub use motion::{AutoPan, RingMod, Rotary, Tremolo};
pub use oversample::{Down2, IirDown2, IirPhase, IirUp2, Up2};
pub use perform::{
    state as perform_state, Perform, AMOUNT_MEANS as PERFORM_AMOUNTS, EFFECTS as PERFORM_EFFECTS,
    LENGTHS as PERFORM_LENGTHS, LENGTH_BEATS as PERFORM_LENGTH_BEATS, MAX_LOOP_SECONDS,
    SLOTS as PERFORM_SLOTS,
};
pub use pitch::PitchShifter;
pub use presets::presets;
pub use print::{wav_f32, Frozen, Print, PrintError, PrintJob, PrintState, Printed, Tail};
pub use registry::{Category, FxEntry, EFFECTS};
pub use svf::{DjFilter, MultiFilter, Svf, SvfMode};
pub use transient::TransientShaper;
pub use xy::{XyAxis, XyPad};

pub use mpfree_lanes;
pub use mpfree_node;

/// Parameter ids, one module per node (`ids::delay::FEEDBACK`). An id is
/// also the parameter's index in the node's `params()` list.
pub mod ids {
    pub use crate::character::ids as character;
    pub use crate::cloud::ids as cloud;
    pub use crate::comb::ids as comb;
    pub use crate::comp::ids as compressor;
    pub use crate::crush::ids as crush;
    pub use crate::delay::ids as delay;
    pub use crate::drive::ids as drive;
    pub use crate::ducker::ids as ducker;
    pub use crate::dynamics::deesser;
    pub use crate::dynamics::gate;
    pub use crate::dynamics::limiter;
    pub use crate::dynamics::multiband;
    pub use crate::eq::ids as eq;
    pub use crate::lofi::ids as lofi;
    pub use crate::master::ids as master;
    pub use crate::mixer::ids as mixer;
    pub use crate::modfx::chorus;
    pub use crate::modfx::flanger;
    pub use crate::modfx::phaser;
    pub use crate::motion::autopan;
    pub use crate::motion::ringmod;
    pub use crate::motion::rotary;
    pub use crate::motion::tremolo;
    pub use crate::perform::ids as perform;
    pub use crate::pitch::ids as pitch;
    pub use crate::svf::dj as dj_filter;
    pub use crate::svf::multi as filter;
    pub use crate::transient::ids as transient;
}

/// A factory preset: a name and the parameter values that differ from the
/// node's defaults (the same shape as `mpfree_synth::Preset`).
#[derive(Clone, Copy, Debug)]
pub struct Preset {
    pub name: &'static str,
    pub values: &'static [(u16, f32)],
}

/// A crossfade from `a` (at 0) to `b` (at 1) that is exact at both ends,
/// so a mix of 0 or 100 % passes the dry or wet signal bit for bit.
#[inline]
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// Decibels to a linear gain.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db * 0.05)
}

/// A linear gain to decibels (-180 dB for silence).
#[inline]
pub fn gain_to_db(g: f32) -> f32 {
    if g <= 1e-9 {
        -180.0
    } else {
        20.0 * g.log10()
    }
}

/// Flushes values too small to hear to zero, so feedback paths never sit in
/// slow denormal arithmetic on ARM.
#[inline]
pub(crate) fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 {
        0.0
    } else {
        x
    }
}

/// A cheap, smooth saturator: about `x` for small `x`, exactly 1 in size
/// from |x| = 3 up, with no corner.
#[inline]
pub(crate) fn soft(x: f32) -> f32 {
    if x >= 3.0 {
        1.0
    } else if x <= -3.0 {
        -1.0
    } else {
        x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
    }
}

/// `tanh(x)` to within about 2e-7, without the library call: a rational
/// fit (odd degree 13 over even degree 6, Eigen's single-precision
/// `ptanh`), never past ±1 and exactly ±1 from |x| = 7.9 up. Several times cheaper than
/// `f32::tanh`, which calls the C library on the MPC's ARM.
#[inline]
pub(crate) fn fast_tanh(x: f32) -> f32 {
    const CLAMP: f32 = 7.905_311;
    let x = x.clamp(-CLAMP, CLAMP);
    let x2 = x * x;
    let p = (((((-2.760_768_5e-16 * x2 + 2.000_188e-13) * x2 - 8.604_672e-11) * x2
        + 5.122_297e-8)
        * x2
        + 1.485_722_4e-5)
        * x2
        + 6.372_619_3e-4)
        * x2
        + 4.893_524_6e-3;
    let q = ((1.198_258_4e-6 * x2 + 1.185_347_1e-4) * x2 + 2.268_434_6e-3) * x2 + 4.893_525e-3;
    (x * p / q).clamp(-1.0, 1.0)
}

/// `log2(x)` for a normal, positive `x`, to within about 3e-6 (plus the
/// rounding of the result), without the library call: the exponent from
/// the bits, and a degree-5 fit for the mantissa taken to [0.71, 1.41).
/// Callers keep `x` in range (zero, denormals, infinities and NaN give
/// garbage). On the MPC's Cortex-A17 `log10f` and `powf` cost hundreds of
/// cycles each; on a desktop the library is as quick as this.
#[inline]
pub(crate) fn fast_log2(x: f32) -> f32 {
    let bits = x.to_bits();
    let mut e = ((bits >> 23) & 0xff) as i32 - 127;
    let mut m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    if m > std::f32::consts::SQRT_2 {
        m *= 0.5;
        e += 1;
    }
    let t = m - 1.0;
    let q = 1.442_715_8
        + t * (-0.721_122_6
            + t * (0.479_324_37 + t * (-0.367_703_97 + t * (0.322_085_5 - t * 0.205_298_29))));
    e as f32 + t * q
}

/// `2^x` to within about 2e-7 of the value, without the library call:
/// Cephes' `exp2f` fit on the fraction, the power of two from the bits.
/// `x` is clamped to [-126, 126]; `fast_exp2(0.0)` is exactly 1.
#[inline]
pub(crate) fn fast_exp2(x: f32) -> f32 {
    let x = x.clamp(-126.0, 126.0);
    // Round to nearest with a truncating cast (no library call on ARMv7).
    let n = (x + 126.5) as i32 - 126;
    let f = x - n as f32;
    let p = 1.0
        + f * (6.931_472e-1
            + f * (2.402_264_8e-1
                + f * (5.550_332_5e-2
                    + f * (9.618_437e-3 + f * (1.339_887_4e-3 + f * 1.535_336_2e-4)))));
    p * f32::from_bits(((n + 127) as u32) << 23)
}

/// [`db_to_gain`] through [`fast_exp2`] (per-sample gains).
#[inline]
pub(crate) fn fast_db_to_gain(db: f32) -> f32 {
    fast_exp2(db * 0.166_096_4)
}

/// A level in dB for a detector, through [`fast_log2`]: -120 dB at and
/// below 1e-6 (and for NaN), 120 dB at and above 1e6.
#[inline]
pub(crate) fn fast_level_db(x: f32) -> f32 {
    if x > 1e-6 {
        6.020_6 * fast_log2(x.min(1e6))
    } else {
        -120.0
    }
}

/// A one-pole DC blocker for one channel.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DcBlock {
    r: f32,
    x1: f32,
    y1: f32,
}

impl DcBlock {
    pub(crate) fn new() -> DcBlock {
        DcBlock {
            r: 0.9995,
            x1: 0.0,
            y1: 0.0,
        }
    }
    /// Sets the corner frequency.
    pub(crate) fn set(&mut self, hz: f32, rate: u32) {
        self.r = (-2.0 * std::f32::consts::PI * hz / rate as f32).exp();
    }
    #[inline]
    pub(crate) fn run(&mut self, x: f32) -> f32 {
        let y = x - self.x1 + self.r * self.y1;
        self.x1 = x;
        self.y1 = flush(y);
        y
    }
    pub(crate) fn reset(&mut self) {
        self.x1 = 0.0;
        self.y1 = 0.0;
    }
}

/// A fixed delay of whole frames for one channel (latency matching).
#[derive(Clone, Debug, Default)]
pub(crate) struct FixedDelay {
    buf: Vec<f32>,
    pos: usize,
}

impl FixedDelay {
    /// Allocates for `frames` of delay (control thread).
    pub(crate) fn new(frames: usize) -> FixedDelay {
        FixedDelay {
            buf: vec![0.0; frames],
            pos: 0,
        }
    }
    #[cfg(test)]
    pub(crate) fn frames(&self) -> usize {
        self.buf.len()
    }
    #[inline]
    pub(crate) fn run(&mut self, x: f32) -> f32 {
        if self.buf.is_empty() {
            return x;
        }
        let y = self.buf[self.pos];
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        y
    }
    pub(crate) fn reset(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }
}

/// A small, fast, deterministic random source (dither, noise).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng(u32);

impl Rng {
    pub(crate) fn new(seed: u32) -> Rng {
        Rng(if seed == 0 { 0x9E37_79B9 } else { seed })
    }
    /// Uniform in [-1, 1).
    #[inline]
    pub(crate) fn bipolar(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// Parameter bookkeeping shared by the nodes: values held by index, where a
/// parameter's id is its index in the node's list.
pub(crate) fn defaults<const N: usize>(params: &[mpfree_node::ParamInfo; N]) -> [f32; N] {
    let mut v = [0.0; N];
    for (i, p) in params.iter().enumerate() {
        debug_assert_eq!(p.id as usize, i, "param ids are their index");
        v[i] = p.default;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soft_is_smooth_and_bounded() {
        assert_eq!(soft(0.0), 0.0);
        assert!((soft(0.1) - 0.1).abs() < 0.001);
        assert_eq!(soft(3.0), 1.0);
        assert_eq!(soft(-50.0), -1.0);
        assert!((soft(2.999) - 1.0).abs() < 1e-4);
        let mut prev = -1.0;
        for i in -300..=300 {
            let y = soft(i as f32 / 100.0);
            assert!(y >= prev - 1e-6, "monotonic");
            prev = y;
        }
    }

    #[test]
    fn fast_tanh_matches_tanh() {
        let mut worst = 0.0f32;
        for i in -12_000..=12_000 {
            let x = i as f32 / 1000.0;
            worst = worst.max((fast_tanh(x) - x.tanh()).abs());
        }
        assert!(worst < 1e-6, "{worst}");
        assert_eq!(fast_tanh(0.0), 0.0);
        assert_eq!((fast_tanh(50.0), fast_tanh(-50.0)), (1.0, -1.0));
        assert!(fast_tanh(f32::NAN).is_nan());
        let mut prev = -1.0;
        for i in -8000..=8000 {
            let y = fast_tanh(i as f32 / 1000.0);
            assert!(y >= prev - 1e-6, "monotonic (to its accuracy) at {i}");
            assert!(y.abs() <= 1.0);
            prev = y;
        }
    }

    #[test]
    fn fast_log2_and_exp2_match_the_library() {
        let mut worst_log = 0.0f64;
        let mut worst_exp = 0.0f64;
        let mut x = 1e-30f32;
        while x < 1e30 {
            // The fit's error, past the rounding of a result as large as 126.
            let want = (x as f64).log2();
            worst_log = worst_log.max((fast_log2(x) as f64 - want).abs() - want.abs() * 1.2e-7);
            x *= 1.000_37;
        }
        for i in -126_000..=126_000 {
            let y = i as f32 / 1000.0;
            let want = (y as f64).exp2();
            worst_exp = worst_exp.max(((fast_exp2(y) as f64 - want) / want).abs());
        }
        assert!(worst_log < 4e-6, "log2 {worst_log}");
        assert!(worst_exp < 4e-7, "exp2 {worst_exp}");
        assert_eq!(fast_exp2(0.0), 1.0);
        assert_eq!(fast_db_to_gain(0.0), 1.0);
        assert_eq!(
            (fast_exp2(1.0), fast_exp2(-1.0), fast_exp2(10.0)),
            (2.0, 0.5, 1024.0)
        );
        assert!(fast_exp2(f32::NAN).is_nan());
        // Monotonic, so a detector or an envelope never steps backwards.
        let mut prev = f32::NEG_INFINITY;
        for i in -20_000..=20_000 {
            let y = fast_exp2(i as f32 / 997.0);
            assert!(y >= prev, "exp2 at {i}");
            prev = y;
        }
        // The detector's dB: the library's to 1e-4 dB, clamped at both ends.
        for db in [-119.0f32, -60.0, -6.02, 0.0, 0.5, 12.0, 119.0] {
            let g = db_to_gain(db);
            assert!((fast_level_db(g) - gain_to_db(g)).abs() < 1e-4, "{db}");
            assert!((fast_db_to_gain(db) / g - 1.0).abs() < 1e-6, "{db}");
        }
        assert_eq!(
            (
                fast_level_db(0.0),
                fast_level_db(1e-7),
                fast_level_db(f32::NAN)
            ),
            (-120.0, -120.0, -120.0)
        );
        assert!((fast_level_db(f32::INFINITY) - 120.0).abs() < 1e-3);
    }

    #[test]
    fn db_round_trip() {
        for db in [-60.0f32, -6.0, 0.0, 12.0] {
            assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-3);
        }
    }

    #[test]
    fn fixed_delay_delays() {
        let mut d = FixedDelay::new(3);
        let out: Vec<f32> = [1.0, 2.0, 3.0, 4.0, 5.0]
            .iter()
            .map(|&x| d.run(x))
            .collect();
        assert_eq!(out, [0.0, 0.0, 0.0, 1.0, 2.0]);
        let mut z = FixedDelay::new(0);
        assert_eq!(z.run(7.0), 7.0);
    }
}
