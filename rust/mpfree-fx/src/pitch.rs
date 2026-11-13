//! Pitch shifter: moves the sound up or down without changing its speed,
//! from two octaves down to two up, with a fine tune, a stereo spread for
//! doubling and feedback for rising or falling cascades.
//!
//! The classic delay-line shifter: two read heads sweep through a short
//! delay at the rate the pitch asks for, half a window apart, each faded in
//! and out with a raised-sine window so one is always at full level while
//! the other jumps back. A longer Window smears less on low, steady sounds;
//! a shorter one follows drums and speech more tightly. It runs on every
//! kind of input without needing to find a pitch, so it never glitches on
//! chords or noise, and costs a few multiplies a sample.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{defaults, fast_exp2};

/// [`PitchShifter`] parameter ids.
pub mod ids {
    pub const PITCH: u16 = 0;
    pub const FINE: u16 = 1;
    pub const SPREAD: u16 = 2;
    pub const WINDOW: u16 = 3;
    pub const FEEDBACK: u16 = 4;
    pub const MIX: u16 = 5;
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.pitch",
    name: "Pitch shifter",
    kind: Kind::Effect,
};

#[allow(clippy::too_many_arguments)]
const fn p(
    id: u16,
    name: &'static str,
    group: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: Unit,
    curve: Curve,
) -> ParamInfo {
    ParamInfo {
        id,
        name,
        group,
        min,
        max,
        default,
        unit,
        curve,
        readonly: false,
    }
}

static PARAMS: [ParamInfo; 6] = [
    p(
        0,
        "Pitch",
        "Pitch",
        -24.0,
        24.0,
        12.0,
        Unit::Semitones,
        Curve::Stepped,
    ),
    p(
        1,
        "Fine",
        "Pitch",
        -100.0,
        100.0,
        0.0,
        Unit::Cents,
        Curve::Linear,
    ),
    p(
        2,
        "Spread",
        "Pitch",
        0.0,
        50.0,
        0.0,
        Unit::Cents,
        Curve::Linear,
    ),
    p(
        3,
        "Window",
        "Sound",
        20.0,
        120.0,
        50.0,
        Unit::Ms,
        Curve::Log,
    ),
    p(
        4,
        "Feedback",
        "Sound",
        0.0,
        90.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        5,
        "Mix",
        "Output",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
    ),
];

/// The longest window, in seconds (sizes the delay lines).
const MAX_WINDOW_S: f32 = 0.12;

/// `sin(pi x)` for x in [0, 1], Bhaskara I's rational form (within 0.0017),
/// with no library call.
#[inline]
fn sin_pi(x: f32) -> f32 {
    let q = x * (1.0 - x);
    16.0 * q / (5.0 - 4.0 * q)
}

/// One channel's delay line and its two heads' phase.
struct Head {
    buf: Vec<f32>,
    mask: usize,
    write: usize,
    /// Where head A is through the window, 0 to 1 (head B is half a window
    /// on).
    phase: f32,
    /// The last output, for feedback.
    last: f32,
}

impl Head {
    fn new() -> Head {
        Head {
            buf: Vec::new(),
            mask: 0,
            write: 0,
            phase: 0.0,
            last: 0.0,
        }
    }

    fn size(&mut self, frames: usize) {
        let n = frames.next_power_of_two().max(8);
        self.buf = vec![0.0; n];
        self.mask = n - 1;
        self.clear();
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.write = 0;
        self.phase = 0.0;
        self.last = 0.0;
    }

    /// The sample `d` frames ago (fractional, at least 1), linearly
    /// interpolated.
    #[inline]
    fn tap(&self, d: f32) -> f32 {
        let di = d as usize;
        let frac = d - di as f32;
        let a = self.buf[(self.write.wrapping_sub(di)) & self.mask];
        let b = self.buf[(self.write.wrapping_sub(di + 1)) & self.mask];
        a + (b - a) * frac
    }

    /// Writes `x` (plus feedback) and returns the shifted sample.
    #[inline]
    fn next(&mut self, x: f32, ratio: f32, win: f32, fb: f32) -> f32 {
        self.write = (self.write + 1) & self.mask;
        self.buf[self.write] = crate::flush(x + fb * self.last);
        // The heads move through the delay at (1 - ratio) a frame: going
        // up shortens the delay, going down lengthens it.
        let mut ph = self.phase + (1.0 - ratio) / win;
        // The step is far below one window, so one add or subtract wraps it
        // (the same value `ph - ph.floor()` gives, without the library
        // call `floor` is on the MPC's ARM).
        if ph >= 1.0 {
            ph -= 1.0;
        } else if ph < 0.0 {
            ph += 1.0;
        }
        if !(0.0..1.0).contains(&ph) {
            ph -= ph.floor();
        }
        self.phase = ph;
        let pb = if ph >= 0.5 { ph - 0.5 } else { ph + 0.5 };
        let (wa, wb) = (sin_pi(ph), sin_pi(pb));
        let (wa, wb) = (wa * wa, wb * wb);
        let norm = 1.0 / (wa + wb).max(1e-6);
        let y = (self.tap(1.0 + ph * win) * wa + self.tap(1.0 + pb * win) * wb) * norm;
        self.last = crate::soft(y);
        y
    }
}

/// A two-head delay-line pitch shifter.
pub struct PitchShifter {
    v: [f32; 6],
    rate: u32,
    /// Semitones plus cents, as cents, smoothed.
    cents: Smooth,
    spread: Smooth,
    window: Smooth,
    feedback: Smooth,
    mix: Smooth,
    heads: [Head; 2],
    /// The cents last turned into ratios and the ratios.
    was: (f32, f32),
    ratios: [f32; 2],
}

impl Default for PitchShifter {
    fn default() -> Self {
        Self::new()
    }
}

impl PitchShifter {
    pub fn new() -> PitchShifter {
        let v = defaults(&PARAMS);
        let mut s = PitchShifter {
            v,
            rate: 0,
            cents: Smooth::new(v[0] * 100.0 + v[1]),
            spread: Smooth::new(v[2]),
            window: Smooth::new(v[3]),
            feedback: Smooth::new(v[4] / 100.0),
            mix: Smooth::new(v[5] / 100.0),
            heads: [Head::new(), Head::new()],
            was: (f32::NAN, f32::NAN),
            ratios: [1.0; 2],
        };
        s.prepare(48_000, mpfree_node::MAX_BLOCK);
        s
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::PITCH | ids::FINE => self.cents.target = self.v[0] * 100.0 + self.v[1],
            ids::SPREAD => self.spread.target = v,
            ids::WINDOW => self.window.target = v,
            ids::FEEDBACK => self.feedback.target = v / 100.0,
            ids::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let ms = self.rate as f32 * 0.001;
        for i in 0..l.len() {
            let cents = self.cents.next();
            let spread = self.spread.next();
            if (cents, spread) != self.was {
                self.was = (cents, spread);
                let half = spread * 0.5;
                self.ratios = [
                    fast_exp2((cents - half) / 1200.0),
                    fast_exp2((cents + half) / 1200.0),
                ];
            }
            let win = self.window.next() * ms;
            let fb = self.feedback.next();
            let mix = self.mix.next();
            let x = [l[i], r[i]];
            let y0 = self.heads[0].next(x[0], self.ratios[0], win, fb);
            let y1 = self.heads[1].next(x[1], self.ratios[1], win, fb);
            l[i] = crate::lerp(x[0], y0, mix);
            r[i] = crate::lerp(x[1], y1, mix);
        }
    }
}

impl Node for PitchShifter {
    fn info(&self) -> &'static NodeInfo {
        &INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        let rate = rate.max(1);
        if rate != self.rate {
            self.rate = rate;
            let frames = (MAX_WINDOW_S * rate as f32) as usize + 4;
            for h in &mut self.heads {
                h.size(frames);
            }
        }
        for s in [
            &mut self.cents,
            &mut self.spread,
            &mut self.window,
            &mut self.feedback,
            &mut self.mix,
        ] {
            s.set_time(self.rate, 30.0);
        }
    }
    fn reset(&mut self) {
        for s in [
            &mut self.cents,
            &mut self.spread,
            &mut self.window,
            &mut self.feedback,
            &mut self.mix,
        ] {
            s.snap(s.target);
        }
        for h in &mut self.heads {
            h.clear();
        }
        self.was = (f32::NAN, f32::NAN);
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
    fn tail_frames(&self) -> u32 {
        // The window, longer with feedback ringing on.
        let w = self.v[3] * 0.001 * self.rate as f32;
        let rings = 1.0 + 20.0 * self.v[4] / 100.0;
        (w * rings) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bhaskara_is_close_to_sine() {
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            assert!((sin_pi(x) - (std::f32::consts::PI * x).sin()).abs() < 0.002);
        }
    }
}
