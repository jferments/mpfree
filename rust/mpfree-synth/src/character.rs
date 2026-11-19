//! Character: what makes each voice of an analog poly a little its own
//! (the voice core's part for it; [Synth architecture], "Character").
//!
//! - [`Cards`]: sixteen voice cards, each with fixed differences of both
//!   oscillators' tuning, the cutoff, the envelopes' times, the pulse
//!   width and the level, as unit normals clamped to ±2.5 from a seeded
//!   generator, so every instance and every run is the same.
//! - [`Drift`]: each card's two oscillators wandering slowly, each an
//!   Ornstein-Uhlenbeck walk with unit rms, stepped once a control step.
//! - **Age** (0 to 100) scales both. At 100 the spreads are the standard
//!   deviations below (our reading of a maker's "+/- 5-10 cents variation
//!   per voice", [Vintage analog polysynths]); at 0 an engine leaves them
//!   out and plays as without them, to the bit.
//!
//! Notes take the cards in turn, as on the old instruments, so a repeated
//! chord lands on different cards. Analog has used these since stage 2;
//! Poly since wave 2.
//!
//! [Synth architecture]: ../../../docs/dev/design/synth-architecture.md
//! [Vintage analog polysynths]: ../../../docs/dev/research/vintage-poly.md

use crate::dsp::Rng;

/// Voice cards.
pub const CARDS: usize = 16;

/// Age's spreads at 100, each the standard deviation of a card's fixed
/// offset: tuning in cents, cutoff in octaves (about 6 %), the times as a
/// share (8 %), the pulse width, the level in dB.
pub const AGE_TUNE_CENTS: f32 = 4.0;
pub const AGE_CUT_OCT: f32 = 0.084;
pub const AGE_TIME: f32 = 0.08;
pub const AGE_PW: f32 = 0.025;
pub const AGE_LEVEL_DB: f32 = 0.5;
/// Drift at Age 100: its rms in cents and how slowly it wanders.
pub const DRIFT_CENTS: f32 = 2.0;
pub const DRIFT_TAU_S: f32 = 5.0;

/// A unit normal draw (three uniforms summed, scaled).
pub fn normal(rng: &mut Rng) -> f32 {
    (rng.unit() + rng.unit() + rng.unit() - 1.5) * 2.0
}

/// One unit-normal draw per card, clamped to ±2.5.
pub fn draw_cards(rng: &mut Rng) -> [f32; CARDS] {
    let mut a = [0.0; CARDS];
    for v in &mut a {
        *v = normal(rng).clamp(-2.5, 2.5);
    }
    a
}

/// Each card's fixed offsets, as unit normals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cards {
    /// Oscillator 1's and 2's tuning.
    pub tune1: [f32; CARDS],
    pub tune2: [f32; CARDS],
    pub cut: [f32; CARDS],
    /// The envelopes' times.
    pub time: [f32; CARDS],
    pub pw: [f32; CARDS],
    pub level: [f32; CARDS],
}

impl Cards {
    /// The cards, the same in every instance, and the generator after
    /// them (for the drift's start).
    pub fn draw() -> (Cards, Rng) {
        let mut rng = Rng::new(0xA1A1_0C4D);
        let [tune1, tune2, cut, time, pw, level] = std::array::from_fn(|_| draw_cards(&mut rng));
        (
            Cards {
                tune1,
                tune2,
                cut,
                time,
                pw,
                level,
            },
            rng,
        )
    }

    /// Each card's fixed detune in cents, oscillator 1 and 2, at **Age**
    /// `age` (0 to 100), drift left out. For the Synth tab's picture.
    pub fn cents(age: f32) -> [(f32, f32); CARDS] {
        let (c, _) = Cards::draw();
        let a = AGE_TUNE_CENTS * (age / 100.0).clamp(0.0, 1.0);
        std::array::from_fn(|k| (a * c.tune1[k], a * c.tune2[k]))
    }
}

/// Each card's two oscillators drifting (unit rms), and the walk's step.
#[derive(Clone, Debug)]
pub struct Drift {
    pub d1: [f32; CARDS],
    pub d2: [f32; CARDS],
    keep: f32,
    new: f32,
    rng: Rng,
}

impl Drift {
    /// The walks' starting places, drawn from `start` (the generator
    /// [`Cards::draw`] hands back), standing still until
    /// [`Drift::set_step`].
    pub fn new(start: &mut Rng) -> Drift {
        let d1 = draw_cards(start);
        let d2 = draw_cards(start);
        Drift {
            d1,
            d2,
            keep: 1.0,
            new: 0.0,
            rng: Rng::new(0xD21F_7001),
        }
    }

    /// The step's factors for steps `dt` seconds apart.
    pub fn set_step(&mut self, dt: f32) {
        self.keep = (-dt / DRIFT_TAU_S).exp();
        self.new = (1.0 - self.keep * self.keep).sqrt();
    }

    /// One control step of every card's walks.
    pub fn step(&mut self) {
        for c in 0..CARDS {
            self.d1[c] = self.d1[c] * self.keep + self.new * normal(&mut self.rng);
            self.d2[c] = self.d2[c] * self.keep + self.new * normal(&mut self.rng);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cards_are_the_same_every_time_and_spread_as_labelled() {
        let (a, mut ra) = Cards::draw();
        let (b, _) = Cards::draw();
        assert_eq!(a, b);
        for set in [a.tune1, a.tune2, a.cut, a.time, a.pw, a.level] {
            assert!(set.iter().all(|x| x.abs() <= 2.5));
            let rms = (set.iter().map(|x| x * x).sum::<f32>() / CARDS as f32).sqrt();
            assert!((0.4..1.8).contains(&rms), "{rms}");
        }
        assert_eq!(Cards::cents(0.0), [(0.0, 0.0); CARDS]);
        let c = Cards::cents(100.0);
        assert_eq!(c[3].0, AGE_TUNE_CENTS * a.tune1[3]);
        // The drift starts from the generator after the cards.
        let d = Drift::new(&mut ra);
        assert!(d.d1.iter().chain(&d.d2).all(|x| x.abs() <= 2.5));
    }

    #[test]
    fn the_drift_wanders_slowly_with_unit_rms() {
        let (_, mut r) = Cards::draw();
        let mut d = Drift::new(&mut r);
        // 16 frames a step at 48 kHz.
        d.set_step(16.0 / 48_000.0);
        let (mut sum, mut n, mut step) = (0.0f64, 0usize, 0.0f32);
        for _ in 0..3_000 * 60 {
            let was = d.d1[0];
            d.step();
            step = step.max((d.d1[0] - was).abs());
            sum += f64::from(d.d1[0] * d.d1[0]);
            n += 1;
        }
        let rms = (sum / n as f64).sqrt();
        assert!((0.4..1.8).contains(&rms), "{rms}");
        // One step moves it by a few hundredths of its rms at most.
        assert!(step < 0.15, "{step}");
    }
}
