//! The phase vocoder: windowed frames of the input are read at the times
//! the map gives and written at a steady hop, each bin's phase moved on
//! by its own frequency so the partials carry on smoothly.
//!
//! - **Phase locking.** Bins are grouped round the spectrum's peaks
//!   (each up to the lowest bin between two peaks), and every bin turns
//!   by its peak's angle, so a partial and its skirt stay one sound
//!   (identity phase locking, Laroche and Dolson).
//! - **Stereo.** Both channels turn by the same angle, worked out on their
//!   sum (or their difference where the sum cancels), so the image and
//!   mono compatibility stay as they were. Left and right go through one
//!   complex FFT together.
//! - **Hits.** At each hit the angles go back to zero, so the attack comes
//!   out as it went in; the map plays the attack at its own speed too
//!   ([`crate::TimeMap`]).

use std::f32::consts::{PI, TAU};
use std::sync::Arc;

use mpfree_analysis::fft::Fft;

use crate::TimeMap;

pub(crate) struct Vocoder {
    input: Arc<[f32]>,
    ch: usize,
    frames_in: i64,
    map: TimeMap,
    /// Input frames where a hit starts, ascending.
    hits: Vec<u64>,
    next_hit: usize,
    n: usize,
    hs: usize,
    fft: Fft,
    win: Vec<f32>,
    norm: f32,
    re: Vec<f32>,
    im: Vec<f32>,
    // One per bin, 0 ..= n / 2.
    lr: Vec<f32>,
    li: Vec<f32>,
    rr: Vec<f32>,
    ri: Vec<f32>,
    power: Vec<f32>,
    phase_m: Vec<f32>,
    phase_s: Vec<f32>,
    prev_m: Vec<f32>,
    prev_s: Vec<f32>,
    side: Vec<bool>,
    theta: Vec<f32>,
    cand: Vec<f32>,
    a_prev: Option<i64>,
    k: i64,
    k_end: i64,
    k_start: i64,
    pub(crate) out: Vec<f32>,
    pub(crate) out_frames: usize,
    pub(crate) resets: usize,
}

/// `x` wrapped to −π..π.
fn wrap(x: f32) -> f32 {
    x - TAU * ((x + PI) / TAU).floor()
}

impl Vocoder {
    /// `out_frames` of output from `input` along `map`, with windows of
    /// `n` frames (a power of two) and a hop of `n / 4`.
    pub(crate) fn new(
        input: Arc<[f32]>,
        ch: usize,
        map: TimeMap,
        hits: Vec<u64>,
        n: usize,
        out_frames: usize,
    ) -> Vocoder {
        let half = n / 2;
        let hs = n / 4;
        let bins = half + 1;
        // Hann on the way in and on the way out; their product overlaps to
        // a steady sum at a hop of n / 4.
        let win: Vec<f32> = (0..n)
            .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos()) as f32)
            .collect();
        let sum_sq: f32 = win.iter().map(|w| w * w).sum();
        let norm = hs as f32 / sum_sq;
        let frames_in = (input.len() / ch) as i64;
        let k_start = 1 - (half / hs) as i64;
        let k_end = (out_frames as i64 - 1 + half as i64) / hs as i64;
        Vocoder {
            input,
            ch,
            frames_in,
            map,
            hits,
            next_hit: 0,
            n,
            hs,
            fft: Fft::new(n),
            win,
            norm,
            re: vec![0.0; n],
            im: vec![0.0; n],
            lr: vec![0.0; bins],
            li: vec![0.0; bins],
            rr: vec![0.0; bins],
            ri: vec![0.0; bins],
            power: vec![0.0; bins],
            phase_m: vec![0.0; bins],
            phase_s: vec![0.0; bins],
            prev_m: vec![0.0; bins],
            prev_s: vec![0.0; bins],
            side: vec![false; bins],
            theta: vec![0.0; bins],
            cand: vec![0.0; bins],
            a_prev: None,
            k: k_start,
            k_end,
            k_start,
            out: vec![0.0; out_frames * ch],
            out_frames,
            resets: 0,
        }
    }

    pub(crate) fn finished(&self) -> bool {
        self.k > self.k_end
    }

    /// Share of the frames done, 0 to 1.
    pub(crate) fn progress(&self) -> f32 {
        let all = (self.k_end - self.k_start + 1).max(1) as f32;
        ((self.k - self.k_start) as f32 / all).clamp(0.0, 1.0)
    }

    /// Output frames each step writes (its hop).
    pub(crate) fn hop(&self) -> usize {
        self.hs
    }

    /// Runs up to `frames` more frames. Returns how many it ran.
    pub(crate) fn run(&mut self, frames: usize) -> usize {
        let mut done = 0;
        while done < frames && !self.finished() {
            self.frame();
            self.k += 1;
            done += 1;
        }
        done
    }

    fn frame(&mut self) {
        let (n, half, hs, ch) = (self.n, self.n / 2, self.hs as i64, self.ch);
        let centre_out = self.k * hs;
        let a = self.map.input_at(centre_out as f64).round() as i64;
        // Read the window: left in the real part, right in the imaginary.
        for i in 0..n {
            let idx = a - half as i64 + i as i64;
            let w = self.win[i];
            if (0..self.frames_in).contains(&idx) {
                let b = idx as usize * ch;
                self.re[i] = self.input[b] * w;
                self.im[i] = if ch == 2 { self.input[b + 1] * w } else { 0.0 };
            } else {
                self.re[i] = 0.0;
                self.im[i] = 0.0;
            }
        }
        self.fft.forward(&mut self.re, &mut self.im);
        let mut max_power = 0.0f32;
        for k in 0..=half {
            let (zr, zi) = (self.re[k], self.im[k]);
            if ch == 2 {
                let j = (n - k) % n;
                let (wr, wi) = (self.re[j], self.im[j]);
                self.lr[k] = 0.5 * (zr + wr);
                self.li[k] = 0.5 * (zi - wi);
                self.rr[k] = 0.5 * (zi + wi);
                self.ri[k] = -0.5 * (zr - wr);
                let (mr, mi) = (self.lr[k] + self.rr[k], self.li[k] + self.ri[k]);
                let (sr, si) = (self.lr[k] - self.rr[k], self.li[k] - self.ri[k]);
                let (m2, s2) = (mr * mr + mi * mi, sr * sr + si * si);
                self.side[k] = s2 > 4.0 * m2;
                self.phase_m[k] = mi.atan2(mr);
                self.phase_s[k] = si.atan2(sr);
                self.power[k] = 0.5 * (m2 + s2);
            } else {
                self.lr[k] = zr;
                self.li[k] = zi;
                self.phase_m[k] = zi.atan2(zr);
                self.power[k] = zr * zr + zi * zi;
            }
            max_power = max_power.max(self.power[k]);
        }
        // A hit between the last read and this one starts the phases again.
        let mut hit = false;
        while self.next_hit < self.hits.len() && (self.hits[self.next_hit] as i64) <= a {
            if self
                .a_prev
                .is_some_and(|p| (self.hits[self.next_hit] as i64) > p)
            {
                hit = true;
            }
            self.next_hit += 1;
        }
        let ha = self.a_prev.map(|p| a - p);
        match ha {
            None => self.theta.fill(0.0),
            Some(_) if hit => {
                self.theta.fill(0.0);
                self.resets += 1;
            }
            Some(ha) if ha <= 0 => {} // read the same place again: turn as before
            Some(ha) => self.advance(ha as f32, hs as f32, max_power),
        }
        self.prev_m.copy_from_slice(&self.phase_m);
        self.prev_s.copy_from_slice(&self.phase_s);
        self.a_prev = Some(a);
        self.synthesise(centre_out);
    }

    /// New angles for a read `ha` frames on from the last and a write `hs`
    /// on.
    fn advance(&mut self, ha: f32, hs: f32, max_power: f32) {
        let half = self.n / 2;
        let step = TAU / self.n as f32;
        let k_ratio = hs / ha - 1.0;
        for k in 0..=half {
            let (cur, prev) = if self.side[k] {
                (self.phase_s[k], self.prev_s[k])
            } else {
                (self.phase_m[k], self.prev_m[k])
            };
            let omega = step * k as f32;
            let dphi = wrap(cur - prev - omega * ha);
            self.cand[k] = wrap(self.theta[k] + (omega * ha + dphi) * k_ratio);
        }
        // Peaks: above both neighbours, and not lost in the noise floor.
        let floor = max_power * 1e-9;
        let p = &self.power;
        let mut last_peak: Option<usize> = None;
        let mut k = 1;
        while k < half {
            if p[k] > floor && p[k] > p[k - 1] && p[k] >= p[k + 1] {
                match last_peak {
                    None => {
                        for t in 0..=k {
                            self.theta[t] = self.cand[k];
                        }
                    }
                    Some(lp) => {
                        // The lowest bin between the two peaks is the border.
                        let mut trough = lp + 1;
                        for t in lp + 1..k {
                            if p[t] < p[trough] {
                                trough = t;
                            }
                        }
                        for t in lp + 1..=trough {
                            self.theta[t] = self.cand[lp];
                        }
                        for t in trough + 1..=k {
                            self.theta[t] = self.cand[k];
                        }
                    }
                }
                last_peak = Some(k);
            }
            k += 1;
        }
        match last_peak {
            Some(lp) => {
                for t in lp + 1..=half {
                    self.theta[t] = self.cand[lp];
                }
            }
            None => self.theta.copy_from_slice(&self.cand),
        }
    }

    fn synthesise(&mut self, centre_out: i64) {
        let (n, half, ch) = (self.n, self.n / 2, self.ch);
        for k in 0..=half {
            let (lr, li, rr, ri) = (self.lr[k], self.li[k], self.rr[k], self.ri[k]);
            if k == 0 || k == half {
                // DC and Nyquist stay real.
                self.re[k] = lr;
                self.im[k] = if ch == 2 { rr } else { 0.0 };
                continue;
            }
            let (s, c) = self.theta[k].sin_cos();
            let (ylr, yli) = (lr * c - li * s, lr * s + li * c);
            if ch == 2 {
                let (yrr, yri) = (rr * c - ri * s, rr * s + ri * c);
                // Z = YL + i·YR, and its mirror conj(YL) + i·conj(YR).
                self.re[k] = ylr - yri;
                self.im[k] = yli + yrr;
                self.re[n - k] = ylr + yri;
                self.im[n - k] = yrr - yli;
            } else {
                self.re[k] = ylr;
                self.im[k] = yli;
                self.re[n - k] = ylr;
                self.im[n - k] = -yli;
            }
        }
        // Inverse by conjugation: ifft(z) = conj(fft(conj(z))) / n.
        for v in self.im.iter_mut() {
            *v = -*v;
        }
        self.fft.forward(&mut self.re, &mut self.im);
        let scale = self.norm / n as f32;
        let start = centre_out - half as i64;
        for i in 0..n {
            let t = start + i as i64;
            if t < 0 || t >= self.out_frames as i64 {
                continue;
            }
            let w = self.win[i] * scale;
            let b = t as usize * ch;
            self.out[b] += self.re[i] * w;
            if ch == 2 {
                self.out[b + 1] -= self.im[i] * w;
            }
        }
    }
}
