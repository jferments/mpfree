//! WAV files in and out: captures from a reference device come in as
//! 16, 24 or 32-bit PCM or 32-bit float; fxlab writes 32-bit float.

/// Stereo audio as two f64 channels (a mono file is copied to both).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stereo {
    pub left: Vec<f64>,
    pub right: Vec<f64>,
    pub rate: u32,
}

impl Stereo {
    pub fn new(left: Vec<f64>, right: Vec<f64>, rate: u32) -> Stereo {
        assert_eq!(left.len(), right.len(), "channels differ in length");
        Stereo { left, right, rate }
    }

    /// The same signal on both sides.
    pub fn mono(x: Vec<f64>, rate: u32) -> Stereo {
        Stereo {
            right: x.clone(),
            left: x,
            rate,
        }
    }

    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    /// (left + right) / 2.
    pub fn mid(&self) -> Vec<f64> {
        self.left
            .iter()
            .zip(&self.right)
            .map(|(l, r)| 0.5 * (l + r))
            .collect()
    }

    /// Frames `start..start+len`, padded with silence past the end.
    pub fn slice(&self, start: usize, len: usize) -> Stereo {
        let take = |v: &Vec<f64>| -> Vec<f64> {
            (start..start + len)
                .map(|i| v.get(i).copied().unwrap_or(0.0))
                .collect()
        };
        Stereo::new(take(&self.left), take(&self.right), self.rate)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WavError {
    NotWav,
    Unsupported(String),
    Truncated,
}

impl std::fmt::Display for WavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WavError::NotWav => write!(f, "not a RIFF/WAVE file"),
            WavError::Unsupported(s) => write!(f, "unsupported WAV: {s}"),
            WavError::Truncated => write!(f, "WAV file is cut short"),
        }
    }
}

impl std::error::Error for WavError {}

fn u16_at(b: &[u8], i: usize) -> Result<u16, WavError> {
    b.get(i..i + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
        .ok_or(WavError::Truncated)
}

fn u32_at(b: &[u8], i: usize) -> Result<u32, WavError> {
    b.get(i..i + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(WavError::Truncated)
}

/// Parses a WAV file's bytes. Channels past the second are ignored.
pub fn parse(bytes: &[u8]) -> Result<Stereo, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWav);
    }
    let mut at = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32_at(bytes, at + 4)? as usize;
        let body = at + 8;
        if id == b"fmt " {
            let mut tag = u16_at(bytes, body)?;
            let channels = u16_at(bytes, body + 2)?;
            let rate = u32_at(bytes, body + 4)?;
            let bits = u16_at(bytes, body + 14)?;
            if tag == 0xFFFE && size >= 26 {
                // WAVE_FORMAT_EXTENSIBLE: the real tag opens the sub-format GUID.
                tag = u16_at(bytes, body + 24)?;
            }
            fmt = Some((tag, channels, rate, bits));
        } else if id == b"data" {
            let (tag, channels, rate, bits) =
                fmt.ok_or(WavError::Unsupported("data before fmt".into()))?;
            if channels == 0 {
                return Err(WavError::Unsupported("no channels".into()));
            }
            let end = (body + size).min(bytes.len());
            let data = &bytes[body..end];
            let width = (bits / 8) as usize;
            let frame = width * channels as usize;
            if frame == 0 {
                return Err(WavError::Unsupported(format!("{bits} bits")));
            }
            let read = |s: &[u8]| -> Result<f64, WavError> {
                Ok(match (tag, bits) {
                    (1, 16) => i16::from_le_bytes([s[0], s[1]]) as f64 / 32768.0,
                    (1, 24) => {
                        let v = i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8;
                        v as f64 / 8_388_608.0
                    }
                    (1, 32) => {
                        i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64 / 2_147_483_648.0
                    }
                    (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f64,
                    (3, 64) => f64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]),
                    _ => return Err(WavError::Unsupported(format!("format {tag}, {bits} bits"))),
                })
            };
            let frames = data.len() / frame;
            let mut left = Vec::with_capacity(frames);
            let mut right = Vec::with_capacity(frames);
            for f in 0..frames {
                let base = f * frame;
                let l = read(&data[base..base + width])?;
                let r = if channels > 1 {
                    read(&data[base + width..base + 2 * width])?
                } else {
                    l
                };
                left.push(l);
                right.push(r);
            }
            return Ok(Stereo { left, right, rate });
        }
        at = body + size + (size & 1);
    }
    Err(WavError::Unsupported("no data chunk".into()))
}

/// Reads a WAV file.
pub fn read(path: &std::path::Path) -> Result<Stereo, Box<dyn std::error::Error>> {
    Ok(parse(&std::fs::read(path)?)?)
}

/// 32-bit float stereo WAV bytes.
pub fn to_bytes(s: &Stereo) -> Vec<u8> {
    let l: Vec<f32> = s.left.iter().map(|x| *x as f32).collect();
    let r: Vec<f32> = s.right.iter().map(|x| *x as f32).collect();
    mpfree_fx::wav_f32(&l, &r, s.rate)
}

/// Writes a 32-bit float stereo WAV file.
pub fn write(path: &std::path::Path, s: &Stereo) -> std::io::Result<()> {
    std::fs::write(path, to_bytes(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_round_trip() {
        let s = Stereo::new(vec![0.0, 0.5, -0.25], vec![1.0, -1.0, 0.125], 44_100);
        let back = parse(&to_bytes(&s)).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn reads_16_bit_mono_pcm() {
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&40u32.to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&44_100u32.to_le_bytes());
        b.extend_from_slice(&88_200u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&4u32.to_le_bytes());
        b.extend_from_slice(&16384i16.to_le_bytes());
        b.extend_from_slice(&(-32768i16).to_le_bytes());
        let s = parse(&b).unwrap();
        assert_eq!(s.left, vec![0.5, -1.0]);
        assert_eq!(s.right, s.left);
        assert_eq!(s.rate, 44_100);
    }
}
