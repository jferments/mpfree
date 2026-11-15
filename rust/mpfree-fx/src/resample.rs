//! Sample-rate conversion for export: 44.1 kHz to 48 kHz (or any two
//! whole-number rates), streaming, on the export's writer thread.
//!
//! A polyphase windowed-sinc filter for the ratio of the rates in lowest
//! terms (`up / down`, 160 / 147 from 44.1 kHz to 48 kHz). Output frame
//! `n` sits at input time `n * down / up` exactly, so the files keep their
//! first frame on the song's first frame and stay lined up with each
//! other and with the MIDI file. The filter passes up to 20 kHz (or 45 %
//! of the lower rate) flat to within about 0.0001 dB and is about 100 dB
//! down from where anything would fold back into that band.
//!
//! It allocates (it makes its tables and grows its buffers), so it never
//! runs on the sound thread.

/// Taps each phase has, in input frames.
pub const TAPS: usize = 80;
/// Kaiser window shape: about 100 dB down in the stop band.
const BETA: f64 = 10.06;
/// The top of the band kept flat, in Hz (45 % of the lower rate when that
/// is less).
const PASS_HZ: f64 = 20_000.0;
/// Input frames that may build up before the used ones are dropped.
const SLACK: usize = 8192;

/// A converter from one rate to another for some channels (see the
/// module docs).
pub struct Resampler {
    up: u64,
    down: u64,
    /// `phases[p][k]`: the filter's taps for phase `p`, each phase adding
    /// up to 1, so a steady level stays exactly where it was.
    phases: Vec<[f32; TAPS]>,
    /// The filter's middle, in steps of `1 / up` input frames.
    centre: u64,
    /// Each channel's input from frame `base` on.
    bufs: Vec<Vec<f32>>,
    base: i64,
    /// Input frames taken.
    taken: u64,
    /// The next output frame.
    next: u64,
    finished: bool,
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    let q = x * x / 4.0;
    for k in 1..200 {
        term *= q / (k * k) as f64;
        sum += term;
        if term < 1e-17 * sum {
            break;
        }
    }
    sum
}

/// The output frames `frames` input frames make going from `from` to
/// `to` Hz: every output frame that falls before the input's end.
pub fn frames_out(frames: u64, from: u32, to: u32) -> u64 {
    if from == to || from == 0 {
        return frames;
    }
    let g = gcd(u64::from(from), u64::from(to));
    let (up, down) = (u64::from(to) / g, u64::from(from) / g);
    (frames * up).div_ceil(down)
}

impl Resampler {
    /// A converter from `from` to `to` Hz for `channels` channels.
    pub fn new(from: u32, to: u32, channels: usize) -> Resampler {
        let g = gcd(u64::from(from), u64::from(to)).max(1);
        let (up, down) = (u64::from(to) / g, u64::from(from) / g);
        let n = up as usize * TAPS;
        // An odd-length filter (the last tap left at zero) has a middle
        // on a whole step.
        let centre = (n / 2 - 1) as f64;
        let lo = f64::from(from.min(to));
        let pass = PASS_HZ.min(0.45 * lo);
        let stop = lo - pass;
        // Normalised to the filter's own rate, `from * up`.
        let cut = (pass + stop) / 2.0 / (f64::from(from) * up as f64);
        let i0b = bessel_i0(BETA);
        let tap = |m: usize| {
            let x = m as f64 - centre;
            let r = x / centre;
            if r.abs() > 1.0 {
                return 0.0;
            }
            let arg = 2.0 * cut * x;
            let sinc = if arg == 0.0 {
                1.0
            } else {
                (std::f64::consts::PI * arg).sin() / (std::f64::consts::PI * arg)
            };
            sinc * bessel_i0(BETA * (1.0 - r * r).sqrt()) / i0b
        };
        let phases = (0..up as usize)
            .map(|p| {
                let h: Vec<f64> = (0..TAPS).map(|k| tap(p + k * up as usize)).collect();
                let sum: f64 = h.iter().sum();
                let mut out = [0f32; TAPS];
                for (o, v) in out.iter_mut().zip(&h) {
                    *o = (v / sum) as f32;
                }
                out
            })
            .collect();
        Resampler {
            up,
            down,
            phases,
            centre: centre as u64,
            // Silence before the start.
            bufs: vec![vec![0.0; TAPS]; channels],
            base: -(TAPS as i64),
            taken: 0,
            next: 0,
            finished: false,
        }
    }

    /// The newest input frame output frame `n` reads, and its phase.
    fn place(&self, n: u64) -> (i64, usize) {
        let u = n * self.down + self.centre;
        ((u / self.up) as i64, (u % self.up) as usize)
    }

    /// Output frames as far as the input reaches, appended to `out` (one
    /// per channel).
    fn run(&mut self, out: &mut [Vec<f32>], until: u64) {
        let end = self.base + self.bufs[0].len() as i64;
        while self.next < until {
            let (i, p) = self.place(self.next);
            if i >= end {
                break;
            }
            let h = &self.phases[p];
            let at = (i - self.base) as usize;
            for (buf, o) in self.bufs.iter().zip(out.iter_mut()) {
                // x[i - k] for k = 0..TAPS: the window ending at `at`.
                let x = &buf[at + 1 - TAPS..=at];
                let mut acc = 0f32;
                for (c, v) in h.iter().zip(x.iter().rev()) {
                    acc += c * v;
                }
                o.push(acc);
            }
            self.next += 1;
        }
        // Drop the input no output frame will read again.
        let (i, _) = self.place(self.next);
        let used = (i - TAPS as i64 + 1 - self.base).max(0) as usize;
        if used > SLACK {
            for b in &mut self.bufs {
                b.drain(..used);
            }
            self.base += used as i64;
        }
    }

    /// Takes the next input frames (`input[c]` for channel `c`, all the
    /// same length) and appends the output frames they complete to
    /// `out[c]`.
    pub fn push(&mut self, input: &[&[f32]], out: &mut [Vec<f32>]) {
        debug_assert!(!self.finished, "pushed after finish");
        let n = input.first().map_or(0, |c| c.len());
        for (b, c) in self.bufs.iter_mut().zip(input) {
            b.extend_from_slice(&c[..n]);
        }
        self.taken += n as u64;
        self.run(out, u64::MAX);
    }

    /// The input is over: appends the output frames still owed, up to the
    /// input's end (silence after it).
    pub fn finish(&mut self, out: &mut [Vec<f32>]) {
        if self.finished {
            return;
        }
        self.finished = true;
        let total = (self.taken * self.up).div_ceil(self.down);
        if total == 0 {
            return;
        }
        let (last, _) = self.place(total - 1);
        let end = self.base + self.bufs[0].len() as i64;
        let pad = (last + 1 - end).max(0) as usize;
        for b in &mut self.bufs {
            b.resize(b.len() + pad, 0.0);
        }
        self.run(out, total);
    }

    /// Output frames made so far.
    pub fn made(&self) -> u64 {
        self.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f64, rate: f64, n: usize, amp: f64) -> Vec<f32> {
        (0..n)
            .map(|i| (amp * (std::f64::consts::TAU * f * i as f64 / rate).sin()) as f32)
            .collect()
    }

    /// The level of frequency `f` in `x` (Goertzel), as a share of a full
    /// sine's.
    fn level(x: &[f32], f: f64, rate: f64) -> f64 {
        let w = std::f64::consts::TAU * f / rate;
        let (mut s1, mut s2) = (0.0, 0.0);
        let n = x.len();
        for (i, v) in x.iter().enumerate() {
            // A Hann window keeps the other tones from leaking in.
            let win = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            let s = f64::from(*v) * win + 2.0 * w.cos() * s1 - s2;
            (s2, s1) = (s1, s);
        }
        let p = s1 * s1 + s2 * s2 - 2.0 * w.cos() * s1 * s2;
        p.sqrt() / (n as f64 / 4.0)
    }

    fn convert(x: &[f32], from: u32, to: u32, chunk: usize) -> Vec<f32> {
        let mut r = Resampler::new(from, to, 1);
        let mut out = vec![Vec::new()];
        for c in x.chunks(chunk) {
            r.push(&[c], &mut out);
        }
        r.finish(&mut out);
        out.pop().unwrap()
    }

    #[test]
    fn a_sine_comes_out_the_same_sine_at_the_new_rate_on_the_same_time() {
        let x = sine(1000.0, 44_100.0, 44_100, 0.5);
        let y = convert(&x, 44_100, 48_000, 333);
        assert_eq!(y.len() as u64, frames_out(44_100, 44_100, 48_000));
        assert_eq!(y.len(), 48_000);
        let want = sine(1000.0, 48_000.0, 48_000, 0.5);
        // Away from the edges (the sound starts and stops there).
        let worst = (2000..46_000)
            .map(|i| (y[i] - want[i]).abs())
            .fold(0f32, f32::max);
        assert!(worst < 2e-5, "within -100 dB of the right sine: {worst}");
    }

    #[test]
    fn a_click_lands_on_the_same_moment() {
        let mut x = vec![0f32; 44_100];
        x[4410] = 1.0; // 0.1 s
        let y = convert(&x, 44_100, 48_000, 4096);
        let peak = (0..y.len())
            .max_by(|&a, &b| y[a].abs().total_cmp(&y[b].abs()))
            .unwrap();
        assert_eq!(peak, 4800, "0.1 s at 48 kHz");
    }

    #[test]
    fn nothing_folds_back_into_the_band() {
        // 19 kHz going up: its image at 25.1 kHz would fold to 22.9 kHz.
        let x = sine(19_000.0, 44_100.0, 88_200, 0.9);
        let y = convert(&x, 44_100, 48_000, 1000);
        let mid = &y[4800..y.len() - 4800];
        let kept = level(mid, 19_000.0, 48_000.0);
        let folded = level(mid, 22_900.0, 48_000.0);
        assert!((kept - 0.9).abs() < 0.001, "19 kHz kept: {kept}");
        assert!(folded < 0.9 * 1e-5, "the image is 100 dB down: {folded}");
        // Going down, 25 kHz must not fold to 19.1 kHz.
        let x = sine(25_000.0, 48_000.0, 96_000, 0.9);
        let y = convert(&x, 48_000, 44_100, 1000);
        let mid = &y[4410..y.len() - 4410];
        let folded = level(mid, 19_100.0, 44_100.0);
        assert!(folded < 0.9 * 1e-5, "25 kHz gone: {folded}");
        // And 18 kHz is kept.
        let x = sine(18_000.0, 48_000.0, 96_000, 0.9);
        let y = convert(&x, 48_000, 44_100, 1000);
        let kept = level(&y[4410..y.len() - 4410], 18_000.0, 44_100.0);
        assert!((kept - 0.9).abs() < 0.001, "18 kHz kept: {kept}");
    }

    #[test]
    fn a_steady_level_stays_exactly_and_any_chunks_give_the_same() {
        let x = vec![0.25f32; 20_000];
        let y = convert(&x, 44_100, 48_000, 64);
        // Inside: every phase adds up to 1.
        for v in &y[200..y.len() - 200] {
            assert!((v - 0.25).abs() < 1e-6, "{v}");
        }
        let noise: Vec<f32> = (0..30_000u32)
            .map(|i| ((i.wrapping_mul(2_654_435_761) >> 8) as f32 / 16_777_216.0) - 0.5)
            .collect();
        let a = convert(&noise, 44_100, 48_000, 1);
        let b = convert(&noise, 44_100, 48_000, 30_000);
        let c = convert(&noise, 44_100, 48_000, 777);
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn frame_counts() {
        assert_eq!(frames_out(147, 44_100, 48_000), 160);
        assert_eq!(frames_out(1, 44_100, 48_000), 2);
        assert_eq!(frames_out(160, 48_000, 44_100), 147);
        assert_eq!(frames_out(5, 44_100, 44_100), 5);
        assert_eq!(convert(&[], 44_100, 48_000, 10).len(), 0);
        assert_eq!(convert(&[0.5; 7], 44_100, 48_000, 3).len(), 8);
    }
}
