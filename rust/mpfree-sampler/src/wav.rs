//! WAV files for the sampler's sounds.
//!
//! - [`write`] keeps a sound losslessly, in the smallest form that gives
//!   every sample back to the bit: 16-bit or 24-bit PCM when the samples
//!   are exactly that (a line-input take, a 16-bit import), else 32-bit
//!   float (a resample, anything processed). Mono stays mono. Read back
//!   with [`read`], a sound is the same [`Source`], with the same id, so a
//!   saved project's pads find their sounds again.
//! - [`read`] takes the WAV files people bring: 8, 16, 24 or 32-bit PCM and
//!   32 or 64-bit float, mono or stereo, plain or `WAVE_FORMAT_EXTENSIBLE`,
//!   at 8 to 192 kHz. A file cut short (a recorder that lost power) gives
//!   the whole frames it has.
//! - [`file_name`] names a sound's file by its id: the same sound is kept
//!   once however many pads and kits use it.
//!
//! Nothing here touches the disk: the caller reads and writes the bytes.

use std::io::{self, Write};

use crate::model::Source;

/// The most samples (frames × channels) [`read`] takes into memory: 64 Mi,
/// about 12 minutes of stereo at 44.1 kHz (256 MiB as floats).
pub const MAX_SAMPLES: usize = 64 << 20;

/// How [`write`] keeps a sound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Pcm16,
    Pcm24,
    Float32,
}

impl Encoding {
    fn bytes(self) -> usize {
        match self {
            Encoding::Pcm16 => 2,
            Encoding::Pcm24 => 3,
            Encoding::Float32 => 4,
        }
    }
}

/// `x` is exactly `k / scale` for a whole `k` in `lo..=hi`, sign of zero
/// included.
fn exact(x: f32, scale: f32, lo: f32, hi: f32) -> Option<i32> {
    let y = x * scale;
    if y != y.trunc() || !(lo..=hi).contains(&y) {
        return None;
    }
    let k = y as i32;
    ((k as f32 / scale).to_bits() == x.to_bits()).then_some(k)
}

const S16: f32 = 32_768.0;
const S24: f32 = 8_388_608.0;

/// The smallest encoding that gives every sample of `s` back exactly.
pub fn encoding(s: &Source) -> Encoding {
    let d = s.data();
    if d.iter().all(|&x| exact(x, S16, -S16, S16 - 1.0).is_some()) {
        Encoding::Pcm16
    } else if d.iter().all(|&x| exact(x, S24, -S24, S24 - 1.0).is_some()) {
        Encoding::Pcm24
    } else {
        Encoding::Float32
    }
}

/// The file a project keeps `s` in: its id, so the same sound is one file.
pub fn file_name(s: &Source) -> String {
    format!("{}.wav", s.id())
}

/// Writes `s` as a WAV file (see the module docs), in chunks.
pub fn write(s: &Source, out: &mut dyn Write) -> io::Result<()> {
    let enc = encoding(s);
    let ch = u16::from(s.channels());
    let rate = s.rate();
    let width = enc.bytes();
    let data_len = s.data().len() as u64 * width as u64;
    let pad = data_len & 1;
    let float = enc == Encoding::Float32;
    // fmt is 18 bytes (with its empty extension) and a fact chunk follows
    // for float, as the format asks; 16 for PCM.
    let fmt_len: u32 = if float { 18 } else { 16 };
    let fact_len: u64 = if float { 12 } else { 0 };
    let riff = 4 + (8 + u64::from(fmt_len)) + fact_len + 8 + data_len + pad;
    let riff = u32::try_from(riff)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "too long for a WAV file"))?;
    let align = ch * width as u16;
    let mut h = Vec::with_capacity(58);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&riff.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&fmt_len.to_le_bytes());
    h.extend_from_slice(&(if float { 3u16 } else { 1u16 }).to_le_bytes());
    h.extend_from_slice(&ch.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * u32::from(align)).to_le_bytes());
    h.extend_from_slice(&align.to_le_bytes());
    h.extend_from_slice(&(width as u16 * 8).to_le_bytes());
    if float {
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(b"fact");
        h.extend_from_slice(&4u32.to_le_bytes());
        h.extend_from_slice(&(s.frames() as u32).to_le_bytes());
    }
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(data_len as u32).to_le_bytes());
    out.write_all(&h)?;
    let mut buf = Vec::with_capacity(64 * 1024);
    for chunk in s.data().chunks(16 * 1024) {
        buf.clear();
        for &x in chunk {
            match enc {
                Encoding::Pcm16 => {
                    let k = exact(x, S16, -S16, S16 - 1.0).unwrap_or(0) as i16;
                    buf.extend_from_slice(&k.to_le_bytes());
                }
                Encoding::Pcm24 => {
                    let k = exact(x, S24, -S24, S24 - 1.0).unwrap_or(0);
                    buf.extend_from_slice(&k.to_le_bytes()[..3]);
                }
                Encoding::Float32 => buf.extend_from_slice(&x.to_le_bytes()),
            }
        }
        out.write_all(&buf)?;
    }
    if pad == 1 {
        out.write_all(&[0])?;
    }
    Ok(())
}

/// [`write`] into memory.
pub fn bytes(s: &Source) -> Vec<u8> {
    let mut v = Vec::with_capacity(64 + s.data().len() * encoding(s).bytes());
    write(s, &mut v).expect("writing to memory");
    v
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// A WAV file's sound, named `name`. See the module docs for what it takes.
pub fn read(name: &str, b: &[u8]) -> Result<Source, String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    // (tag, channels, rate, bits)
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32_at(b, at + 4).unwrap_or(0) as usize;
        let start = at + 8;
        // A chunk running past the end is cut there (a file cut short).
        let end = start.saturating_add(len).min(b.len());
        let body = &b[start..end];
        if id == b"fmt " && fmt.is_none() {
            let bad = || "a broken fmt chunk".to_string();
            let mut tag = u16_at(body, 0).ok_or_else(bad)?;
            let channels = u16_at(body, 2).ok_or_else(bad)?;
            let rate = u32_at(body, 4).ok_or_else(bad)?;
            let bits = u16_at(body, 14).ok_or_else(bad)?;
            if tag == 0xFFFE {
                // WAVE_FORMAT_EXTENSIBLE: the real format leads the
                // sub-format GUID.
                tag = u16_at(body, 24).ok_or_else(bad)?;
            }
            fmt = Some((tag, channels, rate, bits));
        } else if id == b"data" && data.is_none() {
            data = Some(body);
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let audio = data.ok_or("no audio (data chunk)")?;
    if channels != 1 && channels != 2 {
        return Err(format!("{channels} channels (mono or stereo only)"));
    }
    let width = match (tag, bits) {
        (1, 8 | 16 | 24 | 32) | (3, 32 | 64) => usize::from(bits / 8),
        (1 | 3, _) => return Err(format!("{bits}-bit samples aren't supported")),
        _ => return Err(format!("format {tag:#06x} isn't PCM or float")),
    };
    let frame = width * usize::from(channels);
    let samples = audio.len() / frame * usize::from(channels);
    if samples == 0 {
        return Err("no audio in the file".into());
    }
    if samples > MAX_SAMPLES {
        return Err(format!(
            "too long to hold in memory ({} frames; at most {})",
            samples / usize::from(channels),
            MAX_SAMPLES / usize::from(channels)
        ));
    }
    let mut out = Vec::with_capacity(samples);
    for x in audio[..samples * width].chunks_exact(width) {
        out.push(match (tag, width) {
            (1, 1) => (f32::from(x[0]) - 128.0) / 128.0,
            (1, 2) => f32::from(i16::from_le_bytes([x[0], x[1]])) / S16,
            (1, 3) => (i32::from_le_bytes([0, x[0], x[1], x[2]]) >> 8) as f32 / S24,
            (1, 4) => i32::from_le_bytes([x[0], x[1], x[2], x[3]]) as f32 / 2_147_483_648.0,
            (3, 4) => f32::from_le_bytes([x[0], x[1], x[2], x[3]]),
            _ => f64::from_le_bytes(x.try_into().expect("8 bytes")) as f32,
        });
    }
    Source::new(name, rate, channels as u8, out).map_err(|e| match e {
        crate::model::ModelError::BadSource("rate") => {
            format!("{rate} Hz (8 to 192 kHz only)")
        }
        e => format!("{e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(channels: u8, data: Vec<f32>) -> Source {
        Source::new("t", 48_000, channels, data).unwrap()
    }

    fn round_trip(s: &Source) -> Source {
        let b = bytes(s);
        let back = read("t", &b).unwrap();
        assert_eq!(back.id(), s.id(), "the same sound, bit for bit");
        assert_eq!((back.rate(), back.channels()), (s.rate(), s.channels()));
        back
    }

    #[test]
    fn each_sound_is_kept_in_the_smallest_form_that_is_exact() {
        let pcm16: Vec<f32> = (0..1000).map(|i| (i - 500) as f32 * 61.0 / S16).collect();
        let s = source(1, pcm16);
        assert_eq!(encoding(&s), Encoding::Pcm16);
        assert_eq!(bytes(&s).len(), 44 + 2000);
        round_trip(&s);
        // The extremes of 16-bit, and 24-bit steps between them.
        let s = source(2, vec![-1.0, 32_767.0 / S16, 0.0, 3.0 / S24]);
        assert_eq!(encoding(&s), Encoding::Pcm24);
        round_trip(&s);
        // Odd 24-bit mono: a pad byte keeps the chunks aligned.
        let s = source(1, vec![1.0 / S24, -2.0 / S24, 5.0 / S24]);
        assert_eq!(bytes(&s).len(), 44 + 9 + 1);
        round_trip(&s);
        // Anything else is float: 1.0 itself, a processed value, -0.0.
        for odd in [1.0, 0.1, -0.0, 1.0e-9, 3.5] {
            let s = source(1, vec![0.0, odd, 0.25]);
            assert_eq!(encoding(&s), Encoding::Float32, "{odd}");
            round_trip(&s);
        }
    }

    #[test]
    fn files_from_elsewhere_are_read() {
        // 8-bit PCM, mono, and WAVE_FORMAT_EXTENSIBLE 24-bit stereo.
        let wav = |tag: u16, ch: u16, bits: u16, ext: Option<u16>, data: &[u8]| {
            let mut fmt = Vec::new();
            fmt.extend_from_slice(&tag.to_le_bytes());
            fmt.extend_from_slice(&ch.to_le_bytes());
            fmt.extend_from_slice(&44_100u32.to_le_bytes());
            let align = ch * bits / 8;
            fmt.extend_from_slice(&(44_100 * u32::from(align)).to_le_bytes());
            fmt.extend_from_slice(&align.to_le_bytes());
            fmt.extend_from_slice(&bits.to_le_bytes());
            if let Some(sub) = ext {
                fmt.extend_from_slice(&22u16.to_le_bytes());
                fmt.extend_from_slice(&bits.to_le_bytes());
                fmt.extend_from_slice(&3u32.to_le_bytes());
                fmt.extend_from_slice(&sub.to_le_bytes());
                fmt.extend_from_slice(&[0; 14]);
            }
            let mut b = b"RIFF\0\0\0\0WAVE".to_vec();
            b.extend_from_slice(b"LIST");
            b.extend_from_slice(&3u32.to_le_bytes());
            b.extend_from_slice(b"abc\0");
            b.extend_from_slice(b"fmt ");
            b.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
            b.extend_from_slice(&fmt);
            b.extend_from_slice(b"data");
            b.extend_from_slice(&(data.len() as u32).to_le_bytes());
            b.extend_from_slice(data);
            let n = (b.len() - 8) as u32;
            b[4..8].copy_from_slice(&n.to_le_bytes());
            b
        };
        let s = read("eight", &wav(1, 1, 8, None, &[128, 255, 0])).unwrap();
        assert_eq!(&s.data()[..], &[0.0, 127.0 / 128.0, -1.0]);
        assert_eq!((s.channels(), s.rate(), &s.name[..]), (1, 44_100, "eight"));
        let s = read(
            "ext",
            &wav(0xFFFE, 2, 24, Some(1), &[0, 0, 0x80, 0xff, 0xff, 0x7f]),
        )
        .unwrap();
        assert_eq!(&s.data()[..], &[-1.0, 8_388_607.0 / S24]);
        // Cut short mid-frame: the whole frames it has.
        let mut b = wav(1, 2, 16, None, &[1, 0, 2, 0, 3, 0, 4, 0]);
        b.truncate(b.len() - 3);
        assert_eq!(read("cut", &b).unwrap().frames(), 1);
        // What it can't take, it says.
        assert!(read("x", b"hello").is_err());
        assert!(read("x", &wav(1, 3, 16, None, &[0; 6]))
            .unwrap_err()
            .contains("3 channels"));
        assert!(read("x", &wav(2, 1, 4, None, &[0; 4]))
            .unwrap_err()
            .contains("isn't PCM"));
        assert!(read("x", &wav(1, 1, 16, None, &[])).is_err());
    }

    #[test]
    fn the_file_is_named_by_the_sound() {
        let a = source(1, vec![0.5, 0.25]);
        let mut b = a.clone();
        b.name = "another name".into();
        assert_eq!(file_name(&a), file_name(&b));
        assert_eq!(file_name(&a), format!("{}.wav", a.id()));
        assert_ne!(file_name(&a), file_name(&source(1, vec![0.5, 0.5])));
    }
}
