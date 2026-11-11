//! Poly's drawn visuals, worked out from its parameter values (not from the
//! audio), so they update the moment a knob turns: one cycle of each
//! oscillator, the filter's response, the two envelopes and the LFO.
//!
//! Every function returns points in a unit box: x from 0 to 1 left to right,
//! y from 0 (bottom) to 1 (top). The widget scales them into its card.

use mpfree_synth::params as p;
use std::f32::consts::PI;

/// Points across each curve.
pub const POINTS: usize = 160;

/// Reads a value by parameter id.
pub trait Values {
    fn get(&self, id: u16) -> f32;
}

impl<F: Fn(u16) -> f32> Values for F {
    fn get(&self, id: u16) -> f32 {
        self(id)
    }
}

/// One naive cycle of a wave at phase `t` (0 to 1), -1 to 1.
fn wave(shape: f32, t: f32, pw: f32) -> f32 {
    let t = t.rem_euclid(1.0);
    match shape.round() as i32 {
        0 => 1.0 - 2.0 * t,
        1 => if t < pw { 1.0 } else { -1.0 },
        2 => if t < 0.5 { 4.0 * t - 1.0 } else { 3.0 - 4.0 * t },
        _ => (2.0 * PI * t).sin(),
    }
}

/// Two cycles of oscillator 1 and, over the same time, oscillator 2 at its
/// pitch relative to oscillator 1. Each is scaled by its level. Empty when
/// the oscillator's level is zero.
pub fn oscillators(v: &impl Values) -> [Vec<(f32, f32)>; 2] {
    let o1 = (v.get(p::OSC1_WAVE), v.get(p::OSC1_PW) / 100.0, v.get(p::OSC1_LEVEL) / 100.0, v.get(p::OSC1_OCT));
    let o2 = (v.get(p::OSC2_WAVE), v.get(p::OSC2_PW) / 100.0, v.get(p::OSC2_LEVEL) / 100.0);
    let semis = 12.0 * (v.get(p::OSC2_OCT) - o1.3) + v.get(p::OSC2_SEMI) + v.get(p::OSC2_FINE) / 100.0;
    let ratio = 2f32.powf(semis / 12.0).clamp(1.0 / 16.0, 16.0);
    let trace = |shape: f32, pw: f32, level: f32, cycles: f32| -> Vec<(f32, f32)> {
        if level <= 0.0 {
            return Vec::new();
        }
        (0..=POINTS)
            .map(|i| {
                let x = i as f32 / POINTS as f32;
                (x, 0.5 + 0.45 * level * wave(shape, x * cycles, pw))
            })
            .collect()
    };
    [trace(o1.0, o1.1, o1.2, 2.0), trace(o2.0, o2.1, o2.2, 2.0 * ratio)]
}

/// The lowest and highest frequency the filter view shows (log scale).
pub const F_LO: f32 = 20.0;
pub const F_HI: f32 = 20_000.0;
/// The level range the filter view shows, in dB: top and bottom.
pub const DB_TOP: f32 = 18.0;
pub const DB_BOTTOM: f32 = -48.0;

/// The x position of a frequency on the filter view.
pub fn freq_x(hz: f32) -> f32 {
    ((hz / F_LO).ln() / (F_HI / F_LO).ln()).clamp(0.0, 1.0)
}

/// The filter's gain at `hz` (linear), for mode 0 to 3 (LP 24, LP 12, BP 12,
/// HP 12), cutoff `fc` and resonance `res` (0 to 1), from the analogue
/// prototypes of the filters Poly runs.
pub fn filter_gain(mode: f32, fc: f32, res: f32, hz: f32) -> f32 {
    let w = hz / fc.max(1.0);
    let res = res.clamp(0.0, 1.0);
    match mode.round() as i32 {
        0 => {
            // Ladder: makeup / ((1 + s)^4 + k), s = jw.
            let k = 3.95 * res;
            let makeup = 1.0 + 0.6 * k;
            // (1 + jw)^4 = (1 - 6w^2 + w^4) + j(4w - 4w^3)
            let re = 1.0 - 6.0 * w * w + w.powi(4) + k;
            let im = 4.0 * w - 4.0 * w.powi(3);
            makeup / (re * re + im * im).sqrt()
        }
        m => {
            // State variable: denominator 1 - w^2 + j d w.
            let d = 2.0 - 1.95 * res;
            let den = ((1.0 - w * w).powi(2) + (d * w).powi(2)).sqrt();
            let num = match m {
                1 => 1.0,
                2 => d * w,
                _ => w * w,
            };
            num / den
        }
    }
}

/// The filter's response at its set cutoff, and where the filter envelope's
/// peak takes it (the dashed line). Points as (x, y).
pub fn filter(v: &impl Values) -> [Vec<(f32, f32)>; 2] {
    let mode = v.get(p::FILTER_MODE);
    let fc = v.get(p::CUTOFF);
    let res = v.get(p::RESONANCE) / 100.0;
    let env = v.get(p::FENV_AMOUNT);
    let curve = |fc: f32| -> Vec<(f32, f32)> {
        (0..=POINTS)
            .map(|i| {
                let x = i as f32 / POINTS as f32;
                let hz = F_LO * (F_HI / F_LO).powf(x);
                let db = 20.0 * filter_gain(mode, fc, res, hz).max(1e-6).log10();
                (x, ((db - DB_BOTTOM) / (DB_TOP - DB_BOTTOM)).clamp(0.0, 1.0))
            })
            .collect()
    };
    let peak = (fc * 2f32.powf(env)).clamp(F_LO, F_HI);
    let moved = if (peak / fc - 1.0).abs() > 0.01 { curve(peak) } else { Vec::new() };
    [curve(fc), moved]
}

/// An ADSR's shape, with times drawn on a square-root scale so short and
/// long envelopes both read. The sustain is held for a fixed share.
pub fn envelope(a_ms: f32, d_ms: f32, sustain: f32, r_ms: f32) -> Vec<(f32, f32)> {
    let s = sustain.clamp(0.0, 1.0);
    let (wa, wd, wr) = (a_ms.max(0.0).sqrt(), d_ms.max(0.0).sqrt(), r_ms.max(0.0).sqrt());
    let hold = 0.22;
    let scale = (1.0 - hold) / (wa + wd + wr).max(1e-6);
    let (xa, xd, xr) = (wa * scale, wd * scale, wr * scale);
    let n = POINTS / 4;
    let mut pts = Vec::with_capacity(4 * n + 2);
    // Attack: an analogue-style curve that bends over as it nears the top
    // (Poly aims past 1.0 and stops there).
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let y = (1.0 - (-2.0 * t).exp()) / (1.0 - (-2.0f32).exp());
        pts.push((xa * t, y));
    }
    // Decay and release fall exponentially (to within about 1 %).
    for i in 1..=n {
        let t = i as f32 / n as f32;
        pts.push((xa + xd * t, s + (1.0 - s) * (-4.6 * t).exp()));
    }
    pts.push((xa + xd + hold, s));
    for i in 1..=n {
        let t = i as f32 / n as f32;
        pts.push((xa + xd + hold + xr * t, s * (-4.6 * t).exp()));
    }
    pts
}

/// The amp envelope and the filter envelope.
pub fn envelopes(v: &impl Values) -> [Vec<(f32, f32)>; 2] {
    [
        envelope(v.get(p::AMP_A), v.get(p::AMP_D), v.get(p::AMP_S) / 100.0, v.get(p::AMP_R)),
        envelope(v.get(p::FENV_A), v.get(p::FENV_D), v.get(p::FENV_S) / 100.0, v.get(p::FENV_R)),
    ]
}

/// Two cycles of the LFO's shape. Sample and hold shows a fixed pattern of
/// steps, as an example of what it does.
pub fn lfo(v: &impl Values) -> Vec<(f32, f32)> {
    const HELD: [f32; 8] = [0.4, -0.7, 0.9, -0.2, 0.6, -0.9, 0.1, -0.5];
    let shape = v.get(p::LFO_SHAPE).round() as i32;
    (0..=POINTS)
        .map(|i| {
            let x = i as f32 / POINTS as f32;
            let t = (x * 2.0).rem_euclid(1.0);
            let y = match shape {
                0 => (2.0 * PI * t).sin(),
                1 => if t < 0.25 { 4.0 * t } else if t < 0.75 { 2.0 - 4.0 * t } else { 4.0 * t - 4.0 },
                2 => 1.0 - 2.0 * t,
                3 => if t < 0.5 { 1.0 } else { -1.0 },
                _ => HELD[((x * 8.0) as usize).min(7)],
            };
            (x, 0.5 + 0.42 * y)
        })
        .collect()
}

/// The LFO card's caption: its shape and its rate (synced or free).
pub fn lfo_caption(v: &impl Values) -> String {
    let shape = p::LFO_SHAPES.get(v.get(p::LFO_SHAPE).round() as usize).copied().unwrap_or("?");
    let sync = v.get(p::LFO_SYNC).round() as usize;
    if sync > 0 && sync < p::LFO_SYNCS.len() {
        format!("{} · {}", shape.to_lowercase(), p::LFO_SYNCS[sync])
    } else {
        let hz = v.get(p::LFO_RATE);
        format!("{} · {} Hz", shape.to_lowercase(), if hz >= 10.0 { format!("{hz:.0}") } else { format!("{hz:.1}") })
    }
}

/// The oscillator card's caption, such as "saw + pulse".
pub fn osc_caption(v: &impl Values) -> String {
    let name = |id: u16| p::WAVES.get(v.get(id).round() as usize).copied().unwrap_or("?").to_lowercase();
    match (v.get(p::OSC1_LEVEL) > 0.0, v.get(p::OSC2_LEVEL) > 0.0) {
        (true, true) => format!("{} + {}", name(p::OSC1_WAVE), name(p::OSC2_WAVE)),
        (true, false) => name(p::OSC1_WAVE),
        (false, true) => name(p::OSC2_WAVE),
        (false, false) => "off".to_string(),
    }
}

/// The pressure curve: shaped output (y) against raw pad pressure (x), for
/// the shaper settings laid out from `base` (see `mpfree_node::pressure`).
pub fn pressure_curve(v: &impl Values, base: u16) -> Vec<(f32, f32)> {
    let shape = mpfree_node::PressureShape::from_params(base, |id| v.get(id));
    (0..=POINTS).map(|i| i as f32 / POINTS as f32).map(|x| (x, shape.map(x))).collect()
}

/// The pressure card's caption: the curve and the live range, such as
/// "Firm · 8-85 %".
pub fn pressure_caption(v: &impl Values, base: u16) -> String {
    use mpfree_node::pressure::{field, CURVES, MODES};
    let mode = v.get(base + field::MODE).round() as usize;
    if mode == 2 {
        return "off".to_string();
    }
    let curve = CURVES.get(v.get(base + field::CURVE).round() as usize).copied().unwrap_or("");
    let lo = v.get(base + field::THRESHOLD).round();
    let hi = v.get(base + field::FULL_AT).round().max(lo + 1.0);
    let highest = if mode == 1 { format!(" · {}", MODES[1].to_lowercase()) } else { String::new() };
    format!("{curve} · {lo}-{hi} %{highest}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_synth::params::{info, PARAMS};

    fn defaults(id: u16) -> f32 {
        info(id).map(|i| i.default).unwrap_or(0.0)
    }

    fn db(g: f32) -> f32 {
        20.0 * g.log10()
    }

    #[test]
    fn filter_curves_have_the_right_shape() {
        // Lowpass: flat below the cutoff, -24 dB an octave above for LP 24.
        let lp24 = |hz| db(filter_gain(0.0, 1000.0, 0.0, hz));
        assert!(lp24(20.0).abs() < 0.1);
        assert!((lp24(8000.0) - lp24(4000.0) + 24.0).abs() < 1.0, "{} {}", lp24(4000.0), lp24(8000.0));
        let lp12 = |hz| db(filter_gain(1.0, 1000.0, 0.0, hz));
        assert!((lp12(16000.0) - lp12(8000.0) + 12.0).abs() < 0.5);
        // Highpass mirrors it; bandpass peaks at the cutoff at 0 dB.
        let hp = |hz| db(filter_gain(3.0, 1000.0, 0.0, hz));
        assert!(hp(20_000.0).abs() < 0.1 && hp(50.0) < -40.0);
        assert!(db(filter_gain(2.0, 1000.0, 0.0, 1000.0)).abs() < 0.01);
        // Resonance makes a peak near the cutoff.
        assert!(db(filter_gain(0.0, 1000.0, 0.9, 1000.0)) > 6.0);
        assert!(db(filter_gain(1.0, 1000.0, 0.9, 1000.0)) > 6.0);
    }

    #[test]
    fn curves_stay_in_the_box_for_any_setting() {
        for p in PARAMS {
            for v in [p.min, p.max] {
                let values = |id: u16| if id == p.id { v } else { defaults(id) };
                let mut all: Vec<(f32, f32)> = Vec::new();
                all.extend(oscillators(&values).concat());
                all.extend(filter(&values).concat());
                all.extend(envelopes(&values).concat());
                all.extend(lfo(&values));
                for (x, y) in all {
                    assert!((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y), "{} = {v}: ({x}, {y})", p.name);
                }
            }
        }
    }

    #[test]
    fn envelopes_start_and_end_at_zero_and_hold_the_sustain() {
        let e = envelope(10.0, 300.0, 0.6, 250.0);
        assert_eq!(e[0], (0.0, 0.0));
        let top = e.iter().map(|p| p.1).fold(0.0, f32::max);
        assert!((top - 1.0).abs() < 1e-4);
        assert!(e.last().unwrap().1 < 0.01);
        assert!((e.last().unwrap().0 - 1.0).abs() < 1e-4);
        assert!(e.iter().any(|p| (p.1 - 0.6).abs() < 1e-4));
        assert!(e.windows(2).all(|w| w[1].0 >= w[0].0), "time runs forward");
    }

    #[test]
    fn captions() {
        assert_eq!(osc_caption(&defaults), "saw + saw");
        assert_eq!(lfo_caption(&defaults), "sine · 5.0 Hz");
        let synced = |id: u16| if id == p::LFO_SYNC { 6.0 } else { defaults(id) };
        assert_eq!(lfo_caption(&synced), "sine · 1/8");
    }
}
