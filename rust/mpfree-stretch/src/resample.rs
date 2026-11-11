//! A windowed-sinc resampler for the pitch shift: reads the stretched
//! audio `speed` times faster, low-passed first when it speeds up so
//! nothing folds back.

/// Zero crossings of the sinc on each side.
const ZEROS: usize = 24;
/// Table steps per zero crossing.
const STEPS: usize = 512;
/// Kaiser window shape (about 80 dB of stop band).
const BETA: f64 = 8.0;

pub(crate) struct Resampler {
    speed: f64,
    /// Cutoff as a share of the input's Nyquist.
    fc: f64,
    /// Taps each side of the read position, in input frames.
    half: i64,
    table: Vec<f32>,
    next: usize,
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-12 * sum {
            break;
        }
    }
    sum
}

impl Resampler {
    pub(crate) fn new(speed: f64) -> Resampler {
        let fc = if speed > 1.0 { 0.97 / speed } else { 0.97 };
        let half = (ZEROS as f64 / fc).ceil() as i64 + 1;
        let n = ZEROS * STEPS + 2;
        let i0b = bessel_i0(BETA);
        let table = (0..n)
            .map(|i| {
                let u = i as f64 / STEPS as f64; // zero crossings from the centre
                if u >= ZEROS as f64 {
                    return 0.0;
                }
                let sinc = if u == 0.0 {
                    1.0
                } else {
                    (std::f64::consts::PI * u).sin() / (std::f64::consts::PI * u)
                };
                let r = u / ZEROS as f64;
                let w = bessel_i0(BETA * (1.0 - r * r).max(0.0).sqrt()) / i0b;
                (sinc * w) as f32
            })
            .collect();
        Resampler {
            speed,
            fc,
            half,
            table,
            next: 0,
        }
    }

    /// Input frames needed past the last read position.
    pub(crate) fn margin(&self) -> usize {
        self.half as usize + 1
    }

    pub(crate) fn done(&self) -> usize {
        self.next
    }

    fn weight(&self, x: f64) -> f32 {
        let u = (x.abs() * self.fc) * STEPS as f64;
        let i = u as usize;
        if i + 1 >= self.table.len() {
            return 0.0;
        }
        let f = (u - i as f64) as f32;
        let a = self.table[i];
        a + (self.table[i + 1] - a) * f
    }

    /// Writes up to `count` more output frames from `input` (interleaved,
    /// `ch` channels) into `out`, which holds `out.len() / ch` frames.
    /// Returns how many it wrote.
    pub(crate) fn run(&mut self, input: &[f32], ch: usize, out: &mut [f32], count: usize) -> usize {
        let frames_in = (input.len() / ch) as i64;
        let total = out.len() / ch;
        let end = (self.next + count).min(total);
        let gain = self.fc as f32;
        for t in self.next..end {
            let pos = t as f64 * self.speed;
            let i0 = pos.floor() as i64;
            let frac = pos - i0 as f64;
            let mut acc = [0.0f32; 2];
            for j in (-self.half + 1)..=self.half {
                let idx = i0 + j;
                if idx < 0 || idx >= frames_in {
                    continue;
                }
                let w = self.weight(j as f64 - frac);
                if w == 0.0 {
                    continue;
                }
                let base = idx as usize * ch;
                for (c, a) in acc.iter_mut().enumerate().take(ch) {
                    *a += input[base + c] * w;
                }
            }
            for c in 0..ch {
                out[t * ch + c] = acc[c] * gain;
            }
        }
        let wrote = end - self.next;
        self.next = end;
        wrote
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slow_sine_comes_out_at_the_new_speed_at_full_level() {
        let rate = 44_100.0;
        let input: Vec<f32> = (0..20_000)
            .map(|i| (std::f64::consts::TAU * 300.0 * i as f64 / rate).sin() as f32)
            .collect();
        for speed in [0.75f64, 1.0, 1.5, 2.0] {
            let mut r = Resampler::new(speed);
            let n = ((20_000.0 / speed) as usize).saturating_sub(r.margin() + 10);
            let mut out = vec![0.0; n];
            assert_eq!(r.run(&input, 1, &mut out, n), n);
            for t in (n / 4..n / 2).step_by(97) {
                let want = (std::f64::consts::TAU * 300.0 * speed * t as f64 / rate).sin();
                assert!(
                    (f64::from(out[t]) - want).abs() < 2e-3,
                    "speed {speed} frame {t}"
                );
            }
        }
    }
}
