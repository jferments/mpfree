//! MPFree's first Rust-owned, device-independent audio core.
//!
//! The audio path uses fixed-size, precomputed click wavetables. No ALSA,
//! network, USB, filesystem I/O, locks, logging, or heap allocation in render().
//! Configuration and reset MUST NOT be called concurrently with rendering.
#![forbid(unsafe_code)]

use std::f64::consts::PI;

const MAX_CLICK_SAMPLES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClickConfig {
    pub sample_rate: u32,
    pub bpm: f64,
    pub beats_per_bar: u32,
}

impl Default for ClickConfig {
    fn default() -> Self {
        Self {
            sample_rate: 44_100,
            bpm: 120.0,
            beats_per_bar: 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderError {
    ChannelLengthMismatch,
}

pub struct ClickEngine {
    config: ClickConfig,
    normal: [f32; MAX_CLICK_SAMPLES],
    accent: [f32; MAX_CLICK_SAMPLES],
    click_length: usize,
    click_position: usize,
    accented: bool,
    frame: u64,
    beat: u64,
    next_beat_frame: u64,
}

impl Default for ClickEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ClickEngine {
    pub fn new() -> Self {
        let mut engine = Self {
            config: ClickConfig::default(),
            normal: [0.0; MAX_CLICK_SAMPLES],
            accent: [0.0; MAX_CLICK_SAMPLES],
            click_length: 0,
            click_position: 0,
            accented: false,
            frame: 0,
            beat: 0,
            next_beat_frame: 0,
        };
        assert!(engine.configure(ClickConfig::default()));
        engine
    }

    /// Non-real-time configuration; invalid input leaves the current state unchanged.
    pub fn configure(&mut self, candidate: ClickConfig) -> bool {
        if !(8_000..=192_000).contains(&candidate.sample_rate)
            || !candidate.bpm.is_finite()
            || !(20.0..=400.0).contains(&candidate.bpm)
            || !(1..=16).contains(&candidate.beats_per_bar)
        {
            return false;
        }
        self.config = candidate;
        self.click_length =
            ((candidate.sample_rate as f64 * 0.0035).round() as usize).clamp(8, MAX_CLICK_SAMPLES);
        // Calculate sine and envelope outside the real-time render path.
        for i in 0..self.click_length {
            let t = i as f64 / candidate.sample_rate as f64;
            let decay = (-7.0 * i as f64 / self.click_length as f64).exp();
            self.normal[i] = (0.30 * decay * (2.0 * PI * 1550.0 * t).sin()) as f32;
            self.accent[i] = (0.47 * decay * (2.0 * PI * 2250.0 * t).sin()) as f32;
        }
        self.reset();
        true
    }

    /// Non-realtime transport reset. Never call concurrently with render().
    pub fn reset(&mut self) {
        self.frame = 0;
        self.beat = 0;
        self.next_beat_frame = 0;
        self.click_position = self.click_length;
        self.accented = false;
    }

    /// Processes two PLANAR floating-point channels without heap allocation.
    /// On invalid lengths, refuses processing without advancing the transport.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), RenderError> {
        if left.len() != right.len() {
            return Err(RenderError::ChannelLengthMismatch);
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            if self.frame == self.next_beat_frame {
                self.accented = self
                    .beat
                    .is_multiple_of(u64::from(self.config.beats_per_bar));
                self.click_position = 0;
                self.beat = self.beat.saturating_add(1);

                // Absolute time from beat index, not repeatedly rounded periods.
                let next = (self.beat as f64 * 60.0 * self.config.sample_rate as f64
                    / self.config.bpm)
                    .round() as u64;
                self.next_beat_frame = next.max(self.frame.saturating_add(1));
            }
            let value = if self.click_position < self.click_length {
                let value = if self.accented {
                    self.accent[self.click_position]
                } else {
                    self.normal[self.click_position]
                };
                self.click_position += 1;
                value
            } else {
                0.0
            };
            *l = value;
            *r = value;
            self.frame = self.frame.saturating_add(1);
        }
        Ok(())
    }

    pub fn rendered_frames(&self) -> u64 {
        self.frame
    }

    pub fn beats_emitted(&self) -> u64 {
        self.beat
    }

    pub fn configuration(&self) -> ClickConfig {
        self.config
    }
}

/// Signed PCM conversion matching the C++ M0 reference:
/// non-finite -> silence, hard saturation, nearest integer for finite samples.
pub fn float_to_s32(sample: f32) -> i32 {
    if !sample.is_finite() {
        0
    } else if sample >= 1.0 {
        i32::MAX
    } else if sample <= -1.0 {
        i32::MIN
    } else {
        ((sample as f64 * i32::MAX as f64).round()) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tempo_is_sample_accurate() {
        let mut engine = ClickEngine::new();
        let mut left = vec![0.0; 44_100];
        let mut right = vec![0.0; 44_100];
        engine.render(&mut left, &mut right).unwrap();
        assert_eq!(engine.rendered_frames(), 44_100);
        assert_eq!(engine.beats_emitted(), 2);
        engine.render(&mut left[..1], &mut right[..1]).unwrap();
        assert_eq!(engine.beats_emitted(), 3);
        engine.reset();
        assert_eq!(engine.rendered_frames(), 0);
        assert_eq!(engine.beats_emitted(), 0);
        assert!(engine.configure(ClickConfig {
            bpm: 109.0,
            ..ClickConfig::default()
        }));
        let mut a = [0.0; 128];
        let mut b = [0.0; 128];
        let mut remaining = 7 * 44_100;
        while remaining > 0 {
            let n = remaining.min(128);
            engine.render(&mut a[..n], &mut b[..n]).unwrap();
            remaining -= n;
        }
        assert_eq!(engine.rendered_frames(), 7 * 44_100);
        assert_eq!(engine.beats_emitted(), 13);
    }

    #[test]
    fn output_is_invariant_to_render_block_partitioning() {
        let config = ClickConfig {
            bpm: 137.2,
            beats_per_bar: 3,
            ..ClickConfig::default()
        };
        let mut full = ClickEngine::new();
        let mut chunked = ClickEngine::new();
        assert!(full.configure(config) && chunked.configure(config));
        const N: usize = 44_100 * 2 + 13;
        let mut left = vec![0.0; N];
        let mut right = vec![0.0; N];
        let mut chunk_left = vec![0.0; N];
        let mut chunk_right = vec![0.0; N];
        full.render(&mut left, &mut right).unwrap();
        for (a, b) in chunk_left.chunks_mut(128).zip(chunk_right.chunks_mut(128)) {
            chunked.render(a, b).unwrap();
        }
        assert_eq!(full.rendered_frames(), chunked.rendered_frames());
        assert_eq!(full.beats_emitted(), chunked.beats_emitted());
        assert_eq!(left, chunk_left);
        assert_eq!(right, chunk_right);
        assert_eq!(left, right);
        assert!(left.iter().any(|&v| v != 0.0));
        assert!(left.iter().all(|v| v.is_finite() && v.abs() < 0.5));
    }

    #[test]
    fn rejects_invalid_configuration_without_mutation() {
        let mut engine = ClickEngine::new();
        for invalid in [
            ClickConfig {
                sample_rate: 0,
                ..ClickConfig::default()
            },
            ClickConfig {
                bpm: 0.0,
                ..ClickConfig::default()
            },
            ClickConfig {
                bpm: f64::INFINITY,
                ..ClickConfig::default()
            },
            ClickConfig {
                bpm: f64::NAN,
                ..ClickConfig::default()
            },
            ClickConfig {
                beats_per_bar: 0,
                ..ClickConfig::default()
            },
        ] {
            assert!(!engine.configure(invalid));
            assert_eq!(engine.configuration(), ClickConfig::default());
        }
    }

    #[test]
    fn rejects_mismatched_channels_without_advancing() {
        let mut engine = ClickEngine::new();
        assert_eq!(
            engine.render(&mut [0.0; 1], &mut [0.0; 2]),
            Err(RenderError::ChannelLengthMismatch)
        );
        assert_eq!(engine.rendered_frames(), 0);
    }

    #[test]
    fn s32_saturation_and_nonfinite_guard() {
        assert_eq!(float_to_s32(0.0), 0);
        assert_eq!(float_to_s32(1.0), i32::MAX);
        assert_eq!(float_to_s32(-1.0), i32::MIN);
        assert_eq!(float_to_s32(2.0), i32::MAX);
        assert_eq!(float_to_s32(-2.0), i32::MIN);
        assert_eq!(float_to_s32(f32::NAN), 0);
        assert_eq!(float_to_s32(f32::INFINITY), 0);
        assert!(float_to_s32(0.5) > 0);
        assert!(float_to_s32(-0.5) < 0);
    }
}
