//! Vocal effects: **Pitch correct** pulls a voice to the notes of a key,
//! and the **Harmonizer** adds up to two voices a scale's intervals away.
//!
//! Both rest on three parts:
//!
//! - [`Tracker`]: the voice's pitch, by YIN (de Cheveigné and Kawahara,
//!   JASA 2002) on the input low-passed and taken down to about 12 kHz,
//!   every 5 ms: the difference function, its cumulative-mean
//!   normalisation, the first dip under 0.15 (else the deepest under
//!   0.25), refined by a parabola. Quiet input (under -50 dBFS) or a dip
//!   that isn't deep enough is unvoiced. [`Grains::refine`] then finds the
//!   period again at the full rate around it, where a high note's period
//!   is many samples rather than a few.
//! - [`Looks`]: each look's note at the middle of the stretch it heard.
//!   The grains a voice lays now read the input the latency behind, so the
//!   note is read for that stretch, between looks on a straight line: the
//!   correction lines up with the sound it corrects (a vibrato is followed,
//!   not chased).
//! - [`Grains`] and [`Voice`]: a pitch-synchronous overlap-add (PSOLA)
//!   shifter. Marks are laid one period apart on the waveform's peaks while
//!   the voice is voiced (every 5 ms otherwise), each between frames (the
//!   top of a parabola through the peak); a voice lays a grain from the
//!   mark nearest its target time, a period either side, every period
//!   divided by its ratio, read between frames at the mark's fraction less
//!   the grain's. The grains keep their content, so the pitch moves and
//!   the formants (the vowel, the throat) stay: no chipmunk. At ratio 1 it
//!   gives the input back exactly, delayed. The latency is two and a half
//!   of the Range's longest periods.
//!
//! A grain's window rises (a smootherstep, near `sin²`) over the gap back
//! to the mark before and falls as one minus that over the gap to the mark
//! after, so neighbouring grains sum to one; where faster grains overlap
//! more (pitch up) the sum of the windows divides it back down. A grain is
//! read through the pitch shifter's twelve-tap sinc at its one fraction
//! (taps worked out once a grain), so every grain keeps the whole top;
//! harmony voices then lose any DC their uneven overlap leaves.
//!
//! Pitch correct's **Flex-Tune** corrects only notes sung near the note
//! (the correction fades out past half of a reach that shrinks from half a
//! semitone to 5 cents), and **Humanize** slows the correction on a note
//! held past 100 ms (by up to 400 ms more), so a held note's vibrato
//! survives while quick notes still snap. A **Range** change dips the
//! output for 5 ms either side of it, since the tracker starts again and
//! the latency changes.

use mpfree_node::{Ctx, Curve, Kind, Node, NodeEvent, NodeInfo, ParamInfo, Smooth, Timed, Unit};

use crate::pitch::{taps_after, READ_TAPS};
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
pub(crate) const GATE_RMS: f32 = 0.003_16;

/// What a [`Tracker`] listens for: the lowest and highest pitch, the rate
/// its search runs at (about), its low-pass as a fraction of that rate,
/// and how often it looks, in seconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Band {
    pub lo: f32,
    pub hi: f32,
    pub rate: f32,
    pub cut: f32,
    pub every: f32,
}

/// The vocal effects' ranges (Low, Mid, High). Each searches at about ten
/// times its highest note, so each costs about the same (the search goes
/// as the square of the rate).
pub(crate) const VOICE_BANDS: [Band; 3] = [
    Band {
        lo: 60.0,
        hi: 600.0,
        rate: 6_000.0,
        cut: 0.2,
        every: 0.005,
    },
    Band {
        lo: 90.0,
        hi: 900.0,
        rate: 8_000.0,
        cut: 0.2,
        every: 0.005,
    },
    Band {
        lo: 150.0,
        hi: 1500.0,
        rate: 12_000.0,
        cut: 0.2,
        every: 0.005,
    },
];

/// The voice's pitch (see the module docs).
pub(crate) struct Tracker {
    bands: &'static [Band],
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
    pub(crate) fn new(bands: &'static [Band]) -> Tracker {
        Tracker {
            bands,
            rate: 48_000,
            range: 0,
            dec: 1,
            phase: 0,
            rate_dec: bands[0].rate,
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

    /// The decimation for `band` at `rate`.
    fn dec_for(rate: u32, band: &Band) -> usize {
        ((rate as f32 / band.rate).round() as usize).clamp(1, 16)
    }

    /// Sizes everything for `rate` (allocates), with room for every range.
    pub(crate) fn prepare(&mut self, rate: u32, range: usize) {
        self.rate = rate;
        let most = self
            .bands
            .iter()
            .map(|b| {
                let rate_dec = rate as f32 / Self::dec_for(rate, b) as f32;
                (rate_dec / b.lo).ceil() as usize + 2
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
        let range = range.min(self.bands.len() - 1);
        if range == self.range {
            return;
        }
        self.range = range;
        let band = self.bands[range];
        self.dec = Self::dec_for(self.rate, &band);
        self.rate_dec = self.rate as f32 / self.dec as f32;
        self.aa.set(
            SvfMode::LowPass,
            band.cut * self.rate_dec,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            self.rate,
        );
        self.hop = ((self.rate_dec * band.every) as usize).max(1);
        let (lo, hi) = (band.lo, band.hi);
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
                // A dip between two lags (a short period on the coarse
                // grid) can sit above the threshold at both: a local
                // minimum counts by the bottom of the parabola through it.
                let a = self.d[prev - 1];
                let low = if pv <= a && pv <= v {
                    let den = a - 2.0 * pv + v;
                    if den > 1e-9 {
                        pv - (a - v) * (a - v) / (8.0 * den)
                    } else {
                        pv
                    }
                } else {
                    pv
                };
                if low < YIN_THRESHOLD {
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
pub(crate) fn diff(a: &[f32], b: &[f32]) -> f32 {
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
    /// The input (each frame twice, a buffer apart).
    inl: Vec<f32>,
    inr: Vec<f32>,
    /// The middle, `(l + r) / 2`, for mono voices.
    mid: Vec<f32>,
    /// The middle low-passed, to find peaks and the period on (each frame
    /// twice, as the input).
    mono: Vec<f32>,
    mask: usize,
    /// The frame being processed.
    t: u64,
    /// Each mark's frame and how far past it the peak is (0 to 1): marks
    /// sit between frames, so a period that isn't a whole number of frames
    /// isn't rounded at every grain.
    marks: [u64; MARKS],
    fracs: [f32; MARKS],
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
    /// Where the next mark goes while unvoiced (a frame and a fraction).
    unvoiced_at: (u64, f32),
    /// [`Grains::refine`]'s window, shortest and longest, and the most it
    /// reads.
    min_win: usize,
    max_win: usize,
    most: usize,
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
            fracs: [0.0; MARKS],
            count: 0,
            lp: [0.0; 2],
            lp_k: 0.1,
            unvoiced: 240,
            hint: 240.0,
            for_period: -1.0,
            due: 0,
            search: (0, 0),
            unvoiced_at: (0, 0.0),
            min_win: 1,
            max_win: 1,
            most: 0,
        }
    }

    /// Sizes for `rate` with periods up to `longest` frames (allocates).
    pub(crate) fn prepare(&mut self, rate: u32, longest: usize) {
        let n = (8 * longest + 16).next_power_of_two();
        // The input is written twice, a buffer apart, so a grain's
        // twelve-tap read is one slice wherever it falls.
        self.inl = vec![0.0; 2 * n];
        self.inr = vec![0.0; 2 * n];
        self.mid = vec![0.0; 2 * n];
        self.mono = vec![0.0; 2 * n];
        self.mask = n - 1;
        self.lp_k = 1.0 - (-std::f32::consts::TAU * 800.0 / rate as f32).exp();
        self.unvoiced = ((0.005 * rate as f32).round() as u32).max(2);
        self.min_win = (0.004 * rate as f32) as usize;
        self.max_win = longest.max(self.min_win);
        self.most = (self.max_win + longest + 64).min(n);
        self.reset();
    }

    pub(crate) fn reset(&mut self) {
        self.inl.fill(0.0);
        self.inr.fill(0.0);
        self.mid.fill(0.0);
        self.mono.fill(0.0);
        self.t = 0;
        self.marks = [0; MARKS];
        self.fracs = [0.0; MARKS];
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
        let n = self.mask + 1;
        self.inl[at] = l;
        self.inl[at + n] = l;
        self.inr[at] = r;
        self.inr[at + n] = r;
        let m = 0.5 * (l + r);
        self.mid[at] = m;
        self.mid[at + n] = m;
        self.lp[0] += self.lp_k * (m - self.lp[0]);
        self.lp[1] += self.lp_k * (self.lp[0] - self.lp[1]);
        self.lp[0] = crate::flush(self.lp[0]);
        self.lp[1] = crate::flush(self.lp[1]);
        self.mono[at] = self.lp[1];
        self.mono[at + n] = self.lp[1];
        if period != self.for_period {
            self.plan(period);
        }
        if self.t < self.due {
            return;
        }
        let (next, frac) = if period > 0.0 {
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
            // Between frames: the top of a parabola through the peak and
            // its neighbours (both written: the search ends a frame early).
            let at = |i: u64| self.mono[i as usize & self.mask];
            let (l, c, r) = (at(best - 1), top, at(best + 1));
            let den = l - 2.0 * c + r;
            let off = if den < 0.0 {
                (0.5 * (l - r) / den).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            if off < 0.0 {
                (best - 1, 1.0 + off)
            } else {
                (best, off)
            }
        } else {
            self.unvoiced_at
        };
        let k = self.count % MARKS;
        self.marks[k] = next;
        self.fracs[k] = frac;
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
        let j = self.count - 1;
        let (last, frac) = (self.mark(j), self.frac(j));
        if period > 0.0 {
            self.hint = period;
            // From the mark's own fraction: whole frames after `last`.
            let (a, b) = (
                ((0.75 * period + frac) as u32).max(1) as u64,
                (1.25 * period + frac) as u32 as u64 + 1,
            );
            self.search = (last + a, last + b);
            self.due = last + b + 1;
        } else {
            self.hint = self.unvoiced as f32;
            let n = last + self.unvoiced as u64;
            self.unvoiced_at = (n, frac);
            self.search = (n, n);
            self.due = n + 1;
        }
    }

    #[inline]
    fn mark(&self, j: usize) -> u64 {
        self.marks[j % MARKS]
    }

    #[inline]
    fn frac(&self, j: usize) -> f32 {
        self.fracs[j % MARKS]
    }

    /// Mark `j` less mark `i`, in frames (between frames; marks a few
    /// periods apart, so 32 bits: a 64-bit integer to a float is a library
    /// call on the MPC's ARM).
    #[inline]
    fn gap(&self, i: usize, j: usize) -> f32 {
        (self.mark(j) - self.mark(i)) as u32 as f32 + (self.frac(j) - self.frac(i))
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
            self.gap(j - 1, j).max(2.0)
        } else {
            self.hint.max(2.0)
        }
    }

    /// The distance from mark `j` on to the one after it.
    #[inline]
    fn after(&self, j: usize) -> f32 {
        if j + 1 < self.count {
            self.gap(j, j + 1).max(2.0)
        } else {
            self.hint.max(2.0)
        }
    }

    /// Moves `j` to the mark nearest the time `a`.
    #[inline]
    fn nearest(&self, a: f64, j: &mut usize) {
        let (lo, hi) = (self.oldest(), self.count - 1);
        let mut k = (*j).clamp(lo, hi);
        let mid = |k: usize| {
            0.5 * ((self.mark(k) + self.mark(k + 1)) as f64
                + f64::from(self.frac(k) + self.frac(k + 1)))
        };
        while k < hi && mid(k) <= a {
            k += 1;
        }
        while k > lo && mid(k - 1) > a {
            k -= 1;
        }
        *j = k;
    }

    /// The period near the tracker's `coarse` one, exactly: the full-rate
    /// difference function of the low-passed middle (the copy marks are
    /// found on) over the newest frames (a period, at least 4 ms), from the
    /// lag nearest `coarse` downhill to its dip (at most `slack` frames
    /// away), refined by a parabola. The tracker looks at a decimated copy,
    /// where a high note's period is only a few samples and a parabola
    /// through them misses by up to 10 cents. Low-passed, the dip is a
    /// parabola: on the full band a bright note's dip is a V, and a
    /// parabola through it missed by up to 3 cents, a look in ten, which
    /// shook the correction and roughened the voice (a corrected saw's
    /// grains 48 dB down, 63 dB once steady).
    pub(crate) fn refine(&self, coarse: f32, slack: usize) -> f32 {
        let c = coarse.round() as usize;
        let win = c.max(self.min_win).min(self.max_win);
        let n = win + c + slack + 1;
        if n > self.most || c <= slack + 2 {
            return coarse;
        }
        // The newest n frames, oldest first (the frame being processed is
        // not written yet), side by side in the doubled copy.
        let from = (self.t as usize + 1).wrapping_sub(n) & self.mask;
        let x = &self.mono[from..from + n];
        let d = |lag: usize| diff(&x[n - win..], &x[n - win - lag..n - lag]);
        let (lo, hi) = (c - slack, c + slack);
        let (mut at, mut here) = (c, d(c));
        let (mut down, mut up) = (d(c - 1), d(c + 1));
        while down < here && at - 1 > lo {
            at -= 1;
            up = here;
            here = down;
            down = d(at - 1);
        }
        while up < here && at + 1 < hi {
            at += 1;
            down = here;
            here = up;
            up = d(at + 1);
        }
        let den = down - 2.0 * here + up;
        let shift = if den > 1e-12 {
            (0.5 * (down - up) / den).clamp(-0.5, 0.5)
        } else {
            0.0
        };
        at as f32 + shift
    }

    /// The input `delay` frames ago (whole frames).
    #[inline]
    pub(crate) fn dry(&self, delay: usize) -> (f32, f32) {
        let at = (self.t as usize).wrapping_sub(delay) & self.mask;
        (self.inl[at], self.inr[at])
    }
}

/// The rising half of a grain's window, 0 to 1 over `x` from 0 to 1: a
/// smootherstep (flat in slope and curve at both ends), five multiplies.
/// The falling half is one minus it, so a grain's tail and the next
/// grain's head always sum to exactly one. Where grains overlap unevenly
/// (a harmony a 5th up) the window's corners show as overtones of the new
/// note; a smoothstep's (flat in slope only) put them 3 dB higher,
/// over 6 kHz at the edge of the click target.
#[inline]
fn rise(x: f32) -> f32 {
    x * x * x * (10.0 + x * (6.0 * x - 15.0))
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
    mf: f32,
    /// The laying grain's read taps (one fraction for the whole grain).
    h: [f32; READ_TAPS],
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
            mf: 0.0,
            h: [0.0; READ_TAPS],
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
    /// sits near 1 a grain that is off its mark (after a shift, or where
    /// the marks ran ahead) eases back by up to 0.1 % of a period per grain.
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
                self.mf = g.frac(self.j);
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
            // Fully within 0.03 % of 1 (half a cent), fading out by 0.2 %:
            // a ratio that wanders across the edge never jumps the grains
            // (an edge that switched the ease on and off put 9-cent pulses
            // on a note held a few cents from its target).
            let off = (ratio - 1.0).abs();
            if off < EASE_TO {
                let e = ((EASE_TO - off) / (EASE_TO - EASE_FULL)).min(1.0);
                let ease = f64::from(EASE_RATE * self.right);
                step -= f64::from(e) * (a - self.m as f64 - f64::from(self.mf)).clamp(-ease, ease);
            }
            self.next += step;
            self.seen = usize::MAX;
        }
    }

    /// Lays one grain, centred at `next`, from mark `m`. Output frame
    /// `o0 + k` takes the input `k - frac` frames from the mark: one
    /// fraction for the whole grain, so its twelve taps are worked out once
    /// and each frame is twelve multiplies, all in 32-bit offsets from the
    /// newest frame. A blend of two frames (as before) dulls the top by a
    /// different amount at each fraction, and the grains' fractions differ,
    /// so a corrected saw was rough, 36 dB down.
    fn lay(&mut self, g: &Grains) {
        let t = g.t;
        let o0 = self.next as u64;
        let frac = (self.next - o0 as f64) as f32;
        let (left, right) = (self.left, self.right);
        // Where o0 and the mark are from now (o0 may be just behind).
        let orel = o0 as i64 - t as i64;
        let mrel = (self.m as i64 - t as i64) as i32;
        // The input under k is `f` past (mark + k + shift): the mark's
        // fraction less the grain's.
        let c = self.mf - frac;
        let (shift, f) = if c < 0.0 { (-1, 1.0 + c) } else { (0, c) };
        // On a whole frame (a voice at ratio 1 on its marks) the grain is
        // the input itself: the taps' gentle top would take 0.6 dB off
        // white noise.
        self.h = if f < WHOLE {
            WHOLE_TAPS[0]
        } else if f > 1.0 - WHOLE {
            WHOLE_TAPS[1]
        } else {
            taps_after(f)
        };
        // No read past the newest frame (only near the bottom of the range,
        // where the marks run late): the newest tap is 6 frames on.
        let reach = -(READ_TAPS as i32 / 2) - mrel - shift;
        // Frames before now are already out.
        let first = (-orel).max(i32::MIN as i64) as i32;
        let lo = ((frac - left).floor_fast() + 1).max(first);
        let hi = (frac + right).ceil_fast() - 1;
        let tz = t as usize;
        let o_base = tz.wrapping_add(orel as isize as usize);
        let m_base = tz.wrapping_add((mrel + shift) as isize as usize);
        // The rising half (k before the centre), then the falling half.
        let mid = if frac > 0.0 { 1 } else { 0 };
        let at = Span {
            o_base,
            m_base,
            reach,
        };
        let (c, il) = (left - frac, 1.0 / left);
        self.span(g, &at, lo, mid.min(hi + 1), |k| rise((k + c) * il));
        let ir = 1.0 / right;
        self.span(g, &at, mid.max(lo), hi + 1, |k| 1.0 - rise((k - frac) * ir));
    }

    /// Lays frames `k0` to `k1` (not included) of a grain, weighted by
    /// `w(k)`, [`SPAN`] at a time: the weights, then the reads (four frames
    /// at a time through the taps, the input side by side), then the sums.
    #[inline(always)]
    fn span(&mut self, g: &Grains, at: &Span, k0: i32, k1: i32, w: impl Fn(f32) -> f32) {
        let mut k = k0;
        while k < k1 {
            let len = ((k1 - k) as usize).min(SPAN);
            let mut ws = [0.0f32; SPAN];
            for (j, v) in ws[..len].iter_mut().enumerate() {
                *v = w((k + j as i32) as f32);
            }
            // Frames past `reach` read the newest the grain may.
            let free = ((at.reach - k + 1).max(0) as usize).min(len);
            let from = |k: i32| {
                at.m_base
                    .wrapping_add(k as isize as usize)
                    .wrapping_sub(READ_TAPS / 2 - 1)
                    & g.mask
            };
            let s = from(k);
            let mut yl = [0.0f32; SPAN];
            let mut yr = [0.0f32; SPAN];
            let (inl, inr) = if self.stereo {
                (&g.inl, &g.inr)
            } else {
                (&g.mid, &g.mid)
            };
            fir(&inl[s..s + SPAN + FIR_PAD], &self.h, &mut yl[..free]);
            if self.stereo {
                fir(&inr[s..s + SPAN + FIR_PAD], &self.h, &mut yr[..free]);
            }
            if free < len {
                let last = from(at.reach);
                yl[free..len].fill(dot12(&inl[last..last + READ_TAPS], &self.h));
                yr[free..len].fill(dot12(&inr[last..last + READ_TAPS], &self.h));
            }
            // Onto the output, in at most two runs (it wraps).
            let o = at.o_base.wrapping_add(k as isize as usize) & self.mask;
            let one = len.min(self.mask + 1 - o);
            for (src, dst, n) in [(0, o, one), (one, 0, len - one)] {
                let w = &ws[src..src + n];
                mac(&mut self.accl[dst..dst + n], w, &yl[src..src + n]);
                if self.stereo {
                    mac(&mut self.accr[dst..dst + n], w, &yr[src..src + n]);
                }
                for (s, w) in self.wsum[dst..dst + n].iter_mut().zip(w) {
                    *s += w;
                }
            }
            k += len as i32;
        }
    }
}

/// Where a grain's frame `k` goes and reads: output frame `o_base + k`,
/// input frame `m_base + k` (no further than `m_base + reach`).
struct Span {
    o_base: usize,
    m_base: usize,
    reach: i32,
}

/// Frames [`Voice::span`] lays at a time, and the input a run reads past
/// them (the taps, and the last four frames' rounding up).
const SPAN: usize = 128;
const FIR_PAD: usize = READ_TAPS + 3;

/// `y[j]` is the input from `x[j]` through the taps, four frames at a
/// time (each a lane, its taps summed in order): `x` holds `y.len()`
/// rounded up to four, plus the taps.
#[inline(always)]
fn fir(x: &[f32], h: &[f32; READ_TAPS], y: &mut [f32]) {
    let (quads, rest) = y.as_chunks_mut::<4>();
    for (q, out) in quads.iter_mut().enumerate() {
        let x: &[f32; READ_TAPS + 3] = x[4 * q..4 * q + READ_TAPS + 3].try_into().unwrap();
        let mut acc = [0.0f32; 4];
        for (t, h) in h.iter().enumerate() {
            for (l, a) in acc.iter_mut().enumerate() {
                *a += h * x[t + l];
            }
        }
        *out = acc;
    }
    let done = 4 * quads.len();
    for (j, out) in rest.iter_mut().enumerate() {
        let x = &x[done + j..done + j + READ_TAPS];
        *out = h.iter().zip(x).fold(0.0, |a, (h, x)| a + h * x);
    }
}

/// `acc[j] += w[j] * y[j]`.
#[inline(always)]
fn mac(acc: &mut [f32], w: &[f32], y: &[f32]) {
    for ((a, w), y) in acc.iter_mut().zip(w).zip(y) {
        *a += w * y;
    }
}

/// The ratios near 1 where a grain eases back onto its mark: fully, and
/// not at all (see [`Voice::advance`]).
const EASE_FULL: f32 = 0.0003;
const EASE_TO: f32 = 0.001;
/// The most a grain eases a step by, a share of the period: 0.1 %, under
/// 2 cents for the grain (at 0.5 % a vibrato swinging through the ratio 1
/// kept 4 cents of it at Speed 0).
const EASE_RATE: f32 = 0.001;

/// Fractions this near a whole frame read the frame itself.
const WHOLE: f32 = 1.0 / 1024.0;
const WHOLE_TAPS: [[f32; READ_TAPS]; 2] = {
    let mut t = [[0.0; READ_TAPS]; 2];
    t[0][READ_TAPS / 2 - 1] = 1.0;
    t[1][READ_TAPS / 2] = 1.0;
    t
};

/// Twelve samples through twelve taps: four lanes, then their sum in
/// order (summed in pairs, the compiler builds the lanes two wide).
#[inline(always)]
fn dot12(x: &[f32], h: &[f32; READ_TAPS]) -> f32 {
    let (x, _) = x.as_chunks::<4>();
    let (h, _) = h.as_chunks::<4>();
    let mut lane = [0.0f32; 4];
    for (x, h) in x.iter().zip(h) {
        lane = [
            lane[0] + h[0] * x[0],
            lane[1] + h[1] * x[1],
            lane[2] + h[2] * x[2],
            lane[3] + h[3] * x[3],
        ];
    }
    lane.iter().sum()
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

/// How many of the tracker's looks [`Looks`] keeps: 16 looks are 80 ms,
/// more than any latency.
const LOOKS: usize = 16;

/// The tracker's looks, each at the middle of the stretch it heard, so the
/// pitch can be read for the stretch a grain plays, which the latency puts
/// well behind the newest (the analysis centred on the grain played, which
/// the latency has already paid for).
pub(crate) struct Looks {
    at: [u64; LOOKS],
    /// The note heard, fractional (0: unvoiced).
    midi: [f32; LOOKS],
    count: usize,
    /// The look at or before the time last read, and the line from it to
    /// the next: its note, the step a frame, and the frame it starts on.
    cur: usize,
    base: f32,
    slope: f32,
    from: u64,
    /// The look count `slope` was worked out with.
    seen: usize,
}

impl Looks {
    pub(crate) const fn new() -> Looks {
        Looks {
            at: [0; LOOKS],
            midi: [0.0; LOOKS],
            count: 0,
            cur: 0,
            base: 0.0,
            slope: 0.0,
            from: 0,
            seen: usize::MAX,
        }
    }

    pub(crate) fn reset(&mut self) {
        *self = Looks::new();
    }

    /// A look: the note heard (0 for none) at the middle of what it heard
    /// (never before the look before: a voiced look's stretch is shorter).
    pub(crate) fn push(&mut self, at: u64, midi: f32) {
        let k = self.count % LOOKS;
        let at = if self.count > 0 {
            at.max(self.at[(self.count - 1) % LOOKS])
        } else {
            at
        };
        self.at[k] = at;
        self.midi[k] = midi;
        self.count += 1;
        if self.count - self.cur > LOOKS - 1 {
            self.cur = self.count - (LOOKS - 1);
        }
    }

    /// The note at time `t` (times only move on): a straight line between
    /// the looks either side while both are voiced, else the look before;
    /// but from an unvoiced look to a voiced one, the voiced one's note
    /// (the first voiced look sits a few ms into the note).
    #[inline]
    pub(crate) fn at(&mut self, t: u64) -> f32 {
        if self.count == 0 {
            return 0.0;
        }
        let before = self.cur;
        while self.cur + 1 < self.count && self.at[(self.cur + 1) % LOOKS] <= t {
            self.cur += 1;
        }
        if self.cur != before || self.seen != self.count {
            self.seen = self.count;
            let k = self.cur % LOOKS;
            self.base = self.midi[k];
            self.from = self.at[k];
            self.slope = 0.0;
            if self.cur + 1 < self.count {
                let n = (self.cur + 1) % LOOKS;
                let span = (self.at[n] - self.at[k]) as u32 as f32;
                if self.base <= 0.0 {
                    self.base = self.midi[n];
                } else if self.midi[n] > 0.0 && span > 0.0 {
                    self.slope = (self.midi[n] - self.base) / span;
                }
            }
        }
        if t <= self.from {
            return self.base;
        }
        self.base + self.slope * (t - self.from) as u32 as f32
    }
}

/// The note of the scale nearest a moving pitch, worked out again only
/// when the pitch leaves the stretch where it stays the nearest.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Snap {
    pub to: i32,
    lo: f32,
    hi: f32,
}

impl Snap {
    pub(crate) const fn new() -> Snap {
        Snap {
            to: 0,
            lo: 0.0,
            hi: -1.0,
        }
    }

    /// Forgets the note (the key or scale changed).
    pub(crate) fn clear(&mut self) {
        self.hi = -1.0;
    }

    /// The nearest note to `midi`; true when it is a different one.
    #[inline]
    pub(crate) fn note(&mut self, midi: f32, key: usize, scale: usize) -> bool {
        if midi >= self.lo && midi < self.hi {
            return false;
        }
        let to = nearest_note(midi, key, scale);
        // Half way to the scale's notes either side.
        let (mut lo, mut hi) = (to - 1, to + 1);
        while !in_scale(lo, key, scale) {
            lo -= 1;
        }
        while !in_scale(hi, key, scale) {
            hi += 1;
        }
        self.lo = 0.5 * (lo + to) as f32;
        self.hi = 0.5 * (to + hi) as f32;
        let changed = to != self.to;
        self.to = to;
        changed
    }
}

/// The latency for `range` at `rate`: two and a half of its longest
/// periods (a grain reaches a period either side of a mark that can be
/// half a period past its target time) and a little.
fn latency_for(rate: u32, range: usize) -> usize {
    let lo = VOICE_BANDS[range.min(VOICE_BANDS.len() - 1)].lo;
    (2.5 * rate as f32 / lo).ceil() as usize + 2
}

/// Half the stretch the tracker hears in `range`, in frames: the Range's
/// longest period. An unvoiced look sits there, at the middle of what it
/// heard; a voiced one sits at the middle of the shorter stretch
/// [`Grains::refine`] heard. At a note's start the look before the first
/// voiced one already heard most of the note (and its stretch's middle is
/// near the start), so the note's correction starts with the note.
fn span(rate: u32, range: usize) -> u64 {
    (rate as f32 / VOICE_BANDS[range.min(VOICE_BANDS.len() - 1)].lo) as u64
}

/// The longest period any range listens for at `rate`, in frames.
fn longest(rate: u32) -> usize {
    (rate as f32 / VOICE_BANDS[0].lo).ceil() as usize + 2
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
    /// Corrects only notes sung near the note: 0 corrects every one, and
    /// higher settings leave more of the way between notes alone.
    pub const FLEX_TUNE: u16 = 8;
    /// Slows the correction on held notes, so their vibrato survives.
    pub const HUMANIZE: u16 = 9;
}

static TUNE_INFO: NodeInfo = NodeInfo {
    id: "mpfree.fx.pitch_correct",
    name: "Pitch correct",
    kind: Kind::Effect,
};

static TUNE_PARAMS: [ParamInfo; 10] = [
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
    p(
        8,
        "Flex-Tune",
        "Notes",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
    p(
        9,
        "Humanize",
        "Notes",
        0.0,
        100.0,
        0.0,
        Unit::Percent,
        Curve::Linear,
        false,
    ),
];

/// How far from the note Flex-Tune still corrects, in cents: all of it at
/// 0, then from half a semitone at 1 % down to 5 cents at 100 %; past half
/// of it the correction fades out (a smoothstep), so notes sung well off
/// the note, slides and scoops pass as sung.
fn flex_weight(cents_off: f32, flex: f32) -> f32 {
    if flex <= 0.0 {
        return 1.0;
    }
    let reach = 50.0 - 0.45 * flex;
    let x = ((reach - cents_off.abs()) / (0.5 * reach)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Humanize: a note held this long (seconds) starts to take the slower
/// speed, and has it all after twice as long; at 100 % the slower speed is
/// Speed plus this many milliseconds.
const HUMANIZE_AFTER_S: f32 = 0.1;
const HUMANIZE_MS: f32 = 400.0;

/// A Range change fades the output out and back in over this long each
/// (seconds): the tracker starts again and the latency changes, which in a
/// frame would be a click.
const RANGE_FADE_S: f32 = 0.005;

/// A fade out and back in around a change made at the bottom (a Range).
#[derive(Clone, Copy, Debug)]
struct Dip {
    /// The gain's ramp (0 to 1), the step a frame, and which way.
    at: f32,
    step: f32,
    falling: bool,
}

impl Dip {
    const fn new() -> Dip {
        Dip {
            at: 1.0,
            step: 0.01,
            falling: false,
        }
    }

    fn set_time(&mut self, rate: u32) {
        self.step = 1.0 / (RANGE_FADE_S * rate as f32).max(1.0);
    }

    /// Starts down (or keeps going down).
    fn start(&mut self) {
        self.falling = true;
    }

    fn idle(&self) -> bool {
        !self.falling && self.at >= 1.0
    }

    /// One frame: the gain, and true on the frame the change is due.
    #[inline]
    fn next(&mut self) -> (f32, bool) {
        let mut due = false;
        if self.falling {
            self.at -= self.step;
            if self.at <= 0.0 {
                self.at = 0.0;
                self.falling = false;
                due = true;
            }
        } else {
            self.at = (self.at + self.step).min(1.0);
        }
        let x = self.at;
        (x * x * (3.0 - 2.0 * x), due)
    }
}

/// Pitch correct: pulls the voice to the nearest note of the key and scale,
/// over Speed (0 snaps: the hard, stepped effect), by Amount.
pub struct PitchCorrect {
    v: [f32; 10],
    rate: u32,
    tracker: Tracker,
    grains: Grains,
    voice: Voice,
    latency: usize,
    /// The Range the tracker listens to (a change waits for the dip).
    range: usize,
    dip: Dip,
    /// The period now (refined), and the looks behind it.
    period: f32,
    looks: Looks,
    /// The note the correction heads for.
    snap: Snap,
    /// Frames the note has been held, for Humanize.
    held: u32,
    /// The correction now, in semitones, and its one-pole's step; the
    /// ratio, and the correction it is for (a held note's correction sits
    /// still, so its power of two is worked out once).
    corr: f32,
    corr_k: f32,
    ratio: (f32, f32),
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
            tracker: Tracker::new(&VOICE_BANDS),
            grains: Grains::new(),
            voice: Voice::new(true),
            latency: 0,
            range: v[tune_ids::RANGE as usize] as usize,
            dip: Dip::new(),
            period: 0.0,
            looks: Looks::new(),
            snap: Snap::new(),
            held: 0,
            corr: 0.0,
            corr_k: 1.0,
            ratio: (0.0, 1.0),
            mix: Smooth::new(v[5] / 100.0),
        };
        n.prepare(48_000, mpfree_node::MAX_BLOCK);
        n
    }

    /// The correction's one-pole from Speed, slowed by Humanize on a note
    /// held long enough.
    fn speed(&mut self) {
        let held = self.held as f32 / (HUMANIZE_AFTER_S * self.rate as f32);
        let slow = (held - 1.0).clamp(0.0, 1.0) * self.v[tune_ids::HUMANIZE as usize] * 0.01;
        let ms = self.v[tune_ids::SPEED as usize] + slow * HUMANIZE_MS;
        let frames = ms * 0.001 * self.rate as f32;
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
            tune_ids::SPEED | tune_ids::HUMANIZE => self.speed(),
            tune_ids::MIX => self.mix.target = v / 100.0,
            tune_ids::RANGE if v as usize != self.range => self.dip.start(),
            // A key or scale change: the nearest note again.
            tune_ids::KEY | tune_ids::SCALE => self.snap.clear(),
            _ => {}
        }
    }

    /// At the bottom of the dip: the Range takes effect.
    fn change_range(&mut self) {
        self.range = self.v[tune_ids::RANGE as usize] as usize;
        self.tracker.set_range(self.range);
        self.latency = latency_for(self.rate, self.range);
        self.period = 0.0;
    }

    /// After a look: the period, refined, and the note heard for the
    /// stretch the tracker heard.
    fn look(&mut self) {
        let coarse = self.tracker.period;
        let now = self.grains.t + 1;
        if coarse > 0.0 {
            let slack = (self.rate as f32 / VOICE_BANDS[self.range].rate).ceil() as usize + 2;
            let p = self.grains.refine(coarse, slack);
            self.period = p;
            let midi = period_midi(p, self.rate);
            // The middle of what the refinement heard: its window and a
            // period before it.
            let heard = (p.max(self.grains.min_win as f32) + p) as u64 / 2;
            self.looks.push(now.saturating_sub(heard), midi);
            self.v[tune_ids::NOTE as usize] = (midi.round()).clamp(1.0, 127.0);
        } else {
            self.period = 0.0;
            self.looks
                .push(now.saturating_sub(span(self.rate, self.range)), 0.0);
            self.v[tune_ids::NOTE as usize] = 0.0;
        }
        if self.v[tune_ids::HUMANIZE as usize] > 0.0 {
            self.speed();
        }
    }

    /// The correction wanted for the note `midi` (0: none), in semitones.
    #[inline]
    fn target(&mut self, midi: f32) -> f32 {
        if midi <= 0.0 {
            self.held = 0;
            return 0.0;
        }
        let (key, scale) = (self.v[0] as usize, self.v[1] as usize);
        if self.snap.note(midi, key, scale) {
            self.held = 0;
        }
        self.held = self.held.saturating_add(1);
        let off = self.snap.to as f32 - midi;
        off * self.v[3] * 0.01 * flex_weight(100.0 * off, self.v[tune_ids::FLEX_TUNE as usize])
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let mix = self.mix.next();
            let gain = if self.dip.idle() {
                1.0
            } else {
                let (g, due) = self.dip.next();
                if due {
                    self.change_range();
                }
                g
            };
            if self.tracker.push(0.5 * (l[i] + r[i])) {
                self.look();
            }
            // The note heard where the grains laid now read: a period on
            // from the latency behind (the period through 32 bits: a float
            // to 64 bits is a library call on the MPC's ARM).
            let heard = (self.grains.t + 1 + self.grains.hint as u32 as u64)
                .saturating_sub(self.latency as u64);
            let midi = self.looks.at(heard);
            let target = self.target(midi);
            self.corr = crate::flush(self.corr + self.corr_k * (target - self.corr));
            self.grains.push(l[i], r[i], self.period);
            if self.corr != self.ratio.0 {
                self.ratio = (self.corr, fast_exp2(self.corr / 12.0));
            }
            let ratio = self.ratio.1;
            let (wl, wr) = self.voice.run(&self.grains, ratio, self.latency);
            let (dl, dr) = self.grains.dry(self.latency);
            l[i] = gain * (dl + mix * (wl - dl));
            r[i] = gain * (dr + mix * (wr - dr));
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
        self.range = self.v[tune_ids::RANGE as usize] as usize;
        self.tracker.prepare(self.rate, self.range);
        self.grains.prepare(self.rate, longest(self.rate));
        self.voice.prepare(longest(self.rate));
        self.latency = latency_for(self.rate, self.range);
        self.mix.set_time(self.rate, 20.0);
        self.dip = Dip::new();
        self.dip.set_time(self.rate);
        self.speed();
    }
    fn reset(&mut self) {
        // A Range change still waiting takes effect now.
        if self.range != self.v[tune_ids::RANGE as usize] as usize {
            self.change_range();
        }
        self.dip = Dip::new();
        self.dip.set_time(self.rate);
        self.tracker.reset();
        self.grains.reset();
        self.voice.reset();
        self.looks.reset();
        self.period = 0.0;
        self.snap.clear();
        self.held = 0;
        self.corr = 0.0;
        self.ratio = (0.0, 1.0);
        self.speed();
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
    /// The ratio, and the shift it is for (as [`PitchCorrect`]'s).
    ratio: (f32, f32),
    gain: Smooth,
    /// The DC blocker's last input and output (see [`HP_HZ`]).
    hp: (f32, f32),
}

/// The harmony voices' DC blocker, Hz. Shifted up by a ratio between 1
/// and 2, a voice's grains overlap unevenly over each period and their
/// weighted sum leans to one side: a 3rd up on a saw carried a DC offset
/// 28 dB under the voice.
const HP_HZ: f32 = 5.0;

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
    /// As [`PitchCorrect`]'s: the Range listened to, its dip, the period
    /// now, the looks, and the note heard where the grains read.
    range: usize,
    dip: Dip,
    period: f32,
    looks: Looks,
    snap: Snap,
    /// Whether the voices were last set for a note (false: unvoiced, or
    /// to be set again).
    set: bool,
    glide_k: f32,
    hp_k: f32,
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
            ratio: (0.0, 1.0),
            gain: Smooth::new(0.0),
            hp: (0.0, 0.0),
        };
        let mut n = Harmonizer {
            v,
            rate: 48_000,
            tracker: Tracker::new(&VOICE_BANDS),
            grains: Grains::new(),
            parts: [part(), part()],
            latency: 0,
            range: v[harm_ids::RANGE as usize] as usize,
            dip: Dip::new(),
            period: 0.0,
            looks: Looks::new(),
            snap: Snap::new(),
            set: false,
            glide_k: 1.0,
            hp_k: 1.0,
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
            harm_ids::RANGE if v as usize != self.range => self.dip.start(),
            // The voices are set again for the note heard.
            harm_ids::KEY | harm_ids::SCALE => {
                self.snap.clear();
                self.set = false;
            }
            harm_ids::VOICE1 | harm_ids::VOICE2 | harm_ids::LEVEL1 | harm_ids::LEVEL2 => {
                self.set = false
            }
            _ => {}
        }
    }

    /// At the bottom of the dip: the Range takes effect.
    fn change_range(&mut self) {
        self.range = self.v[harm_ids::RANGE as usize] as usize;
        self.tracker.set_range(self.range);
        self.latency = latency_for(self.rate, self.range);
        self.period = 0.0;
    }

    /// After a look: the period, refined, and the note heard for the
    /// stretch the tracker heard (as [`PitchCorrect::look`]).
    fn look(&mut self) {
        let coarse = self.tracker.period;
        let now = self.grains.t + 1;
        if coarse > 0.0 {
            let slack = (self.rate as f32 / VOICE_BANDS[self.range].rate).ceil() as usize + 2;
            let p = self.grains.refine(coarse, slack);
            self.period = p;
            let midi = period_midi(p, self.rate);
            let heard = (p.max(self.grains.min_win as f32) + p) as u64 / 2;
            self.looks.push(now.saturating_sub(heard), midi);
            self.v[harm_ids::NOTE as usize] = midi.round().clamp(1.0, 127.0);
        } else {
            self.period = 0.0;
            self.looks
                .push(now.saturating_sub(span(self.rate, self.range)), 0.0);
            self.v[harm_ids::NOTE as usize] = 0.0;
        }
    }

    /// Sets each voice's interval and level for the note `midi` (0: none).
    fn voices(&mut self, midi: f32) {
        let (key, scale) = (self.v[0] as usize, self.v[1] as usize);
        if midi <= 0.0 {
            if self.set {
                self.set = false;
                for part in &mut self.parts {
                    part.gain.target = 0.0;
                }
            }
            return;
        }
        let changed = self.snap.note(midi, key, scale);
        if self.set && !changed {
            return;
        }
        self.set = true;
        let picks = [harm_ids::VOICE1, harm_ids::VOICE2];
        let levels = [harm_ids::LEVEL1, harm_ids::LEVEL2];
        for (k, part) in self.parts.iter_mut().enumerate() {
            let pick = self.v[picks[k] as usize] as usize;
            match interval_semis(pick, self.snap.to, key, scale) {
                Some(s) => {
                    part.target = s as f32;
                    part.gain.target = self.v[levels[k] as usize] / 100.0;
                }
                None => part.gain.target = 0.0,
            }
        }
    }

    fn run(&mut self, l: &mut [f32], r: &mut [f32]) {
        for i in 0..l.len() {
            let gain = if self.dip.idle() {
                1.0
            } else {
                let (g, due) = self.dip.next();
                if due {
                    self.change_range();
                }
                g
            };
            if self.tracker.push(0.5 * (l[i] + r[i])) {
                self.look();
            }
            let heard = (self.grains.t + 1 + self.grains.hint as u32 as u64)
                .saturating_sub(self.latency as u64);
            let midi = self.looks.at(heard);
            self.voices(midi);
            self.grains.push(l[i], r[i], self.period);
            let (dl, dr) = self.grains.dry(self.latency);
            let dry = self.dry.next();
            let (mut ol, mut or) = (dry * dl, dry * dr);
            for (k, part) in self.parts.iter_mut().enumerate() {
                part.semis = crate::flush(part.semis + self.glide_k * (part.target - part.semis));
                let level = part.gain.next();
                let pan = self.pans[k].next();
                if part.semis != part.ratio.0 {
                    part.ratio = (part.semis, fast_exp2(part.semis / 12.0));
                }
                let ratio = part.ratio.1;
                // Laid down even while silent, so a voice comes back in on
                // its own grains.
                let (x, _) = part.voice.run(&self.grains, ratio, self.latency);
                let y = crate::flush(x - part.hp.0 + self.hp_k * part.hp.1);
                part.hp = (x, y);
                // Equal-power pan from a parabola (no sine on the audio
                // thread): the sides at 1 and 0, 0.707 in the middle.
                let x = 0.5 * (pan + 1.0);
                let (gl, gr) = (1.0 - x * x, 1.0 - (1.0 - x) * (1.0 - x));
                ol += level * gl * y;
                or += level * gr * y;
            }
            l[i] = gain * ol;
            r[i] = gain * or;
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
        self.range = self.v[harm_ids::RANGE as usize] as usize;
        self.tracker.prepare(self.rate, self.range);
        self.grains.prepare(self.rate, longest(self.rate));
        for part in &mut self.parts {
            part.voice.prepare(longest(self.rate));
            part.gain.set_time(self.rate, 20.0);
        }
        self.latency = latency_for(self.rate, self.range);
        self.hp_k = 1.0 - std::f32::consts::TAU * HP_HZ / self.rate as f32;
        self.dip = Dip::new();
        self.dip.set_time(self.rate);
        self.dry.set_time(self.rate, 20.0);
        for s in &mut self.pans {
            s.set_time(self.rate, 20.0);
        }
        self.glide();
    }
    fn reset(&mut self) {
        if self.range != self.v[harm_ids::RANGE as usize] as usize {
            self.change_range();
        }
        self.dip = Dip::new();
        self.dip.set_time(self.rate);
        self.looks.reset();
        self.snap.clear();
        self.set = false;
        self.period = 0.0;
        self.tracker.reset();
        self.grains.reset();
        for part in &mut self.parts {
            part.voice.reset();
            part.hp = (0.0, 0.0);
            part.target = 0.0;
            part.semis = 0.0;
            part.ratio = (0.0, 1.0);
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

    /// A band-limited saw, `n` frames.
    fn saw(hz: f64, rate: u32, n: usize) -> Vec<f32> {
        let k_max = (18_000.0 / hz) as usize;
        let mut ph = 0.0f64;
        (0..n)
            .map(|_| {
                ph = (ph + hz / rate as f64).fract();
                let th = std::f64::consts::TAU * ph;
                let sum: f64 = (1..=k_max).map(|k| (k as f64 * th).sin() / k as f64).sum();
                (0.1 * sum) as f32
            })
            .collect()
    }

    #[test]
    fn flex_tune_leaves_notes_sung_far_off_alone() {
        // Off: every note all the way.
        assert_eq!(flex_weight(45.0, 0.0), 1.0);
        // Halfway: within 13.75 cents all the way, past 27.5 none.
        assert_eq!(flex_weight(10.0, 50.0), 1.0);
        assert_eq!(flex_weight(-30.0, 50.0), 0.0);
        let w = flex_weight(20.0, 50.0);
        assert!(w > 0.0 && w < 1.0, "{w}");
        // All the way up: only notes within 5 cents.
        assert_eq!(flex_weight(6.0, 100.0), 0.0);
    }

    #[test]
    fn a_voice_at_ratio_one_is_the_input() {
        let rate = 44_100;
        let long = longest(rate);
        let (mut g, mut v) = (Grains::new(), Voice::new(false));
        g.prepare(rate, long);
        v.prepare(long);
        let lat = latency_for(rate, 1);
        let x = saw(220.0, rate, rate as usize);
        let period = rate as f32 / 220.0;
        let mut worst = 0.0f32;
        for (i, &s) in x.iter().enumerate() {
            g.push(s, s, period);
            let (y, _) = v.run(&g, 1.0, lat);
            if i > rate as usize / 2 {
                worst = worst.max((y - x[i - lat]).abs());
            }
        }
        assert!(worst < 1e-5, "{worst}");
    }

    #[test]
    fn the_period_is_refined_to_a_hundredth_of_a_frame() {
        let rate = 44_100;
        for hz in [82.4, 220.0, 451.6, 1046.5] {
            let mut g = Grains::new();
            g.prepare(rate, longest(rate));
            let x = saw(hz, rate, rate as usize / 4);
            for &s in &x {
                g.push(s, s, 0.0);
            }
            let want = (rate as f64 / hz) as f32;
            let p = g.refine(want.round() + 0.4, 6);
            assert!((p - want).abs() < 0.01, "{hz} Hz: {p} for {want}");
        }
    }

    #[test]
    fn humanize_slows_a_held_note() {
        let mut n = PitchCorrect::new();
        n.prepare(48_000, 128);
        n.set_param(tune_ids::SPEED, 20.0);
        let quick = n.corr_k;
        n.set_param(tune_ids::HUMANIZE, 100.0);
        // A note just sung corrects at Speed; held 200 ms, 400 ms slower.
        n.held = 0;
        n.speed();
        assert_eq!(n.corr_k, quick);
        n.held = (0.2 * 48_000.0) as u32;
        n.speed();
        let want = 1.0 - (-1.0f32 / (0.42 * 48_000.0)).exp();
        assert!((n.corr_k - want).abs() < 1e-9, "{} {want}", n.corr_k);
    }

    #[test]
    fn a_harmony_voice_carries_no_dc() {
        let rate = 44_100;
        let mut n = Harmonizer::new();
        n.prepare(rate, 128);
        for (id, v) in [
            (harm_ids::VOICE1, 7.0),
            (harm_ids::VOICE2, 0.0),
            (harm_ids::LEVEL1, 100.0),
            (harm_ids::DRY, 0.0),
            (harm_ids::PAN1, 0.0),
        ] {
            n.set_param(id, v);
        }
        n.reset();
        let x = saw(440.0, rate, rate as usize);
        let (mut l, mut r) = (x.clone(), x.clone());
        for (l, r) in l.chunks_mut(128).zip(r.chunks_mut(128)) {
            n.process(&Ctx::new(rate, 0), &[], l, r);
        }
        let tail = &l[rate as usize / 2..];
        let mean = tail.iter().sum::<f32>() / tail.len() as f32;
        let rms = (tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt();
        // Without the blocker the mean was 5 % of the RMS.
        assert!(
            rms > 0.01 && mean.abs() < 5e-3 * rms,
            "mean {mean} rms {rms}"
        );
    }
}
