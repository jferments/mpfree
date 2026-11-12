//! The disk side of streaming: one 32-bit float WAV file a take in a
//! folder (on the MPC, the stick's data partition), written a chunk at a
//! time as it records and read a chunk at a time as the song plays. Runs on
//! its own thread (see [`crate::host`]); nothing here is on the audio
//! thread.
//!
//! A file is written as it records: a 44-byte header with the lengths left
//! at 0, then each chunk at its place (`44 + chunk * CHUNK * 4`), and the
//! header's lengths at the end ([`crate::Io::Finish`]), when a small
//! `.peaks` file beside it is written too: the lowest and highest sample
//! of every [`DISK_PEAK`] samples, for waveforms without reading the take.
//! A take recorded here gets a new name (`<stamp>-<n>.wav`, never reused),
//! so a save that names a file always finds that audio.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::host::TakeWave;
use crate::save::TakeFile;
use crate::CHUNK;

/// Samples per entry of a take's waveform table on the disk side.
pub const DISK_PEAK: usize = 1024;
const HEADER: u64 = 44;

pub(crate) enum ToDisk {
    /// The file a take writes to or reads from.
    Name { take: u16, name: String },
    Write {
        take: u16,
        chunk: u32,
        channels: usize,
        data: Vec<f32>,
        len: usize,
    },
    Read {
        take: u16,
        chunk: u32,
        buf: Vec<f32>,
    },
    Finish {
        take: u16,
        frames: usize,
        channels: usize,
    },
    /// A saved take's file: check it is there and as long as it should be.
    Open { file: TakeFile },
    /// A whole take's audio (for a fit).
    ReadAll {
        take: u16,
        frames: usize,
        channels: usize,
    },
    Wave {
        take: u16,
        frames: usize,
        channels: usize,
        columns: usize,
    },
}

pub(crate) enum FromDisk {
    Written {
        take: u16,
        chunk: u32,
        buf: Vec<f32>,
        ok: bool,
    },
    Read {
        take: u16,
        chunk: u32,
        buf: Vec<f32>,
        got: Option<usize>,
    },
    Finished {
        ok: bool,
    },
    Opened {
        file: TakeFile,
        ok: Result<(), String>,
    },
    Audio {
        take: u16,
        audio: Option<Vec<f32>>,
    },
    Wave(Option<TakeWave>),
}

/// What the disk thread has seen, for the screen and the tests.
#[derive(Default)]
pub struct DiskStats {
    pub reads: AtomicU64,
    pub writes: AtomicU64,
    pub errors: AtomicU64,
    /// The longest single read or write, in microseconds.
    pub max_us: AtomicU64,
    /// A delay added to every read and write, in microseconds: for tests
    /// of a slow or stalling disk.
    pub delay_us: AtomicU64,
}

struct Entry {
    file: File,
    path: PathBuf,
    data_at: u64,
    /// Lowest and highest sample of each DISK_PEAK samples written.
    peaks: Vec<(f32, f32)>,
}

pub(crate) struct Disk {
    dir: PathBuf,
    rate: u32,
    names: HashMap<u16, String>,
    open: HashMap<u16, Entry>,
    bytes: Vec<u8>,
    stats: Arc<DiskStats>,
}

fn timed<T>(stats: &DiskStats, f: impl FnOnce() -> io::Result<T>) -> io::Result<T> {
    let d = stats.delay_us.load(Ordering::Relaxed);
    if d > 0 {
        std::thread::sleep(Duration::from_micros(d));
    }
    let t = Instant::now();
    let r = f();
    let us = t.elapsed().as_micros() as u64 + d;
    stats.max_us.fetch_max(us, Ordering::Relaxed);
    if r.is_err() {
        stats.errors.fetch_add(1, Ordering::Relaxed);
    }
    r
}

fn header(channels: usize, rate: u32, data: u32) -> [u8; 44] {
    let ch = channels as u16;
    let mut h = [0u8; 44];
    h[..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36u32.wrapping_add(data)).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&3u16.to_le_bytes());
    h[22..24].copy_from_slice(&ch.to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * 4 * u32::from(ch)).to_le_bytes());
    h[32..34].copy_from_slice(&(4 * ch).to_le_bytes());
    h[34..36].copy_from_slice(&32u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data.to_le_bytes());
    h
}

fn note_peaks(peaks: &mut Vec<(f32, f32)>, first: usize, data: &[f32]) {
    for (i, block) in data.chunks(DISK_PEAK).enumerate() {
        let b = first / DISK_PEAK + i;
        if peaks.len() <= b {
            peaks.resize(b + 1, (0.0, 0.0));
        }
        peaks[b] = crate::min_max(block);
    }
}

impl Disk {
    pub fn new(dir: PathBuf, rate: u32, stats: Arc<DiskStats>) -> Disk {
        Disk {
            dir,
            rate,
            names: HashMap::new(),
            open: HashMap::new(),
            bytes: Vec::with_capacity(CHUNK * 4),
            stats,
        }
    }

    fn path(&self, name: &str) -> Option<PathBuf> {
        // Only plain names, inside the folder.
        let ok = !name.is_empty()
            && !name.contains('/')
            && !name.starts_with('.')
            && Path::new(name).file_name().is_some();
        ok.then(|| self.dir.join(name))
    }

    fn entry(&mut self, take: u16, channels: usize) -> io::Result<&mut Entry> {
        if !self.open.contains_key(&take) {
            let name = self
                .names
                .get(&take)
                .ok_or_else(|| io::Error::other("no file name for the take"))?;
            let path = self
                .path(name)
                .ok_or_else(|| io::Error::other("a bad file name"))?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all_at(&header(channels, self.rate, 0), 0)?;
            self.open.insert(
                take,
                Entry {
                    file,
                    path,
                    data_at: HEADER,
                    peaks: Vec::new(),
                },
            );
        }
        Ok(self.open.get_mut(&take).expect("just made"))
    }

    fn write(&mut self, take: u16, chunk: u32, channels: usize, data: &[f32]) -> io::Result<()> {
        self.bytes.clear();
        for v in data {
            self.bytes.extend_from_slice(&v.to_le_bytes());
        }
        let bytes = std::mem::take(&mut self.bytes);
        let stats = self.stats.clone();
        let r = self.entry(take, channels).and_then(|e| {
            let at = e.data_at + chunk as u64 * CHUNK as u64 * 4;
            timed(&stats, || e.file.write_all_at(&bytes, at))?;
            note_peaks(&mut e.peaks, chunk as usize * CHUNK, data);
            Ok(())
        });
        self.bytes = bytes;
        self.stats.writes.fetch_add(1, Ordering::Relaxed);
        r
    }

    fn read(&mut self, take: u16, chunk: u32, out: &mut [f32]) -> io::Result<usize> {
        let e = self
            .open
            .get(&take)
            .ok_or_else(|| io::Error::other("the take's file is not open"))?;
        let at = e.data_at + chunk as u64 * CHUNK as u64 * 4;
        let n = out.len().min(CHUNK);
        self.bytes.resize(n * 4, 0);
        let mut got = 0;
        let bytes = &mut self.bytes;
        timed(&self.stats, || {
            while got < bytes.len() {
                match e.file.read_at(&mut bytes[got..], at + got as u64)? {
                    0 => break,
                    k => got += k,
                }
            }
            Ok(())
        })?;
        for (o, b) in out
            .iter_mut()
            .zip(self.bytes[..got / 4 * 4].chunks_exact(4))
        {
            let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            *o = if v.is_finite() { v } else { 0.0 };
        }
        self.stats.reads.fetch_add(1, Ordering::Relaxed);
        Ok(got / 4)
    }

    fn finish(&mut self, take: u16, frames: usize, channels: usize) -> io::Result<()> {
        let rate = self.rate;
        let e = self.entry(take, channels)?;
        let data = (frames * channels * 4) as u32;
        e.file.write_all_at(&header(channels, rate, data), 0)?;
        e.file.set_len(HEADER + u64::from(data))?;
        e.file.sync_data()?;
        let mut p = Vec::with_capacity(e.peaks.len() * 8);
        for (lo, hi) in &e.peaks {
            p.extend_from_slice(&lo.to_le_bytes());
            p.extend_from_slice(&hi.to_le_bytes());
        }
        let mut side = e.path.clone().into_os_string();
        side.push(".peaks");
        std::fs::write(side, p)
    }

    /// Open a saved take's file and check it against what the save says.
    fn open_saved(&mut self, f: &TakeFile) -> Result<(), String> {
        let path = self
            .path(&f.file)
            .ok_or_else(|| format!("take {}: bad file name {:?}", f.id, f.file))?;
        let mut file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut head = vec![0u8; 4096];
        let n = file.read(&mut head).map_err(|e| e.to_string())?;
        let (data_at, len, ch, rate) = crate::save::wav_layout(&head[..n])?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        let need = (f.frames * f.channels * 4) as u64;
        if ch != f.channels || rate != self.rate || len < need || size < data_at + need {
            return Err(format!(
                "{}: {ch} channels at {rate} Hz, {} bytes of audio; the save says {} channels at {} Hz, {need}",
                path.display(),
                len.min(size.saturating_sub(data_at)),
                f.channels,
                self.rate
            ));
        }
        // The waveform table: from the .peaks file if it fits, else read.
        let mut side = path.clone().into_os_string();
        side.push(".peaks");
        let blocks = (f.frames * f.channels).div_ceil(DISK_PEAK);
        let peaks = match std::fs::read(&side) {
            Ok(b) if b.len() == blocks * 8 => b
                .chunks_exact(8)
                .map(|x| {
                    (
                        f32::from_le_bytes([x[0], x[1], x[2], x[3]]),
                        f32::from_le_bytes([x[4], x[5], x[6], x[7]]),
                    )
                })
                .collect(),
            _ => {
                let mut peaks = Vec::with_capacity(blocks);
                let mut buf = vec![0u8; DISK_PEAK * 4 * 64];
                let mut at = 0u64;
                let mut samples = Vec::with_capacity(DISK_PEAK * 64);
                while at < need {
                    let n = (need - at).min(buf.len() as u64) as usize;
                    file.read_exact_at(&mut buf[..n], data_at + at)
                        .map_err(|e| e.to_string())?;
                    samples.clear();
                    samples.extend(
                        buf[..n]
                            .chunks_exact(4)
                            .map(|x| f32::from_le_bytes([x[0], x[1], x[2], x[3]])),
                    );
                    note_peaks(&mut peaks, (at / 4) as usize, &samples);
                    at += n as u64;
                }
                peaks
            }
        };
        self.names.insert(f.id, f.file.clone());
        self.open.insert(
            f.id,
            Entry {
                file: OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .or_else(|_| File::open(&path))
                    .map_err(|e| e.to_string())?,
                path,
                data_at,
                peaks,
            },
        );
        Ok(())
    }

    fn read_all(&mut self, take: u16, frames: usize, channels: usize) -> Option<Vec<f32>> {
        let mut audio = vec![0.0; frames * channels];
        let mut at = 0;
        let mut c = 0;
        while at < audio.len() {
            let end = (at + CHUNK).min(audio.len());
            let got = self.read(take, c, &mut audio[at..end]).ok()?;
            if got < end - at {
                return None;
            }
            at = end;
            c += 1;
        }
        Some(audio)
    }

    fn wave(&self, take: u16, frames: usize, channels: usize, columns: usize) -> Option<TakeWave> {
        let e = self.open.get(&take)?;
        let ch = channels.max(1);
        let step = (DISK_PEAK / ch).max(1);
        let fpc = frames
            .div_ceil(columns.clamp(1, crate::host::MAX_COLUMNS))
            .div_ceil(step)
            .max(1)
            * step;
        let per = fpc * ch / DISK_PEAK;
        let blocks = (frames * ch).div_ceil(DISK_PEAK);
        let mut out = Vec::with_capacity(blocks.div_ceil(per));
        let mut b = 0;
        while b < blocks {
            let end = (b + per).min(blocks);
            if end > e.peaks.len() {
                // Not all written yet: the caller fills the rest.
                break;
            }
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for p in &e.peaks[b..end] {
                lo = lo.min(p.0);
                hi = hi.max(p.1);
            }
            out.push((lo, hi));
            b = end;
        }
        Some(TakeWave {
            id: take,
            frames,
            channels: ch,
            frames_per_column: fpc,
            columns: out,
        })
    }

    /// The thread's loop: answer each request in order until the host goes.
    pub fn serve(mut self, jobs: Receiver<ToDisk>, done: Sender<FromDisk>) {
        for m in jobs {
            let reply = match m {
                ToDisk::Name { take, name } => {
                    self.open.remove(&take);
                    self.names.insert(take, name);
                    continue;
                }
                ToDisk::Write {
                    take,
                    chunk,
                    channels,
                    data,
                    len,
                } => {
                    let ok = self.write(take, chunk, channels, &data[..len]).is_ok();
                    FromDisk::Written {
                        take,
                        chunk,
                        buf: data,
                        ok,
                    }
                }
                ToDisk::Read {
                    take,
                    chunk,
                    mut buf,
                } => {
                    let got = self.read(take, chunk, &mut buf).ok();
                    FromDisk::Read {
                        take,
                        chunk,
                        buf,
                        got,
                    }
                }
                ToDisk::Finish {
                    take,
                    frames,
                    channels,
                } => FromDisk::Finished {
                    ok: self.finish(take, frames, channels).is_ok(),
                },
                ToDisk::Open { file } => {
                    let ok = self.open_saved(&file);
                    FromDisk::Opened { file, ok }
                }
                ToDisk::ReadAll {
                    take,
                    frames,
                    channels,
                } => FromDisk::Audio {
                    take,
                    audio: self.read_all(take, frames, channels),
                },
                ToDisk::Wave {
                    take,
                    frames,
                    channels,
                    columns,
                } => FromDisk::Wave(self.wave(take, frames, channels, columns)),
            };
            if done.send(reply).is_err() {
                break;
            }
        }
    }

    /// The file name of take `take`, if it has one.
    #[allow(dead_code)]
    pub fn name(&self, take: u16) -> Option<&str> {
        self.names.get(&take).map(String::as_str)
    }
}
