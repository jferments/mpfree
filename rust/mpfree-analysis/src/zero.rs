//! Zero crossings, for snapping markers so slices start and end without a
//! click. Works on a mono mix (see [`crate::mono`]) or one channel: floats,
//! or any [`Samples`] of one channel, such as a sound mixed to mono as it
//! is read, with no copy of it made.

use crate::Samples;

/// True when the signal crosses or touches zero between `i - 1` and `i`.
fn crosses<S: Samples + ?Sized>(x: &S, i: usize) -> bool {
    let (a, b) = (x.at(i - 1), x.at(i));
    b == 0.0 || (a < 0.0) != (b < 0.0)
}

/// Of the two samples around a crossing, the one closer to zero.
fn quieter<S: Samples + ?Sized>(x: &S, i: usize) -> usize {
    if x.at(i - 1).abs() < x.at(i).abs() {
        i - 1
    } else {
        i
    }
}

/// The zero crossing nearest to `at`, no more than `radius` frames away.
/// Ties go to the earlier one.
pub fn nearest_zero_crossing<S: Samples + ?Sized>(
    x: &S,
    at: usize,
    radius: usize,
) -> Option<usize> {
    if x.len() < 2 {
        return None;
    }
    let at = at.min(x.len() - 1);
    for d in 0..=radius {
        if at >= d && at - d >= 1 && crosses(x, at - d) {
            return Some(quieter(x, at - d));
        }
        let i = at + d;
        if d > 0 && i < x.len() && i >= 1 && crosses(x, i) {
            return Some(quieter(x, i));
        }
    }
    None
}

/// The last zero crossing at or before `at`, no more than `radius` frames back.
pub fn previous_zero_crossing<S: Samples + ?Sized>(
    x: &S,
    at: usize,
    radius: usize,
) -> Option<usize> {
    if x.len() < 2 {
        return None;
    }
    let at = at.min(x.len() - 1);
    (at.saturating_sub(radius).max(1)..=at)
        .rev()
        .find(|&i| crosses(x, i))
        .map(|i| quieter(x, i))
}

/// The next zero crossing at or after `at`, no more than `ahead` frames on.
pub fn next_zero_crossing<S: Samples + ?Sized>(x: &S, at: usize, ahead: usize) -> Option<usize> {
    let end = x.len().min(at.saturating_add(ahead).saturating_add(1));
    (at.max(1)..end)
        .find(|&i| crosses(x, i))
        .map(|i| quieter(x, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_nearest_crossing() {
        // A sine with period 100: crossings at 0, 50, 100, ...
        let x: Vec<f32> = (0..400)
            .map(|i| (std::f32::consts::TAU * (i as f32 + 0.3) / 100.0).sin())
            .collect();
        assert_eq!(nearest_zero_crossing(&x, 140, 30), Some(150));
        assert_eq!(nearest_zero_crossing(&x, 160, 30), Some(150));
        assert_eq!(nearest_zero_crossing(&x, 125, 10), None);
        assert_eq!(previous_zero_crossing(&x, 149, 100), Some(100));
        assert_eq!(previous_zero_crossing(&x, 220, 10), None);
        assert_eq!(nearest_zero_crossing(&[0.5f32][..], 0, 5), None);
        assert_eq!(previous_zero_crossing(&x, 10_000, 60), Some(350));
        assert_eq!(next_zero_crossing(&x, 140, 30), Some(150));
        assert_eq!(next_zero_crossing(&x, 151, 30), None);
        assert_eq!(next_zero_crossing(&x, 151, 60), Some(200));
    }
}
