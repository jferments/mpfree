//! Bouncing: mixer capture points streamed to WAV files while the song
//! plays, for the export's mixdown and stems.
//!
//! - [`bounce`] makes the two halves for a number of streams, each a
//!   capture point ([`crate::Mixer::set_capture`]), off the sound thread.
//! - [`BounceIn`] is the sound thread's. Once the capture points are set,
//!   [`BounceIn::begin`] takes their latencies
//!   ([`crate::Mixer::capture_latency`]). After the mixer, each block,
//!   [`BounceIn::push`] copies each capture point's block into its
//!   stream's ring: memory made beforehand, shared as atomics, so no lock,
//!   no allocation and no unsafe code. It drops each stream's first
//!   `latency` frames, so every stream starts on the song's first frame.
//!   When a ring has no room (the files can't keep up), the block is
//!   dropped and counted ([`BounceIn::overruns`]): the bounce has failed.
//!   A pass that isn't held to real time (the export's fast mode) asks
//!   [`BounceIn::room`] first and waits for the writer instead.
//! - [`BounceOut`] is a writer thread's. [`BounceOut::pump`] moves what the
//!   rings hold into the files as stereo WAV, holding back each stream's
//!   last frames (the longest latency less its own), so every file ends on
//!   the same frame. Once [`BounceIn::finish`] is called and the rings are
//!   empty, it writes each file's lengths into its header.
//! - [`WavFormat`] is how the files hold the sound
//!   ([`BounceOut::with_format`]): 24-bit (the default), 16-bit with
//!   triangular dither, or 32-bit float, at the mixer's rate or converted
//!   to another ([`crate::resample`], on the writer thread).
//!
//! The pass itself (where it starts, when the tail is done) is the host's;
//! [`BounceIn::quiet`] says how long a stream has been silent, for a tail
//! that runs until the reverb dies away.

use std::io::{self, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

use crate::print::SILENT;

/// The bytes before a 24-bit stereo WAV file's samples.
pub const WAV_HEADER: usize = 44;

/// How each sample is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bits {
    /// 16-bit, with triangular (TPDF) dither of one step each way, so
    /// quiet sound fades into a little noise instead of breaking up.
    I16,
    /// 24-bit, rounded: what the mixer makes, kept to far below hearing.
    I24,
    /// 32-bit float: exactly what the mixer made, levels over full scale
    /// kept.
    F32,
}

impl Bits {
    pub const ALL: [Bits; 3] = [Bits::I24, Bits::I16, Bits::F32];

    /// Bytes a stereo frame takes.
    pub fn frame_bytes(self) -> u64 {
        match self {
            Bits::I16 => 4,
            Bits::I24 => 6,
            Bits::F32 => 8,
        }
    }

    /// "24-bit", "16-bit", "32-bit float".
    pub fn words(self) -> &'static str {
        match self {
            Bits::I16 => "16-bit",
            Bits::I24 => "24-bit",
            Bits::F32 => "32-bit float",
        }
    }
}

/// A stereo WAV file's format: the samples and the rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WavFormat {
    pub bits: Bits,
    pub rate: u32,
}

impl WavFormat {
    /// 24-bit at `rate`.
    pub fn i24(rate: u32) -> WavFormat {
        WavFormat {
            bits: Bits::I24,
            rate,
        }
    }

    /// The bytes before the samples: 44, or 58 for float (its `fmt `
    /// chunk is longer and a `fact` chunk follows, as the format asks).
    pub fn header_len(self) -> usize {
        match self.bits {
            Bits::F32 => 58,
            _ => WAV_HEADER,
        }
    }

    /// The header for `frames` frames.
    pub fn header(self, frames: u64) -> Vec<u8> {
        let fb = self.bits.frame_bytes();
        let hl = self.header_len() as u64;
        let data = (frames * fb).min(u64::from(u32::MAX) - hl) as u32;
        let (tag, bits, fmt_len): (u16, u16, u32) = match self.bits {
            Bits::I16 => (1, 16, 16),
            Bits::I24 => (1, 24, 16),
            Bits::F32 => (3, 32, 18),
        };
        let mut h = Vec::with_capacity(hl as usize);
        h.extend_from_slice(b"RIFF");
        h.extend_from_slice(&(hl as u32 - 8 + data).to_le_bytes());
        h.extend_from_slice(b"WAVEfmt ");
        h.extend_from_slice(&fmt_len.to_le_bytes());
        h.extend_from_slice(&tag.to_le_bytes());
        h.extend_from_slice(&2u16.to_le_bytes());
        h.extend_from_slice(&self.rate.to_le_bytes());
        h.extend_from_slice(&(self.rate * fb as u32).to_le_bytes());
        h.extend_from_slice(&(fb as u16).to_le_bytes());
        h.extend_from_slice(&bits.to_le_bytes());
        if self.bits == Bits::F32 {
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(b"fact");
            h.extend_from_slice(&4u32.to_le_bytes());
            h.extend_from_slice(&(frames.min(u64::from(u32::MAX)) as u32).to_le_bytes());
        }
        h.extend_from_slice(b"data");
        h.extend_from_slice(&data.to_le_bytes());
        h
    }

    /// The file's frames for `frames` frames at the mixer's `rate`.
    pub fn frames_for(self, frames: u64, rate: u32) -> u64 {
        crate::resample::frames_out(frames, rate, self.rate)
    }

    /// "24-bit, 44.1 kHz".
    pub fn words(self) -> String {
        let khz = f64::from(self.rate) / 1000.0;
        format!("{}, {} kHz", self.bits.words(), khz)
    }
}

/// One file's encoder: its dither and, at another rate, its converter.
struct Encoder {
    /// The dither's random state (a different one for each file).
    seed: u32,
    resample: Option<crate::resample::Resampler>,
    /// Frames written.
    frames: u64,
}

/// One stream's ring of interleaved stereo frames, as `f32` bits.
struct Ring {
    data: Box<[AtomicU32]>,
    frames: usize,
    /// Frames written and read since the start (wrapping).
    written: AtomicUsize,
    read: AtomicUsize,
}

struct Shared {
    rings: Vec<Ring>,
    /// Each stream's capture latency, set by [`BounceIn::begin`].
    latency: Vec<AtomicU32>,
    begun: AtomicBool,
    done: AtomicBool,
    overruns: AtomicU32,
}

/// The sound thread's half (see the module docs).
pub struct BounceIn {
    shared: Arc<Shared>,
    /// Frames still to drop at each stream's start.
    skip: Vec<u32>,
    /// Frames since each stream was last above [`SILENT`].
    quiet: Vec<usize>,
}

/// The writer thread's half (see the module docs).
pub struct BounceOut {
    shared: Arc<Shared>,
    /// Frames each stream holds back until the end (from the latencies,
    /// at the first pump after [`BounceIn::begin`]).
    hold: Vec<usize>,
    /// Frames each file has.
    out: Vec<u64>,
    /// Frames up to the last one above [`SILENT`] in any file.
    loud: u64,
    /// Whether each file has had a frame above [`SILENT`].
    heard: Vec<bool>,
    started: bool,
    finished: bool,
    rate: u32,
    format: WavFormat,
    enc: Vec<Encoder>,
    /// One block's frames, before and after conversion.
    left: Vec<f32>,
    right: Vec<f32>,
    conv: [Vec<f32>; 2],
    bytes: Vec<u8>,
}

/// What a [`BounceOut::pump`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pumped {
    /// Wrote what there was; more to come. The shortest file's frames.
    Working(u64),
    /// Every file is whole, its header written: their frames.
    Done(u64),
    /// A ring overflowed: the files have gaps. Stop and drop them.
    Overrun(u32),
}

/// The two halves for `streams` streams, each ring holding `ring_frames`
/// (two seconds is plenty for a stick that stalls now and then). Make it
/// off the sound thread.
pub fn bounce(streams: usize, ring_frames: usize, rate: u32) -> (BounceIn, BounceOut) {
    let frames = ring_frames.max(1);
    let shared = Arc::new(Shared {
        rings: (0..streams)
            .map(|_| Ring {
                data: (0..frames * 2).map(|_| AtomicU32::new(0)).collect(),
                frames,
                written: AtomicUsize::new(0),
                read: AtomicUsize::new(0),
            })
            .collect(),
        latency: (0..streams).map(|_| AtomicU32::new(0)).collect(),
        begun: AtomicBool::new(false),
        done: AtomicBool::new(false),
        overruns: AtomicU32::new(0),
    });
    (
        BounceIn {
            shared: shared.clone(),
            skip: vec![0; streams],
            quiet: vec![0; streams],
        },
        BounceOut {
            shared,
            hold: vec![0; streams],
            out: vec![0; streams],
            loud: 0,
            heard: vec![false; streams],
            started: false,
            finished: false,
            rate,
            format: WavFormat::i24(rate),
            enc: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
            conv: [Vec::new(), Vec::new()],
            bytes: Vec::new(),
        },
    )
}

impl BounceIn {
    /// The streams' capture latencies (frames, in stream order), once
    /// their capture points are set; before it, pushes are dropped. A
    /// stream left out has none. Never allocates.
    pub fn begin(&mut self, latencies: &[u32]) {
        for (s, skip) in self.skip.iter_mut().enumerate() {
            let l = latencies.get(s).copied().unwrap_or(0);
            *skip = l;
            self.shared.latency[s].store(l, Ordering::Relaxed);
        }
        self.shared.begun.store(true, Ordering::Release);
    }

    pub fn begun(&self) -> bool {
        self.shared.begun.load(Ordering::Relaxed)
    }

    /// Frames every ring has room for now: a block this long or shorter
    /// pushes without an overrun.
    pub fn room(&self) -> usize {
        self.shared
            .rings
            .iter()
            .map(|ring| {
                let w = ring.written.load(Ordering::Relaxed);
                let rd = ring.read.load(Ordering::Acquire);
                ring.frames - w.wrapping_sub(rd)
            })
            .min()
            .unwrap_or(usize::MAX)
    }

    /// Stream `s`'s block (its capture point's, this block). Never waits
    /// or allocates.
    pub fn push(&mut self, s: usize, l: &[f32], r: &[f32]) {
        let Some(ring) = self.shared.rings.get(s) else {
            return;
        };
        if !self.begun() {
            return;
        }
        let n = l.len().min(r.len());
        let q = &mut self.quiet[s];
        for i in 0..n {
            if l[i].abs() > SILENT || r[i].abs() > SILENT {
                *q = 0;
            } else {
                *q += 1;
            }
        }
        let skip = (self.skip[s] as usize).min(n);
        self.skip[s] -= skip as u32;
        let (l, r) = (&l[skip..n], &r[skip..n]);
        let n = l.len();
        if n == 0 {
            return;
        }
        let w = ring.written.load(Ordering::Relaxed);
        let rd = ring.read.load(Ordering::Acquire);
        if ring.frames - w.wrapping_sub(rd) < n {
            self.shared.overruns.fetch_add(1, Ordering::Relaxed);
            return;
        }
        for i in 0..n {
            let at = (w.wrapping_add(i) % ring.frames) * 2;
            ring.data[at].store(l[i].to_bits(), Ordering::Relaxed);
            ring.data[at + 1].store(r[i].to_bits(), Ordering::Relaxed);
        }
        ring.written.store(w.wrapping_add(n), Ordering::Release);
    }

    /// The pass is over: nothing more will be pushed.
    pub fn finish(&mut self) {
        self.shared.done.store(true, Ordering::Release);
    }

    /// Frames since stream `s` was last above about -90 dBFS.
    pub fn quiet(&self, s: usize) -> usize {
        self.quiet.get(s).copied().unwrap_or(0)
    }

    /// Blocks dropped because a ring was full.
    pub fn overruns(&self) -> u32 {
        self.shared.overruns.load(Ordering::Relaxed)
    }
}

/// A stereo 24-bit WAV file's header for `frames` frames at `rate`.
pub fn wav_header(rate: u32, frames: u64) -> [u8; WAV_HEADER] {
    let data = (frames * 6).min(u64::from(u32::MAX) - 36) as u32;
    let mut h = [0u8; WAV_HEADER];
    let mut put = |at: usize, b: &[u8]| h[at..at + b.len()].copy_from_slice(b);
    put(0, b"RIFF");
    put(4, &(36 + data).to_le_bytes());
    put(8, b"WAVEfmt ");
    put(16, &16u32.to_le_bytes());
    put(20, &1u16.to_le_bytes());
    put(22, &2u16.to_le_bytes());
    put(24, &rate.to_le_bytes());
    put(28, &(rate * 6).to_le_bytes());
    put(32, &6u16.to_le_bytes());
    put(34, &24u16.to_le_bytes());
    put(36, b"data");
    put(40, &data.to_le_bytes());
    h
}

/// A sample as 24 bits, rounded, clipped at full scale.
fn s24(x: f32) -> [u8; 3] {
    let v = (f64::from(x) * 8_388_607.0)
        .round()
        .clamp(-8_388_608.0, 8_388_607.0) as i32;
    let b = v.to_le_bytes();
    [b[0], b[1], b[2]]
}

/// A random number from -0.5 to 0.5 (an xorshift step).
fn uniform(seed: &mut u32) -> f64 {
    let mut x = *seed;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *seed = x;
    f64::from(x) / 4_294_967_296.0 - 0.5
}

/// A sample as 16 bits: triangular dither of up to one step each way
/// (two uniform numbers added), then rounded and clipped at full scale.
fn s16(x: f32, seed: &mut u32) -> [u8; 2] {
    let d = uniform(seed) + uniform(seed);
    let v = (f64::from(x) * 32_767.0 + d)
        .round()
        .clamp(-32_768.0, 32_767.0) as i16;
    v.to_le_bytes()
}

impl Encoder {
    /// Appends `l` and `r` to `bytes` as `bits`.
    fn encode(&mut self, bits: Bits, l: &[f32], r: &[f32], bytes: &mut Vec<u8>) {
        for (&a, &b) in l.iter().zip(r) {
            match bits {
                Bits::I16 => {
                    bytes.extend_from_slice(&s16(a, &mut self.seed));
                    bytes.extend_from_slice(&s16(b, &mut self.seed));
                }
                Bits::I24 => {
                    bytes.extend_from_slice(&s24(a));
                    bytes.extend_from_slice(&s24(b));
                }
                Bits::F32 => {
                    bytes.extend_from_slice(&a.to_le_bytes());
                    bytes.extend_from_slice(&b.to_le_bytes());
                }
            }
        }
        self.frames += l.len().min(r.len()) as u64;
    }
}

impl BounceOut {
    /// The files in `format` (24-bit at the mixer's rate unless told).
    /// Set it before the first pump.
    pub fn with_format(mut self, format: WavFormat) -> BounceOut {
        debug_assert!(!self.started, "the format is set before the files start");
        self.format = format;
        self
    }

    pub fn format(&self) -> WavFormat {
        self.format
    }

    /// Each file's frames so far, at the file's rate.
    pub fn file_frames(&self) -> Vec<u64> {
        self.enc.iter().map(|e| e.frames).collect()
    }

    /// Frames from the start to the last one above about -90 dBFS in any
    /// file so far: where the sound ends, for trimming a tail's silence.
    pub fn loud(&self) -> u64 {
        self.loud
    }

    /// Whether each file (in stream order) has had a frame above about
    /// -90 dBFS so far: a stem that never sounded can be left out.
    pub fn heard(&self) -> &[bool] {
        &self.heard
    }

    /// Moves what the rings hold into `files` (one per stream, in order,
    /// each empty at the start), and finishes them once the pass is over.
    pub fn pump<W: Write + Seek>(&mut self, files: &mut [W]) -> io::Result<Pumped> {
        let over = self.shared.overruns.load(Ordering::Relaxed);
        if over > 0 {
            return Ok(Pumped::Overrun(over));
        }
        if self.finished {
            return Ok(Pumped::Done(self.out.iter().copied().min().unwrap_or(0)));
        }
        if files.len() < self.shared.rings.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a file for every stream",
            ));
        }
        // Done first, then what was written: the last frames are seen.
        let done = self.shared.done.load(Ordering::Acquire);
        if !self.started {
            if !done && !self.shared.begun.load(Ordering::Acquire) {
                return Ok(Pumped::Working(0));
            }
            let lat: Vec<u32> = self
                .shared
                .latency
                .iter()
                .map(|l| l.load(Ordering::Relaxed))
                .collect();
            let most = lat.iter().copied().max().unwrap_or(0);
            for (h, l) in self.hold.iter_mut().zip(&lat) {
                *h = (most - l) as usize;
            }
            let to = self.format.rate;
            self.enc = (0..files.len())
                .map(|s| Encoder {
                    seed: (0x9E37_79B9 ^ (s as u32).wrapping_mul(0x85EB_CA6B)) | 1,
                    resample: (to != self.rate && to > 0)
                        .then(|| crate::resample::Resampler::new(self.rate, to, 2)),
                    frames: 0,
                })
                .collect();
            for f in files.iter_mut() {
                f.write_all(&self.format.header(0))?;
            }
            self.started = true;
        }
        let bits = self.format.bits;
        for (s, f) in files.iter_mut().enumerate() {
            let ring = &self.shared.rings[s];
            let w = ring.written.load(Ordering::Acquire);
            let rd = ring.read.load(Ordering::Relaxed);
            let have = w.wrapping_sub(rd);
            let hold = self.hold[s];
            let keep = have.saturating_sub(hold);
            self.bytes.clear();
            self.left.clear();
            self.right.clear();
            for i in 0..keep {
                let at = (rd.wrapping_add(i) % ring.frames) * 2;
                let l = f32::from_bits(ring.data[at].load(Ordering::Relaxed));
                let r = f32::from_bits(ring.data[at + 1].load(Ordering::Relaxed));
                if l.abs() > SILENT || r.abs() > SILENT {
                    self.loud = self.loud.max(self.out[s] + i as u64 + 1);
                    self.heard[s] = true;
                }
                self.left.push(l);
                self.right.push(r);
            }
            let enc = &mut self.enc[s];
            match enc.resample.as_mut() {
                Some(rs) => {
                    for c in &mut self.conv {
                        c.clear();
                    }
                    rs.push(&[&self.left, &self.right], &mut self.conv);
                    if done {
                        rs.finish(&mut self.conv);
                    }
                    enc.encode(bits, &self.conv[0], &self.conv[1], &mut self.bytes);
                }
                None => enc.encode(bits, &self.left, &self.right, &mut self.bytes),
            }
            f.write_all(&self.bytes)?;
            self.out[s] += keep as u64;
            // At the end, the held-back frames go unwritten.
            let gone = if done { have } else { keep };
            ring.read.store(rd.wrapping_add(gone), Ordering::Release);
        }
        if !done {
            return Ok(Pumped::Working(self.out.iter().copied().min().unwrap_or(0)));
        }
        for (s, f) in files.iter_mut().enumerate() {
            f.seek(SeekFrom::Start(0))?;
            f.write_all(&self.format.header(self.enc[s].frames))?;
            f.seek(SeekFrom::End(0))?;
            f.flush()?;
        }
        self.finished = true;
        Ok(Pumped::Done(self.out.iter().copied().min().unwrap_or(0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A 24-bit stereo WAV's frames, checking its header.
    pub(crate) fn read24(b: &[u8], rate: u32) -> (Vec<f32>, Vec<f32>) {
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(&b[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([b[22], b[23]]), 2);
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), rate);
        assert_eq!(u16::from_le_bytes([b[34], b[35]]), 24);
        let data = u32::from_le_bytes(b[40..44].try_into().unwrap()) as usize;
        assert_eq!(data + 44, b.len(), "the header's length");
        assert_eq!(
            u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize,
            b.len() - 8
        );
        let s = |i: usize| {
            let v = i32::from_le_bytes([0, b[i], b[i + 1], b[i + 2]]) >> 8;
            v as f32 / 8_388_607.0
        };
        let n = data / 6;
        (
            (0..n).map(|k| s(44 + 6 * k)).collect(),
            (0..n).map(|k| s(44 + 6 * k + 3)).collect(),
        )
    }

    fn song(n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| 0.5 * (i as f32 * 0.013).sin() + 0.25 * (i as f32 * 0.071).cos())
            .collect()
    }

    #[test]
    fn streams_with_different_latencies_line_up_and_end_together() {
        let lat = [0u32, 37, 300];
        let x = song(5000);
        let (mut bin, mut bout) = bounce(3, 1024, 48_000);
        let mut files: Vec<Cursor<Vec<u8>>> = (0..3).map(|_| Cursor::new(Vec::new())).collect();
        // Nothing is kept or written until the latencies are known.
        bin.push(0, &[0.5; 16], &[0.5; 16]);
        assert_eq!(bout.pump(&mut files).unwrap(), Pumped::Working(0));
        assert!(files[0].get_ref().is_empty());
        bin.begin(&lat);
        // Stream s hears the song `lat[s]` frames late, at gain 1, -1, 0.5.
        let gains = [1.0f32, -1.0, 0.5];
        let total = 4000;
        let mut at = 0;
        let mut sizes = [64usize, 1, 333, 128].iter().cycle();
        while at < total {
            let n = (*sizes.next().unwrap()).min(total - at);
            for s in 0..3 {
                let block: Vec<f32> = (at..at + n)
                    .map(|i| {
                        i.checked_sub(lat[s] as usize)
                            .map_or(0.0, |j| x[j] * gains[s])
                    })
                    .collect();
                bin.push(s, &block, &block);
            }
            at += n;
            if at % 3 == 0 {
                assert!(matches!(bout.pump(&mut files).unwrap(), Pumped::Working(_)));
            }
        }
        bin.finish();
        assert_eq!(bout.pump(&mut files).unwrap(), Pumped::Done(3700));
        assert_eq!(bout.loud(), 3700);
        assert_eq!(bout.heard(), [true, true, true]);
        assert_eq!(bout.pump(&mut files).unwrap(), Pumped::Done(3700));
        for s in 0..3 {
            let (l, r) = read24(files[s].get_ref(), 48_000);
            assert_eq!(l.len(), total - 300, "stream {s}");
            for i in 0..l.len() {
                let want = x[i] * gains[s];
                assert!((l[i] - want).abs() < 1.5e-7, "stream {s} frame {i}");
                assert_eq!(l[i], r[i]);
            }
        }
        assert_eq!(bin.overruns(), 0);
    }

    #[test]
    fn a_ring_that_fills_fails_the_bounce() {
        let (mut bin, mut bout) = bounce(1, 100, 48_000);
        let mut files = vec![Cursor::new(Vec::new())];
        bin.begin(&[0]);
        assert_eq!(bin.room(), 100);
        bin.push(0, &[0.1; 80], &[0.1; 80]);
        assert_eq!(bin.room(), 20);
        bin.push(0, &[0.1; 30], &[0.1; 30]);
        assert_eq!(bin.overruns(), 1);
        assert_eq!(bout.pump(&mut files).unwrap(), Pumped::Overrun(1));
    }

    #[test]
    fn full_scale_clips_and_quiet_is_counted() {
        assert_eq!(s24(1.0), [0xFF, 0xFF, 0x7F]);
        assert_eq!(s24(2.0), [0xFF, 0xFF, 0x7F]);
        assert_eq!(s24(-1.0), [0x01, 0x00, 0x80]);
        assert_eq!(s24(-3.0), [0x00, 0x00, 0x80]);
        assert_eq!(s24(0.0), [0, 0, 0]);
        let (mut bin, _) = bounce(1, 4096, 48_000);
        bin.begin(&[0]);
        bin.push(0, &[0.5; 10], &[0.0; 10]);
        bin.push(0, &[1e-6; 100], &[0.0; 100]);
        assert_eq!(bin.quiet(0), 100);
        bin.push(0, &[0.0; 9], &[0.01; 9]);
        assert_eq!(bin.quiet(0), 0);
        assert_eq!(wav_header(48_000, 10)[40..44], 60u32.to_le_bytes());
    }

    /// Any WAV file this module writes: its format and frames, checking
    /// the header.
    pub(crate) fn read_wav(b: &[u8]) -> (WavFormat, Vec<f32>, Vec<f32>) {
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32_at(4) as usize, b.len() - 8, "the RIFF length");
        assert_eq!(&b[8..16], b"WAVEfmt ");
        let fmt_len = u32_at(16) as usize;
        let (tag, rate, bits) = (u16_at(20), u32_at(24), u16_at(34));
        assert_eq!(u16_at(22), 2, "stereo");
        let fb = u32::from(bits / 8) * 2;
        assert_eq!(u32_at(28), rate * fb, "bytes a second");
        assert_eq!(u32::from(u16_at(32)), fb, "bytes a frame");
        let mut at = 20 + fmt_len;
        let format = match (tag, bits) {
            (1, 16) => Bits::I16,
            (1, 24) => Bits::I24,
            (3, 32) => {
                assert_eq!(fmt_len, 18);
                assert_eq!(&b[at..at + 4], b"fact");
                at += 12;
                Bits::F32
            }
            other => panic!("not a format this writes: {other:?}"),
        };
        assert_eq!(&b[at..at + 4], b"data");
        let data = u32_at(at + 4) as usize;
        at += 8;
        assert_eq!(at + data, b.len(), "the data's length");
        let n = data / fb as usize;
        if format == Bits::F32 {
            assert_eq!(u32_at(at - 12) as usize, n, "the fact chunk's frames");
        }
        let sample = |i: usize| match format {
            Bits::I16 => f32::from(i16::from_le_bytes([b[i], b[i + 1]])) / 32_767.0,
            Bits::I24 => {
                (i32::from_le_bytes([0, b[i], b[i + 1], b[i + 2]]) >> 8) as f32 / 8_388_607.0
            }
            Bits::F32 => f32::from_le_bytes(b[i..i + 4].try_into().unwrap()),
        };
        let half = fb as usize / 2;
        let fmt = WavFormat { bits: format, rate };
        (
            fmt,
            (0..n).map(|k| sample(at + fb as usize * k)).collect(),
            (0..n)
                .map(|k| sample(at + fb as usize * k + half))
                .collect(),
        )
    }

    /// `l` and `r` bounced as one stream in `format` from `rate`, in blocks
    /// of 100.
    fn bounce_as(format: WavFormat, rate: u32, l: &[f32], r: &[f32]) -> Vec<u8> {
        let (mut bin, bout) = bounce(1, 1 << 16, rate);
        let mut bout = bout.with_format(format);
        let mut files = vec![Cursor::new(Vec::new())];
        bin.begin(&[0]);
        for (a, b) in l.chunks(100).zip(r.chunks(100)) {
            bin.push(0, a, b);
            bout.pump(&mut files).unwrap();
        }
        bin.finish();
        let n = l.len() as u64;
        assert_eq!(bout.pump(&mut files).unwrap(), Pumped::Done(n));
        assert_eq!(bout.file_frames(), [format.frames_for(n, rate)]);
        files.pop().unwrap().into_inner()
    }

    #[test]
    fn float_keeps_every_sample_exactly_even_over_full_scale() {
        let l: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.37).sin() * 1.5).collect();
        let r: Vec<f32> = l.iter().map(|v| -v * 0.5).collect();
        let b = bounce_as(
            WavFormat {
                bits: Bits::F32,
                rate: 44_100,
            },
            44_100,
            &l,
            &r,
        );
        assert_eq!(b.len(), 58 + 8 * 1000);
        let (f, gl, gr) = read_wav(&b);
        assert_eq!(
            f,
            WavFormat {
                bits: Bits::F32,
                rate: 44_100
            }
        );
        assert_eq!((gl, gr), (l, r));
    }

    #[test]
    fn sixteen_bit_is_dithered_so_quiet_sound_survives() {
        // A third of a 16-bit step: rounding alone would make it silence.
        let step = 1.0 / 32_767.0;
        let n = 48_000;
        let quiet = vec![0.3 * step; n];
        let b = bounce_as(
            WavFormat {
                bits: Bits::I16,
                rate: 48_000,
            },
            48_000,
            &quiet,
            &quiet,
        );
        let (f, l, _) = read_wav(&b);
        assert_eq!(f.bits, Bits::I16);
        assert_eq!(l.len(), n);
        let mean = l.iter().map(|v| f64::from(*v)).sum::<f64>() / n as f64;
        assert!(
            (mean / f64::from(step) - 0.3).abs() < 0.02,
            "on average it is still there: {}",
            mean / f64::from(step)
        );
        // Dither is at most one step each way: silence stays within one
        // step, and the two sides get different noise.
        let silent = vec![0.0; n];
        let (_, l, r) = read_wav(&bounce_as(
            WavFormat {
                bits: Bits::I16,
                rate: 48_000,
            },
            48_000,
            &silent,
            &silent,
        ));
        assert!(l.iter().chain(&r).all(|v| v.abs() <= step * 1.0001));
        assert_ne!(l, r);
        // Full scale clips; loud sound is within a step of where it was.
        let loud: Vec<f32> = (0..n).map(|i| 1.2 * (i as f32 * 0.01).sin()).collect();
        let (_, l, _) = read_wav(&bounce_as(
            WavFormat {
                bits: Bits::I16,
                rate: 48_000,
            },
            48_000,
            &loud,
            &loud,
        ));
        for (got, want) in l.iter().zip(&loud) {
            let want = want.clamp(-32_768.0 / 32_767.0, 1.0);
            assert!((got - want).abs() <= 1.5 * step, "{got} for {want}");
        }
    }

    #[test]
    fn another_rate_converts_on_the_writer_and_keeps_the_frames_lined_up() {
        let x = song(44_100);
        let format = WavFormat {
            bits: Bits::F32,
            rate: 48_000,
        };
        let (f, l, r) = read_wav(&bounce_as(format, 44_100, &x, &x));
        assert_eq!(f, format);
        assert_eq!(l.len(), 48_000);
        assert_eq!(l, r);
        let mut rs = crate::resample::Resampler::new(44_100, 48_000, 1);
        let mut want = vec![Vec::new()];
        rs.push(&[&x], &mut want);
        rs.finish(&mut want);
        assert_eq!(l, want[0], "the same as converting the whole song at once");
        // 24-bit at 48 kHz too.
        let (f, l, _) = read_wav(&bounce_as(
            WavFormat {
                bits: Bits::I24,
                rate: 48_000,
            },
            44_100,
            &x,
            &x,
        ));
        assert_eq!((f.bits, f.rate, l.len()), (Bits::I24, 48_000, 48_000));
        assert_eq!(WavFormat::i24(44_100).words(), "24-bit, 44.1 kHz");
        assert_eq!(format.words(), "32-bit float, 48 kHz");
    }

    #[test]
    fn a_pass_ended_before_it_began_gives_empty_files() {
        let (mut bin, mut bout) = bounce(2, 64, 48_000);
        let mut files: Vec<Cursor<Vec<u8>>> = (0..2).map(|_| Cursor::new(Vec::new())).collect();
        bin.finish();
        assert_eq!(bout.pump(&mut files).unwrap(), Pumped::Done(0));
        assert_eq!(bout.loud(), 0);
        assert_eq!(bout.heard(), [false, false]);
        for f in &files {
            assert_eq!(read24(f.get_ref(), 48_000).0.len(), 0);
        }
    }
}
