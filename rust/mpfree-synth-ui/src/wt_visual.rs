//! The Wavetable synth's drawn visuals: each oscillator's table as a 3D
//! stack of frames with the frame its Position knob reads drawn bright (and
//! bent by its warp), plus the filter and envelopes, which share Poly's
//! drawings (the ids line up: see the test).
//!
//! Like [`crate::visual`], everything comes from parameter values, so it
//! moves the moment a knob turns. A user table isn't in the node's values,
//! so "User" draws Basic's frames under the caption "user table" until the
//! app passes the table in.

use mpfree_synth::wavetable::factory;
use mpfree_synth::wavetable::params::{self as wp, osc_id};
use mpfree_synth::wavetable::table::{Table, PREVIEW};
use mpfree_ui::Color;

use crate::visual::{self, Values};
use crate::widgets::{Card, Trace, QLINK_COLORS};

/// Points of one drawn curve, in the unit box.
pub type Curve = Vec<(f32, f32)>;

/// Frames drawn behind the bright one.
const GHOSTS: usize = 7;
/// How far back the stack leans: the last frame sits this far right and up.
const DEPTH_X: f32 = 0.22;
const DEPTH_Y: f32 = 0.38;
/// A frame's height in the box (it peaks at half of this above its line).
const AMP: f32 = 0.3;

/// The table oscillator `o` plays, by its Table choice.
fn table(v: &impl Values, o: usize) -> &'static Table {
    let f = factory();
    let i = v.get(osc_id(o, wp::TABLE));
    if i >= wp::USER_TABLE {
        &f[0]
    } else {
        &f[(i.max(0.0) as usize).min(f.len() - 1)]
    }
}

/// The frame position (0 to frames - 1) the Position knob reads.
fn position(v: &impl Values, o: usize, t: &Table) -> f32 {
    (v.get(osc_id(o, wp::POSITION)) / 100.0).clamp(0.0, 1.0) * (t.frames() - 1) as f32
}

/// Frame `pos` (crossfaded) at phase `ph`, from the table's preview points.
fn sample(t: &Table, pos: f32, ph: f32) -> f32 {
    let f0 = (pos as usize).min(t.frames() - 1);
    let f1 = (f0 + 1).min(t.frames() - 1);
    let k = pos - f0 as f32;
    let at = |f: usize| {
        let p = t.preview(f);
        let x = ph.rem_euclid(1.0) * PREVIEW as f32;
        let i = (x as usize).min(PREVIEW - 1);
        let fr = x - i as f32;
        p[i] + (p[(i + 1) % PREVIEW] - p[i]) * fr
    };
    at(f0) + (at(f1) - at(f0)) * k
}

/// The phase a warp reads from (the engine's, minus FM, which needs the
/// other oscillator's sound).
fn warp(mode: usize, amt: f32, p: f32) -> f32 {
    let amt = amt.clamp(0.0, 1.0);
    match mode {
        1 => {
            let k = 0.5 - 0.48 * amt;
            if p < k {
                p * 0.5 / k
            } else {
                0.5 + (p - k) * 0.5 / (1.0 - k)
            }
        }
        2 => (p * (1.0 + 7.0 * amt)).fract(),
        3 => {
            let m = 1.0 - (1.0 - 2.0 * p).abs();
            p + (m - p) * amt
        }
        _ => p,
    }
}

/// One frame laid into the stack at depth `d` (0 front, 1 back).
fn layered(d: f32, f: impl Fn(f32) -> f32) -> Vec<(f32, f32)> {
    let w = 1.0 - DEPTH_X - 0.04;
    (0..=visual::POINTS)
        .map(|i| {
            let x = i as f32 / visual::POINTS as f32;
            (
                0.02 + d * DEPTH_X + x * w,
                0.12 + d * DEPTH_Y + AMP * 0.5 * (1.0 + f(x)),
            )
        })
        .collect()
}

/// Oscillator `o`'s stack: ghost frames back to front, then the frame the
/// knob reads, warped. Empty if the oscillator is silent.
pub fn stack(v: &impl Values, o: usize) -> (Vec<Curve>, Curve) {
    if v.get(osc_id(o, wp::OSC_LEVEL)) <= 0.0 {
        return (Vec::new(), Vec::new());
    }
    let t = table(v, o);
    let last = (t.frames() - 1) as f32;
    let ghosts = (0..GHOSTS)
        .rev()
        .map(|g| {
            let d = g as f32 / (GHOSTS - 1) as f32;
            layered(d, |x| sample(t, d * last, x))
        })
        .collect();
    let pos = position(v, o, t);
    let d = if last > 0.0 { pos / last } else { 0.0 };
    let mode = v.get(osc_id(o, wp::WARP_MODE)) as usize;
    let amt = v.get(osc_id(o, wp::WARP)) / 100.0;
    (ghosts, layered(d, |x| sample(t, pos, warp(mode, amt, x))))
}

/// "Vowels · 6/16", or with a warp "6/16 · Bend 30 %" (the card is narrow;
/// the Table tile names the table).
pub fn osc_caption(v: &impl Values, o: usize) -> String {
    if v.get(osc_id(o, wp::OSC_LEVEL)) <= 0.0 {
        return "off".into();
    }
    let t = table(v, o);
    let user = v.get(osc_id(o, wp::TABLE)) >= wp::USER_TABLE;
    let name = if user { "user table" } else { t.name.as_str() };
    let frame = format!("{}/{}", position(v, o, t).round() as usize + 1, t.frames());
    let mode = v.get(osc_id(o, wp::WARP_MODE)) as usize;
    match wp::WARP_MODES.get(mode) {
        Some(m) if mode > 0 => format!("{frame} · {m} {:.0} %", v.get(osc_id(o, wp::WARP))),
        _ => format!("{name} · {frame}"),
    }
}

/// The cards: Osc A and Osc B stacks, the filter, the envelopes (amp
/// white, mod blue). Each card's page name is its title.
pub fn cards(v: &impl Values, page: &str) -> Vec<Card> {
    let (blue, green) = (QLINK_COLORS[0], QLINK_COLORS[1]);
    let ghost = Color::rgb(0x5A6273);
    let grey = Color::rgb(0x98A1B3);
    let white = Color::rgb(0xE9ECF2);
    let tr = |pts: Vec<(f32, f32)>, color: Color, width: f32, fill: bool, dashed: bool| Trace {
        pts,
        color,
        width,
        fill,
        dashed,
    };
    let osc = |o: usize, color: Color| {
        let (ghosts, front) = stack(v, o);
        let title = if o == 0 { "Osc A" } else { "Osc B" };
        let mut traces: Vec<Trace> = ghosts
            .into_iter()
            .map(|g| tr(g, ghost, 1.0, false, false))
            .collect();
        if !front.is_empty() {
            traces.push(tr(front, color, 2.5, false, false));
        }
        Card {
            title: title.into(),
            caption: osc_caption(v, o),
            traces,
            active: page == title,
            ..Card::default()
        }
    };
    let [f, moved] = visual::filter(v);
    let [amp, menv] = visual::envelopes(v);
    let mode = mpfree_synth::params::FILTER_MODES
        .get(v.get(wp::FILTER_MODE) as usize)
        .copied()
        .unwrap_or("");
    let freq_grid = [(100.0, "100"), (1000.0, "1k"), (10_000.0, "10k")]
        .iter()
        .map(|&(hz, s)| (visual::freq_x(hz), s.to_string()))
        .collect();
    vec![
        osc(0, blue),
        osc(1, green),
        Card {
            title: "Filter".into(),
            caption: mode.to_string(),
            traces: [tr(f, blue, 2.5, true, false)]
                .into_iter()
                .chain((!moved.is_empty()).then(|| tr(moved, grey, 1.5, false, true)))
                .collect(),
            grid: freq_grid,
            dot: Some(visual::freq_x(v.get(wp::CUTOFF))),
            active: page == "Filter",
            ..Card::default()
        },
        Card {
            title: "Env".into(),
            caption: "amp, mod".into(),
            traces: vec![
                tr(amp, white, 2.5, true, false),
                tr(menv, blue, 2.5, false, false),
            ],
            active: page == "Env" || page == "Mod",
            ..Card::default()
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_node::Node;
    use mpfree_synth::params as pp;
    use mpfree_synth::Wavetable;

    /// Poly's filter and envelope drawings read these ids; the Wavetable
    /// synth must keep the same meanings under them.
    #[test]
    fn shared_drawings_read_the_same_ids() {
        for (a, b) in [
            (pp::FILTER_MODE, wp::FILTER_MODE),
            (pp::CUTOFF, wp::CUTOFF),
            (pp::RESONANCE, wp::RESONANCE),
            (pp::FENV_AMOUNT, wp::MENV_CUTOFF),
            (pp::KEY_TRACK, wp::KEY_TRACK),
            (pp::FENV_A, wp::MENV_A),
            (pp::FENV_D, wp::MENV_D),
            (pp::FENV_S, wp::MENV_S),
            (pp::FENV_R, wp::MENV_R),
            (pp::AMP_A, wp::AMP_A),
            (pp::AMP_D, wp::AMP_D),
            (pp::AMP_S, wp::AMP_S),
            (pp::AMP_R, wp::AMP_R),
            (pp::LFO_SHAPE, wp::LFO_SHAPE),
            (pp::LFO_RATE, wp::LFO_RATE),
            (pp::LFO_SYNC, wp::LFO_SYNC),
        ] {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn the_bright_frame_follows_position_and_warp() {
        let mut s = Wavetable::new(48_000);
        let read = |s: &Wavetable| {
            let v = |id: u16| s.param(id);
            stack(&v, 0)
        };
        let (ghosts, sine) = read(&s);
        assert_eq!(ghosts.len(), GHOSTS);
        // Basic at 0 is a sine: its peak a quarter through.
        let top = sine
            .iter()
            .cloned()
            .fold((0.0, 0.0f32), |m, p| if p.1 > m.1 { p } else { m });
        assert!(
            (top.0 - (0.02 + 0.25 * (1.0 - DEPTH_X - 0.04))).abs() < 0.02,
            "{top:?}"
        );
        s.set_param(osc_id(0, wp::POSITION), 100.0);
        let (_, square) = read(&s);
        // At the back of the stack and squarer: most points near the extremes.
        assert!(square[0].1 > sine[0].1 + DEPTH_Y * 0.9);
        s.set_param(osc_id(0, wp::WARP_MODE), 2.0);
        s.set_param(osc_id(0, wp::WARP), 100.0);
        let v = |id: u16| s.param(id);
        assert!(osc_caption(&v, 0).contains("Sync 100 %"));
        // Osc B is silent at the defaults.
        assert!(stack(&v, 1).1.is_empty());
        assert_eq!(osc_caption(&v, 1), "off");
        assert_eq!(cards(&v, "Osc A").len(), 4);
    }
}
