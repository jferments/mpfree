//! IEEE half precision for the weights at rest: the model file holds its
//! kernels as halves and the networks keep them so (79 MB rather than
//! 158), unpacking one layer at a time ([`crate::gemm::PackedB::view`]).
//!
//! [`to_f16`] rounds to nearest, ties to even, and flushes a result below
//! the smallest normal half to zero, so every weight is a normal half or a
//! zero ([`is_weight`]) and converts back exactly on either path of
//! `mpfree_simd::f16_to_f32`.

/// `x` as a half: nearest, ties to even; below the smallest normal half
/// (6.1e-5) it becomes a zero of its sign, past the largest (65504) an
/// infinity. A NaN stays a NaN.
pub fn to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32;
    let man = b & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        // Below the smallest normal half: it would be subnormal, or it
        // rounds up to the smallest normal.
        if e < -10 {
            return sign;
        }
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = m >> shift;
        let rem = m & ((1 << shift) - 1);
        let mid = 1 << (shift - 1);
        let r = if rem > mid || (rem == mid && half & 1 == 1) {
            half + 1
        } else {
            half
        };
        return if r >= 0x400 { sign | r as u16 } else { sign };
    }
    let half = ((e as u32) << 10) | (man >> 13);
    let rem = man & 0x1fff;
    let r = if rem > 0x1000 || (rem == 0x1000 && half & 1 == 1) {
        half + 1
    } else {
        half
    };
    // A carry out of the mantissa lands in the exponent, as it should;
    // 0x7c00 is infinity.
    sign | r as u16
}

/// Whether `h` is a half a weight may be: a zero or a normal number (no
/// subnormal, infinity or NaN). The model reader refuses any other.
pub fn is_weight(h: u16) -> bool {
    let e = (h >> 10) & 0x1f;
    (e != 0 || h & 0x3ff == 0) && e != 0x1f
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_simd::plain::half;

    /// The reference: f64 arithmetic, nearest even, then the flush.
    fn slow(x: f32) -> u16 {
        if x == 0.0 {
            return if x.is_sign_negative() { 0x8000 } else { 0 };
        }
        let sign = if x < 0.0 { 0x8000u16 } else { 0 };
        let a = f64::from(x).abs();
        if a >= 65520.0 {
            return sign | 0x7c00;
        }
        let e = (a.log2().floor() as i32).max(-14);
        let q = 2f64.powi(e - 10);
        let n = a / q;
        let fl = n.floor();
        let r = if n - fl > 0.5 || (n - fl == 0.5 && fl as u64 % 2 == 1) {
            fl + 1.0
        } else {
            fl
        };
        let v = r * q;
        if v < 2f64.powi(-14) {
            return sign;
        }
        let e = v.log2().floor() as i32;
        let m = (v / 2f64.powi(e) - 1.0) * 1024.0;
        sign | (((e + 15) as u16) << 10) | m as u16
    }

    #[test]
    fn rounding_matches_a_slow_reference() {
        let mut seed = 5u32;
        for i in 0..200_000u32 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            // Bit patterns over the range weights use, ties and the edges.
            let bits = if i < 1000 {
                0x3800_0000 + i * 4096 + (i & 1) * 0x1000
            } else {
                (seed & 0x0fff_ffff) | 0x3000_0000 | (seed & 0x8000_0000)
            };
            let x = f32::from_bits(bits);
            assert_eq!(to_f16(x), slow(x), "{x:e} ({bits:#x})");
        }
        for x in [
            65504.0f32,
            65519.0,
            65520.0,
            6.103_515_6e-5,
            6.1e-5,
            6.0e-5,
            1.0,
            -1.0,
            0.333_333_34,
            1e-8,
            -0.0,
        ] {
            assert_eq!(to_f16(x), slow(x), "{x:e}");
        }
    }

    #[test]
    fn every_weight_half_comes_back_exactly() {
        let mut weights = 0;
        for h in 0..=u16::MAX {
            if is_weight(h) {
                weights += 1;
                assert_eq!(to_f16(half(h)), h, "{h:#06x}");
            }
        }
        // Two zeros and 30 exponents of 1024 normals, each sign.
        assert_eq!(weights, 2 + 2 * 30 * 1024);
        assert!(!is_weight(0x0001) && !is_weight(0x7c00) && !is_weight(0x7e00));
    }

    #[test]
    fn what_to_f16_makes_of_a_finite_float_is_a_weight() {
        let mut seed = 9u32;
        for _ in 0..100_000 {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let x = f32::from_bits(seed);
            if x.is_finite() && x.abs() < 65504.0 {
                assert!(is_weight(to_f16(x)), "{x:e}");
            }
        }
    }
}
