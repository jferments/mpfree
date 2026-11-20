//! The Classic voices: the drum models exactly as the engine first built
//! them (until 19 Nov 2026), kept so a kit or project saved before plays
//! as it did, to the bit. A kit plays these when its Voices setting is
//! Classic, the default for any kit that doesn't name it. Don't change
//! anything here: the Current voices are in `current.rs`.

use crate::drum_params::*;
use crate::dsp::{drive_sq, osc, Rng, Wave};

pub(super) const SILENT: f32 = 1e-4;

/// The 808's six hat oscillators, in Hz at Tune 0.
pub const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];
/// Clap: the handclaps' starts after the hit, in seconds, then the tail.
pub(super) const CLAP_BURSTS: [f32; 4] = [0.0, 0.0105, 0.0215, 0.0305];

/// A sine of `ph` turns (any value), within about 0.001.
#[inline]
pub fn sin_turns(ph: f32) -> f32 {
    // sin(2 pi x) = -sin(pi s) with s = 2x - 1 in -1..1; a parabola for
    // sin(pi s), refined.
    let x = ph - floor(ph);
    let s = 2.0 * x - 1.0;
    let p = 4.0 * s * (1.0 - s.abs());
    -(0.225 * (p * p.abs() - p) + p)
}

/// `x.floor()`, the same value for every input: a positive value below
/// 2^23 (every phase here in practice) truncates through an i32, which is
/// its floor, instead of the library call `floorf` is on ARMv7.
#[inline]
pub(super) fn floor(x: f32) -> f32 {
    if x > 0.0 && x < 8_388_608.0 {
        x as i32 as f32
    } else {
        x.floor()
    }
}

/// A per-frame multiplier that falls 60 dB in `secs`.
#[inline]
pub(super) fn t60(secs: f32, rate: f32) -> f32 {
    (-6.907_755 / (secs.max(1e-4) * rate)).exp()
}

/// A trapezoidal state-variable filter with every output (Simper).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Svf {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum Out {
    #[default]
    Off,
    Lp,
    Bp,
    Hp,
}

impl Svf {
    pub(super) fn set(&mut self, fc: f32, q: f32, rate: f32) {
        let fc = fc.clamp(20.0, 0.45 * rate);
        let g = (std::f32::consts::PI * fc / rate).tan();
        self.k = 1.0 / q.max(0.3);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }
    #[inline]
    pub(super) fn run(&mut self, out: Out, x: f32) -> f32 {
        if out == Out::Off {
            return x;
        }
        let v3 = x - self.ic2;
        let v1 = self.a1 * self.ic1 + self.a2 * v3;
        let v2 = self.ic2 + self.a2 * self.ic1 + self.a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        match out {
            Out::Lp => v2,
            Out::Bp => v1 * self.k,
            _ => x - self.k * v1 - v2,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(super) enum State {
    #[default]
    Free,
    On,
    /// Fading out (replaced, choked or AllOff), then free.
    Kill,
}

/// One hit. Everything a model needs; a model leaves what it doesn't use
/// at zero.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Voice {
    pub(super) state: State,
    pub(super) pad: u8,
    pub(super) choke: u8,
    pub(super) age: u64,
    /// The hit's gain before pan (velocity and level), for a drum whose
    /// hits go through its effects ([`crate::drum_fx`]), panned after them.
    pub(super) gain: f32,
    pub(super) gain_l: f32,
    pub(super) gain_r: f32,
    pub(super) kill: f32,
    pub(super) kill_step: f32,
    // Body: up to two sines with a pitch sweep, and FM.
    pub(super) f0: f32,
    pub(super) ph_a: f32,
    pub(super) ph_b: f32,
    pub(super) ratio_b: f32,
    pub(super) lvl_a: f32,
    pub(super) lvl_b: f32,
    pub(super) body: f32,
    pub(super) body_mul: f32,
    pub(super) sweep: f32,
    pub(super) pitch: f32,
    pub(super) pitch_mul: f32,
    pub(super) shape: f32,
    pub(super) fm_ph: f32,
    pub(super) fm_ratio: f32,
    pub(super) fm_index: f32,
    // Metal: square oscillators.
    pub(super) n_metal: usize,
    pub(super) ph_m: [f32; 6],
    pub(super) dt_m: [f32; 6],
    pub(super) metal_lvl: f32,
    // Noise and metal share the filters and the noise envelope.
    pub(super) rng: Rng,
    pub(super) noise_lvl: f32,
    pub(super) noise: f32,
    pub(super) noise_mul: f32,
    pub(super) f1: Svf,
    pub(super) o1: Out,
    pub(super) f2: Svf,
    pub(super) o2: Out,
    // Clap: frames since the hit, the bursts, the tail.
    pub(super) clap: bool,
    pub(super) t: u32,
    pub(super) burst: f32,
    pub(super) burst_mul: f32,
    pub(super) next_burst: usize,
    // Snap: a short noise click.
    pub(super) click: f32,
    pub(super) click_mul: f32,
    pub(super) click_lvl: f32,
    pub(super) drive: f32,
    /// What the drive stage takes, from `drive`: the gain, its square root
    /// and the amount.
    pub(super) drive_gain: f32,
    pub(super) drive_sq: f32,
    pub(super) drive_amt: f32,
    /// Frames since the hit: the silence check runs every 64, so when a
    /// hit ends doesn't depend on the block size.
    pub(super) frames: u32,
}

impl Voice {
    // Always inlined: it runs in the plain loop and the effects' loop, and
    // a call per frame cost the plain kit 8 % (35,000 instructions a block).
    #[inline(always)]
    pub(super) fn tick(&mut self, rate: f32) -> f32 {
        let inv = 1.0 / rate;
        let mut out = 0.0;
        if self.lvl_a > 0.0 {
            // No sweep: exp2(0) is exactly 1, so the pitch is f0 as it is.
            let hz = if self.sweep == 0.0 && self.pitch.is_finite() {
                self.f0
            } else {
                self.f0 * (self.sweep * self.pitch * (1.0 / 12.0)).exp2()
            };
            self.pitch *= self.pitch_mul;
            self.ph_a += hz * inv;
            if self.ph_a >= 1.0 {
                self.ph_a -= 1.0;
            }
            let mut s = if self.fm_index > 0.0 {
                self.fm_ph += hz * self.fm_ratio * inv;
                if self.fm_ph >= 1.0 {
                    self.fm_ph -= floor(self.fm_ph);
                }
                let ix = self.fm_index * (0.15 + 0.85 * self.pitch);
                sin_turns(self.ph_a + ix * sin_turns(self.fm_ph) * 0.159_155)
            } else {
                sin_turns(self.ph_a)
            };
            if self.shape > 0.0 {
                s = s * (1.0 + self.shape) / (1.0 + self.shape * s.abs());
            }
            let mut b = self.lvl_a * s;
            if self.lvl_b > 0.0 {
                self.ph_b += hz * self.ratio_b * inv;
                if self.ph_b >= 1.0 {
                    self.ph_b -= floor(self.ph_b);
                }
                b += self.lvl_b * sin_turns(self.ph_b);
            }
            out += b * self.body;
            self.body *= self.body_mul;
        }
        if self.noise_lvl > 0.0 || self.n_metal > 0 {
            let mut x = 0.0;
            if self.n_metal > 0 {
                let mut m = 0.0;
                for k in 0..self.n_metal {
                    m += osc(Wave::Pulse, &mut self.ph_m[k], self.dt_m[k], 0.5);
                }
                x += m * self.metal_lvl;
            }
            if self.noise_lvl > 0.0 {
                x += self.rng.bipolar() * self.noise_lvl;
            }
            let mut env = self.noise;
            self.noise *= self.noise_mul;
            if self.clap {
                if self.next_burst < CLAP_BURSTS.len()
                    && self.t as f32 >= CLAP_BURSTS[self.next_burst] * rate
                {
                    self.burst = 1.0;
                    self.next_burst += 1;
                }
                env = env * 0.35 + self.burst;
                self.burst *= self.burst_mul;
                self.t = self.t.saturating_add(1);
            }
            let y = self.f2.run(self.o2, self.f1.run(self.o1, x));
            out += y * env;
        }
        if self.click > SILENT {
            out += self.rng.bipolar() * self.click * self.click_lvl;
            self.click *= self.click_mul;
        }
        if self.drive > 0.0 {
            // drive(out, 1 + 9 drive, sqrt(drive)) with its square roots
            // worked out at the hit.
            out = drive_sq(out, self.drive_gain, self.drive_sq, self.drive_amt);
        }
        if self.state == State::Kill {
            out *= self.kill;
            self.kill -= self.kill_step;
            if self.kill <= 0.0 {
                self.state = State::Free;
            }
        }
        self.frames = self.frames.wrapping_add(1);
        if self.frames.is_multiple_of(64) && self.state == State::On && self.silent() {
            self.state = State::Free;
        }
        out
    }

    /// About how loud the hit is now: its parts' envelopes times their
    /// levels and the hit's gain (velocity, level, pan).
    pub(super) fn level(&self) -> f32 {
        let parts = self.body * self.lvl_a.max(self.lvl_b)
            + self.noise * (self.noise_lvl + self.metal_lvl)
            + self.click * self.click_lvl
            + if self.clap { self.burst } else { 0.0 };
        parts * self.gain_l.max(self.gain_r) * std::f32::consts::SQRT_2
    }

    fn silent(&self) -> bool {
        self.body * self.lvl_a.max(self.lvl_b) < SILENT
            && self.noise * (self.noise_lvl + self.metal_lvl) < SILENT
            && self.click * self.click_lvl < SILENT
            && (!self.clap || (self.next_burst >= CLAP_BURSTS.len() && self.burst < SILENT))
    }
}

/// A hit of `pad` from its fields (`g` reads one field of the pad), at
/// `rate`: what [`Drums`] plays and [`hit_shape`] draws.
pub(super) fn build(
    g: impl Fn(u16) -> f32,
    rate: f32,
    pad: usize,
    velocity: u8,
    seed: u64,
    clock: u64,
) -> Voice {
    use crate::drum_params::model::*;
    let vel = f32::from(velocity.min(127)) / 127.0;
    let model = g(field::MODEL) as u8;
    let ratio = (g(field::TUNE) / 12.0).exp2();
    let decay = g(field::DECAY) * 0.001;
    let tone =
        (g(field::TONE) / 100.0 - g(field::VEL_TONE) / 100.0 * (1.0 - vel) * 0.6).clamp(0.0, 1.0);
    let snap = g(field::SNAP) / 100.0;
    let nz = g(field::NOISE) / 100.0;
    let vl = g(field::VEL_LEVEL) / 100.0;
    let amp = (1.0 - vl + vl * vel * vel)
        * if g(field::LEVEL) <= -60.0 {
            0.0
        } else {
            10f32.powf(g(field::LEVEL) / 20.0)
        };
    let pan = g(field::PAN) / 100.0;

    let mut v = Voice {
        state: State::On,
        pad: pad as u8,
        choke: g(field::CHOKE) as u8,
        age: clock,
        kill: 1.0,
        rng: Rng::new(seed),
        body: 1.0,
        body_mul: t60(decay, rate),
        noise: 1.0,
        noise_mul: t60(decay, rate),
        pitch: 1.0,
        pitch_mul: (-4.6 / (g(field::SWEEP_TIME) * 0.001 * rate).max(1.0)).exp(),
        sweep: g(field::SWEEP),
        click: 1.0,
        click_mul: t60(0.004, rate),
        click_lvl: snap * 0.5,
        drive: g(field::DRIVE) / 100.0,
        ..Voice::default()
    };
    v.drive_gain = 1.0 + v.drive * 9.0;
    v.drive_sq = v.drive_gain.sqrt();
    v.drive_amt = v.drive.sqrt();
    // How loud each model is at full velocity, so the defaults peak
    // near -12 dBFS.
    let norm;
    match model {
        KICK => {
            v.f0 = 50.0 * ratio;
            v.lvl_a = 1.0;
            v.shape = tone * 4.0;
            v.noise_lvl = nz * 0.3;
            v.noise_mul = t60(decay * 0.3, rate);
            v.f1.set(200.0 + tone * 4000.0, 0.7, rate);
            v.o1 = Out::Lp;
            v.click_lvl = snap * 0.6;
            v.click_mul = t60(0.003, rate);
            norm = 0.56;
        }
        SNARE => {
            v.f0 = 185.0 * ratio;
            v.lvl_a = 0.65 * (1.0 - nz * 0.4);
            v.ratio_b = 1.56;
            v.lvl_b = 0.45 * (1.0 - nz * 0.4);
            v.body_mul = t60(decay * 0.6, rate);
            v.noise_lvl = nz;
            v.f1.set(1500.0 + tone * 6000.0, 0.6, rate);
            v.o1 = Out::Bp;
            v.f2.set(700.0, 0.7, rate);
            v.o2 = Out::Hp;
            norm = 0.38;
        }
        CLAP => {
            v.clap = true;
            v.noise_lvl = 1.0;
            v.burst_mul = t60(0.004 + (1.0 - snap) * 0.012, rate);
            v.f1.set(800.0 + tone * 2400.0, 1.2, rate);
            v.o1 = Out::Bp;
            v.f2.set(400.0, 0.7, rate);
            v.o2 = Out::Hp;
            v.click_lvl = 0.0;
            norm = 0.75;
        }
        CLOSED_HAT | OPEN_HAT | CYMBAL => {
            v.n_metal = 6;
            for (k, hz) in METAL.iter().enumerate() {
                v.dt_m[k] = hz * ratio / rate;
                v.ph_m[k] = (k as f32 * 0.37) % 1.0;
            }
            v.metal_lvl = (1.0 - nz) / 3.0;
            v.noise_lvl = nz * 0.6;
            let (bp, hp) = if model == CYMBAL {
                (5000.0 + tone * 5000.0, 3500.0)
            } else {
                (7000.0 + tone * 5000.0, 6000.0 + tone * 2000.0)
            };
            v.f1.set(bp * ratio.sqrt(), 0.9, rate);
            v.o1 = Out::Bp;
            v.f2.set(hp, 0.7, rate);
            v.o2 = Out::Hp;
            v.click_lvl = snap * 0.25;
            norm = if model == CYMBAL { 0.9 } else { 1.6 };
        }
        TOM => {
            v.f0 = 120.0 * ratio;
            v.lvl_a = 1.0;
            v.shape = tone * 1.5;
            v.noise_lvl = nz * 0.5;
            v.noise_mul = t60(decay * 0.25, rate);
            v.f1.set(v.f0 * 3.0, 1.0, rate);
            v.o1 = Out::Bp;
            norm = 0.3;
        }
        RIM => {
            v.f0 = 500.0 * ratio;
            v.lvl_a = 0.6;
            v.ratio_b = 3.3;
            v.lvl_b = 0.4;
            v.noise_lvl = nz * 0.8;
            v.f1.set(2000.0 + tone * 3000.0, 2.0, rate);
            v.o1 = Out::Bp;
            v.click_lvl = snap * 0.8;
            v.click_mul = t60(0.002, rate);
            norm = 0.36;
        }
        COWBELL => {
            v.n_metal = 2;
            v.dt_m[0] = 540.0 * ratio / rate;
            v.dt_m[1] = 800.0 * ratio / rate;
            v.metal_lvl = 0.5;
            v.noise_lvl = nz * 0.3;
            v.f1.set(2640.0 * ratio * (0.7 + 0.6 * tone), 1.5, rate);
            v.o1 = Out::Bp;
            norm = 0.55;
        }
        ZAP => {
            v.f0 = 90.0 * ratio;
            v.lvl_a = 1.0;
            v.shape = tone * 3.0;
            v.noise_lvl = nz * 0.3;
            v.f1.set(3000.0, 0.7, rate);
            v.o1 = Out::Hp;
            norm = 0.25;
        }
        _ => {
            // FM perc: a sine modulated at an inharmonic ratio; Tone sets
            // the index, which falls with the sweep time.
            v.f0 = 220.0 * ratio;
            v.lvl_a = 1.0;
            v.fm_ratio = 1.41;
            v.fm_index = 0.5 + tone * 8.0 + snap * 2.0;
            v.click_lvl = 0.0;
            v.noise_lvl = nz * 0.3;
            v.f1.set(5000.0, 0.7, rate);
            v.o1 = Out::Hp;
            norm = 0.25;
        }
    }
    let a = amp * norm;
    v.gain = a;
    v.gain_l = a * (1.0 - pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
    v.gain_r = a * (1.0 + pan).max(0.0).sqrt() * std::f32::consts::FRAC_1_SQRT_2;
    v
}
