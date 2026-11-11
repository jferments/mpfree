//! Pad pressure (polyphonic aftertouch), shaped before it reaches a sound.
//!
//! Every engine that responds to [`crate::NodeEvent::Pressure`] runs it
//! through the same small shaper, so the pads feel alike whether they play
//! a synth or a sampler pad. Players' common complaints about poly
//! aftertouch are about feel: pressure that fires by accident, jumps when a
//! note starts or ends, moves in coarse steps, or can't be tuned. The stages
//! answer those, in order:
//!
//! 1. **Strike hold-off.** For the first few milliseconds after a note-on
//!    the value holds still, so the hit itself doesn't modulate. A fresh
//!    voice holds at 0 and then glides in at the rise time.
//! 2. **Threshold.** Pressure below it gives nothing, so a resting finger
//!    does nothing. The scale starts again from 0 at the threshold: no step.
//! 3. **Full at.** The raw pressure that gives the full effect, so it is
//!    reached without pressing as hard as possible.
//! 4. **Curve.** Soft, linear, firm or hard (powers 0.6, 1, 1.6, 2.5).
//! 5. **Smoothing.** Separate rise and fall times, which remove the
//!    128-step staircase.
//! 6. **Release.** What a voice does after its note-off: Hold keeps the
//!    value it has (the fall time means a lifting finger has barely lowered
//!    it yet), Follow keeps following the pad, Fade falls to 0.
//! 7. **Mode** (the engine applies it): per note, the highest among the held
//!    notes for every voice ([`PressureChord`]), or off.
//!
//! The shaper runs once per voice per control step. It allocates nothing,
//! and works in control steps, so it gives the same output for any block
//! size. Each engine lays the settings out as 16 parameters from a base id
//! ([`params`]): the 8 settings, then 4 destination slots of a target and a
//! signed depth. What the targets are and how far a depth reaches is the
//! engine's business.
//!
//! Per-pad calibration is the app's: it scales each pad's raw value before
//! the event reaches the node.

use crate::{Curve, ParamInfo, Unit};

/// Parameter offsets from an engine's pressure base id.
pub mod field {
    pub const MODE: u16 = 0;
    pub const HOLDOFF: u16 = 1;
    pub const THRESHOLD: u16 = 2;
    pub const FULL_AT: u16 = 3;
    pub const CURVE: u16 = 4;
    pub const RISE: u16 = 5;
    pub const FALL: u16 = 6;
    pub const RELEASE: u16 = 7;
    /// Slot `n` (0 to 3): its target.
    pub const fn target(n: usize) -> u16 { 8 + 2 * n as u16 }
    /// Slot `n` (0 to 3): its depth, -100 to 100 %.
    pub const fn depth(n: usize) -> u16 { 9 + 2 * n as u16 }
}

/// Parameters in a pressure block.
pub const FIELDS: usize = 16;
/// Destination slots per engine.
pub const SLOTS: usize = 4;

pub const MODES: &[&str] = &["Per note", "Highest", "Off"];
pub const CURVES: &[&str] = &["Soft", "Linear", "Firm", "Hard"];
pub const RELEASES: &[&str] = &["Hold", "Follow", "Fade"];
const POWERS: [f32; 4] = [0.6, 1.0, 1.6, 2.5];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PressureMode {
    /// Each voice follows its own pad.
    #[default]
    PerNote,
    /// Every voice follows the highest pressure among the held notes, so a
    /// chord swells together.
    Highest,
    Off,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PressureCurve {
    Soft,
    Linear,
    #[default]
    Firm,
    Hard,
}

impl PressureCurve {
    pub fn power(self) -> f32 { POWERS[self as usize] }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PressureRelease {
    /// Keep the value the voice has at its note-off.
    #[default]
    Hold,
    /// Keep following the pad.
    Follow,
    /// Fall to 0 at the fall time.
    Fade,
}

/// The shaper's settings, in the units the player sees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressureShape {
    pub mode: PressureMode,
    pub holdoff_ms: f32,
    /// 0 to 1 of the raw range.
    pub threshold: f32,
    /// 0 to 1 of the raw range: the raw pressure that gives the full effect.
    pub full_at: f32,
    pub curve: PressureCurve,
    pub rise_ms: f32,
    pub fall_ms: f32,
    pub release: PressureRelease,
}

impl Default for PressureShape {
    /// The starting defaults from the design (to be tuned once the MPC One's
    /// pads are measured): 30 ms hold-off, threshold 8 %, full at 85 %, a
    /// firm curve, 8 ms rise, 40 ms fall, release Hold.
    fn default() -> Self {
        PressureShape {
            mode: PressureMode::PerNote,
            holdoff_ms: 30.0,
            threshold: 0.08,
            full_at: 0.85,
            curve: PressureCurve::Firm,
            rise_ms: 8.0,
            fall_ms: 40.0,
            release: PressureRelease::Hold,
        }
    }
}

fn pick<T: Copy>(all: &[T], v: f32) -> T {
    let i = if v.is_finite() { v.round().clamp(0.0, (all.len() - 1) as f32) as usize } else { 0 };
    all[i]
}

impl PressureShape {
    /// Reads the settings from an engine's values, `get(id)`, laid out from
    /// `base` as [`params`] lays them.
    pub fn from_params(base: u16, get: impl Fn(u16) -> f32) -> PressureShape {
        PressureShape {
            mode: pick(&[PressureMode::PerNote, PressureMode::Highest, PressureMode::Off], get(base + field::MODE)),
            holdoff_ms: get(base + field::HOLDOFF).max(0.0),
            threshold: (get(base + field::THRESHOLD) / 100.0).clamp(0.0, 0.9),
            full_at: (get(base + field::FULL_AT) / 100.0).clamp(0.0, 1.0),
            curve: pick(&[PressureCurve::Soft, PressureCurve::Linear, PressureCurve::Firm, PressureCurve::Hard], get(base + field::CURVE)),
            rise_ms: get(base + field::RISE).max(0.0),
            fall_ms: get(base + field::FALL).max(0.0),
            release: pick(&[PressureRelease::Hold, PressureRelease::Follow, PressureRelease::Fade], get(base + field::RELEASE)),
        }
    }

    /// The shaped value, 0 to 1, for a raw value 0 to 1: threshold, full at
    /// and curve (no hold-off or smoothing).
    pub fn map(&self, raw: f32) -> f32 {
        let thr = self.threshold;
        // Full at sits at least one raw step above the threshold.
        let full = self.full_at.max(thr + 1.0 / 127.0);
        let x = ((raw - thr) / (full - thr)).clamp(0.0, 1.0);
        if x <= 0.0 { 0.0 } else { x.powf(self.curve.power()) }
    }

    /// The numbers the shaper runs on, for a control step rate in Hz (the
    /// sample rate over the engine's control step).
    pub fn coefs(&self, step_rate: f32) -> PressureCoefs {
        let mut table = [0.0; 128];
        for (i, t) in table.iter_mut().enumerate() {
            *t = self.map(i as f32 / 127.0);
        }
        PressureCoefs {
            table,
            holdoff_steps: (self.holdoff_ms * 0.001 * step_rate).round().max(0.0) as u32,
            rise: one_pole(self.rise_ms, step_rate),
            fall: one_pole(self.fall_ms, step_rate),
            release: self.release,
            mode: self.mode,
        }
    }
}

/// A one-pole coefficient: about `ms` to cover 63 % of a jump, at
/// `step_rate` steps a second (1.0, instant, for 0 ms).
fn one_pole(ms: f32, step_rate: f32) -> f32 {
    let n = ms * 0.001 * step_rate;
    if n <= 0.0 { 1.0 } else { 1.0 - (-1.0 / n).exp() }
}

/// [`PressureShape`] worked out for a control rate: the shaped value of each
/// raw value, the hold-off in steps, and the smoothing coefficients. Worked
/// out on a parameter change, read every step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PressureCoefs {
    pub table: [f32; 128],
    pub holdoff_steps: u32,
    pub rise: f32,
    pub fall: f32,
    pub release: PressureRelease,
    pub mode: PressureMode,
}

impl Default for PressureCoefs {
    fn default() -> Self { PressureShape::default().coefs(3_000.0) }
}

/// Moves `value` one step toward `target` (none: hold still).
#[inline]
fn glide(value: &mut f32, target: Option<f32>, c: &PressureCoefs) -> f32 {
    if let Some(t) = target {
        let k = if t > *value { c.rise } else { c.fall };
        let next = *value + (t - *value) * k;
        *value = if (t - next).abs() < 1e-6 { t } else { next };
    }
    *value
}

/// One voice's pressure.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pressure {
    raw: u8,
    value: f32,
    steps: u32,
    released: bool,
}

impl Pressure {
    pub const fn new() -> Pressure { Pressure { raw: 0, value: 0.0, steps: 0, released: false } }

    /// Back to nothing, for a fresh voice.
    pub fn clear(&mut self) { *self = Pressure::new(); }

    /// A note-on: the hold-off starts again and the pad starts unpressed.
    /// The value is kept, so a voice that is retriggered while it sounds
    /// doesn't dip; a fresh voice is [`Pressure::clear`]ed first.
    pub fn start(&mut self) {
        self.raw = 0;
        self.steps = 0;
        self.released = false;
    }

    /// A pressure message, 0 to 127.
    pub fn set(&mut self, value: u8) { self.raw = value.min(127); }

    /// The note-off.
    pub fn release(&mut self) { self.released = true; }

    /// The last raw value, 0 to 127.
    pub fn raw(&self) -> u8 { self.raw }

    /// The shaped, smoothed value, 0 to 1.
    pub fn value(&self) -> f32 { self.value }

    pub fn released(&self) -> bool { self.released }

    /// Past the strike hold-off.
    pub fn ready(&self, c: &PressureCoefs) -> bool { self.steps >= c.holdoff_steps }

    /// The shaped value this voice is heading for now, or none while it holds
    /// still (in its hold-off, or after its note-off with Hold).
    pub fn aim(&self, c: &PressureCoefs) -> Option<f32> {
        if self.released {
            match c.release {
                PressureRelease::Hold => None,
                PressureRelease::Follow => Some(c.table[usize::from(self.raw)]),
                PressureRelease::Fade => Some(0.0),
            }
        } else if !self.ready(c) {
            None
        } else {
            Some(c.table[usize::from(self.raw)])
        }
    }

    /// One control step. Returns the value.
    pub fn step(&mut self, c: &PressureCoefs) -> f32 {
        let target = self.aim(c);
        self.steps = self.steps.saturating_add(1);
        glide(&mut self.value, target, c)
    }
}

/// The value every voice shares in [`PressureMode::Highest`]: the highest
/// aim among the held notes, smoothed like a voice. With no note held it
/// follows the releasing voices' aims, so it holds, follows or fades with
/// the release setting.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PressureChord {
    value: f32,
}

impl PressureChord {
    pub fn clear(&mut self) { self.value = 0.0; }

    pub fn value(&self) -> f32 { self.value }

    /// One control step, from each sounding voice's `(held, aim)` (see
    /// [`Pressure::aim`]), taken before the voices step.
    pub fn step<I: IntoIterator<Item = (bool, Option<f32>)>>(&mut self, c: &PressureCoefs, voices: I) -> f32 {
        let (mut any_held, mut held_top, mut rel_top) = (false, None::<f32>, None::<f32>);
        for (held, aim) in voices {
            let top = if held { any_held = true; &mut held_top } else { &mut rel_top };
            if let Some(a) = aim {
                *top = Some(top.map_or(a, |t| t.max(a)));
            }
        }
        glide(&mut self.value, if any_held { held_top } else { rel_top }, c)
    }
}

#[allow(clippy::too_many_arguments)]
const fn info(id: u16, name: &'static str, group: &'static str, min: f32, max: f32, default: f32, unit: Unit, curve: Curve) -> ParamInfo {
    ParamInfo { id, name, group, min, max, default, unit, curve, readonly: false }
}

const SLOT_TARGET_NAMES: [&str; SLOTS] = ["To 1", "To 2", "To 3", "To 4"];
const SLOT_DEPTH_NAMES: [&str; SLOTS] = ["Depth 1", "Depth 2", "Depth 3", "Depth 4"];

/// An engine's 16 pressure parameters from `base`, in `group`: the 8 shaper
/// settings at the defaults of [`PressureShape::default`], then 4 slots.
/// `targets` names what a slot can drive (index 0 should be "Off"), and
/// `slots` gives each slot's default target index and depth (%).
pub const fn params(base: u16, group: &'static str, targets: &'static [&'static str], slots: [(f32, f32); SLOTS]) -> [ParamInfo; FIELDS] {
    use Curve::Linear;
    let mut out = [
        choice(base + field::MODE, "Mode", group, MODES, 0.0),
        info(base + field::HOLDOFF, "Hold-off", group, 0.0, 100.0, 30.0, Unit::Ms, Linear),
        info(base + field::THRESHOLD, "Threshold", group, 0.0, 50.0, 8.0, Unit::Percent, Linear),
        info(base + field::FULL_AT, "Full at", group, 30.0, 100.0, 85.0, Unit::Percent, Linear),
        choice(base + field::CURVE, "Curve", group, CURVES, 2.0),
        info(base + field::RISE, "Rise", group, 0.0, 200.0, 8.0, Unit::Ms, Linear),
        info(base + field::FALL, "Fall", group, 0.0, 200.0, 40.0, Unit::Ms, Linear),
        choice(base + field::RELEASE, "Release", group, RELEASES, 0.0),
        // The slots, filled in below.
        UNSET, UNSET, UNSET, UNSET, UNSET, UNSET, UNSET, UNSET,
    ];
    let mut n = 0;
    while n < SLOTS {
        out[8 + 2 * n] = choice(base + field::target(n), SLOT_TARGET_NAMES[n], group, targets, slots[n].0);
        out[9 + 2 * n] = info(base + field::depth(n), SLOT_DEPTH_NAMES[n], group, -100.0, 100.0, slots[n].1, Unit::Percent, Linear);
        n += 1;
    }
    out
}

const UNSET: ParamInfo = info(0, "", "", 0.0, 1.0, 0.0, Unit::None, Curve::Linear);

const fn choice(id: u16, name: &'static str, group: &'static str, names: &'static [&'static str], default: f32) -> ParamInfo {
    info(id, name, group, 0.0, (names.len() - 1) as f32, default, Unit::None, Curve::Choice(names))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 3_000.0;

    fn coefs(f: impl FnOnce(&mut PressureShape)) -> PressureCoefs {
        let mut s = PressureShape::default();
        f(&mut s);
        s.coefs(RATE)
    }

    /// Steps a voice `n` times, returning the values.
    fn run(p: &mut Pressure, c: &PressureCoefs, n: usize) -> Vec<f32> { (0..n).map(|_| p.step(c)).collect() }

    #[test]
    fn below_the_threshold_is_exactly_zero_and_it_rises_with_no_step() {
        let s = PressureShape { curve: PressureCurve::Linear, ..PressureShape::default() };
        for raw in 0..=10 {
            // 10/127 is 7.9 %, under the 8 % threshold.
            assert_eq!(s.map(raw as f32 / 127.0), 0.0, "raw {raw}");
        }
        // Just over the threshold the output is tiny, not a jump.
        let just = s.map(0.08 + 0.001);
        assert!(just > 0.0 && just < 0.002, "{just}");
        // Full at 85 % gives the full effect, and more stays at 1.
        assert!((s.map(0.85) - 1.0).abs() < 1e-6);
        assert_eq!(s.map(1.0), 1.0);
        // Monotonic for every curve.
        for curve in [PressureCurve::Soft, PressureCurve::Linear, PressureCurve::Firm, PressureCurve::Hard] {
            let s = PressureShape { curve, ..PressureShape::default() };
            let t = s.coefs(RATE).table;
            assert!(t.windows(2).all(|w| w[1] >= w[0]), "{curve:?}");
            assert_eq!(t[0], 0.0);
            assert_eq!(t[127], 1.0);
        }
        // A firm curve keeps light pressure subtle.
        let firm = PressureShape::default();
        assert!(firm.map(0.4) < 0.4 * 0.75, "{}", firm.map(0.4));
    }

    #[test]
    fn a_strike_spike_in_the_hold_off_gives_nothing_then_pressure_glides_in() {
        let c = coefs(|_| {});
        assert_eq!(c.holdoff_steps, 90, "30 ms at 3 kHz");
        let mut p = Pressure::new();
        p.start();
        // The strike: a spike to full on the first frames.
        p.set(127);
        let during = run(&mut p, &c, 30);
        p.set(60);
        let during2 = run(&mut p, &c, 60);
        assert!(during.iter().chain(&during2).all(|&v| v == 0.0), "nothing moves in the hold-off");
        // Then the settled value glides in: no jump on any step.
        let after = run(&mut p, &c, 300);
        let target = c.table[60];
        assert!(after[0] > 0.0 && after[0] <= c.rise * target + 1e-6, "first step {}", after[0]);
        assert!(after.windows(2).all(|w| w[1] >= w[0] && w[1] - w[0] <= c.rise * target + 1e-6));
        assert!((after[299] - target).abs() < 1e-3, "settles at {target}: {}", after[299]);
    }

    #[test]
    fn a_retriggered_voice_keeps_its_value_through_the_hold_off() {
        let c = coefs(|_| {});
        let mut p = Pressure::new();
        p.start();
        p.set(100);
        run(&mut p, &c, 600);
        let held = p.value();
        assert!(held > 0.5);
        p.start();
        let during = run(&mut p, &c, 90);
        assert!(during.iter().all(|&v| v == held), "no dip in the hold-off");
    }

    /// Holds a pad at 100, lifts it to 0 over 20 ms before the note-off, then
    /// runs 100 ms of release. Returns (value at the note-off, values after).
    fn lift(release: PressureRelease) -> (f32, Vec<f32>) {
        let c = coefs(|s| s.release = release);
        let mut p = Pressure::new();
        p.start();
        p.set(100);
        run(&mut p, &c, 900);
        // 20 ms of lifting: 60 steps from 100 down to 0.
        for k in 0..60 {
            p.set((100 - k * 100 / 60) as u8);
            p.step(&c);
        }
        p.set(0);
        let at_off = p.value();
        p.release();
        (at_off, run(&mut p, &c, 600))
    }

    #[test]
    fn release_holds_follows_or_fades() {
        let c = coefs(|_| {});
        let full = c.table[100];
        let (at_off, hold) = lift(PressureRelease::Hold);
        // The 40 ms fall means a 20 ms lift has lowered it by well under half.
        assert!(at_off > 0.6 * full, "{at_off} of {full}");
        assert!(hold.iter().all(|&v| v == at_off), "Hold keeps the value");
        let (_, follow) = lift(PressureRelease::Follow);
        assert!(follow[599] < 0.01, "Follow goes down with the pad: {}", follow[599]);
        let (_, fade) = lift(PressureRelease::Fade);
        assert!(fade[599] < 0.01, "Fade falls to 0: {}", fade[599]);
        // Follow takes new pressure after the note-off; Fade ignores it.
        for (release, moves) in [(PressureRelease::Follow, true), (PressureRelease::Fade, false), (PressureRelease::Hold, false)] {
            let c = coefs(|s| s.release = release);
            let mut p = Pressure::new();
            p.start();
            run(&mut p, &c, 100);
            p.release();
            p.set(127);
            let v = run(&mut p, &c, 100)[99];
            assert_eq!(v > 0.5, moves, "{release:?}: {v}");
        }
    }

    #[test]
    fn each_step_moves_no_more_than_the_rise_or_fall_allows() {
        let c = coefs(|s| s.holdoff_ms = 0.0);
        let mut p = Pressure::new();
        p.start();
        let mut prev = 0.0;
        let mut x = 0x1234_5678u32;
        for _ in 0..5_000 {
            // A pad that jumps around at random.
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            p.set((x % 128) as u8);
            let v = p.step(&c);
            let d = v - prev;
            assert!(d <= c.rise + 1e-6 && -d <= c.fall + 1e-6, "step {d}");
            assert!((0.0..=1.0).contains(&v));
            prev = v;
        }
        // 0 ms means instant.
        let c = coefs(|s| {
            s.holdoff_ms = 0.0;
            s.rise_ms = 0.0;
        });
        let mut p = Pressure::new();
        p.start();
        p.set(127);
        assert_eq!(p.step(&c), 1.0);
    }

    #[test]
    fn highest_follows_the_highest_held_note() {
        let c = coefs(|s| s.holdoff_ms = 0.0);
        let mut a = Pressure::new();
        let mut b = Pressure::new();
        let mut chord = PressureChord::default();
        a.start();
        b.start();
        a.set(40);
        b.set(110);
        for _ in 0..1_000 {
            let aims = [(true, a.aim(&c)), (true, b.aim(&c))];
            chord.step(&c, aims);
            a.step(&c);
            b.step(&c);
        }
        assert!((chord.value() - c.table[110]).abs() < 1e-3);
        // The loud note lifts and releases: the chord falls to the other
        // held note's level, not to 0 or to the released note's value.
        b.set(0);
        b.release();
        for _ in 0..1_000 {
            let aims = [(true, a.aim(&c)), (false, b.aim(&c))];
            chord.step(&c, aims);
            a.step(&c);
            b.step(&c);
        }
        assert!((chord.value() - c.table[40]).abs() < 1e-3, "{}", chord.value());
        // Nothing held: with Hold, the chord holds still.
        a.release();
        let before = chord.value();
        for _ in 0..100 {
            chord.step(&c, [(false, a.aim(&c)), (false, b.aim(&c))]);
        }
        assert_eq!(chord.value(), before);
    }

    #[test]
    fn params_read_back_into_the_shape() {
        let ps = params(100, "Pressure", &["Off", "Cutoff"], [(1.0, 25.0), (0.0, 0.0), (0.0, 0.0), (0.0, 0.0)]);
        assert!(crate::check_params(&ps).is_ok(), "{:?}", crate::check_params(&ps));
        assert_eq!(ps[8].id, 108);
        assert_eq!(ps[15].id, 115);
        let get = |id: u16| ps.iter().find(|p| p.id == id).map_or(f32::NAN, |p| p.default);
        assert_eq!(PressureShape::from_params(100, get), PressureShape::default());
        assert_eq!(get(100 + field::target(0)), 1.0);
        assert_eq!(get(100 + field::depth(0)), 25.0);
    }
}
