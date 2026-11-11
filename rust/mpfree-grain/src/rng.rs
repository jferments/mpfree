//! A small seeded generator (SplitMix64). Not for cryptography: it exists so
//! grain clouds are random to the ear and identical from run to run.

/// Seeded pseudo-random numbers. Copying it copies the sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self { Rng(seed) }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    #[inline]
    pub fn unit(&mut self) -> f32 { (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32 }

    /// Uniform in [-1, 1).
    #[inline]
    pub fn bipolar(&mut self) -> f32 { self.unit() * 2.0 - 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeatable_and_in_range() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        let mut sum = 0.0;
        for _ in 0..10_000 {
            let x = a.unit();
            assert_eq!(x, b.unit());
            assert!((0.0..1.0).contains(&x));
            sum += x;
        }
        assert!((sum / 10_000.0 - 0.5).abs() < 0.02);
        assert_ne!(Rng::new(1).next_u64(), Rng::new(2).next_u64());
    }
}
