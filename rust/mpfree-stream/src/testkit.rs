//! The stream tests' kit, for this crate's tests and for any engine built
//! on the layer (behind the `testkit` feature): sound files and memory
//! sounds whose every sample says which sound and which frame it is, a
//! checker that tells whether a period played exactly what it should, and
//! a rig that runs a player and its coordinator period by period in one
//! thread.
//!
//! The checker is the unload and replace test's: every frame played must
//! be silence, or exactly the frame it should be of the generation the
//! sound thread has now. An engine's own test makes its cursors play
//! marked sounds, captures [`Expect::of`] each cursor before its period
//! and checks after.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use crate::coord::{Coordinator, Inline, Plan};
use crate::cursor::Cursor;
use crate::player::{Config, Player};

/// A fresh folder for one test's files.
pub fn folder(name: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "mpfree-stream-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("temp folder");
    d
}

/// How a test file stores its samples.
#[derive(Clone, Copy, Debug)]
pub enum Store {
    Pcm16,
    Pcm24,
    Float,
}

/// The sample of file `marker` at `frame`, channel `ch`, as a whole number
/// below 2^23, never 0. Channel 0 holds the marker and the frame's low 16
/// bits; channel 1 the frame's high bits. A 24-bit or float file holds it
/// as `v / 2^23` exactly; a 16-bit file holds only `v % 32768`.
pub fn value(marker: u32, frame: u64, ch: usize) -> u32 {
    match ch {
        0 => (marker << 16) | (frame as u32 & 0xFFFF),
        _ => (frame >> 16) as u32 + 1,
    }
}

/// What a 24-bit or float file's sample plays as.
pub fn played(marker: u32, frame: u64, ch: usize) -> f32 {
    value(marker, frame, ch) as f32 / 8_388_608.0
}

/// What a 16-bit file's sample plays as.
pub fn played16(marker: u32, frame: u64, ch: usize) -> f32 {
    let v = (value(marker, frame, ch) % 32_767 + 1) as i16;
    f32::from(v) / 32_768.0
}

/// Read a sample of a 24-bit or float file back into (marker, frame low
/// bits) or (frame high bits).
pub fn unplay(x: f32) -> u32 {
    (x * 8_388_608.0) as u32
}

/// Write a WAV of `frames` frames.
pub fn wav(path: &Path, store: Store, channels: u16, frames: u64, marker: u32) {
    let (tag, bits): (u16, u16) = match store {
        Store::Pcm16 => (1, 16),
        Store::Pcm24 => (1, 24),
        Store::Float => (3, 32),
    };
    let align = u32::from(channels * bits / 8);
    let rate = 44_100u32;
    let mut data = Vec::with_capacity(frames as usize * align as usize);
    for f in 0..frames {
        for c in 0..usize::from(channels) {
            match store {
                Store::Pcm16 => {
                    let v = (value(marker, f, c) % 32_767 + 1) as i16;
                    data.extend_from_slice(&v.to_le_bytes());
                }
                Store::Pcm24 => {
                    let v = value(marker, f, c);
                    data.extend_from_slice(&v.to_le_bytes()[..3]);
                }
                Store::Float => {
                    data.extend_from_slice(&played(marker, f, c).to_le_bytes());
                }
            }
        }
    }
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&tag.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * align).to_le_bytes());
    fmt.extend_from_slice(&(align as u16).to_le_bytes());
    fmt.extend_from_slice(&bits.to_le_bytes());
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&(4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    f.extend_from_slice(b"WAVE");
    f.extend_from_slice(b"fmt ");
    f.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    f.extend_from_slice(&fmt);
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&data);
    fs::write(path, f).expect("write wav");
}

/// A 32-bit float WAV at `rate` holding `samples` (interleaved,
/// `channels` a frame).
pub fn float_wav(path: &Path, rate: u32, channels: u16, samples: &[f32]) {
    let align = u32::from(channels) * 4;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&3u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * align).to_le_bytes());
    fmt.extend_from_slice(&(align as u16).to_le_bytes());
    fmt.extend_from_slice(&32u16.to_le_bytes());
    let data: Vec<u8> = samples.iter().flat_map(|x| x.to_le_bytes()).collect();
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&(4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    f.extend_from_slice(b"WAVE");
    f.extend_from_slice(b"fmt ");
    f.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    f.extend_from_slice(&fmt);
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&data);
    fs::write(path, f).expect("write wav");
}

/// A 24-bit WAV at `rate` holding `samples` (interleaved, `channels` a
/// frame), each rounded to 24 bits.
pub fn int24_wav(path: &Path, rate: u32, channels: u16, samples: &[f32]) {
    let align = u32::from(channels) * 3;
    let mut fmt = Vec::new();
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&channels.to_le_bytes());
    fmt.extend_from_slice(&rate.to_le_bytes());
    fmt.extend_from_slice(&(rate * align).to_le_bytes());
    fmt.extend_from_slice(&(align as u16).to_le_bytes());
    fmt.extend_from_slice(&24u16.to_le_bytes());
    let data: Vec<u8> = samples
        .iter()
        .flat_map(|&x| {
            let v = (f64::from(x) * 8_388_608.0)
                .round()
                .clamp(-8_388_608.0, 8_388_607.0) as i32;
            let b = v.to_le_bytes();
            [b[0], b[1], b[2]]
        })
        .collect();
    let mut f = b"RIFF".to_vec();
    f.extend_from_slice(&(4 + 8 + fmt.len() as u32 + 8 + data.len() as u32).to_le_bytes());
    f.extend_from_slice(b"WAVE");
    f.extend_from_slice(b"fmt ");
    f.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    f.extend_from_slice(&fmt);
    f.extend_from_slice(b"data");
    f.extend_from_slice(&(data.len() as u32).to_le_bytes());
    f.extend_from_slice(&data);
    fs::write(path, f).expect("write wav");
}

/// A player and its coordinator, reading files inline.
pub struct Rig {
    pub p: Player,
    pub c: Coordinator,
}

impl Rig {
    pub fn new(cfg: Config, plan: Plan) -> Rig {
        let (p, rx, tx) = Player::new(cfg);
        let c = Coordinator::new(cfg, plan, rx, tx, Box::<Inline>::default());
        Rig { p, c }
    }

    /// One period of `out.len() / 2` frames for each cursor, then a
    /// coordinator round. Gives the frames missed.
    pub fn period(&mut self, cursors: &mut [&mut Cursor], outs: &mut [Vec<f32>]) -> u64 {
        self.p.begin();
        let mut missed = 0;
        let mut frames = 0;
        for (k, o) in cursors.iter_mut().zip(outs.iter_mut()) {
            missed += k.play(&mut self.p, o);
            frames = o.len() / 2;
        }
        self.p.end(frames as u32);
        self.c.step();
        missed
    }

    /// Hold the cursors where they are until everything they want ahead
    /// is ready (or `periods` pass).
    pub fn warm(&mut self, cursors: &mut [&mut Cursor], periods: usize) {
        let mut outs = vec![vec![0.0; 512]; cursors.len()];
        let mut quiet = 0;
        for _ in 0..periods {
            let at: Vec<(u64, f64)> = cursors
                .iter()
                .map(|k| (k.frame(), k.position() - k.frame() as f64))
                .collect();
            let before = (self.p.ready(), self.c.cached());
            self.period(cursors, &mut outs);
            for (k, (f, _)) in cursors.iter_mut().zip(at) {
                k.seek(f);
            }
            if (self.p.ready(), self.c.cached()) == before && self.p.ready() == self.c.cached() {
                quiet += 1;
                if quiet > 8 {
                    return;
                }
            } else {
                quiet = 0;
            }
        }
    }
}

/// A memory sound of `frames` frames marked like [`wav`]'s 24-bit and
/// float files.
pub fn memory(channels: u16, frames: u64, marker: u32) -> Arc<[f32]> {
    let ch = usize::from(channels);
    (0..frames as usize * ch)
        .map(|i| played(marker, (i / ch) as u64, i % ch))
        .collect()
}

/// `count` marked files of `frames` stereo frames, 24-bit and float in
/// turn: the k-th open of a run (generation k, when every open goes
/// through one coordinator) gets file [`marker`]`(k, count)`.
pub fn files(dir: &Path, frames: u64, count: u32) -> Vec<PathBuf> {
    (1..=count)
        .map(|m| {
            let p = dir.join(format!("{m}.wav"));
            let store = if m % 2 == 0 {
                Store::Float
            } else {
                Store::Pcm24
            };
            wav(&p, store, 2, frames, m);
            p
        })
        .collect()
}

/// The marker of the file opened as generation `gen` from [`files`].
pub fn marker(gen: u32, count: u32) -> u32 {
    (gen - 1) % count + 1
}

/// What a cursor should play this period, taken just before it plays (at
/// speed 1 from a whole frame).
#[derive(Clone, Copy, Debug)]
pub struct Expect {
    pub source: u16,
    pub from: u64,
    pub looping: Option<(u64, u64)>,
    pub end: Option<u64>,
}

impl Expect {
    pub fn of(k: &Cursor) -> Expect {
        Expect {
            source: k.source(),
            from: k.frame(),
            looping: k.looping(),
            end: k.end(),
        }
    }

    /// Check a period's output (stereo) against the source as the sound
    /// thread has it now: each frame silent, or exactly the frame it
    /// should be of the sound of `marker_of(generation)`, which is
    /// `frames` long. Panics on the first frame that isn't. Gives the
    /// frames that played sound.
    pub fn check(
        &self,
        p: &Player,
        out: &[f32],
        frames: u64,
        marker_of: impl Fn(u32) -> u32,
    ) -> u64 {
        let (gen, open) = p.source(self.source).expect("a source");
        let mut sounding = 0;
        for (j, f) in out.chunks_exact(2).enumerate() {
            if f == [0.0, 0.0] {
                continue;
            }
            assert!(open, "a closed source played (source {})", self.source);
            let at = self.from + j as u64;
            let file = match self.looping {
                Some((s, e)) if self.from < e && at >= e => s + (at - e) % (e - s),
                _ => at,
            };
            assert!(file < frames, "played past the end");
            assert!(
                self.end.is_none_or(|e| file < e),
                "played past the cursor's end"
            );
            let v = unplay(f[0]);
            let (m, lo) = (v >> 16, v & 0xFFFF);
            assert_eq!(
                m,
                marker_of(gen),
                "audio of another generation (source {})",
                self.source
            );
            assert_eq!(u64::from(lo), file & 0xFFFF, "audio of another frame");
            assert_eq!(unplay(f[1]), (file >> 16) as u32 + 1, "half a frame");
            sounding += 1;
        }
        sounding
    }
}

// ---- the model stick ------------------------------------------------------

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use crate::fsio::Fs;

/// One file on the model stick: what the running system reads (`cache`),
/// what is on the stick (`disk`) and the writes not yet synced.
#[derive(Clone, Debug, Default)]
struct Inode {
    cache: Vec<u8>,
    disk: Vec<u8>,
    pending: Vec<Pending>,
}

#[derive(Clone, Debug)]
enum Pending {
    /// A write, numbered among all writes on the stick.
    Write {
        n: u64,
        at: u64,
        bytes: Vec<u8>,
    },
    SetLen(u64),
}

fn apply(disk: &mut Vec<u8>, p: &Pending, sectors: Option<usize>) {
    match p {
        Pending::Write { at, bytes, .. } => {
            let land = sectors.map_or(bytes.len(), |s| (s * 512).min(bytes.len()));
            if land == 0 {
                return;
            }
            let at = *at as usize;
            if disk.len() < at + land {
                disk.resize(at + land, 0);
            }
            disk[at..at + land].copy_from_slice(&bytes[..land]);
        }
        Pending::SetLen(n) => disk.resize(*n as usize, 0),
    }
}

/// How the unsynced data comes back after a cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cut {
    /// Nothing unsynced landed.
    Lost,
    /// Everything landed.
    Landed,
    /// Everything landed but the last write, of which only its first
    /// `n` 512-byte sectors did.
    Torn(usize),
}

/// Which names (made, renamed, removed since the folder's last sync) come
/// back after a cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Names {
    /// None of them.
    Lost,
    /// All of them.
    Landed,
    /// Only the latest: the stick wrote its folder out of order.
    LastOnly,
}

#[derive(Clone, Debug)]
enum NameOp {
    Made(PathBuf, usize),
    Renamed(PathBuf, PathBuf, usize),
    Removed(PathBuf),
}

/// Damage a cut does beyond the write in flight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Damage {
    /// The 4 KiB block after the last write.
    pub next_block: bool,
    /// The checkpoint slot the last write didn't touch (when it was to a
    /// checkpoint).
    pub other_slot: bool,
    /// The first 4 KiB of every `.recording` file.
    pub header: bool,
}

/// A model stick for the recording tests: a cut loses whatever wasn't
/// synced (a file's data by its own sync, names by the folder's sync),
/// and the write in flight can land torn. Operations can be made to fail
/// by number, the stick can fill or be pulled, and the power can go after
/// any operation.
#[derive(Clone, Debug, Default)]
pub struct ModelStick {
    inodes: Vec<Inode>,
    names: BTreeMap<PathBuf, usize>,
    durable: BTreeMap<PathBuf, usize>,
    name_ops: Vec<NameOp>,
    /// Operations that change the stick, so far.
    pub ops: u64,
    writes: u64,
    /// The last write: inode, where, how long.
    last: Option<(usize, u64, usize)>,
    /// These operations (by number, from 1) fail. A failed write lands
    /// its first half; a failed sync drops the unsynced pages, as Linux
    /// does, and later syncs succeed without them.
    pub fail: BTreeSet<u64>,
    /// After this many operations the power is gone: every later one
    /// fails and changes nothing.
    pub cut_after: Option<u64>,
    /// Bytes the stick holds; a write past them fails as full.
    pub capacity: Option<u64>,
    /// The stick is out: everything fails as gone.
    pub pulled: bool,
    /// Errors handed out.
    pub errors: u64,
    /// Each sync moves this clock on by this many seconds: a slow stick.
    pub sync_cost: Option<(Arc<FakeClock>, f64)>,
}

/// A clock the tests move by hand.
#[derive(Debug, Default)]
pub struct FakeClock {
    bits: std::sync::atomic::AtomicU64,
}

impl FakeClock {
    pub fn new() -> Arc<FakeClock> {
        Arc::new(FakeClock::default())
    }
    pub fn advance(&self, seconds: f64) {
        let now = self.now_s() + seconds;
        self.bits.store(now.to_bits(), Ordering::Relaxed);
    }
    pub fn now_s(&self) -> f64 {
        f64::from_bits(self.bits.load(Ordering::Relaxed))
    }
}

impl crate::writer::Clock for FakeClock {
    fn now(&self) -> f64 {
        self.now_s()
    }
}

/// A [`ModelStick`] shared between a writer (which owns its file layer)
/// and the test that looks at it.
#[derive(Clone, Debug, Default)]
pub struct SharedStick(pub Arc<std::sync::Mutex<ModelStick>>);

impl SharedStick {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut ModelStick) -> R) -> R {
        f(&mut self.0.lock().expect("stick"))
    }
}

impl Fs for SharedStick {
    fn create(&mut self, path: &Path, new_only: bool) -> io::Result<()> {
        self.with(|s| s.create(path, new_only))
    }
    fn write_at(&mut self, path: &Path, at: u64, bytes: &[u8]) -> io::Result<()> {
        self.with(|s| s.write_at(path, at, bytes))
    }
    fn read_at(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.with(|s| s.read_at(path, at, buf))
    }
    fn len(&mut self, path: &Path) -> io::Result<u64> {
        self.with(|s| s.len(path))
    }
    fn set_len(&mut self, path: &Path, len: u64) -> io::Result<()> {
        self.with(|s| s.set_len(path, len))
    }
    fn sync(&mut self, path: &Path) -> io::Result<()> {
        self.with(|s| s.sync(path))
    }
    fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
        self.with(|s| s.sync_dir(dir))
    }
    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        self.with(|s| s.rename(from, to))
    }
    fn remove(&mut self, path: &Path) -> io::Result<()> {
        self.with(|s| s.remove(path))
    }
    fn list(&mut self, dir: &Path) -> io::Result<Vec<String>> {
        self.with(|s| s.list(dir))
    }
    fn exists(&mut self, path: &Path) -> bool {
        self.with(|s| s.exists(path))
    }
}

fn err(kind: io::ErrorKind, why: &str) -> io::Error {
    io::Error::new(kind, why.to_string())
}

impl ModelStick {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count an operation that changes the stick; Err if it must fail.
    fn op(&mut self) -> io::Result<bool> {
        if self.pulled {
            self.errors += 1;
            return Err(io::Error::from_raw_os_error(19));
        }
        if self.cut_after.is_some_and(|c| self.ops >= c) {
            self.errors += 1;
            return Err(io::Error::from_raw_os_error(5));
        }
        self.ops += 1;
        let fail = self.fail.contains(&self.ops);
        if fail {
            self.errors += 1;
        }
        Ok(fail)
    }

    fn reading(&self) -> io::Result<()> {
        if self.pulled {
            return Err(io::Error::from_raw_os_error(19));
        }
        if self.cut_after.is_some_and(|c| self.ops >= c) {
            return Err(io::Error::from_raw_os_error(5));
        }
        Ok(())
    }

    fn inode(&self, path: &Path) -> io::Result<usize> {
        self.names
            .get(path)
            .copied()
            .ok_or_else(|| err(io::ErrorKind::NotFound, "no such file"))
    }

    fn used(&self) -> u64 {
        self.names
            .values()
            .map(|&i| self.inodes[i].cache.len() as u64)
            .sum()
    }

    /// The bytes of a file as the running system reads them.
    pub fn bytes(&self, path: &Path) -> Option<&[u8]> {
        self.names.get(path).map(|&i| &self.inodes[i].cache[..])
    }

    /// Replace a file's bytes, synced (for setting up damage by hand).
    pub fn put(&mut self, path: &Path, bytes: &[u8]) {
        let i = match self.names.get(path) {
            Some(&i) => i,
            None => {
                self.inodes.push(Inode::default());
                let i = self.inodes.len() - 1;
                self.names.insert(path.to_path_buf(), i);
                self.durable.insert(path.to_path_buf(), i);
                i
            }
        };
        let n = &mut self.inodes[i];
        n.cache = bytes.to_vec();
        n.disk = bytes.to_vec();
        n.pending.clear();
    }

    /// The names on the stick, as the running system sees them.
    pub fn names(&self) -> Vec<PathBuf> {
        self.names.keys().cloned().collect()
    }

    /// The stick as it comes back after the power goes now: unsynced data
    /// by `how`, unsynced names landed or not, then `damage`.
    pub fn cut(&self, how: Cut, names: Names, damage: Damage) -> ModelStick {
        let last_n = self
            .inodes
            .iter()
            .flat_map(|i| i.pending.iter())
            .filter_map(|p| match p {
                Pending::Write { n, .. } => Some(*n),
                Pending::SetLen(_) => None,
            })
            .max();
        let mut inodes = self.inodes.clone();
        for i in inodes.iter_mut() {
            match how {
                Cut::Lost => {}
                Cut::Landed => {
                    for p in &i.pending {
                        apply(&mut i.disk, p, None);
                    }
                }
                Cut::Torn(s) => {
                    for p in &i.pending {
                        let torn = matches!(p, Pending::Write { n, .. } if Some(*n) == last_n);
                        apply(&mut i.disk, p, torn.then_some(s));
                    }
                }
            }
            i.pending.clear();
            i.cache = i.disk.clone();
        }
        let names = match names {
            Names::Landed => self.names.clone(),
            Names::Lost => self.durable.clone(),
            Names::LastOnly => {
                let mut m = self.durable.clone();
                match self.name_ops.last() {
                    Some(NameOp::Made(p, i)) => {
                        m.insert(p.clone(), *i);
                    }
                    Some(NameOp::Renamed(from, to, i)) => {
                        m.remove(from);
                        m.insert(to.clone(), *i);
                    }
                    Some(NameOp::Removed(p)) => {
                        m.remove(p);
                    }
                    None => {}
                }
                m
            }
        };
        let mut out = ModelStick {
            inodes,
            durable: names.clone(),
            names,
            ..ModelStick::default()
        };
        let garble = |b: &mut [u8], seed: u64| {
            let mut x = seed | 1;
            for v in b.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *v = x as u8;
            }
        };
        if let Some((ino, at, len)) = self.last {
            let is_ckpt = self
                .names
                .iter()
                .any(|(p, &i)| i == ino && p.to_string_lossy().ends_with(".wav.checkpoint"));
            let d = &mut out.inodes[ino].disk;
            if damage.next_block {
                let from = (at as usize + len).div_ceil(4096) * 4096;
                let to = (from + 4096).min(d.len());
                if from < to {
                    garble(&mut d[from..to], at + 7);
                }
            }
            if damage.other_slot && is_ckpt {
                let other = if at < 4096 { 4096 } else { 0 };
                let to = (other + 4096).min(d.len());
                if other < to {
                    garble(&mut d[other..to], at + 11);
                }
            }
            out.inodes[ino].cache = out.inodes[ino].disk.clone();
        }
        if damage.header {
            let recs: Vec<usize> = out
                .names
                .iter()
                .filter(|(p, _)| p.to_string_lossy().ends_with(".wav.recording"))
                .map(|(_, &i)| i)
                .collect();
            for i in recs {
                let d = &mut out.inodes[i].disk;
                let to = d.len().min(4096);
                garble(&mut d[..to], 99);
                out.inodes[i].cache = d.clone();
            }
        }
        out
    }
}

impl Fs for ModelStick {
    fn create(&mut self, path: &Path, new_only: bool) -> io::Result<()> {
        if self.op()? {
            return Err(err(io::ErrorKind::Other, "create failed"));
        }
        match self.names.get(path) {
            Some(_) if new_only => Err(err(io::ErrorKind::AlreadyExists, "exists")),
            Some(&i) => {
                let n = &mut self.inodes[i];
                n.cache.clear();
                n.pending.push(Pending::SetLen(0));
                Ok(())
            }
            None => {
                self.inodes.push(Inode::default());
                let i = self.inodes.len() - 1;
                self.names.insert(path.to_path_buf(), i);
                self.name_ops.push(NameOp::Made(path.to_path_buf(), i));
                Ok(())
            }
        }
    }

    fn write_at(&mut self, path: &Path, at: u64, bytes: &[u8]) -> io::Result<()> {
        let fail = self.op()?;
        let i = self.inode(path)?;
        let grows = (at + bytes.len() as u64).saturating_sub(self.inodes[i].cache.len() as u64);
        if self.capacity.is_some_and(|c| self.used() + grows > c) {
            self.errors += 1;
            return Err(io::Error::from_raw_os_error(28));
        }
        let bytes = if fail {
            &bytes[..bytes.len() / 2]
        } else {
            bytes
        };
        self.writes += 1;
        let p = Pending::Write {
            n: self.writes,
            at,
            bytes: bytes.to_vec(),
        };
        let n = &mut self.inodes[i];
        apply(&mut n.cache, &p, None);
        n.pending.push(p);
        self.last = Some((i, at, bytes.len()));
        if fail {
            return Err(err(io::ErrorKind::Other, "write failed"));
        }
        Ok(())
    }

    fn read_at(&mut self, path: &Path, at: u64, buf: &mut [u8]) -> io::Result<usize> {
        self.reading()?;
        let i = self.inode(path)?;
        let c = &self.inodes[i].cache;
        let at = (at as usize).min(c.len());
        let n = buf.len().min(c.len() - at);
        buf[..n].copy_from_slice(&c[at..at + n]);
        Ok(n)
    }

    fn len(&mut self, path: &Path) -> io::Result<u64> {
        self.reading()?;
        let i = self.inode(path)?;
        Ok(self.inodes[i].cache.len() as u64)
    }

    fn set_len(&mut self, path: &Path, len: u64) -> io::Result<()> {
        if self.op()? {
            return Err(err(io::ErrorKind::Other, "truncate failed"));
        }
        let i = self.inode(path)?;
        let n = &mut self.inodes[i];
        n.cache.resize(len as usize, 0);
        n.pending.push(Pending::SetLen(len));
        Ok(())
    }

    fn sync(&mut self, path: &Path) -> io::Result<()> {
        if let Some((clock, s)) = &self.sync_cost {
            clock.advance(*s);
        }
        let fail = self.op()?;
        let i = self.inode(path)?;
        let n = &mut self.inodes[i];
        if fail {
            // Linux: the pages are marked clean and the error reported once.
            n.pending.clear();
            return Err(err(io::ErrorKind::Other, "sync failed"));
        }
        let pending = std::mem::take(&mut n.pending);
        for p in &pending {
            apply(&mut n.disk, p, None);
        }
        Ok(())
    }

    fn sync_dir(&mut self, dir: &Path) -> io::Result<()> {
        if self.op()? {
            return Err(err(io::ErrorKind::Other, "folder sync failed"));
        }
        self.durable.retain(|p, _| p.parent() != Some(dir));
        self.name_ops.clear();
        for (p, &i) in &self.names {
            if p.parent() == Some(dir) {
                self.durable.insert(p.clone(), i);
            }
        }
        Ok(())
    }

    fn rename(&mut self, from: &Path, to: &Path) -> io::Result<()> {
        if self.op()? {
            return Err(err(io::ErrorKind::Other, "rename failed"));
        }
        let i = self.inode(from)?;
        self.names.remove(from);
        self.names.insert(to.to_path_buf(), i);
        self.name_ops
            .push(NameOp::Renamed(from.to_path_buf(), to.to_path_buf(), i));
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        if self.op()? {
            return Err(err(io::ErrorKind::Other, "remove failed"));
        }
        self.inode(path)?;
        self.names.remove(path);
        self.name_ops.push(NameOp::Removed(path.to_path_buf()));
        Ok(())
    }

    fn list(&mut self, dir: &Path) -> io::Result<Vec<String>> {
        self.reading()?;
        Ok(self
            .names
            .keys()
            .filter(|p| p.parent() == Some(dir))
            .filter_map(|p| p.file_name()?.to_str().map(str::to_string))
            .collect())
    }

    fn exists(&mut self, path: &Path) -> bool {
        self.names.contains_key(path)
    }
}

// ---- finished takes for other readers' tests ----------------------------

/// Record `frames` frames of marked audio (marker `marker`, as [`played`])
/// as a finished take in `dir` through the real file system, the way a
/// recording is made. Gives the take's file.
pub fn record_take(dir: &Path, fmt: crate::rec::RecFormat, frames: u64, marker: u32) -> PathBuf {
    use crate::rec::{Meta, RecId, Take};
    let mut fs = crate::fsio::StdFs::new();
    let mut t = Take::arm(
        &mut fs,
        dir,
        "Take",
        RecId::random(),
        fmt,
        Meta::default(),
        1 << 26,
    )
    .expect("arm");
    let ch = usize::from(fmt.channels);
    let mut at = 0u64;
    while at < frames {
        let n = 4096.min(frames - at);
        let block: Vec<f32> = (0..n as usize * ch)
            .map(|i| played(marker, at + (i / ch) as u64, i % ch))
            .collect();
        t.push(&block);
        at += n;
        t.write_ready(&mut fs);
        t.checkpoint(&mut fs);
    }
    t.stop();
    assert!(t.finish(&mut fs), "the take didn't finish: {:?}", t.state());
    t.final_path()
}

/// A finished take's bytes rewritten as RF64 (its `JUNK` becomes `ds64`,
/// the sizes `FFFFFFFF`), as a take past 4 GiB is written, so readers can
/// be tested on RF64 without 4 GiB files.
pub fn as_rf64(take: &[u8]) -> Vec<u8> {
    let u32_at = |i: usize| {
        u64::from(u32::from_le_bytes(
            take[i..i + 4].try_into().expect("4 bytes"),
        ))
    };
    let float = u32_at(52) == 18;
    let data = u32_at(4092);
    let align = u32_at(68) & 0xFFFF;
    let mut b = take.to_vec();
    b[0..4].copy_from_slice(b"RF64");
    b[4..8].copy_from_slice(&[0xFF; 4]);
    b[12..16].copy_from_slice(b"ds64");
    b[16..20].copy_from_slice(&28u32.to_le_bytes());
    b[20..28].copy_from_slice(&(take.len() as u64 - 8).to_le_bytes());
    b[28..36].copy_from_slice(&data.to_le_bytes());
    b[36..44].copy_from_slice(&(data / align).to_le_bytes());
    b[44..48].copy_from_slice(&0u32.to_le_bytes());
    if float {
        b[82..86].copy_from_slice(&[0xFF; 4]);
    }
    b[4092..4096].copy_from_slice(&[0xFF; 4]);
    b
}
