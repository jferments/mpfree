//! The 808 body: the parts of the 808 bass drum that Bass's 808 body and
//! the Drums' 808 kick share, so the two are one implementation.
//!
//! - **The wave**: a sine ([`sin_turns`]) or the 808's triangle
//!   ([`triangle_turns`]), each rising from zero at the start of its cycle.
//! - **The pitch** ([`pitch_semis`]): the **punch**, the circuit's attack
//!   shift (the bridged-T's centre frequency rises by more than an octave
//!   for about 6 ms while the trigger pulse is high: Werner, Abel and
//!   Smith, DAFx 2014, section 8.1), as a quadratic fall from its height to
//!   nothing over [`PUNCH_S`]; and the **drop**, a cubic fall from its
//!   height that lands exactly on the note at its time. Drums use the drop
//!   for the 808's **sigh**: after the attack its pitch falls from about
//!   58 Hz to 48 Hz over the first 300 ms (the paper's Fig. 11; our
//!   reading of the plot's range), [`SIGH_SEMIS`] over [`SIGH_MS`].
//! - **The drive** ([`drive`], [`drive_gains`]): **Soft**, the smooth curve
//!   `u / sqrt(1 + u^2)` with an off-centre rest point ([`DRIVE_BIAS`]), so
//!   it adds the second and third harmonics that let a low note be heard on
//!   small speakers; **Hard**, a Padé `tanh`, towards a square. Both scaled
//!   so a signal peaking at [`DRIVE_REF`] still peaks there.
//!
//! Moved out of `bass.rs` (synth library wave 3) with Bass's output the
//! same to the bit; the sigh is new and Bass doesn't use it.

use std::f32::consts::TAU;

/// The punch: its length (seconds) and its shift at Bass's Punch 100,
/// semitones.
pub const PUNCH_S: f32 = 0.006;
pub const PUNCH_SEMIS: f32 = 12.0;

/// The 808's sigh: how far above its settled pitch the drum starts once the
/// attack is over (58 Hz over 48 Hz), and how long it takes to settle.
pub const SIGH_SEMIS: f32 = 3.28;
pub const SIGH_MS: f32 = 300.0;

/// Drive: its gain at 100 (dB), the curve's rest point (off centre, so it
/// adds even harmonics), and the level it keeps.
pub const DRIVE_DB: f32 = 30.0;
pub const DRIVE_BIAS: f32 = 0.3;
pub const DRIVE_REF: f32 = 0.25;

/// `x.floor()`, the same value for every input: a positive value below
/// 2^23 (every phase here in practice) truncates through an i32, which is
/// its floor, instead of the library call `floorf` is on ARMv7.
#[inline(always)]
pub fn floor(x: f32) -> f32 {
    if x > 0.0 && x < 8_388_608.0 {
        x as i32 as f32
    } else {
        x.floor()
    }
}

/// `sin(2 pi t)` for `t` in turns (0 to 1): an odd polynomial on the
/// folded quarter, within 1e-4 (a ninth-order Taylor series).
#[inline(always)]
pub fn sin_turns(t: f32) -> f32 {
    // Fold to -0.25..0.25 turns.
    let mut x = t - floor(t + 0.5);
    if x > 0.25 {
        x = 0.5 - x;
    } else if x < -0.25 {
        x = -0.5 - x;
    }
    let r = x * TAU;
    let r2 = r * r;
    r * (1.0
        + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5_040.0 + r2 * (1.0 / 362_880.0)))))
}

/// The 808 body's triangle at phase `t` (0 to 1 turns): rising from zero
/// at `t = 0`, its peak at a quarter.
#[inline(always)]
pub fn triangle_turns(t: f32) -> f32 {
    let u = t - 0.25;
    let u = u - floor(u);
    4.0 * (u - 0.5).abs() - 1.0
}

/// Semitones above the note at time `t` into it: a drop of `drop`
/// semitones landing at `drop_t`, plus a punch of `punch` semitones (at
/// its start) gone by `punch_t`. `t`, `drop_t` and `punch_t` in one unit.
#[inline]
pub fn pitch_semis(t: f32, drop: f32, drop_t: f32, punch: f32, punch_t: f32) -> f32 {
    let mut semis = 0.0;
    if t < drop_t {
        let x = 1.0 - t / drop_t;
        semis += drop * x * x * x;
    }
    if t < punch_t {
        let x = 1.0 - t / punch_t;
        semis += punch * x * x;
    }
    semis
}

/// The drive's soft curve: smooth everywhere.
#[inline(always)]
pub fn drive_curve(u: f32) -> f32 {
    u / (1.0 + u * u).sqrt()
}

/// `tanh(x)` by its [3/2] Padé form, exact at 0 and reaching 1 at ±3.
#[inline(always)]
pub fn tanh_pade(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// `y` through the drive with input gain `k` and output scale `norm`
/// ([`drive_gains`]), soft or `hard`. `k` 0 or less is off.
#[inline(always)]
pub fn drive(y: f32, k: f32, norm: f32, hard: bool) -> f32 {
    if k <= 0.0 {
        y
    } else if hard {
        norm * tanh_pade(k * y)
    } else {
        norm * (drive_curve(k * y + DRIVE_BIAS) - drive_curve(DRIVE_BIAS))
    }
}

/// The drive's input gain and output scale at `drive` (0 to 100), soft
/// or `hard`: a signal peaking at [`DRIVE_REF`] comes out peaking there
/// too. 0 is off.
pub fn drive_gains(drive: f32, hard: bool) -> (f32, f32) {
    if drive <= 0.0 {
        return (0.0, 1.0);
    }
    let k = 10f32.powf(drive * 0.01 * DRIVE_DB / 20.0);
    let peak = if hard {
        tanh_pade(k * DRIVE_REF)
    } else {
        drive_curve(k * DRIVE_REF + DRIVE_BIAS) - drive_curve(DRIVE_BIAS)
    };
    (k, DRIVE_REF / peak)
}

/// The drive's gain for a quiet signal against a signal peaking at
/// `peak`, both at the gains [`drive_gains`] gives: how much higher the
/// drive lifts a hit's tail than its start. 1 when off.
pub fn drive_lift(k: f32, norm: f32, hard: bool, peak: f32) -> f32 {
    if k <= 0.0 || peak <= 0.0 {
        return 1.0;
    }
    let small = if hard {
        norm * k
    } else {
        // The soft curve's slope at its rest point: (1 + b^2)^-1.5.
        let b2 = DRIVE_BIAS * DRIVE_BIAS;
        norm * k / ((1.0 + b2) * (1.0 + b2).sqrt())
    };
    let big = drive(peak, k, norm, hard).abs() / peak;
    (small / big.max(1e-9)).max(1.0)
}

/// One cycle of a sine peaking at [`DRIVE_REF`] through the drive at
/// `drive` (0 to 100), soft or `hard`, scaled so the reference peak is 1,
/// `points` points: for the Synth tab's picture.
pub fn drive_shape(drive: f32, hard: bool, points: usize) -> Vec<f32> {
    let n = points.max(2);
    let (k, norm) = drive_gains(drive, hard);
    (0..n)
        .map(|i| {
            let x = DRIVE_REF * sin_turns(i as f32 / (n - 1) as f32 * 0.999_999);
            let y = if k <= 0.0 {
                x
            } else if hard {
                norm * tanh_pade(k * x)
            } else {
                norm * (drive_curve(k * x + DRIVE_BIAS) - drive_curve(DRIVE_BIAS))
            };
            y / DRIVE_REF
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_is_the_library_floor() {
        for x in [
            -3.5f32, -1.0, -0.25, 0.0, 0.25, 0.999_999, 1.0, 7.5, 1e7, 1e9,
        ] {
            assert_eq!(floor(x).to_bits(), x.floor().to_bits(), "{x}");
        }
    }

    #[test]
    fn sin_turns_is_close() {
        for i in 0..1000 {
            let t = i as f32 / 1000.0;
            let want = (TAU * t).sin();
            assert!((sin_turns(t) - want).abs() < 2e-4, "{t}");
        }
    }

    #[test]
    fn the_triangle_rises_from_zero() {
        assert_eq!(triangle_turns(0.0), 0.0);
        assert_eq!(triangle_turns(0.25), 1.0);
        assert_eq!(triangle_turns(0.75), -1.0);
    }

    #[test]
    fn the_pitch_lands_on_the_note() {
        assert!((pitch_semis(0.0, 12.0, 40.0, 0.0, 6.0) - 12.0).abs() < 1e-6);
        assert_eq!(pitch_semis(40.0, 12.0, 40.0, 0.0, 6.0), 0.0);
        assert!((pitch_semis(0.0, 0.0, 40.0, 7.0, 6.0) - 7.0).abs() < 1e-6);
        assert_eq!(pitch_semis(6.0, 0.0, 40.0, 7.0, 6.0), 0.0);
    }

    #[test]
    fn the_sigh_spans_58_to_48_hz() {
        let hz = |ms: f32| 48.0 * (pitch_semis(ms, SIGH_SEMIS, SIGH_MS, 0.0, 1.0) / 12.0).exp2();
        assert!((hz(0.0) - 58.0).abs() < 0.1, "{}", hz(0.0));
        assert_eq!(hz(SIGH_MS), 48.0);
    }

    #[test]
    fn drive_keeps_the_reference_peak() {
        for d in [10.0, 50.0, 100.0] {
            for hard in [false, true] {
                let (k, n) = drive_gains(d, hard);
                let y = drive(DRIVE_REF, k, n, hard);
                assert!((y - DRIVE_REF).abs() < 1e-5, "{d} {hard}: {y}");
            }
        }
    }

    #[test]
    fn the_drive_lifts_a_tail_by_its_slope() {
        for hard in [false, true] {
            let (k, n) = drive_gains(80.0, hard);
            let lift = drive_lift(k, n, hard, DRIVE_REF);
            let quiet = drive(1e-4, k, n, hard) / 1e-4;
            assert!(
                ((quiet / 1.0) / lift - 1.0).abs() < 0.01,
                "{hard}: {quiet} {lift}"
            );
            assert!(lift > 3.0, "{lift}");
        }
        assert_eq!(drive_lift(0.0, 1.0, false, 0.25), 1.0);
    }
}
