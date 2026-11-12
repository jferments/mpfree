//! MPFree's sampler.
//!
//! - [`model`]: the project (sources, regions, chop sets, layers, pads, undo),
//!   owned by the control thread.
//! - [`engine`]: the real-time voice pool that plays an [`engine::EngineView`]
//!   built from the model.
//! - [`chop`]: chop modes (transient, beat, silence) from the analysers.
//! - [`dsp`]: interpolation, envelopes, the filter.
//! - [`node`]: the sampler as an instrument node any track can hold.
//! - [`record`]: the rolling "skip back" buffers and takes.
//! - [`autosample`]: plays an instrument note by note, records each note
//!   and builds a keygroup instrument from the takes.
//! - [`keymap`]: Map to keys, a kit's pads made into one keygroup
//!   instrument at their sounds' notes.
//! - [`save`]: the kits as a saved project's text; [`wav`]: their sounds as
//!   WAV files, kept losslessly, and WAV files read from the library.
//! - [`Sampler`] and [`PreparedSample`] below: the first prototype (M3/M6),
//!   kept for `mpfree-headless` and the `mpfree-sampler` CLI; its WAV loader
//!   also feeds [`model::Source::from_prepared`].
//!
//! Loading is never real-time; `render` and the event calls only touch
//! prepared memory. Design: `docs/dev/design/sampler-design.md`.
#![forbid(unsafe_code)]

pub mod autosample;
pub mod chop;
pub mod dsp;
pub mod engine;
pub mod keymap;
pub mod model;
pub mod node;
pub mod phrase;
pub mod record;
pub mod save;
pub mod warp;
pub mod wav;

use std::fs::File;
use std::io::Read;
use std::path::Path;

pub const MAX_WAV_FILE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_SAMPLE_FRAMES: usize = 5_000_000;
pub const MAX_SAMPLES: usize = 128;
pub const MAX_PREPARED_SAMPLE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_VOICES: usize = 16;
const RELEASE_FRAMES: u16 = 64;
const STEAL_FADE_FRAMES: u16 = 32;

#[derive(Debug)]
pub enum SamplerError {
    InvalidWave(&'static str),
    UnsupportedWave(&'static str),
    TooLarge,
    WrongSampleRate { expected: u32, found: u32 },
    SampleBankFull,
    InvalidNote,
    MissingSample,
    OutputLengthMismatch,
    Io(std::io::Error),
}
impl std::fmt::Display for SamplerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SamplerError {}
impl From<std::io::Error> for SamplerError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
fn le_u16(data: &[u8]) -> Option<u16> {
    Some(u16::from_le_bytes(data.get(..2)?.try_into().ok()?))
}
fn le_u32(data: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(..4)?.try_into().ok()?))
}

pub struct PreparedSample {
    sample_rate: u32,
    frames: Box<[[f32; 2]]>,
}
impl PreparedSample {
    pub fn frames(&self) -> usize {
        self.frames.len()
    }
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    pub fn prepared_bytes(&self) -> usize {
        self.frames.len() * 8
    }
    /// The decoded frames, as [left, right] pairs.
    pub fn frame_slice(&self) -> &[[f32; 2]] {
        &self.frames
    }

    /// Accept only simple RIFF WAVE PCM16/24/32 and float32 mono/stereo.
    /// No WAVE_FORMAT_EXTENSIBLE, RF64 or automatic resampling yet.
    pub fn parse_wav(data: &[u8]) -> Result<Self, SamplerError> {
        if data.len() as u64 > MAX_WAV_FILE_BYTES {
            return Err(SamplerError::TooLarge);
        }
        if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return Err(SamplerError::InvalidWave("not RIFF WAVE"));
        }
        let declared = le_u32(&data[4..]).ok_or(SamplerError::InvalidWave("RIFF size"))? as usize;
        let end = declared.checked_add(8).ok_or(SamplerError::TooLarge)?;
        if end > data.len() || end < 12 {
            return Err(SamplerError::InvalidWave("truncated RIFF"));
        }
        let mut fmt: Option<(u16, u16, u32, u16, u16)> = None;
        let mut pcm = None;
        let mut pos = 12;
        while pos + 8 <= end {
            let id = &data[pos..pos + 4];
            let length = le_u32(&data[pos + 4..pos + 8])
                .ok_or(SamplerError::InvalidWave("chunk length"))?
                as usize;
            let payload_start = pos + 8;
            let payload_end = payload_start
                .checked_add(length)
                .ok_or(SamplerError::TooLarge)?;
            if payload_end > end {
                return Err(SamplerError::InvalidWave("truncated chunk"));
            }
            let chunk = &data[payload_start..payload_end];
            if id == b"fmt " {
                if fmt.is_some() || length < 16 {
                    return Err(SamplerError::InvalidWave("missing or duplicate fmt"));
                }
                let tag = le_u16(chunk).ok_or(SamplerError::InvalidWave("format tag"))?;
                let channels = le_u16(&chunk[2..]).ok_or(SamplerError::InvalidWave("channels"))?;
                let rate = le_u32(&chunk[4..]).ok_or(SamplerError::InvalidWave("rate"))?;
                let byte_rate =
                    le_u32(&chunk[8..]).ok_or(SamplerError::InvalidWave("byte rate"))?;
                let align = le_u16(&chunk[12..]).ok_or(SamplerError::InvalidWave("align"))?;
                let bits = le_u16(&chunk[14..]).ok_or(SamplerError::InvalidWave("bits"))?;
                if !(1..=2).contains(&channels) || !(8000..=192000).contains(&rate) {
                    return Err(SamplerError::UnsupportedWave("channel/rate"));
                }
                let supported = (tag == 1 && (bits == 16 || bits == 24 || bits == 32))
                    || (tag == 3 && bits == 32);
                if !supported {
                    return Err(SamplerError::UnsupportedWave("PCM16/24/32 or float32 only"));
                }
                let actual_align = channels * (bits / 8);
                if align != actual_align || byte_rate != rate * u32::from(align) {
                    return Err(SamplerError::InvalidWave(
                        "bad block alignment or byte rate",
                    ));
                }
                fmt = Some((tag, channels, rate, bits, align));
            } else if id == b"data" {
                if pcm.is_some() {
                    return Err(SamplerError::InvalidWave("duplicate data"));
                }
                pcm = Some(chunk);
            }
            let next = payload_end
                .checked_add(length & 1)
                .ok_or(SamplerError::TooLarge)?;
            if next > end {
                return Err(SamplerError::InvalidWave("missing padding"));
            }
            pos = next;
        }
        let (tag, channels, sample_rate, bits, block_align) =
            fmt.ok_or(SamplerError::InvalidWave("missing fmt"))?;
        let audio = pcm.ok_or(SamplerError::InvalidWave("missing data"))?;
        if audio.len() % usize::from(block_align) != 0 {
            return Err(SamplerError::InvalidWave("incomplete audio frame"));
        }
        let frames = audio.len() / usize::from(block_align);
        if frames == 0 || frames > MAX_SAMPLE_FRAMES {
            return Err(SamplerError::TooLarge);
        }
        let bytes_per_sample = usize::from(bits / 8);
        let mut output = Vec::with_capacity(frames);
        for frame in audio.chunks_exact(usize::from(block_align)) {
            let decode = |channel: usize| {
                let base = channel * bytes_per_sample;
                let raw = &frame[base..base + bytes_per_sample];
                match (tag, bits) {
                    (1, 16) => i16::from_le_bytes([raw[0], raw[1]]) as f32 / 32768.0,
                    (1, 24) => {
                        let sign = if raw[2] & 0x80 != 0 { 0xff } else { 0x00 };
                        i32::from_le_bytes([raw[0], raw[1], raw[2], sign]) as f32 / 8_388_608.0
                    }
                    (1, 32) => i32::from_le_bytes(raw.try_into().unwrap()) as f32 / 2_147_483_648.0,
                    (3, 32) => {
                        let x = f32::from_le_bytes(raw.try_into().unwrap());
                        if x.is_finite() {
                            x.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        }
                    }
                    _ => unreachable!(),
                }
            };
            let l = decode(0);
            let r = if channels == 2 { decode(1) } else { l };
            output.push([l, r]);
        }
        Ok(Self {
            sample_rate,
            frames: output.into_boxed_slice(),
        })
    }

    pub fn read_wav(path: &Path) -> Result<Self, SamplerError> {
        let file = File::open(path)?;
        if file.metadata()?.len() > MAX_WAV_FILE_BYTES {
            return Err(SamplerError::TooLarge);
        }
        // Bound even if file grows between metadata check and read.
        let mut limited = file.take(MAX_WAV_FILE_BYTES + 1);
        let mut bytes = Vec::new();
        limited.read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_WAV_FILE_BYTES {
            return Err(SamplerError::TooLarge);
        }
        Self::parse_wav(&bytes)
    }
}

#[derive(Debug, Clone, Copy)]
struct Voice {
    active: bool,
    note: u8,
    sample: usize,
    position: usize,
    gain: f32,
    release_left: u16,
    tail_left: u16,
    tail_l: f32,
    tail_r: f32,
    last_l: f32,
    last_r: f32,
}
impl Default for Voice {
    fn default() -> Self {
        Self {
            active: false,
            note: 0,
            sample: 0,
            position: 0,
            gain: 0.0,
            release_left: 0,
            tail_left: 0,
            tail_l: 0.0,
            tail_r: 0.0,
            last_l: 0.0,
            last_r: 0.0,
        }
    }
}
pub struct Sampler {
    rate: u32,
    samples: Vec<PreparedSample>,
    prepared_bytes: usize,
    mappings: [Option<usize>; 128],
    voices: [Voice; MAX_VOICES],
    next_steal: usize,
    voice_steals: u64,
}
impl Sampler {
    pub fn new(sample_rate: u32) -> Result<Self, SamplerError> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err(SamplerError::UnsupportedWave("sample rate"));
        }
        Ok(Self {
            rate: sample_rate,
            samples: Vec::new(),
            prepared_bytes: 0,
            mappings: [None; 128],
            voices: [Voice::default(); MAX_VOICES],
            next_steal: 0,
            voice_steals: 0,
        })
    }
    /// Load only during setup. No file reads or resampling inside render.
    pub fn add_sample(&mut self, sample: PreparedSample) -> Result<usize, SamplerError> {
        if sample.sample_rate != self.rate {
            return Err(SamplerError::WrongSampleRate {
                expected: self.rate,
                found: sample.sample_rate,
            });
        }
        if self.samples.len() >= MAX_SAMPLES {
            return Err(SamplerError::SampleBankFull);
        }
        let added_bytes = sample.prepared_bytes();
        let Some(next_bytes) = self.prepared_bytes.checked_add(added_bytes) else {
            return Err(SamplerError::TooLarge);
        };
        if next_bytes > MAX_PREPARED_SAMPLE_BYTES {
            return Err(SamplerError::TooLarge);
        }
        let index = self.samples.len();
        self.samples.push(sample);
        self.prepared_bytes = next_bytes;
        Ok(index)
    }
    pub fn map_note(&mut self, note: u8, sample_index: usize) -> Result<(), SamplerError> {
        if note > 127 {
            return Err(SamplerError::InvalidNote);
        }
        if sample_index >= self.samples.len() {
            return Err(SamplerError::MissingSample);
        }
        self.mappings[usize::from(note)] = Some(sample_index);
        Ok(())
    }
    pub fn note_on(&mut self, note: u8, velocity: u8) -> bool {
        if velocity == 0 {
            self.note_off(note);
            return false;
        }
        let Some(sample) = self
            .mappings
            .get(usize::from(note))
            .and_then(|index| *index)
        else {
            return false;
        };
        let (idx, was_active) = match self.voices.iter().position(|v| !v.active) {
            Some(i) => (i, false),
            None => {
                let i = self.next_steal;
                self.next_steal = (self.next_steal + 1) % MAX_VOICES;
                (i, true)
            }
        };
        let (tail_l, tail_r) = if was_active {
            self.voice_steals += 1;
            (self.voices[idx].last_l, self.voices[idx].last_r)
        } else {
            (0.0, 0.0)
        };
        self.voices[idx] = Voice {
            active: true,
            note,
            sample,
            position: 0,
            gain: f32::from(velocity) / 127.0,
            release_left: 0,
            tail_left: if was_active { STEAL_FADE_FRAMES } else { 0 },
            tail_l,
            tail_r,
            last_l: 0.0,
            last_r: 0.0,
        };
        true
    }
    pub fn note_off(&mut self, note: u8) {
        for voice in &mut self.voices {
            if voice.active && voice.note == note && voice.release_left == 0 {
                voice.release_left = RELEASE_FRAMES;
            }
        }
    }
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<(), SamplerError> {
        if left.len() != right.len() {
            return Err(SamplerError::OutputLengthMismatch);
        }
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            let (mut sum_l, mut sum_r) = (0.0, 0.0);
            for voice in &mut self.voices {
                if !voice.active {
                    continue;
                }
                let Some(frame) = self.samples[voice.sample].frames.get(voice.position) else {
                    voice.active = false;
                    continue;
                };
                let env = if voice.release_left > 0 {
                    f32::from(voice.release_left) / f32::from(RELEASE_FRAMES)
                } else {
                    1.0
                };
                let mut x = frame[0] * voice.gain * env;
                let mut y = frame[1] * voice.gain * env;
                if voice.tail_left > 0 {
                    let fade = f32::from(voice.tail_left) / f32::from(STEAL_FADE_FRAMES);
                    x += voice.tail_l * fade;
                    y += voice.tail_r * fade;
                    voice.tail_left -= 1;
                }
                voice.last_l = x;
                voice.last_r = y;
                sum_l += x;
                sum_r += y;
                voice.position += 1;
                if voice.release_left > 0 {
                    voice.release_left -= 1;
                    if voice.release_left == 0 {
                        voice.active = false;
                    }
                }
                if voice.position == self.samples[voice.sample].frames.len() {
                    voice.active = false;
                }
            }
            *l = sum_l.clamp(-1.0, 1.0);
            *r = sum_r.clamp(-1.0, 1.0);
        }
        Ok(())
    }
    pub fn voice_steals(&self) -> u64 {
        self.voice_steals
    }
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
    pub fn prepared_sample_bytes(&self) -> usize {
        self.prepared_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(tag: u16, bits: u16, channels: u16, rate: u32, pcm: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let padded = pcm.len() + (pcm.len() & 1);
        let riff = 36 + padded;
        out.extend(b"RIFF");
        out.extend((riff as u32).to_le_bytes());
        out.extend(b"WAVEfmt ");
        out.extend(16u32.to_le_bytes());
        out.extend(tag.to_le_bytes());
        out.extend(channels.to_le_bytes());
        out.extend(rate.to_le_bytes());
        let align = channels * (bits / 8);
        out.extend((rate * u32::from(align)).to_le_bytes());
        out.extend(align.to_le_bytes());
        out.extend(bits.to_le_bytes());
        out.extend(b"data");
        out.extend((pcm.len() as u32).to_le_bytes());
        out.extend(pcm);
        if pcm.len() & 1 != 0 {
            out.push(0);
        }
        out
    }
    #[test]
    fn wav_pcm16_mono_stereo_conversion_and_velocity() {
        let data = fixture(1, 16, 1, 44100, &[0xff, 0x7f, 0x00, 0x80]);
        let prepared = PreparedSample::parse_wav(&data).unwrap();
        assert_eq!(prepared.frames(), 2);
        assert_eq!(prepared.frames[0][0], prepared.frames[0][1]);
        assert!(prepared.frames[0][0] > 0.99);
        assert_eq!(prepared.frames[1][0], -1.0);
        let mut sampler = Sampler::new(44100).unwrap();
        let id = sampler.add_sample(prepared).unwrap();
        sampler.map_note(60, id).unwrap();
        assert!(sampler.note_on(60, 127));
        let mut l = [0.0; 2];
        let mut r = [0.0; 2];
        sampler.render(&mut l, &mut r).unwrap();
        assert_eq!(l, r);
        assert!(l[0] > 0.99);
        assert_eq!(l[1], -1.0);
        assert_eq!(sampler.active_voices(), 0);
    }
    #[test]
    fn format_24_and_float_are_decoded() {
        let pcm24 = fixture(1, 24, 2, 48000, &[0, 0, 0x80, 0xff, 0xff, 0x7f]);
        let p = PreparedSample::parse_wav(&pcm24).unwrap();
        assert_eq!(p.frames[0][0], -1.0);
        assert!(p.frames[0][1] > 0.99);
        let mut x = Vec::new();
        x.extend(f32::NAN.to_le_bytes());
        x.extend(0.5f32.to_le_bytes());
        let q = PreparedSample::parse_wav(&fixture(3, 32, 2, 44100, &x)).unwrap();
        assert_eq!(q.frames[0], [0.0, 0.5]);
    }
    #[test]
    fn invalid_wav_and_wrong_rate_are_rejected() {
        assert!(PreparedSample::parse_wav(b"abc").is_err());
        let bytes = fixture(1, 16, 1, 48000, &[0, 0]);
        assert!(PreparedSample::parse_wav(&bytes[..20]).is_err());
        let mut s = Sampler::new(44100).unwrap();
        assert!(matches!(
            s.add_sample(PreparedSample::parse_wav(&bytes).unwrap()),
            Err(SamplerError::WrongSampleRate { .. })
        ));
        let mut invalid = fixture(1, 16, 1, 44100, &[0, 0]);
        invalid[36] = b'X'; // remove required data tag
        assert!(PreparedSample::parse_wav(&invalid).is_err());
    }
    #[test]
    fn note_off_releases_without_pop_or_alloc() {
        let mut source = Vec::new();
        for _ in 0..512 {
            source.extend(16384i16.to_le_bytes());
        }
        let wav = fixture(1, 16, 1, 44100, &source);
        let mut s = Sampler::new(44100).unwrap();
        let n = s
            .add_sample(PreparedSample::parse_wav(&wav).unwrap())
            .unwrap();
        s.map_note(60, n).unwrap();
        assert!(s.note_on(60, 127));
        let mut a = [0.0; 4];
        let mut b = [0.0; 4];
        s.render(&mut a, &mut b).unwrap();
        s.note_off(60);
        let mut left = [0.0; 64];
        let mut right = [0.0; 64];
        s.render(&mut left, &mut right).unwrap();
        assert_eq!(left, right);
        assert!(left[0] > left[62]);
        assert_eq!(s.active_voices(), 0);
    }
    #[test]
    fn finite_voices_and_channel_errors() {
        let mut source = Vec::new();
        for _ in 0..512 {
            source.extend(100i16.to_le_bytes());
        }
        let mut s = Sampler::new(44100).unwrap();
        let idx = s
            .add_sample(PreparedSample::parse_wav(&fixture(1, 16, 1, 44100, &source)).unwrap())
            .unwrap();
        assert!(matches!(
            s.map_note(200, idx),
            Err(SamplerError::InvalidNote)
        ));
        s.map_note(60, idx).unwrap();
        for _ in 0..MAX_VOICES + 5 {
            assert!(s.note_on(60, 127));
        }
        assert_eq!(s.active_voices(), MAX_VOICES);
        assert_eq!(s.voice_steals(), 5);
        let mut a = [0.0; 1];
        let mut b = [0.0; 2];
        assert!(s.render(&mut a, &mut b).is_err());
        assert_eq!(s.active_voices(), MAX_VOICES);
    }
}
