//! Recording a take to a file that survives a power cut or a pulled stick
//! (the recording file protocol, docs/dev/design/recording-files.md).
//!
//! While a take records it is two files: `NAME_ID.wav.recording`, a
//! 4096-byte header and then the audio, and `NAME_ID.wav.checkpoint`, two
//! 4096-byte records saying how much of it is safe. `ID` is the take's
//! random 128-bit recording id in hex, so the files pair up by name and
//! recovery never needs the header. At stop the first becomes
//! `NAME_ID.wav` and the second goes.
//!
//! A [`Take`] is the writer's side of one take. It is driven by calls, so
//! the thread that owns the stick decides when each step runs:
//!
//! - [`Take::arm`] makes the files (steps S1 to S3), before any audio.
//! - [`Take::push`] adds captured frames to capture memory.
//! - [`Take::write_ready`] writes the units that have filled.
//! - [`Take::checkpoint`] runs one checkpoint round (C1 to C5): sync the
//!   audio, then write and sync a record naming it. Only then are those
//!   frames committed and allowed out of capture memory.
//! - [`Take::stop`] then [`Take::finish`] (P1 to P5) make it a finished WAV.
//!
//! Every failure is retried a few times from capture memory and then shown
//! as a state ([`TakeState`]); nothing uncommitted is ever let go.
//! Recovery after a cut is in [`crate::recover`].

use std::collections::VecDeque;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use crate::crc32c;
use crate::fsio::Fs;

/// Where the audio starts in a recording file.
pub const DATA_AT: u64 = 4096;
/// The most audio, with its pad byte, a finished take holds as plain RIFF;
/// past it the take is RF64, so no size field ever holds `FFFFFFFF`.
pub const RIFF_MAX: u64 = 0xFFFF_FFFE - 4088;
/// How much of the end of the committed audio the fast check covers.
pub const TAIL_CHECK: u64 = 64 * 1024;
/// Tries for a write, a sync or a record before a take is at risk.
pub const TRIES: u32 = 3;
/// Bytes in a checkpoint slot.
pub const SLOT: usize = 4096;

const HEADER_MAGIC: &[u8; 16] = b"MPFREE-RECORDING";
const RECORD_MAGIC: &[u8; 8] = b"MPFRCKPT";
pub(crate) const VERSION: u16 = 1;

/// How a take's audio is stored: 24-bit PCM or 32-bit float, mono or
/// stereo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecFormat {
    pub float: bool,
    pub channels: u16,
    pub rate: u32,
}

impl RecFormat {
    /// The WAV format tag: 3 for float, 1 for PCM.
    pub fn tag(self) -> u16 {
        if self.float {
            3
        } else {
            1
        }
    }

    pub fn bits(self) -> u16 {
        if self.float {
            32
        } else {
            24
        }
    }

    /// Bytes in one frame.
    pub fn block_align(self) -> usize {
        usize::from(self.bits() / 8) * usize::from(self.channels)
    }

    /// The write unit: the smallest whole number of frames and of 4 KiB
    /// that is at least 64 KiB (64 KiB for stereo float, 72 KiB for stereo
    /// 24-bit).
    pub fn unit_bytes(self) -> usize {
        let a = self.block_align();
        let l = a / gcd(a, 4096) * 4096;
        65_536usize.div_ceil(l) * l
    }

    /// One MPFree records.
    pub fn valid(self) -> bool {
        (1..=2).contains(&self.channels) && (8_000..=192_000).contains(&self.rate)
    }

    pub(crate) fn from_fields(
        tag: u16,
        channels: u16,
        rate: u32,
        bits: u16,
        align: u16,
    ) -> Option<Self> {
        let float = match (tag, bits) {
            (1, 24) => false,
            (3, 32) => true,
            _ => return None,
        };
        let f = RecFormat {
            float,
            channels,
            rate,
        };
        (f.valid() && usize::from(align) == f.block_align()).then_some(f)
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// A take's recording id: random, 128 bits, in both file names and every
/// checkpoint record.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecId(pub [u8; 16]);

impl RecId {
    /// A fresh random id (from the system's random source; if that can't
    /// be read, from the process's randomly keyed hasher and the clock).
    pub fn random() -> Self {
        let mut b = [0u8; 16];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            if io::Read::read_exact(&mut f, &mut b).is_ok() && b != [0u8; 16] {
                return RecId(b);
            }
        }
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        for (i, half) in b.chunks_mut(8).enumerate() {
            let mut h = RandomState::new().build_hasher();
            h.write_u128(now);
            h.write_u64(n);
            h.write_usize(i);
            half.copy_from_slice(&h.finish().to_le_bytes());
        }
        RecId(b)
    }

    /// 32 lowercase hex digits.
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// From 32 hex digits.
    pub fn parse(s: &str) -> Option<Self> {
        if s.len() != 32 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let mut b = [0u8; 16];
        for (i, out) in b.iter_mut().enumerate() {
            *out = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
        }
        Some(RecId(b))
    }
}

impl fmt::Debug for RecId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RecId({})", self.hex())
    }
}

/// What a checkpoint record says about where the take belongs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Meta {
    /// The project's lasting id.
    pub project: [u8; 16],
    pub track: u16,
    /// Where it starts on the song, in ticks.
    pub start_ticks: u64,
    /// When it was recorded (Unix ms, 0 if the clock isn't set).
    pub recorded_ms: u64,
}

// ---- file names ---------------------------------------------------------

/// The stem `NAME_ID` the take's files are named from.
pub fn stem(name: &str, id: &RecId) -> String {
    format!("{name}_{}", id.hex())
}

/// The recording id at the end of a stem `NAME_ID`.
pub fn id_of_stem(stem: &str) -> Option<RecId> {
    let (name, id) = stem.rsplit_once('_')?;
    if name.is_empty() {
        return None;
    }
    RecId::parse(id)
}

pub fn recording_path(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}.wav.recording"))
}
pub fn checkpoint_path(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}.wav.checkpoint"))
}
pub fn final_path(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}.wav"))
}
pub fn tail_path(dir: &Path, stem: &str) -> PathBuf {
    dir.join(format!("{stem}.tail.unverified.wav"))
}
pub fn unverified_path(dir: &Path, stem: &str, raw: bool) -> PathBuf {
    dir.join(format!(
        "{stem}.unverified.{}",
        if raw { "raw" } else { "wav" }
    ))
}

/// A name the musician's take can be filed under.
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
}

// ---- the header ---------------------------------------------------------

/// The sizes a header is written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sizes {
    /// While recording: `FFFFFFFF`, "not known yet".
    Open,
    /// Finished: `data` bytes of audio (`frames` frames) in a file `end`
    /// bytes long.
    Done { data: u64, frames: u64, end: u64 },
}

impl Sizes {
    /// Sizes for `frames` frames of `fmt`, with the pad byte if odd.
    pub fn done(fmt: RecFormat, frames: u64) -> Self {
        let data = frames * fmt.block_align() as u64;
        Sizes::Done {
            data,
            frames,
            end: DATA_AT + data + (data & 1),
        }
    }
}

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}
pub(crate) fn get16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
pub(crate) fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().expect("4 bytes"))
}
pub(crate) fn get64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().expect("8 bytes"))
}

/// Where the header's MPFree block starts (inside its `JUNK` chunk).
fn block_at(float: bool) -> usize {
    if float {
        86 + 8
    } else {
        72 + 8
    }
}

/// The 4096-byte header of a take.
pub fn header(fmt: RecFormat, id: &RecId, sizes: Sizes) -> Vec<u8> {
    let mut h = vec![0u8; DATA_AT as usize];
    let (rf64, riff, data32, fact, ds64) = match sizes {
        Sizes::Open => (false, u32::MAX, u32::MAX, u32::MAX, None),
        Sizes::Done { data, frames, end } => {
            if end - 8 <= 0xFFFF_FFFE && data <= 0xFFFF_FFFE {
                (false, (end - 8) as u32, data as u32, frames as u32, None)
            } else {
                (
                    true,
                    u32::MAX,
                    u32::MAX,
                    u32::MAX,
                    Some((end - 8, data, frames)),
                )
            }
        }
    };
    h[0..4].copy_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    put32(&mut h, 4, riff);
    h[8..12].copy_from_slice(b"WAVE");
    match ds64 {
        Some((r, d, f)) => {
            h[12..16].copy_from_slice(b"ds64");
            put32(&mut h, 16, 28);
            put64(&mut h, 20, r);
            put64(&mut h, 28, d);
            put64(&mut h, 36, f);
            put32(&mut h, 44, 0);
        }
        None => {
            h[12..16].copy_from_slice(b"JUNK");
            put32(&mut h, 16, 28);
        }
    }
    h[48..52].copy_from_slice(b"fmt ");
    put32(&mut h, 52, if fmt.float { 18 } else { 16 });
    put16(&mut h, 56, fmt.tag());
    put16(&mut h, 58, fmt.channels);
    put32(&mut h, 60, fmt.rate);
    put32(&mut h, 64, fmt.rate * fmt.block_align() as u32);
    put16(&mut h, 68, fmt.block_align() as u16);
    put16(&mut h, 70, fmt.bits());
    let junk = if fmt.float {
        put16(&mut h, 72, 0); // cbSize
        h[74..78].copy_from_slice(b"fact");
        put32(&mut h, 78, 4);
        put32(&mut h, 82, fact);
        86
    } else {
        72
    };
    h[junk..junk + 4].copy_from_slice(b"JUNK");
    put32(&mut h, junk + 4, (4088 - junk - 8) as u32);
    let b = junk + 8;
    h[b..b + 16].copy_from_slice(HEADER_MAGIC);
    h[b + 16..b + 32].copy_from_slice(&id.0);
    put16(&mut h, b + 32, VERSION);
    put16(&mut h, b + 34, 0);
    let crc = crc32c::of(&h[b..b + 36]);
    put32(&mut h, b + 36, crc);
    h[4088..4092].copy_from_slice(b"data");
    put32(&mut h, 4092, data32);
    h
}

/// What the header's MPFree block says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderBlock {
    /// Read cleanly (magic, version and checksum right): this take's id.
    Clean(RecId),
    /// Written by a newer MPFree.
    Newer,
    /// Missing or damaged: says nothing.
    Unreadable,
}

/// Read the MPFree block of header `h` (any length; short means damaged).
pub fn header_block(h: &[u8]) -> HeaderBlock {
    for float in [false, true] {
        let b = block_at(float);
        if h.len() < b + 40 || &h[b..b + 16] != HEADER_MAGIC {
            continue;
        }
        let v = get16(h, b + 32);
        if crc32c::of(&h[b..b + 36]) != get32(h, b + 36) {
            continue;
        }
        if v != VERSION {
            return HeaderBlock::Newer;
        }
        let mut id = [0u8; 16];
        id.copy_from_slice(&h[b + 16..b + 32]);
        return HeaderBlock::Clean(RecId(id));
    }
    HeaderBlock::Unreadable
}

/// The format a header gives, if its `fmt ` chunk reads as one MPFree
/// writes.
pub fn header_format(h: &[u8]) -> Option<RecFormat> {
    if h.len() < 72
        || !matches!(&h[0..4], b"RIFF" | b"RF64")
        || &h[8..12] != b"WAVE"
        || &h[48..52] != b"fmt "
    {
        return None;
    }
    RecFormat::from_fields(
        get16(h, 56),
        get16(h, 58),
        get32(h, 60),
        get16(h, 70),
        get16(h, 68),
    )
}

// ---- checkpoint records -------------------------------------------------

/// One checkpoint record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: RecId,
    pub seq: u64,
    /// The take was stopped and its file finished.
    pub last: bool,
    pub frames: u64,
    pub fmt: RecFormat,
    /// The fast check: CRC-32C of the last committed 64 KiB of audio.
    pub tail_crc: u32,
    /// The whole-take check: CRC-32C of all committed audio.
    pub whole_crc: u32,
    pub meta: Meta,
}

/// What a checkpoint slot holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Valid(Record),
    /// A record of a version this MPFree doesn't know.
    Newer,
    /// Empty or damaged.
    Invalid,
}

impl Record {
    /// Audio bytes committed.
    pub fn bytes(&self) -> u64 {
        self.frames * self.fmt.block_align() as u64
    }

    /// The slot this record goes in.
    pub fn slot(&self) -> usize {
        (self.seq % 2) as usize
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut b = vec![0u8; SLOT];
        b[0..8].copy_from_slice(RECORD_MAGIC);
        put16(&mut b, 8, VERSION);
        put16(&mut b, 10, u16::from(self.last));
        b[16..32].copy_from_slice(&self.id.0);
        put64(&mut b, 32, self.seq);
        put64(&mut b, 40, self.frames);
        put64(&mut b, 48, self.bytes());
        put64(&mut b, 56, DATA_AT);
        put16(&mut b, 64, self.fmt.tag());
        put16(&mut b, 66, self.fmt.channels);
        put32(&mut b, 68, self.fmt.rate);
        put16(&mut b, 72, self.fmt.bits());
        put16(&mut b, 74, self.fmt.block_align() as u16);
        put32(&mut b, 76, self.tail_crc);
        put32(&mut b, 80, self.whole_crc);
        b[84..100].copy_from_slice(&self.meta.project);
        put16(&mut b, 100, self.meta.track);
        put64(&mut b, 104, self.meta.start_ticks);
        put64(&mut b, 112, self.meta.recorded_ms);
        let crc = crc32c::of(&b[..120]);
        put32(&mut b, 120, crc);
        b
    }

    /// Read a slot; valid only for take `id`.
    pub fn decode(b: &[u8], id: &RecId) -> Slot {
        if b.len() < SLOT || &b[0..8] != RECORD_MAGIC {
            return Slot::Invalid;
        }
        if get16(b, 8) != VERSION {
            return Slot::Newer;
        }
        if crc32c::of(&b[..120]) != get32(b, 120)
            || get16(b, 10) > 1
            || b[12..16] != [0; 4]
            || b[102..104] != [0; 2]
            || b[124..SLOT].iter().any(|&x| x != 0)
            || b[16..32] != id.0
            || get64(b, 56) != DATA_AT
        {
            return Slot::Invalid;
        }
        let Some(fmt) = RecFormat::from_fields(
            get16(b, 64),
            get16(b, 66),
            get32(b, 68),
            get16(b, 72),
            get16(b, 74),
        ) else {
            return Slot::Invalid;
        };
        let frames = get64(b, 40);
        if frames.checked_mul(fmt.block_align() as u64) != Some(get64(b, 48)) {
            return Slot::Invalid;
        }
        let mut project = [0u8; 16];
        project.copy_from_slice(&b[84..100]);
        Slot::Valid(Record {
            id: *id,
            seq: get64(b, 32),
            last: get16(b, 10) == 1,
            frames,
            fmt,
            tail_crc: get32(b, 76),
            whole_crc: get32(b, 80),
            meta: Meta {
                project,
                track: get16(b, 100),
                start_ticks: get64(b, 104),
                recorded_ms: get64(b, 112),
            },
        })
    }
}

/// The fast check of committed audio whose last bytes are `tail` (at
/// least the last 64 KiB, or all of it).
pub fn tail_crc(tail: &[u8]) -> u32 {
    let n = tail.len().min(TAIL_CHECK as usize);
    crc32c::of(&tail[tail.len() - n..])
}

// ---- the writer ---------------------------------------------------------

/// Why a take is at risk or stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Why {
    /// Writes keep failing.
    Writes,
    /// Syncs keep failing.
    Syncs,
    /// Checkpoint records can't be written.
    Records,
    /// The stick is full.
    Full,
    /// The stick is gone.
    Gone,
    /// The finished file can't be named or its folder synced.
    Finish,
    /// Capture memory is full (the stick fell too far behind).
    Memory,
}

impl Why {
    fn of(e: &io::Error, otherwise: Why) -> Why {
        match (e.kind(), e.raw_os_error()) {
            (io::ErrorKind::StorageFull, _) | (_, Some(28)) => Why::Full,
            (io::ErrorKind::NotFound, _) | (_, Some(19 | 6)) => Why::Gone,
            _ => otherwise,
        }
    }
}

/// One state per take, shown on its lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TakeState {
    Recording,
    /// Stopped by the musician; the file is being finished.
    Saving,
    /// Finished, synced and named: safe through a cut.
    Saved,
    /// A cut now loses everything after the last committed frame.
    AtRisk(Why),
    /// Recording stopped by itself; what was captured is kept.
    Stopped(Why),
}

/// What went wrong with one take, for Dev.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TakeStats {
    pub write_errors: u32,
    pub sync_errors: u32,
    pub record_errors: u32,
    /// Units written again after a failed sync dropped them.
    pub rewrites: u32,
    /// Checkpoint rounds that committed audio.
    pub rounds: u32,
    /// Leftover checkpoints not removed at stop (removed at next start).
    pub leftovers: u32,
}

struct Unit {
    bytes: Vec<u8>,
    written: bool,
    /// Gave up after [`TRIES`] tries; tried again next round.
    failed: bool,
}

/// The writer's side of one take.
pub struct Take {
    dir: PathBuf,
    stem: String,
    rec: PathBuf,
    ckpt: PathBuf,
    id: RecId,
    fmt: RecFormat,
    meta: Meta,
    unit: usize,
    /// Capture memory: every unit from the first not committed.
    units: VecDeque<Unit>,
    /// The unit number of `units[0]`.
    first: u64,
    /// The unit being filled.
    part: Vec<u8>,
    captured: u64,
    committed: u64,
    /// The sequence number of the last committed record.
    seq: u64,
    whole_crc: u32,
    /// The last committed bytes (up to 64 KiB), for the fast check.
    tail: Vec<u8>,
    limit: usize,
    state: TakeState,
    stopping: bool,
    /// Stop steps done (P1 to P5).
    stop_step: u8,
    stats: TakeStats,
}

impl Take {
    /// Make the take's files in `dir` (S1 to S3): the header, then a
    /// checkpoint whose slot 0 is sequence 0 with nothing committed, both
    /// synced and their folder synced. Capture memory may hold up to
    /// `limit` bytes. On failure nothing is left behind (as far as the
    /// stick allows) and the track doesn't arm.
    pub fn arm(
        fs: &mut dyn Fs,
        dir: &Path,
        name: &str,
        id: RecId,
        fmt: RecFormat,
        meta: Meta,
        limit: usize,
    ) -> io::Result<Take> {
        if !name_ok(name) || !fmt.valid() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a name or format MPFree records",
            ));
        }
        let stem = stem(name, &id);
        let rec = recording_path(dir, &stem);
        let ckpt = checkpoint_path(dir, &stem);
        if fs.exists(&final_path(dir, &stem)) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a take has that name",
            ));
        }
        let mut t = Take {
            dir: dir.to_path_buf(),
            stem,
            rec,
            ckpt,
            id,
            fmt,
            meta,
            unit: fmt.unit_bytes(),
            units: VecDeque::new(),
            first: 0,
            part: Vec::with_capacity(fmt.unit_bytes()),
            captured: 0,
            committed: 0,
            seq: 0,
            whole_crc: 0,
            tail: Vec::new(),
            limit: limit.max(2 * fmt.unit_bytes()),
            state: TakeState::Recording,
            stopping: false,
            stop_step: 0,
            stats: TakeStats::default(),
        };
        let made = t.make(fs);
        if made.is_err() {
            let _ = fs.remove(&t.rec);
            let _ = fs.remove(&t.ckpt);
        }
        made.map(|()| t)
    }

    fn make(&mut self, fs: &mut dyn Fs) -> io::Result<()> {
        // S1
        fs.create(&self.rec, true)?;
        fs.write_at(&self.rec, 0, &header(self.fmt, &self.id, Sizes::Open))?;
        // S2: slot 0 = sequence 0, nothing committed; slot 1 = zeros.
        fs.create(&self.ckpt, true)?;
        let mut both = self.record(0, 0, false, 0).encode();
        both.resize(2 * SLOT, 0);
        fs.write_at(&self.ckpt, 0, &both)?;
        // S3
        fs.sync(&self.rec)?;
        fs.sync(&self.ckpt)?;
        fs.sync_dir(&self.dir)
    }

    fn record(&self, seq: u64, frames: u64, last: bool, tail_crc: u32) -> Record {
        Record {
            id: self.id,
            seq,
            last,
            frames,
            fmt: self.fmt,
            tail_crc,
            whole_crc: self.whole_crc,
            meta: self.meta,
        }
    }

    pub fn id(&self) -> RecId {
        self.id
    }
    pub fn stem(&self) -> &str {
        &self.stem
    }
    pub fn format(&self) -> RecFormat {
        self.fmt
    }
    pub fn state(&self) -> TakeState {
        self.state
    }
    pub fn stats(&self) -> TakeStats {
        self.stats
    }
    /// Frames captured so far.
    pub fn captured(&self) -> u64 {
        self.captured
    }
    /// Frames committed (safe through a cut on a stick that keeps synced
    /// data).
    pub fn committed(&self) -> u64 {
        self.committed
    }
    /// Frames captured but not yet committed.
    pub fn unsaved(&self) -> u64 {
        self.captured - self.committed
    }
    /// Bytes in capture memory.
    pub fn memory(&self) -> usize {
        self.units.len() * self.unit + self.part.len()
    }
    /// Capture memory is at least 90% full: time to warn.
    pub fn nearly_full(&self) -> bool {
        self.memory() * 10 >= self.limit * 9
    }
    /// Where the finished take is (once Saved).
    pub fn final_path(&self) -> PathBuf {
        final_path(&self.dir, &self.stem)
    }

    /// Add captured frames (interleaved, the take's channels) to capture
    /// memory; returns how many were taken. When capture memory is full,
    /// recording stops at that frame ([`TakeState::Stopped`]) and later
    /// frames are refused.
    pub fn push(&mut self, frames: &[f32]) -> usize {
        if self.stopping || matches!(self.state, TakeState::Stopped(_)) {
            return 0;
        }
        let ch = usize::from(self.fmt.channels);
        let align = self.fmt.block_align();
        let mut taken = 0;
        for frame in frames.chunks_exact(ch) {
            if self.memory() + align > self.limit {
                self.state = TakeState::Stopped(Why::Memory);
                break;
            }
            for &x in frame {
                if self.fmt.float {
                    self.part.extend_from_slice(&x.to_le_bytes());
                } else {
                    let v = (f64::from(x) * 8_388_608.0)
                        .round()
                        .clamp(-8_388_608.0, 8_388_607.0) as i32;
                    self.part.extend_from_slice(&v.to_le_bytes()[..3]);
                }
            }
            taken += 1;
            if self.part.len() == self.unit {
                let full = std::mem::replace(&mut self.part, Vec::with_capacity(self.unit));
                self.units.push_back(Unit {
                    bytes: full,
                    written: false,
                    failed: false,
                });
            }
        }
        self.captured += taken as u64;
        taken
    }

    fn at_risk(&mut self, why: Why) {
        if !matches!(self.state, TakeState::Stopped(_)) {
            self.state = TakeState::AtRisk(why);
        }
    }

    fn calm(&mut self) {
        if self.units.iter().any(|u| u.failed) {
            return;
        }
        if let TakeState::AtRisk(_) = self.state {
            self.state = if self.stopping {
                TakeState::Saving
            } else {
                TakeState::Recording
            };
        }
    }

    /// Write unit `i` of capture memory, up to [`TRIES`] times.
    fn write_unit(&mut self, fs: &mut dyn Fs, i: usize) -> bool {
        let at = DATA_AT + (self.first + i as u64) * self.unit as u64;
        let mut why = Why::Writes;
        for _ in 0..TRIES {
            match fs.write_at(&self.rec, at, &self.units[i].bytes) {
                Ok(()) => {
                    self.units[i].written = true;
                    self.units[i].failed = false;
                    return true;
                }
                Err(e) => {
                    self.stats.write_errors += 1;
                    why = Why::of(&e, Why::Writes);
                    if why == Why::Gone {
                        break;
                    }
                }
            }
        }
        self.units[i].failed = true;
        self.at_risk(why);
        false
    }

    /// Write every filled unit not yet written, in order. A unit that
    /// fails [`TRIES`] times is tried again next round; later units go on.
    pub fn write_ready(&mut self, fs: &mut dyn Fs) {
        for i in 0..self.units.len() {
            if !self.units[i].written
                && !self.write_unit(fs, i)
                && matches!(self.state, TakeState::AtRisk(Why::Gone))
            {
                return;
            }
        }
    }

    /// Units at the front of capture memory that are all written.
    fn written_run(&self) -> usize {
        self.units.iter().take_while(|u| u.written).count()
    }

    /// Sync the audio file; after a failed sync the pages may be gone, so
    /// every unit in capture memory counts as unwritten and the first
    /// `upto` are written again before the next try.
    fn sync_audio(&mut self, fs: &mut dyn Fs, upto: usize, header: Option<&[u8]>) -> bool {
        let mut why = Why::Syncs;
        for attempt in 0..TRIES {
            if attempt > 0 {
                let mut ok = true;
                for i in 0..upto {
                    self.stats.rewrites += 1;
                    ok &= self.write_unit(fs, i);
                }
                if let Some(h) = header {
                    if let Err(e) = fs.write_at(&self.rec, 0, h) {
                        self.stats.write_errors += 1;
                        why = Why::of(&e, Why::Writes);
                        ok = false;
                    }
                }
                if !ok {
                    continue;
                }
            }
            match fs.sync(&self.rec) {
                Ok(()) => return true,
                Err(e) => {
                    self.stats.sync_errors += 1;
                    why = Why::of(&e, Why::Syncs);
                    for u in self.units.iter_mut() {
                        u.written = false;
                    }
                }
            }
        }
        self.at_risk(why);
        false
    }

    /// Write `r` into its slot and sync the checkpoint, up to [`TRIES`]
    /// times.
    fn put_record(&mut self, fs: &mut dyn Fs, r: &Record) -> bool {
        let bytes = r.encode();
        let at = (r.slot() * SLOT) as u64;
        let mut why = Why::Records;
        for _ in 0..TRIES {
            let done = fs
                .write_at(&self.ckpt, at, &bytes)
                .and_then(|()| fs.sync(&self.ckpt));
            match done {
                Ok(()) => return true,
                Err(e) => {
                    self.stats.record_errors += 1;
                    why = Why::of(&e, Why::Records);
                }
            }
        }
        self.at_risk(why);
        false
    }

    /// The checks for committing the first `n` units of capture memory.
    fn checks(&self, n: usize, pad: usize) -> (u32, u32) {
        let mut whole = self.whole_crc;
        for (i, u) in self.units.iter().take(n).enumerate() {
            let len = u.bytes.len() - if i + 1 == self.units.len() { pad } else { 0 };
            whole = crc32c::update(whole, &u.bytes[..len]);
        }
        (whole, tail_crc(&self.tail_after(n, pad)))
    }

    /// The last committed bytes (up to 64 KiB) once the first `n` units
    /// are committed.
    fn tail_after(&self, n: usize, pad: usize) -> Vec<u8> {
        let want = TAIL_CHECK as usize;
        let mut pieces: Vec<&[u8]> = vec![&self.tail];
        for (i, u) in self.units.iter().take(n).enumerate() {
            let len = u.bytes.len() - if i + 1 == self.units.len() { pad } else { 0 };
            pieces.push(&u.bytes[..len]);
        }
        let mut out = Vec::with_capacity(want);
        let mut need = want;
        for p in pieces.iter().rev() {
            if need == 0 {
                break;
            }
            let k = p.len().min(need);
            out.splice(0..0, p[p.len() - k..].iter().copied());
            need -= k;
        }
        out
    }

    /// Commit the first `n` units: they leave capture memory.
    fn commit(&mut self, n: usize, frames: u64, seq: u64, whole: u32, pad: usize) {
        self.tail = self.tail_after(n, pad);
        self.whole_crc = whole;
        self.committed = frames;
        self.seq = seq;
        for _ in 0..n {
            self.units.pop_front();
        }
        self.first += n as u64;
        self.stats.rounds += 1;
    }

    /// One checkpoint round (C1 to C5): commit the longest run of written
    /// units from the start. Does nothing if nothing new is written.
    pub fn checkpoint(&mut self, fs: &mut dyn Fs) {
        if self.stopping {
            return;
        }
        self.write_ready(fs);
        // C1
        let n = self.written_run();
        if n == 0 {
            return;
        }
        let frames = (self.first + n as u64) * self.unit as u64 / self.fmt.block_align() as u64;
        // C2
        if !self.sync_audio(fs, n, None) {
            return;
        }
        // C3, C4
        let (whole, tail) = self.checks(n, 0);
        let mut r = self.record(self.seq + 1, frames, false, tail);
        r.whole_crc = whole;
        if !self.put_record(fs, &r) {
            return;
        }
        // C5
        self.commit(n, frames, r.seq, whole, 0);
        self.calm();
    }

    /// The musician stopped: no more frames; [`Take::finish`] makes the
    /// file.
    pub fn stop(&mut self) {
        if self.stopping {
            return;
        }
        self.stopping = true;
        if self.state == TakeState::Recording {
            self.state = TakeState::Saving;
        }
        let data = self.captured * self.fmt.block_align() as u64;
        if !self.part.is_empty() {
            let mut last = std::mem::take(&mut self.part);
            if data & 1 == 1 {
                last.push(0);
            }
            self.units.push_back(Unit {
                bytes: last,
                written: false,
                failed: false,
            });
        }
    }

    /// Finish a stopped take (P1 to P5); true once it is Saved. Call again
    /// after a failure: it carries on from the step that failed.
    pub fn finish(&mut self, fs: &mut dyn Fs) -> bool {
        if !self.stopping {
            return false;
        }
        let fin = final_path(&self.dir, &self.stem);
        let data = self.captured * self.fmt.block_align() as u64;
        let pad = (data & 1) as usize;
        if self.stop_step == 0 {
            // P1: the last unit and the true header, synced.
            self.write_ready(fs);
            if self.written_run() < self.units.len() {
                return false;
            }
            let h = header(self.fmt, &self.id, Sizes::done(self.fmt, self.captured));
            if let Err(e) = fs.write_at(&self.rec, 0, &h) {
                self.stats.write_errors += 1;
                self.at_risk(Why::of(&e, Why::Writes));
                return false;
            }
            let all = self.units.len();
            if !self.sync_audio(fs, all, Some(&h)) {
                return false;
            }
            self.stop_step = 1;
        }
        if self.stop_step == 1 {
            // P2: the final record.
            let n = self.units.len();
            let (whole, tail) = if n == 0 {
                (self.whole_crc, tail_crc(&self.tail))
            } else {
                self.checks(n, pad)
            };
            let mut r = self.record(self.seq + 1, self.captured, true, tail);
            r.whole_crc = whole;
            if !self.put_record(fs, &r) {
                return false;
            }
            let frames = self.captured;
            self.commit(n, frames, r.seq, whole, pad);
            self.stop_step = 2;
        }
        if self.stop_step == 2 {
            // P3
            if let Err(e) = fs.rename(&self.rec, &fin) {
                self.at_risk(Why::of(&e, Why::Finish));
                return false;
            }
            self.stop_step = 3;
        }
        if self.stop_step == 3 {
            // P4
            if let Err(e) = fs.sync_dir(&self.dir) {
                self.at_risk(Why::of(&e, Why::Finish));
                return false;
            }
            self.stop_step = 4;
            self.state = TakeState::Saved;
            self.units.clear();
            self.tail = Vec::new();
            // P5
            if fs.remove(&self.ckpt).is_err() {
                self.stats.leftovers += 1;
            }
        }
        true
    }

    /// Give up an armed take that never recorded: its files go.
    pub fn disarm(self, fs: &mut dyn Fs) -> bool {
        if self.captured > 0 {
            return false;
        }
        let a = fs.remove(&self.rec).is_ok();
        let b = fs.remove(&self.ckpt).is_ok();
        a && b
    }
}
