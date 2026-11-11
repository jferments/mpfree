//! MPFree userspace computation without acquiring ANY factory audio hardware.
//! This crate uses the existing bounded 16-step GrooveEngine and WAV Sampler.
//! The "backend" only validates PCM and computes a deterministic checksum.
//! All samples, patterns, and buffers are prepared before render callbacks.
//! No filesystem, ALSA, MIDI, DRM, touchscreen, socket or device access.
#![forbid(unsafe_code)]

use mpfree_core::float_to_s32;
use mpfree_groove::{GrooveConfig, GrooveEngine, Pattern};
use mpfree_sampler::{PreparedSample, Sampler};
use std::fmt;

pub const SAMPLE_RATE: u32 = 44_100;
pub const BLOCK: usize = 128;
pub const SELF_TEST_FRAMES: u64 = 88_200; // one bar at 120 bpm in 4/4
const TRIGGER_EVERY: u64 = SAMPLE_RATE as u64 / 2; // 22_050 frames
const SAMPLE_FRAMES: usize = 1_024;
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlessError {
    InvalidBlock,
    Groove,
    Sampler,
    MissingNote,
    InvalidOutput,
}
impl fmt::Display for HeadlessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HeadlessError {}

/// An audio abstraction with no access to the actual MPC audio endpoint.
/// Hardware output may be implemented separately ONLY after recovery/handoff.
pub trait AudioSink {
    fn write(&mut self, left: &[f32], right: &[f32]) -> Result<(), HeadlessError>;
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SinkStats {
    pub frames: u64,
    pub nonzero_samples: u64,
    pub checksum_fnv1a64: u64,
    pub peak_abs: f32,
}

/// No heap allocation in write(); no syscalls and no audio-device handles.
pub struct NullAudioBackend {
    stats: SinkStats,
}
impl Default for NullAudioBackend {
    fn default() -> Self {
        Self {
            stats: SinkStats {
                frames: 0,
                nonzero_samples: 0,
                checksum_fnv1a64: FNV_OFFSET,
                peak_abs: 0.0,
            },
        }
    }
}
impl NullAudioBackend {
    pub fn stats(&self) -> SinkStats {
        self.stats
    }
}
impl AudioSink for NullAudioBackend {
    fn write(&mut self, left: &[f32], right: &[f32]) -> Result<(), HeadlessError> {
        if left.len() != right.len() || left.len() > BLOCK {
            return Err(HeadlessError::InvalidBlock);
        }
        // Validate first so error does not partially mutate the sink statistics.
        if left.iter().chain(right.iter()).any(|x| !x.is_finite()) {
            return Err(HeadlessError::InvalidOutput);
        }
        for (&l, &r) in left.iter().zip(right.iter()) {
            self.stats.nonzero_samples += u64::from(l != 0.0) + u64::from(r != 0.0);
            self.stats.peak_abs = self.stats.peak_abs.max(l.abs()).max(r.abs());
            for sample in [float_to_s32(l), float_to_s32(r)] {
                for byte in sample.to_le_bytes() {
                    self.stats.checksum_fnv1a64 ^= u64::from(byte);
                    self.stats.checksum_fnv1a64 =
                        self.stats.checksum_fnv1a64.wrapping_mul(FNV_PRIME);
                }
            }
        }
        self.stats.frames += left.len() as u64;
        Ok(())
    }
}

/// The WAV is entirely generated OFF the audio callback, then passed through
/// our actual WAV parser and Sampler. No proprietary files or imports required.
fn make_fixture() -> Vec<u8> {
    let data_bytes = (SAMPLE_FRAMES * 2) as u32;
    let mut wave = Vec::with_capacity(44 + data_bytes as usize);
    wave.extend_from_slice(b"RIFF");
    wave.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wave.extend_from_slice(b"WAVEfmt ");
    wave.extend_from_slice(&16u32.to_le_bytes());
    wave.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wave.extend_from_slice(&1u16.to_le_bytes()); // mono
    wave.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wave.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wave.extend_from_slice(&2u16.to_le_bytes()); // block align
    wave.extend_from_slice(&16u16.to_le_bytes()); // sample bits
    wave.extend_from_slice(b"data");
    wave.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..SAMPLE_FRAMES {
        let pcm = ((SAMPLE_FRAMES - i) * 16) as i16;
        wave.extend_from_slice(&pcm.to_le_bytes());
    }
    wave
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionStats {
    pub sink: SinkStats,
    pub groove_events: u64,
    pub sampler_notes: u64,
    pub groove_stolen_voices: u64,
    pub sampler_stolen_voices: u64,
    pub prepared_bytes: usize,
}

/// Owns all DSP state and preallocated buffers. This is a *finite computation*,
/// not yet a running daemon or a proven realtime ALSA callback.
pub struct HeadlessSession {
    groove: GrooveEngine,
    sampler: Sampler,
    cursor: u64,
    next_note: u64,
    note_count: u64,
    groove_l: [f32; BLOCK],
    groove_r: [f32; BLOCK],
    sampler_l: [f32; BLOCK],
    sampler_r: [f32; BLOCK],
}
impl HeadlessSession {
    pub fn new() -> Result<Self, HeadlessError> {
        let config = GrooveConfig {
            sample_rate: SAMPLE_RATE,
            bpm: 120.0,
            bars: 1,
        };
        let groove =
            GrooveEngine::new(&Pattern::basic(), config).map_err(|_| HeadlessError::Groove)?;
        let mut sampler = Sampler::new(SAMPLE_RATE).map_err(|_| HeadlessError::Sampler)?;
        let sample =
            PreparedSample::parse_wav(&make_fixture()).map_err(|_| HeadlessError::Sampler)?;
        let index = sampler
            .add_sample(sample)
            .map_err(|_| HeadlessError::Sampler)?;
        sampler
            .map_note(60, index)
            .map_err(|_| HeadlessError::Sampler)?;
        Ok(Self {
            groove,
            sampler,
            cursor: 0,
            next_note: 0,
            note_count: 0,
            groove_l: [0.0; BLOCK],
            groove_r: [0.0; BLOCK],
            sampler_l: [0.0; BLOCK],
            sampler_r: [0.0; BLOCK],
        })
    }

    /// Keeps scheduler/note positions sample-exact, even if a callback block
    /// straddles a MIDI note timestamp (for example 22_050 modulo 128).
    pub fn render<S: AudioSink>(
        &mut self,
        requested: usize,
        sink: &mut S,
    ) -> Result<(), HeadlessError> {
        if requested == 0 || requested > BLOCK {
            return Err(HeadlessError::InvalidBlock);
        }
        let mut done = 0;
        while done < requested {
            if self.cursor == self.next_note {
                if !self.sampler.note_on(60, 112) {
                    return Err(HeadlessError::MissingNote);
                }
                self.note_count += 1;
                self.next_note += TRIGGER_EVERY;
            }
            let until_next_note =
                usize::try_from(self.next_note.saturating_sub(self.cursor).min(BLOCK as u64))
                    .map_err(|_| HeadlessError::InvalidBlock)?;
            let n = (requested - done).min(until_next_note);
            if n == 0 {
                return Err(HeadlessError::InvalidBlock);
            }
            self.groove
                .render(&mut self.groove_l[..n], &mut self.groove_r[..n])
                .map_err(|_| HeadlessError::Groove)?;
            self.sampler
                .render(&mut self.sampler_l[..n], &mut self.sampler_r[..n])
                .map_err(|_| HeadlessError::Sampler)?;
            for i in 0..n {
                self.groove_l[i] = (self.groove_l[i] + self.sampler_l[i] * 0.25).clamp(-1.0, 1.0);
                self.groove_r[i] = (self.groove_r[i] + self.sampler_r[i] * 0.25).clamp(-1.0, 1.0);
            }
            sink.write(&self.groove_l[..n], &self.groove_r[..n])?;
            self.cursor += n as u64;
            done += n;
        }
        Ok(())
    }

    pub fn completed(&self, backend: &NullAudioBackend) -> SessionStats {
        SessionStats {
            sink: backend.stats(),
            groove_events: self.groove.triggered_events(),
            sampler_notes: self.note_count,
            groove_stolen_voices: self.groove.stolen_voices(),
            sampler_stolen_voices: self.sampler.voice_steals(),
            prepared_bytes: self.groove.prepared_sample_bytes()
                + self.sampler.prepared_sample_bytes(),
        }
    }
    pub fn frame(&self) -> u64 {
        self.cursor
    }
}

/// A bounded, hardware-free execution of the real groove and sampler engines.
pub fn self_test() -> Result<SessionStats, HeadlessError> {
    let mut session = HeadlessSession::new()?;
    let mut null = NullAudioBackend::default();
    while session.frame() < SELF_TEST_FRAMES {
        let n = (SELF_TEST_FRAMES - session.frame()).min(BLOCK as u64) as usize;
        session.render(n, &mut null)?;
    }
    let stats = session.completed(&null);
    if stats.sink.frames != SELF_TEST_FRAMES
        || stats.groove_events != 12
        || stats.sampler_notes != 4
        || stats.sink.nonzero_samples < 100
        || stats.sink.checksum_fnv1a64 == FNV_OFFSET
        || stats.sink.peak_abs <= 0.0
        || stats.sink.peak_abs > 1.0
        || stats.groove_stolen_voices != 0
        || stats.sampler_stolen_voices != 0
    {
        return Err(HeadlessError::InvalidOutput);
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_groove_and_sampler_render_through_null() {
        let stats = self_test().unwrap();
        assert_eq!(stats.sink.frames, SELF_TEST_FRAMES);
        assert_eq!(stats.groove_events, 12);
        assert_eq!(stats.sampler_notes, 4);
        assert!(stats.prepared_bytes > SAMPLE_FRAMES * 8);
        assert!(stats.sink.peak_abs <= 1.0);
    }

    #[test]
    fn invariant_to_audio_callback_partitioning() {
        fn render(partition: &[usize]) -> SessionStats {
            let mut engine = HeadlessSession::new().unwrap();
            let mut sink = NullAudioBackend::default();
            let mut i = 0;
            while engine.frame() < SELF_TEST_FRAMES {
                let n = partition[i % partition.len()]
                    .min((SELF_TEST_FRAMES - engine.frame()) as usize);
                engine.render(n, &mut sink).unwrap();
                i += 1;
            }
            engine.completed(&sink)
        }
        assert_eq!(render(&[128]), render(&[17, 29, 113, 3]));
    }

    #[test]
    fn rejects_invalid_callback_size_without_advancing() {
        let mut engine = HeadlessSession::new().unwrap();
        let mut sink = NullAudioBackend::default();
        assert_eq!(
            engine.render(0, &mut sink),
            Err(HeadlessError::InvalidBlock)
        );
        assert_eq!(
            engine.render(129, &mut sink),
            Err(HeadlessError::InvalidBlock)
        );
        assert_eq!(engine.frame(), 0);
        assert_eq!(sink.stats().frames, 0);
    }

    #[test]
    fn null_backend_rejects_invalid_pcm_without_accounting_mutation() {
        let mut sink = NullAudioBackend::default();
        assert_eq!(
            sink.write(&[0.0, f32::NAN], &[0.0, 0.0]),
            Err(HeadlessError::InvalidOutput)
        );
        assert_eq!(sink.stats().frames, 0);
        assert_eq!(sink.stats().checksum_fnv1a64, FNV_OFFSET);
    }
}
