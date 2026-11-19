//! Band-limited classic waves read from tables: the voice core's
//! oscillators (part 2 of the shared voice core, `docs/dev/design/
//! synth-architecture.md`).
//!
//! Each wave is kept at half-octave steps of bandwidth, a level per step
//! as Wavetable's tables are, so a note reads only harmonics below
//! Nyquist and nothing folds back, at any pitch. A level is read linearly
//! between samples, with at least eight samples per cycle of its top
//! harmonic (and more for the few harmonics of high notes).
//!
//! - **Saw, triangle and sine** are tables.
//! - **The pulse** is two saws a width apart, so the width can move with
//!   no table per width and no DC.
//! - **The supersaw** is seven saws on the detune and mix curves Adam Szabo
//!   measured on the original ("How to emulate the super saw", 2010), its
//!   level kept near one saw's whatever the mix.
//!
//! The tables are built once, on the first [`classic`] call: about 340 KB,
//! and it allocates, so an engine calls it from its `new`, off the audio
//! thread. Reading never allocates, locks or panics.

use std::f64::consts::PI;
use std::sync::OnceLock;

use crate::wavetable::fft::fft;

/// Levels per wave, half an octave apart.
pub const LEVELS: usize = 17;
/// Harmonics each level keeps (as Wavetable's levels).
const LEVEL_HARMONICS: [u16; LEVELS] = [
    512, 362, 256, 181, 128, 90, 64, 45, 32, 22, 16, 11, 8, 5, 4, 2, 1,
];
/// The sine's table length: linear reading stays under -100 dB.
const SINE_LEN: usize = 2048;

/// [`level_for`], walking from level `lv` (the one used last): a pitch
/// that moves a little, as with vibrato, costs a compare or two.
#[inline]
pub fn level_near(lv: usize, dt: f32) -> usize {
    let dt = dt.abs();
    let mut i = lv.min(LEVELS - 1);
    while i > 0 && f32::from(LEVEL_HARMONICS[i - 1]) * dt <= 0.5 {
        i -= 1;
    }
    while i + 1 < LEVELS && f32::from(LEVEL_HARMONICS[i]) * dt > 0.5 {
        i += 1;
    }
    i
}

/// A level's length: eight samples per cycle of its top harmonic at
/// least, and never under 512, so the few loud harmonics a high note keeps
/// get 16 or more (linear reading's images of them stay under -60 dB).
fn level_len(harmonics: usize) -> usize {
    (harmonics * 8).next_power_of_two().clamp(512, 4096)
}

/// The level for a phase increment `dt` (cycles a sample): the one with
/// the most harmonics that all stay below Nyquist.
#[inline]
pub fn level_for(dt: f32) -> usize {
    let dt = dt.abs();
    let mut i = 0;
    while i + 1 < LEVELS && f32::from(LEVEL_HARMONICS[i]) * dt > 0.5 {
        i += 1;
    }
    i
}

/// One wave at every level, end to end. Each sample is kept with the step
/// to the next one (the last's step is back to the first), scaled to the
/// phase bits below the index, so a read is one load and one multiply-add,
/// and never wraps.
pub struct Waveset {
    data: Box<[[f32; 2]]>,
    off: [usize; LEVELS],
    len: [usize; LEVELS],
    /// How far a phase shifts down to index the level (32 - log2 len).
    shift: [u32; LEVELS],
}

impl Waveset {
    /// From each harmonic's sine and cosine amplitudes (`amp(k)`, k from 1).
    pub(crate) fn build(amp: impl Fn(usize) -> (f64, f64)) -> Waveset {
        let mut data = Vec::new();
        let (mut off, mut lens) = ([0; LEVELS], [0; LEVELS]);
        for (i, &h) in LEVEL_HARMONICS.iter().enumerate() {
            let len = level_len(usize::from(h));
            let (mut re, mut im) = (vec![0.0f64; len], vec![0.0f64; len]);
            // x[n] = sum c cos + s sin = sum X[k] e^{+i 2 pi k n / len} with
            // X[k] = (c - i s) / 2 and its mirror the conjugate.
            for k in 1..=usize::from(h).min(len / 2 - 1) {
                let (s, c) = amp(k);
                re[k] = c / 2.0;
                im[k] = -s / 2.0;
                re[len - k] = c / 2.0;
                im[len - k] = s / 2.0;
            }
            fft(&mut re, &mut im, true);
            off[i] = data.len();
            lens[i] = len;
            let per = len as f64 / ONE;
            data.extend((0..len).map(|n| {
                let (a, b) = (re[n], re[(n + 1) % len]);
                [a as f32, ((b - a) * per) as f32]
            }));
        }
        Waveset {
            data: data.into_boxed_slice(),
            off,
            len: lens,
            shift: lens.map(|n| 32 - n.trailing_zeros()),
        }
    }

    /// Level `lv`: its samples and its [`Lv`] for reading.
    #[inline]
    fn level(&self, lv: usize) -> (&[[f32; 2]], Lv) {
        let lv = lv.min(LEVELS - 1);
        let (o, n) = (self.off[lv], self.len[lv]);
        (&self.data[o..o + n], Lv::new(self.shift[lv]))
    }

    /// One sample of level `lv` at phase `p` ([`phase`]).
    pub fn at(&self, lv: usize, p: u32) -> f32 {
        let (d, l) = self.level(lv);
        l.read(d, p)
    }

    /// Level `lv`, ready to read many times ([`Read::at`]): for a caller
    /// that stays on one level for a while, as an FM operator does across
    /// a control step.
    #[inline]
    pub fn reader(&self, lv: usize) -> Read<'_> {
        let (d, lv) = self.level(lv);
        Read { d, lv }
    }

    pub(crate) fn bytes(&self) -> usize {
        self.data.len() * 8
    }
}

/// One level of a [`Waveset`] ([`Waveset::reader`]).
#[derive(Clone, Copy)]
pub struct Read<'a> {
    d: &'a [[f32; 2]],
    lv: Lv,
}

impl Read<'_> {
    /// One sample at phase `p` ([`phase`]): the same load and multiply-add
    /// as [`Waveset::at`].
    #[inline]
    pub fn at(self, p: u32) -> f32 {
        self.lv.read(self.d, p)
    }
}

/// A phase as a 32-bit fraction of a cycle: it wraps by itself, and its top
/// bits index a table with no float-to-integer conversion.
pub const ONE: f64 = 4_294_967_296.0;

/// A phase (0 to 1 of a cycle) as the oscillators keep it.
#[inline]
pub fn phase(p: f32) -> u32 {
    (f64::from(p.rem_euclid(1.0)) * ONE) as u32
}

/// A phase increment (cycles a sample, 0 to under one) as the oscillators
/// keep it. NaN and anything below 0 give 0.
#[inline]
pub fn increment(dt: f32) -> u32 {
    (dt * 4_294_967_296.0) as u32
}

/// How a level is read: the shift that gives the index, and the mask for
/// the bits below it (the steps are kept scaled to them).
#[derive(Clone, Copy)]
struct Lv {
    shift: u32,
    mask: u32,
}

impl Lv {
    fn new(shift: u32) -> Lv {
        Lv {
            shift,
            mask: (1u32 << shift) - 1,
        }
    }

    /// How far a phase shifts down to index the level.
    pub(crate) fn shift(self) -> u32 {
        self.shift
    }

    /// Linear between the samples either side of phase `p`.
    #[inline]
    fn read(self, d: &[[f32; 2]], p: u32) -> f32 {
        let [a, step] = d[(p >> self.shift) as usize];
        a + step * (p & self.mask) as f32
    }
}

/// The classic waves, in the order Poly's Wave menu lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Saw,
    Pulse,
    Triangle,
    Sine,
    Supersaw,
}

impl Shape {
    pub fn from_index(i: f32) -> Shape {
        match i as i32 {
            1 => Shape::Pulse,
            2 => Shape::Triangle,
            3 => Shape::Sine,
            4 => Shape::Supersaw,
            _ => Shape::Saw,
        }
    }
}

/// The saw, triangle and sine tables.
pub struct Classic {
    pub saw: Waveset,
    pub triangle: Waveset,
    sine: Box<[[f32; 2]]>,
}

/// The tables, built on the first call (allocates; call it off the audio
/// thread first, from an engine's `new`).
pub fn classic() -> &'static Classic {
    static TABLES: OnceLock<Classic> = OnceLock::new();
    TABLES.get_or_init(|| {
        // 2p - 1 = -(2 / pi) sum sin(2 pi k p) / k: a rising ramp that
        // drops at the cycle's start, as dsp's saw.
        let saw = Waveset::build(|k| (-2.0 / (PI * k as f64), 0.0));
        // 1 - 4 |p - 1/2| = -(8 / pi^2) sum over odd k of cos(2 pi k p) / k^2.
        let triangle = Waveset::build(|k| {
            if k % 2 == 1 {
                (0.0, -8.0 / (PI * PI * (k * k) as f64))
            } else {
                (0.0, 0.0)
            }
        });
        let at = |n: usize| (2.0 * PI * (n % SINE_LEN) as f64 / SINE_LEN as f64).sin();
        let per = SINE_LEN as f64 / ONE;
        let sine: Vec<[f32; 2]> = (0..SINE_LEN)
            .map(|n| [at(n) as f32, ((at(n + 1) - at(n)) * per) as f32])
            .collect();
        Classic {
            saw,
            triangle,
            sine: sine.into_boxed_slice(),
        }
    })
}

impl Classic {
    /// Bytes the tables hold.
    pub fn bytes(&self) -> usize {
        self.saw.bytes() + self.triangle.bytes() + self.sine.len() * 8
    }

    /// One sample of the sine at phase `p` ([`phase`]).
    pub fn sine_at(&self, p: u32) -> f32 {
        sine_lv().read(&self.sine, p)
    }

    /// Writes (`ADD` false: `0 + x * level`) or adds (`x * level`) `shape`
    /// into each frame of `buf`, from phase `t` with increment `dt` (both
    /// [`phase`]s) on level `lv` ([`level_for`]), advancing the phase. `pw`
    /// is the pulse's width (0 to 1). The supersaw is
    /// [`Classic::supersaw`]; here it plays one saw.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    pub fn run<const ADD: bool>(
        &self,
        shape: Shape,
        lv: usize,
        buf: &mut [f32],
        t: &mut u32,
        dt: u32,
        pw: f32,
        level: f32,
    ) {
        let mut p = *t;
        let put = |x: &mut f32, y: f32| *x = if ADD { *x } else { 0.0 } + y * level;
        match shape {
            Shape::Saw | Shape::Supersaw => {
                let (d, l) = self.saw.level(lv);
                for x in buf.iter_mut() {
                    put(x, l.read(d, p));
                    p = p.wrapping_add(dt);
                }
            }
            Shape::Pulse => {
                // saw(p - pw) - saw(p): 2 - 2 pw before the width, -2 pw
                // after, as dsp's pulse with its DC taken out.
                let (d, l) = self.saw.level(lv);
                let back = phase(1.0 - pw);
                for x in buf.iter_mut() {
                    put(x, l.read(d, p.wrapping_add(back)) - l.read(d, p));
                    p = p.wrapping_add(dt);
                }
            }
            Shape::Triangle => {
                let (d, l) = self.triangle.level(lv);
                for x in buf.iter_mut() {
                    put(x, l.read(d, p));
                    p = p.wrapping_add(dt);
                }
            }
            Shape::Sine => {
                let l = sine_lv();
                for x in buf.iter_mut() {
                    put(x, l.read(&self.sine, p));
                    p = p.wrapping_add(dt);
                }
            }
        }
        *t = p;
    }

    /// The supersaw: [`SAWS`] saws, each at `dt` times its ratio from
    /// `sup`, with `sup`'s gains, written or added (`ADD`) into `buf` times
    /// `level`, on level `lv` (for the highest saw:
    /// `level_for(dt * sup.top())`). Saw by saw, so each frame sums them in
    /// the same order whatever the block size.
    ///
    /// The saws run in `mpfree_simd::table_sum`: a NEON kernel on the MPC
    /// (ARMv7 with NEON), its plain Rust twin elsewhere, the same samples
    /// either way (`docs/dev/design/simd.md`). Kept out of line: it is one
    /// call a voice a control step, and Poly's render loop without the saws
    /// inside it costs 1 to 2 % less on the x86 instruction gate.
    #[inline(never)]
    pub fn supersaw<const ADD: bool>(
        &self,
        sup: &Super,
        lv: usize,
        buf: &mut [f32],
        ph: &mut [u32; SAWS],
        dt: f32,
        level: f32,
    ) {
        let (d, l) = self.saw.level(lv);
        let inc: [u32; SAWS] = std::array::from_fn(|k| increment(dt * sup.ratio[k]));
        let gain: [f32; SAWS] = std::array::from_fn(|k| sup.gain[k] * level);
        mpfree_simd::table_sum::<ADD>(d, l.shift(), buf, ph, &inc, &gain);
    }
}

/// How the sine's table is read.
#[inline]
fn sine_lv() -> Lv {
    Lv::new(32 - SINE_LEN.trailing_zeros())
}

/// Saws in the supersaw.
pub const SAWS: usize = 7;

/// Each saw's offset from the note at full detune, as a fraction of its
/// frequency (Szabo's measurements, lowest to highest; the fourth is the
/// note).
const OFFSETS: [f64; SAWS] = [
    -0.110_023_13,
    -0.062_884_39,
    -0.019_523_56,
    0.0,
    0.019_912_21,
    0.062_165_38,
    0.107_452_42,
];

/// The detune knob (0 to 1) to the share of [`OFFSETS`] used: slow at
/// first, then steep, as on the original (Szabo's fit, an 11th-order
/// polynomial).
pub fn detune_curve(x: f64) -> f64 {
    const C: [f64; 12] = [
        10_028.731_289_163_4,
        -50_818.865_204_592_4,
        111_363.480_872_936_8,
        -138_150.676_108_054_8,
        106_649.667_915_829_2,
        -53_046.964_275_187_5,
        17_019.951_858_008_0,
        -3_425.083_659_131_8,
        404.270_393_838_8,
        -24.187_882_439_1,
        0.671_741_763_4,
        0.003_011_559_6,
    ];
    let x = x.clamp(0.0, 1.0);
    C.iter().fold(0.0, |acc, &c| acc * x + c)
}

/// The supersaw's ratios and gains for a detune and a mix (each 0 to 1).
/// Worked out when either knob moves, not while playing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Super {
    pub ratio: [f32; SAWS],
    pub gain: [f32; SAWS],
}

impl Super {
    pub fn new(detune: f32, mix: f32) -> Super {
        let d = detune_curve(f64::from(detune)).max(0.0);
        let m = f64::from(mix.clamp(0.0, 1.0));
        // Szabo's mix curves: the middle saw falls as the six around it rise.
        let mid = -0.553_66 * m + 0.997_85;
        let side = -0.737_64 * m * m + 1.284_1 * m + 0.044_372;
        // Uncorrelated saws add as power: keep the sum near one saw.
        let norm = 1.0 / (mid * mid + 6.0 * side * side).sqrt();
        let mut s = Super {
            ratio: [1.0; SAWS],
            gain: [0.0; SAWS],
        };
        for k in 0..SAWS {
            s.ratio[k] = (1.0 + OFFSETS[k] * d) as f32;
            s.gain[k] = ((if k == 3 { mid } else { side }) * norm) as f32;
        }
        s
    }

    /// The highest saw's ratio, for [`level_for`].
    pub fn top(&self) -> f32 {
        self.ratio.iter().fold(1.0f32, |m, &r| m.max(r))
    }
}

impl Default for Super {
    fn default() -> Self {
        Super::new(0.5, 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Amplitude of harmonic `k` in a one-cycle table (DFT bin `k`).
    fn bin(x: &[f32], k: usize) -> f64 {
        let n = x.len();
        let (mut re, mut im) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            let a = 2.0 * PI * (k * i) as f64 / n as f64;
            re += f64::from(v) * a.cos();
            im += f64::from(v) * a.sin();
        }
        (re * re + im * im).sqrt() * 2.0 / n as f64
    }

    #[test]
    fn each_level_holds_only_its_harmonics() {
        let c = classic();
        for lv in [0, 4, 9, 12, 16] {
            let (d, _) = c.saw.level(lv);
            let x: Vec<f32> = d.iter().map(|s| s[0]).collect();
            let h = usize::from(LEVEL_HARMONICS[lv]);
            for k in [1, h, h + 1, h + 2] {
                let a = bin(&x, k);
                if k <= h {
                    let want = 2.0 / (PI * k as f64);
                    assert!((a - want).abs() < 1e-4, "level {lv} k {k}: {a} vs {want}");
                } else if k < x.len() / 2 {
                    assert!(a < 1e-6, "level {lv} k {k}: {a}");
                }
            }
            // Each step reaches the next sample; the last reaches the first.
            let (n, span) = (d.len(), (ONE / d.len() as f64) as f32);
            assert!((d[n - 1][0] + d[n - 1][1] * span - d[0][0]).abs() < 1e-5);
            assert!((d[3][0] + d[3][1] * span - d[4][0]).abs() < 1e-5);
        }
    }

    #[test]
    fn the_level_keeps_every_harmonic_under_nyquist() {
        for note in 0..128 {
            let hz = 440.0 * 2f32.powf((note as f32 - 69.0) / 12.0);
            for rate in [44_100.0, 48_000.0] {
                let dt = hz / rate;
                let lv = level_for(dt);
                let top = f32::from(LEVEL_HARMONICS[lv]) * hz;
                assert!(top <= rate / 2.0 || lv == LEVELS - 1, "{note}: {top}");
                // And it keeps all it can: the level above would fold.
                if lv > 0 {
                    assert!(f32::from(LEVEL_HARMONICS[lv - 1]) * hz > rate / 2.0);
                }
            }
        }
    }

    #[test]
    fn walking_to_a_level_finds_the_same_one() {
        let mut lv = 0;
        for i in 0..2000 {
            let dt = 0.45 * (0.997f32).powi(i);
            let near = level_near(lv, dt);
            assert_eq!(near, level_for(dt), "{dt}");
            assert_eq!(level_near(LEVELS - 1, dt), near);
            assert_eq!(level_near(0, dt), near);
            lv = near;
        }
    }

    #[test]
    fn phases_wrap_as_cycles() {
        assert_eq!(phase(0.0), 0);
        assert_eq!(phase(0.5), 1 << 31);
        assert_eq!(phase(1.25), 1 << 30);
        assert_eq!(phase(-0.25), 3 << 30);
        assert_eq!(increment(0.25), 1 << 30);
        assert_eq!(increment(f32::NAN), 0);
    }

    #[test]
    fn a_reader_reads_what_at_reads() {
        let c = classic();
        for lv in 0..LEVELS {
            let r = c.saw.reader(lv);
            for k in 0..1000u32 {
                let p = k.wrapping_mul(0x9E37_79B9);
                assert_eq!(r.at(p).to_bits(), c.saw.at(lv, p).to_bits());
            }
        }
    }

    #[test]
    fn the_shapes_match_the_plain_waves_away_from_their_steps() {
        let c = classic();
        let dt = 110.0 / 48_000.0;
        let lv = level_for(dt);
        for (shape, want) in [
            (
                Shape::Saw,
                Box::new(|p: f32| 2.0 * p - 1.0) as Box<dyn Fn(f32) -> f32>,
            ),
            (
                Shape::Triangle,
                Box::new(|p: f32| 1.0 - 4.0 * (p - 0.5).abs()),
            ),
            (
                Shape::Sine,
                Box::new(|p: f32| (std::f32::consts::TAU * p).sin()),
            ),
            (
                Shape::Pulse,
                Box::new(|p: f32| if p < 0.3 { 2.0 - 0.6 } else { -0.6 }),
            ),
        ] {
            let mut buf = [0.0f32; 436];
            let mut t = 0u32;
            c.run::<false>(shape, lv, &mut buf, &mut t, increment(dt), 0.3, 1.0);
            for (i, &y) in buf.iter().enumerate() {
                let p = (i as f32 * dt).fract();
                // Away from the saw's and the pulse's steps.
                let near = |e: f32| (p - e).abs() < 0.03 || (p - e - 1.0).abs() < 0.03;
                if near(0.0) || near(1.0) || (shape == Shape::Pulse && near(0.3)) {
                    continue;
                }
                let err = (y - want(p)).abs();
                let tol = if shape == Shape::Sine { 1e-4 } else { 0.05 };
                assert!(err < tol, "{shape:?} at {p}: {y} vs {}", want(p));
            }
        }
    }

    #[test]
    fn the_pulse_has_no_dc_at_any_width() {
        let c = classic();
        let dt = 1.0 / 480.0;
        for pw in [0.05, 0.3, 0.5, 0.9] {
            let mut buf = [0.0f32; 4800];
            let mut t = 0u32;
            let lv = level_for(dt);
            c.run::<false>(Shape::Pulse, lv, &mut buf, &mut t, increment(dt), pw, 1.0);
            let mean = buf.iter().map(|&x| f64::from(x)).sum::<f64>() / buf.len() as f64;
            assert!(mean.abs() < 1e-3, "{pw}: {mean}");
        }
    }

    #[test]
    fn any_block_size_gives_the_same_samples() {
        let c = classic();
        let sup = Super::new(0.7, 0.6);
        let dt = 523.0 / 44_100.0;
        let start = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7].map(phase);
        let whole = {
            let mut out = vec![0.0f32; 300];
            let (mut t, mut ph) = (phase(0.25), start);
            c.run::<false>(Shape::Pulse, 6, &mut out, &mut t, increment(dt), 0.4, 0.8);
            c.supersaw::<true>(&sup, 6, &mut out, &mut ph, dt, 0.5);
            out
        };
        let mut cut = vec![0.0f32; 300];
        let (mut t, mut ph) = (phase(0.25), start);
        let mut at = 0;
        for n in [1, 7, 16, 64, 100, 112] {
            let b = &mut cut[at..at + n];
            c.run::<false>(Shape::Pulse, 6, b, &mut t, increment(dt), 0.4, 0.8);
            c.supersaw::<true>(&sup, 6, b, &mut ph, dt, 0.5);
            at += n;
        }
        assert_eq!(whole, cut);
    }

    #[test]
    fn the_detune_curve_rises_from_nothing_to_all() {
        assert!(detune_curve(0.0).abs() < 0.01);
        assert!(
            (detune_curve(1.0) - 1.0).abs() < 0.02,
            "{}",
            detune_curve(1.0)
        );
        let mut last = detune_curve(0.0);
        for i in 1..=100 {
            let d = detune_curve(f64::from(i) / 100.0);
            assert!(d >= last - 1e-3, "{i}: {d} after {last}");
            last = d;
        }
        // Slow at first: half the knob gives well under half the detune.
        assert!(detune_curve(0.5) < 0.3, "{}", detune_curve(0.5));
    }

    #[test]
    fn the_supersaw_stays_near_one_saws_level() {
        let c = classic();
        let dt = 220.0 / 48_000.0;
        let rms = |buf: &[f32]| {
            (buf.iter().map(|&x| f64::from(x).powi(2)).sum::<f64>() / buf.len() as f64).sqrt()
        };
        let mut one = vec![0.0f32; 48_000];
        let mut t = 0u32;
        c.run::<false>(
            Shape::Saw,
            level_for(dt),
            &mut one,
            &mut t,
            increment(dt),
            0.5,
            1.0,
        );
        let saw = rms(&one);
        for (detune, mix) in [(0.2, 0.0), (0.5, 0.5), (0.9, 1.0)] {
            let sup = Super::new(detune, mix);
            let mut buf = vec![0.0f32; 48_000];
            let mut ph = [0.0, 0.31, 0.77, 0.5, 0.13, 0.59, 0.91].map(phase);
            c.supersaw::<false>(&sup, level_for(dt * sup.top()), &mut buf, &mut ph, dt, 1.0);
            let db = 20.0 * (rms(&buf) / saw).log10();
            assert!(db.abs() < 2.0, "{detune} {mix}: {db} dB");
        }
    }

    /// The supersaw through `mpfree_simd` gives the samples of the loop it
    /// had before, saw by saw (on the workstation the twin; under qemu-arm
    /// the kernel matches the twin bit for bit, `mpfree-simd`'s tests).
    #[test]
    fn the_supersaw_gives_the_old_loops_samples() {
        let c = classic();
        let sup = Super::new(0.63, 0.41);
        for (dt, add) in [(65.0 / 44_100.0, false), (1046.5 / 48_000.0, true)] {
            let lv = level_for(dt * sup.top());
            let (d, l) = c.saw.level(lv);
            let fill: Vec<f32> = (0..53).map(|i| (i as f32 * 0.37).sin()).collect();
            let start = [0.9, 0.1, 0.45, 0.0, 0.77, 0.3, 0.62].map(phase);
            let mut want = fill.clone();
            let mut pw = start;
            for (k, p) in pw.iter_mut().enumerate() {
                let (inc, g) = (increment(dt * sup.ratio[k]), sup.gain[k] * 0.8);
                for x in want.iter_mut() {
                    *x = if k == 0 && !add { 0.0 } else { *x } + l.read(d, *p) * g;
                    *p = p.wrapping_add(inc);
                }
            }
            let mut got = fill;
            let mut pg = start;
            if add {
                c.supersaw::<true>(&sup, lv, &mut got, &mut pg, dt, 0.8);
            } else {
                c.supersaw::<false>(&sup, lv, &mut got, &mut pg, dt, 0.8);
            }
            let bits = |x: &[f32]| x.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
            assert_eq!(bits(&want), bits(&got), "add {add}");
            assert_eq!(pw, pg);
        }
    }

    #[test]
    fn the_tables_are_small() {
        let b = classic().bytes();
        assert!(b < 400_000, "{b}");
    }
}
