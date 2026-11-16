//! Small DSP building blocks for the sampler voice: interpolation, the
//! AHDSR envelope, a state-variable filter and gain tables. Everything
//! here is allocation-free and safe to call from the audio thread.

use std::f32::consts::{FRAC_PI_2, PI};

/// 4-point, 3rd-order Hermite interpolation between `x1` and `x2`
/// (`t` in 0..1), with `x0` and `x3` as the outer neighbours.
#[inline]
pub fn hermite(x0: f32, x1: f32, x2: f32, x3: f32, t: f32) -> f32 {
    let c1 = 0.5 * (x2 - x0);
    let c2 = x0 - 2.5 * x1 + 2.0 * x2 - 0.5 * x3;
    let c3 = 0.5 * (x3 - x0) + 1.5 * (x1 - x2);
    ((c3 * t + c2) * t + c1) * t + x1
}

/// Linear gain from decibels.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db <= -120.0 {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Equal-power pan: `pan` from -1 (left) to +1 (right), returns (left, right) gains.
/// At centre both are about 0.707 (-3 dB).
pub fn pan_gains(pan: f32) -> (f32, f32) {
    let p = (pan.clamp(-1.0, 1.0) + 1.0) * 0.25 * PI;
    (p.cos(), p.sin())
}

/// Equal-power crossfade weights for `x` in 0..1: (fading out, fading in).
pub fn equal_power(x: f32) -> (f32, f32) {
    let a = x.clamp(0.0, 1.0) * FRAC_PI_2;
    (a.cos(), a.sin())
}

/// Envelope times in milliseconds and a sustain level (0..1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ahdsr {
    pub attack_ms: f32,
    pub hold_ms: f32,
    pub decay_ms: f32,
    pub sustain: f32,
    pub release_ms: f32,
}

impl Ahdsr {
    /// Open: instant attack, full sustain, a short release so nothing clicks.
    pub const OPEN: Ahdsr = Ahdsr {
        attack_ms: 0.0,
        hold_ms: 0.0,
        decay_ms: 0.0,
        sustain: 1.0,
        release_ms: 5.0,
    };
}

impl Default for Ahdsr {
    fn default() -> Self {
        Ahdsr::OPEN
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
    Done,
}

/// Envelope level below which a release counts as finished (about -80 dB).
const FLOOR: f32 = 1.0e-4;

/// A running AHDSR. Attack is linear; decay and release are exponential
/// (one multiply per sample), reaching about -80 dB of their distance in
/// the given time.
#[derive(Clone, Copy, Debug)]
pub struct Envelope {
    stage: Stage,
    level: f32,
    attack_step: f32,
    hold_left: u32,
    decay_coef: f32,
    sustain: f32,
    release_coef: f32,
}

fn exp_coef(ms: f32, rate: f32) -> f32 {
    let n = (ms.max(0.0) * 0.001 * rate).max(1.0);
    // After n samples the remaining distance is FLOOR of the start.
    FLOOR.powf(1.0 / n)
}

impl Envelope {
    pub fn idle() -> Envelope {
        Envelope {
            stage: Stage::Done,
            level: 0.0,
            attack_step: 1.0,
            hold_left: 0,
            decay_coef: 0.0,
            sustain: 1.0,
            release_coef: 0.0,
        }
    }

    pub fn start(p: &Ahdsr, rate: f32) -> Envelope {
        let attack_n = (p.attack_ms.max(0.0) * 0.001 * rate).max(0.0);
        let (stage, level, attack_step) = if attack_n < 1.0 {
            (Stage::Hold, 1.0, 1.0)
        } else {
            (Stage::Attack, 0.0, 1.0 / attack_n)
        };
        Envelope {
            stage,
            level,
            attack_step,
            hold_left: (p.hold_ms.max(0.0) * 0.001 * rate) as u32,
            decay_coef: exp_coef(p.decay_ms, rate),
            sustain: p.sustain.clamp(0.0, 1.0),
            release_coef: exp_coef(p.release_ms, rate),
        }
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Done {
            self.stage = Stage::Release;
        }
    }

    pub fn is_done(&self) -> bool {
        self.stage == Stage::Done
    }
    pub fn is_releasing(&self) -> bool {
        self.stage == Stage::Release
    }
    /// Holding a level that [`Envelope::next`] leaves as it is until a
    /// release: its sustain, above zero. Its level is then the gain every
    /// frame gets.
    pub fn is_steady(&self) -> bool {
        self.stage == Stage::Sustain && self.sustain > 0.0
    }
    pub fn level(&self) -> f32 {
        self.level
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.level += self.attack_step;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = Stage::Hold;
                }
            }
            Stage::Hold => {
                if self.hold_left == 0 {
                    self.stage = Stage::Decay;
                } else {
                    self.hold_left -= 1;
                }
            }
            Stage::Decay => {
                self.level = self.sustain + (self.level - self.sustain) * self.decay_coef;
                if (self.level - self.sustain).abs() < FLOOR {
                    self.level = self.sustain;
                    self.stage = Stage::Sustain;
                }
            }
            Stage::Sustain => {
                if self.sustain <= 0.0 {
                    self.level = 0.0;
                    self.stage = Stage::Done;
                }
            }
            Stage::Release => {
                self.level *= self.release_coef;
                if self.level < FLOOR {
                    self.level = 0.0;
                    self.stage = Stage::Done;
                }
            }
            Stage::Done => self.level = 0.0,
        }
        self.level
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Off,
    LowPass,
    HighPass,
    BandPass,
}

/// A filter setting. `poles` is 2 or 4 (4 = two 2-pole stages in series).
/// `env_octaves` is how far the filter envelope moves the cutoff at full level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    pub kind: FilterKind,
    pub poles: u8,
    pub cutoff_hz: f32,
    /// 0 = no resonance, 1 = close to self-oscillation.
    pub resonance: f32,
    pub env_octaves: f32,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            kind: FilterKind::Off,
            poles: 2,
            cutoff_hz: 20_000.0,
            resonance: 0.0,
            env_octaves: 0.0,
        }
    }
}

/// One 2-pole topology-preserving state-variable filter stage per channel
/// (after Andrew Simper's published "SVF linear trap" design).
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf {
    ic1: [f32; 2],
    ic2: [f32; 2],
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl Svf {
    pub fn set(&mut self, cutoff_hz: f32, resonance: f32, rate: f32) {
        let fc = cutoff_hz.clamp(10.0, rate * 0.49);
        let g = (PI * fc / rate).tan();
        // resonance 0..1 maps to Q 0.5..~20 via k = 1/Q.
        self.k = 2.0 - 1.95 * resonance.clamp(0.0, 1.0);
        self.a1 = 1.0 / (1.0 + g * (g + self.k));
        self.a2 = g * self.a1;
        self.a3 = g * self.a2;
    }

    #[inline]
    pub fn process(&mut self, ch: usize, x: f32, kind: FilterKind) -> f32 {
        let v3 = x - self.ic2[ch];
        let v1 = self.a1 * self.ic1[ch] + self.a2 * v3;
        let v2 = self.ic2[ch] + self.a2 * self.ic1[ch] + self.a3 * v3;
        self.ic1[ch] = 2.0 * v1 - self.ic1[ch];
        self.ic2[ch] = 2.0 * v2 - self.ic2[ch];
        match kind {
            FilterKind::Off => x,
            FilterKind::LowPass => v2,
            FilterKind::BandPass => v1,
            FilterKind::HighPass => x - self.k * v1 - v2,
        }
    }
}

/// A small deterministic random generator (xorshift32) for layer choice
/// and, later, grains. Seeded, so offline renders repeat exactly.
#[derive(Clone, Copy, Debug)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Rng {
        Rng(if seed == 0 { 0x9E37_79B9 } else { seed })
    }
    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// 0..n (n > 0).
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_hits_the_points_and_follows_a_line() {
        assert_eq!(hermite(0.0, 1.0, 2.0, 3.0, 0.0), 1.0);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 1.0) - 2.0).abs() < 1e-6);
        assert!((hermite(0.0, 1.0, 2.0, 3.0, 0.5) - 1.5).abs() < 1e-6);
    }

    #[test]
    fn pan_and_crossfade_keep_power() {
        for p in [-1.0f32, -0.3, 0.0, 0.6, 1.0] {
            let (l, r) = pan_gains(p);
            assert!((l * l + r * r - 1.0).abs() < 1e-5);
        }
        let (l, r) = pan_gains(-1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6);
        for x in [0.0f32, 0.25, 0.5, 1.0] {
            let (a, b) = equal_power(x);
            assert!((a * a + b * b - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn envelope_runs_its_stages() {
        let rate = 1000.0; // 1 sample = 1 ms
        let p = Ahdsr {
            attack_ms: 10.0,
            hold_ms: 5.0,
            decay_ms: 50.0,
            sustain: 0.5,
            release_ms: 20.0,
        };
        let mut e = Envelope::start(&p, rate);
        let mut peak_at = 0;
        for i in 0..10 {
            if e.next() >= 1.0 {
                peak_at = i;
                break;
            }
        }
        assert_eq!(peak_at, 9, "linear attack reaches 1 after 10 samples");
        for _ in 0..200 {
            e.next();
        }
        assert!((e.level() - 0.5).abs() < 1e-3, "settles at sustain");
        e.release();
        let mut n = 0;
        while !e.is_done() {
            e.next();
            n += 1;
            assert!(n < 100);
        }
        assert!(
            (19..=21).contains(&n),
            "release takes about its time, took {n}"
        );
    }

    #[test]
    fn zero_attack_starts_at_full_level() {
        let mut e = Envelope::start(&Ahdsr::OPEN, 44_100.0);
        assert_eq!(e.next(), 1.0);
    }

    #[test]
    fn low_pass_passes_dc_and_cuts_highs() {
        let rate = 44_100.0;
        let mut f = Svf::default();
        f.set(1000.0, 0.0, rate);
        let mut y = 0.0;
        for _ in 0..4000 {
            y = f.process(0, 1.0, FilterKind::LowPass);
        }
        assert!((y - 1.0).abs() < 1e-3, "DC passes: {y}");
        let mut f = Svf::default();
        f.set(200.0, 0.0, rate);
        let mut peak = 0f32;
        for i in 0..4000 {
            let x = if i % 2 == 0 { 1.0 } else { -1.0 }; // Nyquist
            let y = f.process(0, x, FilterKind::LowPass);
            if i > 2000 {
                peak = peak.max(y.abs());
            }
        }
        assert!(peak < 1e-3, "Nyquist is cut: {peak}");
    }

    #[test]
    fn rng_is_repeatable() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..10 {
            assert_eq!(a.next_u32(), b.next_u32());
        }
    }
}
