//! WAV files to the pad player's sound format: 44.1 kHz stereo 16-bit, at
//! most 10 s (a port of offline-factory-kit.py's conversion).

use crate::kitfile::{MAX_FRAMES, RATE};

/// What a WAV file held, and its first seconds as floats.
pub struct Decoded {
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub float: bool,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Longer than the seconds decoded.
    pub cut: bool,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Decodes PCM (8, 16, 24 or 32-bit) or 32-bit float WAV, at most
/// `max_seconds` (plus a frame).
pub fn decode(data: &[u8], max_seconds: u32) -> Result<Decoded, String> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err("not a RIFF WAVE file".into());
    }
    let (mut fmt, mut pcm) = (None, None);
    let mut pos = 12usize;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = le32(data, pos + 4) as usize;
        let end = (pos + 8).saturating_add(size).min(data.len());
        let body = &data[pos + 8..end];
        match id {
            b"fmt " => fmt = Some(body),
            b"data" => pcm = Some(body),
            _ => {}
        }
        pos = pos.saturating_add(8 + size + (size & 1));
    }
    let (fmt, pcm) = match (fmt, pcm) {
        (Some(f), Some(p)) if f.len() >= 16 => (f, p),
        _ => return Err("no fmt or data chunk".into()),
    };
    let mut tag = le16(fmt, 0);
    let channels = le16(fmt, 2);
    let rate = le32(fmt, 4);
    let block = le16(fmt, 12) as usize;
    let bits = le16(fmt, 14);
    if tag == 0xFFFE && fmt.len() >= 26 {
        tag = le16(fmt, 24);
    }
    if !(tag == 1 || tag == 3)
        || channels < 1
        || !(8000..=192_000).contains(&rate)
        || block != channels as usize * bits as usize / 8
        || block == 0
    {
        return Err(format!(
            "unsupported format (tag {tag}, {channels} channels, {rate} Hz, {bits} bits)"
        ));
    }
    let all = pcm.len() / block;
    let limit = max_seconds as usize * rate as usize + 1;
    let n = all.min(limit);
    let float = tag == 3;
    let sample = |i: usize| -> f32 {
        let at = i * (bits as usize / 8);
        match (float, bits) {
            (true, 32) => f32::from_le_bytes([pcm[at], pcm[at + 1], pcm[at + 2], pcm[at + 3]]),
            (false, 16) => i16::from_le_bytes([pcm[at], pcm[at + 1]]) as f32 / 32768.0,
            (false, 32) => {
                i32::from_le_bytes([pcm[at], pcm[at + 1], pcm[at + 2], pcm[at + 3]]) as f32
                    / 2_147_483_648.0
            }
            (false, 24) => {
                (i32::from_le_bytes([0, pcm[at], pcm[at + 1], pcm[at + 2]]) >> 8) as f32
                    / 8_388_608.0
            }
            (false, 8) => (pcm[at] as f32 - 128.0) / 128.0,
            _ => 0.0,
        }
    };
    if !matches!(
        (float, bits),
        (true, 32) | (false, 8) | (false, 16) | (false, 24) | (false, 32)
    ) {
        return Err(format!("unsupported sample size {bits} bits"));
    }
    let ch = channels as usize;
    let left: Vec<f32> = (0..n).map(|f| sample(f * ch)).collect();
    let right: Vec<f32> = if ch > 1 {
        (0..n).map(|f| sample(f * ch + 1)).collect()
    } else {
        left.clone()
    };
    Ok(Decoded {
        rate,
        channels,
        bits,
        float,
        left,
        right,
        cut: all > limit,
    })
}

/// Linear resampling to 44.1 kHz.
pub fn resample(x: &[f32], rate: u32) -> Vec<f32> {
    if rate == RATE || x.is_empty() {
        return x.to_vec();
    }
    let n = (x.len() as f64 * RATE as f64 / rate as f64) as usize;
    let step = rate as f64 / RATE as f64;
    let last = x.len() - 1;
    (0..n)
        .map(|k| {
            let p = k as f64 * step;
            let i = p as usize;
            if i >= last {
                x[last]
            } else {
                let f = (p - i as f64) as f32;
                x[i] + (x[i + 1] - x[i]) * f
            }
        })
        .collect()
}

fn to_i16(v: f32) -> i16 {
    (v * 32767.0).round().clamp(-32768.0, 32767.0) as i16
}

/// Interleaved 16-bit stereo, at most [`MAX_FRAMES`] frames.
pub fn to_s16_stereo(left: &[f32], right: &[f32]) -> Vec<i16> {
    let n = left.len().min(right.len()).min(MAX_FRAMES);
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        out.push(to_i16(left[i]));
        out.push(to_i16(right[i]));
    }
    out
}

/// A WAV file as the pad player's audio, with what it was.
pub fn convert(data: &[u8]) -> Result<(Vec<i16>, Decoded), String> {
    let d = decode(data, (MAX_FRAMES / RATE as usize) as u32)?;
    let left = resample(&d.left, d.rate);
    let right = resample(&d.right, d.rate);
    let audio = to_s16_stereo(&left, &right);
    if audio.is_empty() {
        return Err("no audio".into());
    }
    Ok((audio, d))
}

/// A 16-bit WAV file, for tests here and elsewhere.
pub fn wav_bytes(rate: u32, channels: u16, bits: u16, samples: &[i32]) -> Vec<u8> {
    let bytes = bits as usize / 8;
    let mut data = Vec::new();
    for &s in samples {
        match bits {
            8 => data.push((s + 128) as u8),
            16 => data.extend_from_slice(&(s as i16).to_le_bytes()),
            24 => data.extend_from_slice(&s.to_le_bytes()[..3]),
            _ => data.extend_from_slice(&s.to_le_bytes()),
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * channels as u32 * bytes as u32).to_le_bytes());
    out.extend_from_slice(&(channels * bytes as u16).to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_decode() {
        let d = decode(&wav_bytes(44_100, 2, 16, &[16384, -16384, 0, 32767]), 10).unwrap();
        assert_eq!((d.channels, d.left.len()), (2, 2));
        assert_eq!((d.left[0], d.right[0]), (0.5, -0.5));
        let d = decode(&wav_bytes(48_000, 1, 24, &[4_194_304, -8_388_608]), 10).unwrap();
        assert_eq!(d.left, vec![0.5, -1.0]);
        assert_eq!(d.right, d.left);
        let d = decode(&wav_bytes(22_050, 1, 8, &[64, -128]), 10).unwrap();
        assert_eq!(d.left, vec![0.5, -1.0]);
        assert!(decode(b"RIFF0000WAVEjunk", 10).is_err());
    }

    #[test]
    fn long_files_are_cut_and_resampled() {
        let samples = vec![1000i32; 48_000 * 12];
        let (audio, d) = convert(&wav_bytes(48_000, 1, 16, &samples)).unwrap();
        assert!(d.cut);
        assert_eq!(audio.len() / 2, MAX_FRAMES);
        let (audio, _) = convert(&wav_bytes(22_050, 1, 16, &[0; 22_050])).unwrap();
        assert_eq!(audio.len() / 2, 44_100);
    }
}
