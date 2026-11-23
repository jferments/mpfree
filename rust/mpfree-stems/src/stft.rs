//! The STFT pair Spleeter uses: 4096-point periodic Hann windows a hop of
//! 1024 apart, after 4096 leading zeros, with TensorFlow's framing (the end
//! padded with zeros to whole frames), and its inverse with the 2/3 window
//! compensation. Each 4096-point transform of a real frame is one
//! 2048-point complex FFT and a split, half a full complex FFT's work.

/// Points a frame.
pub(crate) const N: usize = 4096;
/// Samples between frames.
pub(crate) const HOP: usize = 1024;
/// Bins a frame: 0 to N/2.
pub(crate) const BINS: usize = N / 2 + 1;

/// STFT frames for `samples` samples of one channel.
pub(crate) fn frames(samples: usize) -> usize {
    (samples + N).div_ceil(HOP)
}

/// The periodic Hann window.
pub(crate) fn hann() -> Vec<f32> {
    (0..N)
        .map(|i| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N as f64).cos()) as f32)
        .collect()
}

/// N-point transforms of real signals.
pub(crate) struct Rfft {
    m: usize,
    rev: Vec<u32>,
    /// Stage twiddles, cos and sin of −2πk/len for k < len/2, the stages
    /// one after another, so a stage reads its table in order.
    tc: Vec<f32>,
    ts: Vec<f32>,
    /// The split's W^k = e^{−2πik/N}, k ≤ M.
    wc: Vec<f32>,
    ws: Vec<f32>,
}

impl Rfft {
    pub fn new() -> Rfft {
        let m = N / 2;
        let bits = m.trailing_zeros();
        let rev = (0..m as u32)
            .map(|i| i.reverse_bits() >> (32 - bits))
            .collect();
        let (mut tc, mut ts) = (Vec::new(), Vec::new());
        let mut len = 2;
        while len <= m {
            for k in 0..len / 2 {
                let a = -2.0 * std::f64::consts::PI * k as f64 / len as f64;
                tc.push(a.cos() as f32);
                ts.push(a.sin() as f32);
            }
            len *= 2;
        }
        let w = |k: usize| -2.0 * std::f64::consts::PI * k as f64 / N as f64;
        let wc = (0..=m).map(|k| w(k).cos() as f32).collect();
        let ws = (0..=m).map(|k| w(k).sin() as f32).collect();
        Rfft {
            m,
            rev,
            tc,
            ts,
            wc,
            ws,
        }
    }

    /// The M-point complex FFT in place, unnormalised, forward.
    fn complex(&self, re: &mut [f32], im: &mut [f32]) {
        let m = self.m;
        for i in 0..m {
            let j = self.rev[i] as usize;
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let mut len = 2;
        let mut t0 = 0;
        while len <= m {
            let half = len / 2;
            let (tc, ts) = (&self.tc[t0..t0 + half], &self.ts[t0..t0 + half]);
            for start in (0..m).step_by(len) {
                let (lr, hr) = re[start..start + len].split_at_mut(half);
                let (li, hi) = im[start..start + len].split_at_mut(half);
                for k in 0..half {
                    let (c, s) = (tc[k], ts[k]);
                    let tr = hr[k] * c - hi[k] * s;
                    let ti = hr[k] * s + hi[k] * c;
                    hr[k] = lr[k] - tr;
                    hi[k] = li[k] - ti;
                    lr[k] += tr;
                    li[k] += ti;
                }
            }
            t0 += half;
            len *= 2;
        }
    }

    /// X[k], k ≤ N/2, of real `x` (N samples).
    pub fn forward(&self, x: &[f32], re: &mut [f32], im: &mut [f32], w: &mut Frame) {
        let m = self.m;
        assert!(x.len() == N && re.len() == m + 1 && im.len() == m + 1);
        let (zr, zi) = (&mut w.zr, &mut w.zi);
        for i in 0..m {
            zr[i] = x[2 * i];
            zi[i] = x[2 * i + 1];
        }
        self.complex(zr, zi);
        // E = (Z[k] + conj Z[M−k]) / 2, O = (Z[k] − conj Z[M−k]) / 2i,
        // X[k] = E + W^k O.
        for k in 0..=m {
            let (ar, ai) = (zr[k % m], zi[k % m]);
            let (br, bi) = (zr[(m - k) % m], -zi[(m - k) % m]);
            let (er, ei) = (0.5 * (ar + br), 0.5 * (ai + bi));
            let (or, oi) = (0.5 * (ai - bi), -0.5 * (ar - br));
            let (c, s) = (self.wc[k], self.ws[k]);
            re[k] = er + (or * c - oi * s);
            im[k] = ei + (or * s + oi * c);
        }
    }

    /// The unnormalised inverse (N · x) of a real signal's X[k], k ≤ N/2,
    /// into `w.out`, ignoring the imaginary parts of X[0] and X[N/2] as an
    /// inverse real FFT does.
    pub fn inverse(&self, re: &[f32], im: &[f32], w: &mut Frame) {
        let m = self.m;
        assert!(re.len() == m + 1 && im.len() == m + 1);
        let at = |k: usize| -> (f32, f32) {
            if k == 0 || k == m {
                (re[k], 0.0)
            } else {
                (re[k], im[k])
            }
        };
        // E' = X[k] + conj X[M−k], O' = (X[k] − conj X[M−k]) · conj W^k,
        // Z' = E' + i O' (twice Z). The inverse is the forward FFT with re
        // and im swapped.
        for k in 0..m {
            let (ar, ai) = at(k);
            let (br, bi) = at(m - k);
            let bi = -bi;
            let (er, ei) = (ar + br, ai + bi);
            let (dr, di) = (ar - br, ai - bi);
            let (c, s) = (self.wc[k], -self.ws[k]);
            let (or, oi) = (dr * c - di * s, dr * s + di * c);
            w.zr[k] = ei + or;
            w.zi[k] = er - oi;
        }
        self.complex(&mut w.zr, &mut w.zi);
        for i in 0..m {
            w.out[2 * i] = w.zi[i];
            w.out[2 * i + 1] = w.zr[i];
        }
    }
}

/// Work buffers for one stream of frames.
pub(crate) struct Frame {
    out: Vec<f32>,
    zr: Vec<f32>,
    zi: Vec<f32>,
}

impl Frame {
    pub fn new() -> Frame {
        Frame {
            out: vec![0.0; N],
            zr: vec![0.0; N / 2],
            zi: vec![0.0; N / 2],
        }
    }
}

/// Floats a [`Frame`] holds.
pub(crate) const FRAME_FLOATS: usize = 2 * N;

/// One channel of interleaved audio: `channels` samples a frame, the
/// `ch`th of each.
#[derive(Clone, Copy)]
pub(crate) struct Channel<'a> {
    pub data: &'a [f32],
    pub channels: usize,
    pub ch: usize,
}

impl Channel<'_> {
    pub fn samples(&self) -> usize {
        self.data.len() / self.channels
    }
}

/// Frame `t` of one channel's STFT into `re`, `im` (BINS each).
pub(crate) fn analyse(
    x: Channel,
    t: usize,
    rf: &Rfft,
    win: &[f32],
    w: &mut Frame,
    re: &mut [f32],
    im: &mut [f32],
) {
    let len = x.samples();
    let mut frame = std::mem::take(&mut w.out);
    for (i, f) in frame.iter_mut().enumerate() {
        // Index into the signal after its N leading zeros.
        let at = t * HOP + i;
        let v = if at >= N && at - N < len {
            x.data[(at - N) * x.channels + x.ch]
        } else {
            0.0
        };
        *f = v * win[i];
    }
    rf.forward(&frame, re, im, w);
    w.out = frame;
}

/// The inverse STFT a frame at a time: each frame is added in, and the
/// samples no later frame reaches come out.
pub(crate) struct OverlapAdd {
    acc: Vec<f32>,
    w: Frame,
    /// Frames added so far.
    t: usize,
    /// Samples given out so far, and in all.
    done: usize,
    samples: usize,
}

/// Floats an [`OverlapAdd`] holds.
pub(crate) const OVERLAP_FLOATS: usize = N + FRAME_FLOATS;

impl OverlapAdd {
    pub fn new(samples: usize) -> OverlapAdd {
        OverlapAdd {
            acc: vec![0.0; N],
            w: Frame::new(),
            t: 0,
            done: 0,
            samples,
        }
    }

    /// Adds the next frame (BINS complex values); the samples it finishes
    /// go on `out`.
    pub fn push(&mut self, re: &[f32], im: &[f32], rf: &Rfft, win: &[f32], out: &mut Vec<f32>) {
        rf.inverse(re, im, &mut self.w);
        for ((a, &y), &h) in self.acc.iter_mut().zip(&self.w.out).zip(win) {
            *a += y / N as f32 * h;
        }
        // acc holds the padded signal from t·HOP on; its first HOP samples
        // are final now.
        self.give(HOP, out);
        self.acc.copy_within(HOP.., 0);
        self.acc[N - HOP..].fill(0.0);
        self.t += 1;
    }

    /// After the last frame: the rest.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        self.give(N, out);
        let rest = self.samples - self.done;
        out.extend(std::iter::repeat_n(0.0, rest));
        self.done = self.samples;
    }

    fn give(&mut self, n: usize, out: &mut Vec<f32>) {
        // The padded signal's index of acc[0].
        let base = self.t * HOP;
        for i in 0..n {
            let y = base + i;
            if y >= N && self.done < self.samples && y - N == self.done {
                out.push(self.acc[i] * (2.0 / 3.0));
                self.done += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full complex FFT, the reference for [`Rfft`].
    fn fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
        let n = re.len();
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = (i as u32).reverse_bits() as usize >> (32 - bits);
            if j > i {
                re.swap(i, j);
                im.swap(i, j);
            }
        }
        let sign = if inverse { 1.0 } else { -1.0 };
        let mut len = 2;
        while len <= n {
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let a = sign * 2.0 * std::f64::consts::PI * k as f64 / len as f64;
                    let (c, s) = (a.cos() as f32, a.sin() as f32);
                    let (p, q) = (start + k, start + k + len / 2);
                    let tr = re[q] * c - im[q] * s;
                    let ti = re[q] * s + im[q] * c;
                    re[q] = re[p] - tr;
                    im[q] = im[p] - ti;
                    re[p] += tr;
                    im[p] += ti;
                }
            }
            len *= 2;
        }
    }

    fn noise(n: usize, seed: u32) -> Vec<f32> {
        let mut s = seed;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (s >> 8) as f32 / (1u32 << 24) as f32 - 0.5
            })
            .collect()
    }

    #[test]
    fn the_real_transform_matches_a_full_complex_fft() {
        let r = Rfft::new();
        let mut w = Frame::new();
        let x = noise(N, 3);
        let (mut re, mut im) = (vec![0f32; BINS], vec![0f32; BINS]);
        r.forward(&x, &mut re, &mut im, &mut w);
        let (mut fr, mut fi) = (x.clone(), vec![0f32; N]);
        fft(&mut fr, &mut fi, false);
        let peak = fr.iter().chain(&fi).fold(0f32, |a, &b| a.max(b.abs()));
        for k in 0..BINS {
            assert!(
                (re[k] - fr[k]).abs() < 2e-5 * peak && (im[k] - fi[k]).abs() < 2e-5 * peak,
                "bin {k}"
            );
        }
    }

    #[test]
    fn the_real_inverse_matches_a_full_complex_fft() {
        let r = Rfft::new();
        let mut w = Frame::new();
        let re = noise(BINS, 5);
        let im = noise(BINS, 9);
        r.inverse(&re, &im, &mut w);
        let (mut fr, mut fi) = (vec![0f32; N], vec![0f32; N]);
        fr[..BINS].copy_from_slice(&re);
        fi[..BINS].copy_from_slice(&im);
        for k in BINS..N {
            fr[k] = re[N - k];
            fi[k] = -im[N - k];
        }
        fi[0] = 0.0;
        fi[N / 2] = 0.0;
        fft(&mut fr, &mut fi, true);
        let peak = fr.iter().fold(0f32, |a, &b| a.max(b.abs()));
        for i in 0..N {
            assert!((w.out[i] - fr[i]).abs() < 2e-5 * peak, "sample {i}");
        }
    }

    /// A frame at a time, unmasked, the signal comes back; and the
    /// streamed inverse matches a whole-signal one (every frame, then
    /// overlap-added and cropped as Spleeter's `_inverse_stft` does).
    #[test]
    fn the_streamed_pair_gives_the_signal_back() {
        for (len, channels) in [(3 * 44_100 + 17, 2), (5000, 1), (1, 2), (0, 2)] {
            let x = noise(len * channels, 11);
            let (rf, win) = (Rfft::new(), hann());
            let nf = frames(len);
            for ch in 0..channels {
                let c = Channel {
                    data: &x,
                    channels,
                    ch,
                };
                let mut w = Frame::new();
                let mut ola = OverlapAdd::new(len);
                let (mut re, mut im) = (vec![0f32; BINS], vec![0f32; BINS]);
                let mut got = Vec::new();
                let mut whole = vec![0f32; (nf - 1) * HOP + N];
                let mut wi = Frame::new();
                for t in 0..nf {
                    analyse(c, t, &rf, &win, &mut w, &mut re, &mut im);
                    ola.push(&re, &im, &rf, &win, &mut got);
                    rf.inverse(&re, &im, &mut wi);
                    for i in 0..N {
                        whole[t * HOP + i] += wi.out[i] / N as f32 * win[i];
                    }
                }
                ola.finish(&mut got);
                assert_eq!(got.len(), len);
                for (i, &y) in got.iter().enumerate() {
                    let want = x[i * channels + ch];
                    assert!(
                        (y - want).abs() < 1e-5,
                        "{len} ch {ch} [{i}]: {y} vs {want}"
                    );
                    assert_eq!(y, whole[N + i] * (2.0 / 3.0), "{len} ch {ch} [{i}]");
                }
            }
        }
    }
}
