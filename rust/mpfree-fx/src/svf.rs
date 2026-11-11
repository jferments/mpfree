//! Zero-delay-feedback (trapezoidal) state-variable filters, after Andrew
//! Simper's "Linear Trapezoidal Integrated SVF" (Cytomic, 2013): stable at
//! any cutoff below Nyquist and under fast modulation, so a sweep never
//! blows up or zips. Coefficients are recomputed every frame while a cutoff
//! or resonance is moving and cached once it settles.

use std::f32::consts::PI;

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::{db_to_gain, defaults, flush, soft};

/// What a [`Svf`] outputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SvfMode {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    /// A bell (peaking EQ) with the gain given to [`Svf::set`].
    Bell,
    LowShelf,
    HighShelf,
}

/// One stereo state-variable filter stage (12 dB per octave).
#[derive(Clone, Copy, Debug)]
pub struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    m0: f32,
    m1: f32,
    m2: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
}

impl Default for Svf {
    fn default() -> Svf {
        let mut s = Svf {
            a1: 0.0,
            a2: 0.0,
            a3: 0.0,
            m0: 0.0,
            m1: 0.0,
            m2: 1.0,
            ic1: [0.0; 2],
            ic2: [0.0; 2],
        };
        s.set(SvfMode::LowPass, 1000.0, 0.707, 0.0, 48_000);
        s
    }
}

impl Svf {
    /// Sets the response. `q` is the resonance (0.707 is flat for
    /// low-pass), `gain_db` is used by the bell and shelves. The cutoff is
    /// kept between 5 Hz and 45 % of the sample rate.
    pub fn set(&mut self, mode: SvfMode, hz: f32, q: f32, gain_db: f32, rate: u32) {
        let fs = rate as f32;
        let hz = hz.clamp(5.0, 0.45 * fs);
        let q = q.max(0.05);
        let t = (PI * hz / fs).tan();
        let a = db_to_gain(gain_db * 0.5); // sqrt of the linear gain
        let (g, k, m0, m1, m2) = match mode {
            SvfMode::LowPass => (t, 1.0 / q, 0.0, 0.0, 1.0),
            SvfMode::HighPass => (t, 1.0 / q, 1.0, -1.0 / q, -1.0),
            SvfMode::BandPass => (t, 1.0 / q, 0.0, 1.0, 0.0),
            SvfMode::Notch => (t, 1.0 / q, 1.0, -1.0 / q, 0.0),
            SvfMode::Bell => {
                let k = 1.0 / (q * a);
                (t, k, 1.0, k * (a * a - 1.0), 0.0)
            }
            SvfMode::LowShelf => {
                let k = 1.0 / q;
                (t / a.sqrt(), k, 1.0, k * (a - 1.0), a * a - 1.0)
            }
            SvfMode::HighShelf => {
                let k = 1.0 / q;
                (t * a.sqrt(), k, a * a, k * (1.0 - a) * a, 1.0 - a * a)
            }
        };
        self.a1 = 1.0 / (1.0 + g * (g + k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
        self.m0 = m0;
        self.m1 = m1;
        self.m2 = m2;
    }

    /// Filters one sample of channel `ch` (0 or 1).
    #[inline]
    pub fn run(&mut self, ch: usize, v0: f32) -> f32 {
        let (ic1, ic2) = (self.ic1[ch], self.ic2[ch]);
        let v3 = v0 - ic2;
        let v1 = self.a1 * ic1 + self.a2 * v3;
        let v2 = ic2 + self.a2 * ic1 + self.a3 * v3;
        self.ic1[ch] = flush(2.0 * v1 - ic1);
        self.ic2[ch] = flush(2.0 * v2 - ic2);
        self.m0 * v0 + self.m1 * v1 + self.m2 * v2
    }

    pub fn reset(&mut self) {
        self.ic1 = [0.0; 2];
        self.ic2 = [0.0; 2];
    }

    /// The steady-state response at `hz` as a complex gain `(re, im)`, for
    /// drawing curves. Exact: the trapezoidal filter is the bilinear
    /// transform of its analog prototype, so at `hz` the prototype sees
    /// `s = j tan(pi hz / rate) / g`.
    pub fn response(&self, hz: f32, rate: u32) -> (f64, f64) {
        let (a1, a2) = (f64::from(self.a1), f64::from(self.a2));
        let g = a2 / a1;
        let k = (1.0 / a1 - 1.0) / g - g;
        let w = (std::f64::consts::PI * f64::from(hz) / f64::from(rate)).tan() / g;
        // H = m0 + (m2 + j m1 w) / (1 - w^2 + j k w)
        let (dr, di) = (1.0 - w * w, k * w);
        let (nr, ni) = (f64::from(self.m2), f64::from(self.m1) * w);
        let den = dr * dr + di * di;
        (
            f64::from(self.m0) + (nr * dr + ni * di) / den,
            (ni * dr - nr * di) / den,
        )
    }
}

/// Resonance 0 to 1 as a filter Q: 0 is flat (0.707), 1 is 40 (rings
/// long, close to self-oscillation, but never unstable).
#[inline]
fn res_to_q(res: f32) -> f32 {
    0.707 * (40.0f32 / 0.707).powf(res.clamp(0.0, 1.0))
}

const TYPES: &[&str] = &[
    "Low-pass",
    "High-pass",
    "Band-pass",
    "Notch",
    "Bell",
    "Low shelf",
    "High shelf",
];
const SLOPES: &[&str] = &["12 dB", "24 dB"];

/// [`MultiFilter`] parameter ids.
pub mod multi {
    pub const TYPE: u16 = 0;
    pub const SLOPE: u16 = 1;
    pub const CUTOFF: u16 = 2;
    pub const RESONANCE: u16 = 3;
    pub const GAIN: u16 = 4;
    pub const DRIVE: u16 = 5;
}

static MULTI_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.filter",
    name: "Multi filter",
    kind: Kind::Effect,
};
static MULTI_PARAMS: [ParamInfo; 6] = [
    ParamInfo {
        id: 0,
        name: "Type",
        group: "Filter",
        min: 0.0,
        max: 6.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Slope",
        group: "Filter",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SLOPES),
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Cutoff",
        group: "Filter",
        min: 20.0,
        max: 20_000.0,
        default: 20_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Resonance",
        group: "Filter",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Gain",
        group: "Filter",
        min: -24.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Drive",
        group: "Filter",
        min: 0.0,
        max: 24.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// Sets a [`MultiFilter`]'s one or two stages.
#[allow(clippy::too_many_arguments)]
fn set_stages(
    a: &mut Svf,
    b: &mut Svf,
    mode: SvfMode,
    cascaded: bool,
    hz: f32,
    res: f32,
    gain: f32,
    rate: u32,
) {
    let q = res_to_q(res);
    if cascaded {
        // 24 dB: a Butterworth-shaped first stage, resonance on the second.
        let q1 = if matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
            0.541
        } else {
            q
        };
        let q2 = if matches!(mode, SvfMode::LowPass | SvfMode::HighPass) {
            1.307 * (q / 0.707)
        } else {
            q
        };
        a.set(mode, hz, q1, gain, rate);
        b.set(mode, hz, q2, gain, rate);
    } else {
        a.set(mode, hz, q, gain, rate);
    }
}

/// The product of two complex gains.
pub(crate) fn cmul(a: (f64, f64), b: (f64, f64)) -> (f64, f64) {
    (a.0 * b.0 - a.1 * b.1, a.0 * b.1 + a.1 * b.0)
}

/// A complex gain's level in dB (-240 dB for silence).
pub(crate) fn power_db(h: (f64, f64)) -> f32 {
    (10.0 * (h.0 * h.0 + h.1 * h.1).max(1e-24).log10()) as f32
}

/// A multi-mode filter: low-pass, high-pass, band-pass and notch at 12 or
/// 24 dB per octave, plus a bell and two shelves; resonance up to a long
/// ring; optional drive into a soft saturator before the filter
/// (deliberate distortion, off at 0 dB).
pub struct MultiFilter {
    v: [f32; 6],
    rate: u32,
    log_hz: Smooth,
    res: Smooth,
    gain: Smooth,
    drive: Smooth,
    a: Svf,
    b: Svf,
}

impl Default for MultiFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiFilter {
    pub fn new() -> MultiFilter {
        let v = defaults(&MULTI_PARAMS);
        let mut f = MultiFilter {
            v,
            rate: 48_000,
            log_hz: Smooth::new(v[2].log2()),
            res: Smooth::new(v[3] / 100.0),
            gain: Smooth::new(v[4]),
            drive: Smooth::new(v[5]),
            a: Svf::default(),
            b: Svf::default(),
        };
        f.prepare(48_000, mpfree_node::MAX_BLOCK);
        f
    }

    fn mode(&self) -> SvfMode {
        match self.v[0] as usize {
            1 => SvfMode::HighPass,
            2 => SvfMode::BandPass,
            3 => SvfMode::Notch,
            4 => SvfMode::Bell,
            5 => SvfMode::LowShelf,
            6 => SvfMode::HighShelf,
            _ => SvfMode::LowPass,
        }
    }

    fn cascaded(&self) -> bool {
        self.v[1] >= 0.5 && (self.v[0] as usize) < 4
    }

    fn update(&mut self) {
        let (mode, cascaded) = (self.mode(), self.cascaded());
        let hz = self.log_hz.value.exp2();
        let (res, gain, rate) = (self.res.value, self.gain.value, self.rate);
        set_stages(
            &mut self.a,
            &mut self.b,
            mode,
            cascaded,
            hz,
            res,
            gain,
            rate,
        );
    }

    /// The response in dB at `hz` for the settings the filter is heading
    /// to, for drawing. Exact to the filter; the drive is left out (it
    /// colours by level, it is not a curve).
    pub fn response_db(&self, hz: f32) -> f32 {
        let (mut a, mut b) = (Svf::default(), Svf::default());
        let (mode, cascaded) = (self.mode(), self.cascaded());
        let (cut, res, gain) = (self.v[2], self.v[3] / 100.0, self.v[4]);
        set_stages(&mut a, &mut b, mode, cascaded, cut, res, gain, self.rate);
        let mut h = a.response(hz, self.rate);
        if cascaded {
            h = cmul(h, b.response(hz, self.rate));
        }
        power_db(h)
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = MULTI_PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        let was_cascaded = self.cascaded();
        self.v[id as usize] = v;
        match id {
            multi::CUTOFF => self.log_hz.target = v.log2(),
            multi::RESONANCE => self.res.target = v / 100.0,
            multi::GAIN => self.gain.target = v,
            multi::DRIVE => self.drive.target = v,
            _ => {
                // Type or slope: new coefficients now; a second stage that
                // was idle starts from silence.
                if self.cascaded() && !was_cascaded {
                    self.b.reset();
                }
                self.update();
            }
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let cascaded = self.cascaded();
        for i in 0..l.len() {
            let moving = !(self.log_hz.settled() && self.res.settled() && self.gain.settled());
            if moving {
                self.log_hz.next();
                self.res.next();
                self.gain.next();
                self.update();
            }
            let d = self.drive.next();
            let (mut x, mut y) = (l[i], r[i]);
            if d > 0.0 {
                // Deliberate: blend into the saturator over the first 3 dB.
                let g = db_to_gain(d);
                let amt = (d / 3.0).min(1.0);
                x += amt * (soft(x * g) - x);
                y += amt * (soft(y * g) - y);
            }
            x = self.a.run(0, x);
            y = self.a.run(1, y);
            if cascaded {
                x = self.b.run(0, x);
                y = self.b.run(1, y);
            }
            l[i] = x;
            r[i] = y;
        }
    }
}

impl Node for MultiFilter {
    fn info(&self) -> &'static NodeInfo {
        &MULTI_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &MULTI_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        self.log_hz.set_time(self.rate, 10.0);
        self.res.set_time(self.rate, 10.0);
        self.gain.set_time(self.rate, 10.0);
        self.drive.set_time(self.rate, 10.0);
        self.update();
    }
    fn reset(&mut self) {
        self.log_hz.snap(self.log_hz.target);
        self.res.snap(self.res.target);
        self.gain.snap(self.gain.target);
        self.drive.snap(self.drive.target);
        self.a.reset();
        self.b.reset();
        self.update();
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
        // A high-Q ring: about Q / (pi * f) seconds to fall 27 dB; allow 4x.
        let q = res_to_q(self.res.target);
        let hz = self.log_hz.target.exp2().max(20.0);
        (4.0 * q / (PI * hz) * self.rate as f32) as u32
    }
}

/// Walks a block's events and the pieces between them without borrowing
/// the node twice (the closure form of `mpfree_node::split` cannot call
/// `&mut self` from both closures). Same order as `split`: events at an offset apply
/// before the frames from that offset.
pub(crate) struct Segs<'e> {
    events: &'e [Timed],
    i: usize,
    at: usize,
    done: bool,
}

impl<'e> Segs<'e> {
    pub(crate) fn new(events: &'e [Timed]) -> Segs<'e> {
        Segs {
            events,
            i: 0,
            at: 0,
            done: false,
        }
    }

    /// Applies the events due at the current frame, then returns the next
    /// piece `(from, to)`, or `None` at the end (after applying any events
    /// at or past the end, so nothing is lost).
    pub(crate) fn next<F: FnMut(&NodeEvent)>(
        &mut self,
        len: usize,
        mut apply: F,
    ) -> Option<(usize, usize)> {
        if self.done {
            return None;
        }
        while self.at < len {
            while self.i < self.events.len() && (self.events[self.i].offset as usize) <= self.at {
                apply(&self.events[self.i].ev);
                self.i += 1;
            }
            let next = self
                .events
                .get(self.i)
                .map_or(len, |e| (e.offset as usize).min(len));
            let from = self.at;
            self.at = next;
            if next > from {
                return Some((from, next));
            }
        }
        for e in &self.events[self.i..] {
            apply(&e.ev);
        }
        self.i = self.events.len();
        self.done = true;
        None
    }
}

/// [`DjFilter`] parameter ids.
pub mod dj {
    pub const POSITION: u16 = 0;
    pub const RESONANCE: u16 = 1;
}

static DJ_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.dj_filter",
    name: "DJ filter",
    kind: Kind::Effect,
};
static DJ_PARAMS: [ParamInfo; 2] = [
    ParamInfo {
        id: 0,
        name: "Position",
        group: "DJ filter",
        min: -100.0,
        max: 100.0,
        default: 0.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Resonance",
        group: "DJ filter",
        min: 0.0,
        max: 100.0,
        default: 30.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// The DJ filter's corner at `pos` (-1 to 1): a low-pass from 20 kHz down
/// to 80 Hz to the left, a high-pass from 20 Hz up to 8 kHz to the right.
fn dj_corner(pos: f32) -> f32 {
    let a = pos.abs();
    if pos < 0.0 {
        20_000.0 * (80.0f32 / 20_000.0).powf(a)
    } else {
        20.0 * (8_000.0f32 / 20.0).powf(a)
    }
}

/// Sets the DJ filter's stage for `pos` and `res` (both -1 or 0 to 1).
fn dj_set(f: &mut Svf, pos: f32, res: f32, rate: u32) {
    let mode = if pos < 0.0 {
        SvfMode::LowPass
    } else {
        SvfMode::HighPass
    };
    f.set(mode, dj_corner(pos), 0.707 + res * 6.0, 0.0, rate);
}

/// The one-knob DJ filter: position below 0 is a low-pass falling from
/// 20 kHz to 80 Hz, above 0 a high-pass rising from 20 Hz to 8 kHz, and the
/// middle is off. The filter fades in over the first 10 % each side, so the
/// knob can cross the middle without a click.
pub struct DjFilter {
    v: [f32; 2],
    rate: u32,
    pos: Smooth,
    res: Smooth,
    f: Svf,
    last_pos: f32,
    last_res: f32,
}

impl Default for DjFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl DjFilter {
    pub fn new() -> DjFilter {
        let v = defaults(&DJ_PARAMS);
        let mut d = DjFilter {
            v,
            rate: 48_000,
            pos: Smooth::new(0.0),
            res: Smooth::new(v[1] / 100.0),
            f: Svf::default(),
            last_pos: f32::NAN,
            last_res: f32::NAN,
        };
        d.prepare(48_000, mpfree_node::MAX_BLOCK);
        d
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = DJ_PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            dj::POSITION => self.pos.target = v / 100.0,
            _ => self.res.target = v / 100.0,
        }
    }

    fn coefs(&mut self, pos: f32, res: f32) {
        if pos == self.last_pos && res == self.last_res {
            return;
        }
        self.last_pos = pos;
        self.last_res = res;
        dj_set(&mut self.f, pos, res, self.rate);
    }

    /// The response in dB at `hz` for where the knob is heading, for
    /// drawing (exact, the fade-in near the middle included).
    pub fn response_db(&self, hz: f32) -> f32 {
        let (pos, res) = (self.v[0] / 100.0, self.v[1] / 100.0);
        let wet = f64::from((pos.abs() / 0.1).min(1.0));
        if wet == 0.0 {
            return 0.0;
        }
        let mut f = Svf::default();
        dj_set(&mut f, pos, res, self.rate);
        let h = f.response(hz, self.rate);
        power_db((1.0 - wet + wet * h.0, wet * h.1))
    }

    /// The filter's corner for where the knob is heading: `None` in the
    /// middle, else whether it is the low-pass, and its corner in Hz.
    pub fn corner(&self) -> Option<(bool, f32)> {
        let pos = self.v[0] / 100.0;
        if pos == 0.0 {
            return None;
        }
        Some((pos < 0.0, dj_corner(pos)))
    }

    /// One stereo frame (Perform FX runs the filter inside its own loop).
    #[inline]
    pub(crate) fn frame(&mut self, x: f32, y: f32) -> (f32, f32) {
        let p = self.pos.next();
        let q = self.res.next();
        let wet = (p.abs() / 0.1).min(1.0);
        if wet == 0.0 && self.pos.settled() {
            // Off: keep the filter's state quiet so a later turn starts clean.
            self.f.reset();
            return (x, y);
        }
        self.coefs(p, q);
        let fx = self.f.run(0, x);
        let fy = self.f.run(1, y);
        (crate::lerp(x, fx, wet), crate::lerp(y, fy, wet))
    }

    /// Off and staying off: the knob is in the middle and not moving.
    pub(crate) fn resting(&self) -> bool {
        self.pos.value == 0.0 && self.pos.settled()
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            (l[i], r[i]) = self.frame(l[i], r[i]);
        }
    }
}

impl Node for DjFilter {
    fn info(&self) -> &'static NodeInfo {
        &DJ_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &DJ_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        self.pos.set_time(self.rate, 15.0);
        self.res.set_time(self.rate, 15.0);
        self.last_pos = f32::NAN;
    }
    fn reset(&mut self) {
        self.pos.snap(self.pos.target);
        self.res.snap(self.res.target);
        self.f.reset();
        self.last_pos = f32::NAN;
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
        self.rate / 10
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gain for a sine, from RMS over the second half (a sampled peak can
    /// miss the true peak at high frequencies).
    fn sine_gain(f: &mut Svf, hz: f32, rate: u32) -> f32 {
        f.reset();
        let n = rate as usize;
        let (mut sx, mut sy) = (0.0f64, 0.0f64);
        for i in 0..n {
            let x = (2.0 * PI * hz * i as f32 / rate as f32).sin();
            let y = f.run(0, x);
            if i >= n / 2 {
                sx += (x * x) as f64;
                sy += (y * y) as f64;
            }
        }
        (sy / sx).sqrt() as f32
    }

    #[test]
    fn low_pass_and_high_pass_slopes() {
        let mut f = Svf::default();
        f.set(SvfMode::LowPass, 1000.0, 0.707, 0.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 100.0, 48_000))).abs() < 0.2);
        let at10k = crate::gain_to_db(sine_gain(&mut f, 10_000.0, 48_000));
        assert!(at10k < -38.0 && at10k > -44.0, "{at10k}");
        f.set(SvfMode::HighPass, 1000.0, 0.707, 0.0, 48_000);
        assert!(crate::gain_to_db(sine_gain(&mut f, 10_000.0, 48_000)).abs() < 0.2);
        assert!(crate::gain_to_db(sine_gain(&mut f, 100.0, 48_000)) < -38.0);
    }

    #[test]
    fn bell_and_shelves_hit_their_gain() {
        let mut f = Svf::default();
        f.set(SvfMode::Bell, 1000.0, 1.0, 6.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 1000.0, 48_000)) - 6.0).abs() < 0.2);
        f.set(SvfMode::LowShelf, 300.0, 0.707, -9.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 30.0, 48_000)) + 9.0).abs() < 0.3);
        assert!(crate::gain_to_db(sine_gain(&mut f, 8000.0, 48_000)).abs() < 0.2);
        f.set(SvfMode::HighShelf, 3000.0, 0.707, 4.0, 48_000);
        assert!((crate::gain_to_db(sine_gain(&mut f, 15_000.0, 48_000)) - 4.0).abs() < 0.3);
    }
}
