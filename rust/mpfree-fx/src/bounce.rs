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
//!   rings hold into the files as stereo 24-bit WAV, holding back each
//!   stream's last frames (the longest latency less its own), so every
//!   file ends on the same frame. Once [`BounceIn::finish`] is called and
//!   the rings are empty, it writes each file's lengths into its header.
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
    started: bool,
    finished: bool,
    rate: u32,
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
            started: false,
            finished: false,
            rate,
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

impl BounceOut {
    /// Frames from the start to the last one above about -90 dBFS in any
    /// file so far: where the sound ends, for trimming a tail's silence.
    pub fn loud(&self) -> u64 {
        self.loud
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
            for f in files.iter_mut() {
                f.write_all(&wav_header(self.rate, 0))?;
            }
            self.started = true;
        }
        for (s, f) in files.iter_mut().enumerate() {
            let ring = &self.shared.rings[s];
            let w = ring.written.load(Ordering::Acquire);
            let rd = ring.read.load(Ordering::Relaxed);
            let have = w.wrapping_sub(rd);
            let hold = self.hold[s];
            let keep = have.saturating_sub(hold);
            self.bytes.clear();
            for i in 0..keep {
                let at = (rd.wrapping_add(i) % ring.frames) * 2;
                let l = f32::from_bits(ring.data[at].load(Ordering::Relaxed));
                let r = f32::from_bits(ring.data[at + 1].load(Ordering::Relaxed));
                if l.abs() > SILENT || r.abs() > SILENT {
                    self.loud = self.loud.max(self.out[s] + i as u64 + 1);
                }
                self.bytes.extend_from_slice(&s24(l));
                self.bytes.extend_from_slice(&s24(r));
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
            f.write_all(&wav_header(self.rate, self.out[s]))?;
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

    #[test]
    fn a_pass_ended_before_it_began_gives_empty_files() {
        let (mut bin, mut bout) = bounce(2, 64, 48_000);
        let mut files: Vec<Cursor<Vec<u8>>> = (0..2).map(|_| Cursor::new(Vec::new())).collect();
        bin.finish();
        assert_eq!(bout.pump(&mut files).unwrap(), Pumped::Done(0));
        assert_eq!(bout.loud(), 0);
        for f in &files {
            assert_eq!(read24(f.get_ref(), 48_000).0.len(), 0);
        }
    }
}
