//! Silence split, for sample packs and one-shot chains recorded as one take,
//! and for trimming the silence around a recording.

use crate::AnalysisError;

/// Settings for [`sound_spans`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SilenceSettings {
    /// Anything quieter than this (peak, dBFS) is silence.
    pub threshold_db: f32,
    /// Gaps shorter than this don't split a sound (so a stutter or a short
    /// pause inside one sample stays in one piece).
    pub min_gap_ms: f32,
    /// Sounds shorter than this are dropped as clicks.
    pub min_sound_ms: f32,
    /// How much earlier than the first loud sample a sound starts, so a soft
    /// attack isn't cut.
    pub pre_roll_ms: f32,
}

impl Default for SilenceSettings {
    fn default() -> Self { SilenceSettings { threshold_db: -48.0, min_gap_ms: 120.0, min_sound_ms: 15.0, pre_roll_ms: 2.0 } }
}

/// The sounding parts of interleaved audio, as (start, end) frames, end
/// exclusive, in order and never overlapping.
pub fn sound_spans(data: &[f32], channels: usize, rate: u32, s: SilenceSettings) -> Result<Vec<(u64, u64)>, AnalysisError> {
    if channels == 0 || channels > 2 { return Err(AnalysisError::BadInput("channels")); }
    if !data.len().is_multiple_of(channels) { return Err(AnalysisError::BadInput("length")); }
    if rate == 0 || !s.threshold_db.is_finite() { return Err(AnalysisError::BadInput("settings")); }
    let frames = data.len() / channels;
    let level = 10f32.powf(s.threshold_db / 20.0);
    let ms = |m: f32| (m.max(0.0) / 1000.0 * rate as f32) as usize;
    let loud = |f: usize| data[f * channels..(f + 1) * channels].iter().any(|x| x.abs() > level);
    // Runs of loud frames, joined across short gaps.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut f = 0;
    while f < frames {
        if !loud(f) { f += 1; continue; }
        let start = f;
        let mut last = f;
        while f < frames {
            if loud(f) { last = f; } else if f - last > ms(s.min_gap_ms) { break; }
            f += 1;
        }
        spans.push((start, last + 1));
    }
    spans.retain(|&(a, b)| b - a >= ms(s.min_sound_ms).max(1));
    let mut out = Vec::with_capacity(spans.len());
    let mut floor = 0usize;
    for (a, b) in spans {
        let a = a.saturating_sub(ms(s.pre_roll_ms)).max(floor);
        out.push((a as u64, b as u64));
        floor = b;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn burst(out: &mut [f32], at: usize, len: usize, amp: f32) {
        for i in 0..len { out[at + i] += amp * (i as f32 * 0.3).sin() * (1.0 - i as f32 / len as f32); }
    }

    #[test]
    fn splits_a_chain_of_one_shots() {
        let rate = 44_100;
        let mut x = vec![0.0f32; rate as usize * 3];
        burst(&mut x, 1_000, 8_000, 0.8);
        burst(&mut x, 30_000, 4_000, 0.5);
        // A stutter 50 ms after the second sound: same sound.
        burst(&mut x, 34_000 + 2_205, 2_000, 0.5);
        burst(&mut x, 90_000, 20_000, 0.3);
        burst(&mut x, 120_000, 20, 0.9); // a click: dropped
        let spans = sound_spans(&x, 1, rate, SilenceSettings::default()).unwrap();
        assert_eq!(spans.len(), 3, "{spans:?}");
        let pre = 88;
        assert!(spans[0].0 <= 1_000 && spans[0].0 >= 1_000 - pre);
        assert!(spans[1].0 <= 30_000 && spans[1].1 > 36_205);
        assert!(spans[2].0 <= 90_000 && spans[2].1 <= 110_000);
        for w in spans.windows(2) { assert!(w[0].1 <= w[1].0); }
    }

    #[test]
    fn stereo_counts_either_side_and_silence_has_no_spans() {
        let mut x = vec![0.0f32; 20_000];
        for i in 0..2_000 { x[2 * (3_000 + i) + 1] = 0.5; } // right only
        let spans = sound_spans(&x, 2, 44_100, SilenceSettings::default()).unwrap();
        assert_eq!(spans.len(), 1);
        assert!(sound_spans(&vec![0.0; 1000], 1, 44_100, SilenceSettings::default()).unwrap().is_empty());
        assert!(sound_spans(&[0.0; 3], 2, 44_100, SilenceSettings::default()).is_err());
    }
}
