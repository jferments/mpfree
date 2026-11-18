//! Pitch shifter: moves the sound up or down without changing its speed,
//! from two octaves down to two up, with a fine tune, a stereo spread for
//! doubling and feedback for rising or falling cascades.
//!
//! A delay-line shifter that splices where the sound lines up (the
//! Eventide H949's de-glitch, as WSOLA): one read sweeps through a short
//! delay at the rate the pitch asks for, and when it reaches the end of
//! its Window it jumps back (or on) by most of a Window. Where it lands is
//! searched for: up to half the Window of jumps (at most 25 ms, a period
//! of a bass's low E) are tried against the last 8 ms the read heard, on
//! a copy at an eighth of the rate, then frame by frame around the best,
//! then between frames, and the old read fades out over the new one in at
//! most 6 ms. On a steady note the two line up, so a splice is not heard;
//! on chords and noise the best match still keeps the level, a crossfade
//! gain from how well they matched making up what they don't share. The
//! reads are a twelve-tap windowed sinc, as bright at any fraction of a
//! frame and with its images 76 dB down; shifting down, a low-pass at the
//! shifted band's top takes out what is left of them. It needs no pitch to
//! be found, so it runs on anything, and it reports no latency: the
//! shifted sound lags by a quarter of the shortest and longest jumps
//! together (about 11 ms at the default 30 ms Window).

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::kit::Glide;
use crate::modfx::{const_i0, const_sin, const_sqrt};
use crate::svf::{Segs, Svf, SvfCoefs, SvfMode};
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
        30.0,
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

/// The longest window, in seconds.
const MAX_WINDOW_S: f32 = 0.12;
/// The nearest a read gets to the write, frames ([`SincLine::read`]'s
/// sinc needs half its taps written past its point).
const NEAREST: f64 = (TAPS / 2) as f64;
/// The widest range of jumps searched at a splice, ms: a period of 40 Hz
/// (a bass's low E), so a low note always has one that lines up. At most
/// half the Window (15 ms at the default, a period of 67 Hz).
const SEARCH_MS: f32 = 25.0;
/// How much of the recent past the coarse search compares, ms.
const MATCH_MS: f32 = 8.0;
/// How much the fine search compares, ms.
const FINE_MS: f32 = 4.0;
/// The longest crossfade at a splice, ms; shorter when splices come fast
/// (at most half the time between them).
const FADE_MS: f32 = 6.0;
/// The coarse copy keeps the mean of each `DEC` frames (to about 2.7 kHz
/// at 44.1 kHz; above that the fine search, a coarse step either side,
/// spans a whole period on its own).
const DEC: usize = 8;

/// Taps in [`SincLine::read`]: the read point sits between the 6th and
/// 7th.
const TAPS: usize = 12;
/// Fractions of a frame [`TABLE`] holds; reads between them blend the two
/// nearest rows.
const PHASES: usize = 64;
/// The windowed sinc's cutoff (cycles a frame) and Kaiser beta. Reading
/// faster or slower than the input came, the reconstruction's images of
/// the input land in the band (a tone's at the rate less the tone, times
/// the ratio, folded): these keep them 76 dB down, where the chorus's
/// eight taps leave them 62 dB down, and the top within 0.05 dB to 14 kHz
/// at 44.1 kHz.
const CUTOFF: f64 = 0.46;
const BETA: f64 = 6.0;

/// The taps at a fraction `t` (0 to 1) of a frame past a whole delay,
/// summing to 1, as the chorus's.
const fn sinc_taps(t: f64) -> [f64; TAPS] {
    let half = (TAPS / 2) as f64;
    let mut h = [0.0; TAPS];
    let mut sum = 0.0;
    let mut j = 0;
    while j < TAPS {
        let tau = (j as f64 - (half - 1.0)) - t;
        let x = tau / half;
        let w = const_i0(BETA * const_sqrt(1.0 - x * x)) - 1.0;
        let arg = std::f64::consts::PI * 2.0 * CUTOFF * tau;
        let s = if arg == 0.0 {
            1.0
        } else {
            const_sin(arg) / arg
        };
        h[j] = 2.0 * CUTOFF * s * w;
        sum += h[j];
        j += 1;
    }
    let mut j = 0;
    while j < TAPS {
        h[j] /= sum;
        j += 1;
    }
    h
}

/// Each fraction's taps, oldest sample first, and the step to the next
/// fraction's.
struct Table {
    taps: [[f32; TAPS]; PHASES],
    step: [[f32; TAPS]; PHASES],
}

const fn table() -> Table {
    let mut rows = [[0.0f64; TAPS]; PHASES + 1];
    let mut r = 0;
    while r <= PHASES {
        rows[r] = sinc_taps(r as f64 / PHASES as f64);
        r += 1;
    }
    let mut t = Table {
        taps: [[0.0; TAPS]; PHASES],
        step: [[0.0; TAPS]; PHASES],
    };
    let mut r = 0;
    while r < PHASES {
        let mut j = 0;
        while j < TAPS {
            let k = TAPS - 1 - j;
            t.taps[r][k] = rows[r][j] as f32;
            t.step[r][k] = (rows[r + 1][j] - rows[r][j]) as f32;
            j += 1;
        }
        r += 1;
    }
    t
}

/// Built at compile time, so it is the same on every machine.
static TABLE: Table = table();

/// Taps in [`taps_after`]'s reads.
pub(crate) const READ_TAPS: usize = TAPS;

/// The taps for a read `f` (0 to 1) of a frame after a sample, in time
/// order: tap `j` weighs the sample `j - 5` frames from it. The vocal
/// effects' grains each read at one fraction, so they take one set a grain
/// and spend the read on twelve multiplies a frame.
pub(crate) fn taps_after(f: f32) -> [f32; TAPS] {
    let p = f.clamp(0.0, 1.0) * PHASES as f32;
    let i = (p as usize).min(PHASES - 1);
    let a = p - i as f32;
    let (taps, step) = (&TABLE.taps[i], &TABLE.step[i]);
    std::array::from_fn(|j| taps[TAPS - 1 - j] + a * step[TAPS - 1 - j])
}

/// A delay line read through the twelve-tap sinc. Every sample is written
/// twice, a line's length apart, so the taps of any read sit side by side.
#[derive(Clone, Debug, Default)]
struct SincLine {
    buf: Vec<f32>,
    n: usize,
    w: usize,
    /// The longest delay a read may ask for.
    far: f32,
}

impl SincLine {
    /// Room for `frames` of delay (control thread).
    fn new(frames: usize) -> SincLine {
        let n = (frames + TAPS).next_power_of_two();
        SincLine {
            buf: vec![0.0; 2 * n],
            n,
            w: 0,
            far: (n - TAPS) as f32,
        }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.buf[self.w] = x;
        self.buf[self.w + self.n] = x;
        self.w = (self.w + 1) & (self.n - 1);
    }
    /// The signal `d` frames before the last write (0 is the last write),
    /// `d` at least half of [`TAPS`].
    #[inline]
    fn read(&self, d: f32) -> f32 {
        let d = d.clamp((TAPS / 2) as f32, self.far);
        // One cast for the whole frames and the fraction: `d` times a
        // power of two is exact. Through i32: a float to usize costs
        // several instructions more.
        let p = d * PHASES as f32;
        let i = p as i32;
        let a = p - i as f32;
        let whole = (i >> PHASES.trailing_zeros()) as usize;
        let r = (i & (PHASES as i32 - 1)) as usize;
        // The oldest tap is whole + 6 frames before the last write.
        let from = (self.w + self.n - 1 - whole - TAPS / 2) & (self.n - 1);
        let (x, _) = self.buf[from..from + TAPS].as_chunks::<4>();
        let (taps, _) = TABLE.taps[r].as_chunks::<4>();
        let (step, _) = TABLE.step[r].as_chunks::<4>();
        // Four lanes, then their sum in order (see `sum4`).
        let mut lane = [0.0f32; 4];
        for ((x, t), s) in x.iter().zip(taps).zip(step) {
            lane = [
                lane[0] + (t[0] + a * s[0]) * x[0],
                lane[1] + (t[1] + a * s[1]) * x[1],
                lane[2] + (t[2] + a * s[2]) * x[2],
                lane[3] + (t[3] + a * s[3]) * x[3],
            ];
        }
        lane.iter().sum()
    }
    /// The `len` samples from `d` whole frames before the last write back
    /// (0 is the last), oldest first, side by side.
    #[inline]
    fn slice(&self, d: usize, len: usize) -> &[f32] {
        let from = (self.w + 2 * self.n - (d + len) % self.n) & (self.n - 1);
        &self.buf[from..from + len]
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
        self.w = 0;
    }
}

/// The sums of `a * b` and of `b * b` over the two (the same length),
/// in four lanes.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> (f32, f32) {
    (sum4(a, b), sum4(b, b))
}

/// The sum of `a * b` over the two, in four lanes.
#[inline]
fn sum4(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (ac, ar) = a[..n].as_chunks::<4>();
    let (bc, br) = b[..n].as_chunks::<4>();
    let mut acc = [0.0f32; 4];
    for (x, y) in ac.iter().zip(bc) {
        acc = lanes(acc, x, y);
    }
    let mut rest = 0.0;
    for (x, y) in ar.iter().zip(br) {
        rest += x * y;
    }
    // Lane by lane in order: summed in pairs (0 with 2), the compiler
    // builds the lanes two wide instead of four.
    acc.iter().sum::<f32>() + rest
}

/// `acc + x * y`, lane by lane.
#[inline(always)]
fn lanes(acc: [f32; 4], x: &[f32; 4], y: &[f32; 4]) -> [f32; 4] {
    [
        acc[0] + x[0] * y[0],
        acc[1] + x[1] * y[1],
        acc[2] + x[2] * y[2],
        acc[3] + x[3] * y[3],
    ]
}

/// One side: its delay line, the coarse copy the search reads, and its
/// read (and the read fading out after a splice).
struct Voice {
    line: SincLine,
    coarse: Vec<f32>,
    /// Where the next coarse value goes, the frames summed toward it and
    /// how many.
    cw: usize,
    acc: f32,
    n: usize,
    /// The heard read's delay, frames before the last write.
    d: f64,
    /// While a splice fades: the old read's delay, frames into the fade,
    /// and its length (0 when not fading).
    old: f64,
    fade: u32,
    fade_len: u32,
    /// How alike the two reads were at the splice, 0 to 1.
    rho: f32,
    /// Reading slower than the input came (a shift down), the read's
    /// images of the input land in the band; this low-pass at the shifted
    /// band's top takes them out, and whether it is running.
    lp: Svf,
    lp_on: bool,
    /// The last output, for feedback.
    last: f32,
}

impl Voice {
    fn new() -> Voice {
        Voice {
            line: SincLine::default(),
            coarse: Vec::new(),
            cw: 0,
            acc: 0.0,
            n: 0,
            d: NEAREST,
            old: NEAREST,
            fade: 0,
            fade_len: 0,
            rho: 1.0,
            lp: Svf::default(),
            lp_on: false,
            last: 0.0,
        }
    }

    /// Room for `frames` of delay (control thread: it allocates).
    fn size(&mut self, frames: usize) {
        self.line = SincLine::new(frames);
        // Written twice, a length apart, as the line.
        let n = (frames / DEC + 8).next_power_of_two();
        self.coarse = vec![0.0; 2 * n];
    }

    /// Silent, the read `d` frames back.
    fn clear(&mut self, d: f64) {
        self.line.clear();
        self.coarse.fill(0.0);
        self.cw = 0;
        self.acc = 0.0;
        self.n = 0;
        self.d = d;
        self.old = d;
        self.fade = 0;
        self.fade_len = 0;
        self.rho = 1.0;
        self.lp.reset();
        self.lp_on = false;
        self.last = 0.0;
    }

    /// The `len` coarse values from `m` back from the newest whole one
    /// (0), oldest first, side by side.
    #[inline]
    fn coarse_slice(&self, m: usize, len: usize) -> &[f32] {
        let n = self.coarse.len() / 2;
        let from = (self.cw + 2 * n + 1 - (m + len) % n) & (n - 1);
        &self.coarse[from..from + len]
    }

    /// The coarse values from `m` back, where a read `d` frames back sits.
    fn coarse_lag(&self, d: f64) -> usize {
        // The newest whole coarse value covers the frames `n` to
        // `n + DEC - 1` back.
        let m = (d - self.n as f64 - 0.5 * (DEC - 1) as f64) / DEC as f64;
        if m <= 0.0 {
            0
        } else {
            (m + 0.5) as usize
        }
    }

    /// Splices: the read jumps `up` (to a longer delay, for a shift up) or
    /// down by between `lo` and `hi` frames, where the signal there is most
    /// like what the read has just heard, and the old read fades out over
    /// `fade_len` frames.
    #[inline(never)]
    fn splice(&mut self, up: bool, lo: f64, hi: f64, nc: usize, nf: usize, fade_len: u32) {
        let here = self.d;
        // Going down, the new read must stay behind the write.
        let hi = if up { hi } else { hi.min(here - NEAREST) };
        let lo = lo.min(hi);
        // Coarse: every DEC frames of jump, against the last `nc` coarse
        // values the read has passed.
        let ma = self.coarse_lag(here);
        let (jlo, jhi) = (
            (lo / DEC as f64).ceil() as usize,
            (hi / DEC as f64) as usize,
        );
        let mut best = (f32::MIN, jlo.max(1));
        for j in jlo.max(1)..=jhi.max(jlo.max(1)) {
            let mb = if up { ma + j } else { ma.saturating_sub(j) };
            let (ab, bb) = dot(self.coarse_slice(ma, nc), self.coarse_slice(mb, nc));
            let score = ab / (bb + 1e-12).sqrt();
            if score > best.0 {
                best = (score, j);
            }
        }
        // Fine: each frame within a coarse step of it, then between frames.
        let base = here.round().max(0.0) as usize;
        let centre = (best.1 * DEC) as i64;
        let (flo, fhi) = (lo.ceil() as i64, hi.floor() as i64);
        let mut scores = [f32::MIN; 2 * DEC + 1];
        let a = self.line.slice(base, nf);
        let (_, aa) = dot(a, a);
        let mut top = (f32::MIN, centre.clamp(flo, fhi.max(flo)), 0.0f32);
        for (k, sc) in scores.iter_mut().enumerate() {
            let j = centre + k as i64 - DEC as i64;
            if j < flo || j > fhi || j < 1 {
                continue;
            }
            let from = if up {
                base + j as usize
            } else {
                base.saturating_sub(j as usize)
            };
            let (ab, bb) = dot(a, self.line.slice(from, nf));
            *sc = ab / (bb + 1e-12).sqrt();
            if *sc > top.0 {
                top = (*sc, j, ab / (aa * bb + 1e-24).sqrt());
            }
        }
        // Between frames: the peak of a parabola through the best and its
        // neighbours.
        let k = (top.1 - centre + DEC as i64) as usize;
        let mut jump = top.1 as f64;
        if k > 0 && k < 2 * DEC {
            let (l, c, r) = (scores[k - 1], scores[k], scores[k + 1]);
            if l > f32::MIN && r > f32::MIN {
                let den = l - 2.0 * c + r;
                if den < 0.0 {
                    jump += f64::from((0.5 * (l - r) / den).clamp(-0.5, 0.5));
                }
            }
        }
        let jump = jump.clamp(lo.max(1.0), hi.max(1.0));
        self.old = here;
        self.d = if up { here + jump } else { here - jump };
        self.fade = 0;
        self.fade_len = fade_len.max(1);
        self.rho = top.2.clamp(0.0, 1.0);
    }

    /// Writes `x` (plus feedback) and returns the shifted sample.
    #[inline]
    fn next(&mut self, x: f32, ratio: f64, plan: &Plan, fb: f32) -> f32 {
        let into = crate::flush(x + fb * self.last);
        self.line.push(into);
        self.acc += into;
        self.n += 1;
        if self.n == DEC {
            let n = self.coarse.len() / 2;
            self.cw = (self.cw + 1) & (n - 1);
            let v = self.acc * (1.0 / DEC as f32);
            self.coarse[self.cw] = v;
            self.coarse[self.cw + n] = v;
            self.acc = 0.0;
            self.n = 0;
        }
        // The read moves (1 - ratio) a frame: going up shortens the delay,
        // going down lengthens it.
        let step = 1.0 - ratio;
        self.d += step;
        if self.fade_len > 0 {
            self.old += step;
        } else if ratio > 1.0 {
            // Splice while the old read still has room to fade out before
            // it reaches the write.
            if self.d <= NEAREST + 1.0 + (ratio - 1.0) * f64::from(plan.fade) {
                self.splice(true, plan.lo, plan.win, plan.nc, plan.nf, plan.fade);
            }
        } else if ratio < 1.0 && self.d >= NEAREST + plan.win {
            self.splice(false, plan.lo, plan.win, plan.nc, plan.nf, plan.fade);
        }
        let a = self.line.read(self.d as f32);
        let y = if self.fade_len > 0 {
            let b = self.line.read(self.old as f32);
            let t = (self.fade as f32 + 0.5) / self.fade_len as f32;
            // A smoothstep: level-matched, starting and ending flat.
            let gn = t * t * (3.0 - 2.0 * t);
            let go = 1.0 - gn;
            // What the two don't share adds in power, not level.
            let comp = 1.0 / (go * go + gn * gn + 2.0 * self.rho * go * gn).sqrt();
            self.fade += 1;
            if self.fade >= self.fade_len {
                self.fade_len = 0;
            }
            (a * gn + b * go) * comp
        } else {
            a
        };
        let y = if plan.lp_mix > 0.0 {
            // From rest when it comes in, where its share is still small.
            if !self.lp_on {
                self.lp.reset();
                self.lp_on = true;
            }
            let f = self.lp.run(0, y);
            y + plan.lp_mix * (f - y)
        } else {
            self.lp_on = false;
            y
        };
        // Kept at 0 while there is no feedback, so it comes in from rest.
        self.last = if fb != 0.0 { crate::soft(y) } else { 0.0 };
        y
    }
}

/// What a side's splices go by, worked out when the controls move.
#[derive(Clone, Copy, Debug)]
struct Plan {
    /// The Window, frames: a read sweeps that far, then jumps by between
    /// `lo` and it.
    win: f64,
    lo: f64,
    /// The crossfade, frames.
    fade: u32,
    /// How many coarse values and frames the searches compare.
    nc: usize,
    nf: usize,
    /// The share of the image low-pass (0 from a ratio of 0.7 up, all of
    /// it from 0.5 down) and its design.
    lp_mix: f32,
    lp: SvfCoefs,
}

impl Default for Plan {
    fn default() -> Plan {
        Plan {
            win: 0.0,
            lo: 0.0,
            fade: 1,
            nc: 4,
            nf: 8,
            lp_mix: 0.0,
            lp: SvfCoefs::bilinear(SvfMode::LowPass, 1000.0, 0.707, 0.0, 48_000),
        }
    }
}

/// A delay-line pitch shifter with splices that line up.
pub struct PitchShifter {
    v: [f32; 6],
    rate: u32,
    /// Semitones plus cents, as cents, smoothed.
    cents: Smooth,
    spread: Smooth,
    window: Smooth,
    feedback: Smooth,
    mix: Smooth,
    voices: [Voice; 2],
    /// The cents, spread and window last worked out, and what they gave.
    was: (f32, f32, f32),
    ratios: [f64; 2],
    plans: [Plan; 2],
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
            voices: [Voice::new(), Voice::new()],
            was: (f32::NAN, f32::NAN, f32::NAN),
            ratios: [1.0; 2],
            plans: [Plan::default(); 2],
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

    /// The splices' plan for a `ratio` and a Window of `win_ms`.
    fn plan(&self, ratio: f64, win_ms: f32) -> Plan {
        let ms = f64::from(self.rate) * 0.001;
        let win = f64::from(win_ms) * ms;
        let search = f64::from(SEARCH_MS.min(0.5 * win_ms)) * ms;
        let lo = win - search;
        // A splice comes at least every lo / |ratio - 1| frames; a fade
        // takes at most half of that.
        let gap = (ratio - 1.0).abs();
        let mut fade = f64::from(FADE_MS) * ms;
        if gap * fade > 0.5 * lo {
            fade = 0.5 * lo / gap;
        }
        // Read at `ratio` of the input's pace, the input's band lands below
        // ratio * Nyquist and the read's images of it near ratio * rate.
        let lp_mix = ((0.7 - ratio as f32) / 0.2).clamp(0.0, 1.0);
        let lp = if lp_mix > 0.0 {
            let hz = 0.45 * ratio as f32 * self.rate as f32;
            SvfCoefs::bilinear(SvfMode::LowPass, hz, 0.707, 0.0, self.rate)
        } else {
            Plan::default().lp
        };
        Plan {
            win,
            lo,
            fade: (fade as u32).max(1),
            nc: ((f64::from(MATCH_MS) * ms) as usize / DEC).max(4),
            nf: ((f64::from(FINE_MS) * ms) as usize).max(8),
            lp_mix,
            lp,
        }
    }

    /// Where a read starts after a reset: the middle of an up sweep.
    fn start(&self) -> f64 {
        let p = self.plan(1.0, self.window.target);
        NEAREST + 0.25 * (p.lo + p.win)
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let cents = self.cents.glide();
            let spread = self.spread.glide();
            let win = self.window.glide();
            if (cents, spread, win) != self.was {
                self.was = (cents, spread, win);
                let half = spread * 0.5;
                for (k, c) in [cents - half, cents + half].into_iter().enumerate() {
                    self.ratios[k] = f64::from(fast_exp2(c / 1200.0));
                    self.plans[k] = self.plan(self.ratios[k], win);
                    if self.plans[k].lp_mix > 0.0 {
                        self.voices[k].lp.set_coefs(self.plans[k].lp);
                    }
                }
            }
            let fb = self.feedback.glide();
            let mix = self.mix.glide();
            let x = [l[i], r[i]];
            let y0 = self.voices[0].next(x[0], self.ratios[0], &self.plans[0], fb);
            let y1 = self.voices[1].next(x[1], self.ratios[1], &self.plans[1], fb);
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
            // The longest Window, the old read's fade beyond it and what
            // the search compares past the far end.
            let frames = ((MAX_WINDOW_S + 0.05) * rate as f32) as usize;
            for v in &mut self.voices {
                v.size(frames);
            }
            let d = self.start();
            for v in &mut self.voices {
                v.clear(d);
            }
            self.was = (f32::NAN, f32::NAN, f32::NAN);
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
        let d = self.start();
        for v in &mut self.voices {
            v.clear(d);
        }
        self.was = (f32::NAN, f32::NAN, f32::NAN);
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

    /// Renders `x` (both sides) through a shifter at `semis`, a block at
    /// a time, and returns the left side.
    fn shifted(semis: f32, x: &[f32], rate: u32) -> Vec<f32> {
        let mut s = PitchShifter::new();
        s.set_param(ids::PITCH, semis);
        s.prepare(rate, 128);
        s.reset();
        let (mut l, mut r) = (x.to_vec(), x.to_vec());
        let ctx = Ctx::new(rate, 0);
        for k in (0..l.len()).step_by(128) {
            let e = (k + 128).min(l.len());
            s.process(&ctx, &[], &mut l[k..e], &mut r[k..e]);
        }
        l
    }

    /// The level at `hz` in `x`, by a single-bin DFT (whole cycles when `x`
    /// is a second long and `hz` whole).
    fn level_at(x: &[f32], hz: f64, rate: u32) -> f64 {
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let w = std::f64::consts::TAU * hz * i as f64 / f64::from(rate);
            re += f64::from(*v) * w.cos();
            im += f64::from(*v) * w.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

    #[test]
    fn every_note_comes_out_clean_an_octave_up_or_down() {
        // Splices that line up: the shifted tone at its level, and next to
        // nothing else, from a bass's low notes up.
        let rate = 44_100;
        for hz in [56.0, 112.0, 440.0, 1760.0] {
            for semis in [12.0f32, -12.0] {
                let x: Vec<f32> = (0..2 * rate as usize)
                    .map(|i| {
                        (0.5 * (std::f64::consts::TAU * hz * i as f64 / 44_100.0).sin()) as f32
                    })
                    .collect();
                let y = shifted(semis, &x, rate);
                let last = &y[rate as usize..];
                let want = hz * 2f64.powf(f64::from(semis) / 12.0);
                let a = level_at(last, want, rate);
                let total: f64 =
                    last.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / last.len() as f64;
                let off = (total - a * a / 2.0).max(0.0) / (a * a / 2.0);
                assert!((a - 0.5).abs() < 0.02, "{hz} Hz at {semis}: level {a}");
                assert!(off < 1e-3, "{hz} Hz at {semis}: off the tone {off}");
            }
        }
    }

    /// The mean delay of the sound through a shifter at `semis`: each of a
    /// train of clicks' energy-weighted time out, ms.
    fn mean_delay_ms(semis: f32) -> f64 {
        let rate = 44_100u32;
        let gap = (0.0373 * rate as f32) as usize;
        let n = 3 * rate as usize;
        let mut x = vec![0.0f32; n];
        let clicks: Vec<usize> = (rate as usize / 2..n - gap).step_by(gap).collect();
        for &k in &clicks {
            x[k] = 1.0;
        }
        let y = shifted(semis, &x, rate);
        let mut sum = 0.0f64;
        for &k in &clicks {
            let (mut e, mut et) = (0.0f64, 0.0f64);
            for j in 0..gap {
                let v = f64::from(y[k + j]).powi(2);
                e += v;
                et += v * j as f64;
            }
            sum += et / e.max(1e-30);
        }
        sum / clicks.len() as f64 * 1000.0 / f64::from(rate)
    }

    #[test]
    fn it_lags_about_half_the_window_and_reports_none() {
        // FX quality H4 (decision 12.2): immediate, about 15 ms behind.
        for (semis, most) in [
            (0.0, 15.0),
            (12.0, 15.0),
            (7.0, 15.0),
            (-5.0, 22.0),
            (-12.0, 20.0),
        ] {
            let ms = mean_delay_ms(semis);
            assert!(ms > 2.0 && ms < most, "{semis}: {ms} ms");
        }
        assert_eq!(PitchShifter::new().latency(), 0);
    }

    #[test]
    fn the_twelve_tap_read_passes_the_lows_exactly() {
        for r in 0..PHASES {
            let sum: f32 = TABLE.taps[r].iter().sum();
            assert!((sum - 1.0).abs() < 1e-5, "row {r}: {sum}");
        }
        // A steady line reads back steady at any fraction.
        let mut line = SincLine::new(64);
        for _ in 0..64 {
            line.push(0.25);
        }
        for k in 0..40 {
            let v = line.read(6.0 + k as f32 * 0.37);
            assert!((v - 0.25).abs() < 1e-5, "{v}");
        }
    }
}
