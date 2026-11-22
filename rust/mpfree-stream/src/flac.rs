//! FLAC as a compressed source (disk streaming's P6, steps 1 to 4): its
//! seek index, and decoding any run of its frames into the same float
//! units the cache already holds.
//!
//! A WAV unit's place in the file is worked out from its number; a FLAC
//! unit's has to be found. So each FLAC file gets an index of points (a
//! frame's first sample and the byte it starts at):
//!
//! - **The file's own seek table** is used only if it checks out: every
//!   point lands on a frame whose header passes its check and starts at
//!   the point's sample, and no gap between points is wider than
//!   [`MAX_GAP_S`] ([`Index::from_table`]). Nothing else is read.
//! - **Otherwise an index is built** in the background before the file is
//!   offered for jumping around ([`Index::build`]): the whole file is read
//!   once, its frames walked (each header checked and each frame starting
//!   where the last ended), a point kept every [`POINT_S`] second, and the
//!   compressed bytes hashed. The saved index ([`Index::to_text`]) carries
//!   that hash, the index format and the indexer's version. FLAC's own
//!   checksum isn't enough: it covers the decoded audio, so the same audio
//!   encoded with other settings has the same checksum but frames in other
//!   places.
//!
//! An ordinary open finds a saved index by the file's size and change time
//! ([`Index::fits`]); preparing a set checks the full hash. Every jump
//! checks the frame it lands on (its sync code, header checksum and sample
//! number) and every frame after it (its CRC-16, and that it starts where
//! the last ended), so a stale index costs a rescan ([`Error::Stale`]),
//! never wrong audio.

use std::fmt;
use std::io;
use std::path::Path;

use mpfree_flac::{header, Decoder, StreamInfo};
use sha2::{Digest, Sha256};

use crate::fsio::Fs;
use crate::publish::hex;
use crate::snapshot::Meta;

/// The saved index's format.
pub const INDEX_FORMAT: u32 = 1;
/// The indexer's version: an index another one made is built again.
pub const INDEXER: &str = "fi1";
/// Seconds between the points of an index built here.
pub const POINT_S: u64 = 1;
/// The widest gap between a file's own seek points it may be used with,
/// in seconds: wider would decode too much before each jump.
pub const MAX_GAP_S: u64 = 2;

/// Bytes read at a time while building an index.
const CHUNK: usize = 1 << 20;
/// The most bytes a frame header takes.
const HEADER_MOST: usize = 16;

/// Where one frame starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// Its first sample frame.
    pub sample: u64,
    /// The byte it starts at, from the start of the file.
    pub offset: u64,
}

/// How an index was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Made {
    /// From the file's own seek table, checked.
    Table,
    /// Built by walking every frame.
    Scan,
}

/// Why an index can't be made or a run of frames decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Not a FLAC file this layer streams (only 1 or 2 channels play).
    NotFlac(String),
    /// A frame isn't where the index says, or isn't the one it says: the
    /// file changed since it was indexed. Build the index again.
    Stale,
    /// A frame fails its check: the file is damaged there.
    Damaged {
        sample: u64,
    },
    /// The file ended sooner than it said while it was indexed.
    Short,
    Io(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFlac(why) => write!(f, "not a FLAC file that streams: {why}"),
            Error::Stale => write!(f, "the FLAC file changed since it was indexed"),
            Error::Damaged { sample } => write!(f, "the FLAC file is damaged at frame {sample}"),
            Error::Short => write!(f, "the FLAC file is shorter than its frames"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e.to_string())
    }
}

/// A FLAC file's seek index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Index {
    pub info: StreamInfo,
    /// Sample frames in the file.
    pub frames: u64,
    /// Where its first frame starts, and where its last ends (or the
    /// file does).
    pub audio: (u64, u64),
    /// From frame 0, in order of sample and of byte.
    pub points: Vec<Point>,
    /// The file's size and change time when it was indexed.
    pub meta: Meta,
    /// SHA-256 of the whole compressed file (an index built here).
    pub hash: Option<[u8; 32]>,
    pub made: Made,
}

/// The file's metadata blocks, read in growing steps (pictures can make
/// them long): STREAMINFO, where the frames start, and the seek table's
/// body if there is one.
fn head(fs: &mut dyn Fs, path: &Path, size: u64) -> Result<(StreamInfo, u64, Vec<u8>), Error> {
    let mut want = 8192u64;
    loop {
        let mut b = vec![0u8; want.min(size) as usize];
        let n = fs.read_at(path, 0, &mut b)?;
        b.truncate(n);
        match StreamInfo::read(&b) {
            Ok((info, at)) => {
                if !(1..=2).contains(&info.channels) {
                    return Err(Error::NotFlac(format!("{} channels", info.channels)));
                }
                let table = seek_table(&b, at);
                return Ok((info, at as u64, table));
            }
            Err(mpfree_flac::Error::Short) if want < size => want *= 4,
            Err(e) => return Err(Error::NotFlac(e.to_string())),
        }
    }
}

/// The SEEKTABLE block's body among the metadata blocks before `end`.
fn seek_table(b: &[u8], end: usize) -> Vec<u8> {
    let mut at = 0;
    if b.len() >= 10 && &b[..3] == b"ID3" {
        // An ID3v2 tag in front, stepped over as STREAMINFO::read does.
        let size = b[6..10]
            .iter()
            .fold(0usize, |s, &x| s << 7 | usize::from(x & 0x7F));
        at = 10 + size + if b[5] & 0x10 != 0 { 10 } else { 0 };
    }
    if b.get(at..at + 4) != Some(b"fLaC") {
        return Vec::new();
    }
    at += 4;
    while at + 4 <= end {
        let (last, kind) = (b[at] & 0x80 != 0, b[at] & 0x7F);
        let len =
            usize::from(b[at + 1]) << 16 | usize::from(b[at + 2]) << 8 | usize::from(b[at + 3]);
        at += 4;
        if kind == 3 {
            return b.get(at..at + len).map(<[u8]>::to_vec).unwrap_or_default();
        }
        at += len;
        if last {
            break;
        }
    }
    Vec::new()
}

impl Index {
    /// The index from the file's own seek table, if it has one that checks
    /// out (see the module's notes); None otherwise.
    pub fn from_table(fs: &mut dyn Fs, path: &Path, meta: Meta) -> Result<Option<Index>, Error> {
        let size = meta.0;
        let (info, first, table) = head(fs, path, size)?;
        if info.total == 0 {
            return Ok(None);
        }
        let mut points = Vec::new();
        for p in table.chunks_exact(18) {
            let sample = u64::from_be_bytes(p[..8].try_into().expect("8 bytes"));
            if sample == u64::MAX {
                continue; // a placeholder
            }
            let offset = u64::from_be_bytes(p[8..16].try_into().expect("8 bytes"));
            points.push(Point {
                sample,
                offset: first.saturating_add(offset),
            });
        }
        let gap = MAX_GAP_S * u64::from(info.rate);
        let ordered = points
            .first()
            .is_some_and(|p| p.sample == 0 && p.offset == first)
            && points
                .windows(2)
                .all(|w| w[1].sample > w[0].sample && w[1].offset > w[0].offset)
            && points.windows(2).all(|w| w[1].sample - w[0].sample <= gap)
            && points.last().is_some_and(|p| {
                p.offset < size && p.sample < info.total && info.total - p.sample <= gap
            });
        if !ordered {
            return Ok(None);
        }
        let mut h = [0u8; HEADER_MOST];
        for p in &points {
            let n = fs.read_at(path, p.offset, &mut h)?;
            if !header(&h[..n], &info).is_ok_and(|x| x.first == p.sample) {
                return Ok(None);
            }
        }
        Ok(Some(Index {
            info,
            frames: info.total,
            audio: (first, size),
            points,
            meta,
            hash: None,
            made: Made::Table,
        }))
    }

    /// Build the index by reading the whole file once (see the module's
    /// notes). For a background job: it reads every byte.
    pub fn build(fs: &mut dyn Fs, path: &Path, meta: Meta) -> Result<Index, Error> {
        let size = meta.0;
        let (info, first, _) = head(fs, path, size)?;
        let mut hash = Sha256::new();
        // The bytes in hand: `buf` holds the file from `at`; `read` bytes
        // have been read (and hashed) so far.
        let (mut buf, mut at, mut read) = (Vec::<u8>::new(), 0u64, 0u64);
        let mut chunk = vec![0u8; CHUNK];
        // Read until the bytes up to `to` are in hand, or the file ends.
        let mut more = |buf: &mut Vec<u8>, at: u64, read: &mut u64, to: u64| -> Result<(), Error> {
            while *read < to.min(size) {
                let n = fs.read_at(path, *read, &mut chunk)?;
                if n == 0 {
                    return Err(Error::Short);
                }
                hash.update(&chunk[..n]);
                buf.extend_from_slice(&chunk[..n]);
                *read += n as u64;
            }
            debug_assert!(at <= *read);
            Ok(())
        };
        let rate = u64::from(info.rate);
        let mut points = Vec::new();
        let mut next_point = 0u64;
        let (mut pos, mut expect) = (first, 0u64);
        loop {
            more(&mut buf, at, &mut read, pos + HEADER_MOST as u64)?;
            let h = header(&buf[(pos - at) as usize..], &info)
                .map_err(|_| Error::NotFlac(format!("no frame at byte {pos}")))?;
            if h.first != expect {
                return Err(Error::NotFlac(format!(
                    "frame {} where frame {expect} should be",
                    h.first
                )));
            }
            if h.first >= next_point {
                points.push(Point {
                    sample: h.first,
                    offset: pos,
                });
                next_point = h.first + POINT_S * rate;
            }
            expect = h.first + h.block as u64;
            // The next frame: the first sync after this header whose header
            // passes its check and starts where this frame ends.
            let mut from = pos + h.len as u64;
            let next = loop {
                more(&mut buf, at, &mut read, from + CHUNK as u64)?;
                let end = read;
                let hay = &buf[(from - at) as usize..(end - at) as usize];
                let found = (0..hay.len().saturating_sub(1)).find(|&i| {
                    hay[i] == 0xFF
                        && hay[i + 1] & 0xFE == 0xF8
                        && header(&hay[i..], &info).is_ok_and(|x| x.first == expect)
                });
                match found {
                    Some(i) => break Some(from + i as u64),
                    None if end >= size => break None,
                    None => {
                        // Keep the last few bytes (a header may straddle)
                        // and drop the rest.
                        from = end.saturating_sub(HEADER_MOST as u64).max(from);
                        let keep = (from - at) as usize;
                        if keep > CHUNK {
                            buf.drain(..keep);
                            at = from;
                        }
                    }
                }
            };
            match next {
                Some(n) => {
                    // Drop what is behind: memory stays about a chunk.
                    let keep = (n - at) as usize;
                    if keep > CHUNK {
                        buf.drain(..keep);
                        at = n;
                    }
                    pos = n;
                }
                None => break,
            }
        }
        // The rest of the file, for the hash.
        let mut tail = Vec::new();
        more(&mut tail, read, &mut read, size)?;
        Ok(Index {
            info,
            frames: expect,
            audio: (first, size),
            points,
            meta,
            hash: Some(hash.finalize().into()),
            made: Made::Scan,
        })
    }

    /// Whether a saved index still fits the file, by its size and change
    /// time: for an ordinary open. Preparing a set checks the hash.
    pub fn fits(&self, meta: Meta) -> bool {
        self.meta == meta
    }

    /// The last point at or before sample `s`.
    pub fn point(&self, s: u64) -> Point {
        let i = self.points.partition_point(|p| p.sample <= s);
        self.points[i.saturating_sub(1)]
    }

    /// The bytes (`start..end`) holding sample frames `a..b`, and the
    /// first sample of the frame `start` is at.
    pub fn span(&self, a: u64, b: u64) -> (u64, u64, u64) {
        let p = self.point(a);
        let i = self.points.partition_point(|p| p.sample < b);
        let end = self.points.get(i).map_or(self.audio.1, |p| p.offset);
        (p.offset, end.max(p.offset), p.sample)
    }

    /// Decode sample frames `a..b` into `out` (interleaved, as floats from
    /// -1 to just under 1) from `bytes`, which start at the frame that
    /// begins at sample `from` ([`Index::span`]). Gives the samples
    /// written. The first frame must be that frame, and each after it must
    /// start where the last ended and pass its checks. Bytes that run out
    /// before frame `b` mean the frames aren't where the index says: stale.
    pub fn decode(
        &self,
        dec: &mut Decoder,
        bytes: &[u8],
        from: u64,
        a: u64,
        b: u64,
        out: &mut [f32],
    ) -> Result<usize, Error> {
        let ch = usize::from(self.info.channels);
        let b = b.min(self.frames);
        if a >= b {
            return Ok(0);
        }
        let scale = 1.0 / (1u64 << (self.info.bits - 1)) as f64;
        let (mut pos, mut expect) = (0usize, from);
        while expect < b {
            let rest = bytes
                .get(pos..)
                .filter(|r| !r.is_empty())
                .ok_or(Error::Stale)?;
            let (h, len) = match dec.frame(rest) {
                Ok(x) => x,
                Err(mpfree_flac::Error::Short) => return Err(Error::Stale),
                Err(mpfree_flac::Error::Damaged { .. }) => {
                    return Err(Error::Damaged { sample: expect })
                }
                // The frame a jump lands on isn't one: the index is stale.
                Err(_) if pos == 0 => return Err(Error::Stale),
                Err(_) => {
                    // Bytes between frames that aren't a frame: step to the
                    // next one, if it is the frame that should come.
                    let info = &self.info;
                    let skip = (1..rest.len().saturating_sub(1)).find(|&i| {
                        rest[i] == 0xFF
                            && rest[i + 1] & 0xFE == 0xF8
                            && header(&rest[i..], info).is_ok_and(|x| x.first == expect)
                    });
                    match skip {
                        Some(k) => {
                            pos += k;
                            continue;
                        }
                        None => return Err(Error::Damaged { sample: expect }),
                    }
                }
            };
            if h.first != expect {
                return Err(if pos == 0 {
                    Error::Stale
                } else {
                    Error::Damaged { sample: expect }
                });
            }
            let lo = a.max(h.first);
            let hi = b.min(h.first + h.block as u64);
            for s in lo..hi {
                let i = (s - h.first) as usize;
                let o = (s - a) as usize * ch;
                for c in 0..ch {
                    out[o + c] = (dec.channel(c)[i] as f64 * scale) as f32;
                }
            }
            expect = h.first + h.block as u64;
            pos += len;
        }
        Ok((b - a) as usize * ch)
    }

    /// The saved index, as text.
    pub fn to_text(&self) -> String {
        let i = &self.info;
        let mut t = format!(
            "mpfree-flac-index {INDEX_FORMAT} {INDEXER}\n\
             size {} changed {}\n\
             hash {}\n\
             info {} {} {} {} {} {} {}\n\
             frames {} audio {} {}\n",
            self.meta.0,
            self.meta
                .1
                .map_or("-".into(), |(s, n)| format!("{s}.{n:09}")),
            self.hash.map_or("-".into(), |h| hex(&h)),
            i.rate,
            i.channels,
            i.bits,
            i.total,
            i.min_block,
            i.max_block,
            i.max_frame,
            self.frames,
            self.audio.0,
            self.audio.1,
        );
        for p in &self.points {
            t.push_str(&format!("p {} {}\n", p.sample, p.offset));
        }
        t
    }

    /// A saved index read back; None if it isn't one this indexer made, in
    /// this format, or is damaged.
    pub fn from_text(text: &str) -> Option<Index> {
        let mut lines = text.lines();
        let head: Vec<&str> = lines.next()?.split(' ').collect();
        if head != ["mpfree-flac-index", &INDEX_FORMAT.to_string(), INDEXER] {
            return None;
        }
        let words = |l: Option<&str>| -> Option<Vec<String>> {
            Some(l?.split(' ').map(str::to_string).collect())
        };
        let w = words(lines.next())?;
        let (size, changed) = match w.as_slice() {
            [s, size, c, changed] if s == "size" && c == "changed" => (size.parse().ok()?, changed),
            _ => return None,
        };
        let changed = match changed.as_str() {
            "-" => None,
            t => {
                let (s, n) = t.split_once('.')?;
                Some((s.parse().ok()?, n.parse().ok()?))
            }
        };
        let w = words(lines.next())?;
        let hash = match w.as_slice() {
            [h, v] if h == "hash" && v == "-" => None,
            [h, v] if h == "hash" && v.len() == 64 => {
                let mut out = [0u8; 32];
                for (k, o) in out.iter_mut().enumerate() {
                    *o = u8::from_str_radix(v.get(k * 2..k * 2 + 2)?, 16).ok()?;
                }
                Some(out)
            }
            _ => return None,
        };
        let w = words(lines.next())?;
        let n: Vec<u64> = match w.split_first()? {
            (h, rest) if h == "info" && rest.len() == 7 => {
                rest.iter().map(|x| x.parse().ok()).collect::<Option<_>>()?
            }
            _ => return None,
        };
        let info = StreamInfo {
            rate: u32::try_from(n[0]).ok()?,
            channels: u8::try_from(n[1]).ok()?,
            bits: u8::try_from(n[2]).ok()?,
            total: n[3],
            min_block: u16::try_from(n[4]).ok()?,
            max_block: u16::try_from(n[5]).ok()?,
            max_frame: u32::try_from(n[6]).ok()?,
        };
        if info.rate == 0 || !(4..=32).contains(&info.bits) || !(1..=2).contains(&info.channels) {
            return None;
        }
        let w = words(lines.next())?;
        let (frames, audio) = match w.as_slice() {
            [f, n, a, s, e] if f == "frames" && a == "audio" => {
                (n.parse().ok()?, (s.parse().ok()?, e.parse().ok()?))
            }
            _ => return None,
        };
        let mut points = Vec::new();
        for l in lines {
            let w: Vec<&str> = l.split(' ').collect();
            match w.as_slice() {
                ["p", s, o] => points.push(Point {
                    sample: s.parse().ok()?,
                    offset: o.parse().ok()?,
                }),
                _ => return None,
            }
        }
        let ordered = points.first().is_some_and(|p| p.sample == 0)
            && points
                .windows(2)
                .all(|w| w[1].sample > w[0].sample && w[1].offset > w[0].offset);
        if !ordered {
            return None;
        }
        Some(Index {
            info,
            frames,
            audio,
            points,
            meta: (size, changed),
            made: if hash.is_some() {
                Made::Scan
            } else {
                Made::Table
            },
            hash,
        })
    }
}
