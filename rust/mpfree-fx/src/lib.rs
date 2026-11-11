//! MPFree's effects, built on the shared [`mpfree_node::Node`] interface.
//!
//! - [`MultiFilter`]: zero-delay-feedback state-variable filter (low-pass,
//!   high-pass, band-pass, notch, bell, shelves; 12 or 24 dB) with drive.
//! - [`DjFilter`]: one knob, low-pass to the left, high-pass to the right,
//!   off in the middle.
//! - [`Drive`]: soft, hard, tube, foldback, fuzz and rectify shapers inside
//!   2x or 4x oversampling, with tone, bias, mix and auto-gain.
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
//! - [`Cloud`]: a lush algorithmic reverb ported from Cloud Seed, with
//!   five characters (plate, room, hall, cloud, echoes) and freeze.
//! - [`MasterSafety`]: the last stage of every output: DC block, trim, a
//!   look-ahead peak limiter that counts overs, and Soft clip and Raw modes.
//! - [`Chain`]: serial inserts with click-free bypass, tails that keep
//!   ringing after bypass, latency-matched dry paths and a NaN guard.
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
mod cloud;
mod comp;
mod crush;
mod delay;
mod drive;
mod eq;
mod lofi;
mod master;
mod mixer;
mod modfx;
mod oversample;
mod svf;

pub use chain::{Chain, ChainStats, Insert, MAX_SLOT_EVENTS};
pub use cloud::{Cloud, PRESETS as CLOUD_PRESETS};
pub use comp::Compressor;
pub use crush::Crush;
pub use delay::Delay;
pub use drive::Drive;
pub use eq::Eq;
pub use lofi::{LoFi, PRESETS as LOFI_PRESETS};
pub use master::{MasterSafety, MasterStats};
pub use mixer::{param_applies as mixer_param_applies, strip_params, CapturePoint, Edit, Mixer, MixerConfig, MixerStats, StripId, BUSES, CAPTURES, MAX_INPUTS, MAX_PADS, MAX_SLOTS, MAX_TRACKS, RETURNS, SILENT_DB};
pub use modfx::{Chorus, Flanger, Phaser};
pub use oversample::{Down2, Up2};
pub use svf::{DjFilter, MultiFilter, Svf, SvfMode};

pub use mpfree_node;

/// Parameter ids, one module per node (`ids::delay::FEEDBACK`). An id is
/// also the parameter's index in the node's `params()` list.
pub mod ids {
    pub use crate::cloud::ids as cloud;
    pub use crate::comp::ids as compressor;
    pub use crate::crush::ids as crush;
    pub use crate::delay::ids as delay;
    pub use crate::drive::ids as drive;
    pub use crate::eq::ids as eq;
    pub use crate::lofi::ids as lofi;
    pub use crate::master::ids as master;
    pub use crate::mixer::ids as mixer;
    pub use crate::modfx::chorus;
    pub use crate::modfx::flanger;
    pub use crate::modfx::phaser;
    pub use crate::svf::dj as dj_filter;
    pub use crate::svf::multi as filter;
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
pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 { a * (1.0 - t) + b * t }

/// Decibels to a linear gain.
#[inline]
pub fn db_to_gain(db: f32) -> f32 { 10f32.powf(db * 0.05) }

/// A linear gain to decibels (-180 dB for silence).
#[inline]
pub fn gain_to_db(g: f32) -> f32 { if g <= 1e-9 { -180.0 } else { 20.0 * g.log10() } }

/// Flushes values too small to hear to zero, so feedback paths never sit in
/// slow denormal arithmetic on ARM.
#[inline]
pub(crate) fn flush(x: f32) -> f32 { if x.abs() < 1e-20 { 0.0 } else { x } }

/// A cheap, smooth saturator: about `x` for small `x`, exactly 1 in size
/// from |x| = 3 up, with no corner.
#[inline]
pub(crate) fn soft(x: f32) -> f32 {
    if x >= 3.0 { 1.0 } else if x <= -3.0 { -1.0 } else { x * (27.0 + x * x) / (27.0 + 9.0 * x * x) }
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
    let p = (((((-2.760_768_5e-16 * x2 + 2.000_188e-13) * x2 - 8.604_672e-11) * x2 + 5.122_297e-8) * x2
        + 1.485_722_4e-5) * x2 + 6.372_619_3e-4) * x2 + 4.893_524_6e-3;
    let q = ((1.198_258_4e-6 * x2 + 1.185_347_1e-4) * x2 + 2.268_434_6e-3) * x2 + 4.893_525e-3;
    (x * p / q).clamp(-1.0, 1.0)
}

/// A one-pole DC blocker for one channel.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DcBlock { r: f32, x1: f32, y1: f32 }

impl DcBlock {
    pub(crate) fn new() -> DcBlock { DcBlock { r: 0.9995, x1: 0.0, y1: 0.0 } }
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
    pub(crate) fn reset(&mut self) { self.x1 = 0.0; self.y1 = 0.0; }
}

/// A fixed delay of whole frames for one channel (latency matching).
#[derive(Clone, Debug, Default)]
pub(crate) struct FixedDelay { buf: Vec<f32>, pos: usize }

impl FixedDelay {
    /// Allocates for `frames` of delay (control thread).
    pub(crate) fn new(frames: usize) -> FixedDelay { FixedDelay { buf: vec![0.0; frames], pos: 0 } }
    #[cfg(test)]
    pub(crate) fn frames(&self) -> usize { self.buf.len() }
    #[inline]
    pub(crate) fn run(&mut self, x: f32) -> f32 {
        if self.buf.is_empty() { return x; }
        let y = self.buf[self.pos];
        self.buf[self.pos] = x;
        self.pos += 1;
        if self.pos == self.buf.len() { self.pos = 0; }
        y
    }
    pub(crate) fn reset(&mut self) { self.buf.fill(0.0); self.pos = 0; }
}

/// A small, fast, deterministic random source (dither, noise).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng(u32);

impl Rng {
    pub(crate) fn new(seed: u32) -> Rng { Rng(if seed == 0 { 0x9E37_79B9 } else { seed }) }
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
    fn db_round_trip() {
        for db in [-60.0f32, -6.0, 0.0, 12.0] {
            assert!((gain_to_db(db_to_gain(db)) - db).abs() < 1e-3);
        }
    }

    #[test]
    fn fixed_delay_delays() {
        let mut d = FixedDelay::new(3);
        let out: Vec<f32> = [1.0, 2.0, 3.0, 4.0, 5.0].iter().map(|&x| d.run(x)).collect();
        assert_eq!(out, [0.0, 0.0, 0.0, 1.0, 2.0]);
        let mut z = FixedDelay::new(0);
        assert_eq!(z.run(7.0), 7.0);
    }
}
