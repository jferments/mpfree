//! The look-ahead peak limiter that the [`Limiter`](crate::Limiter) and
//! [`MasterSafety`](crate::MasterSafety)'s Safe limiter share (FX quality
//! round 4, targets P1 to P5 and P7).
//!
//! Each frame, for both channels together:
//!
//! 1. **Detect.** The peak of the sample [`TP_LAG`] frames back and, with
//!    true peaks on, of the three points a quarter, a half and three
//!    quarters of the way to the next sample, interpolated as a 4x
//!    true-peak meter does: 32-tap Kaiser-windowed sinc phases, which read
//!    a sine within 0.08 dB of its level up to 19 kHz at 44.1 kHz, and
//!    above that at least what fxlab's 48-tap meter reads. The gain the
//!    frame needs is the ceiling over that peak.
//! 2. **Hold.** The smallest need over the look-ahead window and a hold
//!    after it (a quarter of Release, at most 5 ms), so the gain barely
//!    moves between the crests of a bass note instead of riding inside
//!    each cycle: 50 Hz pushed 6 dB keeps its distortion 54 dB down. A
//!    10 ms hold keeps it still, but costs 0.3 LU more on a pushed mix.
//! 3. **Release**, in decibels, so the gain is 63 % of the way back when
//!    Release says (the hold and the ramp's half window are taken out of
//!    the time constant). Auto release adds a slow stage that follows a
//!    sustained push over half a second and lets go at three times
//!    Release; the gain comes back no faster than it, so a long push
//!    recovers slowly and a short peak at a third of Release.
//! 4. **Ramp.** The released gain averaged over the look-ahead window, so
//!    it is down in a smooth ramp by the time the peak leaves.
//!
//! Every stage only ever lowers the gain from what the frame's peak
//! needs, never raises it past that, so the output's sample peaks (and,
//! with true peaks on, its 4x true peaks) stay at the ceiling. The audio
//! leaves `window + TP_LAG - 1` frames late, whatever the settings.

use crate::kit::coef;
use crate::{fast_db_to_gain, fast_level_db, FixedDelay};

/// Taps of each true-peak phase, and how far behind the newest input the
/// detector reads.
pub(crate) const TP_TAPS: usize = 32;
pub(crate) const TP_LAG: usize = TP_TAPS / 2;
/// Each phase folded about the centre of the window (see [`Folded`]).
const HALF: usize = TP_TAPS / 2;
/// The folded pairs from here on are the 12 taps nearest the points
/// between samples; the 20 further out add at most 0.31 of the loudest
/// sample to any point.
const NEAR_FROM: usize = 10;
/// Shares of the ceiling the window's samples are tracked under, past
/// the quiet share: below one, the near taps alone can show that no point
/// between samples passes the ceiling.
const RUNGS: [f32; 3] = [0.7, 1.0, 1.4];
/// The Kaiser window's shape. 4 trades a ripple of up to 0.08 dB below
/// 19 kHz for a reading that doesn't fall short of the meter's near the
/// top of the band: at 6 the ripple is 0.01 dB, but the phases read
/// 0.3 dB short at 20 kHz, and a soft-clipped master passed 0.11 dB over
/// its ceiling.
const TP_BETA: f64 = 4.0;
/// The longest hold, ms (it is a quarter of Release up to this).
const HOLD_MS: f32 = 5.0;
/// Auto release's slow stage: how fast a sustained push pulls it down.
const SLOW_ATTACK_MS: f32 = 500.0;
/// Below this many dB of reduction a stage counts as fully back.
const BACK_DB: f32 = -1e-4;

/// The zeroth modified Bessel function, by its series (control thread).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = 0.25 * x * x;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-17 * sum {
            break;
        }
    }
    sum
}

/// The true-peak phases: for the points a quarter, a half and three
/// quarters of the way from the sample at `TP_LAG - 1` to the next, the
/// Kaiser-windowed sinc taps over the 32 samples around them (16 on each
/// side), each phase normalised to unity at DC.
fn tp_phases() -> [[f32; TP_TAPS]; 3] {
    use std::f64::consts::PI;
    let half = TP_LAG as f64 + 0.5;
    let mut out = [[0.0f32; TP_TAPS]; 3];
    for (k, ph) in out.iter_mut().enumerate() {
        let frac = (k + 1) as f64 * 0.25;
        let mut taps = [0.0f64; TP_TAPS];
        for (j, tap) in taps.iter_mut().enumerate() {
            let x = j as f64 - (TP_LAG as f64 - 1.0) - frac;
            let sinc = if x.abs() < 1e-12 {
                1.0
            } else {
                (PI * x).sin() / (PI * x)
            };
            let r = (x / half).clamp(-1.0, 1.0);
            *tap = sinc * bessel_i0(TP_BETA * (1.0 - r * r).sqrt()) / bessel_i0(TP_BETA);
        }
        let sum: f64 = taps.iter().sum();
        for (o, t) in ph.iter_mut().zip(taps) {
            *o = (t / sum) as f32;
        }
    }
    out
}

/// The phases folded about the centre of the window, so the detector
/// does half the multiplies. The half-way phase is symmetric, and the
/// three-quarter phase is the quarter phase backwards: with `s` and `d`
/// the sums and differences of the samples paired from both ends of the
/// window, the half-way point is `mid . s`, and the quarter and
/// three-quarter points are `even . s + odd . d` and `even . s - odd . d`.
struct Folded {
    even: [f32; HALF],
    odd: [f32; HALF],
    mid: [f32; HALF],
    /// Over this share of the ceiling no sample in the window can make a
    /// point between samples pass it: one over the largest sum of a
    /// phase's taps' sizes.
    quiet: f32,
    /// The largest sum of the far taps' sizes (before [`NEAR_FROM`]) of
    /// any phase, a hair over.
    far: f32,
}

impl Folded {
    fn new() -> Folded {
        let ph = tp_phases();
        let mut f = Folded {
            even: [0.0; HALF],
            odd: [0.0; HALF],
            mid: [0.0; HALF],
            quiet: 0.0,
            far: 0.0,
        };
        for j in 0..HALF {
            let (a, b) = (ph[0][j], ph[0][TP_TAPS - 1 - j]);
            f.even[j] = 0.5 * (a + b);
            f.odd[j] = 0.5 * (a - b);
            f.mid[j] = ph[1][j];
        }
        let l1 = ph
            .iter()
            .map(|p| p.iter().map(|v| f64::from(v.abs())).sum::<f64>())
            .fold(0.0, f64::max);
        // A hair under, for the rounding of the sums.
        f.quiet = (0.999 / l1) as f32;
        let far = ph
            .iter()
            .map(|p| {
                (0..NEAR_FROM)
                    .map(|j| f64::from(p[j].abs()) + f64::from(p[TP_TAPS - 1 - j].abs()))
                    .sum::<f64>()
            })
            .fold(0.0, f64::max);
        f.far = (1.001 * far) as f32;
        f
    }
}

/// The shared limiter. See the module docs.
pub(crate) struct PeakLimit {
    rate: u32,
    /// Look-ahead window in frames.
    w: usize,
    /// Hold after the window, frames, and the most it can be.
    hold: usize,
    hold_cap: usize,
    release_ms: f32,
    auto: bool,
    true_peak: bool,
    /// The fast stage's release, and the slow stage's fall and release.
    k_fast: f32,
    k_slow_down: f32,
    k_slow_up: f32,
    /// The stages, dB (0 or below).
    fast_db: f32,
    slow_db: f32,
    /// The last held need, and it in dB.
    held_was: f32,
    held_db: f32,
    /// Sliding minimum of the needed gain: (frame, value), increasing.
    dq: Vec<(u64, f32)>,
    dq_head: usize,
    dq_len: usize,
    /// The released gains of the last `w` frames, and their sum.
    box_buf: Vec<f32>,
    box_pos: usize,
    box_sum: f64,
    inv_w: f64,
    /// The frame after the last that the released gain was under 1: from
    /// `w` frames on, the box holds nothing but ones.
    whole_from: u64,
    /// Input history for the detector, written twice so the last
    /// TP_TAPS frames always sit side by side from `hist_at` (oldest
    /// first).
    hist: [[f32; 2 * TP_TAPS]; 2],
    hist_at: usize,
    folded: Folded,
    /// Frames since a sample over each share of the ceiling: the quiet
    /// share (over it, the points between samples need working out, or
    /// with true peaks off a gain might be needed), then [`RUNGS`]. At
    /// [`TP_TAPS`] or more, no sample in the window is over it.
    ago: [usize; 1 + RUNGS.len()],
    /// Those shares of `lim_for`, the ceiling they were worked out for.
    lims: [f32; 1 + RUNGS.len()],
    lim_for: f32,
    delay: [FixedDelay; 2],
    frame: u64,
}

impl PeakLimit {
    /// Allocates (control thread); [`PeakLimit::setup`] before use.
    pub(crate) fn new() -> PeakLimit {
        PeakLimit {
            rate: 48_000,
            w: 1,
            hold: 1,
            hold_cap: 1,
            release_ms: 80.0,
            auto: false,
            true_peak: false,
            k_fast: 0.0,
            k_slow_down: 0.0,
            k_slow_up: 0.0,
            fast_db: 0.0,
            slow_db: 0.0,
            held_was: 1.0,
            held_db: 0.0,
            dq: Vec::new(),
            dq_head: 0,
            dq_len: 0,
            box_buf: Vec::new(),
            box_pos: 0,
            box_sum: 0.0,
            inv_w: 1.0,
            whole_from: 0,
            hist: [[0.0; 2 * TP_TAPS]; 2],
            hist_at: 0,
            folded: Folded::new(),
            ago: [TP_TAPS; 1 + RUNGS.len()],
            lims: [0.0; 1 + RUNGS.len()],
            lim_for: f32::NAN,
            delay: [FixedDelay::new(0), FixedDelay::new(0)],
            frame: 0,
        }
    }

    /// Sizes everything for `rate` with a `look_ms` look-ahead (control
    /// thread), and clears.
    pub(crate) fn setup(&mut self, rate: u32, look_ms: f32) {
        self.rate = rate.max(1);
        let frames = |ms: f32| (ms * 0.001 * self.rate as f32).round() as usize;
        self.w = frames(look_ms).max(1);
        self.hold_cap = frames(HOLD_MS).max(1);
        self.inv_w = 1.0 / self.w as f64;
        self.dq = vec![(0, 1.0); self.w + self.hold_cap + 2];
        self.box_buf = vec![1.0; self.w];
        let lat = self.latency() as usize;
        self.delay = [FixedDelay::new(lat), FixedDelay::new(lat)];
        self.retime();
        self.clear();
    }

    /// Release in ms, and whether Auto release is on.
    pub(crate) fn set_release(&mut self, ms: f32, auto: bool) {
        self.release_ms = ms;
        self.auto = auto;
        self.retime();
    }

    pub(crate) fn set_true_peak(&mut self, on: bool) {
        if on && !self.true_peak {
            // The rungs aren't tracked with true peaks off.
            self.ago[1..].fill(0);
        }
        self.true_peak = on;
    }

    fn retime(&mut self) {
        let rate = self.rate;
        let ms = self.release_ms.max(0.1);
        let hold_ms = (0.25 * ms).min(HOLD_MS);
        self.hold = ((hold_ms * 0.001 * rate as f32).round() as usize).clamp(1, self.hold_cap);
        // The release reads its label at 63 %: take out the hold and the
        // ramp's half window, which come first.
        let ramp_ms = 500.0 * self.w as f32 / rate as f32;
        let label = if self.auto { ms / 3.0 } else { ms };
        let tau = (label - hold_ms - ramp_ms).max(0.25 * label);
        self.k_fast = coef(tau, rate);
        self.k_slow_down = coef(SLOW_ATTACK_MS, rate);
        self.k_slow_up = coef(3.0 * ms, rate);
        if !self.auto {
            self.slow_db = 0.0;
        }
    }

    /// Frames from a frame in to the same frame out.
    pub(crate) fn latency(&self) -> u32 {
        (self.w + TP_LAG - 1) as u32
    }

    /// Frames until the gain is back from any reduction after silence
    /// starts: ten of the slowest stage, plus the hold and the latency.
    pub(crate) fn rest(&self) -> u32 {
        let slowest = if self.auto {
            3.0 * self.release_ms
        } else {
            self.release_ms
        };
        (10.0 * slowest * 0.001 * self.rate as f32) as u32
            + (self.hold + self.w) as u32
            + self.latency()
    }

    pub(crate) fn clear(&mut self) {
        self.fast_db = 0.0;
        self.slow_db = 0.0;
        self.held_was = 1.0;
        self.held_db = 0.0;
        self.dq_head = 0;
        self.dq_len = 0;
        self.box_buf.fill(1.0);
        self.box_pos = 0;
        self.box_sum = self.w as f64;
        self.hist = [[0.0; 2 * TP_TAPS]; 2];
        self.hist_at = 0;
        self.ago = [TP_TAPS; 1 + RUNGS.len()];
        for d in &mut self.delay {
            d.reset();
        }
        self.frame = 0;
        self.whole_from = 0;
    }

    /// The detector's level for the sample TP_LAG frames back and, with
    /// `between`, for the points between it and the next sample.
    #[inline(always)]
    fn level(&self, ch: usize, between: bool, room: f32) -> f32 {
        // The window starts at `hist_at`, under TP_TAPS: masked so the
        // compiler sees it in bounds.
        let p = self.hist[ch][self.hist_at % TP_TAPS + TP_LAG - 1].abs();
        if between {
            p.max(self.points(ch, room))
        } else {
            p
        }
    }

    /// The loudest of the points between the sample TP_LAG frames back
    /// and the next. When the near taps' points come to no more than
    /// `room`, the far taps can't take any point past the ceiling, and
    /// this is 0 (the need is the same).
    #[inline(never)]
    fn points(&self, ch: usize, room: f32) -> f32 {
        let at = self.hist_at % TP_TAPS;
        let h: &[f32; TP_TAPS] = self.hist[ch][at..at + TP_TAPS]
            .try_into()
            .expect("the window is TP_TAPS long");
        // Plain sums: the MPC's ARM has no float vector unit the compiler
        // uses, so fewer multiplies is what counts there.
        let f = &self.folded;
        let (mut e, mut o, mut m) = (0.0f32, 0.0f32, 0.0f32);
        for j in NEAR_FROM..HALF {
            let (a, b) = (h[j], h[TP_TAPS - 1 - j]);
            let (s, d) = (a + b, a - b);
            e += f.even[j] * s;
            o += f.odd[j] * d;
            m += f.mid[j] * s;
        }
        if (e + o).abs().max((e - o).abs()).max(m.abs()) <= room {
            return 0.0;
        }
        for j in 0..NEAR_FROM {
            let (a, b) = (h[j], h[TP_TAPS - 1 - j]);
            let (s, d) = (a + b, a - b);
            e += f.even[j] * s;
            o += f.odd[j] * d;
            m += f.mid[j] * s;
        }
        (e + o).abs().max((e - o).abs()).max(m.abs())
    }

    /// Pushes this frame's need and returns the smallest over the window
    /// and the hold.
    #[inline]
    fn held(&mut self, g: f32) -> f32 {
        let cap = self.dq.len();
        // Indices wrap by one subtraction: a `%` is a library call on
        // the MPC's ARM, which has no divide instruction.
        let wrap = |i: usize| if i >= cap { i - cap } else { i };
        // Drop values no smaller than the new one from the back.
        while self.dq_len > 0 {
            let back = wrap(self.dq_head + self.dq_len - 1);
            if self.dq[back].1 >= g {
                self.dq_len -= 1;
            } else {
                break;
            }
        }
        let at = wrap(self.dq_head + self.dq_len);
        self.dq[at] = (self.frame, g);
        self.dq_len += 1;
        // Drop the front once it leaves the window and the hold (the
        // newest entry always stays).
        let oldest = (self.frame + 1).saturating_sub((self.w + self.hold) as u64);
        while self.dq[self.dq_head].0 < oldest {
            self.dq_head = wrap(self.dq_head + 1);
            self.dq_len -= 1;
        }
        self.dq[self.dq_head].1
    }

    /// The released gain for a held need of `held`: never above it.
    #[inline]
    fn release(&mut self, held: f32) -> f32 {
        if held >= 1.0 && self.fast_db >= 0.0 && self.slow_db >= 0.0 {
            return 1.0;
        }
        // The need in dB, worked out again only when it moves (it sits
        // still through a hold).
        if held != self.held_was {
            self.held_was = held;
            self.held_db = if held >= 1.0 {
                0.0
            } else {
                fast_level_db(held)
            };
        }
        let want = self.held_db;
        // Down at once (and still through the hold), back up in decibels.
        let mut exact = want <= self.fast_db;
        self.fast_db = if exact {
            want
        } else {
            let v = self.fast_db + (want - self.fast_db) * self.k_fast;
            if v - want > BACK_DB {
                exact = true;
                want
            } else {
                v
            }
        };
        let mut env = self.fast_db;
        if self.auto {
            let k = if want < self.slow_db {
                self.k_slow_down
            } else {
                self.k_slow_up
            };
            let v = self.slow_db + (want - self.slow_db) * k;
            self.slow_db = if (v - want).abs() < -BACK_DB { want } else { v };
            if self.slow_db < env {
                env = self.slow_db;
                exact = false;
            }
        }
        if exact || env >= 0.0 {
            held.min(1.0)
        } else {
            fast_db_to_gain(env).min(held)
        }
    }

    /// The shares of a new ceiling. The window's samples were tracked
    /// against the old one, so until it has passed they count as loud.
    #[cold]
    fn retrack(&mut self, ceiling: f32) {
        self.lim_for = ceiling;
        self.lims[0] = ceiling * self.folded.quiet;
        for (lim, r) in self.lims[1..].iter_mut().zip(RUNGS) {
            *lim = ceiling * r;
        }
        self.ago = [0; 1 + RUNGS.len()];
    }

    /// Writes a frame into the detector's history.
    #[inline]
    fn push(&mut self, x: [f32; 2]) {
        let at = self.hist_at % TP_TAPS;
        for (ch, &v) in x.iter().enumerate() {
            self.hist[ch][at] = v;
            self.hist[ch][at + TP_TAPS] = v;
        }
        self.hist_at = (at + 1) % TP_TAPS;
    }

    /// A whole block in place, times `gain`, when it needs no limiting:
    /// the limiter is at rest (nothing loud in the detector's window, no
    /// need held, the gain released and averaged back to 1) and every
    /// sample in the block times `gain` is under the quiet share of
    /// `ceiling`. Then [`PeakLimit::run`] would give a gain of exactly 1
    /// for every frame, so the block is only delayed, at a fraction of the
    /// cost. Returns false, having changed nothing, when the block needs
    /// [`PeakLimit::run`] frame by frame.
    pub(crate) fn quiet_block(
        &mut self,
        l: &mut [f32],
        r: &mut [f32],
        gain: f32,
        ceiling: f32,
    ) -> bool {
        let n = l.len().min(r.len());
        if n == 0 {
            return true;
        }
        let rest = ceiling == self.lim_for
            && self.ago[0] >= TP_TAPS
            && self.frame >= self.whole_from + self.w as u64
            && self.fast_db >= 0.0
            && self.slow_db >= 0.0
            && (self.dq_len == 0 || self.dq[self.dq_head].1 >= 1.0);
        if !rest {
            return false;
        }
        let quiet = self.lims[0];
        let mut loudest = 0.0f32;
        for (a, b) in l[..n].iter().zip(&r[..n]) {
            loudest = loudest.max(a.abs()).max(b.abs());
        }
        // Rounding keeps order, so the loudest times the gain is the
        // loudest of the samples times it. Not a number never passes (the
        // frames' path deals with it).
        let fits = loudest * gain <= quiet;
        if !fits {
            return false;
        }
        if gain != 1.0 {
            for v in l[..n].iter_mut().chain(r[..n].iter_mut()) {
                *v *= gain;
            }
        }
        if n >= TP_TAPS {
            for (ch, x) in [&l[n - TP_TAPS..n], &r[n - TP_TAPS..n]]
                .into_iter()
                .enumerate()
            {
                self.hist[ch][..TP_TAPS].copy_from_slice(x);
                self.hist[ch][TP_TAPS..].copy_from_slice(x);
            }
            self.hist_at = 0;
        } else {
            for i in 0..n {
                self.push([l[i], r[i]]);
            }
        }
        self.delay[0].run_block(&mut l[..n]);
        self.delay[1].run_block(&mut r[..n]);
        self.box_pos = (self.box_pos + n) % self.w;
        self.frame += n as u64;
        // Every need was 1: the newest is all the window keeps.
        self.dq_head = 0;
        self.dq_len = 1;
        self.dq[0] = (self.frame - 1, 1.0);
        true
    }

    /// One frame in; the frame `latency` frames back and the gain to
    /// multiply it by, which keeps it (and with true peaks on, the points
    /// between it and its neighbours) at or under `ceiling`.
    #[inline]
    pub(crate) fn run(&mut self, x: [f32; 2], ceiling: f32) -> ([f32; 2], f32) {
        self.push(x);
        let out = [self.delay[0].run(x[0]), self.delay[1].run(x[1])];
        // The points between samples only once a sample in the window is
        // loud enough to make one pass the ceiling; and then only the near
        // taps' share, while the window's samples are low enough that the
        // far taps can't take a point past it.
        if ceiling != self.lim_for {
            self.retrack(ceiling);
        }
        let loudest = x[0].abs().max(x[1].abs());
        let (mut between, mut room) = (false, -1.0);
        if self.true_peak {
            for (ago, &lim) in self.ago.iter_mut().zip(&self.lims) {
                if loudest > lim {
                    *ago = 0;
                } else if *ago < TP_TAPS {
                    *ago += 1;
                }
            }
            between = self.ago[0] < TP_TAPS;
            if let Some(k) = self.ago[1..].iter().position(|&a| a >= TP_TAPS) {
                room = ceiling - self.folded.far * self.lims[1 + k];
            }
        } else if loudest > self.lims[0] {
            self.ago[0] = 0;
        } else if self.ago[0] < TP_TAPS {
            self.ago[0] += 1;
        }
        let peak = self
            .level(0, between, room)
            .max(self.level(1, between, room));
        let need = if peak > ceiling { ceiling / peak } else { 1.0 };
        let held = self.held(need);
        let env = self.release(held);
        self.box_sum += f64::from(env) - f64::from(self.box_buf[self.box_pos]);
        self.box_buf[self.box_pos] = env;
        self.box_pos += 1;
        if self.box_pos == self.w {
            self.box_pos = 0;
        }
        if env < 1.0 {
            self.whole_from = self.frame + 1;
        }
        self.frame += 1;
        (out, ((self.box_sum * self.inv_w) as f32).min(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oversample::dot;

    #[test]
    fn the_phases_read_a_sine_at_its_level() {
        // Each phase's gain for a sine: within 0.08 dB of 1 up to
        // 19 kHz at 44.1 kHz, over rather than under at 20 kHz, and at
        // 20.5 kHz no shorter than fxlab's meter (0.54 dB).
        let ph = tp_phases();
        for (hz, low, high) in [
            (100.0f64, -0.08, 0.08),
            (1000.0, -0.08, 0.08),
            (10_000.0, -0.08, 0.08),
            (15_000.0, -0.08, 0.08),
            (18_850.0, -0.08, 0.08),
            (19_000.0, -0.08, 0.08),
            (20_000.0, 0.0, 0.15),
            (20_500.0, -0.5, 0.15),
        ] {
            let w = std::f64::consts::TAU * hz / 44_100.0;
            for (k, p) in ph.iter().enumerate() {
                let frac = (k + 1) as f64 * 0.25;
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (j, &c) in p.iter().enumerate() {
                    let t = (j as f64 - (TP_LAG as f64 - 1.0) - frac) * w;
                    re += f64::from(c) * t.cos();
                    im += f64::from(c) * t.sin();
                }
                let db = 10.0 * (re * re + im * im).log10();
                assert!(low <= db && db <= high, "{hz} Hz, phase {k}: {db:.3} dB");
            }
        }
    }

    #[test]
    fn the_folded_detector_reads_what_the_full_phases_read() {
        let ph = tp_phases();
        let mut l = PeakLimit::new();
        l.setup(44_100, 1.5);
        l.set_true_peak(true);
        let mut seed = 9u32;
        for n in 0..2000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = (seed >> 9) as f32 / (1u32 << 22) as f32 - 1.0;
            l.run([v, 0.25 * v], 0.01);
            if n < TP_TAPS {
                continue;
            }
            let h = &l.hist[0][l.hist_at..l.hist_at + TP_TAPS];
            let full = ph
                .iter()
                .map(|p| dot(p, h).abs())
                .fold(h[TP_LAG - 1].abs(), f32::max);
            assert!((l.level(0, true, -1.0) - full).abs() < 1e-5, "{n}");
            assert_eq!(l.level(0, false, -1.0), h[TP_LAG - 1].abs());
        }
    }

    #[test]
    fn quiet_windows_skip_the_points_between_samples_safely() {
        // Under the quiet share of the ceiling, no point between samples
        // can pass it, even for the worst signs.
        let ph = tp_phases();
        let f = Folded::new();
        for p in &ph {
            let worst: f32 = p.iter().map(|v| v.abs()).sum();
            assert!(worst * f.quiet < 1.0, "{worst} {}", f.quiet);
        }
    }

    #[test]
    fn the_far_taps_never_take_a_point_past_what_the_near_taps_leave_room_for() {
        // Windows with no sample over a rung, random and with the signs
        // that push the far taps hardest: wherever the near taps leave the
        // room, the whole sums stay at or under the ceiling.
        let ph = tp_phases();
        let f = Folded::new();
        let ceiling = 0.8f32;
        let mut seed = 3u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 9) as f32 / (1u32 << 22) as f32 - 1.0
        };
        let mut skipped = 0;
        for n in 0..60_000 {
            let lim = ceiling * RUNGS[n % RUNGS.len()];
            let p = &ph[n % 3];
            let mut h = [0.0f32; TP_TAPS];
            for (j, v) in h.iter_mut().enumerate() {
                let far = !(NEAR_FROM..TP_TAPS - NEAR_FROM).contains(&j);
                *v = if far && n % 2 == 0 {
                    lim * p[j].signum()
                } else {
                    lim * rnd()
                };
            }
            let near = (NEAR_FROM..TP_TAPS - NEAR_FROM)
                .map(|j| p[j] * h[j])
                .sum::<f32>()
                .abs();
            if near <= ceiling - f.far * lim {
                skipped += 1;
                let full = dot(p, &h).abs();
                assert!(full <= ceiling, "{n}: {full} with {near} near");
            }
        }
        assert!(skipped > 10_000, "{skipped}");
    }

    #[test]
    fn the_latency_is_fixed_and_the_output_never_passes_the_ceiling() {
        let mut l = PeakLimit::new();
        l.setup(44_100, 1.5);
        let lat = l.latency();
        for (ms, auto, tp) in [
            (5.0, false, false),
            (80.0, true, true),
            (1000.0, true, false),
        ] {
            l.set_release(ms, auto);
            l.set_true_peak(tp);
            assert_eq!(l.latency(), lat);
            l.clear();
            let mut seed = 1u32;
            let ceiling = 0.5;
            for n in 0..20_000 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 9) as f32 / (1u32 << 22) as f32 - 1.0;
                let burst = if (n / 3000) % 2 == 0 { 4.0 } else { 0.3 };
                let ([a, b], g) = l.run([noise * burst, -0.5 * noise * burst], ceiling);
                assert!(g <= 1.0 && g > 0.0);
                assert!(
                    (a * g).abs() <= ceiling * (1.0 + 1e-6),
                    "{ms} {n}: {}",
                    a * g
                );
                assert!((b * g).abs() <= ceiling * (1.0 + 1e-6));
            }
        }
    }

    #[test]
    fn a_quiet_block_comes_out_as_the_frames_would() {
        // Blocks of noise, some loud enough to limit and some quiet, in
        // blocks of every length up to 200 frames: the block path, where
        // it is taken, gives the same numbers bit for bit as frame by
        // frame, and it is taken once the limiter has let go.
        for (ms, auto, tp) in [(80.0, false, true), (30.0, true, false), (20.0, true, true)] {
            let (mut a, mut b) = (PeakLimit::new(), PeakLimit::new());
            for p in [&mut a, &mut b] {
                p.setup(44_100, 1.5);
                p.set_release(ms, auto);
                p.set_true_peak(tp);
            }
            let ceiling = 0.7;
            let mut seed = 7u32;
            let (mut taken, mut frames) = (0, 0usize);
            for k in 0..2000 {
                let n = 1 + (k * 37) % 200;
                // Limiting, then near the ceiling, then long enough under
                // its quiet share (0.38 of it) for every release to finish.
                let level = match k % 1000 {
                    0..40 if k % 7 == 0 => 2.0,
                    0..60 => 0.6,
                    _ => 0.2,
                };
                let mut l = vec![0.0f32; n];
                let mut r = vec![0.0f32; n];
                for i in 0..n {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    let x = (seed >> 9) as f32 / (1u32 << 22) as f32 - 1.0;
                    l[i] = x * level;
                    r[i] = -0.8 * x * level;
                }
                let want: Vec<[f32; 2]> = (0..n)
                    .map(|i| {
                        let ([x, y], g) = b.run([l[i], r[i]], ceiling);
                        [x * g, y * g]
                    })
                    .collect();
                if a.quiet_block(&mut l, &mut r, 1.0, ceiling) {
                    taken += 1;
                } else {
                    for i in 0..n {
                        let ([x, y], g) = a.run([l[i], r[i]], ceiling);
                        (l[i], r[i]) = (x * g, y * g);
                    }
                }
                for i in 0..n {
                    assert_eq!(
                        [l[i].to_bits(), r[i].to_bits()],
                        [want[i][0].to_bits(), want[i][1].to_bits()],
                        "{ms} {auto} {tp}: block {k}, frame {i}"
                    );
                }
                frames += n;
            }
            assert!(
                taken > 500,
                "{ms} {auto} {tp}: {taken} of 2000 ({frames} frames)"
            );
        }
    }
}
