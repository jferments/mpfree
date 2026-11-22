//! Where a sound file's audio is and how it is stored, from its header,
//! so the audio can be read a piece at a time in its own format and turned
//! into float as it arrives ([`decode`]).
//!
//! - **WAV:** RIFF, RF64 and BW64 (the 64-bit sizes of a `ds64` chunk, EBU
//!   Tech 3306); PCM of 8 (unsigned), 16, 24 and 32 bits and float of 32
//!   and 64 bits, also as `WAVE_FORMAT_EXTENSIBLE`. A data size of
//!   `FFFFFFFF` in a plain RIFF file means "to the end of the file".
//! - **AIFF and AIFF-C:** PCM of 8, 16, 24 and 32 bits, big-endian (`NONE`,
//!   `twos`) or little-endian (`sowt`), and float (`fl32`, `fl64`).
//!
//! A file that says it holds more audio than it does is read to its end in
//! whole frames, and says so ([`Layout::cut_short`]). Mono and stereo only.

use std::io::{self, Read, Seek, SeekFrom};

/// How one sample is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    /// 8-bit unsigned (WAV).
    U8,
    /// 8-bit signed (AIFF).
    I8,
    I16,
    I24,
    I32,
    F32,
    F64,
}

impl Sample {
    pub fn bytes(self) -> usize {
        match self {
            Sample::U8 | Sample::I8 => 1,
            Sample::I16 => 2,
            Sample::I24 => 3,
            Sample::I32 | Sample::F32 => 4,
            Sample::F64 => 8,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Container {
    Wav,
    Rf64,
    Aiff,
}

/// Where a file's audio is and how it is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub container: Container,
    /// The byte the first frame starts at.
    pub data_at: u64,
    /// Whole frames of audio in the file.
    pub frames: u64,
    pub channels: u16,
    pub rate: u32,
    pub sample: Sample,
    pub endian: Endian,
    /// The header says there is more audio than the file holds.
    pub cut_short: bool,
}

impl Layout {
    /// Bytes in one frame.
    pub fn block_align(&self) -> usize {
        self.sample.bytes() * usize::from(self.channels)
    }

    /// Where frame `frame` starts in the file.
    pub fn byte_of(&self, frame: u64) -> u64 {
        self.data_at + frame * self.block_align() as u64
    }
}

/// The most chunks looked at before the audio is found.
const MAX_CHUNKS: usize = 256;

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn u64le(b: &[u8]) -> u64 {
    u64::from_le_bytes(b[..8].try_into().expect("8 bytes"))
}
fn u16be(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn u32be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// The layout of the sound file `r` (read from its start; the position
/// afterwards is anywhere).
pub fn layout<R: Read + Seek>(r: &mut R) -> io::Result<Layout> {
    let len = r.seek(SeekFrom::End(0))?;
    r.seek(SeekFrom::Start(0))?;
    let mut magic = [0u8; 12];
    r.read_exact(&mut magic)?;
    let l = match (&magic[..4], &magic[8..]) {
        (b"RIFF", b"WAVE") => wav(r, len, false)?,
        (b"RF64" | b"BW64", b"WAVE") => wav(r, len, true)?,
        (b"FORM", b"AIFF") => aiff(r, len, false)?,
        (b"FORM", b"AIFC") => aiff(r, len, true)?,
        _ => return Err(bad("not a WAV or AIFF file")),
    };
    check(l)
}

fn check(l: Layout) -> io::Result<Layout> {
    if !(1..=2).contains(&l.channels) {
        return Err(bad(&format!(
            "{} channels (mono or stereo only)",
            l.channels
        )));
    }
    if !(8_000..=192_000).contains(&l.rate) {
        return Err(bad(&format!("{} Hz (8 to 192 kHz only)", l.rate)));
    }
    Ok(l)
}

/// Frames `said` the header gives, cut to what the file holds from `at`.
fn whole(said: u64, at: u64, len: u64, align: usize) -> (u64, bool) {
    let there = len.saturating_sub(at) / align as u64;
    (said.min(there), said > there)
}

fn wav<R: Read + Seek>(r: &mut R, len: u64, rf64: bool) -> io::Result<Layout> {
    let mut big_data: Option<u64> = None;
    let mut fmt: Option<(Sample, u16, u32)> = None;
    let mut at = 12u64;
    for _ in 0..MAX_CHUNKS {
        let mut h = [0u8; 8];
        if r.read_exact(&mut h).is_err() {
            break;
        }
        let size = u64::from(u32le(&h[4..]));
        let body = at + 8;
        match &h[..4] {
            b"ds64" if rf64 => {
                if size < 28 {
                    return Err(bad("RF64 ds64 chunk too short"));
                }
                let mut b = [0u8; 28];
                r.read_exact(&mut b)?;
                big_data = Some(u64le(&b[8..]));
            }
            b"fmt " => {
                if size < 16 {
                    return Err(bad("WAV fmt chunk too short"));
                }
                let mut b = vec![0u8; size.min(40) as usize];
                r.read_exact(&mut b)?;
                let mut tag = u16le(&b[0..]);
                if tag == 0xFFFE {
                    if b.len() < 26 {
                        return Err(bad("WAV extensible fmt chunk too short"));
                    }
                    tag = u16le(&b[24..]);
                }
                let (channels, rate, align, bits) = (
                    u16le(&b[2..]),
                    u32le(&b[4..]),
                    u16le(&b[12..]),
                    u16le(&b[14..]),
                );
                let sample = match (tag, bits) {
                    (1, 8) => Sample::U8,
                    (1, 16) => Sample::I16,
                    (1, 24) => Sample::I24,
                    (1, 32) => Sample::I32,
                    (3, 32) => Sample::F32,
                    (3, 64) => Sample::F64,
                    (1 | 3, b) => return Err(bad(&format!("{b}-bit WAV samples"))),
                    (t, _) => return Err(bad(&format!("WAV format {t:#06x} (PCM or float only)"))),
                };
                if usize::from(align) != sample.bytes() * usize::from(channels) {
                    return Err(bad("WAV block size does not match its samples"));
                }
                fmt = Some((sample, channels, rate));
            }
            b"data" => {
                let (sample, channels, rate) = fmt.ok_or_else(|| bad("WAV data before fmt"))?;
                let said_bytes = match (rf64, size) {
                    (true, 0xFFFF_FFFF) => big_data.ok_or_else(|| bad("RF64 with no ds64"))?,
                    (false, 0xFFFF_FFFF) => u64::MAX,
                    (_, s) => s,
                };
                let align = sample.bytes() * usize::from(channels);
                let (frames, cut_short) = whole(said_bytes / align as u64, body, len, align);
                return Ok(Layout {
                    container: if rf64 {
                        Container::Rf64
                    } else {
                        Container::Wav
                    },
                    data_at: body,
                    frames,
                    channels,
                    rate,
                    sample,
                    endian: Endian::Little,
                    cut_short: cut_short && said_bytes != u64::MAX,
                });
            }
            _ => {}
        }
        at = body + size + (size & 1);
        r.seek(SeekFrom::Start(at))?;
    }
    Err(bad(if fmt.is_some() {
        "WAV has no data chunk"
    } else {
        "WAV has no fmt chunk"
    }))
}

/// An 80-bit extended float (AIFF's sample rate), rounded to whole Hz.
fn extended(b: &[u8]) -> u32 {
    let exp = i32::from(u16be(b) & 0x7FFF) - 16383 - 63;
    let mant = u64::from_be_bytes(b[2..10].try_into().expect("8 bytes"));
    let v = mant as f64 * 2f64.powi(exp);
    if v.is_finite() && v > 0.0 && v < f64::from(u32::MAX) {
        v.round() as u32
    } else {
        0
    }
}

fn aiff<R: Read + Seek>(r: &mut R, len: u64, aifc: bool) -> io::Result<Layout> {
    let mut comm: Option<(u16, u64, Sample, Endian, u32)> = None;
    let mut at = 12u64;
    for _ in 0..MAX_CHUNKS {
        let mut h = [0u8; 8];
        if r.read_exact(&mut h).is_err() {
            break;
        }
        let size = u64::from(u32be(&h[4..]));
        let body = at + 8;
        match &h[..4] {
            b"COMM" => {
                let need = if aifc { 22 } else { 18 };
                if size < need {
                    return Err(bad("AIFF COMM chunk too short"));
                }
                let mut b = [0u8; 22];
                r.read_exact(&mut b[..need as usize])?;
                let channels = u16be(&b[0..]);
                let frames = u64::from(u32be(&b[2..]));
                let bits = u16be(&b[6..]);
                let rate = extended(&b[8..18]);
                let codec = if aifc {
                    [b[18], b[19], b[20], b[21]]
                } else {
                    *b"NONE"
                };
                let pcm = |bits: u16| match bits {
                    8 => Ok(Sample::I8),
                    16 => Ok(Sample::I16),
                    24 => Ok(Sample::I24),
                    32 => Ok(Sample::I32),
                    b => Err(bad(&format!("{b}-bit AIFF samples"))),
                };
                let (sample, endian) = match &codec {
                    b"NONE" | b"twos" => (pcm(bits)?, Endian::Big),
                    b"sowt" => (pcm(bits)?, Endian::Little),
                    b"fl32" | b"FL32" => (Sample::F32, Endian::Big),
                    b"fl64" | b"FL64" => (Sample::F64, Endian::Big),
                    c => {
                        return Err(bad(&format!(
                            "AIFF-C compression {:?} (uncompressed only)",
                            String::from_utf8_lossy(c)
                        )))
                    }
                };
                comm = Some((channels, frames, sample, endian, rate));
            }
            b"SSND" => {
                let (channels, said, sample, endian, rate) =
                    comm.ok_or_else(|| bad("AIFF sound data before COMM"))?;
                let mut b = [0u8; 8];
                r.read_exact(&mut b)?;
                let data_at = body + 8 + u64::from(u32be(&b[0..]));
                let align = sample.bytes() * usize::from(channels);
                let (frames, cut_short) = whole(said, data_at, len, align);
                return Ok(Layout {
                    container: Container::Aiff,
                    data_at,
                    frames,
                    channels,
                    rate,
                    sample,
                    endian,
                    cut_short,
                });
            }
            _ => {}
        }
        at = body + size + (size & 1);
        r.seek(SeekFrom::Start(at))?;
    }
    Err(bad(if comm.is_some() {
        "AIFF has no sound data"
    } else {
        "AIFF has no COMM chunk"
    }))
}

/// Turn `bytes` (whole samples as `sample` and `endian` store them) into
/// float in `out`; gives the samples written (as many as both hold). Any
/// value that isn't finite becomes 0.
pub fn decode(sample: Sample, endian: Endian, bytes: &[u8], out: &mut [f32]) -> usize {
    let n = (bytes.len() / sample.bytes()).min(out.len());
    let le = endian == Endian::Little;
    let b = &bytes[..n * sample.bytes()];
    let o = &mut out[..n];
    match sample {
        Sample::U8 => {
            for (o, &x) in o.iter_mut().zip(b) {
                *o = (f32::from(x) - 128.0) / 128.0;
            }
        }
        Sample::I8 => {
            for (o, &x) in o.iter_mut().zip(b) {
                *o = f32::from(x as i8) / 128.0;
            }
        }
        Sample::I16 => {
            for (o, x) in o.iter_mut().zip(b.chunks_exact(2)) {
                let v = if le {
                    i16::from_le_bytes([x[0], x[1]])
                } else {
                    i16::from_be_bytes([x[0], x[1]])
                };
                *o = f32::from(v) / 32_768.0;
            }
        }
        Sample::I24 => {
            for (o, x) in o.iter_mut().zip(b.chunks_exact(3)) {
                let v = if le {
                    i32::from_le_bytes([0, x[0], x[1], x[2]])
                } else {
                    i32::from_be_bytes([x[0], x[1], x[2], 0])
                } >> 8;
                *o = v as f32 / 8_388_608.0;
            }
        }
        Sample::I32 => {
            for (o, x) in o.iter_mut().zip(b.chunks_exact(4)) {
                let a = [x[0], x[1], x[2], x[3]];
                let v = if le {
                    i32::from_le_bytes(a)
                } else {
                    i32::from_be_bytes(a)
                };
                *o = (f64::from(v) / 2_147_483_648.0) as f32;
            }
        }
        Sample::F32 => {
            for (o, x) in o.iter_mut().zip(b.chunks_exact(4)) {
                let a = [x[0], x[1], x[2], x[3]];
                let v = if le {
                    f32::from_le_bytes(a)
                } else {
                    f32::from_be_bytes(a)
                };
                *o = if v.is_finite() { v } else { 0.0 };
            }
        }
        Sample::F64 => {
            for (o, x) in o.iter_mut().zip(b.chunks_exact(8)) {
                let a: [u8; 8] = x.try_into().expect("8 bytes");
                let v = if le {
                    f64::from_le_bytes(a)
                } else {
                    f64::from_be_bytes(a)
                };
                *o = if v.is_finite() { v as f32 } else { 0.0 };
            }
        }
    }
    n
}
