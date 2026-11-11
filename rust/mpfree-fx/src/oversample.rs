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

#[cfg(test)]
mod tests {
    use super::*;

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
}
