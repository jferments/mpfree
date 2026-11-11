//! Building blocks shared by the synths: band-limited oscillators, filters,
//! envelopes, an LFO and a seeded noise source. Plain structs, no
//! allocation, `f32` throughout.

use std::f32::consts::PI;

/// A small seeded generator (SplitMix64), so renders repeat exactly.
#[derive(Clone, Copy, Debug, Default)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 {
        // The top 24 bits fit a u32, and a u32 below 2^24 converts to f32
        // exactly, so this is the same value as going from the u64; the
        // u64 conversion is a library call on ARMv7.
        ((self.next_u64() >> 40) as u32) as f32 / (1u64 << 24) as f32
    }
    /// Uniform in [-1, 1).
    #[inline]
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// Whether two cache keys hold the same words. Compared word by word in
/// line: an array `==` becomes a `bcmp` library call on ARMv7.
#[inline]
pub(crate) fn same_key<const N: usize>(a: &[u32; N], b: &[u32; N]) -> bool {
    let mut d = 0;
    for i in 0..N {
        d |= a[i] ^ b[i];
    }
    d == 0
}

/// The polynomial band-limited step correction at phase `t` for a phase
/// increment `dt`.
#[inline]
pub fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Oscillator shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wave {
    Saw,
    Pulse,
    Triangle,
    Sine,
}

impl Wave {
    pub fn from_index(i: f32) -> Wave {
        match i as i32 {
            1 => Wave::Pulse,
            2 => Wave::Triangle,
            3 => Wave::Sine,
            _ => Wave::Saw,
        }
    }
}

/// One sample of `wave` at phase `t` (0 to 1) with increment `dt`, then
/// advances the phase. Saw and pulse are band-limited (PolyBLEP); the
/// triangle has no steps and the sine no harmonics to alias.
#[inline]
pub fn osc(wave: Wave, t: &mut f32, dt: f32, pw: f32) -> f32 {
    match wave {
        Wave::Saw => saw(t, dt),
        Wave::Pulse => pulse(t, dt, pw),
        Wave::Triangle => triangle(t, dt),
        Wave::Sine => sine(t, dt),
    }
}

/// Adds `osc(wave, ..) * level` to each frame of `buf`, advancing the
/// phase: the same samples as calling [`osc`] once a frame, with the
/// choice of shape made once for the whole buffer.
#[inline]
pub fn osc_add(wave: Wave, buf: &mut [f32], t: &mut f32, dt: f32, pw: f32, level: f32) {
    osc_into::<true>(wave, buf, t, dt, pw, level);
}

/// Like [`osc_add`] onto a buffer of zeros: each frame becomes
/// `0.0 + osc(wave, ..) * level`, so the buffer need not be cleared first.
#[inline]
pub fn osc_set(wave: Wave, buf: &mut [f32], t: &mut f32, dt: f32, pw: f32, level: f32) {
    osc_into::<false>(wave, buf, t, dt, pw, level);
}

#[inline]
fn osc_into<const ADD: bool>(
    wave: Wave,
    buf: &mut [f32],
    t: &mut f32,
    dt: f32,
    pw: f32,
    level: f32,
) {
    let mut p = *t;
    let put = |x: &mut f32, y: f32| *x = if ADD { *x } else { 0.0 } + y * level;
    match wave {
        Wave::Saw => buf.iter_mut().for_each(|x| put(x, saw(&mut p, dt))),
        Wave::Pulse => buf.iter_mut().for_each(|x| put(x, pulse(&mut p, dt, pw))),
        Wave::Triangle => buf.iter_mut().for_each(|x| put(x, triangle(&mut p, dt))),
        Wave::Sine => buf.iter_mut().for_each(|x| put(x, sine(&mut p, dt))),
    }
    *t = p;
}

#[inline]
fn advance(t: &mut f32, dt: f32) {
    let mut n = *t + dt;
    if n >= 1.0 {
        n -= 1.0;
    }
    *t = n;
}

#[inline]
fn saw(t: &mut f32, dt: f32) -> f32 {
    let p = *t;
    let y = 2.0 * p - 1.0 - poly_blep(p, dt);
    advance(t, dt);
    y
}

#[inline]
fn pulse(t: &mut f32, dt: f32, pw: f32) -> f32 {
    let p = *t;
    let mut y = if p < pw { 1.0 } else { -1.0 };
    y += poly_blep(p, dt);
    let q = p - pw;
    y -= poly_blep(if q < 0.0 { q + 1.0 } else { q }, dt);
    advance(t, dt);
    // Remove the DC a narrow pulse carries.
    y - (2.0 * pw - 1.0)
}

#[inline]
fn triangle(t: &mut f32, dt: f32) -> f32 {
    let y = 1.0 - 4.0 * (*t - 0.5).abs();
    advance(t, dt);
    y
}

#[inline]
fn sine(t: &mut f32, dt: f32) -> f32 {
    let y = (2.0 * PI * *t).sin();
    advance(t, dt);
    y
}

/// Filter types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMode {
    /// 4-pole ladder low-pass, 24 dB per octave.
    Lp24,
    /// State-variable low-pass, 12 dB per octave.
    Lp12,
    Bp12,
    Hp12,
}

impl FilterMode {
    pub fn from_index(i: f32) -> FilterMode {
        match i as i32 {
            1 => FilterMode::Lp12,
            2 => FilterMode::Bp12,
            3 => FilterMode::Hp12,
            _ => FilterMode::Lp24,
        }
    }
}

/// Coefficients for one filter setting, worked out at the control rate and
/// shared by a voice's two channels.
#[derive(Clone, Copy, Debug, Default)]
pub struct FilterCoefs {
    /// Ladder: one stage's gain `g / (1 + g)`, feedback, and makeup gain.
    big_g: f32,
    inv_1g: f32,
    k: f32,
    makeup: f32,
    /// State-variable filter.
    a1: f32,
    a2: f32,
    a3: f32,
    damp: f32,
}

impl FilterCoefs {
    /// `fc` in Hz, `res` 0 to 1 (1 is at the edge of self-oscillation).
    pub fn new(fc: f32, res: f32, rate: f32) -> FilterCoefs {
        let fc = fc.clamp(10.0, 0.45 * rate);
        let g = (PI * fc / rate).tan();
        let res = res.clamp(0.0, 1.0);
        let k = 3.95 * res;
        let damp = 2.0 - 1.95 * res;
        let a1 = 1.0 / (1.0 + g * (g + damp));
        let a2 = g * a1;
        FilterCoefs {
            big_g: g / (1.0 + g),
            inv_1g: 1.0 / (1.0 + g),
            k,
            makeup: 1.0 + 0.6 * k,
            a1,
            a2,
            a3: g * a2,
            damp,
        }
    }
}

/// One channel of filter state (ladder stages and SVF integrators).
#[derive(Clone, Copy, Debug, Default)]
pub struct Filter {
    s: [f32; 4],
}

impl Filter {
    pub fn clear(&mut self) {
        self.s = [0.0; 4];
    }

    #[inline]
    pub fn process(&mut self, mode: FilterMode, c: &FilterCoefs, x: f32) -> f32 {
        match mode {
            FilterMode::Lp24 => {
                // Zero-delay-feedback ladder (Zavalishin), linear core.
                let g = c.big_g;
                let sig = c.inv_1g
                    * (g * g * g * self.s[0] + g * g * self.s[1] + g * self.s[2] + self.s[3]);
                let g4 = g * g * g * g;
                let y4 = (g4 * x + sig) / (1.0 + c.k * g4);
                let mut u = x - c.k * y4;
                for s in &mut self.s {
                    let v = (u - *s) * g;
                    let y = v + *s;
                    *s = y + v;
                    u = y;
                }
                u * c.makeup
            }
            _ => {
                // Trapezoidal state-variable filter (Simper).
                let (ic1, ic2) = (self.s[0], self.s[1]);
                let v3 = x - ic2;
                let v1 = c.a1 * ic1 + c.a2 * v3;
                let v2 = ic2 + c.a2 * ic1 + c.a3 * v3;
                self.s[0] = 2.0 * v1 - ic1;
                self.s[1] = 2.0 * v2 - ic2;
                match mode {
                    FilterMode::Lp12 => v2,
                    FilterMode::Bp12 => v1 * c.damp,
                    _ => x - c.damp * v1 - v2,
                }
            }
        }
    }

    /// Filters `buf` in place: the same samples as calling
    /// [`Filter::process`] once a frame, with the mode chosen once and the
    /// ladder's powers of `g` worked out once for the buffer.
    #[inline]
    pub fn run(&mut self, mode: FilterMode, c: &FilterCoefs, buf: &mut [f32]) {
        match mode {
            FilterMode::Lp24 => {
                let g = c.big_g;
                let g2 = g * g;
                let g3 = g2 * g;
                let g4 = g3 * g;
                let den = 1.0 + c.k * g4;
                let [mut s0, mut s1, mut s2, mut s3] = self.s;
                for x in buf {
                    let sig = c.inv_1g * (g3 * s0 + g2 * s1 + g * s2 + s3);
                    let y4 = (g4 * *x + sig) / den;
                    let u = *x - c.k * y4;
                    let v = (u - s0) * g;
                    let y = v + s0;
                    s0 = y + v;
                    let v = (y - s1) * g;
                    let y = v + s1;
                    s1 = y + v;
                    let v = (y - s2) * g;
                    let y = v + s2;
                    s2 = y + v;
                    let v = (y - s3) * g;
                    let y = v + s3;
                    s3 = y + v;
                    *x = y * c.makeup;
                }
                self.s = [s0, s1, s2, s3];
            }
            FilterMode::Lp12 => self.svf::<0>(c, buf),
            FilterMode::Bp12 => self.svf::<1>(c, buf),
            FilterMode::Hp12 => self.svf::<2>(c, buf),
        }
    }

    /// The state-variable filter over a buffer; `OUT` 0 low-pass, 1
    /// band-pass, 2 high-pass.
    #[inline]
    fn svf<const OUT: u8>(&mut self, c: &FilterCoefs, buf: &mut [f32]) {
        let (mut ic1, mut ic2) = (self.s[0], self.s[1]);
        for x in buf {
            let v3 = *x - ic2;
            let v1 = c.a1 * ic1 + c.a2 * v3;
            let v2 = ic2 + c.a2 * ic1 + c.a3 * v3;
            ic1 = 2.0 * v1 - ic1;
            ic2 = 2.0 * v2 - ic2;
            *x = match OUT {
                0 => v2,
                1 => v1 * c.damp,
                _ => *x - c.damp * v1 - v2,
            };
        }
        self.s[0] = ic1;
        self.s[1] = ic2;
    }

    /// True when both states hold exactly the same numbers, so the same
    /// input gives the same output.
    #[inline]
    pub fn same_as(&self, other: &Filter) -> bool {
        self.s
            .iter()
            .zip(&other.s)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

/// Soft drive before the filter. `amount` 0 leaves the signal untouched;
/// above that it blends in a `tanh` stage pushed by `gain`.
#[inline]
pub fn drive(x: f32, gain: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        return x;
    }
    x + ((x * gain).tanh() / gain.sqrt() - x) * amount
}

/// [`drive`] with the gain's square root already worked out as `sq`: the
/// same sample.
#[inline]
pub fn drive_sq(x: f32, gain: f32, sq: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        return x;
    }
    x + ((x * gain).tanh() / sq - x) * amount
}

/// [`drive`] over a buffer in place, with the gain's square root worked
/// out once: the same samples.
#[inline]
pub fn drive_run(buf: &mut [f32], gain: f32, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let sq = gain.sqrt();
    for x in buf {
        *x = *x + ((*x * gain).tanh() / sq - *x) * amount;
    }
}

/// Envelope stages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Idle,
    Attack,
    Decay,
    Release,
}

/// An ADSR envelope with exponential segments. Times are the time to cover
/// about 99 % of a segment.
#[derive(Clone, Copy, Debug)]
pub struct Env {
    pub level: f32,
    pub stage: Stage,
    ca: f32,
    cd: f32,
    cr: f32,
    sustain: f32,
}

impl Default for Env {
    fn default() -> Env {
        Env {
            level: 0.0,
            stage: Stage::Idle,
            ca: 1.0,
            cd: 1.0,
            cr: 1.0,
            sustain: 1.0,
        }
    }
}

/// The per-step coefficient that covers about 99 % of a segment in `ms`
/// at `steps_per_sec`.
pub fn seg_coef(ms: f32, steps_per_sec: f32) -> f32 {
    let n = (ms * 0.001 * steps_per_sec).max(1.0);
    1.0 - (-4.6 / n).exp()
}

impl Env {
    pub fn set(&mut self, a_ms: f32, d_ms: f32, s: f32, r_ms: f32, steps_per_sec: f32) {
        self.ca = seg_coef(a_ms, steps_per_sec);
        self.cd = seg_coef(d_ms, steps_per_sec);
        self.cr = seg_coef(r_ms, steps_per_sec);
        self.sustain = s.clamp(0.0, 1.0);
    }
    /// Starts the attack from the current level, so a retrigger never clicks.
    pub fn gate_on(&mut self) {
        self.stage = Stage::Attack;
    }
    pub fn gate_off(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }
    pub fn idle(&self) -> bool {
        self.stage == Stage::Idle
    }
    pub fn clear(&mut self) {
        self.level = 0.0;
        self.stage = Stage::Idle;
    }

    #[inline]
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Idle => {}
            Stage::Attack => {
                self.level += (1.3 - self.level) * self.ca;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => self.level += (self.sustain - self.level) * self.cd,
            Stage::Release => {
                self.level += (-0.01 - self.level) * self.cr;
                if self.level <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Idle;
                }
            }
        }
        self.level
    }
}

/// LFO shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
    SampleHold,
}

impl LfoShape {
    pub fn from_index(i: f32) -> LfoShape {
        match i as i32 {
            1 => LfoShape::Triangle,
            2 => LfoShape::Saw,
            3 => LfoShape::Square,
            4 => LfoShape::SampleHold,
            _ => LfoShape::Sine,
        }
    }
}

/// The LFO's value (-1 to 1) at phase `p` (0 to 1). Sample and hold reads
/// `held`.
#[inline]
pub fn lfo_value(shape: LfoShape, p: f32, held: f32) -> f32 {
    match shape {
        LfoShape::Sine => (2.0 * PI * p).sin(),
        LfoShape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
        LfoShape::Saw => 2.0 * p - 1.0,
        LfoShape::Square => {
            if p < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        LfoShape::SampleHold => held,
    }
}

/// `2^x`, the pitch and cutoff curve.
#[inline]
pub fn exp2(x: f32) -> f32 {
    x.exp2()
}

/// The frequency of a (fractional) MIDI note.
#[inline]
pub fn note_hz(note: f32) -> f32 {
    440.0 * exp2((note - 69.0) / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saw_is_bounded_and_periodic() {
        let mut t = 0.0;
        let dt = 440.0 / 48_000.0;
        let mut peak = 0.0f32;
        for _ in 0..48_000 {
            peak = peak.max(osc(Wave::Saw, &mut t, dt, 0.5).abs());
        }
        assert!(peak <= 1.05, "peak {peak}");
    }

    #[test]
    fn env_runs_through_its_stages() {
        let mut e = Env::default();
        e.set(1.0, 10.0, 0.5, 10.0, 48_000.0);
        e.gate_on();
        for _ in 0..100 {
            e.next();
        }
        assert_eq!(e.stage, Stage::Decay);
        for _ in 0..4800 {
            e.next();
        }
        assert!((e.level - 0.5).abs() < 0.01);
        e.gate_off();
        for _ in 0..4800 {
            e.next();
        }
        assert!(e.idle());
        assert_eq!(e.level, 0.0);
    }

    #[test]
    fn lowpass_passes_dc_and_cuts_highs() {
        for mode in [FilterMode::Lp24, FilterMode::Lp12] {
            let c = FilterCoefs::new(500.0, 0.0, 48_000.0);
            let mut f = Filter::default();
            let mut y = 0.0;
            for _ in 0..48_000 {
                y = f.process(mode, &c, 1.0);
            }
            assert!((y - 1.0).abs() < 0.02, "{mode:?} dc {y}");
            // A 10 kHz sine comes out far quieter.
            let mut f = Filter::default();
            let mut peak = 0.0f32;
            for i in 0..48_000 {
                let x = (2.0 * PI * 10_000.0 * i as f32 / 48_000.0).sin();
                let y = f.process(mode, &c, x);
                if i > 4800 {
                    peak = peak.max(y.abs());
                }
            }
            assert!(peak < 0.05, "{mode:?} 10 kHz peak {peak}");
        }
    }

    #[test]
    fn highpass_blocks_dc() {
        let c = FilterCoefs::new(500.0, 0.3, 48_000.0);
        let mut f = Filter::default();
        let mut y = 1.0;
        for _ in 0..48_000 {
            y = f.process(FilterMode::Hp12, &c, 1.0);
        }
        assert!(y.abs() < 1e-3);
    }

    /// A test signal with steps, both signs and loud parts.
    fn signal(n: usize) -> Vec<f32> {
        let mut rng = Rng::new(7);
        (0..n)
            .map(|i| rng.bipolar() * if i % 50 < 25 { 3.0 } else { 0.4 })
            .collect()
    }

    #[test]
    fn buffer_oscillators_give_the_same_samples_as_one_frame_at_a_time() {
        let waves = [Wave::Saw, Wave::Pulse, Wave::Triangle, Wave::Sine];
        for wave in waves {
            for dt in [0.001, 0.013, 0.2, 0.45] {
                let (mut t, mut tb) = (0.37f32, 0.37f32);
                let mut buf = signal(16);
                let mut want = buf.clone();
                for chunk in [5usize, 16, 1, 11] {
                    let mut set = vec![9.0f32; chunk];
                    let mut t2 = tb;
                    osc_set(wave, &mut set, &mut t2, dt, 0.3, 0.7);
                    osc_add(wave, &mut buf[..chunk], &mut tb, dt, 0.3, 0.7);
                    assert_eq!(t2.to_bits(), tb.to_bits());
                    for (k, w) in want[..chunk].iter_mut().enumerate() {
                        let y = osc(wave, &mut t, dt, 0.3);
                        assert_eq!(set[k].to_bits(), (0.0 + y * 0.7).to_bits(), "{wave:?}");
                        *w += y * 0.7;
                    }
                    assert_eq!(t.to_bits(), tb.to_bits(), "{wave:?} phase");
                    let same = buf[..chunk].iter().zip(&want[..chunk]);
                    assert!(
                        same.into_iter().all(|(a, b)| a.to_bits() == b.to_bits()),
                        "{wave:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn buffer_filter_and_drive_give_the_same_samples_as_one_frame_at_a_time() {
        let modes = [
            FilterMode::Lp24,
            FilterMode::Lp12,
            FilterMode::Bp12,
            FilterMode::Hp12,
        ];
        for mode in modes {
            for (fc, res) in [(80.0, 0.0), (900.0, 0.5), (12_000.0, 1.0)] {
                let c = FilterCoefs::new(fc, res, 44_100.0);
                let (mut a, mut b) = (Filter::default(), Filter::default());
                let x = signal(4_000);
                let mut buf = x.clone();
                for chunk in buf.chunks_mut(13) {
                    drive_run(chunk, 3.5, 0.6);
                    b.run(mode, &c, chunk);
                }
                for (k, &xk) in x.iter().enumerate() {
                    let y = a.process(mode, &c, drive(xk, 3.5, 0.6));
                    assert_eq!(y.to_bits(), buf[k].to_bits(), "{mode:?} frame {k}");
                }
                assert!(a.same_as(&b));
            }
        }
        // No drive leaves the signal alone.
        let mut buf = signal(64);
        let x = buf.clone();
        drive_run(&mut buf, 4.0, 0.0);
        assert_eq!(buf, x);
    }
}
