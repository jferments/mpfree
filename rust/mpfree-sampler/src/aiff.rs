//! AIFF and AIFF-C files for the sampler's sounds.
//!
//! [`read`] takes 8 to 32-bit PCM (big-endian, or little-endian `sowt`, as
//! some Mac apps write), 24 and 32-bit `in24`/`in32`, and 32 or 64-bit
//! float AIFF-C, mono or stereo, at 8 to 192 kHz. A file cut short (a
//! recorder that lost power) gives the whole frames it has and says so.
//! Compressed AIFF-C (IMA, µ-law and the like) is refused, saying which.
//!
//! Nothing here touches the disk: the caller reads the bytes.

use crate::model::Source;
use crate::wav::MAX_SAMPLES;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// An 80-bit IEEE extended float, as AIFF keeps its sample rate.
fn extended(b: &[u8]) -> f64 {
    let exp = i32::from(u16::from_be_bytes([b[0], b[1]]) & 0x7FFF);
    let mant = u64::from_be_bytes(b[2..10].try_into().expect("8 bytes"));
    if exp == 0 && mant == 0 {
        return 0.0;
    }
    let v = mant as f64 * 2f64.powi(exp - 16383 - 63);
    if b[0] & 0x80 != 0 {
        -v
    } else {
        v
    }
}

/// How an AIFF keeps its samples.
#[derive(Clone, Copy)]
enum Enc {
    /// Signed whole numbers, `width` bytes, the bits in the top of them.
    Int {
        width: usize,
        little: bool,
    },
    F32,
    F64,
}

/// An AIFF file's sound, named `name`, and a note when the file was cut
/// short. See the module docs for what it takes.
pub fn read(name: &str, b: &[u8]) -> Result<(Source, Option<String>), String> {
    if b.len() < 12 || &b[0..4] != b"FORM" {
        return Err("not an AIFF file".into());
    }
    let aifc = match &b[8..12] {
        b"AIFF" => false,
        b"AIFC" => true,
        _ => return Err("not an AIFF file".into()),
    };
    // (channels, frames, bits, rate, compression)
    let mut comm: Option<(u16, u32, u16, f64, [u8; 4])> = None;
    let mut audio: Option<&[u8]> = None;
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32_at(b, at + 4).unwrap_or(0) as usize;
        let start = at + 8;
        // A chunk running past the end is cut there (a file cut short).
        let end = start.saturating_add(len).min(b.len());
        let body = &b[start..end];
        if id == b"COMM" && comm.is_none() {
            let bad = || "a broken COMM chunk".to_string();
            if body.len() < 18 {
                return Err(bad());
            }
            let kind = if aifc {
                body.get(18..22)
                    .ok_or_else(bad)?
                    .try_into()
                    .expect("4 bytes")
            } else {
                *b"NONE"
            };
            comm = Some((
                u16_at(body, 0).ok_or_else(bad)?,
                u32_at(body, 2).ok_or_else(bad)?,
                u16_at(body, 6).ok_or_else(bad)?,
                extended(&body[8..18]),
                kind,
            ));
        } else if id == b"SSND" && audio.is_none() {
            let offset = u32_at(body, 0).ok_or("a broken SSND chunk")? as usize;
            audio = Some(body.get(8usize.saturating_add(offset)..).unwrap_or(&[]));
        }
        at = start.saturating_add(len).saturating_add(len & 1);
    }
    let (channels, frames, bits, rate, kind) = comm.ok_or("no COMM chunk")?;
    let audio = audio.ok_or("no audio (SSND chunk)")?;
    if channels != 1 && channels != 2 {
        return Err(format!("{channels} channels (mono or stereo only)"));
    }
    let int = |width: usize, little: bool| Enc::Int { width, little };
    let enc = match (&kind, bits) {
        (b"NONE" | b"twos", 1..=32) => int(usize::from(bits).div_ceil(8), false),
        (b"sowt", 1..=32) => int(usize::from(bits).div_ceil(8), true),
        (b"in24", _) => int(3, false),
        (b"in32", _) => int(4, false),
        (b"fl32" | b"FL32", _) => Enc::F32,
        (b"fl64" | b"FL64", _) => Enc::F64,
        (b"NONE" | b"twos" | b"sowt", _) => {
            return Err(format!("{bits}-bit samples aren't supported"))
        }
        _ => {
            let k = String::from_utf8_lossy(&kind).trim().to_string();
            return Err(format!("AIFF-C compressed as \"{k}\" isn't supported"));
        }
    };
    let width = match enc {
        Enc::Int { width, .. } => width,
        Enc::F32 => 4,
        Enc::F64 => 8,
    };
    let ch = usize::from(channels);
    let have = audio.len() / (width * ch);
    let declared = frames as usize;
    let frames = declared.min(have);
    let note =
        (frames < declared).then(|| format!("the file is cut short: its first {frames} frames"));
    let samples = frames * ch;
    if samples == 0 {
        return Err("no audio in the file".into());
    }
    if samples > MAX_SAMPLES {
        return Err(format!(
            "too long to hold in memory ({frames} frames; at most {})",
            MAX_SAMPLES / ch
        ));
    }
    let mut out = Vec::with_capacity(samples);
    for x in audio[..samples * width].chunks_exact(width) {
        out.push(match enc {
            Enc::Int { width, little } => {
                // Into the top of an i32, then down to -1..1.
                let mut v = [0u8; 4];
                for k in 0..width {
                    v[k] = if little { x[width - 1 - k] } else { x[k] };
                }
                i32::from_be_bytes(v) as f32 / 2_147_483_648.0
            }
            Enc::F32 => f32::from_be_bytes(x.try_into().expect("4 bytes")),
            Enc::F64 => f64::from_be_bytes(x.try_into().expect("8 bytes")) as f32,
        });
    }
    let hz = rate.round();
    let rate = if hz.is_finite() && (1.0..=u32::MAX as f64).contains(&hz) {
        hz as u32
    } else {
        0
    };
    let s = Source::new(name, rate, channels as u8, out).map_err(|e| match e {
        crate::model::ModelError::BadSource("rate") => {
            format!("{rate} Hz (8 to 192 kHz only)")
        }
        e => format!("{e}"),
    })?;
    Ok((s, note))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An AIFF (or with `kind`, AIFF-C) at 44.1 kHz holding `data`.
    fn aiff(ch: u16, bits: u16, frames: u32, kind: Option<&[u8; 4]>, data: &[u8]) -> Vec<u8> {
        let mut comm = Vec::new();
        comm.extend_from_slice(&ch.to_be_bytes());
        comm.extend_from_slice(&frames.to_be_bytes());
        comm.extend_from_slice(&bits.to_be_bytes());
        // 44100 as an 80-bit extended float.
        comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        if let Some(k) = kind {
            comm.extend_from_slice(k);
            comm.extend_from_slice(&[0]); // an empty name, padded
            comm.push(0);
        }
        let mut b = b"FORM\0\0\0\0".to_vec();
        b.extend_from_slice(if kind.is_some() { b"AIFC" } else { b"AIFF" });
        b.extend_from_slice(b"NAME");
        b.extend_from_slice(&3u32.to_be_bytes());
        b.extend_from_slice(b"abc\0");
        b.extend_from_slice(b"COMM");
        b.extend_from_slice(&(comm.len() as u32).to_be_bytes());
        b.extend_from_slice(&comm);
        b.extend_from_slice(b"SSND");
        b.extend_from_slice(&(data.len() as u32 + 8).to_be_bytes());
        b.extend_from_slice(&[0; 8]);
        b.extend_from_slice(data);
        let n = (b.len() - 8) as u32;
        b[4..8].copy_from_slice(&n.to_be_bytes());
        b
    }

    fn data(b: &[u8]) -> Vec<f32> {
        read("t", b).unwrap().0.data().to_vec()
    }

    #[test]
    fn each_kind_of_sample_reads_to_the_same_levels() {
        // 16-bit big-endian, stereo: -1, just under 1, 0, a quarter.
        let s = read(
            "Pad",
            &aiff(2, 16, 2, None, &[0x80, 0, 0x7F, 0xFF, 0, 0, 0x20, 0]),
        )
        .unwrap();
        assert!(s.1.is_none());
        let s = s.0;
        assert_eq!(
            (s.name.as_str(), s.rate(), s.channels(), s.frames()),
            ("Pad", 44_100, 2, 2)
        );
        assert_eq!(&s.data()[..], &[-1.0, 32_767.0 / 32_768.0, 0.0, 0.25]);
        // 8-bit AIFF is signed, unlike WAV's.
        assert_eq!(
            data(&aiff(1, 8, 3, None, &[0x80, 0x7F, 0x40])),
            [-1.0, 127.0 / 128.0, 0.5]
        );
        // 24-bit, and 12-bit kept in the top of 2 bytes.
        assert_eq!(data(&aiff(1, 24, 1, None, &[0xC0, 0, 0])), [-0.5]);
        assert_eq!(
            data(&aiff(1, 12, 1, None, &[0x7F, 0xF0])),
            [2047.0 / 2048.0]
        );
        // AIFF-C: little-endian, in24, float, double.
        assert_eq!(data(&aiff(1, 16, 1, Some(b"sowt"), &[0x00, 0xC0])), [-0.5]);
        assert_eq!(data(&aiff(1, 24, 1, Some(b"in24"), &[0x40, 0, 0])), [0.5]);
        assert_eq!(
            data(&aiff(1, 32, 1, Some(b"fl32"), &0.75f32.to_be_bytes())),
            [0.75]
        );
        assert_eq!(
            data(&aiff(1, 64, 1, Some(b"FL64"), &(-0.125f64).to_be_bytes())),
            [-0.125]
        );
    }

    #[test]
    fn what_it_cant_take_it_says_and_a_cut_file_gives_what_it_has() {
        // COMM says 3 frames; the file ends after 2 and a half.
        let mut b = aiff(1, 16, 3, None, &[0x10, 0, 0x20, 0, 0x30, 0]);
        b.truncate(b.len() - 1);
        let (s, note) = read("cut", &b).unwrap();
        assert_eq!(s.frames(), 2);
        assert_eq!(
            note.as_deref(),
            Some("the file is cut short: its first 2 frames")
        );
        let err = |b: &[u8]| read("x", b).unwrap_err();
        assert_eq!(err(b"RIFF\0\0\0\0WAVE"), "not an AIFF file");
        assert_eq!(
            err(&aiff(3, 16, 1, None, &[0; 6])),
            "3 channels (mono or stereo only)"
        );
        assert_eq!(
            err(&aiff(1, 4, 1, Some(b"ima4"), &[0; 34])),
            "AIFF-C compressed as \"ima4\" isn't supported"
        );
        assert_eq!(
            err(&aiff(1, 40, 1, None, &[0; 5])),
            "40-bit samples aren't supported"
        );
        assert_eq!(err(&aiff(1, 16, 0, None, &[])), "no audio in the file");
        let mut slow = aiff(1, 16, 1, None, &[0, 0]);
        // 4000 Hz: 0x400A 0xFA00...
        let at = slow.windows(4).position(|w| w == b"COMM").unwrap() + 16;
        slow[at..at + 10].copy_from_slice(&[0x40, 0x0A, 0xFA, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(err(&slow), "4000 Hz (8 to 192 kHz only)");
    }
}
