//! Vocal effects: **Pitch correct** pulls a voice to the notes of a key,
//! and the **Harmonizer** adds up to two voices a scale's intervals away.
//!
//! Both rest on two parts:
//!
//! - [`Tracker`]: the voice's pitch, by YIN (de Cheveigné and Kawahara,
//!   JASA 2002) on the input low-passed and taken down to about 12 kHz,
//!   every 5 ms: the difference function, its cumulative-mean
//!   normalisation, the first dip under 0.15 (else the deepest under
//!   0.25), refined by a parabola. Quiet input (under -50 dBFS) or a dip
//!   that isn't deep enough is unvoiced.
//! - [`Grains`] and [`Voice`]: a pitch-synchronous overlap-add (PSOLA)
//!   shifter. Marks are laid one period apart on the waveform's peaks while
//!   the voice is voiced (every 5 ms otherwise); a voice lays a grain from
//!   the mark nearest its target time, a period either side, every period
//!   divided by its ratio. The grains keep their content, so the pitch
//!   moves and the formants (the vowel, the throat) stay: no chipmunk. At
//!   ratio 1 it gives the input back exactly, delayed. The latency is two
//!   and a half of the Range's longest periods.
//!
//! A grain's window rises as `sin²` over the gap back to the mark before
//! and falls as one minus that over the gap to the mark after, so
//! neighbouring grains sum to one; where faster grains overlap more (pitch
//! up) the sum of the windows divides it back down.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::svf::{Segs, Svf, SvfMode};
use crate::{defaults, fast_exp2, fast_log2};

/// The keys, as the Key parameter's choices.
pub const KEYS: &[&str] = &[
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
/// The scales, as the Scale parameter's choices.
pub const SCALES: &[&str] = &[
    "Chromatic",
    "Major",
    "Minor",
    "Harmonic minor",
    "Major pentatonic",
    "Minor pentatonic",
    "Blues",
];
/// Each scale's notes as bits over the semitones from the key (bit 0 the
/// key itself).
const SCALE_BITS: [u16; 7] = [
    0xfff,
    bits(&[0, 2, 4, 5, 7, 9, 11]),
    bits(&[0, 2, 3, 5, 7, 8, 10]),
    bits(&[0, 2, 3, 5, 7, 8, 11]),
    bits(&[0, 2, 4, 7, 9]),
    bits(&[0, 3, 5, 7, 10]),
    bits(&[0, 3, 5, 6, 7, 10]),
];
/// The voice ranges the tracker listens for, as the Range parameter's
/// choices, and their lowest and highest pitch in Hz.
pub const RANGES: &[&str] = &["Low", "Mid", "High"];
const RANGE_HZ: [(f32, f32); 3] = [(60.0, 600.0), (90.0, 900.0), (150.0, 1500.0)];
/// The detected note, as a meter's choices: "-" for none, then MIDI notes
/// 1 to 127 by name (C4 is middle C).
pub const NOTE_NAMES: &[&str] = &[
    "-", "C#-1", "D-1", "D#-1", "E-1", "F-1", "F#-1", "G-1", "G#-1", "A-1", "A#-1", "B-1", "C0",
    "C#0", "D0", "D#0", "E0", "F0", "F#0", "G0", "G#0", "A0", "A#0", "B0", "C1", "C#1", "D1",
    "D#1", "E1", "F1", "F#1", "G1", "G#1", "A1", "A#1", "B1", "C2", "C#2", "D2", "D#2", "E2", "F2",
    "F#2", "G2", "G#2", "A2", "A#2", "B2", "C3", "C#3", "D3", "D#3", "E3", "F3", "F#3", "G3",
    "G#3", "A3", "A#3", "B3", "C4", "C#4", "D4", "D#4", "E4", "F4", "F#4", "G4", "G#4", "A4",
    "A#4", "B4", "C5", "C#5", "D5", "D#5", "E5", "F5", "F#5", "G5", "G#5", "A5", "A#5", "B5", "C6",
    "C#6", "D6", "D#6", "E6", "F6", "F#6", "G6", "G#6", "A6", "A#6", "B6", "C7", "C#7", "D7",
    "D#7", "E7", "F7", "F#7", "G7", "G#7", "A7", "A#7", "B7", "C8", "C#8", "D8", "D#8", "E8", "F8",
    "F#8", "G8", "G#8", "A8", "A#8", "B8", "C9", "C#9", "D9", "D#9", "E9", "F9", "F#9", "G9",
];

const fn bits(notes: &[u16]) -> u16 {
    let mut b = 0;
    let mut i = 0;
    while i < notes.len() {
        b |= 1 << notes[i];
        i += 1;
    }
    b
}

/// Whether MIDI note `n` is in `scale` on `key`.
fn in_scale(n: i32, key: usize, scale: usize) -> bool {
    let bits = SCALE_BITS[scale.min(SCALE_BITS.len() - 1)];
    bits & (1 << (n - key as i32).rem_euclid(12)) != 0
}

/// The note of `scale` on `key` nearest to `midi` (a fractional note).
pub fn nearest_note(midi: f32, key: usize, scale: usize) -> i32 {
    let r = midi.round() as i32;
    let mut best = r;
    let mut far = f32::MAX;
    for n in r - 6..=r + 6 {
        let d = (n as f32 - midi).abs();
        if in_scale(n, key, scale) && d < far {
            far = d;
            best = n;
        }
    }
    best
}

/// The Harmonizer's intervals, as its voices' choices.
pub const INTERVALS: &[&str] = &[
    "Off",
    "Octave down",
    "6th down",
    "5th down",
    "4th down",
    "3rd down",
    "Unison",
    "3rd up",
    "4th up",
    "5th up",
    "6th up",
    "Octave up",
];
/// Each interval as scale steps (in a seven-note scale) and as semitones
/// (in any other).
const INTERVAL_STEPS: [(i32, i32); 12] = [
    (0, 0),
    (-7, -12),
    (-5, -8),
    (-4, -7),
    (-3, -5),
    (-2, -3),
    (0, 0),
    (2, 4),
    (3, 5),
    (4, 7),
    (5, 9),
    (7, 12),
];

/// How many semitones interval `k` ([`INTERVALS`]) is from note `n` in
/// `scale` on `key`: in a seven-note scale, that many of its steps from
/// `n` (taken to the scale first), so a 3rd up is major or minor as the
/// key has it; in the others, the interval's usual size taken to the
/// nearest note of the scale. None for Off.
pub fn interval_semis(k: usize, n: i32, key: usize, scale: usize) -> Option<i32> {
    if k == 0 || k >= INTERVAL_STEPS.len() {
        return None;
    }
    let (steps, semis) = INTERVAL_STEPS[k];
    let bits = SCALE_BITS[scale.min(SCALE_BITS.len() - 1)];
    if bits.count_ones() != 7 {
        let target = nearest_note((n + semis) as f32, key, scale);
        return Some(target - n);
    }
    let from = nearest_note(n as f32, key, scale);
    let mut at = from;
    let dir = steps.signum();
    for _ in 0..steps.abs() {
        at += dir;
        while !in_scale(at, key, scale) {
            at += dir;
        }
    }
    Some(at - n)
}

/// YIN's threshold for a voiced dip, and the deepest a dip may be and
/// still count when none is under it.
const YIN_THRESHOLD: f32 = 0.15;
const YIN_WEAK: f32 = 0.25;
/// Under this level (-50 dBFS) the input is unvoiced.
const GATE_RMS: f32 = 0.003_16;
/// The tracker's rate for each range, about: ten times the range's
/// highest note, so the search costs about the same in each (its cost
/// goes as the square of the rate).
const TRACK_HZ: [f32; 3] = [6_000.0, 8_000.0, 12_000.0];

/// The voice's pitch (see the module docs).
pub(crate) struct Tracker {
    rate: u32,
    range: usize,
    /// Take one sample in `dec`.
    dec: usize,
    phase: usize,
    rate_dec: f32,
    /// Two low-pass stages (the filter's two channels, in series).
    aa: Svf,
    buf: Vec<f32>,
    mask: usize,
    w: usize,
    hop: usize,
    since: usize,
    min_lag: usize,
    max_lag: usize,
    lin: Vec<f32>,
    d: Vec<f32>,
    /// The period found at the last look, in frames at the full rate (0:
    /// unvoiced), and how many looks a dropout has lasted.
    pub period: f32,
    misses: u8,
}

impl Tracker {
    pub(crate) fn new() -> Tracker {
        Tracker {
            rate: 48_000,
            range: 1,
            dec: 1,
            phase: 0,
            rate_dec: TRACK_HZ[1],
            aa: Svf::default(),
            buf: Vec::new(),
            mask: 0,
            w: 0,
            hop: 60,
            since: 0,
            min_lag: 10,
            max_lag: 100,
            lin: Vec::new(),
            d: Vec::new(),
            period: 0.0,
            misses: 0,
        }
    }

    /// The decimation for `range` at `rate`.
    fn dec_for(rate: u32, range: usize) -> usize {
        ((rate as f32 / TRACK_HZ[range]).round() as usize).clamp(1, 16)
    }

    /// Sizes everything for `rate` (allocates), with room for every range.
    pub(crate) fn prepare(&mut self, rate: u32, range: usize) {
        self.rate = rate;
        let most = (0..RANGE_HZ.len())
            .map(|r| {
                let rate_dec = rate as f32 / Self::dec_for(rate, r) as f32;
                (rate_dec / RANGE_HZ[r].0).ceil() as usize + 2
            })
            .max()
            .unwrap_or(2);
        let n = (2 * most + 2).next_power_of_two();
        self.buf = vec![0.0; n];
        self.mask = n - 1;
        self.lin = vec![0.0; 2 * most + 2];
        self.d = vec![0.0; most + 2];
        self.range = usize::MAX;
        self.set_range(range);
    }

    /// Listens for `range` (its rate, filter and lags); starts afresh when
    /// that changes. No allocation.
    pub(crate) fn set_range(&mut self, range: usize) {
        let range = range.min(RANGE_HZ.len() - 1);
        if range == self.range {
            return;
        }
        self.range = range;
        self.dec = Self::dec_for(self.rate, range);
        self.rate_dec = self.rate as f32 / self.dec as f32;
        self.aa.set(
            SvfMode::LowPass,
            0.2 * self.rate_dec,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            self.rate,
        );
        self.hop = ((self.rate_dec * 0.005) as usize).max(1);
        let (lo, hi) = RANGE_HZ[range];
        self.min_lag = ((self.rate_dec / hi) as usize).max(2);
        self.max_lag = ((self.rate_dec / lo).ceil() as usize).min(self.d.len().saturating_sub(2));
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.aa.reset();
        self.buf.fill(0.0);
        self.w = 0;
        self.phase = 0;
        self.since = 0;
        self.period = 0.0;
        self.misses = 0;
    }

    /// Takes one input sample; true when it looked again (every 5 ms).
    #[inline]
    pub(crate) fn push(&mut self, x: f32) -> bool {
        let y0 = self.aa.run(0, x);
        let y = self.aa.run(1, y0);
        self.phase += 1;
        if self.phase < self.dec {
            return false;
        }
        self.phase = 0;
        self.w = (self.w + 1) & self.mask;
        self.buf[self.w] = y;
        self.since += 1;
        if self.since < self.hop {
            return false;
        }
        self.since = 0;
        self.look();
        true
    }

    /// Once every 5 ms: kept out of line so `push`, every frame, stays small.
    #[inline(never)]
    fn look(&mut self) {
        let (lo, hi) = (self.min_lag, self.max_lag);
        if hi <= lo + 2 || self.buf.is_empty() {
            return;
        }
        // The window is as long as the longest period; it and its shifts
        // need twice that, the newest samples last.
        let win = hi;
        let n = win + hi;
        for (k, v) in self.lin[..n].iter_mut().enumerate() {
            *v = self.buf[(self.w + 1 + self.mask + 1 - n + k) & self.mask];
        }
        let x = &self.lin[..n];
        let energy: f32 = x[..win].iter().map(|v| v * v).sum::<f32>() / win as f32;
        let mut found = None;
        if energy > GATE_RMS * GATE_RMS {
            // The difference function and its cumulative-mean normal form,
            // only as far as it takes: the first dip under the threshold is
            // the answer as soon as the curve turns up again (a voice in the
            // middle of the range costs about half the full search).
            let base = &x[..win];
            let mut sum = 0.0f32;
            self.d[0] = 1.0;
            let mut dip = false;
            let mut pick = None;
            let (mut best, mut at) = (YIN_WEAK, None);
            for tau in 1..=hi {
                let acc = diff(base, &x[tau..tau + win]);
                sum += acc;
                let v = if sum > 0.0 {
                    acc * tau as f32 / sum
                } else {
                    1.0
                };
                self.d[tau] = v;
                if tau <= lo {
                    continue;
                }
                // `prev` is the candidate (lo to hi - 1), `v` the one after.
                let prev = tau - 1;
                let pv = self.d[prev];
                if pv < YIN_THRESHOLD {
                    dip = true;
                }
                if dip {
                    if v >= pv {
                        pick = Some(prev);
                        break;
                    }
                } else if pv < best {
                    best = pv;
                    at = Some(prev);
                }
            }
            if dip && pick.is_none() {
                pick = Some(hi - 1);
            }
            let pick = pick.or(at);
            let d = &self.d;
            if let Some(t) = pick {
                // A parabola through the dip and its neighbours.
                let (a, b, c) = (d[t - 1], d[t], d[(t + 1).min(hi)]);
                let den = a - 2.0 * b + c;
                let shift = if den.abs() > 1e-9 {
                    (0.5 * (a - c) / den).clamp(-0.5, 0.5)
                } else {
                    0.0
                };
                found = Some((t as f32 + shift) * self.dec as f32);
            }
        }
        match found {
            Some(p) => {
                self.period = p;
                self.misses = 0;
            }
            // Two looks (10 ms) of grace before a dropout counts.
            None if self.misses < 2 && self.period > 0.0 => self.misses += 1,
            None => self.period = 0.0,
        }
    }
}

/// The sum of squared differences of `a` and `b`, four at a time (the
/// difference function's inner loop: most of the tracker's cost).
#[inline]
fn diff(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0.0f32; 4];
    let (ca, cb) = (a.chunks_exact(4), b.chunks_exact(4));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (p, q) in ca.zip(cb) {
        for k in 0..4 {
            let e = p[k] - q[k];
            acc[k] += e * e;
        }
    }
    let mut tail = 0.0;
    for (p, q) in ra.iter().zip(rb) {
        let e = p - q;
        tail += e * e;
    }
    (acc[0] + acc[1]) + (acc[2] + acc[3]) + tail
}

/// A period in frames at `rate` as a fractional MIDI note.
fn period_midi(period: f32, rate: u32) -> f32 {
    69.0 + 12.0 * (fast_log2(rate as f32 / period) - fast_log2(440.0))
}

/// How many marks a [`Grains`] keeps.
const MARKS: usize = 64;

/// The input side of the PSOLA shifter: the input's last moments, a
/// low-passed mono copy to find peaks on, and the pitch marks.
pub(crate) struct Grains {
    inl: Vec<f32>,
    inr: Vec<f32>,
    /// The middle, `(l + r) / 2`, for mono voices.
    mid: Vec<f32>,
    /// The middle low-passed, to find peaks on.
    mono: Vec<f32>,
    mask: usize,
    /// The frame being processed.
    t: u64,
    marks: [u64; MARKS],
    /// Marks laid so far (mark `j` is `marks[j % MARKS]`).
    count: usize,
    lp: [f32; 2],
    lp_k: f32,
    /// The period marks are laid at while unvoiced.
    unvoiced: u32,
    /// The period now (the tracker's, or `unvoiced`): a grain's width when
    /// the mark beside it isn't laid yet.
    hint: f32,
    /// The period the next mark's search was worked out for, the frame it
    /// can be laid on, and where it searches.
    for_period: f32,
    due: u64,
    search: (u64, u64),
}

impl Grains {
    pub(crate) fn new() -> Grains {
        Grains {
            inl: Vec::new(),
            inr: Vec::new(),
            mid: Vec::new(),
            mono: Vec::new(),
            mask: 0,
            t: 0,
            marks: [0; MARKS],
            count: 0,
            lp: [0.0; 2],
            lp_k: 0.1,
            unvoiced: 240,
            hint: 240.0,
            for_period: -1.0,
            due: 0,
            search: (0, 0),
        }
    }

    /// Sizes for `rate` with periods up to `longest` frames (allocates).
    pub(crate) fn prepare(&mut self, rate: u32, longest: usize) {
        let n = (8 * longest + 16).next_power_of_two();
        self.inl = vec![0.0; n];
        self.inr = vec![0.0; n];
        self.mid = vec![0.0; n];
        self.mono = vec![0.0; n];
        self.mask = n - 1;
        self.lp_k = 1.0 - (-std::f32::consts::TAU * 800.0 / rate as f32).exp();
        self.unvoiced = ((0.005 * rate as f32).round() as u32).max(2);
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.inl.fill(0.0);
        self.inr.fill(0.0);
        self.mid.fill(0.0);
        self.mono.fill(0.0);
        self.t = 0;
        self.marks = [0; MARKS];
        self.count = 1;
        self.lp = [0.0; 2];
        self.hint = self.unvoiced as f32;
        self.for_period = -1.0;
    }

    /// Takes the next frame and lays the next mark when it can: on the
    /// highest peak from 3/4 to 5/4 of `period` after the last while voiced
    /// (`period` > 0), else 5 ms after it.
    #[inline]
    pub(crate) fn push(&mut self, l: f32, r: f32, period: f32) {
        self.t += 1;
        let at = self.t as usize & self.mask;
        self.inl[at] = l;
        self.inr[at] = r;
        let m = 0.5 * (l + r);
        self.mid[at] = m;
        self.lp[0] += self.lp_k * (m - self.lp[0]);
        self.lp[1] += self.lp_k * (self.lp[0] - self.lp[1]);
        self.lp[0] = crate::flush(self.lp[0]);
        self.lp[1] = crate::flush(self.lp[1]);
        self.mono[at] = self.lp[1];
        if period != self.for_period {
            self.plan(period);
        }
        if self.t < self.due {
            return;
        }
        let next = if period > 0.0 {
            let (a, b) = self.search;
            let mut best = a;
            let mut top = f32::MIN;
            for i in a..=b {
                let v = self.mono[i as usize & self.mask];
                if v > top {
                    top = v;
                    best = i;
                }
            }
            best
        } else {
            self.search.0
        };
        self.marks[self.count % MARKS] = next;
        self.count += 1;
        self.plan(period);
    }

    /// Works out where the next mark goes and when it can be laid: on the
    /// highest peak from 3/4 to 5/4 of `period` after the last while voiced,
    /// laid once that stretch has gone by; 5 ms after the last otherwise.
    /// (32-bit conversions: a float to 64 bits is a library call on the
    /// MPC's ARM.)
    fn plan(&mut self, period: f32) {
        self.for_period = period;
        let last = self.mark(self.count - 1);
        if period > 0.0 {
            self.hint = period;
            let (a, b) = (
                last + ((0.75 * period) as u32).max(1) as u64,
                last + (1.25 * period) as u32 as u64 + 1,
            );
            self.search = (a, b);
            self.due = b + 1;
        } else {
            self.hint = self.unvoiced as f32;
            let n = last + self.unvoiced as u64;
            self.search = (n, n);
            self.due = n + 1;
        }
    }

    #[inline]
    fn mark(&self, j: usize) -> u64 {
        self.marks[j % MARKS]
    }

    /// The oldest mark kept.
    #[inline]
    fn oldest(&self) -> usize {
        self.count.saturating_sub(MARKS)
    }

    /// The distance from mark `j` back to the one before it.
    #[inline]
    fn before(&self, j: usize) -> f32 {
        if j > self.oldest() {
            ((self.mark(j) - self.mark(j - 1)) as f32).max(2.0)
        } else {
            self.hint.max(2.0)
        }
    }

    /// The distance from mark `j` on to the one after it.
    #[inline]
    fn after(&self, j: usize) -> f32 {
        if j + 1 < self.count {
            ((self.mark(j + 1) - self.mark(j)) as f32).max(2.0)
        } else {
            self.hint.max(2.0)
        }
    }

    /// Moves `j` to the mark nearest the time `a`.
    #[inline]
    fn nearest(&self, a: f64, j: &mut usize) {
        let (lo, hi) = (self.oldest(), self.count - 1);
        let mut k = (*j).clamp(lo, hi);
        while k < hi && 0.5 * (self.mark(k) + self.mark(k + 1)) as f64 <= a {
            k += 1;
        }
        while k > lo && 0.5 * (self.mark(k - 1) + self.mark(k)) as f64 > a {
            k -= 1;
        }
        *j = k;
    }

    /// The input `delay` frames ago (whole frames).
    #[inline]
    pub(crate) fn dry(&self, delay: usize) -> (f32, f32) {
        let at = (self.t as usize).wrapping_sub(delay) & self.mask;
        (self.inl[at], self.inr[at])
    }
}

/// The rising half of a grain's window, 0 to 1 over `x` from 0 to 1: a
/// smoothstep, within 0.02 of `sin²` and three multiplies. The falling half
/// is one minus it, so a grain's tail and the next grain's head always sum
/// to exactly one.
#[inline]
fn rise(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

/// One shifted voice (see the module docs): where its next grain is
/// centred, the mark it reads, and what its grains have laid down ahead.
pub(crate) struct Voice {
    next: f64,
    /// The mark the next grain reads (an index into [`Grains`]' marks), its
    /// time, and the gaps back and on from it; worked out again only when a
    /// mark has been laid since (`seen`, the marks' count then).
    j: usize,
    m: u64,
    left: f32,
    right: f32,
    seen: usize,
    /// The frame the next grain starts on (when it is laid).
    start: u64,
    fresh: bool,
    accl: Vec<f32>,
    accr: Vec<f32>,
    wsum: Vec<f32>,
    mask: usize,
    stereo: bool,
}

impl Voice {
    pub(crate) fn new(stereo: bool) -> Voice {
        Voice {
            next: 0.0,
            j: 0,
            m: 0,
            left: 2.0,
            right: 2.0,
            seen: usize::MAX,
            start: 0,
            fresh: true,
            accl: Vec::new(),
            accr: Vec::new(),
            wsum: Vec::new(),
            mask: 0,
            stereo,
        }
    }

    pub(crate) fn prepare(&mut self, longest: usize) {
        let n = (4 * longest + 16).next_power_of_two();
        self.accl = vec![0.0; n];
        self.accr = vec![0.0; if self.stereo { n } else { 0 }];
        self.wsum = vec![0.0; n];
        self.mask = n - 1;
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.accl.fill(0.0);
        self.accr.fill(0.0);
        self.wsum.fill(0.0);
        self.next = 0.0;
        self.j = 0;
        self.seen = usize::MAX;
        self.fresh = true;
    }

    /// Lays the grains due by this frame (`g` has just taken it), `ratio`
    /// times the pitch, `latency` frames behind, and returns this frame's
    /// output (left, right; mono voices put the input's middle in both).
    ///
    /// A grain centred at output time `s` reads the mark nearest `s -
    /// latency`; its rising half is as wide as the gap back to the mark
    /// before, its falling half the gap on to the mark after, and the next
    /// grain comes that second gap over `ratio` later. At ratio 1 each grain
    /// lands on the next mark and the halves meet exactly, so the voice is
    /// the input, `latency` frames late, sample for sample; while the ratio
    /// sits at 1 a grain that is off its mark (after a shift, or where the
    /// marks ran ahead) eases back by half a percent of a period per grain.
    #[inline]
    pub(crate) fn run(&mut self, g: &Grains, ratio: f32, latency: usize) -> (f32, f32) {
        // Most frames lay nothing and only read out.
        if self.fresh || self.seen != g.count || g.t >= self.start {
            self.advance(g, ratio, latency);
        }
        let at = g.t as usize & self.mask;
        let norm = 1.0 / self.wsum[at].max(1.0);
        let l = self.accl[at] * norm;
        self.accl[at] = 0.0;
        self.wsum[at] = 0.0;
        let r = if self.stereo {
            let r = self.accr[at] * norm;
            self.accr[at] = 0.0;
            r
        } else {
            l
        };
        (l, r)
    }

    /// Finds the next grain's mark again when a mark has been laid, and
    /// lays every grain due by now (out of line: it runs about once a
    /// period).
    #[inline(never)]
    fn advance(&mut self, g: &Grains, ratio: f32, latency: usize) {
        let t = g.t;
        if self.fresh {
            // The first grain sits on the first mark.
            self.fresh = false;
            self.next = latency as f64;
        }
        loop {
            let a = self.next - latency as f64;
            if self.seen != g.count {
                g.nearest(a, &mut self.j);
                self.m = g.mark(self.j);
                self.left = g.before(self.j);
                self.right = g.after(self.j);
                self.seen = g.count;
                // The first frame past `next - left` (whole frames, so the
                // test in `run` is one integer compare: a 64-bit integer to
                // a float is a library call on the MPC's ARM).
                let s = (self.next - self.left as f64).max(0.0);
                let w = s as u64;
                self.start = if (w as f64) < s { w + 1 } else { w };
            }
            if t < self.start {
                break;
            }
            self.lay(g);
            let ratio = ratio.clamp(0.25, 4.0);
            let mut step = (self.right / ratio) as f64;
            if (ratio - 1.0).abs() < 0.002 {
                let ease = 0.005 * self.right as f64;
                step -= (a - self.m as f64).clamp(-ease, ease);
            }
            self.next += step;
            self.seen = usize::MAX;
        }
    }

    /// Lays one grain, centred at `next`, from mark `m`. Output frame
    /// `o0 + k` takes the input `k - frac` frames from the mark: one
    /// fraction for the whole grain, so each frame is two reads and a
    /// blend, all in 32-bit offsets from the newest frame.
    fn lay(&mut self, g: &Grains) {
        let t = g.t;
        let o0 = self.next as u64;
        let frac = (self.next - o0 as f64) as f32;
        let (left, right) = (self.left, self.right);
        // Where o0 and the mark are from now (o0 may be just behind).
        let orel = o0 as i64 - t as i64;
        let mrel = (self.m as i64 - t as i64) as i32;
        // The input under k is (mark + k + shift) and the next frame,
        // blended by `f`.
        let (shift, f) = if frac > 0.0 {
            (-1, 1.0 - frac)
        } else {
            (0, 0.0)
        };
        // No read past the newest frame (only near the bottom of the range,
        // where the marks run late).
        let reach = -1 - mrel - shift;
        // Frames before now are already out.
        let first = (-orel).max(i32::MIN as i64) as i32;
        let lo = ((frac - left).floor_fast() + 1).max(first);
        let hi = (frac + right).ceil_fast() - 1;
        let tz = t as usize;
        let o_base = tz.wrapping_add(orel as isize as usize);
        let m_base = tz.wrapping_add((mrel + shift) as isize as usize);
        // The rising half (k before the centre), then the falling half.
        let mid = if frac > 0.0 { 1 } else { 0 };
        let il = 1.0 / left;
        for k in lo..mid.min(hi + 1) {
            let w = rise((k as f32 - frac + left) * il);
            self.add(g, o_base, m_base, k, reach, f, w);
        }
        let ir = 1.0 / right;
        for k in mid.max(lo)..=hi {
            let w = 1.0 - rise((k as f32 - frac) * ir);
            self.add(g, o_base, m_base, k, reach, f, w);
        }
    }

    /// One frame of a grain: the input at `k` (no further than `reach`)
    /// blended by `f`, weighted `w`, onto output frame `k`.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn add(
        &mut self,
        g: &Grains,
        o_base: usize,
        m_base: usize,
        k: i32,
        reach: i32,
        f: f32,
        w: f32,
    ) {
        let i = m_base.wrapping_add(k.min(reach) as isize as usize);
        let (i0, i1) = (i & g.mask, i.wrapping_add(1) & g.mask);
        let at = o_base.wrapping_add(k as isize as usize) & self.mask;
        if self.stereo {
            let (a, b) = (g.inl[i0], g.inl[i1]);
            self.accl[at] += w * (a + (b - a) * f);
            let (a, b) = (g.inr[i0], g.inr[i1]);
            self.accr[at] += w * (a + (b - a) * f);
        } else {
            let (a, b) = (g.mid[i0], g.mid[i1]);
            self.accl[at] += w * (a + (b - a) * f);
        }
        self.wsum[at] += w;
    }
}

/// Floor and ceiling to `i32` without the library call `f32::floor` is
/// on the MPC's ARM (values well inside `i32`).
trait FastRound {
    fn floor_fast(self) -> i32;
    fn ceil_fast(self) -> i32;
}

impl FastRound for f32 {
    #[inline]
    fn floor_fast(self) -> i32 {
        let i = self as i32;
        if (i as f32) > self {
            i - 1
        } else {
            i
        }
    }
    #[inline]
    fn ceil_fast(self) -> i32 {
        let i = self as i32;
        if (i as f32) < self {
            i + 1
        } else {
            i
        }
    }
}

/// The latency for `range` at `rate`: two and a half of its longest
/// periods (a grain reaches a period either side of a mark that can be
/// half a period past its target time) and a little.
fn latency_for(rate: u32, range: usize) -> usize {
    let lo = RANGE_HZ[range.min(RANGE_HZ.len() - 1)].0;
    (2.5 * rate as f32 / lo).ceil() as usize + 2
}

/// The longest period any range listens for at `rate`, in frames.
fn longest(rate: u32) -> usize {
    (rate as f32 / RANGE_HZ[0].0).ceil() as usize + 2
}

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
    readonly: bool,
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
        readonly,
    }
}

/// [`PitchCorrect`] parameter ids.
pub mod tune_ids {
    pub const KEY: u16 = 0;
    pub const SCALE: u16 = 1;
    pub const SPEED: u16 = 2;
    pub const AMOUNT: u16 = 3;
    pub const RANGE: u16 = 4;
    pub const MIX: u16 = 5;
    /// Meter: the note heard ([`super::NOTE_NAMES`]; 0 for none).
    pub const NOTE: u16 = 6;
    /// Meter: the correction now, in cents.
    pub const CORRECTION: u16 = 7;
}

static TUNE_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.pitch_correct",
    name: "Pitch correct",
    kind: Kind::Effect,
};

static TUNE_PARAMS: [ParamInfo; 8] = [
    p(
        0,
        "Key",
        "Notes",
        0.0,
        11.0,
        0.0,
        Unit::None,
        Curve::Choice(KEYS),
        false,
    ),
    p(
        1,
        "Scale",
        "Notes",
        0.0,
        6.0,
        0.0,
        Unit::None,
        Curve::Choice(SCALES),
        false,
    ),
    p(
        2,
        "Speed",
        "Notes",
        0.0,
        200.0,
        20.0,
        Unit::Ms,
        Curve::Linear,
        false,
    ),
    p(
        3,
        "Amount",
        "Notes",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        4,
        "Range",
        "Voice",
        0.0,
        2.0,
        1.0,
        Unit::None,
        Curve::Choice(RANGES),
        false,
    ),
    p(
        5,
        "Mix",
        "Voice",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        6,
        "Note",
        "Meters",
        0.0,
        127.0,
        0.0,
        Unit::None,
        Curve::Choice(NOTE_NAMES),
        true,
    ),
    p(
        7,
        "Correction",
        "Meters",
        -600.0,
        600.0,
        0.0,
        Unit::Cents,
        Curve::Linear,
        true,
    ),
];

/// Pitch correct: pulls the voice to the nearest note of the key and scale,
/// over Speed (0 snaps: the hard, stepped effect), by Amount.
pub struct PitchCorrect {
    v: [f32; 8],
    rate: u32,
    tracker: Tracker,
    grains: Grains,
    voice: Voice,
    latency: usize,
    /// The correction wanted and the correction now, in semitones.
    target: f32,
    corr: f32,
    corr_k: f32,
    mix: Smooth,
}

impl Default for PitchCorrect {
    fn default() -> Self {
        Self::new()
    }
}

impl PitchCorrect {
    pub fn new() -> PitchCorrect {
        let v = defaults(&TUNE_PARAMS);
        let mut n = PitchCorrect {
            v,
            rate: 48_000,
            tracker: Tracker::new(),
            grains: Grains::new(),
            voice: Voice::new(true),
            latency: 0,
            target: 0.0,
            corr: 0.0,
            corr_k: 1.0,
            mix: Smooth::new(v[5] / 100.0),
        };
        n.prepare(48_000, mpfree_node::MAX_BLOCK);
        n
    }

    fn speed(&mut self) {
        let frames = self.v[tune_ids::SPEED as usize] * 0.001 * self.rate as f32;
        self.corr_k = if frames < 1.0 {
            1.0
        } else {
            1.0 - (-1.0 / frames).exp()
        };
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = TUNE_PARAMS.get(id as usize) else {
            return;
        };
        if p.readonly {
            return;
        }
        let v = p.clamp(value);
        let v = if matches!(p.curve, Curve::Choice(_)) {
            v.round()
        } else {
            v
        };
        self.v[id as usize] = v;
        match id {
            tune_ids::SPEED => self.speed(),
            tune_ids::MIX => self.mix.target = v / 100.0,
            tune_ids::RANGE => {
                self.tracker.set_range(v as usize);
                self.latency = latency_for(self.rate, v as usize);
            }
            _ => {}
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let (key, scale) = (self.v[0] as usize, self.v[1] as usize);
        let amount = self.v[3] / 100.0;
        for i in 0..l.len() {
            let mix = self.mix.next();
            if self.tracker.push(0.5 * (l[i] + r[i])) {
                let p = self.tracker.period;
                if p > 0.0 {
                    let midi = period_midi(p, self.rate);
                    let to = nearest_note(midi, key, scale) as f32;
                    self.target = (to - midi) * amount;
                    self.v[tune_ids::NOTE as usize] = (midi.round()).clamp(1.0, 127.0);
                } else {
                    self.target = 0.0;
                    self.v[tune_ids::NOTE as usize] = 0.0;
                }
            }
            self.corr = crate::flush(self.corr + self.corr_k * (self.target - self.corr));
            let period = self.tracker.period;
            self.grains.push(l[i], r[i], period);
            let ratio = fast_exp2(self.corr / 12.0);
            let (wl, wr) = self.voice.run(&self.grains, ratio, self.latency);
            let (dl, dr) = self.grains.dry(self.latency);
            l[i] = dl + mix * (wl - dl);
            r[i] = dr + mix * (wr - dr);
        }
        self.v[tune_ids::CORRECTION as usize] = (100.0 * self.corr).clamp(-600.0, 600.0);
    }
}

impl Node for PitchCorrect {
    fn info(&self) -> &'static NodeInfo {
        &TUNE_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &TUNE_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        let range = self.v[tune_ids::RANGE as usize] as usize;
        self.tracker.prepare(self.rate, range);
        self.grains.prepare(self.rate, longest(self.rate));
        self.voice.prepare(longest(self.rate));
        self.latency = latency_for(self.rate, range);
        self.mix.set_time(self.rate, 20.0);
        self.speed();
    }
    fn reset(&mut self) {
        self.tracker.reset();
        self.grains.reset();
        self.voice.reset();
        self.target = 0.0;
        self.corr = 0.0;
        self.mix.snap(self.mix.target);
        self.v[tune_ids::NOTE as usize] = 0.0;
        self.v[tune_ids::CORRECTION as usize] = 0.0;
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
    fn latency(&self) -> u32 {
        self.latency as u32
    }
    fn tail_frames(&self) -> u32 {
        self.latency as u32 + 2 * longest(self.rate) as u32
    }
}

/// [`Harmonizer`] parameter ids.
pub mod harm_ids {
    pub const KEY: u16 = 0;
    pub const SCALE: u16 = 1;
    /// Voice 1's interval ([`super::INTERVALS`]), level and pan.
    pub const VOICE1: u16 = 2;
    pub const LEVEL1: u16 = 3;
    pub const PAN1: u16 = 4;
    pub const VOICE2: u16 = 5;
    pub const LEVEL2: u16 = 6;
    pub const PAN2: u16 = 7;
    /// How quickly a voice moves to a new interval when the note changes.
    pub const GLIDE: u16 = 8;
    pub const DRY: u16 = 9;
    pub const RANGE: u16 = 10;
    /// Meter: the note heard ([`super::NOTE_NAMES`]; 0 for none).
    pub const NOTE: u16 = 11;
}

static HARM_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.harmonizer",
    name: "Harmonizer",
    kind: Kind::Effect,
};

static HARM_PARAMS: [ParamInfo; 12] = [
    p(
        0,
        "Key",
        "Notes",
        0.0,
        11.0,
        0.0,
        Unit::None,
        Curve::Choice(KEYS),
        false,
    ),
    p(
        1,
        "Scale",
        "Notes",
        0.0,
        6.0,
        1.0,
        Unit::None,
        Curve::Choice(SCALES),
        false,
    ),
    p(
        2,
        "Voice 1",
        "Voices",
        0.0,
        11.0,
        7.0,
        Unit::None,
        Curve::Choice(INTERVALS),
        false,
    ),
    p(
        3,
        "Level 1",
        "Voices",
        0.0,
        100.0,
        70.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        4,
        "Pan 1",
        "Voices",
        -100.0,
        100.0,
        -40.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        5,
        "Voice 2",
        "Voices",
        0.0,
        11.0,
        9.0,
        Unit::None,
        Curve::Choice(INTERVALS),
        false,
    ),
    p(
        6,
        "Level 2",
        "Voices",
        0.0,
        100.0,
        60.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        7,
        "Pan 2",
        "Voices",
        -100.0,
        100.0,
        40.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        8,
        "Glide",
        "Notes",
        0.0,
        200.0,
        30.0,
        Unit::Ms,
        Curve::Linear,
        false,
    ),
    p(
        9,
        "Dry",
        "Voices",
        0.0,
        100.0,
        100.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        10,
        "Range",
        "Notes",
        0.0,
        2.0,
        1.0,
        Unit::None,
        Curve::Choice(RANGES),
        false,
    ),
    p(
        11,
        "Note",
        "Meters",
        0.0,
        127.0,
        0.0,
        Unit::None,
        Curve::Choice(NOTE_NAMES),
        true,
    ),
];

/// One of the Harmonizer's voices: its shifter, the interval it is gliding
/// to and at, and its level.
struct Part {
    voice: Voice,
    target: f32,
    semis: f32,
    gain: Smooth,
}

/// Harmonizer: up to two voices a scale's intervals from the note sung,
/// each with its level and pan, over the dry voice. A voice is silent
/// while the input is unvoiced (a breath, a consonant).
pub struct Harmonizer {
    v: [f32; 12],
    rate: u32,
    tracker: Tracker,
    grains: Grains,
    parts: [Part; 2],
    latency: usize,
    glide_k: f32,
    dry: Smooth,
    pans: [Smooth; 2],
}

impl Default for Harmonizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Harmonizer {
    pub fn new() -> Harmonizer {
        let v = defaults(&HARM_PARAMS);
        let part = || Part {
            voice: Voice::new(false),
            target: 0.0,
            semis: 0.0,
            gain: Smooth::new(0.0),
        };
        let mut n = Harmonizer {
            v,
            rate: 48_000,
            tracker: Tracker::new(),
            grains: Grains::new(),
            parts: [part(), part()],
            latency: 0,
            glide_k: 1.0,
            dry: Smooth::new(v[9] / 100.0),
            pans: [Smooth::new(v[4] / 100.0), Smooth::new(v[7] / 100.0)],
        };
        n.prepare(48_000, mpfree_node::MAX_BLOCK);
        n
    }

    fn glide(&mut self) {
        let frames = self.v[harm_ids::GLIDE as usize] * 0.001 * self.rate as f32;
        self.glide_k = if frames < 1.0 {
            1.0
        } else {
            1.0 - (-1.0 / frames).exp()
        };
    }

    fn apply(&mut self, id: u16, value: f32) {
        let Some(p) = HARM_PARAMS.get(id as usize) else {
            return;
        };
        if p.readonly {
            return;
        }
        let v = p.clamp(value);
        let v = if matches!(p.curve, Curve::Choice(_)) {
            v.round()
        } else {
            v
        };
        self.v[id as usize] = v;
        match id {
            harm_ids::GLIDE => self.glide(),
            harm_ids::DRY => self.dry.target = v / 100.0,
            harm_ids::PAN1 => self.pans[0].target = v / 100.0,
            harm_ids::PAN2 => self.pans[1].target = v / 100.0,
            harm_ids::RANGE => {
                self.tracker.set_range(v as usize);
                self.latency = latency_for(self.rate, v as usize);
            }
            _ => {}
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        let (key, scale) = (self.v[0] as usize, self.v[1] as usize);
        let picks = [
            self.v[harm_ids::VOICE1 as usize] as usize,
            self.v[harm_ids::VOICE2 as usize] as usize,
        ];
        let levels = [
            self.v[harm_ids::LEVEL1 as usize] / 100.0,
            self.v[harm_ids::LEVEL2 as usize] / 100.0,
        ];
        for i in 0..l.len() {
            if self.tracker.push(0.5 * (l[i] + r[i])) {
                let p = self.tracker.period;
                let midi = if p > 0.0 {
                    Some(period_midi(p, self.rate))
                } else {
                    None
                };
                self.v[harm_ids::NOTE as usize] = midi.map_or(0.0, |m| m.round().clamp(1.0, 127.0));
                for (k, part) in self.parts.iter_mut().enumerate() {
                    let semis = midi.and_then(|m| {
                        interval_semis(picks[k], nearest_note(m, key, scale), key, scale)
                    });
                    match semis {
                        Some(s) => {
                            part.target = s as f32;
                            part.gain.target = levels[k];
                        }
                        None => part.gain.target = 0.0,
                    }
                }
            }
            let period = self.tracker.period;
            self.grains.push(l[i], r[i], period);
            let (dl, dr) = self.grains.dry(self.latency);
            let dry = self.dry.next();
            let (mut ol, mut or) = (dry * dl, dry * dr);
            for (k, part) in self.parts.iter_mut().enumerate() {
                part.semis = crate::flush(part.semis + self.glide_k * (part.target - part.semis));
                let gain = part.gain.next();
                let pan = self.pans[k].next();
                let ratio = fast_exp2(part.semis / 12.0);
                // Laid down even while silent, so a voice comes back in on
                // its own grains.
                let (y, _) = part.voice.run(&self.grains, ratio, self.latency);
                // Equal-power pan from a parabola (no sine on the audio
                // thread): the sides at 1 and 0, 0.707 in the middle.
                let x = 0.5 * (pan + 1.0);
                let (gl, gr) = (1.0 - x * x, 1.0 - (1.0 - x) * (1.0 - x));
                ol += gain * gl * y;
                or += gain * gr * y;
            }
            l[i] = ol;
            r[i] = or;
        }
    }
}

impl Node for Harmonizer {
    fn info(&self) -> &'static NodeInfo {
        &HARM_INFO
    }
    fn params(&self) -> &'static [ParamInfo] {
        &HARM_PARAMS
    }
    fn set_param(&mut self, id: u16, value: f32) {
        self.apply(id, value);
    }
    fn param(&self, id: u16) -> f32 {
        self.v.get(id as usize).copied().unwrap_or(f32::NAN)
    }
    fn prepare(&mut self, rate: u32, _max_block: usize) {
        self.rate = rate.max(1);
        let range = self.v[harm_ids::RANGE as usize] as usize;
        self.tracker.prepare(self.rate, range);
        self.grains.prepare(self.rate, longest(self.rate));
        for part in &mut self.parts {
            part.voice.prepare(longest(self.rate));
            part.gain.set_time(self.rate, 20.0);
        }
        self.latency = latency_for(self.rate, range);
        self.dry.set_time(self.rate, 20.0);
        for s in &mut self.pans {
            s.set_time(self.rate, 20.0);
        }
        self.glide();
    }
    fn reset(&mut self) {
        self.tracker.reset();
        self.grains.reset();
        for part in &mut self.parts {
            part.voice.reset();
            part.target = 0.0;
            part.semis = 0.0;
            part.gain.snap(0.0);
            part.gain.target = 0.0;
        }
        self.dry.snap(self.dry.target);
        for s in &mut self.pans {
            s.snap(s.target);
        }
        self.v[harm_ids::NOTE as usize] = 0.0;
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
    fn latency(&self) -> u32 {
        self.latency as u32
    }
    fn tail_frames(&self) -> u32 {
        self.latency as u32 + 2 * longest(self.rate) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_snap_to_the_scale() {
        // C major: C# goes to C or D, whichever is nearer.
        assert_eq!(nearest_note(61.4, 0, 1), 62);
        assert_eq!(nearest_note(60.6, 0, 1), 60);
        assert_eq!(nearest_note(64.4, 0, 1), 64);
        // Chromatic: the nearest semitone.
        assert_eq!(nearest_note(61.4, 0, 0), 61);
        // A minor pentatonic (A C D E G).
        assert_eq!(nearest_note(70.0, 9, 5), 69);
    }

    #[test]
    fn intervals_follow_the_key() {
        // C major: a 3rd up from E is G (minor), from C is E (major).
        assert_eq!(interval_semis(7, 64, 0, 1), Some(3));
        assert_eq!(interval_semis(7, 60, 0, 1), Some(4));
        // A 5th up from B is F (diminished) in C major.
        assert_eq!(interval_semis(9, 71, 0, 1), Some(6));
        // Octaves are octaves; Off is none.
        assert_eq!(interval_semis(11, 65, 0, 1), Some(12));
        assert_eq!(interval_semis(1, 65, 0, 1), Some(-12));
        assert_eq!(interval_semis(0, 65, 0, 1), None);
        // Chromatic: the usual sizes.
        assert_eq!(interval_semis(7, 61, 0, 0), Some(4));
        assert_eq!(interval_semis(5, 61, 0, 0), Some(-3));
    }
}
