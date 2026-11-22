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
//!
//! Requests are answered in order, except the long ones that read a whole
//! file (a saved take's waveform table when it has no `.peaks` file, a
//! whole take for a fit): those go a step ([`STEP_BYTES`]) at a time, with
//! up to [`BETWEEN`] other requests answered between steps, so the reads
//! that play the song and the writes that record never wait behind them.

use std::collections::{HashMap, VecDeque};
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
/// The most audio bytes a WAV header can give (its RIFF length, 36 more,
/// must fit in 32 bits): about 3 hours of stereo at 48 kHz. A take longer
/// than this is not finished with a wrong header: its finish fails.
pub const WAV_MAX_DATA: u64 = u32::MAX as u64 - 36;
/// Bytes a long job reads in one step (whole waveform-table blocks).
pub const STEP_BYTES: usize = 128 * DISK_PEAK * 4;
/// Other requests answered, at most, between two steps of a long job.
pub const BETWEEN: usize = 8;

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
        take: u16,
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
    /// For the fault tests: the next this many writes fail, finishes
    /// fail, and reads come back short (half what was asked).
    pub fail_writes: AtomicU64,
    pub fail_finishes: AtomicU64,
    pub short_reads: AtomicU64,
    /// Steps of long jobs done (see the module notes).
    pub long_steps: AtomicU64,
}

/// Takes one from a fault-test knob: true if it was above 0.
fn knob(a: &AtomicU64) -> bool {
    a.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
        .is_ok()
}

/// A job that reads a whole file, a step at a time (see the module notes).
enum Long {
    /// A saved take's waveform table, from its audio.
    Scan {
        f: TakeFile,
        file: File,
        path: PathBuf,
        data_at: u64,
        need: u64,
        at: u64,
        peaks: Vec<(f32, f32)>,
        samples: Vec<f32>,
    },
    /// A whole take's audio, for a fit.
    ReadAll {
        take: u16,
        file: File,
        data_at: u64,
        audio: Vec<f32>,
        at: usize,
    },
    /// Answered (left in its place for a moment).
    Done,
}

/// A request answered at once, or a long job started.
enum Started {
    Done(FromDisk),
    Long(Long),
}

/// A saved take's file, checked against the save.
struct Checked {
    file: File,
    path: PathBuf,
    data_at: u64,
    need: u64,
    /// From the `.peaks` file beside it, if that fits.
    peaks: Option<Vec<(f32, f32)>>,
}

fn from_le(bytes: &[u8], out: &mut [f32]) {
    for (o, b) in out.iter_mut().zip(bytes.chunks_exact(4)) {
        let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        *o = if v.is_finite() { v } else { 0.0 };
    }
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
        if knob(&self.stats.fail_writes) {
            self.stats.errors.fetch_add(1, Ordering::Relaxed);
            return Err(io::Error::other("a write failed (fault test)"));
        }
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
        from_le(&self.bytes[..got / 4 * 4], out);
        self.stats.reads.fetch_add(1, Ordering::Relaxed);
        if knob(&self.stats.short_reads) {
            return Ok(got / 8);
        }
        Ok(got / 4)
    }

    fn finish(&mut self, take: u16, frames: usize, channels: usize) -> io::Result<()> {
        if knob(&self.stats.fail_finishes) {
            self.stats.errors.fetch_add(1, Ordering::Relaxed);
            return Err(io::Error::other("a finish failed (fault test)"));
        }
        let rate = self.rate;
        let data = (frames as u64)
            .checked_mul(channels as u64 * 4)
            .filter(|&d| d <= WAV_MAX_DATA)
            .ok_or_else(|| io::Error::other("too long for a WAV file's header"))?
            as u32;
        let e = self.entry(take, channels)?;
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

    /// Open a saved take's file and check it against what the save says;
    /// its waveform table comes from the `.peaks` file beside it, or else
    /// from its audio, a long job.
    fn open_saved(&mut self, f: TakeFile) -> Started {
        match self.check_saved(&f) {
            Err(e) => Started::Done(FromDisk::Opened {
                file: f,
                ok: Err(e),
            }),
            Ok(Checked {
                path,
                data_at,
                peaks: Some(peaks),
                ..
            }) => {
                let ok = self.keep_open(&f, path, data_at, peaks);
                Started::Done(FromDisk::Opened { file: f, ok })
            }
            Ok(Checked {
                file,
                path,
                data_at,
                need,
                peaks: None,
            }) => Started::Long(Long::Scan {
                f,
                file,
                path,
                data_at,
                need,
                at: 0,
                peaks: Vec::with_capacity((need / 4).div_ceil(DISK_PEAK as u64) as usize),
                samples: Vec::with_capacity(STEP_BYTES / 4),
            }),
        }
    }

    fn check_saved(&self, f: &TakeFile) -> Result<Checked, String> {
        let path = self
            .path(&f.file)
            .ok_or_else(|| format!("take {}: bad file name {:?}", f.id, f.file))?;
        let mut file = File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut head = vec![0u8; 4096];
        let n = file.read(&mut head).map_err(|e| e.to_string())?;
        let (data_at, len, ch, rate) = crate::save::wav_layout(&head[..n])?;
        let size = file.metadata().map_err(|e| e.to_string())?.len();
        let need = (f.frames as u64).saturating_mul(f.channels as u64 * 4);
        if ch != f.channels
            || rate != self.rate
            || len < need
            || size < data_at.saturating_add(need)
        {
            return Err(format!(
                "{}: {ch} channels at {rate} Hz, {} bytes of audio; the save says {} channels at {} Hz, {need}",
                path.display(),
                len.min(size.saturating_sub(data_at)),
                f.channels,
                self.rate
            ));
        }
        let mut side = path.clone().into_os_string();
        side.push(".peaks");
        let blocks = (need / 4).div_ceil(DISK_PEAK as u64);
        let peaks = match std::fs::read(&side) {
            Ok(b) if b.len() as u64 == blocks * 8 => Some(
                b.chunks_exact(8)
                    .map(|x| {
                        (
                            f32::from_le_bytes([x[0], x[1], x[2], x[3]]),
                            f32::from_le_bytes([x[4], x[5], x[6], x[7]]),
                        )
                    })
                    .collect(),
            ),
            _ => None,
        };
        Ok(Checked {
            file,
            path,
            data_at,
            need,
            peaks,
        })
    }

    /// A checked saved take's file stays open, for its reads.
    fn keep_open(
        &mut self,
        f: &TakeFile,
        path: PathBuf,
        data_at: u64,
        peaks: Vec<(f32, f32)>,
    ) -> Result<(), String> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .or_else(|_| File::open(&path))
            .map_err(|e| e.to_string())?;
        self.names.insert(f.id, f.file.clone());
        self.open.insert(
            f.id,
            Entry {
                file,
                path,
                data_at,
                peaks,
            },
        );
        Ok(())
    }

    /// A whole take's audio, for a fit: a long job.
    fn read_all(&mut self, take: u16, frames: usize, channels: usize) -> Started {
        let file = self.open.get(&take).and_then(|e| {
            let f = e.file.try_clone().ok()?;
            Some((f, e.data_at))
        });
        match (file, frames.checked_mul(channels)) {
            (Some((file, data_at)), Some(n)) => Started::Long(Long::ReadAll {
                take,
                file,
                data_at,
                audio: vec![0.0; n],
                at: 0,
            }),
            _ => Started::Done(FromDisk::Audio { take, audio: None }),
        }
    }

    /// One step of a long job: its answer once it is done.
    fn step(&mut self, job: &mut Long) -> Option<FromDisk> {
        self.stats.long_steps.fetch_add(1, Ordering::Relaxed);
        match job {
            Long::Scan {
                file,
                data_at,
                need,
                at,
                peaks,
                samples,
                ..
            } => {
                let n = (*need - *at).min(STEP_BYTES as u64) as usize;
                self.bytes.resize(n, 0);
                let bytes = &mut self.bytes;
                let r = timed(&self.stats, || {
                    file.read_exact_at(&mut bytes[..n], *data_at + *at)
                });
                if let Err(e) = r {
                    let Long::Scan { f, path, .. } = std::mem::replace(job, Long::Done) else {
                        unreachable!()
                    };
                    return Some(FromDisk::Opened {
                        file: f,
                        ok: Err(format!("{}: {e}", path.display())),
                    });
                }
                self.stats.reads.fetch_add(1, Ordering::Relaxed);
                samples.resize(n / 4, 0.0);
                from_le(&self.bytes[..n], samples);
                note_peaks(peaks, (*at / 4) as usize, samples);
                *at += n as u64;
                if *at < *need {
                    return None;
                }
                let Long::Scan {
                    f,
                    path,
                    data_at,
                    peaks,
                    ..
                } = std::mem::replace(job, Long::Done)
                else {
                    unreachable!()
                };
                let ok = self.keep_open(&f, path, data_at, peaks);
                Some(FromDisk::Opened { file: f, ok })
            }
            Long::ReadAll {
                take,
                file,
                data_at,
                audio,
                at,
            } => {
                let end = (*at + STEP_BYTES / 4).min(audio.len());
                let n = (end - *at) * 4;
                self.bytes.resize(n, 0);
                let bytes = &mut self.bytes;
                let from = *data_at + *at as u64 * 4;
                let r = timed(&self.stats, || file.read_exact_at(&mut bytes[..n], from));
                if r.is_err() {
                    return Some(FromDisk::Audio {
                        take: *take,
                        audio: None,
                    });
                }
                self.stats.reads.fetch_add(1, Ordering::Relaxed);
                from_le(&self.bytes[..n], &mut audio[*at..end]);
                *at = end;
                (end == audio.len()).then(|| FromDisk::Audio {
                    take: *take,
                    audio: Some(std::mem::take(audio)),
                })
            }
            Long::Done => None,
        }
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

    /// The thread's loop: answer each request in order until the host
    /// goes, long jobs a step at a time (see the module notes).
    pub fn serve(mut self, jobs: Receiver<ToDisk>, done: Sender<FromDisk>) {
        let mut long: VecDeque<Long> = VecDeque::new();
        let mut between = 0;
        loop {
            let m = if long.is_empty() {
                match jobs.recv() {
                    Ok(m) => Some(m),
                    Err(_) => return,
                }
            } else if between < BETWEEN {
                // (With the host gone, the long jobs still finish: their
                // answers may be waited for.)
                jobs.try_recv().ok()
            } else {
                None
            };
            let reply = match m {
                Some(m) => {
                    between += 1;
                    self.answer(m, &mut long)
                }
                None => {
                    between = 0;
                    let job = long.front_mut().expect("a long job");
                    let r = self.step(job);
                    if r.is_some() {
                        long.pop_front();
                    }
                    r
                }
            };
            if let Some(r) = reply {
                if done.send(r).is_err() {
                    return;
                }
            }
        }
    }

    /// One request: its answer, or none (a name, or a long job started).
    fn answer(&mut self, m: ToDisk, long: &mut VecDeque<Long>) -> Option<FromDisk> {
        let started = match m {
            ToDisk::Name { take, name } => {
                self.open.remove(&take);
                self.names.insert(take, name);
                return None;
            }
            ToDisk::Write {
                take,
                chunk,
                channels,
                data,
                len,
            } => {
                let ok = self.write(take, chunk, channels, &data[..len]).is_ok();
                Started::Done(FromDisk::Written {
                    take,
                    chunk,
                    buf: data,
                    ok,
                })
            }
            ToDisk::Read {
                take,
                chunk,
                mut buf,
            } => {
                let got = self.read(take, chunk, &mut buf).ok();
                Started::Done(FromDisk::Read {
                    take,
                    chunk,
                    buf,
                    got,
                })
            }
            ToDisk::Finish {
                take,
                frames,
                channels,
            } => Started::Done(FromDisk::Finished {
                take,
                ok: self.finish(take, frames, channels).is_ok(),
            }),
            ToDisk::Open { file } => self.open_saved(file),
            ToDisk::ReadAll {
                take,
                frames,
                channels,
            } => self.read_all(take, frames, channels),
            ToDisk::Wave {
                take,
                frames,
                channels,
                columns,
            } => Started::Done(FromDisk::Wave(self.wave(take, frames, channels, columns))),
        };
        match started {
            Started::Done(r) => Some(r),
            Started::Long(j) => {
                long.push_back(j);
                None
            }
        }
    }

    /// The file name of take `take`, if it has one.
    #[allow(dead_code)]
    pub fn name(&self, take: u16) -> Option<&str> {
        self.names.get(&take).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::channel;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mpfree-disk-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn sig(i: usize) -> f32 {
        ((i * 7919) % 20011) as f32 / 20011.0 - 0.5
    }

    /// A finished mono take `take` of `chunks` whole chunks, in `name`.
    fn take_file(d: &mut Disk, take: u16, name: &str, chunks: usize) -> Vec<f32> {
        d.names.insert(take, name.into());
        let audio: Vec<f32> = (0..chunks * CHUNK).map(sig).collect();
        for c in 0..chunks {
            d.write(take, c as u32, 1, &audio[c * CHUNK..(c + 1) * CHUNK])
                .unwrap();
        }
        d.finish(take, chunks * CHUNK, 1).unwrap();
        audio
    }

    /// Run the thread over `jobs` (all sent before it starts), giving its
    /// answers in the order they came.
    fn serve(d: Disk, jobs: Vec<ToDisk>) -> Vec<FromDisk> {
        let (tx, rx) = channel();
        let (dtx, drx) = channel();
        for j in jobs {
            tx.send(j).unwrap();
        }
        drop(tx);
        std::thread::spawn(move || d.serve(rx, dtx)).join().unwrap();
        drx.into_iter().collect()
    }

    #[test]
    fn a_whole_take_read_for_a_fit_lets_the_reads_that_play_go_first() {
        let dir = scratch("long-read");
        let stats = Arc::new(DiskStats::default());
        let mut d = Disk::new(dir.clone(), 44_100, stats.clone());
        let big = take_file(&mut d, 1, "big.wav", 64);
        let small = take_file(&mut d, 2, "small.wav", 1);
        let replies = serve(
            d,
            vec![
                ToDisk::ReadAll {
                    take: 1,
                    frames: big.len(),
                    channels: 1,
                },
                ToDisk::Read {
                    take: 2,
                    chunk: 0,
                    buf: vec![0.0; CHUNK],
                },
            ],
        );
        assert_eq!(replies.len(), 2);
        match &replies[0] {
            FromDisk::Read {
                take: 2,
                buf,
                got: Some(n),
                ..
            } => assert!(*n == CHUNK && buf[..] == small[..]),
            _ => panic!("the read that plays was answered after the long job"),
        }
        match &replies[1] {
            FromDisk::Audio {
                take: 1,
                audio: Some(a),
            } => assert!(a[..] == big[..]),
            _ => panic!("the whole take was not read"),
        }
        let steps = (big.len() * 4).div_ceil(STEP_BYTES) as u64;
        assert_eq!(stats.long_steps.load(Ordering::Relaxed), steps);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_saved_take_with_no_peaks_file_is_scanned_a_step_at_a_time() {
        let dir = scratch("long-scan");
        let stats = Arc::new(DiskStats::default());
        let mut d = Disk::new(dir.clone(), 44_100, stats.clone());
        let audio = take_file(&mut d, 1, "old.wav", 40);
        let peaks = d.open[&1].peaks.clone();
        let small = take_file(&mut d, 2, "small.wav", 1);
        std::fs::remove_file(dir.join("old.wav.peaks")).unwrap();
        d.open.remove(&1);
        let file = TakeFile {
            id: 7,
            rec: crate::RecordingId::NONE,
            track: 0,
            group: 0,
            pass: 0,
            bpm: 0.0,
            anchor: 0.0,
            frames: audio.len(),
            channels: 1,
            file: "old.wav".into(),
            audio: Vec::new(),
        };
        let (tx, rx) = channel();
        let (dtx, drx) = channel();
        tx.send(ToDisk::Open { file }).unwrap();
        tx.send(ToDisk::Read {
            take: 2,
            chunk: 0,
            buf: vec![0.0; CHUNK],
        })
        .unwrap();
        // Once open, its waveform is the one the file had.
        let t = std::thread::spawn(move || d.serve(rx, dtx));
        let first = drx.recv().unwrap();
        assert!(
            matches!(&first, FromDisk::Read { take: 2, buf, got: Some(_), .. } if buf[..] == small[..]),
            "the read that plays was answered after the scan"
        );
        match drx.recv().unwrap() {
            FromDisk::Opened { file, ok } => assert!(ok.is_ok() && file.id == 7, "{ok:?}"),
            _ => panic!("not opened"),
        }
        tx.send(ToDisk::Wave {
            take: 7,
            frames: audio.len(),
            channels: 1,
            columns: peaks.len(),
        })
        .unwrap();
        match drx.recv().unwrap() {
            FromDisk::Wave(Some(w)) => {
                let want: Vec<(f32, f32)> = peaks.clone();
                assert_eq!(w.columns, want);
            }
            _ => panic!("no waveform"),
        }
        drop(tx);
        t.join().unwrap();
        assert!(stats.long_steps.load(Ordering::Relaxed) > 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_take_too_long_for_a_wav_header_is_never_finished_with_a_wrong_one() {
        let dir = scratch("too-long");
        let mut d = Disk::new(dir.clone(), 44_100, Arc::new(DiskStats::default()));
        d.names.insert(1, "long.wav".into());
        d.write(1, 0, 2, &vec![0.25; CHUNK]).unwrap();
        let before = std::fs::read(dir.join("long.wav")).unwrap();
        // One stereo frame past what a header can give.
        let frames = (WAV_MAX_DATA / 8 + 1) as usize;
        assert!(d.finish(1, frames, 2).is_err());
        // On a 32-bit MPC this many frames once wrapped to a small length.
        assert!(d.finish(1, usize::MAX / 2, 2).is_err());
        assert_eq!(std::fs::read(dir.join("long.wav")).unwrap(), before);
        // Its true length still finishes it.
        d.finish(1, CHUNK / 2, 2).unwrap();
        let b = std::fs::read(dir.join("long.wav")).unwrap();
        let (at, len, ch, _) = crate::save::wav_layout(&b).unwrap();
        assert_eq!((at, len, ch), (HEADER, CHUNK as u64 * 4, 2));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_fault_knobs_fail_what_they_say_once_each() {
        let dir = scratch("knobs");
        let stats = Arc::new(DiskStats::default());
        let mut d = Disk::new(dir.clone(), 44_100, stats.clone());
        d.names.insert(1, "k.wav".into());
        stats.fail_writes.store(1, Ordering::Relaxed);
        assert!(d.write(1, 0, 1, &vec![0.5; CHUNK]).is_err());
        assert!(d.write(1, 0, 1, &vec![0.5; CHUNK]).is_ok());
        stats.fail_finishes.store(1, Ordering::Relaxed);
        assert!(d.finish(1, CHUNK, 1).is_err());
        assert!(d.finish(1, CHUNK, 1).is_ok());
        stats.short_reads.store(1, Ordering::Relaxed);
        let mut buf = vec![0.0; CHUNK];
        assert_eq!(d.read(1, 0, &mut buf).unwrap(), CHUNK / 2);
        assert_eq!(d.read(1, 0, &mut buf).unwrap(), CHUNK);
        assert_eq!(stats.errors.load(Ordering::Relaxed), 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
