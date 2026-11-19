//! The divide-down bank: one band-limited oscillator for every note, 0 to
//! 127, as the string machines and the combo organs make every note from
//! twelve top-octave oscillators divided down. Strings and Drawbar's Combo
//! model play it.
//!
//! - **Locked octaves.** In a tuning whose octaves are pure (12-TET at any
//!   A4 and every 12-note temperament) each note's phase is exactly twice
//!   the phase of the note an octave below, plus half a cycle, as when the
//!   top octave is divided down: an 8' and a 4' make one steady wave, two
//!   keys an octave apart never beat, and the half cycle keeps their steps
//!   apart, so a chord's peaks stay low. In any other tuning (24-TET, a
//!   stretched octave) each note runs free at its own pitch, so every key
//!   is in tune; only the locking is lost, and the cost is the same.
//! - **Phases from the absolute frame.** A note's phase follows from the
//!   absolute frame, so a silent oscillator costs nothing and comes back
//!   where it would have been.
//! - **Buses.** The engine adds each note's level for the coming control
//!   step to the buses it wants ([`Bank::add`]), and the bank renders only
//!   the notes some bus hears, each level ramping across the step, into
//!   `B` buses. Each bus takes one [`Shape`] of the notes: the bank costs
//!   what the keys use, not 128 oscillators.
//!
//! Nothing allocates; the bank is a plain value in its engine.

use mpfree_node::Tuning;

use crate::dsp::{poly_blep4, Rng};

/// Notes in the bank.
pub const NOTES: usize = 128;
/// Frames per control step: the levels ramp across one step.
pub const STEP: usize = 16;
/// An oscillator above this share of the rate is silent.
const TOP: f64 = 0.45;
const INV_24: f32 = 1.0 / 16_777_216.0;
const INV_32: f32 = 1.0 / 4_294_967_296.0;
/// A locked note's phase against twice the octave below's: half a cycle.
const LOCK_OFFSET: u32 = 1 << 31;
/// The cubic's scale: `t (t - 1/2) (t - 1)` peaks at `1 / (12 √3)`.
const CUBIC_SCALE: f32 = 20.784_609;

/// The wave a bus takes from each note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A sawtooth, band-limited with the four-point step correction
    /// ([`poly_blep4`]): every harmonic at 1/k.
    Saw,
    /// A square, as a divider's output, both steps corrected: the odd
    /// harmonics at 1/k.
    Square,
    /// The saw integrated twice, `t (t - 1/2) (t - 1)` scaled to a peak of
    /// 1: a near sine with every harmonic at 1/k³ (the second 18 dB down,
    /// the third 29 dB), the same shape at every pitch. Its first
    /// harmonic is 1.006.
    Cubic,
    // Each shape's first harmonic is in step with the saw's (the square
    // low for the first half cycle, the cubic turned over), so voices
    // mixed from one note add rather than cancel.
}

/// One cycle of `shape` at phase `t` (0 to 1) as the bank plays it, but
/// without the step correction: for pictures.
pub fn naive(shape: Shape, t: f32) -> f32 {
    match shape {
        Shape::Saw => t + t - 1.0,
        Shape::Square => {
            if t < 0.5 {
                -1.0
            } else {
                1.0
            }
        }
        Shape::Cubic => t * (t - 0.5) * (1.0 - t) * CUBIC_SCALE,
    }
}

impl Shape {
    fn index(self) -> usize {
        match self {
            Shape::Saw => 0,
            Shape::Square => 1,
            Shape::Cubic => 2,
        }
    }
}

/// The bank with `B` buses. See the module docs.
#[derive(Clone)]
pub struct Bank<const B: usize> {
    rate: u32,
    // Each note's step, its phase at frame 0, and its step as a share of
    // a cycle; the phase of each oscillator heard.
    inc: [u32; NOTES],
    base: [u32; NOTES],
    dt: [f32; NOTES],
    phase: [u32; NOTES],
    /// Each bus's level of each oscillator, its step per frame and its
    /// level at the step's end.
    amp: [[f32; NOTES]; B],
    damp: [[f32; NOTES]; B],
    target: [[f32; NOTES]; B],
    heard: [u8; NOTES],
    n_heard: usize,
    is_heard: [bool; NOTES],
}

impl<const B: usize> Default for Bank<B> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const B: usize> Bank<B> {
    pub const fn new() -> Self {
        Bank {
            rate: 48_000,
            inc: [0; NOTES],
            base: [0; NOTES],
            dt: [0.0; NOTES],
            phase: [0; NOTES],
            amp: [[0.0; NOTES]; B],
            damp: [[0.0; NOTES]; B],
            target: [[0.0; NOTES]; B],
            heard: [0; NOTES],
            n_heard: 0,
            is_heard: [false; NOTES],
        }
    }

    /// Fresh random phases from `seed`, one per note: the notes above
    /// follow from the lowest octave's where the octaves lock.
    pub fn new_phases(&mut self, seed: u64) {
        let mut rng = Rng::new(seed);
        for b in self.base.iter_mut() {
            *b = rng.next_u64() as u32;
        }
    }

    /// Silence: no level on any bus, nothing heard. The tuning and the
    /// phases stay.
    pub fn clear(&mut self) {
        self.amp = [[0.0; NOTES]; B];
        self.damp = [[0.0; NOTES]; B];
        self.target = [[0.0; NOTES]; B];
        self.is_heard = [false; NOTES];
        self.n_heard = 0;
    }

    /// Every note's step at `rate` from `tuning`, `shift` semitones up,
    /// each oscillator going on from where it is at `frame`. A note whose
    /// pitch is twice the one an octave below locks to it.
    pub fn retune(&mut self, tuning: &Tuning, shift: f32, rate: u32, frame: u64) {
        self.rate = rate;
        let now = frame as u32;
        let r = f64::from(rate);
        let mut hz = [0f64; NOTES];
        for (n, h) in hz.iter_mut().enumerate() {
            *h = f64::from(tuning.hz(n as f32 + shift));
        }
        for n in 0..NOTES {
            let at = self.base[n].wrapping_add(self.inc[n].wrapping_mul(now));
            let below = if n >= 12 { self.inc[n - 12] } else { 0 };
            let pure = n >= 12 && below != 0 && (hz[n] / (2.0 * hz[n - 12]) - 1.0).abs() < 1e-5;
            if hz[n] >= r * TOP {
                self.inc[n] = 0;
                self.base[n] = at;
            } else if pure && below < 1 << 31 {
                // Divided down: exactly twice the octave below, locked.
                self.inc[n] = below * 2;
                self.base[n] = self.base[n - 12].wrapping_mul(2).wrapping_add(LOCK_OFFSET);
            } else {
                self.inc[n] = (hz[n] / r * 4_294_967_296.0).round() as u32;
                self.base[n] = at.wrapping_sub(self.inc[n].wrapping_mul(now));
            }
            self.dt[n] = self.inc[n] as f32 * INV_32;
            if self.is_heard[n] {
                // A heard oscillator goes on from its own phase.
                let p = self.base[n].wrapping_add(self.inc[n].wrapping_mul(now));
                self.phase[n] = p;
            }
        }
    }

    /// Whether note `n` sounds at all (it is under the bank's top).
    #[inline]
    pub fn sounds(&self, n: usize) -> bool {
        self.inc[n] != 0
    }

    /// Whether note `n`'s phase is locked to the note an octave below.
    pub fn locked(&self, n: usize) -> bool {
        (12..NOTES).contains(&n)
            && self.inc[n] != 0
            && self.inc[n] == self.inc[n - 12].wrapping_mul(2)
            && self.base[n] == self.base[n - 12].wrapping_mul(2).wrapping_add(LOCK_OFFSET)
    }

    /// Note `n`'s frequency, Hz (0 when silent).
    pub fn hz(&self, n: usize) -> f64 {
        f64::from(self.inc[n]) / 4_294_967_296.0 * f64::from(self.rate)
    }

    /// Oscillators heard in this step.
    pub fn heard(&self) -> usize {
        self.n_heard
    }

    /// A control step starts: last step's targets are where the levels
    /// are now, and every target goes back to silence for the engine's
    /// [`add`](Self::add)s.
    pub fn begin_step(&mut self) {
        for i in 0..self.n_heard {
            let o = usize::from(self.heard[i]);
            for b in 0..B {
                self.amp[b][o] = self.target[b][o];
                self.target[b][o] = 0.0;
            }
        }
    }

    /// Adds `gain` to note `n`'s level on bus `bus` at the end of the step
    /// that starts at absolute frame `frame`. A note not heard yet comes
    /// in from silence at its own phase. `n` must sound
    /// ([`sounds`](Self::sounds)).
    #[inline]
    pub fn add(&mut self, bus: usize, n: usize, gain: f32, frame: u64) {
        self.target[bus][n] += gain;
        self.hear(n, frame);
    }

    /// As [`add`](Self::add), `a` on bus `bus` and `b` on the bus after it
    /// (a pair of buses one note feeds, looked up once).
    #[inline]
    pub fn add_pair(&mut self, bus: usize, n: usize, a: f32, b: f32, frame: u64) {
        self.target[bus][n] += a;
        self.target[bus + 1][n] += b;
        self.hear(n, frame);
    }

    /// Note `n` joins the notes heard this step, from silence at its own
    /// phase, if it isn't one already.
    #[inline]
    fn hear(&mut self, n: usize, frame: u64) {
        if !self.is_heard[n] {
            self.is_heard[n] = true;
            self.heard[self.n_heard] = n as u8;
            self.n_heard += 1;
            let now = frame as u32;
            self.phase[n] = self.base[n].wrapping_add(self.inc[n].wrapping_mul(now));
            for b in 0..B {
                self.amp[b][n] = 0.0;
            }
        }
    }

    /// The step's ramps, once every level is added; an oscillator silent
    /// at both ends of the step is dropped.
    pub fn end_step(&mut self) {
        let inv = 1.0 / STEP as f32;
        let mut i = 0;
        while i < self.n_heard {
            let o = usize::from(self.heard[i]);
            let mut any = false;
            for b in 0..B {
                self.damp[b][o] = (self.target[b][o] - self.amp[b][o]) * inv;
                any |= self.target[b][o] != 0.0 || self.amp[b][o] != 0.0;
            }
            if !any {
                self.is_heard[o] = false;
                self.n_heard -= 1;
                self.heard[i] = self.heard[self.n_heard];
                continue;
            }
            i += 1;
        }
    }

    /// Adds the first `n` frames (one control step at most) of every
    /// oscillator heard into the buses, bus `b` taking `shapes[b]`.
    pub fn render(&mut self, n: usize, bus: &mut [[f32; STEP]; B], shapes: &[Shape; B]) {
        if shapes.iter().all(|&s| s == shapes[0]) {
            self.render_one(n, bus, shapes[0]);
            return;
        }
        let mut waves = [[0.0f32; STEP]; 3];
        let index = shapes.map(Shape::index);
        for i in 0..self.n_heard {
            let o = usize::from(self.heard[i]);
            // The shapes some bus hears this step.
            let mut want = [false; 3];
            for b in 0..B {
                if self.amp[b][o] != 0.0 || self.damp[b][o] != 0.0 {
                    want[index[b]] = true;
                }
            }
            for (k, &shape) in [Shape::Saw, Shape::Square, Shape::Cubic].iter().enumerate() {
                if want[k] {
                    self.wave(o, shape, &mut waves[k], n);
                }
            }
            self.phase[o] = self.phase[o].wrapping_add(self.inc[o].wrapping_mul(n as u32));
            // Most notes feed few of the buses: only those.
            for b in 0..B {
                let wave = &waves[index[b]];
                self.mix_into(&mut bus[b], b, o, wave, n);
            }
        }
    }

    /// [`render`](Self::render) when every bus takes one shape (Strings):
    /// one wave a note.
    fn render_one(&mut self, n: usize, bus: &mut [[f32; STEP]; B], shape: Shape) {
        let mut wave = [0.0f32; STEP];
        for i in 0..self.n_heard {
            let o = usize::from(self.heard[i]);
            self.wave(o, shape, &mut wave, n);
            self.phase[o] = self.phase[o].wrapping_add(self.inc[o].wrapping_mul(n as u32));
            for (b, out) in bus.iter_mut().enumerate() {
                self.mix_into(out, b, o, &wave, n);
            }
        }
    }

    /// The first `n` frames of oscillator `o` as `shape`, from its phase.
    #[inline(always)]
    fn wave(&self, o: usize, shape: Shape, w: &mut [f32; STEP], n: usize) {
        let (inc, dt) = (self.inc[o], self.dt[o]);
        let mut p = self.phase[o];
        match shape {
            Shape::Saw => {
                for w in w.iter_mut().take(n) {
                    let t = (p >> 8) as f32 * INV_24;
                    p = p.wrapping_add(inc);
                    *w = t + t - 1.0 - poly_blep4(t, dt);
                }
            }
            Shape::Square => {
                for w in w.iter_mut().take(n) {
                    let t = (p >> 8) as f32 * INV_24;
                    let h = (p.wrapping_add(LOCK_OFFSET) >> 8) as f32 * INV_24;
                    let down = if p < LOCK_OFFSET { -1.0 } else { 1.0 };
                    p = p.wrapping_add(inc);
                    *w = down - poly_blep4(t, dt) + poly_blep4(h, dt);
                }
            }
            Shape::Cubic => {
                for w in w.iter_mut().take(n) {
                    let t = (p >> 8) as f32 * INV_24;
                    p = p.wrapping_add(inc);
                    *w = t * (t - 0.5) * (1.0 - t) * CUBIC_SCALE;
                }
            }
        }
    }

    /// Oscillator `o`'s `wave` into bus `b`'s `out` along its level's ramp,
    /// if it is heard on that bus at all.
    #[inline(always)]
    fn mix_into(
        &mut self,
        out: &mut [f32; STEP],
        b: usize,
        o: usize,
        wave: &[f32; STEP],
        n: usize,
    ) {
        let mut a = self.amp[b][o];
        let d = self.damp[b][o];
        if a == 0.0 && d == 0.0 {
            return;
        }
        for (x, w) in out.iter_mut().zip(wave).take(n) {
            *x += w * a;
            a += d;
        }
        self.amp[b][o] = a;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn bank() -> Bank<3> {
        let mut b = Bank::<3>::new();
        b.new_phases(1);
        b.retune(&Tuning::standard(), 0.0, RATE, 0);
        b
    }

    /// `frames` of note `n` alone on bus `bus` (shapes saw, square,
    /// cubic), at full level from the second step.
    fn play(n: usize, bus: usize, frames: usize) -> Vec<f32> {
        let mut b = bank();
        let shapes = [Shape::Saw, Shape::Square, Shape::Cubic];
        let mut out = Vec::new();
        let mut frame = 0u64;
        while out.len() < frames {
            b.begin_step();
            b.add(bus, n, 1.0, frame);
            b.end_step();
            let mut buses = [[0.0f32; STEP]; 3];
            b.render(STEP, &mut buses, &shapes);
            out.extend_from_slice(&buses[bus]);
            frame += STEP as u64;
        }
        out.split_off(STEP)
    }

    /// The amplitude of harmonic `k` of `x`, whose period is `period`
    /// frames (a whole number of periods).
    fn harmonic(x: &[f32], period: f64, k: usize) -> f64 {
        let cycles = (x.len() as f64 / period).floor();
        let len = (cycles * period).round() as usize;
        let w = std::f64::consts::TAU * k as f64 / period;
        let (mut re, mut im) = (0.0, 0.0);
        for (i, &v) in x[..len].iter().enumerate() {
            re += f64::from(v) * (w * i as f64).cos();
            im += f64::from(v) * (w * i as f64).sin();
        }
        2.0 * (re * re + im * im).sqrt() / len as f64
    }

    #[test]
    fn octaves_lock_in_twelve_tet() {
        let b = bank();
        for n in 12..100 {
            assert!(b.locked(n), "note {n}");
        }
        let want = 440.0;
        assert!((b.hz(69) / want - 1.0).abs() < 1e-6, "{}", b.hz(69));
    }

    #[test]
    fn each_shape_has_its_harmonics() {
        // A4 at 48 kHz is not a whole number of frames a period: read the
        // harmonics over many periods of the bank's own pitch.
        let b = bank();
        let period = f64::from(RATE) / b.hz(57);
        let saw = play(57, 0, 48_000);
        let square = play(57, 1, 48_000);
        let cubic = play(57, 2, 48_000);
        let db = |a: f64, b: f64| 20.0 * (a / b).log10();
        // The saw: every harmonic at 1/k (within 0.3 dB to the 5th).
        let s1 = harmonic(&saw, period, 1);
        assert!((s1 - 2.0 / std::f64::consts::PI).abs() < 0.01, "{s1}");
        for k in 2..=5 {
            let got = db(harmonic(&saw, period, k), s1);
            let want = -20.0 * (k as f64).log10();
            assert!((got - want).abs() < 0.3, "saw {k}: {got}");
        }
        // The square: the odd harmonics at 1/k, the even ones gone.
        let q1 = harmonic(&square, period, 1);
        assert!((q1 - 4.0 / std::f64::consts::PI).abs() < 0.02, "{q1}");
        assert!(db(harmonic(&square, period, 2), q1) < -60.0);
        assert!((db(harmonic(&square, period, 3), q1) + 9.54).abs() < 0.3);
        // The cubic: a near sine, the harmonics at 1/k³.
        let c1 = harmonic(&cubic, period, 1);
        assert!((c1 - 1.0055).abs() < 0.01, "{c1}");
        for k in 2..=4 {
            let got = db(harmonic(&cubic, period, k), c1);
            let want = -60.0 * (k as f64).log10();
            assert!((got - want).abs() < 0.5, "cubic {k}: {got} against {want}");
        }
    }

    #[test]
    fn a_silent_oscillator_comes_back_where_it_would_have_been() {
        // Note 60 on its own from frame 0, against note 60 heard only from
        // frame 4800: the same samples from then on.
        let shapes = [Shape::Saw; 1];
        let run = |from: u64| {
            let mut b = Bank::<1>::new();
            b.new_phases(9);
            b.retune(&Tuning::standard(), 0.0, RATE, 0);
            let mut out = Vec::new();
            let mut frame = 0u64;
            while frame < 9_600 {
                b.begin_step();
                if frame >= from {
                    b.add(0, 60, 1.0, frame);
                }
                b.end_step();
                let mut buses = [[0.0f32; STEP]; 1];
                b.render(STEP, &mut buses, &shapes);
                out.extend_from_slice(&buses[0]);
                frame += STEP as u64;
            }
            out
        };
        let (a, b) = (run(0), run(4_800));
        assert_eq!(&a[4_816..], &b[4_816..]);
    }

    #[test]
    fn a_dropped_note_is_forgotten() {
        let mut b = bank();
        b.begin_step();
        b.add(0, 60, 1.0, 0);
        b.end_step();
        assert_eq!(b.heard(), 1);
        // Down to silence over a step, then dropped.
        for _ in 0..2 {
            b.begin_step();
            b.end_step();
        }
        assert_eq!(b.heard(), 0);
    }
}
