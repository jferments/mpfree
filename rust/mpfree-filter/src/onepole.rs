//! A one-pole stage (6 dB an octave), low-pass or high-pass: the
//! trapezoidal integrator the state-variable stage is built from
//! ([`crate::svf::Svf`]), on its own, so it sweeps as safely. The odd
//! slopes of the pads' multimode filter (6 and 18 dB) start with one
//! ([`crate::multi::Multi`]; the filter gap plan's F4).

/// One stereo one-pole stage.
#[derive(Clone, Copy, Debug)]
pub struct OnePole {
    /// `g / (1 + g)`, with `g = tan(pi hz / rate)` the prewarped corner.
    gain: f32,
    /// A high-pass (the input less the low-pass), not a low-pass.
    hp: bool,
    s: [f32; 2],
}

impl Default for OnePole {
    fn default() -> OnePole {
        let mut o = OnePole {
            gain: 0.0,
            hp: false,
            s: [0.0; 2],
        };
        o.set(OnePole::gain_for(0.065_543_6), false);
        o
    }
}

impl OnePole {
    /// The stage's gain for a prewarped corner `t = tan(pi hz / rate)`.
    #[inline]
    pub fn gain_for(t: f32) -> f32 {
        t / (1.0 + t)
    }

    /// Sets its gain ([`OnePole::gain_for`]) and whether it is a
    /// high-pass.
    #[inline]
    pub fn set(&mut self, gain: f32, hp: bool) {
        self.gain = gain;
        self.hp = hp;
    }

    /// Sets its gain alone, as a glide moves it.
    #[inline(always)]
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    /// The gain it is set to.
    #[inline]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// One sample of channel `ch`.
    #[inline(always)]
    fn step(&mut self, ch: usize, x: f32) -> f32 {
        let s = self.s[ch];
        let v = (x - s) * self.gain;
        let lp = v + s;
        self.s[ch] = lp + v;
        if self.hp {
            x - lp
        } else {
            lp
        }
    }

    /// One frame of both channels; the host flushes the states every few
    /// frames ([`OnePole::flush_below`]).
    #[inline(always)]
    pub fn step_stereo(&mut self, l: f32, r: f32) -> (f32, f32) {
        (self.step(0, l), self.step(1, r))
    }

    /// One sample of channel `ch`, its state flushed to zero once too
    /// small to hear, as [`crate::svf::Svf::run`] flushes its own (the
    /// Filter effect's odd slopes, whose host flushes nothing).
    #[inline]
    pub fn run(&mut self, ch: usize, x: f32) -> f32 {
        let y = self.step(ch, x);
        self.s[ch] = crate::flush(self.s[ch]);
        y
    }

    pub fn reset(&mut self) {
        self.s = [0.0; 2];
    }

    /// Starts as if it had been hearing a sound whose low part is `low`
    /// ([`crate::svf::Svf::start_from`]).
    #[inline]
    pub fn start_from(&mut self, low: [f32; 2]) {
        self.s = low;
    }

    /// Its state, each channel's: the low-pass of what it has heard.
    #[inline]
    pub fn low_state(&self) -> [f32; 2] {
        self.s
    }

    /// Carries on from `from`'s state (the same stage at another rate).
    #[inline]
    pub fn copy_states(&mut self, from: &OnePole) {
        self.s = from.s;
    }

    /// Zeros each channel's state once it is under `quiet`.
    #[inline]
    pub fn flush_below(&mut self, quiet: f32) {
        for s in &mut self.s {
            if s.abs() < quiet {
                *s = 0.0;
            }
        }
    }

    /// The steady-state response at `hz` as a complex gain `(re, im)`:
    /// exact, as the stage is the bilinear transform of `1 / (1 + s)` (or
    /// `s / (1 + s)`) with `s` scaled to its prewarped corner.
    pub fn response(&self, hz: f32, rate: u32) -> (f64, f64) {
        let gain = f64::from(self.gain);
        let g = gain / (1.0 - gain);
        let w = (std::f64::consts::PI * f64::from(hz) / f64::from(rate)).tan() / g;
        let d = 1.0 + w * w;
        if self.hp {
            (w * w / d, w / d)
        } else {
            (1.0 / d, -w / d)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_is_3_db_down_at_its_corner_and_6_db_an_octave_past_it() {
        let rate = 44_100;
        for hp in [false, true] {
            // Well under Nyquist, where the bilinear transform's warping
            // is small.
            let corner = if hp { 4_000.0 } else { 100.0 };
            let t = (std::f32::consts::PI * corner / rate as f32).tan();
            let mut o = OnePole::default();
            o.set(OnePole::gain_for(t), hp);
            let set = o;
            let db = |hz: f32| {
                let (re, im) = set.response(hz, rate);
                10.0 * (re * re + im * im).log10()
            };
            assert!((db(corner) + 3.0103).abs() < 1e-3, "{hp}");
            // From 4 to 8 times past the corner: 10 log(65 / 17), 5.8 dB.
            let (near, far) = if hp { (1_000.0, 500.0) } else { (400.0, 800.0) };
            let slope = db(near) - db(far);
            assert!((slope - 5.82).abs() < 0.05, "{hp} {slope}");
            // What it does is what it draws.
            let n = 20_000;
            let hz = 3_000.0;
            let mut peak = 0.0f32;
            for i in 0..n {
                let w = std::f64::consts::TAU * f64::from(hz) * i as f64 / f64::from(rate);
                let x = w.sin() as f32;
                let (y, _) = o.step_stereo(x, x);
                if i > n / 2 {
                    peak = peak.max(y.abs());
                }
            }
            let heard = 20.0 * f64::from(peak).log10();
            assert!((heard - db(hz)).abs() < 0.05, "{hp}: {heard} {}", db(hz));
        }
    }
}
