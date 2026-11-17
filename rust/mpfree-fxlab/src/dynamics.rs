//! Dynamics: the static curve (output level against input level on a
//! steady sine) and the attack and release times on level steps.

use crate::dut::Dut;
use crate::signals::{db_to_amp, level_steps, sine};
use crate::wav::Stereo;

/// RMS of `x` as the dBFS peak of a sine with that RMS.
fn sine_db(x: &[f64]) -> f64 {
    let rms = (x.iter().map(|v| v * v).sum::<f64>() / x.len().max(1) as f64).sqrt();
    20.0 * (rms * std::f64::consts::SQRT_2).max(1e-15).log10()
}

/// Output level (dBFS, from the last 200 ms of 800 ms) for a 1 kHz sine at
/// each input level.
pub fn static_curve(dut: &mut dyn Dut, levels: &[f64]) -> Vec<(f64, f64)> {
    let rate = dut.rate();
    let n = (0.8 * rate as f64) as usize;
    let tail = (0.2 * rate as f64) as usize;
    let lat = dut.latency();
    levels
        .iter()
        .map(|&db| {
            let stim = Stereo::mono(sine(1000.0, db_to_amp(db), n, rate), rate);
            let out = dut.render(&stim, lat);
            (db, sine_db(&out.left[n + lat - tail..n + lat]))
        })
        .collect()
}

/// Attack and release, measured on a 1 kHz sine stepping between two
/// levels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    /// Gain change (dB) on the loud level once settled, against the quiet.
    pub depth_db: f64,
    /// Milliseconds to cover 63 % (in dB) of the gain change after the
    /// step up.
    pub attack_ms: Option<f64>,
    /// Milliseconds to 90 % after the step up.
    pub attack90_ms: Option<f64>,
    /// Milliseconds to cover 63 % back after the step down.
    pub release_ms: Option<f64>,
    /// Milliseconds to 90 % after the step down.
    pub release90_ms: Option<f64>,
}

/// The tone the level steps are made of.
const TONE_HZ: f64 = 1000.0;

/// Gain in dB at frame `centre`: the tone's amplitude out over in, each
/// read through a 1 ms Hann window (a cycle of the tone) at the tone's
/// frequency, or `None` past either end. Reading the tone itself rather
/// than the power in a window keeps a fast attack from being smeared or
/// pulled toward the louder end of the window (2 ms of power read a 1 ms
/// attack as 1.5 ms); the Hann window's nulls fall on the tone's second
/// harmonic, where a changing gain's ripple sits.
fn gain_at(input: &[f64], output: &[f64], centre: f64, rate: u32) -> Option<f64> {
    let half = 0.0005 * rate as f64;
    let from = (centre - half).round();
    let to = (centre + half).round();
    if from < 0.0 || to as usize > input.len().min(output.len()) {
        return None;
    }
    let (from, to) = (from as usize, to as usize);
    let n = (to - from) as f64;
    let w = std::f64::consts::TAU * TONE_HZ / rate as f64;
    let (mut x, mut y) = ((0.0, 0.0), (0.0, 0.0));
    for k in from..to {
        let h = 0.5 - 0.5 * (std::f64::consts::TAU * ((k - from) as f64 + 0.5) / n).cos();
        let (s, c) = (w * k as f64).sin_cos();
        x.0 += h * c * input[k];
        x.1 += h * s * input[k];
        y.0 += h * c * output[k];
        y.1 += h * s * output[k];
    }
    let power = |(a, b): (f64, f64)| (a * a + b * b).max(1e-300);
    Some(10.0 * (power(y) / power(x)).log10())
}

/// The gain every millisecond from frame `start` on, each window centred
/// on its millisecond (frames at the exact rate, so the times don't drift
/// at 44.1 kHz).
fn gain_track(input: &[f64], output: &[f64], start: usize, rate: u32) -> Vec<f64> {
    let per_ms = 0.001 * rate as f64;
    (0..)
        .map_while(|i| gain_at(input, output, start as f64 + i as f64 * per_ms, rate))
        .collect()
}

/// Milliseconds until `track` (one value a millisecond, from the step)
/// has covered `share` of the way from `from` to `to`, found between the
/// milliseconds by halving with `at` (the gain that many milliseconds
/// after the step).
fn time_to(
    track: &[f64],
    at: impl Fn(f64) -> Option<f64>,
    from: f64,
    to: f64,
    share: f64,
) -> Option<f64> {
    if (to - from).abs() < 0.1 {
        return None;
    }
    let goal = from + share * (to - from);
    let past = |g: f64| if to < from { g <= goal } else { g >= goal };
    let i = track.iter().position(|g| past(*g))?;
    if i == 0 {
        return Some(0.0);
    }
    let (mut lo, mut hi) = (i as f64 - 1.0, i as f64);
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        if at(mid).is_some_and(past) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Steps between `low_db` and `high_db` every `secs` seconds (low, high,
/// low) and times how the gain follows.
pub fn timing(dut: &mut dyn Dut, low_db: f64, high_db: f64, secs: f64) -> Timing {
    let rate = dut.rate();
    let step = (secs * rate as f64) as usize;
    let lat = dut.latency();
    let x = level_steps(TONE_HZ, low_db, high_db, step, 3, rate);
    let out = dut.render(&Stereo::mono(x.clone(), rate), lat);
    timing_of(&x, &out.left[lat..], step, rate)
}

/// The same from a recording: `x` is the stepped stimulus, `y` the output
/// lined up with it (latency removed), `step` frames per level.
pub fn timing_of(x: &[f64], y: &[f64], step: usize, rate: u32) -> Timing {
    let per_ms = 0.001 * rate as f64;
    // Steady gain over the 20 ms before frame `end`.
    let settled = |end: usize| -> f64 {
        let g: Vec<f64> = (1..=20)
            .filter_map(|k| gain_at(x, y, end as f64 - (k as f64 + 1.0) * per_ms, rate))
            .collect();
        g.iter().sum::<f64>() / g.len().max(1) as f64
    };
    // Steady gain at the end of the quiet, the loud and the quiet again.
    let (g_low, g_high) = (settled(step), settled(2 * step));
    let up = gain_track(x, y, step, rate);
    let up = &up[..up.len().min((step as f64 / per_ms) as usize)];
    let down = gain_track(x, y, 2 * step, rate);
    let after = |from: usize| move |ms: f64| gain_at(x, y, from as f64 + ms * per_ms, rate);
    Timing {
        depth_db: g_high - g_low,
        attack_ms: time_to(up, after(step), g_low, g_high, 0.63),
        attack90_ms: time_to(up, after(step), g_low, g_high, 0.9),
        release_ms: time_to(&down, after(2 * step), g_high, g_low, 0.63),
        release90_ms: time_to(&down, after(2 * step), g_high, g_low, 0.9),
    }
}

/// How the gain comes back after `loud_secs` of a 1 kHz tone at
/// `high_db` (half a second at `low_db` before, `quiet_secs` after):
/// milliseconds to 63 % and to 90 % of the way from the gain at the end of
/// the loud part to the gain at the end. A program-dependent release comes
/// back slower after a long loud part than after a short one.
pub fn release_after(
    dut: &mut dyn Dut,
    low_db: f64,
    high_db: f64,
    loud_secs: f64,
    quiet_secs: f64,
) -> (Option<f64>, Option<f64>) {
    let rate = dut.rate();
    let per_ms = 0.001 * rate as f64;
    let (lead, loud) = (rate as usize / 2, (loud_secs * rate as f64) as usize);
    let n = lead + loud + (quiet_secs * rate as f64) as usize;
    let x: Vec<f64> = sine(TONE_HZ, 1.0, n, rate)
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let high = i >= lead && i < lead + loud;
            v * db_to_amp(if high { high_db } else { low_db })
        })
        .collect();
    let lat = dut.latency();
    let y = dut.render(&Stereo::mono(x.clone(), rate), lat);
    let y = &y.left[lat..];
    let down = lead + loud;
    let Some(from) = gain_at(&x, y, down as f64 - 1.5 * per_ms, rate) else {
        return (None, None);
    };
    let ends: Vec<f64> = (1..=20)
        .filter_map(|k| gain_at(&x, y, n as f64 - (k as f64 + 1.0) * per_ms, rate))
        .collect();
    let to = ends.iter().sum::<f64>() / ends.len().max(1) as f64;
    let track = gain_track(&x, y, down, rate);
    let at = |ms: f64| gain_at(&x, y, down as f64 + ms * per_ms, rate);
    (
        time_to(&track, at, from, to, 0.63),
        time_to(&track, at, from, to, 0.9),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dut::Wire;

    #[test]
    fn a_wire_has_a_straight_curve_and_no_gain_change() {
        let c = static_curve(&mut Wire { rate: 48_000 }, &[-40.0, -20.0, 0.0]);
        for (i, o) in c {
            assert!((i - o).abs() < 0.01);
        }
        let t = timing(&mut Wire { rate: 48_000 }, -30.0, -6.0, 0.5);
        assert!(t.depth_db.abs() < 0.01);
        assert_eq!(t.attack_ms, None);
    }

    #[test]
    fn the_registry_compressor_compresses_and_has_an_attack() {
        let mut d = crate::dut::NodeDut::from_registry("mpfree.fx.compressor", 48_000).unwrap();
        let c = static_curve(&mut d, &[-40.0, 0.0]);
        let gain_loud = c[1].1 - c[1].0;
        let gain_quiet = c[0].1 - c[0].0;
        assert!(gain_loud < gain_quiet - 3.0, "{c:?}");
        let t = timing(&mut d, -40.0, 0.0, 0.5);
        assert!(t.depth_db < -3.0, "{t:?}");
        assert!(t.attack_ms.is_some() && t.release_ms.is_some(), "{t:?}");
    }
}
