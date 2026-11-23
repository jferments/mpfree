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

/// The four-point band-limited step correction (the integrated cubic
/// B-spline's residual, Välimäki, Pekonen and Nam, "Perceptually informed
/// synthesis of bandlimited classical waveforms using integrated
/// polynomial interpolation", JASA 2012) at phase `t` for a phase
/// increment `dt`: the same sign and scale as [`poly_blep`], spread over
/// two samples either side of the step, so far less folds back. It rolls
/// the top octave off a little more than [`poly_blep`]. Needs `dt` under
/// a quarter; above that it gives [`poly_blep`].
#[inline]
pub fn poly_blep4(t: f32, dt: f32) -> f32 {
    if dt >= 0.25 {
        return poly_blep(t, dt);
    }
    let two = dt + dt;
    if t < two {
        // x samples after the step, 0 to 2.
        let x = t / dt;
        if x < 1.0 {
            let x2 = x * x;
            2.0 * (-0.5 + x * (2.0 / 3.0) - x2 * x * (1.0 / 3.0) + x2 * x2 * 0.125)
        } else {
            let u = 2.0 - x;
            let u2 = u * u;
            -2.0 * u2 * u2 * (1.0 / 24.0)
        }
    } else if t > 1.0 - two {
        // x samples before the step, -2 to 0.
        let x = (t - 1.0) / dt;
        if x > -1.0 {
            let x2 = x * x;
            2.0 * (0.5 + x * (2.0 / 3.0) - x2 * x * (1.0 / 3.0) - x2 * x2 * 0.125)
        } else {
            let u = x + 2.0;
            let u2 = u * u;
            2.0 * u2 * u2 * (1.0 / 24.0)
        }
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

/// The drive's clipping level for a mix at level 1 (one oscillator at
/// full level; see [`drive_scales`]): a signal peaking here leaves the
/// `tanh` stage peaking here too, at any gain, so a driven voice starts
/// out near its own level and [`DriveLevel`] only has the waveform's shape
/// left to correct. (Until live test 2 the stage divided by the gain's
/// square root, and a full voice came out up to 14 dB quieter at 24 dB of
/// drive: "Drive seems to make synths quieter".)
pub const DRIVE_REF: f32 = 1.0;

/// The quietest mix [`drive_scales`] scales for: quieter mixes are driven
/// as if they were this loud, so a nearly silent oscillator is not pushed
/// up into full distortion.
const MIN_DRIVE_LEVEL: f32 = 0.1;

/// What the drive's `tanh` stage is scaled by at `gain` for a mix at
/// level 1 (see [`DRIVE_REF`]). Work it out once a run or a hit.
#[inline]
pub fn drive_norm(gain: f32) -> f32 {
    DRIVE_REF / (gain * DRIVE_REF).tanh()
}

/// About how loud a mix into the drive is, for [`drive_scales`]: `stacks`
/// unison stacks of two oscillators at `osc` levels, plus a sub and noise.
/// Stacks detuned against each other add in power, so four stacks are
/// about twice as loud as one.
#[inline]
pub fn mix_level(stacks: usize, osc: [f32; 2], sub: f32, noise: f32) -> f32 {
    (stacks as f32 * (osc[0] * osc[0] + osc[1] * osc[1]) + sub * sub + noise * noise).sqrt()
}

/// The drive's input gain and output scale at `gain` for a mix at `level`
/// ([`mix_level`]), for [`drive_n`] and [`drive_run_n`]: the drive treats
/// the mix as if one full oscillator fed it, so how hard it drives and how
/// loud it comes out don't depend on how many oscillators and unison
/// stacks feed it.
#[inline]
pub fn drive_scales(gain: f32, level: f32) -> (f32, f32) {
    let lv = level.max(MIN_DRIVE_LEVEL);
    (gain / lv, drive_norm(gain) * lv)
}

/// Soft drive before the filter, for a mix at level 1. `amount` 0 leaves
/// the signal untouched; above that it blends in a `tanh` stage pushed by
/// `gain`, scaled to keep the level ([`DRIVE_REF`]).
#[inline]
pub fn drive(x: f32, gain: f32, amount: f32) -> f32 {
    drive_n(x, gain, drive_norm(gain), amount)
}

/// [`drive`] with the input gain `k` and output scale `n` worked out
/// ([`drive_scales`], or `gain` and [`drive_norm`] for level 1).
#[inline]
pub fn drive_n(x: f32, k: f32, n: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        return x;
    }
    x + ((x * k).tanh() * n - x) * amount
}

/// How long [`DriveLevel`] listens: its powers follow the sound with
/// about this time constant, long enough to span a low note's cycle.
const DRIVE_LEVEL_SECONDS: f32 = 0.04;

/// Keeps a driven voice as loud as it was without the drive, whatever its
/// waveform: the power going into the drive and coming out is summed as it
/// runs ([`drive_run_level`] or [`DriveLevel::add`]), and once a control
/// step [`DriveLevel::step`] gives the gain that brings the output back to
/// the input's level, for the voice's gain (which already ramps across the
/// step, so the correction never clicks). The drive then adds grit, not
/// level, as the FX Drive's auto-gain does. A static scale alone can't do
/// it: a pad or organ table with a high crest factor comes out of a
/// clipper much louder than a saw does.
#[derive(Clone, Copy, Debug)]
pub struct DriveLevel {
    sum_in: f32,
    sum_out: f32,
    /// Frames summed since the sums were last taken.
    frames: u32,
    p_in: f32,
    p_out: f32,
    gain: f32,
    /// For a drive with latency ([`DriveLevel::step_late`]): the squares
    /// of the last two inputs summed (`late`, the newer first) and of the
    /// two before the frames now summed (`late_from`), so that the input's
    /// power can be taken a frame and a half late, as the output comes
    /// out, at the step rather than in the sample loop (on fixed frames,
    /// so it is the same however blocks are cut).
    late: [f32; 2],
    late_from: [f32; 2],
}

/// The least a note's first measure is taken over by
/// [`DriveLevel::step_late`], in frames: a note that starts near the end of
/// a control step has only a frame or two in it, too little to judge a
/// level by behind a drive with latency, whose output hasn't arrived yet,
/// so its sums carry on into the next step.
const DRIVE_LEVEL_FIRST: u32 = 16;

impl Default for DriveLevel {
    fn default() -> DriveLevel {
        DriveLevel::new()
    }
}

impl DriveLevel {
    pub const fn new() -> DriveLevel {
        DriveLevel {
            sum_in: 0.0,
            sum_out: 0.0,
            frames: 0,
            p_in: 0.0,
            p_out: 0.0,
            gain: 1.0,
            late: [0.0; 2],
            late_from: [0.0; 2],
        }
    }

    /// One sample into the drive, `x`, and out of it, `y`.
    #[inline]
    pub fn add(&mut self, x: f32, y: f32) {
        self.sum_in += x * x;
        self.sum_out += y * y;
        self.frames += 1;
    }

    /// Once a control step of `frames` at `rate`: folds the step's sums
    /// into the running powers and gives the gain (1 until there is
    /// something to measure; never more than 18 dB either way).
    pub fn step(&mut self, frames: u64, rate: u32) -> f32 {
        self.step_after(frames, rate, 0)
    }

    /// [`DriveLevel::step`] for a drive with latency (Poly's oversampled
    /// one): a note's first measure waits until it spans 16 frames
    /// (`DRIVE_LEVEL_FIRST`), and until then the gain is the one that keeps
    /// a quiet sound as loud as it was, `1 / quiet`, `quiet` being the
    /// stage's own gain where its curve is still a straight line
    /// ([`soft_quiet_gain`]). A note starts quiet, so its first frames
    /// come out at their undriven level instead of up to 18 dB over it.
    pub fn step_late(&mut self, frames: u64, rate: u32, quiet: f32) -> f32 {
        let first = self.p_in == 0.0 && self.p_out == 0.0;
        if first && self.frames < DRIVE_LEVEL_FIRST {
            self.gain = (1.0 / quiet).clamp(0.125, 8.0);
            return self.gain;
        }
        self.step_after(frames, rate, DRIVE_LEVEL_FIRST)
    }

    /// The input's power summed since the sums were last taken, a frame
    /// and a half late: the frames summed less the last one and half the
    /// one before it, plus the same of the two before them.
    #[inline]
    fn late_sum_in(&self) -> f32 {
        let [a1, a2] = self.late_from;
        let [b1, b2] = self.late;
        self.sum_in + (a1 + 0.5 * a2) - (b1 + 0.5 * b2)
    }

    #[inline]
    fn step_after(&mut self, frames: u64, rate: u32, first_frames: u32) -> f32 {
        let first = self.p_in == 0.0 && self.p_out == 0.0;
        if first && self.frames < first_frames {
            // Too short a start to judge: keep summing into the next step.
            return self.gain;
        }
        let si = if first_frames > 0 {
            let si = self.late_sum_in();
            self.late_from = self.late;
            si
        } else {
            self.sum_in
        };
        let so = self.sum_out;
        self.sum_in = 0.0;
        self.sum_out = 0.0;
        self.frames = 0;
        if first {
            // The first step of a note: its own sums, not a slow rise.
            self.p_in = si;
            self.p_out = so;
        } else {
            let a = (frames as f32 / (DRIVE_LEVEL_SECONDS * rate as f32)).min(1.0);
            self.p_in += a * (si - self.p_in);
            self.p_out += a * (so - self.p_out);
        }
        if self.p_in > 1e-12 && self.p_out > 1e-12 {
            self.gain = (self.p_in / self.p_out).sqrt().clamp(0.125, 8.0);
        }
        self.gain
    }

    /// Forget everything (a fresh note, or the drive turned off).
    pub fn clear(&mut self) {
        *self = DriveLevel::new();
    }
}

/// [`drive_run_n`], summing the power in and out into `level`.
#[inline]
pub fn drive_run_level(buf: &mut [f32], k: f32, n: f32, amount: f32, level: &mut DriveLevel) {
    if amount <= 0.0 {
        return;
    }
    let len = buf.len() as u32;
    let (mut si, mut so) = (level.sum_in, level.sum_out);
    for x in buf {
        let y = *x + ((*x * k).tanh() * n - *x) * amount;
        si += *x * *x;
        so += y * y;
        *x = y;
    }
    level.sum_in = si;
    level.sum_out = so;
    level.frames += len;
}

/// Where [`soft_clip`] stops bending: flat at ±1 from here out.
pub const SOFT_KNEE: f32 = 1.5;

/// Poly's drive curve, a cubic soft clip: `x - 4x³/27` up to ±1.5 and ±1
/// beyond, with no corner (its slope comes down to 0 at ±1.5). Once it is
/// pushed it plays as `tanh` does (the odd harmonics of a sine within
/// 1.5 dB of `tanh`'s from 12 dB of drive up); under about 9 dB it is a
/// little cleaner, mostly a third harmonic. Its antiderivatives are
/// polynomials, so the second-order antialiasing in [`DriveAa`] needs no
/// `exp` or `ln`. (Until DSP quality step 2.1 Poly drove a `tanh`.)
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x.abs() >= SOFT_KNEE {
        1f32.copysign(x)
    } else {
        x - (4.0 / 27.0) * x * x * x
    }
}

/// The gain of Poly's drive stage ([`drive_run_aa`] with [`soft_scales`])
/// on a quiet sound, where [`soft_clip`] is still a straight line: `k n`
/// blended in by `amount`. It doesn't depend on the mix's level.
#[inline]
pub fn soft_quiet_gain(gain: f32, amount: f32) -> f32 {
    let kn = gain * DRIVE_REF / soft_clip(gain * DRIVE_REF);
    1.0 + (kn - 1.0) * amount.min(1.0)
}

/// [`drive_scales`] for [`soft_clip`]: the input gain and output scale
/// that drive a mix at `level` ([`mix_level`]) as one full oscillator
/// and bring a full one out at its own level.
#[inline]
pub fn soft_scales(gain: f32, level: f32) -> (f32, f32) {
    let lv = level.max(MIN_DRIVE_LEVEL);
    (gain / lv, DRIVE_REF / soft_clip(gain * DRIVE_REF) * lv)
}

/// The half-bands round Poly's drive: three first-order all-pass sections
/// in two branches (the polyphase IIR design of Valenzuela and
/// Constantinides, `mpfree_fx::oversample::iir_halfband(3, 0.05)`). Flat
/// to within 0.01 dB up to 20 kHz at 44.1 kHz; 35 to 45 dB down above
/// 24.1 kHz, enough here because the antialiasing has already taken most
/// of what folds.
const DRIVE_HB: [f32; 3] = [0.189_894_76, 0.551_678_24, 0.860_015_5];

/// Below this step the divided differences in [`soft_adaa2_mixed`] take
/// a midpoint instead (in 64-bit floats, where the curve's antiderivatives
/// near the knee keep 15 digits).
const SOFT_TOL: f64 = 1e-4;

/// The antiderivative of [`soft_clip`], zero at 0.
#[inline]
fn soft_f1(x: f64) -> f64 {
    let a = x.abs();
    if a < 1.5 {
        a * a * (0.5 - a * a * (1.0 / 27.0))
    } else {
        0.9375 + (a - 1.5)
    }
}

/// The antiderivative of [`soft_f1`], zero at 0 (odd).
#[inline]
fn soft_f2(x: f64) -> f64 {
    let a = x.abs();
    let v = if a < 1.5 {
        a * a * a * (1.0 / 6.0 - a * a * (1.0 / 135.0))
    } else {
        let b = a - 1.5;
        0.506_25 + b * (0.9375 + 0.5 * b)
    };
    v.copysign(x)
}

/// [`soft_clip`] in 64-bit floats.
#[inline]
fn soft_f(x: f64) -> f64 {
    if x.abs() >= 1.5 {
        1f64.copysign(x)
    } else {
        x - (4.0 / 27.0) * x * x * x
    }
}

/// The first divided difference of [`soft_f2`] over `a` and `b`.
#[inline]
fn soft_d(a: f64, b: f64) -> f64 {
    let d = a - b;
    if d.abs() < SOFT_TOL {
        soft_f1(0.5 * (a + b))
    } else {
        (soft_f2(a) - soft_f2(b)) / d
    }
}

/// Second-order ADAA of [`soft_clip`] over inputs `x0` (newest), `x1`
/// and `x2` that lie on both sides of a knee: twice the second divided
/// difference of its second antiderivative, with the usual fallbacks
/// where two inputs nearly meet.
#[inline(never)]
fn soft_adaa2_mixed(x0: f32, x1: f32, x2: f32) -> f32 {
    let (x0, x1, x2) = (f64::from(x0), f64::from(x1), f64::from(x2));
    let y = if (x0 - x2).abs() < SOFT_TOL {
        let xb = 0.5 * (x0 + x2);
        let delta = xb - x1;
        if delta.abs() < SOFT_TOL {
            soft_f(0.5 * (xb + x1))
        } else {
            (2.0 / delta) * (soft_f1(xb) + (soft_f2(x1) - soft_f2(xb)) / delta)
        }
    } else {
        2.0 * (soft_d(x0, x1) - soft_d(x1, x2)) / (x0 - x2)
    };
    y as f32
}

/// Second-order ADAA of [`soft_clip`] over three inputs that all lie
/// between the knees, from their power sums `p1` (the inputs), `p2`
/// (their squares) and `p3` (their cubes): `2 (x0 + x1 + x2) / 6 -
/// 2 h3 / 135`, `h3` the sum of every product of three of them,
/// `(p1³ + 3 p1 p2 + 2 p3) / 6`.
#[inline(always)]
fn soft_adaa2_poly(p1: f32, p2: f32, p3: f32) -> f32 {
    p1 * (1.0 / 3.0) - (p1 * (p1 * p1 + 3.0 * p2) + 2.0 * p3) * (1.0 / 405.0)
}

/// Which side of the knees `u` is on: -1, 0 (between) or 1.
#[inline(always)]
fn soft_side(u: f32) -> i32 {
    i32::from(u > SOFT_KNEE) - i32::from(u < -SOFT_KNEE)
}

/// One first-order all-pass at the base rate, `y = a (x - y1) + x1`.
#[inline(always)]
fn hb_ap(a: f32, s: &mut [f32; 2], x: f32) -> f32 {
    let y = a * (x - s[1]) + s[0];
    *s = [x, y];
    y
}

/// The antialiasing round Poly's drive and distortion (DSP quality step
/// 2.1). The curve ([`soft_clip`]) runs at twice the rate between the
/// half-bands ([`DRIVE_HB`]), and each sample there is the curve averaged
/// twice over the last three inputs, from its second antiderivative
/// (second-order antiderivative antialiasing: Bilbao, Esqueda, Parker and
/// Välimäki, "Antiderivative antialiasing for memoryless nonlinearities",
/// IEEE SPL 2017). Between the knees that average is a polynomial in the
/// three inputs, and past one knee it is ±1, so only the samples that
/// straddle a knee take the divisions. It delays the driven sound by
/// about two frames, with some phase turn near the top, and is flat
/// within 1.5 dB to 10 kHz and 3.7 dB to 15 kHz (the first-order ADAA at
/// the base rate before it lost 4.8 and 12.7). The fold-back of a saw
/// driven 24 dB came down from -50 to -77 dB at C5 and from -45 to -72
/// at C6 (A1).
#[derive(Clone, Copy, Debug, Default)]
pub struct DriveAa {
    /// The half-bands' sections, `[x1, y1]` each: the up-sampler's and
    /// the down-sampler's.
    up: [[f32; 2]; 3],
    down: [[f32; 2]; 3],
    /// The last two inputs to the curve at twice the rate (`u1` the
    /// newer), their squares and cubes, and their sides of the knees.
    u1: f32,
    u2: f32,
    s1: f32,
    s2: f32,
    c1: f32,
    c2: f32,
    r1: i32,
    r2: i32,
    /// Set by the first sample after a [`DriveAa::clear`].
    on: bool,
}

impl DriveAa {
    /// Forget everything (a fresh note, or the drive turned off).
    pub fn clear(&mut self) {
        *self = DriveAa::default();
    }

    /// True when both would carry on alike: neither started yet (a
    /// cleared memory starts afresh from its first input, whatever it
    /// held), or both started with exactly the same numbers. The test runs
    /// every control step for every voice, drive on or off, so it stops
    /// at the first difference.
    #[inline]
    pub fn same_as(&self, other: &DriveAa) -> bool {
        if !self.on || !other.on {
            return self.on == other.on;
        }
        let same = |a: f32, b: f32| a.to_bits() == b.to_bits();
        let sections = |a: &[[f32; 2]; 3], b: &[[f32; 2]; 3]| {
            a.iter()
                .flatten()
                .zip(b.iter().flatten())
                .all(|(&x, &y)| same(x, y))
        };
        self.r1 == other.r1
            && self.r2 == other.r2
            && same(self.u1, other.u1)
            && same(self.u2, other.u2)
            && same(self.s1, other.s1)
            && same(self.s2, other.s2)
            && same(self.c1, other.c1)
            && same(self.c2, other.c2)
            && sections(&self.up, &other.up)
            && sections(&self.down, &other.down)
    }

    /// Starts from a steady input, `u` into the curve coming out as `w`:
    /// every state as if it had always been there, so a drive turned on
    /// mid-note starts without a bump.
    fn start(&mut self, u: f32, w: f32) {
        self.up = [[u; 2]; 3];
        self.down = [[w; 2]; 3];
        self.u1 = u;
        self.u2 = u;
        self.s1 = u * u;
        self.s2 = u * u;
        self.c1 = u * u * u;
        self.c2 = u * u * u;
        self.r1 = soft_side(u);
        self.r2 = self.r1;
        self.on = true;
    }

    /// Flushes half-band states too small to hear to zero (once a run).
    fn settle(&mut self) {
        for v in self.up.iter_mut().chain(self.down.iter_mut()).flatten() {
            if v.abs() < 1e-20 {
                *v = 0.0;
            }
        }
    }

    /// The drive over `buf` in place: `x` becomes
    /// `x + (soft(x k) n - x) amount`, the dry part taken from the middle
    /// input of each average so it lines up with the wet. With `LEVEL`
    /// the power in and out is summed into `level`. Kept out of line: the
    /// wrappers below skip it when the drive is off, and Poly's voice loop
    /// stays small.
    #[inline(never)]
    fn run<const LEVEL: bool>(
        &mut self,
        buf: &mut [f32],
        k: f32,
        n: f32,
        amount: f32,
        level: &mut DriveLevel,
    ) {
        if amount <= 0.0 || buf.is_empty() {
            return;
        }
        let amount = amount.min(1.0);
        let wet = n * amount;
        let dry = (1.0 - amount) / k;
        if !self.on {
            let u = buf[0] * k;
            self.start(u, soft_clip(u) * wet + u * dry);
            if LEVEL {
                // As if the first input had always been there.
                level.late = [buf[0] * buf[0]; 2];
                level.late_from = level.late;
            }
        }
        // Every state in a local for the loop: the half-bands' sections,
        // then the last two inputs to the curve with their squares, cubes
        // and sides.
        let [h0, h1, h2] = DRIVE_HB;
        let [mut ua, mut ub, mut uc] = self.up;
        let [mut da, mut db, mut dc] = self.down;
        let (mut u1, mut u2) = (self.u1, self.u2);
        let (mut s1, mut s2) = (self.s1, self.s2);
        let (mut c1, mut c2) = (self.c1, self.c2);
        let (mut r1, mut r2) = (self.r1, self.r2);
        // The input's power is summed as it comes, with the last two
        // inputs' squares kept for the step to take it a frame and a half
        // late (`DriveLevel::late_sum_in`).
        let len = buf.len();
        let last = |i: usize| buf[len - 1 - i] * buf[len - 1 - i];
        let late = if len >= 2 {
            [last(0), last(1)]
        } else {
            [last(0), level.late[0]]
        };
        let (mut si, mut so) = (level.sum_in, level.sum_out);
        for x in buf.iter_mut() {
            let u = *x * k;
            let e = hb_ap(h0, &mut ua, u);
            let a = hb_ap(h2, &mut uc, e);
            let b = hb_ap(h1, &mut ub, u);
            // The curve averaged over a and the two before it, then over b,
            // a and the one before.
            let (sa, ca, ra) = (a * a, a * a * a, soft_side(a));
            let ya = if ra == r1 && ra == r2 {
                if ra == 0 {
                    soft_adaa2_poly(a + u1 + u2, sa + s1 + s2, ca + c1 + c2)
                } else {
                    ra as f32
                }
            } else {
                soft_adaa2_mixed(a, u1, u2)
            };
            let (sb, cb, rb) = (b * b, b * b * b, soft_side(b));
            let yb = if rb == ra && rb == r1 {
                if rb == 0 {
                    soft_adaa2_poly(b + a + u1, sb + sa + s1, cb + ca + c1)
                } else {
                    rb as f32
                }
            } else {
                soft_adaa2_mixed(b, a, u1)
            };
            let wa = ya * wet + u1 * dry;
            let wb = yb * wet + a * dry;
            (u2, s2, c2, r2) = (a, sa, ca, ra);
            (u1, s1, c1, r1) = (b, sb, cb, rb);
            let e = hb_ap(h0, &mut da, wb);
            let e = hb_ap(h2, &mut dc, e);
            let o = hb_ap(h1, &mut db, wa);
            let y = 0.5 * (e + o);
            if LEVEL {
                si += *x * *x;
                so += y * y;
            }
            *x = y;
        }
        if LEVEL {
            level.sum_in = si;
            level.sum_out = so;
            level.frames += len as u32;
            level.late = late;
        }
        self.up = [ua, ub, uc];
        self.down = [da, db, dc];
        (self.u1, self.u2, self.s1, self.s2) = (u1, u2, s1, s2);
        (self.c1, self.c2, self.r1, self.r2) = (c1, c2, r1, r2);
        self.settle();
    }
}

/// Poly's drive over a buffer in place ([`DriveAa`]): [`soft_clip`]
/// pushed by `k` and scaled by `n` ([`soft_scales`]), blended in by
/// `amount`.
#[inline]
pub fn drive_run_aa(buf: &mut [f32], k: f32, n: f32, amount: f32, st: &mut DriveAa) {
    if amount > 0.0 && !buf.is_empty() {
        st.run::<false>(buf, k, n, amount, &mut DriveLevel::new());
    }
}

/// [`drive_run_aa`], summing the power in and out into `level`.
#[inline]
pub fn drive_run_level_aa(
    buf: &mut [f32],
    k: f32,
    n: f32,
    amount: f32,
    level: &mut DriveLevel,
    st: &mut DriveAa,
) {
    if amount > 0.0 && !buf.is_empty() {
        st.run::<true>(buf, k, n, amount, level);
    }
}

/// The drum voices' drive (until live test 2, every engine's): a `tanh`
/// stage divided by the gain's square root `sq`. A drum hit comes into it
/// well under full level, where this lifts it by up to the square root of
/// the gain, so drums keep it; at full level it loses level, which is why
/// the synths moved to [`drive_n`].
#[inline]
pub fn drive_sq(x: f32, gain: f32, sq: f32, amount: f32) -> f32 {
    if amount <= 0.0 {
        return x;
    }
    x + ((x * gain).tanh() / sq - x) * amount
}

/// [`drive`] over a buffer in place: the same samples.
#[inline]
pub fn drive_run(buf: &mut [f32], gain: f32, amount: f32) {
    drive_run_n(buf, gain, drive_norm(gain), amount);
}

/// [`drive_n`] over a buffer in place: the same samples.
#[inline]
pub fn drive_run_n(buf: &mut [f32], k: f32, n: f32, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    for x in buf {
        *x = *x + ((*x * k).tanh() * n - *x) * amount;
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
            Stage::Decay => {
                self.level += (self.sustain - self.level) * self.cd;
                // Toward a sustain of 0 the level would sink through the
                // denormals for as long as the key is held, and every frame
                // it scales would be slow; under -400 dB it is exact 0.
                if self.level < 1e-20 {
                    self.level = 0.0;
                }
            }
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
    #[test]
    fn a_decay_to_no_sustain_lands_on_exact_zero_not_the_denormals() {
        let mut e = super::Env::default();
        e.set(1.0, 5.0, 0.0, 100.0, 44_100.0);
        e.gate_on();
        for _ in 0..44_100 * 3 {
            let x = e.next();
            assert!(!x.is_subnormal());
        }
        assert_eq!(e.level, 0.0);
        assert_eq!(e.stage, super::Stage::Decay, "still held");
    }

    #[test]
    fn the_four_point_step_correction_is_continuous_and_meets_the_step() {
        let dt = 0.01;
        // At the step it jumps by 2, as the naive saw falls by 2.
        let before = poly_blep4(1.0 - 1e-7, dt);
        let after = poly_blep4(0.0, dt);
        assert!((before - after - 2.0).abs() < 1e-3, "{before} {after}");
        // Continuous where the pieces meet, and 0 at the window's ends.
        for t in [dt, 1.0 - dt] {
            let (a, b) = (poly_blep4(t - 1e-6, dt), poly_blep4(t + 1e-6, dt));
            assert!((a - b).abs() < 1e-3, "{t}: {a} {b}");
        }
        assert!(poly_blep4(2.0 * dt - 1e-6, dt).abs() < 1e-4);
        assert!(poly_blep4(1.0 - 2.0 * dt + 1e-6, dt).abs() < 1e-4);
        assert_eq!(poly_blep4(0.5, dt), 0.0);
    }

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

    #[test]
    fn the_soft_clips_antiderivatives_are_its_own() {
        // Their slopes by central differences, across both knees.
        let h = 1e-5;
        for i in 0..=600 {
            let x = -3.0 + f64::from(i) * 0.01;
            let d1 = (soft_f1(x + h) - soft_f1(x - h)) / (2.0 * h);
            let d2 = (soft_f2(x + h) - soft_f2(x - h)) / (2.0 * h);
            assert!((d1 - soft_f(x)).abs() < 1e-8, "f1' at {x}: {d1}");
            assert!((d2 - soft_f1(x)).abs() < 1e-8, "f2' at {x}: {d2}");
            assert!(
                (soft_f(x) as f32 - soft_clip(x as f32)).abs() < 1e-6,
                "at {x}"
            );
        }
        assert_eq!(soft_clip(1.5), 1.0);
        assert_eq!(soft_clip(-7.0), -1.0);
    }

    #[test]
    fn the_adaa_between_the_knees_is_its_general_form() {
        let mut rng = Rng::new(7);
        for _ in 0..20_000 {
            let [x0, x1, x2] = [0; 3].map(|_| rng.bipolar() * 1.49);
            let p = soft_adaa2_poly(
                x0 + x1 + x2,
                x0 * x0 + x1 * x1 + x2 * x2,
                x0 * x0 * x0 + x1 * x1 * x1 + x2 * x2 * x2,
            );
            let m = soft_adaa2_mixed(x0, x1, x2);
            assert!((p - m).abs() < 1e-5, "{x0} {x1} {x2}: {p} against {m}");
        }
        // Three equal inputs give the curve itself, on either side.
        for x in [-2.0f32, -1.2, -0.3, 0.0, 0.7, 1.49, 3.0] {
            let m = soft_adaa2_mixed(x, x, x);
            assert!((m - soft_clip(x)).abs() < 1e-6, "{x}: {m}");
        }
    }

    #[test]
    fn the_antialiased_drive_starts_without_a_bump_and_keeps_a_low_notes_level() {
        let (k, n) = soft_scales(4.0, 1.0);
        // A steady input comes out steady from its first frame, as the
        // curve gives it: a drive turned on mid-note doesn't bump.
        for x in [0.05f32, 0.3, -0.6, 0.9] {
            let mut st = DriveAa::default();
            let mut buf = vec![x; 200];
            drive_run_aa(&mut buf, k, n, 0.7, &mut st);
            let want = x + (soft_clip(x * k) * n - x) * 0.7;
            for (i, y) in buf.iter().enumerate() {
                assert!((y - want).abs() < 2e-6, "{x} frame {i}: {y} against {want}");
            }
        }
        // A low note is as loud as through the plain curve, block by block
        // or all at once, to the bit.
        let x: Vec<f32> = (0..8_820)
            .map(|i| (std::f32::consts::TAU * 110.0 * i as f32 / 44_100.0).sin() * 0.9)
            .collect();
        let (mut a, mut b) = (DriveAa::default(), DriveAa::default());
        let mut whole = x.clone();
        drive_run_aa(&mut whole, k, n, 1.0, &mut a);
        let mut parts = x.clone();
        for chunk in parts.chunks_mut(16) {
            drive_run_aa(chunk, k, n, 1.0, &mut b);
        }
        assert_eq!(whole, parts);
        assert!(a.same_as(&b));
        let power = |v: &[f32]| v[441..].iter().map(|y| y * y).sum::<f32>();
        let plain: Vec<f32> = x.iter().map(|&x| soft_clip(x * k) * n).collect();
        let db = 10.0 * (power(&whole) / power(&plain)).log10();
        assert!(db.abs() < 0.05, "{db} dB");
    }

    #[test]
    fn two_drives_are_the_same_only_while_they_carry_on_alike() {
        let (k, n) = soft_scales(8.0, 1.0);
        let (mut a, mut b) = (DriveAa::default(), DriveAa::default());
        assert!(a.same_as(&b));
        let mut buf = signal(64);
        drive_run_aa(&mut buf, k, n, 1.0, &mut a);
        assert!(!a.same_as(&b), "a started drive against a fresh one");
        let mut buf = signal(64);
        drive_run_aa(&mut buf, k, n, 1.0, &mut b);
        assert!(a.same_as(&b));
        let mut buf = vec![0.25; 3];
        drive_run_aa(&mut buf, k, n, 1.0, &mut b);
        assert!(!a.same_as(&b));
        b.clear();
        assert!(!a.same_as(&b) && b.same_as(&DriveAa::default()));
    }

    #[test]
    fn a_late_drive_waits_for_sixteen_frames_before_a_notes_first_measure() {
        let (mut now, mut late) = (DriveLevel::new(), DriveLevel::new());
        for _ in 0..3 {
            now.add(0.5, 1.0);
            late.add(0.5, 1.0);
        }
        assert!((now.step(16, 44_100) - 0.5).abs() < 1e-6);
        // Until then, the gain that keeps a quiet sound's level.
        assert_eq!(late.step_late(16, 44_100, 4.0), 0.25);
        for _ in 0..16 {
            late.add(0.25, 1.0);
        }
        // Its first measure spans all 19 frames.
        let want = (1.75f32 / 19.0).sqrt();
        assert!((late.step_late(16, 44_100, 4.0) - want).abs() < 1e-6);
        // Later measures don't wait.
        late.add(0.5, 1.0);
        assert!(late.step_late(16, 44_100, 4.0) != want);
        // The quiet gain is the stage's slope at 0, mix level or not.
        for (g, amt) in [(1.0f32, 0.3f32), (4.0, 1.0), (16.0, 0.8)] {
            for lv in [0.2f32, 1.0, 2.0] {
                let (k, n) = soft_scales(g, lv);
                let x = 1e-4 * lv;
                let mut buf = vec![x; 64];
                drive_run_aa(&mut buf, k, n, amt, &mut DriveAa::default());
                let slope = buf[63] / x;
                let q = soft_quiet_gain(g, amt);
                assert!(
                    (slope - q).abs() < 1e-3 * q,
                    "{g} {amt} {lv}: {slope} against {q}"
                );
            }
        }
    }

    #[test]
    fn the_drives_input_power_is_summed_a_frame_and_a_half_late() {
        let (k, n) = soft_scales(8.0, 1.0);
        let x = signal(300);
        // In runs of every length from 1 up, as a voice's blocks cut them.
        let mut st = DriveAa::default();
        let mut level = DriveLevel::new();
        let mut buf = x.clone();
        let (mut at, mut run) = (0, 1);
        while at < buf.len() {
            let end = (at + run).min(buf.len());
            drive_run_level_aa(&mut buf[at..end], k, n, 1.0, &mut level, &mut st);
            at = end;
            run += 1;
        }
        let sq = |i: isize| {
            let v = x[i.max(0) as usize];
            f64::from(v * v)
        };
        let want: f64 = (0..x.len() as isize)
            .map(|i| 0.5 * (sq(i - 1) + sq(i - 2)))
            .sum();
        let got = f64::from(level.late_sum_in());
        assert!((got - want).abs() < 1e-5 * want, "{got} against {want}");
        assert_eq!(level.frames, 300);
        // And to the bit what one run over the whole gives.
        let (mut st1, mut one) = (DriveAa::default(), DriveLevel::new());
        let mut all = x.clone();
        drive_run_level_aa(&mut all, k, n, 1.0, &mut one, &mut st1);
        assert_eq!(all, buf);
        assert_eq!(one.late_sum_in().to_bits(), level.late_sum_in().to_bits());
    }
}
