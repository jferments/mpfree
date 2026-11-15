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

/// Gain in dB every millisecond (2 ms windows, so two cycles of 1 kHz).
fn gain_track(input: &[f64], output: &[f64], rate: u32) -> Vec<f64> {
    let win = (0.002 * rate as f64) as usize;
    let hop = (rate as usize / 1000).max(1);
    let mut g = Vec::new();
    let mut at = 0;
    while at + win <= input.len().min(output.len()) {
        g.push(sine_db(&output[at..at + win]) - sine_db(&input[at..at + win]));
        at += hop;
    }
    g
}

fn time_to(track: &[f64], from: f64, to: f64, share: f64) -> Option<f64> {
    if (to - from).abs() < 0.1 {
        return None;
    }
    let goal = from + share * (to - from);
    let i = if to < from {
        track.iter().position(|g| *g <= goal)?
    } else {
        track.iter().position(|g| *g >= goal)?
    };
    Some(i as f64)
}

/// Steps between `low_db` and `high_db` every `secs` seconds (low, high,
/// low) and times how the gain follows.
pub fn timing(dut: &mut dyn Dut, low_db: f64, high_db: f64, secs: f64) -> Timing {
    let rate = dut.rate();
    let step = (secs * rate as f64) as usize;
    let lat = dut.latency();
    let x = level_steps(1000.0, low_db, high_db, step, 3, rate);
    let out = dut.render(&Stereo::mono(x.clone(), rate), lat);
    timing_of(&x, &out.left[lat..], step, rate)
}

/// The same from a recording: `x` is the stepped stimulus, `y` the output
/// lined up with it (latency removed), `step` frames per level.
pub fn timing_of(x: &[f64], y: &[f64], step: usize, rate: u32) -> Timing {
    let g = gain_track(x, y, rate);
    let ms = step * 1000 / rate as usize;
    let settled = |at: usize| -> f64 {
        let at = at.min(g.len());
        let s = &g[at.saturating_sub(20)..at];
        s.iter().sum::<f64>() / s.len().max(1) as f64
    };
    // Steady gain at the end of the quiet, the loud and the quiet again.
    let (g_low, g_high) = (settled(ms - 2), settled(2 * ms - 2));
    let up = &g[(ms + 1).min(g.len())..(2 * ms).min(g.len())];
    let down = &g[(2 * ms + 1).min(g.len())..];
    Timing {
        depth_db: g_high - g_low,
        attack_ms: time_to(up, g_low, g_high, 0.63),
        attack90_ms: time_to(up, g_low, g_high, 0.9),
        release_ms: time_to(down, g_high, g_low, 0.63),
        release90_ms: time_to(down, g_high, g_low, 0.9),
    }
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
