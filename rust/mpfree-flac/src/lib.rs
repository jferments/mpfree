//! FLAC, decoded by our own code, with no dependencies (RFC 9639).
//!
//! - [`decode`] gives every sample of a whole file, for the sampler to
//!   load onto a pad.
//! - [`StreamInfo::read`] reads a file's STREAMINFO and finds where its
//!   frames start; [`Decoder::frame`] decodes any one frame and
//!   [`find_frame`] finds the next one in a run of bytes, so the browser can
//!   draw a waveform from frames spread over a file without decoding all of
//!   it ([`overview`]).
//!
//! It takes what encoders write: 4 to 32 bits, 1 to 8 channels, fixed and
//! variable block sizes, every subframe type (constant, verbatim, fixed
//! and LPC predictors), both residual codings and their escapes, wasted
//! bits, and left/side, side/right and mid/side stereo. An ID3v2 tag in
//! front of the stream is stepped over.
//!
//! Every frame header's CRC-8 and every frame's CRC-16 is checked, so a
//! damaged frame is an error saying where it is, never noise. A file cut
//! short gives the whole frames it has and says so ([`Decoded::cut_short`]);
//! bytes between frames that aren't a frame are stepped over and counted
//! ([`Decoded::skipped`]), so the caller can say that too.

use std::fmt;
use std::io::{Read, Seek, SeekFrom};

/// What can go wrong reading a FLAC stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No `fLaC` marker where the stream should start.
    NotFlac,
    /// The bytes end inside a header or a frame.
    Short,
    /// Something the format doesn't allow, and what.
    Bad(&'static str),
    /// A frame whose CRC-16 doesn't match: damaged, `ms` milliseconds in.
    Damaged { ms: u64 },
    /// More frames than the caller takes into memory.
    TooLong { frames: u64, max: u64 },
    /// A stream with no audio in it.
    Empty,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFlac => write!(f, "not a FLAC file"),
            Error::Short => write!(f, "the file ends inside a frame"),
            Error::Bad(what) => write!(f, "a broken FLAC stream ({what})"),
            Error::Damaged { ms } => write!(
                f,
                "a damaged frame {}.{:02} s in",
                ms / 1000,
                ms % 1000 / 10
            ),
            Error::TooLong { frames, max } => write!(
                f,
                "too long to hold in memory ({frames} frames; at most {max})"
            ),
            Error::Empty => write!(f, "no audio in the file"),
        }
    }
}

impl std::error::Error for Error {}

/// A stream's STREAMINFO.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub min_block: u16,
    pub max_block: u16,
    /// The largest frame in bytes; 0 when the encoder didn't say.
    pub max_frame: u32,
    pub rate: u32,
    pub channels: u8,
    pub bits: u8,
    /// Sample frames; 0 when the encoder didn't say.
    pub total: u64,
}

impl StreamInfo {
    /// A stream's STREAMINFO and the byte its first frame starts at.
    pub fn read(b: &[u8]) -> Result<(StreamInfo, usize), Error> {
        let mut at = 0;
        if b.len() >= 10 && &b[..3] == b"ID3" {
            // An ID3v2 tag: its size is 4 bytes of 7 bits, and a footer
            // adds 10 more.
            let size = b[6..10]
                .iter()
                .fold(0usize, |s, &x| s << 7 | usize::from(x & 0x7F));
            at = 10 + size + if b[5] & 0x10 != 0 { 10 } else { 0 };
        }
        if b.get(at..at + 4) != Some(b"fLaC") {
            return Err(Error::NotFlac);
        }
        at += 4;
        let mut info = None;
        loop {
            let h = b.get(at..at + 4).ok_or(Error::Short)?;
            let (last, kind) = (h[0] & 0x80 != 0, h[0] & 0x7F);
            let len = usize::from(h[1]) << 16 | usize::from(h[2]) << 8 | usize::from(h[3]);
            at += 4;
            let body = b.get(at..at + len).ok_or(Error::Short)?;
            match kind {
                0 if info.is_some() => return Err(Error::Bad("two STREAMINFO blocks")),
                0 => info = Some(StreamInfo::parse(body)?),
                127 => return Err(Error::Bad("an invalid metadata block")),
                _ if info.is_none() => return Err(Error::Bad("STREAMINFO isn't first")),
                _ => {}
            }
            at += len;
            if last {
                break;
            }
        }
        Ok((info.ok_or(Error::Bad("no STREAMINFO"))?, at))
    }

    fn parse(b: &[u8]) -> Result<StreamInfo, Error> {
        if b.len() < 34 {
            return Err(Error::Bad("STREAMINFO too short"));
        }
        let x = u64::from_be_bytes(b[10..18].try_into().expect("8 bytes"));
        let info = StreamInfo {
            min_block: u16::from_be_bytes([b[0], b[1]]),
            max_block: u16::from_be_bytes([b[2], b[3]]),
            max_frame: u32::from_be_bytes([0, b[7], b[8], b[9]]),
            rate: (x >> 44) as u32,
            channels: ((x >> 41) & 7) as u8 + 1,
            bits: ((x >> 36) & 31) as u8 + 1,
            total: x & 0xF_FFFF_FFFF,
        };
        if info.rate == 0 {
            return Err(Error::Bad("a sample rate of 0"));
        }
        if info.bits < 4 {
            return Err(Error::Bad("fewer than 4 bits a sample"));
        }
        Ok(info)
    }
}

/// How a frame's channels are coded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assign {
    /// Each channel as itself.
    Independent,
    /// Left, then left minus right.
    LeftSide,
    /// Left minus right, then right.
    SideRight,
    /// (left + right) / 2, then left minus right.
    MidSide,
}

/// A frame's header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// The frame's first sample frame in the stream.
    pub first: u64,
    /// Sample frames in it.
    pub block: usize,
    pub rate: u32,
    pub channels: u8,
    pub bits: u8,
    pub assign: Assign,
    /// The header's length in bytes, its CRC-8 included.
    pub len: usize,
}

/// The frame header at the start of `b`, checked against its CRC-8.
pub fn header(b: &[u8], info: &StreamInfo) -> Result<Header, Error> {
    if b.len() < 2 {
        return Err(Error::Short);
    }
    if b[0] != 0xFF || b[1] & 0xFE != 0xF8 {
        return Err(Error::Bad("no frame where one should start"));
    }
    let variable = b[1] & 1 == 1;
    let byte = |i: usize| b.get(i).copied().ok_or(Error::Short);
    let (b2, b3) = (byte(2)?, byte(3)?);
    if b3 & 1 != 0 {
        return Err(Error::Bad("a reserved bit set in a frame header"));
    }
    // The frame (or, for variable blocks, sample) number, coded like UTF-8.
    let lead = byte(4)?;
    let ones = lead.leading_ones();
    let (mut number, extra) = match ones {
        0 => (u64::from(lead), 0),
        2..=7 => (u64::from(lead) & ((1 << (7 - ones)) - 1), ones as usize - 1),
        _ => return Err(Error::Bad("a broken frame number")),
    };
    let mut at = 5;
    for _ in 0..extra {
        let c = byte(at)?;
        if c & 0xC0 != 0x80 {
            return Err(Error::Bad("a broken frame number"));
        }
        number = number << 6 | u64::from(c & 0x3F);
        at += 1;
    }
    let block = match b2 >> 4 {
        0 => return Err(Error::Bad("a reserved block size")),
        1 => 192,
        c @ 2..=5 => 576 << (c - 2),
        6 => {
            at += 1;
            usize::from(byte(at - 1)?) + 1
        }
        7 => {
            at += 2;
            usize::from(u16::from_be_bytes([byte(at - 2)?, byte(at - 1)?])) + 1
        }
        c => 256 << (c - 8),
    };
    let rate = match b2 & 15 {
        0 => info.rate,
        1 => 88_200,
        2 => 176_400,
        3 => 192_000,
        4 => 8_000,
        5 => 16_000,
        6 => 22_050,
        7 => 24_000,
        8 => 32_000,
        9 => 44_100,
        10 => 48_000,
        11 => 96_000,
        12 => {
            at += 1;
            u32::from(byte(at - 1)?) * 1000
        }
        c @ (13 | 14) => {
            at += 2;
            let v = u32::from(u16::from_be_bytes([byte(at - 2)?, byte(at - 1)?]));
            if c == 13 {
                v
            } else {
                v * 10
            }
        }
        _ => return Err(Error::Bad("an invalid sample rate code")),
    };
    let (channels, assign) = match b3 >> 4 {
        c @ 0..=7 => (c + 1, Assign::Independent),
        8 => (2, Assign::LeftSide),
        9 => (2, Assign::SideRight),
        10 => (2, Assign::MidSide),
        _ => return Err(Error::Bad("a reserved channel layout")),
    };
    let bits = match (b3 >> 1) & 7 {
        0 => info.bits,
        1 => 8,
        2 => 12,
        4 => 16,
        5 => 20,
        6 => 24,
        7 => 32,
        _ => return Err(Error::Bad("a reserved sample size")),
    };
    if crc8(&b[..at]) != byte(at)? {
        return Err(Error::Bad("a frame header that fails its check"));
    }
    let first = if variable {
        number
    } else {
        number * u64::from(info.max_block)
    };
    Ok(Header {
        first,
        block,
        rate,
        channels,
        bits,
        assign,
        len: at + 1,
    })
}

/// Where the next frame starts in `b`: the first frame sync whose header
/// passes its check. None if no frame starts in `b`.
pub fn find_frame(b: &[u8], info: &StreamInfo) -> Option<usize> {
    (0..b.len().saturating_sub(1))
        .find(|&i| b[i] == 0xFF && b[i + 1] & 0xFE == 0xF8 && header(&b[i..], info).is_ok())
}

/// Decodes frames one at a time into buffers it keeps.
pub struct Decoder {
    info: StreamInfo,
    ch: Vec<Vec<i64>>,
    block: usize,
}

impl Decoder {
    pub fn new(info: StreamInfo) -> Decoder {
        Decoder {
            info,
            ch: vec![Vec::new(); usize::from(info.channels)],
            block: 0,
        }
    }

    /// Decodes the frame at the start of `b`: its header and how many
    /// bytes it took. Its samples are in [`Decoder::channel`] until the
    /// next frame.
    pub fn frame(&mut self, b: &[u8]) -> Result<(Header, usize), Error> {
        let h = header(b, &self.info)?;
        if h.channels != self.info.channels {
            return Err(Error::Bad("a frame with another number of channels"));
        }
        if h.bits != self.info.bits {
            return Err(Error::Bad("a frame with another sample size"));
        }
        let n = h.block;
        let mut r = Bits::new(b, h.len);
        for (c, buf) in self.ch.iter_mut().enumerate() {
            let side = matches!(
                (h.assign, c),
                (Assign::LeftSide, 1) | (Assign::SideRight, 0) | (Assign::MidSide, 1)
            );
            if buf.len() < n {
                buf.resize(n, 0);
            }
            subframe(&mut r, &mut buf[..n], u32::from(h.bits) + u32::from(side))?;
        }
        let end = r.pos.div_ceil(8);
        let crc = b.get(end..end + 2).ok_or(Error::Short)?;
        if crc16(&b[..end]) != u16::from_be_bytes([crc[0], crc[1]]) {
            return Err(Error::Damaged {
                ms: h.first * 1000 / u64::from(h.rate.max(1)),
            });
        }
        if h.assign != Assign::Independent {
            let (a, rest) = self.ch.split_at_mut(1);
            let (a, s) = (&mut a[0][..n], &mut rest[0][..n]);
            for (x, y) in a.iter_mut().zip(s.iter_mut()) {
                (*x, *y) = match h.assign {
                    // left, side: right = left - side
                    Assign::LeftSide => (*x, x.wrapping_sub(*y)),
                    // side, right: left = side + right
                    Assign::SideRight => (x.wrapping_add(*y), *y),
                    _ => {
                        let mid = *x << 1 | (*y & 1);
                        (mid.wrapping_add(*y) >> 1, mid.wrapping_sub(*y) >> 1)
                    }
                };
            }
        }
        self.block = n;
        Ok((h, end + 2))
    }

    /// Channel `c` of the last frame decoded, as whole numbers of
    /// [`StreamInfo::bits`] bits.
    pub fn channel(&self, c: usize) -> &[i64] {
        &self.ch[c][..self.block]
    }
}

/// A whole stream, decoded.
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub info: StreamInfo,
    /// The samples, channels interleaved, from -1 to just under 1.
    pub samples: Vec<f32>,
    /// True when the file ends before its last frame does, or before as
    /// many frames as STREAMINFO says: [`Decoded::samples`] holds the whole
    /// frames before that.
    pub cut_short: bool,
    /// Bytes between frames that weren't a frame and were stepped over.
    pub skipped: usize,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.info.channels)
    }
}

/// Every sample of the FLAC stream in `b`, taking at most `max_samples`
/// samples (frames × channels) into memory.
pub fn decode(b: &[u8], max_samples: usize) -> Result<Decoded, Error> {
    let (info, mut at) = StreamInfo::read(b)?;
    let ch = usize::from(info.channels);
    let max = (max_samples / ch) as u64;
    if info.total > max {
        return Err(Error::TooLong {
            frames: info.total,
            max,
        });
    }
    let mut samples = Vec::with_capacity(info.total as usize * ch);
    let scale = 1.0 / (1u64 << (info.bits - 1)) as f64;
    let mut dec = Decoder::new(info);
    let (mut skipped, mut short) = (0, false);
    while at < b.len() {
        match dec.frame(&b[at..]) {
            Ok((h, len)) => {
                let frames = (samples.len() / ch + h.block) as u64;
                if frames > max {
                    return Err(Error::TooLong { frames, max });
                }
                for i in 0..h.block {
                    for c in 0..ch {
                        samples.push((dec.ch[c][i] as f64 * scale) as f32);
                    }
                }
                at += len;
            }
            Err(Error::Short) => {
                // Cut short inside a frame, or a few stray bytes at the end.
                short = header(&b[at..], &info).is_ok();
                break;
            }
            Err(Error::Bad(_)) if header(&b[at..], &info).is_err() => {
                // Not a frame here: step to the next one, if any (a tag
                // at the end has none after it).
                match find_frame(&b[at + 1..], &info) {
                    Some(k) => {
                        skipped += k + 1;
                        at += k + 1;
                    }
                    None => break,
                }
            }
            Err(e) => return Err(e),
        }
    }
    let frames = (samples.len() / ch) as u64;
    if frames == 0 {
        return Err(if short { Error::Short } else { Error::Empty });
    }
    let cut_short = if info.total != 0 {
        frames < info.total
    } else {
        short
    };
    Ok(Decoded {
        info,
        samples,
        cut_short,
        skipped,
    })
}

/// A quick picture of a FLAC file for the browser: `columns` (min, max)
/// pairs from -1 to 1, each from at most `take` sample frames at that
/// column's place in the file. Each column finds its frame by guessing
/// where it lies from the bytes around it and its frames' numbers, and
/// decodes that one frame, so a long file costs about the same as a short
/// one. None when the file isn't FLAC or can't be read.
pub fn overview<R: Read + Seek>(r: &mut R, columns: usize, take: usize) -> Option<Vec<(f32, f32)>> {
    let columns = columns.clamp(1, 2048);
    let len = r.seek(SeekFrom::End(0)).ok()?;
    // The metadata comes first; pictures can make it long, so read it in
    // growing steps until STREAMINFO and the first frame's place are known.
    let mut head = Vec::new();
    let mut want = 8192u64;
    let (info, start) = loop {
        r.seek(SeekFrom::Start(0)).ok()?;
        head.resize(want.min(len) as usize, 0);
        r.read_exact(&mut head).ok()?;
        match StreamInfo::read(&head) {
            Ok(x) => break x,
            Err(Error::Short) if want < len => want = (want * 4).min(len).min(64 << 20),
            Err(_) => return None,
        }
    };
    let ch = usize::from(info.channels);
    let start = start as u64;
    let total = if info.total != 0 {
        info.total
    } else {
        // No length given: a guess from the bytes, at 16-bit stereo's
        // usual 60 %. Columns past the end come out flat.
        ((len - start) as f64 / (0.6 * f64::from(info.bits) / 8.0 * ch as f64)) as u64
    };
    if total == 0 || len <= start {
        return Some(vec![(0.0, 0.0); columns]);
    }
    // The most bytes a frame can take: what STREAMINFO says, else a whole
    // block stored as is.
    let worst = u64::from(info.max_block) * ch as u64 * (u64::from(info.bits) + 1) / 8 + 64;
    let frame_cap = match info.max_frame {
        0 => worst,
        m => u64::from(m) + 64,
    }
    .clamp(64, 1 << 22) as usize;
    let mut buf = vec![0u8; frame_cap.max(CHUNK)];
    let mut dec = Decoder::new(info);
    let scale = 1.0 / (1u64 << (info.bits - 1)) as f32;
    let per = total.div_ceil(columns as u64);
    // A frame's usual length in bytes, from the whole stream's.
    let avg = (len - start) / total.div_ceil(u64::from(info.max_block.max(1))).max(1);
    let mut out = Vec::with_capacity(columns);
    // A frame's start and its first sample, at or before the column's
    // place; columns go left to right, so it carries on from one to the
    // next.
    let mut lo = (start, 0u64);
    for col in 0..columns as u64 {
        let want = col * per;
        if want >= total {
            out.push((0.0, 0.0));
            continue;
        }
        // Guess from the bytes for the frame holding the place. A frame
        // found after a guess that starts past the place means the place's
        // frame ends where that one starts: the next guess steps back from
        // there by about a frame, then by twice that, and so on.
        let mut hi = (len, total.max(want + 1));
        let mut back: Option<(u64, u64)> = None;
        for _ in 0..8 {
            if want - lo.1 < u64::from(info.min_block) || hi.0 <= lo.0 + 16 {
                break;
            }
            let pos = match back {
                Some((end, by)) => end - by.min(end - lo.0 - 1),
                None => {
                    let frac = (want - lo.1) as f64 / (hi.1 - lo.1).max(1) as f64;
                    (lo.0 + ((hi.0 - lo.0) as f64 * frac) as u64).clamp(lo.0 + 1, hi.0 - 1)
                }
            };
            match next_header(r, &mut buf, &info, pos, len, frame_cap) {
                Some((at, h)) if h.first <= want => {
                    lo = (at, h.first);
                    if want < h.first + h.block as u64 {
                        break;
                    }
                    back = None;
                }
                Some((at, h)) => {
                    hi = (pos, h.first.max(lo.1 + 1));
                    let by = back.map_or(avg + avg / 4 + 64, |(_, by)| by * 2);
                    back = Some((at.max(lo.0 + 1), by));
                }
                None => hi = (pos, hi.1),
            }
        }
        // Then frame by frame from there to the one holding the place.
        let mut got = None;
        let mut pos = lo.0;
        for _ in 0..64 {
            let n = (frame_cap as u64).min(len - pos) as usize;
            if r.seek(SeekFrom::Start(pos)).is_err() || r.read_exact(&mut buf[..n]).is_err() {
                break;
            }
            let Ok((h, flen)) = dec.frame(&buf[..n]) else {
                break;
            };
            if h.first > want {
                break;
            }
            lo = (pos, h.first);
            if want < h.first + h.block as u64 {
                got = Some(h);
                break;
            }
            pos += flen as u64;
        }
        let Some(h) = got else {
            out.push((0.0, 0.0));
            continue;
        };
        // The frame's samples from the place on.
        let from = (want - h.first) as usize;
        let to = (from + take.max(1)).min(h.block);
        let (mut a, mut z) = (0f32, 0f32);
        for c in 0..ch {
            for &s in &dec.channel(c)[from..to] {
                let v = s as f32 * scale;
                a = a.min(v);
                z = z.max(v);
            }
        }
        out.push((a.max(-1.0), z.min(1.0)));
    }
    Some(out)
}

/// How much the browser's picture reads at a time looking for a frame.
const CHUNK: usize = 4096;

/// The first frame header at or after `pos`, read a chunk at a time and
/// looking at most `max` bytes on: where it starts and the header.
fn next_header<R: Read + Seek>(
    r: &mut R,
    buf: &mut [u8],
    info: &StreamInfo,
    pos: u64,
    len: u64,
    max: usize,
) -> Option<(u64, Header)> {
    let mut at = pos;
    while at < len && at - pos <= max as u64 {
        let n = (CHUNK as u64).min(len - at) as usize;
        r.seek(SeekFrom::Start(at)).ok()?;
        r.read_exact(&mut buf[..n]).ok()?;
        if let Some(k) = find_frame(&buf[..n], info) {
            return Some((at + k as u64, header(&buf[k..n], info).ok()?));
        }
        if at + (n as u64) >= len {
            break;
        }
        // Overlap the next chunk by a header's length, so a header across
        // the edge is found.
        at += n as u64 - 16;
    }
    None
}

/// Reads a stream bit by bit, most significant first.
struct Bits<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(b: &'a [u8], byte: usize) -> Bits<'a> {
        Bits { b, pos: byte * 8 }
    }

    /// The next `n` bits (at most 57) as a whole number.
    fn bits(&mut self, n: u32) -> Result<u64, Error> {
        if n == 0 {
            return Ok(0);
        }
        let end = self.pos + n as usize;
        if end > self.b.len() * 8 {
            return Err(Error::Short);
        }
        let (first, last) = (self.pos / 8, (end - 1) / 8);
        let mut v = 0u64;
        for &x in &self.b[first..=last] {
            v = v << 8 | u64::from(x);
        }
        let used = (last - first + 1) * 8;
        v >>= used - self.pos % 8 - n as usize;
        self.pos = end;
        Ok(if n < 64 { v & ((1u64 << n) - 1) } else { v })
    }

    /// The next `n` bits (at most 57) as a two's complement number.
    fn signed(&mut self, n: u32) -> Result<i64, Error> {
        if n == 0 {
            return Ok(0);
        }
        let s = 64 - n;
        Ok(((self.bits(n)? << s) as i64) >> s)
    }

    /// The number of 0 bits before the next 1, the 1 read too.
    fn unary(&mut self) -> Result<u32, Error> {
        let mut count = 0u32;
        loop {
            let x = *self.b.get(self.pos / 8).ok_or(Error::Short)?;
            let off = self.pos % 8;
            let v = x << off;
            if v != 0 {
                let z = v.leading_zeros();
                self.pos += z as usize + 1;
                return Ok(count.saturating_add(z));
            }
            count = count.saturating_add(8 - off as u32);
            self.pos += 8 - off;
        }
    }
}

/// One channel's subframe into `out` (one frame's samples), `bps` bits a
/// sample (one more for a side channel).
fn subframe(r: &mut Bits, out: &mut [i64], bps: u32) -> Result<(), Error> {
    if r.bits(1)? != 0 {
        return Err(Error::Bad("a subframe's padding bit set"));
    }
    let kind = r.bits(6)? as u32;
    let wasted = if r.bits(1)? == 1 {
        r.unary()?.saturating_add(1)
    } else {
        0
    };
    if wasted >= bps {
        return Err(Error::Bad("more wasted bits than bits"));
    }
    let bps = bps - wasted;
    match kind {
        0 => {
            let v = r.signed(bps)?;
            out.fill(v);
        }
        1 => {
            for x in out.iter_mut() {
                *x = r.signed(bps)?;
            }
        }
        8..=12 => fixed(r, out, bps, (kind - 8) as usize)?,
        32..=63 => lpc(r, out, bps, (kind - 31) as usize)?,
        _ => return Err(Error::Bad("a reserved subframe type")),
    }
    if wasted > 0 {
        for x in out.iter_mut() {
            *x <<= wasted;
        }
    }
    Ok(())
}

fn warm_up(r: &mut Bits, out: &mut [i64], bps: u32, order: usize) -> Result<(), Error> {
    if order > out.len() {
        return Err(Error::Bad("a predictor longer than its block"));
    }
    for x in &mut out[..order] {
        *x = r.signed(bps)?;
    }
    Ok(())
}

fn fixed(r: &mut Bits, out: &mut [i64], bps: u32, order: usize) -> Result<(), Error> {
    warm_up(r, out, bps, order)?;
    residual(r, out, order)?;
    for i in order..out.len() {
        let p = match order {
            0 => 0,
            1 => out[i - 1],
            2 => (out[i - 1] << 1).wrapping_sub(out[i - 2]),
            3 => (out[i - 1].wrapping_sub(out[i - 2]))
                .wrapping_mul(3)
                .wrapping_add(out[i - 3]),
            _ => (out[i - 1].wrapping_add(out[i - 3]))
                .wrapping_mul(4)
                .wrapping_sub(out[i - 2].wrapping_mul(6))
                .wrapping_sub(out[i - 4]),
        };
        out[i] = out[i].wrapping_add(p);
    }
    Ok(())
}

fn lpc(r: &mut Bits, out: &mut [i64], bps: u32, order: usize) -> Result<(), Error> {
    warm_up(r, out, bps, order)?;
    let precision = r.bits(4)? as u32 + 1;
    if precision == 16 {
        return Err(Error::Bad("an invalid predictor precision"));
    }
    let shift = r.signed(5)?;
    if shift < 0 {
        return Err(Error::Bad("a negative predictor shift"));
    }
    let mut coef = [0i64; 32];
    for c in &mut coef[..order] {
        *c = r.signed(precision)?;
    }
    residual(r, out, order)?;
    let coef = &coef[..order];
    for i in order..out.len() {
        let mut sum = 0i64;
        for (j, &c) in coef.iter().enumerate() {
            sum = sum.wrapping_add(c.wrapping_mul(out[i - 1 - j]));
        }
        out[i] = out[i].wrapping_add(sum >> shift);
    }
    Ok(())
}

/// The residual after `order` warm-up samples, into `out[order..]`.
fn residual(r: &mut Bits, out: &mut [i64], order: usize) -> Result<(), Error> {
    let (param_bits, escape) = match r.bits(2)? {
        0 => (4, 15),
        1 => (5, 31),
        _ => return Err(Error::Bad("a reserved residual coding")),
    };
    let porder = r.bits(4)? as u32;
    let n = out.len();
    let per = n >> porder;
    if per << porder != n || per < order {
        return Err(Error::Bad("a residual that doesn't fit its block"));
    }
    let mut i = order;
    for p in 0..1usize << porder {
        let end = (p + 1) * per;
        let param = r.bits(param_bits)? as u32;
        if param == escape {
            let bits = r.bits(5)? as u32;
            for x in &mut out[i..end] {
                *x = r.signed(bits)?;
            }
        } else {
            for x in &mut out[i..end] {
                let q = u64::from(r.unary()?);
                let v = q << param | r.bits(param)?;
                if v > u64::from(u32::MAX) {
                    return Err(Error::Bad("a residual too big for 32 bits"));
                }
                *x = (v >> 1) as i64 ^ -((v & 1) as i64);
            }
        }
        i = end;
    }
    Ok(())
}

const fn crc8_table() -> [u8; 256] {
    let mut t = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u8;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x80 != 0 { c << 1 ^ 0x07 } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const fn crc16_table() -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u16) << 8;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000 != 0 {
                c << 1 ^ 0x8005
            } else {
                c << 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC8: [u8; 256] = crc8_table();
static CRC16: [u16; 256] = crc16_table();

/// CRC-8, polynomial x^8 + x^2 + x + 1, as frame headers end with.
pub fn crc8(b: &[u8]) -> u8 {
    b.iter().fold(0, |c, &x| CRC8[usize::from(c ^ x)])
}

/// CRC-16, polynomial x^16 + x^15 + x^2 + 1, as frames end with.
pub fn crc16(b: &[u8]) -> u16 {
    b.iter()
        .fold(0, |c, &x| c << 8 ^ CRC16[usize::from((c >> 8) as u8 ^ x)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_come_most_significant_first_and_signed_ones_extend() {
        let b = [0b1011_0011, 0b0100_0000, 0xFF, 0x01];
        let mut r = Bits::new(&b, 0);
        assert_eq!(r.bits(3).unwrap(), 0b101);
        assert_eq!(r.signed(4).unwrap(), 0b1001 - 16);
        assert_eq!(r.unary().unwrap(), 0, "a 1 at once: no zeros");
        assert_eq!(r.unary().unwrap(), 1, "0 1");
        assert_eq!(r.bits(0).unwrap(), 0);
        assert_eq!(r.pos, 10);
        assert_eq!(r.unary().unwrap(), 6, "the zeros run on into the 0xFF");
        assert_eq!(r.bits(15).unwrap(), 0x7F01);
        assert_eq!(r.bits(1), Err(Error::Short));
        let mut r = Bits::new(&[0x80, 0, 0, 0, 0, 0, 0, 0], 0);
        assert_eq!(r.signed(33).unwrap(), -(1 << 32));
    }

    /// Writes bits most significant first, for streams built by hand.
    #[derive(Default)]
    struct W {
        v: Vec<u8>,
        bits: usize,
    }

    impl W {
        fn put(&mut self, n: u32, val: u64) {
            for k in (0..n).rev() {
                if self.bits.is_multiple_of(8) {
                    self.v.push(0);
                }
                let bit = if k < 64 { (val >> k) & 1 } else { 0 };
                *self.v.last_mut().unwrap() |= (bit as u8) << (7 - self.bits % 8);
                self.bits += 1;
            }
        }
        fn signed(&mut self, n: u32, val: i64) {
            self.put(n, val as u64 & ((1u64 << n) - 1));
        }
        fn unary(&mut self, q: u32) {
            self.put(q, 0);
            self.put(1, 1);
        }
        fn rice(&mut self, param: u32, x: i64) {
            let v = ((x << 1) ^ (x >> 63)) as u64;
            self.unary((v >> param) as u32);
            self.put(param, v & ((1 << param) - 1));
        }
        fn align(&mut self) {
            self.bits = self.v.len() * 8;
        }
    }

    fn stream_info(channels: u8, bits: u8, total: u64) -> Vec<u8> {
        let mut w = W::default();
        w.put(8, 0x80); // the last metadata block, STREAMINFO
        w.put(24, 34);
        w.put(16, 1);
        w.put(16, 300);
        w.put(48, 0);
        w.put(20, 48_000);
        w.put(3, u64::from(channels) - 1);
        w.put(5, u64::from(bits) - 1);
        w.put(36, total);
        w.put(128, 0);
        let mut b = b"fLaC".to_vec();
        b.extend_from_slice(&w.v);
        b
    }

    /// A variable-block frame starting at sample `first` (below 4096), its
    /// block size given in 8 bits; `body` writes its subframes.
    fn frame(first: u64, block: u64, ch_code: u64, ss_code: u64, body: impl Fn(&mut W)) -> Vec<u8> {
        let mut w = W::default();
        w.put(16, 0xFFF9);
        w.put(4, 6);
        w.put(4, 0);
        w.put(4, ch_code);
        w.put(3, ss_code);
        w.put(1, 0);
        if first < 128 {
            w.put(8, first);
        } else {
            w.put(8, 0xC0 | first >> 6);
            w.put(8, 0x80 | (first & 0x3F));
        }
        w.put(8, block - 1);
        let c = crc8(&w.v);
        w.put(8, u64::from(c));
        body(&mut w);
        w.align();
        let c = crc16(&w.v);
        w.put(16, u64::from(c));
        w.v
    }

    fn ints(b: &[u8]) -> Vec<Vec<i64>> {
        let (info, mut at) = StreamInfo::read(b).unwrap();
        let mut dec = Decoder::new(info);
        let mut out = vec![Vec::new(); usize::from(info.channels)];
        while at < b.len() {
            let (h, len) = dec.frame(&b[at..]).unwrap();
            assert_eq!(
                h.first,
                out[0].len() as u64,
                "variable blocks say their first sample"
            );
            for (c, o) in out.iter_mut().enumerate() {
                o.extend_from_slice(dec.channel(c));
            }
            at += len;
        }
        out
    }

    #[test]
    fn what_ffmpeg_never_writes_decodes_too() {
        // 8-bit mono in variable blocks: verbatim; a fixed predictor whose
        // residual has an escaped partition and a Rice one with parameter
        // 0; a constant with wasted bits; LPC with a shift.
        let verbatim = [-128i64, 127, 0, 1, -1];
        let fixed2 = [3i64, 5, 9, 12, 11, 4];
        let mut b = stream_info(1, 8, 19);
        b.extend(frame(0, 5, 0, 1, |w| {
            w.put(8, 0b0000_0010); // verbatim, no wasted bits
            for &x in &verbatim {
                w.signed(8, x);
            }
        }));
        b.extend(frame(5, 6, 0, 1, |w| {
            w.put(8, 0b0001_0100); // fixed, order 2
            w.signed(8, fixed2[0]);
            w.signed(8, fixed2[1]);
            let res: Vec<i64> = (2..6)
                .map(|i| fixed2[i] - (2 * fixed2[i - 1] - fixed2[i - 2]))
                .collect();
            w.put(2, 0); // 4-bit Rice parameters
            w.put(4, 1); // two partitions of 3, the first after the 2 warm-ups
            w.put(4, 15); // escaped: 4 raw bits a residual
            w.put(5, 4);
            w.signed(4, res[0]);
            w.put(4, 0); // Rice, parameter 0
            for &x in &res[1..] {
                w.rice(0, x);
            }
        }));
        b.extend(frame(11, 4, 0, 1, |w| {
            w.put(8, 0b0000_0001); // constant, wasted bits follow
            w.unary(1); // 2 wasted bits
            w.signed(6, -7);
        }));
        let lpc = [10i64, 12, 15, 13];
        b.extend(frame(15, 4, 0, 1, |w| {
            w.put(8, 0b0100_0000); // LPC, order 1
            w.signed(8, lpc[0]);
            w.put(4, 2); // 3-bit coefficients
            w.signed(5, 1); // shift 1
            w.signed(3, 2); // (2 × previous) >> 1
            w.put(2, 1); // 5-bit Rice parameters
            w.put(4, 0);
            w.put(5, 1);
            for i in 1..4 {
                w.rice(1, lpc[i] - lpc[i - 1]);
            }
        }));
        let mut want = verbatim.to_vec();
        want.extend_from_slice(&fixed2);
        want.extend_from_slice(&[-28; 4]);
        want.extend_from_slice(&lpc);
        assert_eq!(ints(&b), vec![want.clone()]);
        let d = decode(&b, 100).unwrap();
        assert_eq!(d.frames(), 19);
        assert_eq!(d.samples[0..2], [-1.0, 127.0 / 128.0]);

        // 20-bit side/right and 12-bit left/side, and 32-bit side/right
        // whose side channel needs 33 bits.
        for (bits, ss_code, left, right) in [
            (
                20u8,
                5u64,
                [-524_288i64, 3, 524_287],
                [524_287i64, -2, -524_288],
            ),
            (12, 2, [-2048, 0, 2047], [2047, -1, -2048]),
            (
                32,
                7,
                [i64::from(i32::MIN), 7, i64::from(i32::MAX)],
                [i64::from(i32::MAX), -9, i64::from(i32::MIN)],
            ),
        ] {
            let mut b = stream_info(2, bits, 3);
            let side_right = bits != 12;
            b.extend(frame(0, 3, if side_right { 9 } else { 8 }, ss_code, |w| {
                let n = u32::from(bits);
                let side: Vec<i64> = (0..3).map(|i| left[i] - right[i]).collect();
                let (a, an, z, zn) = if side_right {
                    (&side[..], n + 1, &right[..], n)
                } else {
                    (&left[..], n, &side[..], n + 1)
                };
                w.put(8, 0b0000_0010);
                for &x in a {
                    w.signed(an, x);
                }
                w.put(8, 0b0000_0010);
                for &x in z {
                    w.signed(zn, x);
                }
            }));
            assert_eq!(ints(&b), vec![left.to_vec(), right.to_vec()], "{bits}-bit");
        }
    }

    #[test]
    fn the_checks_match_the_standard_ones() {
        // The usual check values of CRC-8 (poly 0x07) and CRC-16/UMTS
        // (poly 0x8005, no reflection): "123456789".
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }
}
