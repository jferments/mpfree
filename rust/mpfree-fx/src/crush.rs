//! Crush: bit-depth and sample-rate reduction. The quantisation noise
//! and the sample and hold's images are the point; what no sampler being
//! imitated makes is band-limited away. Each hold takes the input at its
//! exact time between the session's frames and steps to it band-limited,
//! so the steps keep their timing and nothing folds back from above the
//! session's Nyquist: a rate sounds the same at 44.1 and 48 kHz, and a
//! rate that doesn't divide the session's makes no stray tones (FX
//! quality round 2, plan-2 section 5, decision 10.2). The step is minimum
//! phase (as a sampler's analog output filter is), so nothing is heard
//! late: no latency.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
use crate::{defaults, Rng};

/// [`Crush`] parameter ids.
pub mod ids {
    pub const BITS: u16 = 0;
    pub const RATE: u16 = 1;
    pub const DITHER: u16 = 2;
    pub const MIX: u16 = 3;
    pub const TAILS: u16 = 4;
    pub const FILTERS: u16 = 5;
    pub const JITTER: u16 = 6;
    pub const OUTPUT: u16 = 7;
}

/// How the quantiser treats what is under half a step: Cut rounds it to
/// silence (mid-tread), Buzz keeps it as a buzz of half a step either
/// side (mid-riser).
pub const TAILS: &[&str] = &["Cut", "Buzz"];
/// The low-passes tied to the rate: before the sampler (anti-alias) and
/// after it (the image filter), as a sampler's converters have.
pub const FILTERS: &[&str] = &["Off", "Before", "After", "Both"];

static INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.crush",
    name: "Crush",
    kind: Kind::Effect,
};
static PARAMS: [ParamInfo; 8] = [
    ParamInfo {
        id: 0,
        name: "Bits",
        group: "Crush",
        min: 1.0,
        max: 16.0,
        default: 8.0,
        unit: Unit::None,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 1,
        name: "Rate",
        group: "Crush",
        min: 200.0,
        max: 48_000.0,
        default: 11_025.0,
        unit: Unit::Hz,
        curve: Curve::Log,
        readonly: false,
    },
    ParamInfo {
        id: 2,
        name: "Dither",
        group: "Crush",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Toggle,
        readonly: false,
    },
    ParamInfo {
        id: 3,
        name: "Mix",
        group: "Crush",
        min: 0.0,
        max: 100.0,
        default: 100.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 4,
        name: "Tails",
        group: "Crush",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(TAILS),
        readonly: false,
    },
    ParamInfo {
        id: 5,
        name: "Filters",
        group: "Crush",
        min: 0.0,
        max: 3.0,
        default: 0.0,
        unit: Unit::None,
        curve: Curve::Choice(FILTERS),
        readonly: false,
    },
    ParamInfo {
        id: 6,
        name: "Jitter",
        group: "Crush",
        min: 0.0,
        max: 100.0,
        default: 0.0,
        unit: Unit::Percent,
        curve: Curve::Linear,
        readonly: false,
    },
    ParamInfo {
        id: 7,
        name: "Output",
        group: "Output",
        min: -24.0,
        max: 12.0,
        default: 0.0,
        unit: Unit::Db,
        curve: Curve::Linear,
        readonly: false,
    },
];

/// The band-limited step's length in frames.
const TAPS: usize = 32;
/// Table points per frame (linearly interpolated between).
const OVER: usize = 16;
const RING: usize = 64;
const MASK: usize = RING - 1;
/// The rate-tied low-passes: fourth-order Butterworth (two stages) with
/// the corner at this fraction of the rate.
const FILTER_AT: f32 = 0.45;
const BUTTERWORTH_Q: [f32; 2] = [0.541_196_1, 1.306_563];
/// Jitter at 100 %: each hold is up to half its length longer or shorter.
const JITTER_SPAN: f64 = 0.5;

/// The minimum-phase band-limited step less the plain step's 1, by
/// phase: `PHASES[p][k]` is its value `k + p / OVER` frames after it
/// starts. Phase [`OVER`] is phase 0 a frame later, so any fraction
/// interpolates between two neighbouring rows.
static PHASES: [[f32; TAPS]; OVER + 1] = phases();

const fn phases() -> [[f32; TAPS]; OVER + 1] {
    let mut t = [[0.0f32; TAPS]; OVER + 1];
    let mut p = 0;
    while p <= OVER {
        let mut k = 0;
        while k < TAPS {
            t[p][k] = STEP[p + k * OVER] - 1.0;
            k += 1;
        }
        p += 1;
    }
    t
}

/// The step's residual at `f` (0 to 1) frames into each of its taps.
#[inline]
fn residuals(f: f32, out: &mut [f32; TAPS]) {
    let pos = f * OVER as f32;
    let p = (pos as usize).min(OVER - 1);
    let frac = pos - p as f32;
    let (a, b) = (&PHASES[p], &PHASES[p + 1]);
    for k in 0..TAPS {
        out[k] = a[k] + frac * (b[k] - a[k]);
    }
}

/// The input `f` (0 to 1) frames before the newest of `p` (the last
/// four inputs, oldest first): the cubic through all four.
#[inline]
fn cubic(p: &[f32; 4], f: f32) -> f32 {
    let [p0, p1, p2, p3] = *p;
    // Lagrange on the nodes -3, -2, -1 and 0, at t = -f.
    let t = -f;
    let (a, b, c, d) = (t + 3.0, t + 2.0, t + 1.0, t);
    (-p0 * b * c * d + 3.0 * p1 * a * c * d - 3.0 * p2 * a * b * d + p3 * a * b * c) / 6.0
}

/// One channel's state.
#[derive(Clone, Copy)]
struct Chan {
    /// The last four inputs, oldest first.
    hist: [f32; 4],
    /// The value held now (the plain staircase's level).
    held: f32,
    /// The band-limited steps' corrections still to come, by frame.
    acc: [f32; RING],
}

impl Chan {
    const fn new() -> Chan {
        Chan {
            hist: [0.0; 4],
            held: 0.0,
            acc: [0.0; RING],
        }
    }
}

/// Bit-depth and sample-rate reduction with optional TPDF dither, a
/// quantiser that cuts or keeps quiet tails, low-passes tied to the rate
/// and jitter. Steps are band-limited at their exact times.
pub struct Crush {
    v: [f32; 8],
    rate: u32,
    bits: Smooth,
    hz: Smooth,
    mix: Smooth,
    /// The wet's level (Output).
    out: Smooth,
    /// Phase toward the next hold (in holds), and where the next one
    /// falls (1, or nearby with jitter).
    phase: f64,
    next: f64,
    ch: [Chan; 2],
    /// The ring position of the frame being output.
    w: usize,
    /// Frames since reset, modulo 16: the low-passes follow a gliding
    /// rate every 16 frames on this grid, so any block size sounds the same.
    tick: u32,
    /// The rate-tied low-passes, before and after, and the rate they are
    /// set for.
    pre: [Svf; 2],
    post: [Svf; 2],
    filters_hz: f32,
    /// How much of each low-pass is heard (Filters fades over 10 ms), and
    /// whether each is at rest (faded out and cleared).
    pre_w: Smooth,
    post_w: Smooth,
    pre_rest: bool,
    post_rest: bool,
    rng: Rng,
    jitter_rng: Rng,
}

impl Default for Crush {
    fn default() -> Self {
        Self::new()
    }
}

impl Crush {
    pub fn new() -> Crush {
        let v = defaults(&PARAMS);
        let mut c = Crush {
            v,
            rate: 48_000,
            bits: Smooth::new(v[0]),
            hz: Smooth::new(v[1]),
            mix: Smooth::new(v[3] / 100.0),
            out: Smooth::new(1.0),
            phase: 1.0,
            next: 1.0,
            ch: [Chan::new(); 2],
            w: 0,
            tick: 0,
            pre: [Svf::default(); 2],
            post: [Svf::default(); 2],
            filters_hz: 0.0,
            pre_w: Smooth::new(0.0),
            post_w: Smooth::new(0.0),
            pre_rest: true,
            post_rest: true,
            rng: Rng::new(0x5EED),
            jitter_rng: Rng::new(0x7177),
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
            ids::BITS => self.bits.target = v,
            ids::RATE => self.hz.target = v,
            ids::MIX => self.mix.target = v / 100.0,
            ids::OUTPUT => self.out.target = crate::db_to_gain(v),
            ids::FILTERS => {
                let f = v as usize;
                self.pre_w.target = if f & 1 != 0 { 1.0 } else { 0.0 };
                self.post_w.target = if f & 2 != 0 { 1.0 } else { 0.0 };
            }
            _ => {}
        }
    }

    fn set_filters(&mut self, hz: f32) {
        let corner = (FILTER_AT * hz).min(FILTER_AT * self.rate as f32);
        for (s, q) in self.pre.iter_mut().zip(BUTTERWORTH_Q) {
            s.set(SvfMode::LowPass, corner, q, 0.0, self.rate);
        }
        for (s, q) in self.post.iter_mut().zip(BUTTERWORTH_Q) {
            s.set(SvfMode::LowPass, corner, q, 0.0, self.rate);
        }
        self.filters_hz = hz;
    }

    /// The rate-tied low-passes' gain at `hz`, dB, as set now (before and
    /// after together, as Filters chooses): what the page draws.
    pub fn filters_db(&self, hz: f32) -> f32 {
        Self::filters_response_db(
            self.v[ids::FILTERS as usize],
            self.v[ids::RATE as usize],
            self.rate,
            hz,
        )
    }

    /// The low-passes' gain at `hz`, dB, for Filters `filters` (its
    /// choice's index), Rate `rate_hz` and the session's rate `session`.
    pub fn filters_response_db(filters: f32, rate_hz: f32, session: u32, hz: f32) -> f32 {
        let n = match filters as usize {
            1 | 2 => 1.0,
            3 => 2.0,
            _ => return 0.0,
        };
        let session = session.max(1);
        let corner = (FILTER_AT * rate_hz).min(FILTER_AT * session as f32);
        let mut db = 0.0f64;
        for q in BUTTERWORTH_Q {
            let mut s = Svf::default();
            s.set(SvfMode::LowPass, corner, q, 0.0, session);
            let (re, im) = s.response(hz, session);
            db += 10.0 * (re * re + im * im).log10();
        }
        (db * n) as f32
    }

    /// The value held for input `x`, at the bit depth's `levels`.
    #[inline]
    fn quantise(x: f32, levels: f32, buzz: bool) -> f32 {
        if buzz {
            // Mid-riser: no level at zero, so what is under half a step
            // stays as a buzz; digital silence stays silent.
            if x == 0.0 {
                0.0
            } else {
                ((x * levels).floor() + 0.5) / levels
            }
        } else {
            (x * levels).round() / levels
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let dither = self.v[ids::DITHER as usize] >= 0.5;
        let buzz = self.v[ids::TAILS as usize] >= 0.5;
        let jitter = f64::from(self.v[ids::JITTER as usize]) / 100.0 * JITTER_SPAN;
        let per_frame = 1.0 / f64::from(self.rate);
        // The levels for the bit depth, worked out again only while it
        // glides (exp2 per sample is a library call on the MPC's ARM).
        let (mut bits_was, mut levels) = (f32::NAN, 1.0);
        for i in 0..l.len() {
            let bits = self.bits.next();
            if bits != bits_was {
                bits_was = bits;
                levels = (bits - 1.0).exp2();
            }
            let hz = self.hz.next();
            let (pw, qw) = (self.pre_w.next(), self.post_w.next());
            // A low-pass faded out runs no more, and starts again from rest.
            if pw == 0.0 && !self.pre_rest {
                self.pre.iter_mut().for_each(Svf::reset);
                self.pre_rest = true;
            }
            if qw == 0.0 && !self.post_rest {
                self.post.iter_mut().for_each(Svf::reset);
                self.post_rest = true;
            }
            if (pw > 0.0 || qw > 0.0)
                && hz != self.filters_hz
                && (self.tick == 0 || self.hz.settled())
            {
                self.set_filters(hz);
            }
            self.tick = (self.tick + 1) % 16;
            let step = f64::from(hz) * per_frame;
            let mix = self.mix.next();
            let gain = self.out.next();
            let x = [l[i], r[i]];
            let w = self.w;
            for (c, ch) in self.ch.iter_mut().enumerate() {
                let mut v = x[c];
                if pw > 0.0 {
                    self.pre_rest = false;
                    let mut f = v;
                    for s in &mut self.pre {
                        f = s.run(c, f);
                    }
                    v += (f - v) * pw;
                }
                ch.hist = [ch.hist[1], ch.hist[2], ch.hist[3], v];
            }
            if step >= 1.0 {
                // At or above the session's rate: a hold every frame, on
                // the frame.
                self.phase = 0.0;
                self.next = 1.0;
                for ch in &mut self.ch {
                    let d = if dither {
                        (self.rng.bipolar() + self.rng.bipolar()) * 0.5 / levels
                    } else {
                        0.0
                    };
                    ch.held = Self::quantise(ch.hist[3] + d, levels, buzz);
                }
            } else {
                self.phase += step;
                while self.phase >= self.next {
                    self.phase -= self.next;
                    // How long ago, in frames, the hold fell due: it takes
                    // the input then and steps from then.
                    let f = (self.phase / step).min(1.0 - 1e-9) as f32;
                    self.next = if jitter > 0.0 {
                        1.0 + jitter * f64::from(self.jitter_rng.bipolar())
                    } else {
                        1.0
                    };
                    let mut d = [0.0f32; 2];
                    for (c, ch) in self.ch.iter_mut().enumerate() {
                        let dith = if dither {
                            (self.rng.bipolar() + self.rng.bipolar()) * 0.5 / levels
                        } else {
                            0.0
                        };
                        let held = Self::quantise(cubic(&ch.hist, f) + dith, levels, buzz);
                        d[c] = held - ch.held;
                        ch.held = held;
                    }
                    if d == [0.0; 2] {
                        // The same value again: no step to smooth.
                        continue;
                    }
                    let mut s = [0.0f32; TAPS];
                    residuals(f, &mut s);
                    // The corrections run from this frame on, around the
                    // ring: in two straight runs.
                    let first = TAPS.min(RING - w);
                    for (ch, d) in self.ch.iter_mut().zip(d) {
                        let (head, tail) = s.split_at(first);
                        for (a, s) in ch.acc[w..w + first].iter_mut().zip(head) {
                            *a += d * s;
                        }
                        for (a, s) in ch.acc[..tail.len()].iter_mut().zip(tail) {
                            *a += d * s;
                        }
                    }
                }
            }
            let mut y = [0.0f32; 2];
            for (c, ch) in self.ch.iter_mut().enumerate() {
                let mut v = ch.held + ch.acc[w];
                ch.acc[w] = 0.0;
                if qw > 0.0 {
                    self.post_rest = false;
                    let mut f = v;
                    for s in &mut self.post {
                        f = s.run(c, f);
                    }
                    v += (f - v) * qw;
                }
                y[c] = crate::lerp(x[c], v * gain, mix);
            }
            self.w = (w + 1) & MASK;
            l[i] = y[0];
            r[i] = y[1];
        }
    }
}

impl Node for Crush {
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
        self.bits.set_time(self.rate, 20.0);
        self.hz.set_time(self.rate, 20.0);
        self.mix.set_time(self.rate, 20.0);
        self.out.set_time(self.rate, 20.0);
        self.pre_w.set_time(self.rate, 10.0);
        self.post_w.set_time(self.rate, 10.0);
        self.set_filters(self.hz.value);
    }
    fn reset(&mut self) {
        self.bits.snap(self.bits.target);
        self.hz.snap(self.hz.target);
        self.mix.snap(self.mix.target);
        self.out.snap(self.out.target);
        self.pre_w.snap(self.pre_w.target);
        self.post_w.snap(self.post_w.target);
        // Just under a hold, so the first frame takes one.
        self.phase = 1.0 - 1e-9;
        self.next = 1.0;
        self.ch = [Chan::new(); 2];
        self.w = 0;
        self.tick = 0;
        for s in self.pre.iter_mut().chain(&mut self.post) {
            s.reset();
        }
        self.pre_rest = true;
        self.post_rest = true;
        self.set_filters(self.hz.value);
        self.rng = Rng::new(0x5EED);
        self.jitter_rng = Rng::new(0x7177);
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
        // Dither makes noise from silence, and a lower rate keeps the
        // place of its held samples across silence (reset, the steps would
        // fall elsewhere).
        let full = self.rate as f32;
        if self.v[ids::DITHER as usize] >= 0.5 || self.hz.target < full || self.hz.value < full {
            return u32::MAX;
        }
        // The last step's tail and the image filter's ring.
        TAPS as u32 + self.rate / 100
    }
}

/// The minimum-phase band-limited step, `TAPS * OVER + 1` points from its
/// start (made by `tests::recipe`; print it with `MPFREE_PRINT_CRUSH_STEP=1
/// cargo test -p mpfree-fx --lib the_step_is_what -- --nocapture`).
#[rustfmt::skip]
static STEP: [f32; TAPS * OVER + 1] = [
    0.00000221, 0.000007758, 0.000019027, 0.000039428,
    0.000073676, 0.000128111, 0.000211064, 0.000333251,
    0.000508211, 0.000752776, 0.001087557, 0.001537455,
    0.002132169, 0.002906696, 0.003901808, 0.00516449,
    0.006748314, 0.008713742, 0.011128318, 0.014066752,
    0.017610861, 0.021849344, 0.026877388, 0.032796074,
    0.03971157, 0.047734123, 0.05697682, 0.067554116,
    0.07958015, 0.09316688, 0.10842197, 0.12544656,
    0.14433292, 0.16516185, 0.18800032, 0.21289876,
    0.23988864, 0.26898003, 0.3001594, 0.33338758,
    0.36859804, 0.40569556, 0.4445551, 0.48502156,
    0.52690953, 0.57000405, 0.6140616, 0.6588119,
    0.70396036, 0.74919087, 0.7941694, 0.8385483,
    0.8819706, 0.92407507, 0.96450204, 1.0028987,
    1.0389249, 1.0722598, 1.1026065, 1.1296991,
    1.1533072, 1.173241, 1.1893566, 1.2015581,
    1.2098024, 1.2141001, 1.2145175, 1.2111758,
    1.2042513, 1.1939731, 1.1806197, 1.1645155,
    1.1460255, 1.1255498, 1.1035163, 1.0803736,
    1.0565834, 1.0326111, 1.0089188, 0.9859551,
    0.96414775, 0.94389504, 0.9255583, 0.90945494,
    0.8958523, 0.8849627, 0.8769393, 0.87187356,
    0.8697937, 0.87066466, 0.8743893, 0.880811,
    0.8897178, 0.9008475, 0.9138937, 0.9285132,
    0.9443342, 0.9609646, 0.97800153, 0.99504024,
    1.0116837, 1.0275517, 1.0422891, 1.0555743,
    1.0671259, 1.0767095, 1.0841417, 1.0892948,
    1.0920981, 1.0925399, 1.0906659, 1.0865779,
    1.0804303, 1.0724255, 1.0628078, 1.0518576,
    1.0398828, 1.0272111, 1.0141814, 1.0011351,
    0.98840714, 0.97631764, 0.9651639, 0.9552131,
    0.9466961, 0.93980163, 0.93467236, 0.93140197,
    0.9300333, 0.9305585, 0.9329195, 0.93701106,
    0.9426838, 0.9497496, 0.95798665, 0.9671465,
    0.97696096, 0.9871498, 0.9974284, 1.0075158,
    1.0171419, 1.0260551, 1.0340284, 1.0408653,
    1.0464047, 1.0505246, 1.0531446, 1.0542276,
    1.0537797, 1.0518496, 1.0485257, 1.0439335,
    1.0382315, 1.031605, 1.024262, 1.0164254,
    1.0083272, 1.000202, 0.9922793, 0.98477817,
    0.9779008, 0.971827, 0.96670973, 0.9626712,
    0.9598002, 0.95814997, 0.95773774, 0.95854485,
    0.960518, 0.9635717, 0.96759146, 0.97243744,
    0.9779493, 0.9839514, 0.99025786, 0.99667865,
    1.0030248, 1.0091143, 1.0147766, 1.0198582,
    1.0242262, 1.0277715, 1.0304121, 1.0320944,
    1.0327941, 1.0325168, 1.0312963, 1.0291933,
    1.0262929, 1.0227021, 1.0185448, 1.0139589,
    1.0090915, 1.0040938, 0.9991172, 0.99430853,
    0.98980564, 0.9857337, 0.9822015, 0.97929925,
    0.9770954, 0.9756361, 0.9749438, 0.97501737,
    0.97583276, 0.97734433, 0.9794869, 0.982178,
    0.98532104, 0.98880845, 0.9925255, 0.99635375,
    1.0001745, 1.0038732, 1.0073419, 1.0104828,
    1.013211, 1.0154567, 1.0171666, 1.0183053,
    1.018856, 1.0188202, 1.0182171, 1.0170827,
    1.0154682, 1.013438, 1.0110669, 1.0084382,
    1.0056401, 1.0027635, 0.99989897, 0.9971335,
    0.9945485, 0.9922172, 0.99020267, 0.98855627,
    0.98731613, 0.9865068, 0.9861387, 0.98620814,
    0.98669815, 0.9875792, 0.9888108, 0.9903427,
    0.9921172, 0.9940711, 0.9961375, 0.9982486,
    1.0003372, 1.0023396, 1.0041965, 1.0058556,
    1.0072724, 1.0084116, 1.009248, 1.0097666,
    1.0099629, 1.0098428, 1.0094217, 1.0087242,
    1.0077823, 1.0066351, 1.005326, 1.0039024,
    1.0024135, 1.0009084, 0.999435, 0.99803865,
    0.99676013, 0.9956351, 0.9946931, 0.99395645,
    0.9934403, 0.993152, 0.9930916, 0.9932517,
    0.99361825, 0.99417114, 0.99488515, 0.9957309,
    0.99667597, 0.99768615, 0.99872655, 0.9997626,
    1.0007615, 1.0016929, 1.0025297, 1.0032495,
    1.0038341, 1.0042707, 1.0045516, 1.0046749,
    1.0046432, 1.0044646, 1.0041515, 1.0037197,
    1.0031887, 1.00258, 1.0019169, 1.0012233,
    1.0005231, 0.9998392, 0.9991932, 0.9986042,
    0.9980887, 0.9976601, 0.9973282, 0.99709934,
    0.996976, 0.99695724, 0.99703854, 0.99721247,
    0.9974688, 0.99779516, 0.99817735, 0.9986,
    0.9990472, 0.999503, 0.99995196, 1.0003794,
    1.0007725, 1.0011196, 1.0014119, 1.0016422,
    1.0018061, 1.0019015, 1.0019287, 1.0018904,
    1.0017912, 1.0016377, 1.0014377, 1.0012004,
    1.0009358, 1.0006541, 1.0003657, 1.0000808,
    0.9998088, 0.99955803, 0.99933594, 0.9991485,
    0.99900013, 0.99889374, 0.9988306, 0.99881065,
    0.99883205, 0.99889183, 0.9989858, 0.9991089,
    0.9992553, 0.9994186, 0.99959236, 0.99977,
    0.99994516, 1.000112, 1.0002654, 1.0004005,
    1.000514, 1.0006032, 1.0006664, 1.0007027,
    1.0007128, 1.0006976, 1.0006593, 1.0006005,
    1.0005244, 1.0004349, 1.0003357, 1.000231,
    1.0001247, 1.0000206, 0.99992234, 0.99983287,
    0.9997549, 0.9996904, 0.9996408, 0.999607,
    0.99958915, 0.9995869, 0.9995993, 0.999625,
    0.99966234, 0.9997091, 0.9997632, 0.9998221,
    0.99988335, 0.99994475, 1.000004, 1.0000591,
    1.0001085, 1.0001507, 1.0001845, 1.0002095,
    1.0002252, 1.0002317, 1.0002295, 1.0002192,
    1.0002016, 1.000178, 1.0001496, 1.0001178,
    1.0000839, 1.0000495, 1.0000156, 0.9999837,
    0.99995464, 0.99992937, 0.9999085, 0.9998926,
    0.99988174, 0.9998761, 0.9998754, 0.9998795,
    0.99988765, 0.9998994, 0.999914, 0.9999307,
    0.9999487, 0.9999673, 0.9999856, 1.0000031,
    1.0000191, 1.000033, 1.0000447, 1.0000536,
    1.00006, 1.0000634, 1.0000641, 1.0000625,
    1.0000584, 1.0000525, 1.000045, 1.0000364,
    1.0000271, 1.0000175, 1.0000081, 0.99999905,
    0.9999909, 0.9999838, 0.9999779, 0.9999734,
    0.9999704, 0.9999688, 0.9999686, 0.99996966,
    0.99997187, 0.999975, 0.9999789, 0.9999833,
    0.99998796, 0.9999927, 0.9999973, 1.0000017,
    1.0000055, 1.0000087, 1.0000113, 1.0000131,
    1.0000143, 1.0000147, 1.0000144, 1.0000136,
    1.0000122, 1.0000105, 1.0000083, 1.0000062,
    1.0000039, 1.0000017, 0.9999996, 0.99999774,
    0.9999962, 0.99999493, 0.9999941, 0.99999356,
    0.9999934, 0.99999356, 0.99999404, 0.99999475,
    0.99999565, 0.99999666, 0.99999774, 0.9999988,
    0.9999998, 1.0000008, 1.0000015, 1.0000023,
    1.0000027, 1.000003, 1.000003, 1.0000029,
    1.0000026, 1.0000021, 1.0000017, 1.0000011,
    1.0000005, 0.99999994, 0.9999994, 0.999999,
    0.9999987, 0.9999985, 0.9999985, 0.9999986,
    0.99999887, 0.9999992, 0.99999964, 1.0000001,
    1.0000005, 1.0000007, 1.0000008, 1.0000008,
    1.0000005, 1.0000001, 0.99999964, 0.99999946,
    1.0,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cubic_passes_through_its_points_and_lines() {
        let p = [0.0, 1.0, 2.0, 3.0];
        assert_eq!(cubic(&p, 0.0), 3.0);
        assert_eq!(cubic(&p, 1.0), 2.0);
        assert!((cubic(&p, 0.25) - 2.75).abs() < 1e-6);
        // A cubic is reproduced exactly.
        let q = |t: f32| 0.5 * t * t * t - t * t + 2.0;
        let p = [q(-3.0), q(-2.0), q(-1.0), q(0.0)];
        assert!((cubic(&p, 0.4) - q(-0.4)).abs() < 1e-5);
    }

    fn crush(rate: u32, values: &[(u16, f32)]) -> Crush {
        let mut c = Crush::new();
        c.prepare(rate, mpfree_node::MAX_BLOCK);
        for &(id, v) in values {
            c.set_param(id, v);
        }
        c.reset();
        c
    }

    #[test]
    fn each_hold_takes_the_input_at_its_exact_time() {
        // A ramp through 3 kHz holds in a 44.1 kHz session: 14.7 frames a
        // hold, so the holds fall between frames. The first falls due a
        // frame before the first frame; hold j at -1 + 14.7 j frames.
        let (fs, hz, a) = (44_100u32, 3000.0f32, 1e-3f32);
        let mut c = crush(fs, &[(ids::BITS, 16.0), (ids::RATE, hz)]);
        let t = f64::from(fs) / f64::from(hz);
        let mut holds = Vec::new();
        for n in 0..600 {
            let (mut l, mut r) = ([a * n as f32], [a * n as f32]);
            c.run(&mut l, &mut r);
            if holds.last() != Some(&c.ch[0].held) {
                holds.push(c.ch[0].held);
            }
        }
        assert!(holds.len() > 30, "{}", holds.len());
        let lsb = 1.0 / 32_768.0;
        for (j, h) in holds.iter().enumerate().skip(1) {
            let want = a * (-1.0 + j as f64 * t) as f32;
            // Within a step of 16 bits: 0.03 of a frame on this ramp.
            assert!(
                (h - want).abs() <= lsb + 1e-6,
                "hold {j}: {h} against {want}"
            );
        }
    }

    #[test]
    fn silence_stays_silent_with_either_tails() {
        for tails in [0.0, 1.0] {
            let mut c = crush(48_000, &[(ids::BITS, 4.0), (ids::TAILS, tails)]);
            let (mut l, mut r) = (vec![0.0f32; 4800], vec![0.0f32; 4800]);
            c.run(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|v| *v == 0.0), "tails {tails}");
        }
    }

    #[test]
    fn turning_the_filters_on_and_off_does_not_click() {
        // A 1 kHz sine at 16 bits and 22,050 Hz; Filters to Both, then Off.
        let mut c = crush(44_100, &[(ids::BITS, 16.0), (ids::RATE, 22_050.0)]);
        let n = 44_100;
        let x: Vec<f32> = (0..n)
            .map(|i| 0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / 44_100.0).sin())
            .collect();
        let (mut l, mut r) = (x.clone(), x.clone());
        let third = n / 3;
        c.run(&mut l[..third], &mut r[..third]);
        c.set_param(ids::FILTERS, 3.0);
        c.run(&mut l[third..2 * third], &mut r[third..2 * third]);
        c.set_param(ids::FILTERS, 0.0);
        c.run(&mut l[2 * third..], &mut r[2 * third..]);
        let slope = |a: usize, b: usize| {
            l[a..b]
                .windows(2)
                .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
        };
        let steady = slope(2000, third - 10).max(slope(third + 4000, 2 * third - 10));
        for at in [third, 2 * third] {
            let around = slope(at - 10, at + 2000);
            assert!(around < 1.1 * steady, "at {at}: {around} against {steady}");
        }
    }

    #[test]
    fn the_step_starts_at_zero_and_ends_at_one() {
        assert_eq!(STEP.len(), TAPS * OVER + 1);
        assert!(STEP[0].abs() < 0.01, "{}", STEP[0]);
        assert!((STEP[STEP.len() - 1] - 1.0).abs() < 1e-6);
        let mut s = [0.0; TAPS];
        residuals(0.0, &mut s);
        assert!((s[0] + 1.0).abs() < 0.01);
        residuals(1.0 - 1e-6, &mut s);
        assert!(s[TAPS - 1].abs() < 1e-3);
        // Each phase row is the step read at its own fraction.
        for p in 0..=OVER {
            for k in 0..TAPS {
                assert_eq!(PHASES[p][k], STEP[p + k * OVER] - 1.0);
            }
        }
    }

    /// A radix-2 FFT in f64, in place (for the step's recipe).
    fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
        let n = re.len();
        let mut j = 0;
        for i in 1..n {
            let mut bit = n >> 1;
            while j & bit != 0 {
                j ^= bit;
                bit >>= 1;
            }
            j |= bit;
            if i < j {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let a = if inverse { 1.0 } else { -1.0 } * std::f64::consts::TAU / len as f64;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (s, c) = (a * k as f64).sin_cos();
                    let (u, v) = (start + k, start + k + len / 2);
                    let (tr, ti) = (re[v] * c - im[v] * s, re[v] * s + im[v] * c);
                    re[v] = re[u] - tr;
                    im[v] = im[u] - ti;
                    re[u] += tr;
                    im[u] += ti;
                }
            }
            len <<= 1;
        }
        if inverse {
            for (r, i) in re.iter_mut().zip(im.iter_mut()) {
                *r /= n as f64;
                *i /= n as f64;
            }
        }
    }

    fn bessel_i0(x: f64) -> f64 {
        let (mut sum, mut term, q) = (1.0, 1.0, x * x / 4.0);
        for k in 1..60 {
            term *= q / (k * k) as f64;
            sum += term;
        }
        sum
    }

    /// The recipe of [`STEP`]: a Kaiser-windowed sinc (beta 9, its edge at
    /// 0.9 of the session's Nyquist, 32 frames, 16 points a frame), made
    /// minimum phase through its cepstrum, summed into a step.
    fn recipe() -> Vec<f64> {
        let n = TAPS * OVER;
        let i0b = bessel_i0(9.0);
        let h: Vec<f64> = (0..=n)
            .map(|i| {
                let x = (i as f64 - (n / 2) as f64) / OVER as f64;
                let r = 2.0 * i as f64 / n as f64 - 1.0;
                let w = bessel_i0(9.0 * (1.0 - r * r).max(0.0).sqrt()) / i0b;
                let a = std::f64::consts::PI * 0.9 * x;
                (if a.abs() < 1e-12 { 1.0 } else { a.sin() / a }) * w
            })
            .collect();
        let m = 1 << 15;
        let (mut re, mut im) = (vec![0.0; m], vec![0.0; m]);
        re[..h.len()].copy_from_slice(&h);
        fft(&mut re, &mut im, false);
        for k in 0..m {
            re[k] = (re[k] * re[k] + im[k] * im[k]).sqrt().max(1e-12).ln();
            im[k] = 0.0;
        }
        fft(&mut re, &mut im, true);
        // Fold the cepstrum onto positive quefrencies.
        for k in 1..m / 2 {
            re[k] *= 2.0;
            re[m - k] = 0.0;
        }
        im.iter_mut().for_each(|v| *v = 0.0);
        fft(&mut re, &mut im, false);
        for k in 0..m {
            let (e, (s, c)) = (re[k].exp(), im[k].sin_cos());
            re[k] = e * c;
            im[k] = e * s;
        }
        fft(&mut re, &mut im, true);
        let mut s = 0.0;
        let mut step: Vec<f64> = re[..=n]
            .iter()
            .map(|v| {
                s += v;
                s
            })
            .collect();
        let end = step[n];
        step.iter_mut().for_each(|v| *v /= end);
        step
    }

    #[test]
    fn the_step_is_what_its_recipe_makes() {
        let r = recipe();
        if std::env::var_os("MPFREE_PRINT_CRUSH_STEP").is_some() {
            for c in r.chunks(4) {
                let v: Vec<String> = c.iter().map(|x| format!("{},", *x as f32)).collect();
                println!("    {}", v.join(" "));
            }
        }
        for (i, (a, b)) in STEP.iter().zip(&r).enumerate() {
            assert!((f64::from(*a) - b).abs() < 1e-6, "{i}: {a} against {b}");
        }
    }
}
