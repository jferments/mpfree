//! Chance from stable names (contract 2 of the design).
//!
//! Every random choice is drawn from dice named by the seed, the part, the
//! phrase, the job, the bar and the try, never from one shared stream. So
//! trying bar 6 again can't change bars 7 to 16 because an earlier step
//! happened to roll more dice, and the same names always roll the same
//! numbers on the same engine version.

/// The job a roll is for. Adding a job at the end never changes the rolls
/// of the others; reordering would, so new jobs go last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Job {
    Plan = 1,
    Harmony = 2,
    Rhythm = 3,
    Pitch = 4,
    Playing = 5,
    Groove = 6,
    Drift = 7,
    Bass = 8,
    Chords = 9,
    Drums = 10,
    Variation = 11,
}

/// The names a roll is drawn from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DiceName {
    pub seed: u64,
    pub part: u16,
    pub phrase: u16,
    pub job: Job,
    pub bar: u16,
    /// Try again's count for this bar (0 the first time).
    pub attempt: u16,
}

impl DiceName {
    pub fn dice(self) -> Dice {
        let mut h = mix(self.seed ^ 0x6d70_6672_6565_2d63);
        for v in [
            u64::from(self.part),
            u64::from(self.phrase),
            self.job as u64,
            u64::from(self.bar),
            u64::from(self.attempt),
        ] {
            h = mix(h ^ v.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        }
        Dice { state: h }
    }
}

/// SplitMix64's finaliser: every bit of the input moves every bit of the
/// output.
const fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A stream of rolls (SplitMix64). Small, `Copy`, no allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dice {
    state: u64,
}

impl Dice {
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        mix(self.state)
    }

    /// A number in [0, 1).
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// True with chance `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    /// A number in 0..n (0 when n is 0).
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            ((self.next_u64() as u128 * n as u128) >> 64) as usize
        }
    }

    /// An index picked by weight; weights at or below 0 never win. `None`
    /// when no weight is above 0.
    pub fn pick(&mut self, weights: &[f64]) -> Option<usize> {
        let total: f64 = weights.iter().filter(|w| **w > 0.0).sum();
        if total <= 0.0 || !total.is_finite() {
            return None;
        }
        let mut x = self.unit() * total;
        let mut last = None;
        for (i, &w) in weights.iter().enumerate() {
            if w > 0.0 {
                last = Some(i);
                if x < w {
                    return Some(i);
                }
                x -= w;
            }
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(bar: u16, attempt: u16) -> DiceName {
        DiceName {
            seed: 42,
            part: 1,
            phrase: 0,
            job: Job::Pitch,
            bar,
            attempt,
        }
    }

    #[test]
    fn the_same_names_roll_the_same_numbers() {
        let roll = |n: DiceName| {
            let mut d = n.dice();
            (0..8).map(|_| d.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(roll(name(6, 0)), roll(name(6, 0)));
    }

    #[test]
    fn every_name_matters() {
        let base = name(6, 0);
        let first = |n: DiceName| n.dice().next_u64();
        let v = first(base);
        assert_ne!(v, first(DiceName { seed: 43, ..base }));
        assert_ne!(v, first(DiceName { part: 2, ..base }));
        assert_ne!(v, first(DiceName { phrase: 1, ..base }));
        assert_ne!(
            v,
            first(DiceName {
                job: Job::Rhythm,
                ..base
            })
        );
        assert_ne!(v, first(DiceName { bar: 7, ..base }));
        assert_ne!(v, first(DiceName { attempt: 1, ..base }));
    }

    #[test]
    fn rolling_more_for_one_bar_never_moves_another() {
        // However many dice bar 5 rolls, bar 6's are its own.
        let bar6 = name(6, 0).dice().next_u64();
        for rolls in [0, 1, 10, 1000] {
            let mut d5 = name(5, 0).dice();
            for _ in 0..rolls {
                d5.next_u64();
            }
            assert_eq!(name(6, 0).dice().next_u64(), bar6);
        }
    }

    #[test]
    fn rolls_are_spread_and_picks_follow_weights() {
        let mut d = name(1, 0).dice();
        let mut counts = [0u32; 4];
        for _ in 0..40_000 {
            counts[d.pick(&[1.0, 0.0, 2.0, 1.0]).unwrap()] += 1;
        }
        assert_eq!(counts[1], 0);
        let share = f64::from(counts[2]) / 40_000.0;
        assert!((share - 0.5).abs() < 0.02, "{share}");
        assert_eq!(d.pick(&[0.0, -1.0]), None);
        assert!((0..1000).all(|_| d.below(7) < 7));
        assert_eq!(d.below(0), 0);
        let u = d.unit();
        assert!((0.0..1.0).contains(&u));
    }
}
