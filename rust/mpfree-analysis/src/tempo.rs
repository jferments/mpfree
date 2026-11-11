//! Tempo estimation, offered to the user, never forced.
//!
//! The onset strength curve (spectral flux, one value per hop) has its local
//! mean removed, then its autocorrelation is searched over 60-200 BPM. Each
//! candidate period is scored by the autocorrelation at that period and its
//! multiples, which favours the period the whole groove repeats at, times a
//! gentle preference around 120 BPM that settles half-time and double-time
//! ties. The winner is refined to a fraction of a hop.
//!
//! For a loop cut to length, [`loop_tempo`] is better still: the loop holds
//! a whole number of beats, so its length gives the tempo exactly once the
//! estimate has picked the beat count.

use crate::onsets::{OnsetAnalysis, HOP};

pub const MIN_BPM: f32 = 60.0;
pub const MAX_BPM: f32 = 200.0;
/// Preferred tempo for settling octave ties, and how wide the preference is
/// (in octaves).
const PREFERRED_BPM: f32 = 120.0;
const PREFERENCE_OCTAVES: f32 = 1.2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tempo {
    pub bpm: f32,
    /// 0 to 1: how strongly the onsets repeat at this period.
    pub confidence: f32,
}

/// Estimates the tempo from an onset analysis. None when there is too little
/// rhythm to say (fewer than about four beats, or no repetition).
pub fn estimate_tempo(o: &OnsetAnalysis) -> Option<Tempo> {
    let flux = o.flux();
    let hop_s = HOP as f32 / o.rate() as f32;
    let lag_of = |bpm: f32| 60.0 / bpm / hop_s;
    let max_lag = lag_of(MIN_BPM).ceil() as usize;
    let min_lag = lag_of(MAX_BPM).floor() as usize;
    if flux.len() < 2 * max_lag {
        return None;
    }
    // Novelty: flux above its local mean (±16 hops), half-wave rectified.
    let n = flux.len();
    let mut prefix = vec![0.0f64; n + 1];
    for (i, f) in flux.iter().enumerate() {
        prefix[i + 1] = prefix[i] + f64::from(*f);
    }
    let nov: Vec<f32> = (0..n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(16), (i + 17).min(n));
            let mean = (prefix[b] - prefix[a]) / (b - a) as f64;
            (f64::from(flux[i]) - mean).max(0.0) as f32
        })
        .collect();
    // Unbiased autocorrelation at a fractional lag (linear interpolation).
    let ac = |lag: f32| -> f32 {
        let l0 = lag.floor() as usize;
        let t = lag - l0 as f32;
        if l0 + 1 >= n {
            return 0.0;
        }
        let mut s = 0.0f64;
        for i in 0..n - l0 - 1 {
            let other = nov[i + l0] * (1.0 - t) + nov[i + l0 + 1] * t;
            s += f64::from(nov[i] * other);
        }
        (s / (n - l0 - 1) as f64) as f32
    };
    let zero = ac(0.0);
    if zero <= 0.0 {
        return None;
    }
    // A period scores by itself and its multiples within the audio.
    let score = |lag: f32| -> f32 {
        let mut s = 0.0;
        let mut w = 1.0;
        let mut k = 1.0;
        while k * lag < (n / 2) as f32 && k <= 4.0 {
            s += w * ac(k * lag);
            w *= 0.5;
            k += 1.0;
        }
        s
    };
    let pref =
        |bpm: f32| (-0.5 * ((bpm / PREFERRED_BPM).log2() / PREFERENCE_OCTAVES).powi(2)).exp();
    let mut best = (0.0f32, 0usize);
    for lag in min_lag.max(1)..=max_lag {
        let bpm = 60.0 / (lag as f32 * hop_s);
        if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            continue;
        }
        let v = score(lag as f32) * pref(bpm);
        if v > best.0 {
            best = (v, lag);
        }
    }
    if best.0 <= 0.0 {
        return None;
    }
    // Refine to 1/50 of a hop.
    let mut fine = (0.0f32, best.1 as f32);
    for step in -50..=50 {
        let lag = best.1 as f32 + step as f32 / 50.0;
        let v = score(lag);
        if v > fine.0 {
            fine = (v, lag);
        }
    }
    let bpm = 60.0 / (fine.1 * hop_s);
    let confidence = (ac(fine.1) / zero).clamp(0.0, 1.0);
    Some(Tempo { bpm, confidence })
}

/// The exact tempo of a loop of `frames` that holds a whole number of beats,
/// choosing the beat count whose tempo is closest to `estimate` (or to 120
/// BPM without one). None if no common beat count gives 60-200 BPM.
pub fn loop_tempo(frames: u64, rate: u32, estimate: Option<f32>) -> Option<f32> {
    if frames == 0 || rate == 0 {
        return None;
    }
    let seconds = frames as f64 / f64::from(rate);
    let target = f64::from(estimate.unwrap_or(PREFERRED_BPM));
    [1u32, 2, 3, 4, 6, 8, 12, 16, 24, 32, 48, 64, 96, 128]
        .iter()
        .map(|&beats| 60.0 * f64::from(beats) / seconds)
        .filter(|bpm| (f64::from(MIN_BPM)..=f64::from(MAX_BPM)).contains(bpm))
        .min_by(|a, b| (a / target).ln().abs().total_cmp(&(b / target).ln().abs()))
        .map(|b| b as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onsets::tests::beat;

    #[test]
    fn finds_common_tempos() {
        for bpm in [85.0f32, 92.0, 100.0, 120.0, 128.0, 140.0, 150.0] {
            let (audio, _) = beat(bpm, 8, 44_100.0);
            let o = OnsetAnalysis::analyse(&audio, 1, 44_100).unwrap();
            let t = estimate_tempo(&o).unwrap();
            assert!(
                (t.bpm - bpm).abs() < 0.5,
                "{bpm} BPM estimated as {}",
                t.bpm
            );
            assert!(t.confidence > 0.2, "{bpm}: confidence {}", t.confidence);
        }
    }

    #[test]
    fn too_short_or_silent_says_nothing() {
        let o = OnsetAnalysis::analyse(&vec![0.0; 44_100 * 4], 1, 44_100).unwrap();
        assert_eq!(estimate_tempo(&o), None);
        let o = OnsetAnalysis::analyse(&vec![0.0; 10_000], 1, 44_100).unwrap();
        assert_eq!(estimate_tempo(&o), None);
    }

    #[test]
    fn loop_length_gives_the_exact_tempo() {
        // Two bars at 93.5 BPM, cut exactly.
        let frames = (8.0 * 60.0 / 93.5 * 44_100.0f64).round() as u64;
        let t = loop_tempo(frames, 44_100, Some(95.0)).unwrap();
        assert!((t - 93.5).abs() < 0.01, "{t}");
        // A half-time guess still lands on a beat count near it.
        let t = loop_tempo(frames, 44_100, Some(47.0)).unwrap();
        assert!((t - 70.125).abs() < 0.01, "{t}");
        assert_eq!(loop_tempo(0, 44_100, None), None);
    }
}
