//! 2x up- and down-sampling with linear-phase half-band FIR filters, for
//! effects that make new harmonics (drive): run the shaper at 2x or 4x so
//! harmonics above the base Nyquist are filtered out instead of folding
//! back into the audible band as harsh, inharmonic aliasing.
//!
//! The filters are windowed sinc (Kaiser) half-bands designed once on the
//! control thread. Every other tap of a half-band is zero, and the filters
//! skip them. Both directions together delay the signal by `taps - 1`
//! samples at the higher rate, that is `(taps - 1) / 2` base frames.

/// Taps of the base-rate stage (2x): about -90 dB stop band from 0.56 of
/// the base Nyquist up.
pub const TAPS_2X: usize = 95;
/// Taps of the second stage (2x to 4x), whose transition band is wide.
pub const TAPS_4X: usize = 31;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..60 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-14 * sum {
            break;
        }
    }
    sum
}

/// A half-band low-pass (cutoff at a quarter of its own rate), Kaiser
/// windowed, normalised to unity gain at DC. `taps` must be odd with
/// `(taps - 1) / 2` odd, so the zero taps fall on every odd index.
pub(crate) fn halfband(taps: usize, beta: f64) -> Vec<f32> {
    assert!(
        taps % 2 == 1 && ((taps - 1) / 2) % 2 == 1,
        "half-band needs taps = 4k + 3"
    );
    let c = (taps / 2) as f64;
    let i0b = bessel_i0(beta);
    let mut h: Vec<f64> = (0..taps)
        .map(|n| {
            let t = n as f64 - c;
            let sinc = if t == 0.0 {
                0.5
            } else {
                (std::f64::consts::PI * t * 0.5).sin() / (std::f64::consts::PI * t)
            };
            let r = t / c;
            sinc * bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / i0b
        })
        .collect();
    let sum: f64 = h.iter().sum();
    for v in &mut h {
        *v /= sum;
    }
    h.into_iter().map(|v| v as f32).collect()
}

/// A history ring read newest first, kept twice over so reads never wrap
/// and the newest `len` samples are one contiguous slice.
#[derive(Clone, Debug)]
struct Hist {
    buf: Vec<f32>,
    len: usize,
    pos: usize,
}

impl Hist {
    fn new(len: usize) -> Hist {
        Hist {
            buf: vec![0.0; 2 * len],
            len,
            pos: 0,
        }
    }
    #[inline]
    fn push(&mut self, x: f32) {
        self.pos = if self.pos == 0 {
            self.len - 1
        } else {
            self.pos - 1
        };
        self.buf[self.pos] = x;
        self.buf[self.pos + self.len] = x;
    }
    /// The sample pushed `k` pushes ago (0 = newest).
    #[inline]
    fn get(&self, k: usize) -> f32 {
        self.buf[self.pos + k]
    }
    /// The newest `len` samples, newest first.
    #[inline]
    fn newest(&self) -> &[f32] {
        &self.buf[self.pos..self.pos + self.len]
    }
    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// A dot product in eight independent sums, which the compiler can keep
/// in vector registers (NEON on the MPC). The order of the additions is
/// fixed, so the result never depends on the block size.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let whole = n / 8 * 8;
    let mut acc = [0.0f32; 8];
    for (x, y) in a[..whole].chunks_exact(8).zip(b[..whole].chunks_exact(8)) {
        for i in 0..8 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut s = ((acc[0] + acc[4]) + (acc[1] + acc[5])) + ((acc[2] + acc[6]) + (acc[3] + acc[7]));
    for i in whole..n {
        s += a[i] * b[i];
    }
    s
}

/// A half-band split into its two polyphase branches: the even taps
/// (contiguous, by input-history index) and the one odd tap, the centre.
fn polyphase(taps: usize) -> (Vec<f32>, f32, usize) {
    let h = halfband(taps, 8.0);
    let c = taps / 2;
    let even: Vec<f32> = h.iter().step_by(2).copied().collect();
    // The centre tap acts on the history sample (c - 1) / 2 back.
    (even, h[c], (c - 1) / 2)
}

/// 2x up-sampler for one channel.
#[derive(Clone, Debug)]
pub struct Up2 {
    even: Vec<f32>,
    center: f32,
    at: usize,
    hist: Hist,
}

impl Up2 {
    /// Allocates (control thread). `taps` is 4k + 3.
    pub fn new(taps: usize) -> Up2 {
        let (even, center, at) = polyphase(taps);
        // Zero-stuffed input: output 2n uses the even taps, 2n + 1 the
        // centre; the factor 2 restores the level lost to the stuffed zeros.
        let even = even.into_iter().map(|c| 2.0 * c).collect::<Vec<_>>();
        let n = even.len();
        Up2 {
            even,
            center: 2.0 * center,
            at,
            hist: Hist::new(n),
        }
    }

    /// One input sample in, two output samples out (in time order).
    #[inline]
    pub fn run(&mut self, x: f32) -> [f32; 2] {
        self.hist.push(x);
        [
            dot(&self.even, self.hist.newest()),
            self.center * self.hist.get(self.at),
        ]
    }

    pub fn reset(&mut self) {
        self.hist.clear();
    }
}

/// 2x down-sampler for one channel.
#[derive(Clone, Debug)]
pub struct Down2 {
    even: Vec<f32>,
    center: f32,
    at: usize,
    /// The even-phase inputs, and the odd-phase inputs for the centre tap.
    hist_e: Hist,
    hist_o: Hist,
}

impl Down2 {
    /// Allocates (control thread). `taps` is 4k + 3.
    pub fn new(taps: usize) -> Down2 {
        let (even, center, at) = polyphase(taps);
        let n = even.len();
        Down2 {
            even,
            center,
            at,
            hist_e: Hist::new(n),
            hist_o: Hist::new(at + 1),
        }
    }

    /// Two input samples in (in time order), one output sample out.
    #[inline]
    pub fn run(&mut self, x: [f32; 2]) -> f32 {
        // Keep the even phase, so up then down delays by a whole number of
        // base frames, `(taps - 1) / 2`. The centre tap reaches back to an
        // odd-phase sample pushed `at + 1` calls ago.
        self.hist_e.push(x[0]);
        let y = dot(&self.even, self.hist_e.newest()) + self.center * self.hist_o.get(self.at);
        self.hist_o.push(x[1]);
        y
    }

    pub fn reset(&mut self) {
        self.hist_e.clear();
        self.hist_o.clear();
    }
}

/// Coefficients of a polyphase IIR half-band low-pass: `n` first-order
/// all-pass sections, flat up to `0.25 - tbw` of the higher rate and
/// stopping from `0.25 + tbw`. It is the elliptic design of Valenzuela and
/// Constantinides, worked out in closed form (theta-function sums), once,
/// on the control thread. Even-indexed coefficients make one branch, odd
/// ones the other.
pub(crate) fn iir_halfband(n: usize, tbw: f64) -> Vec<f32> {
    use std::f64::consts::PI;
    let tbw = tbw.clamp(1e-4, 0.24);
    let k = ((1.0 - 2.0 * tbw) * PI / 4.0).tan().powi(2);
    let kk = (1.0 - k * k).sqrt().sqrt();
    let e = 0.5 * (1.0 - kk) / (1.0 + kk);
    let e4 = e.powi(4);
    let q = e * (1.0 + e4 * (2.0 + e4 * (15.0 + 150.0 * e4)));
    let order = (2 * n + 1) as f64;
    (1..=n)
        .map(|c| {
            let c = c as f64;
            // The numerator's and the denominator's theta-function sums.
            let mut num = 0.0;
            let mut sign = 1.0;
            for i in 0..64 {
                let i = i as f64;
                let t = q.powf(i * (i + 1.0)) * ((2.0 * i + 1.0) * c * PI / order).sin() * sign;
                num += t;
                sign = -sign;
                if t.abs() < 1e-100 {
                    break;
                }
            }
            let mut den = 0.0;
            let mut sign = -1.0;
            for i in 1..64 {
                let i = i as f64;
                let t = q.powf(i * i) * (2.0 * i * c * PI / order).cos() * sign;
                den += t;
                sign = -sign;
                if t.abs() < 1e-100 {
                    break;
                }
            }
            let ww = num * q.powf(0.25) / (den + 0.5);
            let w2 = ww * ww;
            let x = ((1.0 - w2 * k) * (1.0 - w2 / k)).sqrt() / (1.0 + w2);
            ((1.0 - x) / (1.0 + x)) as f32
        })
        .collect()
}

/// One first-order all-pass at the lower rate, `y = a (x - y1) + x1`.
#[derive(Clone, Copy, Debug, Default)]
struct Ap1 {
    a: f32,
    x1: f32,
    y1: f32,
}

impl Ap1 {
    #[inline(always)]
    fn run(&mut self, x: f32) -> f32 {
        let y = crate::flush(self.a * (x - self.y1) + self.x1);
        self.x1 = x;
        self.y1 = y;
        y
    }
}

/// The two branches of an IIR half-band: the even coefficients' sections
/// and the odd ones'.
#[derive(Clone, Debug)]
struct Branches {
    even: Vec<Ap1>,
    odd: Vec<Ap1>,
}

impl Branches {
    fn new(n: usize, tbw: f64) -> Branches {
        let c = iir_halfband(n, tbw);
        let ap = |a: &f32| Ap1 {
            a: *a,
            ..Ap1::default()
        };
        Branches {
            even: c.iter().step_by(2).map(ap).collect(),
            odd: c.iter().skip(1).step_by(2).map(ap).collect(),
        }
    }
    #[inline]
    fn run(&mut self, e: f32, o: f32) -> (f32, f32) {
        let (mut e, mut o) = (e, o);
        for s in &mut self.even {
            e = s.run(e);
        }
        for s in &mut self.odd {
            o = s.run(o);
        }
        (e, o)
    }
    fn reset(&mut self) {
        for s in self.even.iter_mut().chain(self.odd.iter_mut()) {
            s.x1 = 0.0;
            s.y1 = 0.0;
        }
    }
}

/// 2x up-sampler for one channel with an IIR half-band: a few multiplies
/// a frame instead of a FIR's dozens, for paths where phase does not
/// matter (a reverb's wet signal). Not linear phase.
#[derive(Clone, Debug)]
pub struct IirUp2(Branches);

impl IirUp2 {
    /// Allocates (control thread): `n` coefficients, transition `tbw`.
    pub fn new(n: usize, tbw: f64) -> IirUp2 {
        IirUp2(Branches::new(n, tbw))
    }

    /// One input sample in, two output samples out (in time order).
    #[inline]
    pub fn run(&mut self, x: f32) -> [f32; 2] {
        let (e, o) = self.0.run(x, x);
        [e, o]
    }

    pub fn reset(&mut self) {
        self.0.reset();
    }
}

/// 2x down-sampler for one channel with an IIR half-band (see [`IirUp2`]).
#[derive(Clone, Debug)]
pub struct IirDown2(Branches);

impl IirDown2 {
    /// Allocates (control thread): `n` coefficients, transition `tbw`.
    pub fn new(n: usize, tbw: f64) -> IirDown2 {
        IirDown2(Branches::new(n, tbw))
    }

    /// Two input samples in (in time order), one output sample out.
    #[inline]
    pub fn run(&mut self, x: [f32; 2]) -> f32 {
        let (e, o) = self.0.run(x[1], x[0]);
        0.5 * (e + o)
    }

    pub fn reset(&mut self) {
        self.0.reset();
    }
}

/// The pass-band phase of an [`IirUp2`] then [`IirDown2`] of the same
/// design, at the base rate. With a straight line between them the pair
/// is exactly one all-pass, the product of every section of both branches
/// (the up-sampler's even branch feeds the down-sampler's odd one and the
/// other way round), so a dry path through this stays in phase with a wet
/// path through the pair at every frequency, and a mix of the two never
/// combs. `n` sections a frame.
#[derive(Clone, Debug)]
pub struct IirPhase(Vec<Ap1>);

impl IirPhase {
    /// Allocates (control thread): the same `n` and `tbw` as the pair.
    pub fn new(n: usize, tbw: f64) -> IirPhase {
        IirPhase(
            iir_halfband(n, tbw)
                .iter()
                .map(|&a| Ap1 {
                    a,
                    ..Ap1::default()
                })
                .collect(),
        )
    }

    #[inline]
    pub fn run(&mut self, x: f32) -> f32 {
        let mut y = x;
        for s in &mut self.0 {
            y = s.run(y);
        }
        y
    }

    pub fn reset(&mut self) {
        for s in &mut self.0 {
            s.x1 = 0.0;
            s.y1 = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_phase_match_is_the_pair_with_a_straight_line_between() {
        let (mut up, mut down) = (IirUp2::new(5, 0.06), IirDown2::new(5, 0.06));
        let mut phase = IirPhase::new(5, 0.06);
        let mut seed = 3u32;
        for _ in 0..4000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = (seed >> 9) as f32 / (1u32 << 22) as f32 - 1.0;
            let [a, b] = up.run(x);
            let wet = down.run([a, b]);
            let dry = phase.run(x);
            assert!((wet - dry).abs() < 1e-5, "{wet} vs {dry}");
        }
    }

    #[test]
    fn halfband_shape() {
        let h = halfband(TAPS_2X, 8.0);
        let c = TAPS_2X / 2;
        assert!((h[c] - 0.5).abs() < 1e-3);
        for (n, &v) in h.iter().enumerate() {
            if n != c && (n as isize - c as isize) % 2 == 0 {
                assert!(v.abs() < 1e-9, "tap {n} = {v}");
            }
        }
        assert!((h.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn up_then_down_is_a_pure_delay_at_low_frequency() {
        let mut u = Up2::new(TAPS_2X);
        let mut d = Down2::new(TAPS_2X);
        let lat = (TAPS_2X - 1) / 2;
        let rate = 48_000.0f32;
        let x: Vec<f32> = (0..4000)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate).sin())
            .collect();
        let y: Vec<f32> = x.iter().map(|&s| d.run(u.run(s))).collect();
        for i in 500..3000 {
            assert!(
                (y[i + lat] - x[i]).abs() < 1e-3,
                "frame {i}: {} vs {}",
                y[i + lat],
                x[i]
            );
        }
    }

    /// The magnitude of the IIR half-band's response at `f` (a fraction of
    /// the higher rate), from its two branches.
    fn iir_gain(c: &[f32], f: f64) -> f64 {
        use std::f64::consts::PI;
        // A first-order all-pass in z^2: (a + z^-2) / (1 + a z^-2).
        let w = 2.0 * PI * f;
        let ap = |a: f64| {
            let (cr, ci) = ((2.0 * w).cos(), -(2.0 * w).sin());
            let (nr, ni) = (a + cr, ci);
            let (dr, di) = (1.0 + a * cr, a * ci);
            let d = dr * dr + di * di;
            ((nr * dr + ni * di) / d, (ni * dr - nr * di) / d)
        };
        let branch = |start: usize| {
            c.iter()
                .skip(start)
                .step_by(2)
                .fold((1.0, 0.0), |(r, i), &a| {
                    let (br, bi) = ap(a as f64);
                    (r * br - i * bi, r * bi + i * br)
                })
        };
        let (er, ei) = branch(0);
        let (or, oi) = branch(1);
        // The odd branch is a frame later: times z^-1.
        let (zr, zi) = (w.cos(), -w.sin());
        let (or, oi) = (or * zr - oi * zi, or * zi + oi * zr);
        0.5 * ((er + or).powi(2) + (ei + oi).powi(2)).sqrt()
    }

    #[test]
    fn iir_halfband_is_flat_then_stops() {
        let c = iir_halfband(6, 0.06);
        assert_eq!(c.len(), 6);
        assert!(c.iter().all(|&a| a > 0.0 && a < 1.0), "{c:?}");
        for k in 0..=190 {
            let g = 20.0 * iir_gain(&c, k as f64 / 1000.0).log10();
            assert!(g.abs() < 0.01, "{k}/1000: {g:.4} dB");
        }
        for k in 310..=500 {
            let g = 20.0 * iir_gain(&c, k as f64 / 1000.0).log10();
            assert!(g < -80.0, "{k}/1000: {g:.1} dB");
        }
    }

    #[test]
    fn iir_up_then_down_keeps_a_low_sine_and_its_level() {
        let mut u = IirUp2::new(6, 0.06);
        let mut d = IirDown2::new(6, 0.06);
        let rate = 24_000.0f32;
        let f = 500.0;
        let x: Vec<f32> = (0..4000)
            .map(|i| (2.0 * std::f32::consts::PI * f * i as f32 / rate).sin())
            .collect();
        let y: Vec<f32> = x.iter().map(|&s| d.run(u.run(s))).collect();
        // A sine comes out a sine of the same level, a little later: the
        // best fit of the output against the input's sine and cosine.
        let (mut ps, mut pc, mut e) = (0.0f64, 0.0f64, 0.0f64);
        for (i, &v) in y.iter().enumerate().skip(1000) {
            let t = 2.0 * std::f64::consts::PI * f as f64 * i as f64 / rate as f64;
            ps += v as f64 * t.sin();
            pc += v as f64 * t.cos();
            e += (v as f64).powi(2);
        }
        let n = (y.len() - 1000) as f64;
        let amp = 2.0 * (ps * ps + pc * pc).sqrt() / n;
        let rest = e / n - amp * amp / 2.0;
        assert!((amp - 1.0).abs() < 1e-3, "level {amp}");
        assert!(rest < 1e-6, "left over {rest}");
    }

    #[test]
    fn iir_resamplers_go_back_to_silence() {
        let mut u = IirUp2::new(6, 0.06);
        let mut d = IirDown2::new(6, 0.06);
        let mut last = 0;
        for i in 0..400 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            let [a, b] = u.run(x);
            if a.abs().max(b.abs()) > 1e-5 {
                last = i;
            }
            let _ = d.run([a, b]);
        }
        assert!(last < 128, "the up-sampler rings to {last}");
        u.reset();
        d.reset();
        assert_eq!(u.run(0.0), [0.0, 0.0]);
        assert_eq!(d.run([0.0, 0.0]), 0.0);
    }
}
