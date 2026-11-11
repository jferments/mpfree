//! Comb: a tuned feedback comb filter. The input rings at a pitch and its
//! harmonics, like a plucked string or a metal tube; negative feedback
//! keeps only the odd harmonics, an octave lower and hollow. Notes sent to
//! it (with Key follow on) play the pitch.
//!
//! The loop is a delay of one period with a damping low-pass in it. The
//! fractional part of the delay is a first-order all-pass (Thiran), which
//! loses no top end however many times the sound goes round, and the
//! low-pass's own delay at the pitch is taken off the line, so the comb is
//! in tune from 20 Hz to 5 kHz. The input is scaled by `1 - |feedback|`,
//! so the peaks stay near unity gain at any feedback.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::Segs;
use crate::{defaults, flush};

/// [`Comb`] parameter ids.
pub mod ids {
    pub const PITCH: u16 = 0;
    pub const FEEDBACK: u16 = 1;
    pub const DAMPING: u16 = 2;
    pub const SPREAD: u16 = 3;
    pub const GLIDE: u16 = 4;
    pub const KEY_FOLLOW: u16 = 5;
    pub const MIX: u16 = 6;
}

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.comb",
    name: "Comb",
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

static PARAMS: [ParamInfo; 7] = [
    p(
        0,
        "Pitch",
        "Comb",
        20.0,
        5_000.0,
        220.0,
        Unit::Hz,
        Curve::Log,
    ),
    p(
        1,
        "Feedback",
        "Comb",
        -99.0,
        99.0,
        85.0,
        Unit::Percent,
        Curve::Linear,
    ),
    p(
        2,
        "Damping",
        "Comb",
        500.0,
        20_000.0,
        9_000.0,
        Unit::Hz,
        Curve::Log,
    ),
    p(
        3,
        "Spread",
        "Comb",
        0.0,
        100.0,
        8.0,
        Unit::Cents,
        Curve::Linear,
    ),
    p(4, "Glide", "Keys", 1.0, 2_000.0, 30.0, Unit::Ms, Curve::Log),
    p(
        5,
        "Key follow",
        "Keys",
        0.0,
        1.0,
        1.0,
        Unit::None,
        Curve::Toggle,
    ),
    p(
        6,
        "Mix",
        "Output",
        0.0,
        100.0,
        50.0,
        Unit::Percent,
        Curve::Linear,
    ),
];

/// The lowest pitch, Hz (sets the line length).
const LOWEST: f32 = 20.0;
/// Coefficients move every this many frames while the pitch glides.
const STEP: u32 = 16;

/// One channel's loop.
#[derive(Default)]
struct Loop {
    buf: Vec<f32>,
    mask: usize,
    w: usize,
    /// The all-pass's last output, the damping filter's state.
    last: f32,
    lp: f32,
    /// Whole delay, all-pass coefficient.
    d: usize,
    eta: f32,
}

impl Loop {
    fn new(frames: usize) -> Loop {
        let n = frames.next_power_of_two().max(16);
        Loop {
            buf: vec![0.0; n],
            mask: n - 1,
            ..Loop::default()
        }
    }
    /// Sets the loop's delay (frames, at least 1.5).
    fn tune(&mut self, delay: f32) {
        let delay = delay.clamp(1.5, (self.buf.len() - 4) as f32);
        let d = (delay - 0.5) as usize; // floor, delay > 0; d >= 1
        let frac = delay - d as f32; // 0.5 to 1.5
        self.d = d;
        self.eta = (1.0 - frac) / (1.0 + frac);
    }
    #[inline]
    fn at(&self, k: usize) -> f32 {
        self.buf[self.w.wrapping_sub(k) & self.mask]
    }
    /// One frame: `x` in (already scaled), `g` feedback, `a` the damping
    /// coefficient (the pole); returns the comb's output.
    #[inline]
    fn run(&mut self, x: f32, g: f32, a: f32) -> f32 {
        // at(0) is the frame before this one, so at(d - 1) is d frames back.
        let c = flush(self.at(self.d) + self.eta * (self.at(self.d - 1) - self.last));
        self.last = c;
        self.lp = flush(c + a * (self.lp - c));
        let y = x + g * self.lp;
        self.w = (self.w + 1) & self.mask;
        self.buf[self.w] = y;
        y
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
        self.last = 0.0;
        self.lp = 0.0;
    }
}

/// A one-pole low-pass's pole for a cutoff.
fn pole(hz: f32, rate: u32) -> f32 {
    let x = 2.0 * PI * hz.min(0.49 * rate as f32) / rate as f32;
    let n = 2.0 - x.cos();
    n - (n * n - 1.0).max(0.0).sqrt()
}

/// The one-pole low-pass's phase delay at `w` radians per frame, frames.
fn pole_delay(a: f32, w: f32) -> f32 {
    (a * w.sin()).atan2(1.0 - a * w.cos()) / w
}

/// A tuned feedback comb filter with damping, stereo spread and key
/// follow.
pub struct Comb {
    v: [f32; 7],
    rate: u32,
    /// The pitch, in octaves above 1 Hz, gliding.
    log_hz: Smooth,
    fb: Smooth,
    mix: Smooth,
    damp: f32,
    loops: [Loop; 2],
    count: u32,
    tuned_at: f32,
}

impl Default for Comb {
    fn default() -> Self {
        Self::new()
    }
}

impl Comb {
    /// Allocates for 48 kHz; [`Node::prepare`] reallocates for other rates.
    pub fn new() -> Comb {
        let v = defaults(&PARAMS);
        let mut c = Comb {
            v,
            rate: 0,
            log_hz: Smooth::new(v[0].log2()),
            fb: Smooth::new(v[1] / 100.0),
            mix: Smooth::new(v[6] / 100.0),
            damp: 0.0,
            loops: [Loop::default(), Loop::default()],
            count: 0,
            tuned_at: f32::NAN,
        };
        c.prepare(48_000, mpfree_node::MAX_BLOCK);
        c
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::PITCH => self.log_hz.target = v.log2(),
            ids::FEEDBACK => self.fb.target = v / 100.0,
            ids::DAMPING => {
                self.damp = pole(v, self.rate);
                self.tuned_at = f32::NAN;
            }
            ids::SPREAD => self.tuned_at = f32::NAN,
            ids::GLIDE => self.log_hz.set_time(self.rate, v),
            ids::MIX => self.mix.target = v / 100.0,
            _ => {}
        }
    }

    fn note(&mut self, note: u8) {
        if self.v[ids::KEY_FOLLOW as usize] >= 0.5 {
            let hz = 440.0 * ((note as f32 - 69.0) / 12.0).exp2();
            self.apply(ids::PITCH, hz);
        }
    }

    /// Tunes both loops for the pitch now (the right one spread up).
    fn tune(&mut self) {
        let lh = self.log_hz.value;
        self.tuned_at = lh;
        let spread = self.v[ids::SPREAD as usize] / 1200.0;
        for (c, l) in self.loops.iter_mut().enumerate() {
            let hz = (lh + if c == 1 { spread } else { 0.0 }).exp2();
            let w = 2.0 * PI * hz / self.rate as f32;
            let period = self.rate as f32 / hz;
            l.tune(period - pole_delay(self.damp, w));
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            self.log_hz.next();
            if self.log_hz.value != self.tuned_at && self.count == 0 {
                self.tune();
            }
            self.count = (self.count + 1) % STEP;
            let g = self.fb.next();
            let mix = self.mix.next();
            let gain = 1.0 - g.abs();
            let a = self.damp;
            for (c, x) in [&mut l[i], &mut r[i]].into_iter().enumerate() {
                let y = self.loops[c].run(gain * *x, g, a);
                *x += mix * (y - *x);
            }
        }
    }

    /// The comb's gain at `hz`, dB, at its settings now (for drawing).
    pub fn response_db(&self, hz: f32) -> f32 {
        let f0 = self.v[ids::PITCH as usize];
        let g = self.v[ids::FEEDBACK as usize] / 100.0;
        let rate = self.rate as f32;
        let a = self.damp;
        let w = 2.0 * PI * hz / rate;
        let period = rate / f0;
        let d = period - pole_delay(a, 2.0 * PI * f0 / rate);
        // H = (1 - |g|) / (1 - g * L(w) * e^{-jwd}), L the one-pole.
        let (lr, li) = {
            let (c, s) = (w.cos(), w.sin());
            let den = (1.0 - a * c).powi(2) + (a * s).powi(2);
            ((1.0 - a) * (1.0 - a * c) / den, -(1.0 - a) * a * s / den)
        };
        let (c, s) = ((w * d).cos(), -(w * d).sin());
        let (pr, pi) = (lr * c - li * s, lr * s + li * c);
        let (dr, di) = (1.0 - g * pr, -g * pi);
        let mag = (1.0 - g.abs()) / (dr * dr + di * di).sqrt().max(1e-9);
        20.0 * mag.max(1e-9).log10()
    }
}

impl Node for Comb {
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
            let n = (rate as f32 / LOWEST) as usize + 8;
            self.loops = [Loop::new(n), Loop::new(n)];
        }
        self.log_hz.set_time(rate, self.v[ids::GLIDE as usize]);
        self.fb.set_time(rate, 20.0);
        self.mix.set_time(rate, 20.0);
        self.damp = pole(self.v[ids::DAMPING as usize], rate);
        self.reset();
    }
    fn reset(&mut self) {
        self.log_hz.snap(self.log_hz.target);
        self.fb.snap(self.fb.target);
        self.mix.snap(self.mix.target);
        for l in &mut self.loops {
            l.clear();
        }
        self.count = 0;
        self.tune();
    }
    fn process(&mut self, _ctx: &Ctx, events: &[Timed], left: &mut [f32], right: &mut [f32]) {
        let len = left.len().min(right.len());
        let mut segs = Segs::new(events);
        while let Some((from, to)) = segs.next(len, |ev| match *ev {
            NodeEvent::Param { id, value } => self.apply(id, value),
            NodeEvent::NoteOn { note, velocity } if velocity > 0 => self.note(note),
            _ => {}
        }) {
            self.run(&mut left[from..to], &mut right[from..to]);
        }
    }
    fn tail_frames(&self) -> u32 {
        // The ring falls by |g| (and the damping) every period: to -80 dB.
        let g = self.fb.target.abs().clamp(0.01, 0.99);
        let periods = (-80.0 / (20.0 * g.log10())).ceil();
        let period = self.rate as f32 / self.log_hz.target.exp2();
        (periods * period).min(30.0 * self.rate as f32) as u32
    }
}
