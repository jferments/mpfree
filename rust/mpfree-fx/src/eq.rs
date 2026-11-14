//! A four-band parametric EQ with low and high cuts, built on the same
//! zero-delay-feedback state-variable filters as the Multi filter, so a
//! band swept or boosted under automation never zips or blows up.
//!
//! - Low cut and high cut: Off, 12 or 24 dB per octave (Butterworth). A
//!   slope change crossfades over about 20 ms.
//! - Four bands, each a bell, a low shelf or a high shelf, with frequency,
//!   gain (±18 dB) and Q. A band at 0 dB is exactly flat. A shelf's Q stops
//!   at 2, where it rings a 5 dB bump past the corner at full gain. A type
//!   change glides the band's gain to 0 dB, where every type passes the
//!   input exactly, swaps, and glides back, so it never clicks.
//! - Output gain.
//!
//! [`Eq::response_db`] gives the curve the settings are heading to, exact
//! to the filters, for drawing on an EQ page.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
use crate::{db_to_gain, defaults};

/// [`Eq`] parameter ids. Each band has four in a row: `band_type(b)`,
/// `freq(b)`, `gain(b)` and `q(b)` for band `b` from 0 to 3.
pub mod ids {
    pub const LOW_CUT: u16 = 0;
    pub const LOW_SLOPE: u16 = 1;
    pub const HIGH_CUT: u16 = 2;
    pub const HIGH_SLOPE: u16 = 3;
    /// How many bands.
    pub const BANDS: usize = 4;
    pub const fn band_type(b: usize) -> u16 {
        4 + 4 * b as u16
    }
    pub const fn freq(b: usize) -> u16 {
        5 + 4 * b as u16
    }
    pub const fn gain(b: usize) -> u16 {
        6 + 4 * b as u16
    }
    pub const fn q(b: usize) -> u16 {
        7 + 4 * b as u16
    }
    pub const OUTPUT: u16 = 20;
}

const SLOPES: &[&str] = &["Off", "12 dB", "24 dB"];
const TYPES: &[&str] = &["Bell", "Low shelf", "High shelf"];
const N: usize = 21;

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.eq",
    name: "EQ",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; N] = [
    ParamInfo {
        id: 0,
        name: "Low cut",
        group: "Cuts",
        min: 20.0,
        max: 2_000.0,
        default: 80.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Low slope",
        group: "Cuts",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SLOPES),
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "High cut",
        group: "Cuts",
        min: 1_000.0,
        max: 20_000.0,
        default: 12_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "High slope",
        group: "Cuts",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(SLOPES),
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Type",
        group: "Band 1",
        min: 0.0,
        max: 2.0,
        default: 1.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Freq",
        group: "Band 1",
        min: 20.0,
        max: 20_000.0,
        default: 100.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Gain",
        group: "Band 1",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Q",
        group: "Band 1",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 8,
        name: "Type",
        group: "Band 2",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 9,
        name: "Freq",
        group: "Band 2",
        min: 20.0,
        max: 20_000.0,
        default: 400.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 10,
        name: "Gain",
        group: "Band 2",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 11,
        name: "Q",
        group: "Band 2",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 12,
        name: "Type",
        group: "Band 3",
        min: 0.0,
        max: 2.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 13,
        name: "Freq",
        group: "Band 3",
        min: 20.0,
        max: 20_000.0,
        default: 2_500.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 14,
        name: "Gain",
        group: "Band 3",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 15,
        name: "Q",
        group: "Band 3",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 16,
        name: "Type",
        group: "Band 4",
        min: 0.0,
        max: 2.0,
        default: 2.0,
        unit: Unit::None,
        curve: Curve::Choice(TYPES),
        readonly: false,
    },
    ParamInfo {
        id: 17,
        name: "Freq",
        group: "Band 4",
        min: 20.0,
        max: 20_000.0,
        default: 8_000.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 18,
        name: "Gain",
        group: "Band 4",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 19,
        name: "Q",
        group: "Band 4",
        min: 0.1,
        max: 18.0,
        default: 0.71,
        unit: Unit::None,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 20,
        name: "Output",
        group: "Output",
        min: -18.0,
        max: 18.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// Butterworth Qs: one 12 dB stage, or two stages for 24 dB.
const Q12: f32 = std::f32::consts::FRAC_1_SQRT_2;
const Q24: [f32; 2] = [0.541_196_1, 1.306_563];
/// The most Q a shelf takes: past this its resonant bump grows fast (a
/// +18 dB shelf at Q 18 peaks at +42 dB).
const SHELF_Q: f32 = 2.0;

/// The Q a band of type `kind` uses for the Q knob's `q`.
fn band_q(kind: usize, q: f32) -> f32 {
    if kind == 0 {
        q
    } else {
        q.min(SHELF_Q)
    }
}

fn band_mode(kind: usize) -> SvfMode {
    match kind {
        1 => SvfMode::LowShelf,
        2 => SvfMode::HighShelf,
        _ => SvfMode::Bell,
    }
}

/// One parametric band.
#[derive(Clone, Copy)]
struct Band {
    log_hz: Smooth,
    gain: Smooth,
    log_q: Smooth,
    /// The type playing and the type asked for; while they differ the gain
    /// glides to 0 dB (`amt` to 0), then the type swaps.
    kind: usize,
    want: usize,
    amt: f32,
    /// At 0 dB the band is exactly the input: it rests, its state cleared.
    flat: bool,
    svf: Svf,
}

impl Band {
    fn new(kind: usize, hz: f32, gain: f32, q: f32) -> Band {
        Band {
            log_hz: Smooth::new(hz.log2()),
            gain: Smooth::new(gain),
            log_q: Smooth::new(q.log2()),
            kind,
            want: kind,
            amt: 1.0,
            flat: gain == 0.0,
            svf: Svf::default(),
        }
    }
    fn update(&mut self, rate: u32) {
        // An S-curve on the type-swap glide, so the gain eases in and out
        // instead of turning a corner.
        let ease = self.amt * self.amt * (3.0 - 2.0 * self.amt);
        let db = self.gain.value * ease;
        self.svf.set(
            band_mode(self.kind),
            self.log_hz.value.exp2(),
            band_q(self.kind, self.log_q.value.exp2()),
            db,
            rate,
        );
        // Waking from rest it starts from silence, which only the gain,
        // gliding up from nothing, lets through.
        self.flat = db == 0.0;
        if self.flat {
            self.svf.reset();
        }
    }
    fn moving(&self) -> bool {
        !(self.log_hz.settled()
            && self.gain.settled()
            && self.log_q.settled()
            && self.amt == 1.0
            && self.kind == self.want)
    }
    /// One frame of glide: the smoothers, then the type swap.
    fn advance(&mut self, step: f32) {
        self.log_hz.next();
        self.gain.next();
        self.log_q.next();
        if self.kind != self.want {
            self.amt -= step;
            if self.amt <= 0.0 {
                // At 0 dB every type passes the input exactly, so the swap
                // is silent; whatever the old type left in the state fades
                // while the gain glides back up from nothing.
                self.amt = 0.0;
                self.kind = self.want;
            }
        } else if self.amt < 1.0 {
            self.amt = (self.amt + step).min(1.0);
        }
    }
    fn snap(&mut self) {
        self.log_hz.snap(self.log_hz.target);
        self.gain.snap(self.gain.target);
        self.log_q.snap(self.log_q.target);
        self.kind = self.want;
        self.amt = 1.0;
        self.svf.reset();
    }
}

/// A low or high cut: the output crossfades between the input and the 12
/// and 24 dB filters by weight. A filter at weight 0 rests with its state
/// cleared.
#[derive(Clone, Copy)]
struct Cut {
    mode: SvfMode,
    log_hz: Smooth,
    w12: Smooth,
    w24: Smooth,
    one: Svf,
    two: [Svf; 2],
}

impl Cut {
    fn new(mode: SvfMode, hz: f32) -> Cut {
        Cut {
            mode,
            log_hz: Smooth::new(hz.log2()),
            w12: Smooth::new(0.0),
            w24: Smooth::new(0.0),
            one: Svf::default(),
            two: [Svf::default(); 2],
        }
    }
    fn slope(&mut self, s: usize) {
        self.w12.target = if s == 1 { 1.0 } else { 0.0 };
        self.w24.target = if s == 2 { 1.0 } else { 0.0 };
    }
    fn update(&mut self, rate: u32) {
        let hz = self.log_hz.value.exp2();
        self.one.set(self.mode, hz, Q12, 0.0, rate);
        self.two[0].set(self.mode, hz, Q24[0], 0.0, rate);
        self.two[1].set(self.mode, hz, Q24[1], 0.0, rate);
    }
    /// Steps the weights one frame and clears a filter that rests.
    #[inline]
    fn weights(&mut self) -> (f32, f32) {
        let (w12, w24) = (self.w12.next(), self.w24.next());
        if w12 == 0.0 {
            self.one.reset();
        }
        if w24 == 0.0 {
            self.two[0].reset();
            self.two[1].reset();
        }
        (w12, w24)
    }
    #[inline]
    fn run(&mut self, ch: usize, x: f32, w12: f32, w24: f32) -> f32 {
        let mut y = (1.0 - w12 - w24) * x;
        if w12 != 0.0 {
            y += w12 * self.one.run(ch, x);
        }
        if w24 != 0.0 {
            let half = self.two[0].run(ch, x);
            y += w24 * self.two[1].run(ch, half);
        }
        y
    }
    fn snap(&mut self) {
        self.log_hz.snap(self.log_hz.target);
        self.w12.snap(self.w12.target);
        self.w24.snap(self.w24.target);
        self.one.reset();
        self.two[0].reset();
        self.two[1].reset();
    }
}

/// A four-band parametric EQ with low and high cuts and an output gain.
pub struct Eq {
    v: [f32; N],
    rate: u32,
    low: Cut,
    high: Cut,
    bands: [Band; ids::BANDS],
    out: Smooth,
    /// The type-swap glide per frame (10 ms each way).
    step: f32,
}

impl Default for Eq {
    fn default() -> Self {
        Self::new()
    }
}

impl Eq {
    pub fn new() -> Eq {
        let v = defaults(&PARAMS);
        let band = |b: usize| {
            Band::new(
                v[ids::band_type(b) as usize] as usize,
                v[ids::freq(b) as usize],
                v[ids::gain(b) as usize],
                v[ids::q(b) as usize],
            )
        };
        let mut eq = Eq {
            v,
            rate: 48_000,
            low: Cut::new(SvfMode::HighPass, v[0]),
            high: Cut::new(SvfMode::LowPass, v[2]),
            bands: [band(0), band(1), band(2), band(3)],
            out: Smooth::new(db_to_gain(v[20])),
            step: 0.0,
        };
        eq.low.slope(v[1] as usize);
        eq.high.slope(v[3] as usize);
        eq.prepare(48_000, mpfree_node::MAX_BLOCK);
        eq.reset();
        eq
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = PARAMS.get(id as usize) else {
            return;
        };
        let v = p.clamp(value);
        self.v[id as usize] = v;
        match id {
            ids::LOW_CUT => self.low.log_hz.target = v.log2(),
            ids::LOW_SLOPE => self.low.slope(v as usize),
            ids::HIGH_CUT => self.high.log_hz.target = v.log2(),
            ids::HIGH_SLOPE => self.high.slope(v as usize),
            ids::OUTPUT => self.out.target = db_to_gain(v),
            _ => {
                let b = &mut self.bands[(id as usize - 4) / 4];
                match (id - 4) % 4 {
                    0 => b.want = v as usize,
                    1 => b.log_hz.target = v.log2(),
                    2 => b.gain.target = v,
                    _ => b.log_q.target = v.log2(),
                }
            }
        }
    }

    fn update_all(&mut self) {
        self.low.update(self.rate);
        self.high.update(self.rate);
        for b in &mut self.bands {
            b.update(self.rate);
        }
    }

    /// Nothing glides: every smoother has reached its target and no band
    /// is changing type, so a frame needs no per-sample upkeep.
    fn still(&self) -> bool {
        let cut = |c: &Cut| c.log_hz.settled() && c.w12.settled() && c.w24.settled();
        cut(&self.low)
            && cut(&self.high)
            && self.out.settled()
            && self.bands.iter().all(|b| !b.moving())
    }

    /// [`Eq::run`] while [`Eq::still`]: the same output, sample for sample,
    /// without stepping settled smoothers (which would return their targets)
    /// or clearing filters that rest (which are already clear). With the
    /// cuts off, every band flat and the output at 0 dB, the EQ is exactly
    /// the input and the block is left as it is.
    fn run_still(&mut self, l: &mut [f32], r: &mut [f32]) {
        let (lo12, lo24) = (self.low.w12.value, self.low.w24.value);
        let (hi12, hi24) = (self.high.w12.value, self.high.w24.value);
        let g = self.out.value;
        if lo12 == 0.0
            && lo24 == 0.0
            && hi12 == 0.0
            && hi24 == 0.0
            && g == 1.0
            && self.bands.iter().all(|b| b.flat)
        {
            return;
        }
        for i in 0..l.len() {
            let mut y = [l[i], r[i]];
            for (ch, x) in y.iter_mut().enumerate() {
                let mut s = self.low.run(ch, *x, lo12, lo24);
                s = self.high.run(ch, s, hi12, hi24);
                for b in &mut self.bands {
                    if !b.flat {
                        s = b.svf.run(ch, s);
                    }
                }
                *x = g * s;
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        if self.still() {
            self.run_still(l, r)
        } else {
            self.run_gliding(l, r)
        }
    }

    /// The general frame loop: smoothers, type swaps and filter updates
    /// per sample, then the filters.
    fn run_gliding(&mut self, l: &mut [f32], r: &mut [f32]) {
        let rate = self.rate;
        for i in 0..l.len() {
            for cut in [&mut self.low, &mut self.high] {
                if !cut.log_hz.settled() {
                    cut.log_hz.next();
                    cut.update(rate);
                }
            }
            for b in &mut self.bands {
                if b.moving() {
                    b.advance(self.step);
                    b.update(rate);
                }
            }
            let (lo12, lo24) = self.low.weights();
            let (hi12, hi24) = self.high.weights();
            let g = self.out.next();
            let mut y = [l[i], r[i]];
            for (ch, x) in y.iter_mut().enumerate() {
                let mut s = self.low.run(ch, *x, lo12, lo24);
                s = self.high.run(ch, s, hi12, hi24);
                for b in &mut self.bands {
                    if !b.flat {
                        s = b.svf.run(ch, s);
                    }
                }
                *x = g * s;
            }
            l[i] = y[0];
            r[i] = y[1];
        }
    }

    /// The response in dB at each frequency of `hz` (written to `out`),
    /// for the settings the EQ is heading to: what an EQ page draws. Exact
    /// to the filters, cuts and output gain included.
    pub fn response_db(&self, hz: &[f32], out: &mut [f32]) {
        let mut stages = [Svf::default(); 10];
        let mut n = 0;
        for (cut, f, s) in [
            (&self.low, self.v[0], self.v[1] as usize),
            (&self.high, self.v[2], self.v[3] as usize),
        ] {
            if s == 1 {
                stages[n].set(cut.mode, f, Q12, 0.0, self.rate);
                n += 1;
            } else if s == 2 {
                for q in Q24 {
                    stages[n].set(cut.mode, f, q, 0.0, self.rate);
                    n += 1;
                }
            }
        }
        for b in 0..ids::BANDS {
            let gain = self.v[ids::gain(b) as usize];
            if gain != 0.0 {
                let kind = self.v[ids::band_type(b) as usize] as usize;
                stages[n].set(
                    band_mode(kind),
                    self.v[ids::freq(b) as usize],
                    band_q(kind, self.v[ids::q(b) as usize]),
                    gain,
                    self.rate,
                );
                n += 1;
            }
        }
        for (o, &f) in out.iter_mut().zip(hz) {
            let mut db = f64::from(self.v[ids::OUTPUT as usize]);
            for s in &stages[..n] {
                let (re, im) = s.response(f, self.rate);
                db += 10.0 * (re * re + im * im).max(1e-30).log10();
            }
            *o = db as f32;
        }
    }
}

impl Node for Eq {
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
        self.rate = rate.max(1);
        for cut in [&mut self.low, &mut self.high] {
            cut.log_hz.set_time(self.rate, 20.0);
            cut.w12.set_time(self.rate, 20.0);
            cut.w24.set_time(self.rate, 20.0);
        }
        for b in &mut self.bands {
            b.log_hz.set_time(self.rate, 20.0);
            b.gain.set_time(self.rate, 20.0);
            b.log_q.set_time(self.rate, 20.0);
        }
        self.out.set_time(self.rate, 20.0);
        self.step = 1.0 / (0.01 * self.rate as f32);
        self.update_all();
    }
    fn reset(&mut self) {
        self.low.snap();
        self.high.snap();
        for b in &mut self.bands {
            b.snap();
        }
        self.out.snap(self.out.target);
        self.update_all();
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
    fn rest_frames(&self) -> u32 {
        // What it holds is its tail: once rung out it is as good as reset.
        self.tail_frames()
    }
    fn tail_frames(&self) -> u32 {
        // A resonance rings for about Q / (pi f) seconds per 27 dB; allow 4x.
        let ring = |q: f32, hz: f32| 4.0 * q / (std::f32::consts::PI * hz.max(20.0));
        let mut secs = 0.0f32;
        if self.v[1] > 0.0 {
            secs = secs.max(ring(Q24[1], self.v[0]));
        }
        if self.v[3] > 0.0 {
            secs = secs.max(ring(Q24[1], self.v[2]));
        }
        for b in 0..ids::BANDS {
            if self.v[ids::gain(b) as usize] != 0.0 {
                let kind = self.v[ids::band_type(b) as usize] as usize;
                secs = secs.max(ring(
                    band_q(kind, self.v[ids::q(b) as usize]),
                    self.v[ids::freq(b) as usize],
                ));
            }
        }
        (secs * self.rate as f32).ceil() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut rng = crate::Rng::new(seed);
        (0..n).map(|_| 0.5 * rng.bipolar()).collect()
    }

    /// Once nothing glides, the lean loop gives the general loop's output
    /// bit for bit, and an EQ that does nothing leaves the block as it is.
    #[test]
    fn the_still_loop_matches_the_general_one() {
        let settings: [&[(u16, f32)]; 4] = [
            &[],
            &[(1, 1.0), (3, 2.0)],
            &[
                (1, 2.0),
                (ids::gain(0), 6.0),
                (ids::gain(2), -4.5),
                (ids::q(2), 3.0),
                (ids::OUTPUT, -2.0),
            ],
            &[
                (ids::band_type(3), 1.0),
                (ids::gain(3), 3.0),
                (ids::freq(1), 700.0),
                (ids::gain(1), 9.0),
            ],
        ];
        for (n, set) in settings.iter().enumerate() {
            let mut a = Eq::new();
            let mut b = Eq::new();
            for e in [&mut a, &mut b] {
                e.prepare(48_000, 512);
                for &(id, v) in *set {
                    e.set_param(id, v);
                }
                // Settle every glide and type swap.
                let (mut l, mut r) = (noise(48_000, 3), noise(48_000, 4));
                e.run(&mut l, &mut r);
                assert!(e.still(), "setting {n} settles");
            }
            let (mut la, mut ra) = (noise(4096, 5), noise(4096, 6));
            let (mut lb, mut rb) = (la.clone(), ra.clone());
            let (l0, r0) = (la.clone(), ra.clone());
            a.run_still(&mut la, &mut ra);
            b.run_gliding(&mut lb, &mut rb);
            assert!(
                la.iter()
                    .zip(&lb)
                    .chain(ra.iter().zip(&rb))
                    .all(|(x, y)| x.to_bits() == y.to_bits()),
                "setting {n}"
            );
            if n == 0 {
                assert_eq!(
                    (la, ra),
                    (l0, r0),
                    "the default EQ passes the input untouched"
                );
            }
        }
    }
}
