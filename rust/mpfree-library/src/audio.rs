//! What an audio file holds, read from its header alone: format, rate,
//! channels, bits and length. Nothing here decodes audio; the sampler does
//! that when a sound is loaded.

use std::io::{self, Read, Seek, SeekFrom};

/// The audio file types the library knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Aiff,
    Flac,
}

impl AudioFormat {
    /// From a file name's extension, ignoring case.
    pub fn from_name(name: &str) -> Option<AudioFormat> {
        let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
        match ext.as_str() {
            "wav" | "wave" => Some(AudioFormat::Wav),
            "aif" | "aiff" | "aifc" => Some(AudioFormat::Aiff),
            "flac" => Some(AudioFormat::Flac),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AudioFormat::Wav => "WAV",
            AudioFormat::Aiff => "AIFF",
            AudioFormat::Flac => "FLAC",
        }
    }
}

/// An audio file's header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioInfo {
    pub format: AudioFormat,
    pub rate: u32,
    pub channels: u16,
    pub bits: u16,
    /// True for 32- or 64-bit float samples.
    pub float: bool,
    /// Sample frames; 0 when the header does not say (a FLAC file streamed
    /// without a length).
    pub frames: u64,
}

impl AudioInfo {
    pub fn seconds(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.frames as f64 / f64::from(self.rate)
        }
    }

    /// `"0:03.25 · 44.1 kHz · 24-bit · stereo"`.
    pub fn summary(&self) -> String {
        let ch = match self.channels {
            1 => "mono".to_string(),
            2 => "stereo".to_string(),
            n => format!("{n} ch"),
        };
        let bits = if self.float {
            format!("{}-bit float", self.bits)
        } else {
            format!("{}-bit", self.bits)
        };
        format!(
            "{} · {} · {bits} · {ch}",
            duration(self.seconds()),
            rate_label(self.rate)
        )
    }
}

/// `"0:03.25"`, `"1:02:03"` for an hour or more.
pub fn duration(secs: f64) -> String {
    let secs = secs.max(0.0);
    let whole = secs as u64;
    if whole >= 3600 {
        format!("{}:{:02}:{:02}", whole / 3600, whole / 60 % 60, whole % 60)
    } else {
        let cs = ((secs - whole as f64) * 100.0) as u64;
        format!("{}:{:02}.{:02}", whole / 60, whole % 60, cs.min(99))
    }
}

/// `"44.1 kHz"`, `"48 kHz"`.
pub fn rate_label(rate: u32) -> String {
    if rate.is_multiple_of(1000) {
        format!("{} kHz", rate / 1000)
    } else {
        format!("{:.1} kHz", f64::from(rate) / 1000.0)
    }
}

fn bad(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}
fn u16be(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}
fn u32be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Reads the header of a WAV, AIFF or FLAC file. Reads at most a few
/// chunk headers and seeks past chunk bodies, so it is quick on any length.
pub fn read_info<R: Read + Seek>(r: &mut R) -> io::Result<AudioInfo> {
    let mut magic = [0u8; 12];
    r.read_exact(&mut magic[..4])?;
    if &magic[..4] == b"fLaC" {
        return flac(r);
    }
    r.read_exact(&mut magic[4..])?;
    match (&magic[..4], &magic[8..]) {
        (b"RIFF", b"WAVE") => wav(r),
        (b"FORM", b"AIFF") => aiff(r, false),
        (b"FORM", b"AIFC") => aiff(r, true),
        _ => Err(bad("not a WAV, AIFF or FLAC file")),
    }
}

/// The most chunks looked at before giving up.
const MAX_CHUNKS: usize = 64;

fn wav<R: Read + Seek>(r: &mut R) -> io::Result<AudioInfo> {
    let mut fmt: Option<(u16, u16, u32, u16, u16)> = None;
    let mut data: Option<u64> = None;
    for _ in 0..MAX_CHUNKS {
        let mut h = [0u8; 8];
        if r.read_exact(&mut h).is_err() {
            break;
        }
        let size = u64::from(u32le(&h[4..]));
        match &h[..4] {
            b"fmt " => {
                if size < 16 {
                    return Err(bad("WAV fmt chunk too short"));
                }
                let mut b = vec![0u8; size.min(40) as usize];
                r.read_exact(&mut b)?;
                let mut tag = u16le(&b[0..]);
                let bits = u16le(&b[14..]);
                if tag == 0xFFFE && b.len() >= 26 {
                    tag = u16le(&b[24..]);
                }
                fmt = Some((tag, u16le(&b[2..]), u32le(&b[4..]), u16le(&b[12..]), bits));
                r.seek(SeekFrom::Current(
                    (size - b.len() as u64 + (size & 1)) as i64,
                ))?;
            }
            b"data" => {
                data = Some(size);
                if fmt.is_some() {
                    break;
                }
                r.seek(SeekFrom::Current((size + (size & 1)) as i64))?;
            }
            _ => {
                r.seek(SeekFrom::Current((size + (size & 1)) as i64))?;
            }
        }
        if fmt.is_some() && data.is_some() {
            break;
        }
    }
    let (tag, channels, rate, align, bits) = fmt.ok_or_else(|| bad("WAV has no fmt chunk"))?;
    if !(tag == 1 || tag == 3) {
        return Err(bad("WAV is compressed; only PCM and float are read"));
    }
    if channels == 0 || rate == 0 || align == 0 {
        return Err(bad("WAV fmt chunk has zeros"));
    }
    Ok(AudioInfo {
        format: AudioFormat::Wav,
        rate,
        channels,
        bits,
        float: tag == 3,
        frames: data.unwrap_or(0) / u64::from(align),
    })
}

/// An 80-bit IEEE extended float, as AIFF stores its rate.
fn extended(b: &[u8]) -> f64 {
    let exp = i32::from(u16be(b) & 0x7FFF);
    let mant = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
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

fn aiff<R: Read + Seek>(r: &mut R, aifc: bool) -> io::Result<AudioInfo> {
    for _ in 0..MAX_CHUNKS {
        let mut h = [0u8; 8];
        r.read_exact(&mut h)?;
        let size = u64::from(u32be(&h[4..]));
        if &h[..4] == b"COMM" {
            if size < 18 {
                return Err(bad("AIFF COMM chunk too short"));
            }
            let mut b = vec![0u8; size.min(22) as usize];
            r.read_exact(&mut b)?;
            let channels = u16be(&b[0..]);
            let frames = u64::from(u32be(&b[2..]));
            let bits = u16be(&b[6..]);
            let rate = extended(&b[8..18]);
            let float = aifc && b.len() >= 22 && matches!(&b[18..22], b"fl32" | b"FL32" | b"fl64");
            if aifc && b.len() >= 22 && !float && !matches!(&b[18..22], b"NONE" | b"sowt" | b"twos")
            {
                return Err(bad("AIFF-C is compressed; only PCM and float are read"));
            }
            if channels == 0 || !(1.0..=1e6).contains(&rate) {
                return Err(bad("AIFF COMM chunk has zeros"));
            }
            return Ok(AudioInfo {
                format: AudioFormat::Aiff,
                rate: rate.round() as u32,
                channels,
                bits,
                float,
                frames,
            });
        }
        r.seek(SeekFrom::Current((size + (size & 1)) as i64))?;
    }
    Err(bad("AIFF has no COMM chunk"))
}

fn flac<R: Read + Seek>(r: &mut R) -> io::Result<AudioInfo> {
    let mut h = [0u8; 4];
    r.read_exact(&mut h)?;
    if h[0] & 0x7F != 0 {
        return Err(bad("FLAC does not start with STREAMINFO"));
    }
    let mut b = [0u8; 34];
    r.read_exact(&mut b)?;
    let rate = (u32::from(b[10]) << 12) | (u32::from(b[11]) << 4) | (u32::from(b[12]) >> 4);
    let channels = u16::from((b[12] >> 1) & 0x07) + 1;
    let bits = ((u16::from(b[12] & 1) << 4) | u16::from(b[13] >> 4)) + 1;
    let frames = (u64::from(b[13] & 0x0F) << 32) | u64::from(u32be(&b[14..]));
    if rate == 0 {
        return Err(bad("FLAC rate is zero"));
    }
    Ok(AudioInfo {
        format: AudioFormat::Flac,
        rate,
        channels,
        bits,
        float: false,
        frames,
    })
}

/// How a file stores one sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Enc {
    /// WAV's 8-bit: unsigned, 128 is silence.
    U8,
    /// AIFF's 8-bit: signed.
    S8,
    /// Signed integers of this many bytes, little or big endian.
    Int { bytes: usize, big: bool },
    /// 32-bit float.
    F32 { big: bool },
}

impl Enc {
    fn bytes(self) -> usize {
        match self {
            Enc::U8 | Enc::S8 => 1,
            Enc::Int { bytes, .. } => bytes,
            Enc::F32 { .. } => 4,
        }
    }

    fn value(self, s: &[u8]) -> f32 {
        match self {
            Enc::U8 => (f32::from(s[0]) - 128.0) / 128.0,
            Enc::S8 => f32::from(s[0] as i8) / 128.0,
            Enc::Int { bytes, big } => {
                // The sample in the top bytes of an i32, so every size scales alike.
                let mut b = [0u8; 4];
                for k in 0..bytes {
                    b[k] = if big { s[k] } else { s[bytes - 1 - k] };
                }
                i32::from_be_bytes(b) as f32 / 2_147_483_648.0
            }
            Enc::F32 { big } => {
                let b = [s[0], s[1], s[2], s[3]];
                if big {
                    f32::from_be_bytes(b)
                } else {
                    f32::from_le_bytes(b)
                }
            }
        }
    }
}

/// A quick picture of a PCM or float WAV or AIFF for the browser:
/// `columns` (min, max) pairs from -1 to 1, each from at most 512 frames
/// read at that column's place in the file, so a long file costs the same
/// as a short one. None for anything else (FLAC needs a decoder; the
/// sampler draws the full waveform once a sound is loaded).
pub fn overview<R: Read + Seek>(r: &mut R, columns: usize) -> Option<Vec<(f32, f32)>> {
    let mut h = [0u8; 12];
    r.read_exact(&mut h).ok()?;
    let (enc, channels, start, frames) = match (&h[..4], &h[8..]) {
        (b"RIFF", b"WAVE") => wav_layout(r)?,
        (b"FORM", b"AIFF") => aiff_layout(r, false)?,
        (b"FORM", b"AIFC") => aiff_layout(r, true)?,
        _ => return None,
    };
    sample_columns(r, columns, enc, channels, start, frames)
}

/// A WAV's encoding, channels, where its samples start and how many frames.
fn wav_layout<R: Read + Seek>(r: &mut R) -> Option<(Enc, usize, u64, u64)> {
    let mut fmt: Option<(u16, usize, usize, usize)> = None;
    let mut data: Option<(u64, u64)> = None;
    for _ in 0..MAX_CHUNKS {
        let mut c = [0u8; 8];
        r.read_exact(&mut c).ok()?;
        let size = u64::from(u32le(&c[4..]));
        let here = r.stream_position().ok()?;
        match &c[..4] {
            b"fmt " if size >= 16 => {
                let mut b = vec![0u8; size.min(40) as usize];
                r.read_exact(&mut b).ok()?;
                let mut tag = u16le(&b[0..]);
                if tag == 0xFFFE && b.len() >= 26 {
                    tag = u16le(&b[24..]);
                }
                fmt = Some((
                    tag,
                    usize::from(u16le(&b[2..])),
                    usize::from(u16le(&b[12..])),
                    usize::from(u16le(&b[14..])),
                ));
            }
            b"data" => data = Some((here, size)),
            _ => {}
        }
        if fmt.is_some() && data.is_some() {
            break;
        }
        r.seek(SeekFrom::Start(here + size + (size & 1))).ok()?;
    }
    let (tag, channels, align, bits) = fmt?;
    let (start, size) = data?;
    let enc = match (tag, bits) {
        (1, 8) => Enc::U8,
        (1, 16 | 24 | 32) => Enc::Int {
            bytes: bits / 8,
            big: false,
        },
        (3, 32) => Enc::F32 { big: false },
        _ => return None,
    };
    if channels == 0 || align != channels * enc.bytes() {
        return None;
    }
    Some((enc, channels, start, size / align as u64))
}

/// An AIFF's (or uncompressed AIFF-C's) encoding, channels, where its
/// samples start and how many frames.
fn aiff_layout<R: Read + Seek>(r: &mut R, aifc: bool) -> Option<(Enc, usize, u64, u64)> {
    let mut comm: Option<(usize, u64, usize, [u8; 4])> = None;
    let mut ssnd: Option<(u64, u64)> = None;
    for _ in 0..MAX_CHUNKS {
        let mut c = [0u8; 8];
        r.read_exact(&mut c).ok()?;
        let size = u64::from(u32be(&c[4..]));
        let here = r.stream_position().ok()?;
        match &c[..4] {
            b"COMM" if size >= 18 => {
                let mut b = vec![0u8; size.min(22) as usize];
                r.read_exact(&mut b).ok()?;
                let mut kind = *b"NONE";
                if aifc && b.len() >= 22 {
                    kind.copy_from_slice(&b[18..22]);
                }
                comm = Some((
                    usize::from(u16be(&b[0..])),
                    u64::from(u32be(&b[2..])),
                    usize::from(u16be(&b[6..])),
                    kind,
                ));
            }
            b"SSND" if size >= 8 => {
                let mut b = [0u8; 8];
                r.read_exact(&mut b).ok()?;
                ssnd = Some((here + 8 + u64::from(u32be(&b[0..])), size - 8));
            }
            _ => {}
        }
        if comm.is_some() && ssnd.is_some() {
            break;
        }
        r.seek(SeekFrom::Start(here + size + (size & 1))).ok()?;
    }
    let (channels, frames, bits, kind) = comm?;
    let (start, size) = ssnd?;
    let enc = match (&kind, bits) {
        (b"NONE" | b"twos", 8) => Enc::S8,
        (b"NONE" | b"twos", 16 | 24 | 32) => Enc::Int {
            bytes: bits / 8,
            big: true,
        },
        (b"sowt", 16 | 24 | 32) => Enc::Int {
            bytes: bits / 8,
            big: false,
        },
        (b"fl32" | b"FL32", _) => Enc::F32 { big: true },
        _ => return None,
    };
    if channels == 0 {
        return None;
    }
    let align = (channels * enc.bytes()) as u64;
    Some((enc, channels, start, frames.min(size / align)))
}

fn sample_columns<R: Read + Seek>(
    r: &mut R,
    columns: usize,
    enc: Enc,
    channels: usize,
    start: u64,
    frames: u64,
) -> Option<Vec<(f32, f32)>> {
    let columns = columns.clamp(1, 2048);
    let bytes = enc.bytes();
    let align = channels * bytes;
    if frames == 0 {
        return Some(vec![(0.0, 0.0); columns]);
    }
    let per = frames.div_ceil(columns as u64);
    let take = per.min(512) as usize;
    let mut buf = vec![0u8; take * align];
    let mut out = Vec::with_capacity(columns);
    for col in 0..columns as u64 {
        let first = col * per;
        if first >= frames {
            out.push((0.0, 0.0));
            continue;
        }
        let n = take.min((frames - first) as usize);
        r.seek(SeekFrom::Start(start + first * align as u64)).ok()?;
        r.read_exact(&mut buf[..n * align]).ok()?;
        let (mut lo, mut hi) = (0f32, 0f32);
        for s in buf[..n * align].chunks_exact(bytes) {
            let v = enc.value(s);
            if v.is_finite() {
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        out.push((lo.max(-1.0), hi.min(1.0)));
    }
    Some(out)
}

/// Test files: a WAV of a decaying 16-bit stereo tone.
pub fn tone_wav(rate: u32, frames: u32, hz: f32, decay: f32) -> Vec<u8> {
    let mut v = wav_bytes(rate, 2, 16, frames);
    for i in 0..frames as usize {
        let t = i as f32 / rate as f32;
        let s = (t * hz * std::f32::consts::TAU).sin() * (-t * decay).exp() * 0.9;
        let b = ((s * 32767.0) as i16).to_le_bytes();
        let o = 44 + i * 4;
        v[o..o + 2].copy_from_slice(&b);
        v[o + 2..o + 4].copy_from_slice(&b);
    }
    v
}

/// Test files: the same tone as [`tone_wav`], as a 16-bit stereo AIFF.
pub fn tone_aiff(rate: u32, frames: u32, hz: f32, decay: f32) -> Vec<u8> {
    let w = tone_wav(rate, frames, hz, decay);
    let mut data = w[44..].to_vec();
    for s in data.chunks_exact_mut(2) {
        s.swap(0, 1);
    }
    // The rate as an 80-bit extended float.
    let lz = u64::from(rate).leading_zeros();
    let mut ext = [0u8; 10];
    ext[..2].copy_from_slice(&((16383 + 63 - lz) as u16).to_be_bytes());
    ext[2..].copy_from_slice(&(u64::from(rate) << lz).to_be_bytes());
    let mut v = b"FORM".to_vec();
    v.extend_from_slice(&(4 + 8 + 18 + 8 + 8 + data.len() as u32).to_be_bytes());
    v.extend_from_slice(b"AIFFCOMM");
    v.extend_from_slice(&18u32.to_be_bytes());
    v.extend_from_slice(&2u16.to_be_bytes());
    v.extend_from_slice(&frames.to_be_bytes());
    v.extend_from_slice(&16u16.to_be_bytes());
    v.extend_from_slice(&ext);
    v.extend_from_slice(b"SSND");
    v.extend_from_slice(&(8 + data.len() as u32).to_be_bytes());
    v.extend_from_slice(&[0; 8]);
    v.extend_from_slice(&data);
    v
}

/// Test files: a minimal WAV header and data of `frames` silent frames.
pub fn wav_bytes(rate: u32, channels: u16, bits: u16, frames: u32) -> Vec<u8> {
    let align = u32::from(channels) * u32::from(bits / 8);
    let data = frames * align;
    let mut v = Vec::with_capacity(44 + data as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes());
    v.extend_from_slice(&channels.to_le_bytes());
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * align).to_le_bytes());
    v.extend_from_slice(&(align as u16).to_le_bytes());
    v.extend_from_slice(&bits.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data.to_le_bytes());
    v.resize(44 + data as usize, 0);
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_wav_header() {
        let i = read_info(&mut Cursor::new(wav_bytes(44_100, 2, 16, 44_100))).unwrap();
        assert_eq!(
            (i.format, i.rate, i.channels, i.bits, i.float, i.frames),
            (AudioFormat::Wav, 44_100, 2, 16, false, 44_100)
        );
        assert_eq!(i.summary(), "0:01.00 · 44.1 kHz · 16-bit · stereo");
    }

    #[test]
    fn a_wav_with_a_chunk_before_fmt_and_an_odd_one() {
        let w = wav_bytes(48_000, 1, 24, 480);
        let mut v = Vec::new();
        v.extend_from_slice(&w[..12]);
        v.extend_from_slice(b"LIST");
        v.extend_from_slice(&3u32.to_le_bytes());
        v.extend_from_slice(b"abc\0");
        v.extend_from_slice(&w[12..]);
        let i = read_info(&mut Cursor::new(v)).unwrap();
        assert_eq!((i.rate, i.channels, i.bits, i.frames), (48_000, 1, 24, 480));
    }

    #[test]
    fn an_aiff_header() {
        let mut v = Vec::new();
        v.extend_from_slice(b"FORM\0\0\0\x2EAIFFCOMM\0\0\0\x12");
        v.extend_from_slice(&2u16.to_be_bytes());
        v.extend_from_slice(&1000u32.to_be_bytes());
        v.extend_from_slice(&16u16.to_be_bytes());
        // 44100 as an 80-bit extended float.
        v.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
        let i = read_info(&mut Cursor::new(v)).unwrap();
        assert_eq!(
            (i.format, i.rate, i.channels, i.bits, i.frames),
            (AudioFormat::Aiff, 44_100, 2, 16, 1000)
        );
    }

    #[test]
    fn a_flac_header() {
        let mut v = b"fLaC".to_vec();
        v.extend_from_slice(&[0x80, 0, 0, 34]);
        let mut si = [0u8; 34];
        // 48000 Hz, 2 channels, 24 bits, 96000 frames.
        let rate: u32 = 48_000;
        si[10] = (rate >> 12) as u8;
        si[11] = (rate >> 4) as u8;
        // 4 bits of rate, 3 of channels - 1, the top bit of bits - 1 (23).
        si[12] = ((rate & 0xF) as u8) << 4 | (1 << 1) | 1;
        si[13] = 0x7 << 4;
        si[14..18].copy_from_slice(&96_000u32.to_be_bytes());
        v.extend_from_slice(&si);
        let i = read_info(&mut Cursor::new(v)).unwrap();
        assert_eq!(
            (i.format, i.rate, i.channels, i.bits, i.frames),
            (AudioFormat::Flac, 48_000, 2, 24, 96_000)
        );
        assert!((i.seconds() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn junk_is_refused_not_panicked_on() {
        for v in [
            Vec::new(),
            b"RIFF".to_vec(),
            b"RIFF\0\0\0\0WAVE".to_vec(),
            b"RIFF\0\0\0\0WAVEfmt \x02\0\0\0ab".to_vec(),
            b"hello world, not audio".to_vec(),
            b"fLaC\x01".to_vec(),
        ] {
            assert!(read_info(&mut Cursor::new(v)).is_err());
        }
    }

    #[test]
    fn an_overview_follows_the_sound() {
        let w = tone_wav(44_100, 44_100, 220.0, 6.0);
        let o = overview(&mut Cursor::new(w), 50).unwrap();
        assert_eq!(o.len(), 50);
        assert!(o[0].1 > 0.5 && o[0].0 < -0.5);
        assert!(o[49].1 < 0.05);
        let silent = overview(&mut Cursor::new(wav_bytes(44_100, 1, 24, 100)), 10).unwrap();
        assert!(silent.iter().all(|&(a, b)| a == 0.0 && b == 0.0));
        assert!(overview(&mut Cursor::new(b"junk".to_vec()), 10).is_none());
    }

    #[test]
    fn an_aiff_overview_matches_the_same_sound_as_a_wav() {
        let w = tone_wav(44_100, 30_000, 220.0, 6.0);
        let want = overview(&mut Cursor::new(w.clone()), 64).unwrap();
        // The WAV's 16-bit stereo samples, as an AIFF (big endian) and an
        // AIFF-C "sowt" (little endian), with a chunk before COMM.
        let pcm = &w[44..];
        let aiff = |kind: Option<&[u8; 4]>| {
            let mut comm = Vec::new();
            comm.extend_from_slice(&2u16.to_be_bytes());
            comm.extend_from_slice(&30_000u32.to_be_bytes());
            comm.extend_from_slice(&16u16.to_be_bytes());
            comm.extend_from_slice(&[0x40, 0x0E, 0xAC, 0x44, 0, 0, 0, 0, 0, 0]);
            let mut data: Vec<u8> = pcm.to_vec();
            if let Some(k) = kind {
                comm.extend_from_slice(k);
                comm.extend_from_slice(&[0, 0]);
            }
            if kind != Some(b"sowt") {
                for s in data.chunks_exact_mut(2) {
                    s.swap(0, 1);
                }
            }
            let mut body = Vec::new();
            body.extend_from_slice(if kind.is_some() { b"AIFC" } else { b"AIFF" });
            body.extend_from_slice(b"NAME\0\0\0\x03abc\0");
            body.extend_from_slice(b"COMM");
            body.extend_from_slice(&(comm.len() as u32).to_be_bytes());
            body.extend_from_slice(&comm);
            body.extend_from_slice(b"SSND");
            body.extend_from_slice(&(data.len() as u32 + 8 + 4).to_be_bytes());
            body.extend_from_slice(&4u32.to_be_bytes()); // offset
            body.extend_from_slice(&0u32.to_be_bytes());
            body.extend_from_slice(&[9, 9, 9, 9]);
            body.extend_from_slice(&data);
            let mut f = b"FORM".to_vec();
            f.extend_from_slice(&(body.len() as u32).to_be_bytes());
            f.extend_from_slice(&body);
            f
        };
        let plain = aiff(None);
        assert_eq!(
            read_info(&mut Cursor::new(plain.clone())).unwrap().frames,
            30_000
        );
        assert_eq!(overview(&mut Cursor::new(plain), 64).unwrap(), want);
        assert_eq!(
            overview(&mut Cursor::new(aiff(Some(b"sowt"))), 64).unwrap(),
            want
        );
        assert_eq!(
            overview(&mut Cursor::new(aiff(Some(b"NONE"))), 64).unwrap(),
            want
        );
        // Compressed AIFF-C has no picture.
        assert!(overview(&mut Cursor::new(aiff(Some(b"ima4"))), 64).is_none());
        // The test files' AIFF reads back as the WAV it was made from.
        let a = tone_aiff(48_000, 30_000, 220.0, 6.0);
        let i = read_info(&mut Cursor::new(a.clone())).unwrap();
        assert_eq!((i.rate, i.frames, i.channels), (48_000, 30_000, 2));
        assert_eq!(
            overview(&mut Cursor::new(a), 64),
            overview(&mut Cursor::new(tone_wav(48_000, 30_000, 220.0, 6.0)), 64)
        );
    }

    #[test]
    fn every_sample_size_scales_to_one() {
        let full = |enc: Enc, b: &[u8]| enc.value(b);
        assert_eq!(full(Enc::U8, &[0]), -1.0);
        assert_eq!(full(Enc::S8, &[0x80]), -1.0);
        assert_eq!(
            full(
                Enc::Int {
                    bytes: 2,
                    big: false
                },
                &[0x00, 0x80]
            ),
            -1.0
        );
        assert_eq!(
            full(
                Enc::Int {
                    bytes: 2,
                    big: true
                },
                &[0x40, 0x00]
            ),
            0.5
        );
        assert_eq!(
            full(
                Enc::Int {
                    bytes: 3,
                    big: false
                },
                &[0, 0, 0xC0]
            ),
            -0.5
        );
        assert_eq!(
            full(
                Enc::Int {
                    bytes: 3,
                    big: true
                },
                &[0xC0, 0, 0]
            ),
            -0.5
        );
        assert_eq!(
            full(
                Enc::Int {
                    bytes: 4,
                    big: false
                },
                &[0, 0, 0, 0x40]
            ),
            0.5
        );
        assert_eq!(full(Enc::F32 { big: true }, &0.25f32.to_be_bytes()), 0.25);
    }

    #[test]
    fn formats_from_names() {
        assert_eq!(AudioFormat::from_name("Kick.WAV"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::from_name("pad.aif"), Some(AudioFormat::Aiff));
        assert_eq!(AudioFormat::from_name("x.flac"), Some(AudioFormat::Flac));
        assert_eq!(AudioFormat::from_name("notes.txt"), None);
        assert_eq!(AudioFormat::from_name("wav"), None);
        assert_eq!(duration(3725.0), "1:02:05");
        assert_eq!(duration(61.5), "1:01.50");
    }
}
