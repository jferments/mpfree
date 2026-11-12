//! mpfree-audiotracks: audio tracks on the song timeline.
//!
//! Up to [`MAX_TRACKS`] audio tracks that record the line input in a line
//! along the song, the way a tape machine or a DAW does, next to the MIDI
//! tracks of the arrangement:
//!
//! - **Record.** An armed track records while the transport records
//!   (Record from stopped after the count-in, or a punch in while
//!   playing), until the punch out, a stop, or the track is disarmed.
//!   Each recording is a **take**: the audio, and the song tick its first
//!   frame belongs to (its `anchor`).
//! - **Regions.** What a track plays is its list of [`Region`]s: stretches
//!   of song ticks, each playing a stretch of one take. A new recording
//!   puts its region over whatever was there; the older audio is not lost,
//!   only covered (the take is still there to comp from).
//! - **Loop recording.** While the transport loops, each pass is a take of
//!   its own (one `group`, numbered by `pass`); the newest pass plays.
//! - **Comping.** [`Command::Comp`] plays a stretch of any take of the
//!   track in place of what was there (a swipe across a take lane). Split,
//!   move, erase, gain and fades edit regions. Undo and Redo walk one
//!   history of every edit and recording, for all tracks.
//! - **No clicks.** Every region fades in over [`FADE`] frames from its
//!   start and plays on [`FADE`] frames past its end fading out (equal
//!   power), so two regions that meet crossfade there: a punch in or out,
//!   or a comp boundary, is seamless when the takes are of the same part.
//! - **Time.** A region sits on song ticks, so it stays on its bar when the
//!   tempo before it changes. Its audio plays at the rate it was recorded
//!   at, from the region's start, as a DAW plays audio that is not warped:
//!   at another tempo it does not stretch (time-stretch is a later step,
//!   with `mpfree-stretch`).
//! - **Latency.** Input that arrives `latency` frames after a moment
//!   belongs to that moment ([`Command::SetLatency`]): takes are placed
//!   that much earlier, and a recording keeps writing that long after its
//!   punch out (and [`FADE`] frames more, for the crossfade past its end).
//! - **Monitoring.** [`Monitor::Auto`] (the default) hears the input on an
//!   armed track while stopped or recording, and the track's regions
//!   otherwise; [`Monitor::In`] always hears the input as well;
//!   [`Monitor::Off`] never does.
//! - **One memory.** Takes share one pool of samples, asked for once in
//!   [`AudioTracks::new`]. A recording writes into the largest free room;
//!   when two record at once, a new one takes half of the room another is
//!   still filling, so several tracks can record together with no
//!   planning. A take holds up to [`MAX_EXTENTS`] stretches of the pool;
//!   nothing is ever moved. [`Command::Forget`] gives a take back.
//!
//! [`AudioTracks::command`] and [`AudioTracks::process`] never allocate,
//! lock or panic, and give the same output for any period size.

#![forbid(unsafe_code)]

pub mod save;
pub mod stretch;

use save::{Saved, TakeFile, TrackFile};

use mpfree_transport::time::{TempoMap, Tick, PPQ};
use mpfree_transport::transport::MAX_PERIOD;
use mpfree_transport::{Block, Mark, Span, State};

/// Audio tracks at most.
pub const MAX_TRACKS: usize = 8;
/// Takes at most, for all tracks together.
pub const MAX_TAKES: usize = 256;
/// Regions on one track at most.
pub const MAX_REGIONS: usize = 256;
/// Stretches of the pool one take can hold. A recording that would need
/// more stops there.
pub const MAX_EXTENTS: usize = 32;
/// Frames of the crossfade at each region's start and past its end (about
/// 1.5 ms at 44.1 kHz).
pub const FADE: usize = 64;
/// A loaded take's audio copies in at this many frames for each frame
/// that plays (a minute of audio in about a second).
pub const LOAD_SPEED: usize = 64;
/// Edits and recordings Undo can take back.
pub const HISTORY: usize = 32;
/// Commands waiting for the next period.
const QUEUE: usize = 64;
/// The most frames the tracks run in one go.
const RUN: usize = 128;
/// The smallest room worth recording into, in samples (even).
const MIN_ROOM: usize = 4096;
/// Samples in one block of the peak table: the waveform overview keeps the
/// lowest and highest sample of every block of the pool. Every stretch of
/// the pool starts and ends on a block's edge, so a block never holds two
/// takes.
pub const PEAK_BLOCK: usize = 256;

fn round_up(n: usize) -> usize {
    n.div_ceil(PEAK_BLOCK) * PEAK_BLOCK
}

fn min_max(xs: &[f32]) -> (f32, f32) {
    xs.iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), &v| (lo.min(v), hi.max(v)))
}

/// What a track records from the line input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// The left jack, as a mono take (the MPC One's line input records
    /// on the left jack: round 7).
    Left,
    Right,
    /// Both jacks, as a stereo take.
    Stereo,
    /// Both jacks summed to a mono take.
    Sum,
}

impl Input {
    fn channels(self) -> usize {
        match self {
            Input::Stereo => 2,
            _ => 1,
        }
    }
}

/// When a track hears its input; see the module notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Monitor {
    Off,
    Auto,
    In,
}

/// What the screen, the buttons and the host ask for. Commands apply in
/// order at the start of the next period. A command naming a track past
/// the last, a region that is not there, or an edit that does not fit is
/// counted in [`Stats::refused`] and changes nothing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Arm a track to record (or disarm it: a recording ends there).
    Arm(usize, bool),
    /// What the track's next recording takes.
    SetInput(usize, Input),
    SetMonitor(usize, Monitor),
    /// The track's level, 0 to 2 (1 is as recorded).
    SetLevel(usize, f32),
    /// The track's balance, -1 (left) to 1 (right).
    SetPan(usize, f32),
    Mute(usize, bool),
    Solo(usize, bool),
    /// Input latency to make up for, in frames, for every track.
    SetLatency(u32),
    /// Whether the tracks play and record (the host turns them off when
    /// the song is not playing, say in clip launch mode). Monitoring goes
    /// on either way.
    SetActive(bool),
    /// Play take `take` on its track from tick `from` to `to`, over what
    /// was there (clipped to where the take has audio).
    Comp {
        take: u16,
        from: Tick,
        to: Tick,
    },
    /// Cut the region of `track` playing at tick `at` in two there.
    Split {
        track: usize,
        at: Tick,
    },
    /// Silence `track` from `from` to `to`.
    Erase {
        track: usize,
        from: Tick,
        to: Tick,
    },
    /// Move the region of `track` playing at `at` to start at `to`, over
    /// what is there (its audio moves with it).
    Move {
        track: usize,
        at: Tick,
        to: Tick,
    },
    /// The level of the region of `track` playing at `at`, 0 to 4.
    SetGain {
        track: usize,
        at: Tick,
        gain: f32,
    },
    /// Fades of the region of `track` playing at `at`, in frames from its
    /// start and to its end (each at most half the region; on top of the
    /// crossfades every region has).
    SetFades {
        track: usize,
        at: Tick,
        fade_in: u32,
        fade_out: u32,
    },
    Undo,
    Redo,
    /// Give a take's memory back. Refused while it records or while a
    /// region, or a step Undo or Redo could bring back, plays it (see
    /// [`AudioTracks::in_use`]); [`Command::ClearHistory`] first.
    Forget(u16),
    /// Forget the undo history (so takes only it used can be forgotten).
    ClearHistory,
    /// Play take `new` wherever take `old` plays, its frames `ratio` times
    /// as many (a stretched copy: see [`stretch`]); the regions keep their
    /// place on the song. One undo step. Refused unless both takes are of
    /// one track and `new` has copied in.
    Swap {
        old: u16,
        new: u16,
        ratio: f64,
    },
}

/// A stretch of song ticks playing a stretch of a take: tick `start`
/// plays the take's frame `src`, and the frames after it follow at the
/// recorded rate up to tick `end`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Region {
    pub start: Tick,
    pub end: Tick,
    pub take: u16,
    /// The take's frame that plays at `start` (may be below 0 or past the
    /// take's end: those frames are silent).
    pub src: i64,
    pub gain: f32,
    pub fade_in: u32,
    pub fade_out: u32,
}

impl Region {
    const EMPTY: Region = Region {
        start: 0,
        end: 0,
        take: 0,
        src: 0,
        gain: 1.0,
        fade_in: 0,
        fade_out: 0,
    };

    pub fn contains(&self, t: Tick) -> bool {
        (self.start..self.end).contains(&t)
    }
}

/// What the screen shows of a take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TakeInfo {
    pub id: u16,
    pub track: usize,
    /// Recordings of one press of Record share a group; each pass of a
    /// loop recording is a take of its own, numbered from 0.
    pub group: u16,
    pub pass: u16,
    /// The tempo it was recorded at (0: not known).
    pub bpm: f64,
    /// The song tick (with its fraction) its frame 0 belongs to.
    pub anchor: f64,
    pub frames: usize,
    pub channels: usize,
    pub recording: bool,
    /// The recording stopped early because the memory was full or the take
    /// held [`MAX_EXTENTS`] stretches.
    pub cut_short: bool,
}

/// Counters for the screen and the tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Commands that changed nothing (see [`Command`]).
    pub refused: u32,
    /// Commands dropped because the queue was full.
    pub dropped: u32,
    /// Recordings cut short by a full memory or [`MAX_TAKES`].
    pub full: u32,
}

/// The memory gauge, in samples (a stereo frame is two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub used: usize,
    pub capacity: usize,
}

/// A stretch of the pool, in samples; start and length are whole
/// [`PEAK_BLOCK`]s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ext {
    start: usize,
    len: usize,
}

const NO_EXT: Ext = Ext { start: 0, len: 0 };

/// One memory for every take: free stretches sorted by start, merged when
/// they meet, all on [`PEAK_BLOCK`] edges. The list is made big enough once
/// that it never grows. `peaks` holds each block's lowest and highest
/// sample, kept as the audio is written.
struct Pool {
    data: Vec<f32>,
    free: Vec<Ext>,
    used: usize,
    peaks: Vec<(f32, f32)>,
}

impl Pool {
    fn new(samples: usize) -> Pool {
        let samples = samples / PEAK_BLOCK * PEAK_BLOCK;
        let mut free = Vec::with_capacity(MAX_TAKES * MAX_EXTENTS + MAX_TRACKS + 2);
        if samples > 0 {
            free.push(Ext {
                start: 0,
                len: samples,
            });
        }
        let mut data = vec![0.0; samples];
        // Touch every page now, so the audio thread never waits for the
        // system to map one in the middle of a recording (a zeroed
        // allocation is mapped lazily).
        for i in (0..data.len()).step_by(1024) {
            data[i] = std::hint::black_box(0.0);
        }
        Pool {
            data,
            free,
            used: 0,
            peaks: vec![(0.0, 0.0); samples / PEAK_BLOCK],
        }
    }

    /// Samples `start..start + len` were just written: their blocks' peaks
    /// take them in. A take writes each stretch in order from its start,
    /// so a write that begins on a block's edge is that block's first and
    /// replaces what an earlier take left there.
    fn note(&mut self, start: usize, len: usize) {
        let mut p = start;
        let end = start + len;
        while p < end {
            let b = p / PEAK_BLOCK;
            let to = end.min((b + 1) * PEAK_BLOCK);
            let (lo, hi) = min_max(&self.data[p..to]);
            let pk = &mut self.peaks[b];
            *pk = if p.is_multiple_of(PEAK_BLOCK) {
                (lo, hi)
            } else {
                (pk.0.min(lo), pk.1.max(hi))
            };
            p = to;
        }
    }

    /// The lowest and highest sample of `start..start + len`, from the
    /// table for whole blocks.
    fn range_peaks(&self, start: usize, len: usize) -> (f32, f32) {
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut p = start;
        let end = start + len;
        while p < end {
            let b = p / PEAK_BLOCK;
            let to = end.min((b + 1) * PEAK_BLOCK);
            let (l, h) = if p.is_multiple_of(PEAK_BLOCK) && to == (b + 1) * PEAK_BLOCK {
                self.peaks[b]
            } else {
                min_max(&self.data[p..to])
            };
            lo = lo.min(l);
            hi = hi.max(h);
            p = to;
        }
        (lo, hi)
    }

    fn largest(&self) -> Option<usize> {
        let mut best = None;
        let mut len = 0;
        for (i, e) in self.free.iter().enumerate() {
            if e.len > len {
                len = e.len;
                best = Some(i);
            }
        }
        best
    }

    fn take(&mut self, i: usize) -> Ext {
        let e = self.free.remove(i);
        self.used += e.len;
        e
    }

    fn give(&mut self, e: Ext) {
        if e.len == 0 {
            return;
        }
        self.used -= e.len;
        let i = self.free.partition_point(|f| f.start < e.start);
        let joins_prev = i > 0 && {
            let p = self.free[i - 1];
            p.start + p.len == e.start
        };
        let joins_next = i < self.free.len() && e.start + e.len == self.free[i].start;
        match (joins_prev, joins_next) {
            (true, true) => {
                let n = self.free.remove(i);
                self.free[i - 1].len += e.len + n.len;
            }
            (true, false) => self.free[i - 1].len += e.len,
            (false, true) => {
                self.free[i].start = e.start;
                self.free[i].len += e.len;
            }
            (false, false) => {
                if self.free.len() < self.free.capacity() {
                    self.free.insert(i, e);
                } else {
                    // Cannot happen: the list holds one more than every
                    // stretch a take can own. Leak rather than allocate.
                    self.used += e.len;
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Take {
    live: bool,
    track: u8,
    channels: u8,
    group: u16,
    pass: u16,
    bpm: f64,
    anchor: f64,
    /// Frames written.
    frames: usize,
    ext: [Ext; MAX_EXTENTS],
    n_ext: usize,
    /// Samples written into the last stretch.
    wpos: usize,
    recording: bool,
    cut_short: bool,
    /// Its saved audio is still copying in: it plays nothing yet.
    loading: bool,
}

const NO_TAKE: Take = Take {
    live: false,
    track: 0,
    channels: 1,
    group: 0,
    bpm: 0.0,
    pass: 0,
    anchor: 0.0,
    frames: 0,
    ext: [NO_EXT; MAX_EXTENTS],
    n_ext: 0,
    wpos: 0,
    recording: false,
    cut_short: false,
    loading: false,
};

impl Take {
    /// Adds `gain`-weighted frames `k..k + n` of the take into the
    /// interleaved stereo `out` (n frames); frames the take does not have
    /// add nothing. `gain(j)` is the gain of frame `j` of the run.
    fn add_into(&self, data: &[f32], k: i64, out: &mut [f32], gain: &impl Fn(usize) -> f32) {
        let n = out.len() / 2;
        let ch = self.channels as usize;
        let mut j = 0usize;
        // Skip frames before the take.
        if k < 0 {
            j = ((-k) as usize).min(n);
        }
        let mut base = 0usize; // first frame of stretch e
        for e in &self.ext[..self.n_ext] {
            if j >= n {
                break;
            }
            let frames_in = e.len / ch;
            let have = if base + frames_in > self.frames {
                self.frames.saturating_sub(base)
            } else {
                frames_in
            };
            let at = k + j as i64; // take frame
            if at as usize >= base + have {
                base += frames_in;
                continue;
            }
            let from = at as usize - base;
            let count = (have - from).min(n - j);
            let src = &data[e.start + from * ch..e.start + (from + count) * ch];
            let dst = &mut out[2 * j..2 * (j + count)];
            if ch == 1 {
                for (i, (d, s)) in dst.chunks_exact_mut(2).zip(src).enumerate() {
                    let g = gain(j + i) * *s;
                    d[0] += g;
                    d[1] += g;
                }
            } else {
                for (i, (d, s)) in dst.chunks_exact_mut(2).zip(src.chunks_exact(2)).enumerate() {
                    let g = gain(j + i);
                    d[0] += g * s[0];
                    d[1] += g * s[1];
                }
            }
            j += count;
            base += frames_in;
        }
    }

    fn sample(&self, data: &[f32], k: usize, c: usize) -> Option<f32> {
        if k >= self.frames {
            return None;
        }
        let ch = self.channels as usize;
        let mut base = 0;
        for e in &self.ext[..self.n_ext] {
            let frames_in = e.len / ch;
            if k < base + frames_in {
                return Some(data[e.start + (k - base) * ch + c.min(ch - 1)]);
            }
            base += frames_in;
        }
        None
    }
}

/// A recording in progress: input frame `g` is the take's frame `g - g0`.
#[derive(Clone, Copy, Debug)]
struct Rec {
    take: u16,
    g0: u64,
    /// Song tick at `g0`.
    t_in: f64,
}

/// A recording past its punch out, still writing the input that belongs
/// before it (the latency), until frame `stop_at`.
#[derive(Clone, Copy, Debug)]
struct Tail {
    take: u16,
    stop_at: u64,
}

struct Track {
    regions: [Region; MAX_REGIONS],
    n: usize,
    armed: bool,
    input: Input,
    monitor: Monitor,
    level: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    rec: Option<Rec>,
    tail: Option<Tail>,
    /// The memory ran out: no new take until the transport stops
    /// recording.
    blocked: bool,
    /// Gains (left, right) at the end of the last period, for the ramp.
    gains: [f32; 2],
    buf: Vec<f32>,
}

impl Track {
    fn new() -> Track {
        Track {
            regions: [Region::EMPTY; MAX_REGIONS],
            n: 0,
            armed: false,
            input: Input::Left,
            monitor: Monitor::Auto,
            level: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            rec: None,
            tail: None,
            blocked: false,
            gains: [1.0, 1.0],
            buf: vec![0.0; 2 * MAX_PERIOD as usize],
        }
    }

    fn regions(&self) -> &[Region] {
        &self.regions[..self.n]
    }

    fn at(&self, t: Tick) -> Option<usize> {
        self.regions().iter().position(|r| r.contains(t))
    }

    /// Regions the clear of `from..to` would leave, or None if they would
    /// not fit (with `extra` more to come).
    fn clear_fits(&self, from: Tick, to: Tick, extra: usize) -> bool {
        let splits = self
            .regions()
            .iter()
            .filter(|r| r.start < from && r.end > to)
            .count();
        self.n + splits + extra <= MAX_REGIONS
    }

    /// Take `from..to` out of the regions: ones inside go, ones across an
    /// edge are cut there (a cut start moves `src` on), and one across the
    /// whole stretch becomes two. Checked by `clear_fits` first.
    fn clear(&mut self, map: &TempoMap, rate: u32, from: Tick, to: Tick) {
        if to <= from {
            return;
        }
        let mut i = 0;
        while i < self.n {
            let r = self.regions[i];
            if r.end <= from || r.start >= to {
                i += 1;
                continue;
            }
            if r.start >= from && r.end <= to {
                self.regions.copy_within(i + 1..self.n, i);
                self.n -= 1;
                continue;
            }
            if r.start < from && r.end > to {
                let mut right = r;
                right.start = to;
                right.src =
                    r.src + frames_from(map, r.start as f64, to as f64, rate).round() as i64;
                self.regions[i].end = from;
                self.regions.copy_within(i + 1..self.n, i + 2);
                self.regions[i + 1] = right;
                self.n += 1;
                i += 2;
                continue;
            }
            if r.start < from {
                self.regions[i].end = from;
            } else {
                self.regions[i].start = to;
                self.regions[i].src =
                    r.src + frames_from(map, r.start as f64, to as f64, rate).round() as i64;
            }
            i += 1;
        }
    }

    /// Put `r` over whatever is in its stretch. False (nothing changed) if
    /// it would not fit.
    fn put(&mut self, map: &TempoMap, rate: u32, r: Region) -> bool {
        if r.end <= r.start || !self.clear_fits(r.start, r.end, 1) {
            return false;
        }
        self.clear(map, rate, r.start, r.end);
        let i = self.regions().partition_point(|x| x.start < r.start);
        self.regions.copy_within(i..self.n, i + 1);
        self.regions[i] = r;
        self.n += 1;
        true
    }
}

/// One step of the history: a track's regions before (or, once undone,
/// after) an edit.
struct Snap {
    track: usize,
    n: usize,
    regions: [Region; MAX_REGIONS],
}

/// Frames from tick `a` to tick `b` (fractional) along the tempo map.
pub fn frames_from(map: &TempoMap, a: f64, b: f64, rate: u32) -> f64 {
    if b < a {
        return -frames_from(map, b, a, rate);
    }
    let segs = map.segments();
    let per_tick = |bpm: f64| 60.0 * f64::from(rate) / (bpm * f64::from(PPQ));
    let mut f = 0.0;
    let mut t = a;
    for (i, s) in segs.iter().enumerate() {
        let seg_end = segs.get(i + 1).map_or(f64::INFINITY, |n| n.start as f64);
        if seg_end <= t {
            continue;
        }
        let e = seg_end.min(b);
        if e > t {
            f += (e - t) * per_tick(s.bpm);
            t = e;
        }
        if t >= b {
            break;
        }
    }
    f
}

/// The tick (fractional) `frames` frames after tick `a` along the tempo map.
pub fn tick_after(map: &TempoMap, a: f64, frames: f64, rate: u32) -> f64 {
    let segs = map.segments();
    let per_tick = |bpm: f64| 60.0 * f64::from(rate) / (bpm * f64::from(PPQ));
    let mut left = frames;
    let mut t = a;
    for (i, s) in segs.iter().enumerate() {
        let seg_end = segs.get(i + 1).map_or(f64::INFINITY, |n| n.start as f64);
        if seg_end <= t {
            continue;
        }
        let room = (seg_end - t) * per_tick(s.bpm);
        if left <= room {
            return t + left / per_tick(s.bpm);
        }
        left -= room;
        t = seg_end;
    }
    t
}

/// Equal-power fade-in over [`FADE`] frames: `xfade_in(j)` for frame `j`
/// of the fade; the matching fade-out is `xfade_in(FADE - j)`.
fn fade_table() -> [f32; FADE + 1] {
    let mut t = [0.0; FADE + 1];
    for (j, v) in t.iter_mut().enumerate() {
        *v = ((j as f64 / FADE as f64) * std::f64::consts::FRAC_PI_2).sin() as f32;
    }
    t
}

/// The audio tracks: see the module notes.
pub struct AudioTracks {
    rate: u32,
    pool: Pool,
    takes: Vec<Take>,
    tracks: Vec<Track>,
    queue: [Option<Command>; QUEUE],
    q_len: usize,
    latency: u32,
    active: bool,
    groups: u16,
    hist: Vec<Snap>,
    /// The track's regions before the edit in progress.
    scratch: Box<Snap>,
    /// A loop jump ended these tracks' takes: the next pass (group, pass)
    /// starts where the song lands.
    pending: [Option<(u16, u16)>; MAX_TRACKS],
    h_base: usize,
    h_done: usize,
    h_redo: usize,
    stats: Stats,
    /// Takes whose saved audio is copying in (room for all, made once).
    loads: Vec<(u16, Vec<f32>)>,
    /// Loaded takes' audio, done with, for the host to drop off the audio
    /// thread.
    returned: Vec<Vec<f32>>,
    fade: [f32; FADE + 1],
}

impl AudioTracks {
    /// Audio tracks at `rate` with memory for `seconds` of stereo audio
    /// (twice that of mono takes), all allocated here.
    pub fn new(rate: u32, seconds: u32) -> AudioTracks {
        let samples = rate as usize * seconds as usize * 2;
        let mut hist = Vec::with_capacity(HISTORY);
        for _ in 0..HISTORY {
            hist.push(Snap {
                track: 0,
                n: 0,
                regions: [Region::EMPTY; MAX_REGIONS],
            });
        }
        AudioTracks {
            rate,
            pool: Pool::new(samples),
            takes: vec![NO_TAKE; MAX_TAKES],
            tracks: (0..MAX_TRACKS).map(|_| Track::new()).collect(),
            queue: [None; QUEUE],
            q_len: 0,
            latency: 0,
            active: true,
            groups: 0,
            hist,
            scratch: Box::new(Snap {
                track: 0,
                n: 0,
                regions: [Region::EMPTY; MAX_REGIONS],
            }),
            pending: [None; MAX_TRACKS],
            h_base: 0,
            h_done: 0,
            h_redo: 0,
            stats: Stats::default(),
            loads: Vec::with_capacity(MAX_TAKES),
            returned: Vec::with_capacity(MAX_TAKES),
            fade: fade_table(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// Queue a command for the next period. False if the queue is full.
    pub fn command(&mut self, c: Command) -> bool {
        if self.q_len == QUEUE {
            self.stats.dropped += 1;
            return false;
        }
        self.queue[self.q_len] = Some(c);
        self.q_len += 1;
        true
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn memory(&self) -> Memory {
        Memory {
            used: self.pool.used,
            capacity: self.pool.data.len(),
        }
    }

    pub fn latency(&self) -> u32 {
        self.latency
    }

    pub fn active(&self) -> bool {
        self.active
    }

    /// The regions of a track, in order (empty past the last track).
    pub fn regions(&self, track: usize) -> &[Region] {
        self.tracks.get(track).map_or(&[], Track::regions)
    }

    pub fn armed(&self, track: usize) -> bool {
        self.tracks.get(track).is_some_and(|t| t.armed)
    }

    /// The take a track is recording, if any.
    pub fn recording(&self, track: usize) -> Option<u16> {
        self.tracks.get(track)?.rec.map(|r| r.take)
    }

    pub fn take(&self, id: u16) -> Option<TakeInfo> {
        let t = self.takes.get(id as usize).filter(|t| t.live)?;
        Some(TakeInfo {
            id,
            track: t.track as usize,
            group: t.group,
            pass: t.pass,
            bpm: t.bpm,
            anchor: t.anchor,
            frames: t.frames,
            channels: t.channels as usize,
            recording: t.recording,
            cut_short: t.cut_short,
        })
    }

    /// An id no take has, for a take made off the audio thread (a stretch)
    /// to load under.
    pub fn free_take_id(&self) -> Option<u16> {
        self.takes.iter().position(|t| !t.live).map(|i| i as u16)
    }

    /// Every take, oldest id first.
    pub fn takes(&self) -> impl Iterator<Item = TakeInfo> + '_ {
        (0..MAX_TAKES as u16).filter_map(|id| self.take(id))
    }

    /// Frame `k`, channel `c` of a take (a mono take answers for both).
    pub fn sample(&self, id: u16, k: usize, c: usize) -> Option<f32> {
        let t = self.takes.get(id as usize).filter(|t| t.live)?;
        t.sample(&self.pool.data, k, c)
    }

    /// The lowest and highest sample of frames `from..to` of a take, over
    /// its channels: for a waveform on the screen. It reads the peak table
    /// for whole blocks of [`PEAK_BLOCK`] samples, so a long stretch costs
    /// little; it works while the take records (up to what is written) and
    /// does not allocate. A host shares the result with the screen thread
    /// as it shares the rest of the state.
    pub fn peaks(&self, id: u16, from: usize, to: usize) -> Option<(f32, f32)> {
        let t = self.takes.get(id as usize).filter(|t| t.live)?;
        let to = to.min(t.frames);
        if from >= to {
            return None;
        }
        let ch = t.channels as usize;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut base = 0;
        for e in &t.ext[..t.n_ext] {
            let frames_in = e.len / ch;
            let a = from.max(base);
            let b = to.min(base + frames_in);
            if a < b {
                let (l, h) = self
                    .pool
                    .range_peaks(e.start + (a - base) * ch, (b - a) * ch);
                lo = lo.min(l);
                hi = hi.max(h);
            }
            base += frames_in;
            if base >= to {
                break;
            }
        }
        Some((lo, hi))
    }

    /// Steps Undo can take back and Redo can bring back.
    pub fn history(&self) -> (usize, usize) {
        (self.h_done, self.h_redo)
    }

    /// Whether a region, or a step of the history, plays this take.
    pub fn in_use(&self, id: u16) -> bool {
        let uses = |rs: &[Region]| rs.iter().any(|r| r.take == id);
        self.tracks.iter().any(|t| uses(t.regions()))
            || (0..self.h_done + self.h_redo).any(|i| {
                let s = &self.hist[(self.h_base + i) % HISTORY];
                uses(&s.regions[..s.n])
            })
    }

    /// A track's sound in the last period: interleaved stereo, after its
    /// level, pan, mute and solo (for the host's mixer strip).
    pub fn track_out(&self, track: usize, frames: usize) -> &[f32] {
        self.tracks
            .get(track)
            .map_or(&[], |t| &t.buf[..2 * frames.min(MAX_PERIOD as usize)])
    }

    /// The take as a 32-bit float WAV (for the store). Allocates: not on
    /// the audio thread.
    pub fn take_wav(&self, id: u16) -> Option<Vec<u8>> {
        let t = self.takes.get(id as usize).filter(|t| t.live)?;
        let ch = t.channels as usize;
        let bytes = t.frames * ch * 4;
        let mut w = Vec::with_capacity(44 + bytes);
        let u32le = |w: &mut Vec<u8>, v: u32| w.extend_from_slice(&v.to_le_bytes());
        let u16le = |w: &mut Vec<u8>, v: u16| w.extend_from_slice(&v.to_le_bytes());
        w.extend_from_slice(b"RIFF");
        u32le(&mut w, 36 + bytes as u32);
        w.extend_from_slice(b"WAVEfmt ");
        u32le(&mut w, 16);
        u16le(&mut w, 3);
        u16le(&mut w, ch as u16);
        u32le(&mut w, self.rate);
        u32le(&mut w, self.rate * ch as u32 * 4);
        u16le(&mut w, ch as u16 * 4);
        u16le(&mut w, 32);
        w.extend_from_slice(b"data");
        u32le(&mut w, bytes as u32);
        for k in 0..t.frames {
            for c in 0..ch {
                let s = t.sample(&self.pool.data, k, c).unwrap_or(0.0);
                w.extend_from_slice(&s.to_le_bytes());
            }
        }
        Some(w)
    }

    // ---- commands ------------------------------------------------------

    fn refuse(&mut self) {
        self.stats.refused += 1;
    }

    /// Save the scratch copy (a track's regions before an edit) as an undo
    /// step, dropping the oldest when full and anything Redo could bring
    /// back.
    fn push_history(&mut self) {
        let idx = (self.h_base + self.h_done) % HISTORY;
        let (src, dst) = (&*self.scratch, &mut self.hist[idx]);
        dst.track = src.track;
        dst.n = src.n;
        dst.regions[..src.n].copy_from_slice(&src.regions[..src.n]);
        if self.h_done == HISTORY {
            self.h_base = (self.h_base + 1) % HISTORY;
        } else {
            self.h_done += 1;
        }
        self.h_redo = 0;
    }

    fn swap_history(&mut self, idx: usize) {
        let s = &mut self.hist[idx];
        let t = &mut self.tracks[s.track];
        let n = s.n.max(t.n);
        for i in 0..n {
            std::mem::swap(&mut s.regions[i], &mut t.regions[i]);
        }
        std::mem::swap(&mut s.n, &mut t.n);
    }

    /// An edit of one track's regions: `f` changes them and says whether it
    /// did; if so, the regions before it become an undo step, and if not,
    /// they are put back as they were.
    fn edit(
        &mut self,
        track: usize,
        f: impl FnOnce(&mut Track, &TempoMap, u32) -> bool,
        map: &TempoMap,
    ) {
        if track >= MAX_TRACKS {
            return self.refuse();
        }
        {
            let (t, s) = (&self.tracks[track], &mut *self.scratch);
            s.track = track;
            s.n = t.n;
            s.regions[..t.n].copy_from_slice(t.regions());
        }
        let rate = self.rate;
        if f(&mut self.tracks[track], map, rate) {
            self.push_history();
        } else {
            let (t, s) = (&mut self.tracks[track], &*self.scratch);
            t.n = s.n;
            t.regions[..s.n].copy_from_slice(&s.regions[..s.n]);
            self.refuse();
        }
    }

    fn apply(&mut self, c: Command, map: &TempoMap, now: u64) {
        let ok_track = |t: usize| t < MAX_TRACKS;
        match c {
            Command::Arm(t, on) if ok_track(t) => {
                self.tracks[t].armed = on;
                if !on {
                    self.punch_out(t, map, now, None);
                }
            }
            Command::SetInput(t, i) if ok_track(t) => self.tracks[t].input = i,
            Command::SetMonitor(t, m) if ok_track(t) => self.tracks[t].monitor = m,
            Command::SetLevel(t, l) if ok_track(t) && l.is_finite() => {
                self.tracks[t].level = l.clamp(0.0, 2.0)
            }
            Command::SetPan(t, p) if ok_track(t) && p.is_finite() => {
                self.tracks[t].pan = p.clamp(-1.0, 1.0)
            }
            Command::Mute(t, m) if ok_track(t) => self.tracks[t].mute = m,
            Command::Solo(t, s) if ok_track(t) => self.tracks[t].solo = s,
            Command::SetLatency(l) => self.latency = l.min(MAX_PERIOD * 4),
            Command::SetActive(a) => {
                self.active = a;
                if !a {
                    for t in 0..MAX_TRACKS {
                        self.punch_out(t, map, now, None);
                    }
                }
            }
            Command::Comp { take, from, to } => {
                let Some(tk) = self.takes.get(take as usize).filter(|t| t.live).copied() else {
                    return self.refuse();
                };
                let rate = self.rate;
                let a = tk.anchor;
                let b = tick_after(map, a, tk.frames as f64, rate);
                let from = from.max(a.ceil() as Tick);
                let to = to.min(b.floor() as Tick);
                if to <= from {
                    return self.refuse();
                }
                let r = Region {
                    start: from,
                    end: to,
                    take,
                    src: frames_from(map, a, from as f64, rate).round() as i64,
                    gain: 1.0,
                    fade_in: 0,
                    fade_out: 0,
                };
                self.edit(tk.track as usize, |t, m, rate| t.put(m, rate, r), map);
            }
            Command::Split { track, at } if ok_track(track) => {
                let Some(i) = self.tracks[track]
                    .at(at)
                    .filter(|&i| self.tracks[track].regions[i].start < at)
                else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, m, rate| {
                        if t.n == MAX_REGIONS {
                            return false;
                        }
                        let r = t.regions[i];
                        let mut right = r;
                        right.start = at;
                        right.src =
                            r.src + frames_from(m, r.start as f64, at as f64, rate).round() as i64;
                        right.fade_in = 0;
                        t.regions[i].end = at;
                        t.regions[i].fade_out = 0;
                        t.regions.copy_within(i + 1..t.n, i + 2);
                        t.regions[i + 1] = right;
                        t.n += 1;
                        true
                    },
                    map,
                );
            }
            Command::Erase { track, from, to } if ok_track(track) && from < to => {
                if !self.tracks[track]
                    .regions()
                    .iter()
                    .any(|r| r.start < to && r.end > from)
                {
                    return self.refuse();
                }
                self.edit(
                    track,
                    |t, m, rate| {
                        t.clear_fits(from, to, 0) && {
                            t.clear(m, rate, from, to);
                            true
                        }
                    },
                    map,
                );
            }
            Command::Move { track, at, to } if ok_track(track) && to >= 0 => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, m, rate| {
                        let mut r = t.regions[i];
                        t.regions.copy_within(i + 1..t.n, i);
                        t.n -= 1;
                        r.end = to + (r.end - r.start);
                        r.start = to;
                        t.put(m, rate, r)
                    },
                    map,
                );
            }
            Command::SetGain { track, at, gain } if ok_track(track) && gain.is_finite() => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, _, _| {
                        t.regions[i].gain = gain.clamp(0.0, 4.0);
                        true
                    },
                    map,
                );
            }
            Command::SetFades {
                track,
                at,
                fade_in,
                fade_out,
            } if ok_track(track) => {
                let Some(i) = self.tracks[track].at(at) else {
                    return self.refuse();
                };
                self.edit(
                    track,
                    |t, _, _| {
                        t.regions[i].fade_in = fade_in;
                        t.regions[i].fade_out = fade_out;
                        true
                    },
                    map,
                );
            }
            Command::Undo => {
                if self.h_done == 0 || self.any_recording() {
                    return self.refuse();
                }
                self.h_done -= 1;
                self.h_redo += 1;
                self.swap_history((self.h_base + self.h_done) % HISTORY);
            }
            Command::Redo => {
                if self.h_redo == 0 || self.any_recording() {
                    return self.refuse();
                }
                self.swap_history((self.h_base + self.h_done) % HISTORY);
                self.h_done += 1;
                self.h_redo -= 1;
            }
            Command::Forget(id) => {
                let ok = self
                    .takes
                    .get(id as usize)
                    .is_some_and(|t| t.live && !t.recording && !t.loading);
                if !ok || self.in_use(id) {
                    return self.refuse();
                }
                let t = self.takes[id as usize];
                for e in &t.ext[..t.n_ext] {
                    self.pool.give(*e);
                }
                self.takes[id as usize] = NO_TAKE;
            }
            Command::Swap { old, new, ratio } => {
                let ok = |id: u16| {
                    self.takes
                        .get(id as usize)
                        .filter(|t| t.live && !t.recording && !t.loading)
                        .map(|t| t.track as usize)
                };
                let (Some(a), Some(b)) = (ok(old), ok(new)) else {
                    return self.refuse();
                };
                if a != b || old == new || !ratio.is_finite() || ratio <= 0.0 {
                    return self.refuse();
                }
                self.edit(
                    a,
                    |t, _, _| {
                        let mut any = false;
                        for r in &mut t.regions[..t.n] {
                            if r.take == old {
                                r.take = new;
                                r.src = (r.src as f64 * ratio).round() as i64;
                                any = true;
                            }
                        }
                        any
                    },
                    map,
                );
            }
            Command::ClearHistory => {
                self.h_done = 0;
                self.h_redo = 0;
            }
            _ => self.refuse(),
        }
    }

    fn any_recording(&self) -> bool {
        self.tracks.iter().any(|t| t.rec.is_some())
    }

    // ---- saving and loading --------------------------------------------

    /// The tracks' settings and regions and every take's place, for a
    /// save (see [`save`]): None while any track records or a take is
    /// still loading. Tracks with nothing to keep (no regions, no takes,
    /// settings as new) are left out. The takes' audio is not in it: read
    /// each with [`AudioTracks::read_take`]. Allocates: not on the audio
    /// thread.
    pub fn save_file(&self) -> Option<Saved> {
        if self
            .tracks
            .iter()
            .any(|t| t.rec.is_some() || t.tail.is_some())
            || self.takes.iter().any(|t| t.loading)
        {
            return None;
        }
        let takes: Vec<TakeFile> = self
            .takes()
            .map(|k| TakeFile {
                id: k.id,
                track: k.track,
                group: k.group,
                pass: k.pass,
                bpm: k.bpm,
                anchor: k.anchor,
                frames: k.frames,
                channels: k.channels,
                file: String::new(),
                audio: Vec::new(),
            })
            .collect();
        let tracks = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                t.n > 0
                    || takes.iter().any(|k| k.track == *i)
                    || t.input != Input::Left
                    || t.monitor != Monitor::Auto
                    || t.level != 1.0
                    || t.pan != 0.0
                    || t.mute
                    || t.solo
            })
            .map(|(i, t)| TrackFile {
                index: i,
                input: t.input,
                monitor: t.monitor,
                level: t.level,
                pan: t.pan,
                muted: t.mute,
                solo: t.solo,
                regions: t.regions().to_vec(),
            })
            .collect();
        Some(Saved {
            rate: self.rate,
            tracks,
            takes,
        })
    }

    /// Copy frames `from..` of a take into `out` (interleaved, the take's
    /// channels), as many as fit: the number of frames copied, or None if
    /// the take is not there, records or is loading. Does not allocate.
    pub fn read_take(&self, id: u16, from: usize, out: &mut [f32]) -> Option<usize> {
        let t = self
            .takes
            .get(id as usize)
            .filter(|t| t.live && !t.recording && !t.loading)?;
        let ch = t.channels as usize;
        let want = (out.len() / ch).min(t.frames.saturating_sub(from));
        let mut done = 0;
        let mut base = 0;
        for e in &t.ext[..t.n_ext] {
            if done == want {
                break;
            }
            let frames_in = e.len / ch;
            let at = from + done;
            if at >= base + frames_in {
                base += frames_in;
                continue;
            }
            let k = at - base;
            let count = (frames_in - k).min(want - done);
            out[done * ch..(done + count) * ch]
                .copy_from_slice(&self.pool.data[e.start + k * ch..e.start + (k + count) * ch]);
            done += count;
            base += frames_in;
        }
        Some(done)
    }

    /// Put a saved take back, under its own id: its memory is taken now,
    /// and its audio (moved in, never grown or freed here) copies in at
    /// [`LOAD_SPEED`] over the next periods; it plays once copied. Then
    /// the audio waits in [`AudioTracks::take_returned`] for the host to
    /// drop. Does not allocate. Gives `f` back if the id is past the last
    /// or in use, the track past the last, the audio not `frames *
    /// channels` long, the memory short, or [`MAX_TAKES`] loads waiting
    /// to copy or to be collected.
    pub fn load_take(&mut self, f: TakeFile) -> Result<(), TakeFile> {
        let id = f.id as usize;
        let need = f.frames * f.channels;
        if id >= MAX_TAKES
            || self.takes[id].live
            || f.track >= MAX_TRACKS
            || !(1..=2).contains(&f.channels)
            || f.frames == 0
            || f.audio.len() != need
            || !f.anchor.is_finite()
            || self.loads.len() + self.returned.len() >= MAX_TAKES
        {
            return Err(f);
        }
        self.takes[id] = Take {
            live: true,
            track: f.track as u8,
            channels: f.channels as u8,
            group: f.group,
            pass: f.pass,
            bpm: if f.bpm.is_finite() && f.bpm > 0.0 {
                f.bpm
            } else {
                0.0
            },
            anchor: f.anchor,
            loading: true,
            ..NO_TAKE
        };
        let need = round_up(need);
        loop {
            let have: usize = self.takes[id].ext[..self.takes[id].n_ext]
                .iter()
                .map(|e| e.len)
                .sum();
            if have >= need {
                let tk = &mut self.takes[id];
                let last = &mut tk.ext[tk.n_ext - 1];
                let rest = Ext {
                    start: last.start + last.len - (have - need),
                    len: have - need,
                };
                last.len -= rest.len;
                self.pool.give(rest);
                break;
            }
            if !self.grow(id) {
                let t = self.takes[id];
                for e in &t.ext[..t.n_ext] {
                    self.pool.give(*e);
                }
                self.takes[id] = NO_TAKE;
                return Err(f);
            }
        }
        self.groups = self.groups.max(f.group);
        self.loads.push((f.id, f.audio));
        Ok(())
    }

    /// Put a saved track's settings and regions back (its takes loaded
    /// with [`AudioTracks::load_take`], before or after). The undo history
    /// is forgotten. Does not allocate. False (nothing changed) if the
    /// track is past the last or recording, or the regions are too many or
    /// out of order.
    pub fn load_track(&mut self, f: &TrackFile) -> bool {
        let ok = f.index < MAX_TRACKS
            && f.regions.len() <= MAX_REGIONS
            && f.regions.iter().all(|r| r.end > r.start)
            && f.regions.windows(2).all(|w| w[0].end <= w[1].start)
            && self.tracks[f.index].rec.is_none();
        if !ok {
            return false;
        }
        let t = &mut self.tracks[f.index];
        t.input = f.input;
        t.monitor = f.monitor;
        t.level = if f.level.is_finite() {
            f.level.clamp(0.0, 2.0)
        } else {
            1.0
        };
        t.pan = if f.pan.is_finite() {
            f.pan.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        t.mute = f.muted;
        t.solo = f.solo;
        t.n = f.regions.len();
        t.regions[..t.n].copy_from_slice(&f.regions);
        self.h_done = 0;
        self.h_redo = 0;
        true
    }

    /// Takes still copying in.
    pub fn loading(&self) -> usize {
        self.loads.len()
    }

    /// A loaded take's audio, done with: drop it off the audio thread.
    pub fn take_returned(&mut self) -> Option<Vec<f32>> {
        self.returned.pop()
    }

    /// Copy the loads in, `LOAD_SPEED * n` frames' worth of samples a
    /// period, oldest first.
    fn copy_loads(&mut self, n: usize) {
        let mut budget = LOAD_SPEED * n.max(1) * 2;
        while budget > 0 && !self.loads.is_empty() {
            let (id, ref audio) = self.loads[0];
            let tk = &mut self.takes[id as usize];
            let ch = tk.channels as usize;
            let done = tk.frames * ch;
            let count = (audio.len() - done).min(budget);
            // Write samples done..done + count across the stretches.
            let mut base = 0;
            let mut w = 0;
            for e in &tk.ext[..tk.n_ext] {
                if w == count {
                    break;
                }
                let at = done + w;
                if at >= base + e.len {
                    base += e.len;
                    continue;
                }
                let k = at - base;
                let c = (e.len - k).min(count - w);
                self.pool.data[e.start + k..e.start + k + c].copy_from_slice(&audio[at..at + c]);
                self.pool.note(e.start + k, c);
                w += c;
                base += e.len;
            }
            tk.frames = (done + count) / ch;
            budget -= count.max(1);
            if done + count == audio.len() {
                tk.loading = false;
                let (_, audio) = self.loads.remove(0);
                if self.returned.len() < self.returned.capacity() {
                    self.returned.push(audio);
                }
            }
        }
    }

    // ---- recording -----------------------------------------------------

    /// Start recording track `t` at absolute frame `g`, song tick `tick`
    /// (`bpm` there, for the latency).
    fn punch_in(&mut self, t: usize, g: u64, tick: f64, bpm: f64, group: u16, pass: u16) {
        let Some(id) = self.takes.iter().position(|k| !k.live) else {
            self.stats.full += 1;
            self.tracks[t].blocked = true;
            return;
        };
        let lat_ticks = mpfree_transport::time::ticks_in(f64::from(self.latency), bpm, self.rate);
        let ch = self.tracks[t].input.channels();
        self.takes[id] = Take {
            live: true,
            track: t as u8,
            channels: ch as u8,
            group,
            pass,
            bpm,
            anchor: tick - lat_ticks,
            recording: true,
            ..NO_TAKE
        };
        if !self.grow(id) {
            self.takes[id] = NO_TAKE;
            self.stats.full += 1;
            self.tracks[t].blocked = true;
            return;
        }
        self.tracks[t].rec = Some(Rec {
            take: id as u16,
            g0: g,
            t_in: tick,
        });
    }

    /// End track `t`'s recording at absolute frame `g`. `end` is the song
    /// tick it ends on when known exactly (a loop jump); otherwise the tick
    /// at `g` comes from `span`, or (no span: a command at the period's
    /// start) from the take's own length.
    fn punch_out(&mut self, t: usize, map: &TempoMap, g: u64, end: Option<f64>) {
        let Some(rec) = self.tracks[t].rec.take() else {
            return;
        };
        let rate = self.rate;
        let id = rec.take as usize;
        let t_out =
            end.unwrap_or_else(|| tick_after(map, rec.t_in, g.saturating_sub(rec.g0) as f64, rate));
        // Close an older tail at once.
        if let Some(old) = self.tracks[t].tail.take() {
            self.close(old.take as usize);
        }
        // Go on writing the input that belongs before the punch out (the
        // latency), and a crossfade's worth more, so the end can fade into
        // what follows.
        let stop_at = g + u64::from(self.latency) + FADE as u64;
        if self.takes[id].recording && !self.takes[id].cut_short && stop_at > g {
            self.tracks[t].tail = Some(Tail {
                take: rec.take,
                stop_at,
            });
        } else {
            self.close(id);
        }
        let start = rec.t_in.round() as Tick;
        let end_t = t_out.round() as Tick;
        if end_t > start {
            let anchor = self.takes[id].anchor;
            let r = Region {
                start,
                end: end_t,
                take: rec.take,
                src: frames_from(map, anchor, start as f64, rate).round() as i64,
                gain: 1.0,
                fade_in: 0,
                fade_out: 0,
            };
            self.edit(t, |tr, m, rate| tr.put(m, rate, r), map);
        }
    }

    /// The take stops recording: the unwritten end of its last stretch goes
    /// back to the pool.
    fn close(&mut self, id: usize) {
        let tk = &mut self.takes[id];
        if !tk.recording {
            return;
        }
        tk.recording = false;
        if tk.n_ext > 0 {
            let last = &mut tk.ext[tk.n_ext - 1];
            let used = round_up(tk.wpos).min(last.len);
            let rest = Ext {
                start: last.start + used,
                len: last.len - used,
            };
            last.len = used;
            if used == 0 {
                tk.n_ext -= 1;
            }
            self.pool.give(rest);
        }
    }

    /// Room for take `id` to go on writing: the largest free stretch, or
    /// half of what another recording has not filled yet, whichever is
    /// bigger. False if there is none worth having.
    fn grow(&mut self, id: usize) -> bool {
        if self.takes[id].n_ext == MAX_EXTENTS {
            return false;
        }
        let free = self.pool.largest();
        let free_len = free.map_or(0, |i| self.pool.free[i].len);
        let mut steal: Option<(usize, usize)> = None;
        for (k, o) in self.takes.iter().enumerate() {
            if k != id && o.live && o.recording && o.n_ext > 0 {
                let rest =
                    (o.ext[o.n_ext - 1].len - round_up(o.wpos)) / 2 / PEAK_BLOCK * PEAK_BLOCK;
                if rest > steal.map_or(0, |s| s.1) {
                    steal = Some((k, rest));
                }
            }
        }
        let ext = if steal.map_or(0, |s| s.1) > free_len {
            let (k, half) = steal.unwrap_or((0, 0));
            if half < MIN_ROOM {
                return false;
            }
            let o = &mut self.takes[k];
            let last = &mut o.ext[o.n_ext - 1];
            last.len -= half;
            Ext {
                start: last.start + last.len,
                len: half,
            }
        } else {
            if free_len < MIN_ROOM {
                return false;
            }
            let Some(i) = free else { return false };
            self.pool.take(i)
        };
        let tk = &mut self.takes[id];
        tk.ext[tk.n_ext] = ext;
        tk.n_ext += 1;
        tk.wpos = 0;
        true
    }

    /// Append input frames to a recording take. False if the memory ran
    /// out (the take is cut short; what was written stays).
    fn write(&mut self, id: usize, input: &[f32], sel: Input) -> bool {
        let mut j = 0;
        let n = input.len() / 2;
        while j < n {
            let ch = self.takes[id].channels as usize;
            let room = {
                let tk = &self.takes[id];
                if tk.n_ext == 0 {
                    0
                } else {
                    (tk.ext[tk.n_ext - 1].len - tk.wpos) / ch
                }
            };
            if room == 0 {
                if !self.grow(id) {
                    self.takes[id].cut_short = true;
                    return false;
                }
                continue;
            }
            let count = room.min(n - j);
            let tk = &mut self.takes[id];
            let e = tk.ext[tk.n_ext - 1];
            let dst = &mut self.pool.data[e.start + tk.wpos..e.start + tk.wpos + count * ch];
            let src = &input[2 * j..2 * (j + count)];
            match sel {
                Input::Stereo => dst.copy_from_slice(src),
                Input::Left => {
                    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
                        *d = s[0];
                    }
                }
                Input::Right => {
                    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
                        *d = s[1];
                    }
                }
                Input::Sum => {
                    for (d, s) in dst.iter_mut().zip(src.chunks_exact(2)) {
                        *d = 0.5 * (s[0] + s[1]);
                    }
                }
            }
            self.pool.note(e.start + tk.wpos, count * ch);
            tk.wpos += count * ch;
            tk.frames += count;
            j += count;
        }
        true
    }

    // ---- the period ----------------------------------------------------

    /// Run one period. `input` is the line input and `out` the mix, both
    /// interleaved stereo with `block.frames` frames; the tracks are added
    /// to `out`. `map` is the transport's tempo map
    /// ([`mpfree_transport::Transport::tempo_map`]).
    pub fn process(&mut self, block: &Block, map: &TempoMap, input: &[f32], out: &mut [f32]) {
        let n = (block.frames as usize)
            .min(input.len() / 2)
            .min(out.len() / 2)
            .min(MAX_PERIOD as usize);
        let now = block.start_frame;
        for i in 0..self.q_len {
            if let Some(c) = self.queue[i].take() {
                self.apply(c, map, now);
            }
        }
        self.q_len = 0;
        self.copy_loads(n);
        for t in &mut self.tracks {
            t.buf[..2 * n].fill(0.0);
        }
        let markers = block.markers();
        let mut m = 0;
        for span in block.spans() {
            let span_len = span.len as usize;
            let mut i = 0;
            while i < span_len {
                let off = span.offset as usize + i;
                if off >= n {
                    break;
                }
                let f = span.frame + i as u64;
                while m < markers.len() && markers[m].frame <= f {
                    self.marker(map, markers[m].mark, markers[m].frame);
                    m += 1;
                }
                let mut end = span_len.min(i + RUN).min(n - span.offset as usize);
                if m < markers.len() {
                    let to = (markers[m].frame - span.frame) as usize;
                    if to > i {
                        end = end.min(to);
                    }
                }
                self.run(
                    map,
                    span,
                    f,
                    off,
                    end - i,
                    &input[2 * off..2 * (off + end - i)],
                );
                i = end;
            }
        }
        while m < markers.len() {
            self.marker(map, markers[m].mark, markers[m].frame);
            m += 1;
        }
        self.mix(n, out);
    }

    /// A loop jump or a locate while recording: each recording track ends
    /// its take where the song left off and starts a new one (the next
    /// pass) where it lands, at the same frame.
    fn marker(&mut self, map: &TempoMap, mark: Mark, frame: u64) {
        let end = match mark {
            Mark::Looped { end, .. } => end as f64,
            Mark::Located { from, .. } => from as f64,
            _ => return,
        };
        for t in 0..MAX_TRACKS {
            if let Some(rec) = self.tracks[t].rec {
                let tk = self.takes[rec.take as usize];
                self.punch_out(t, map, frame, Some(end));
                // The next pass starts in run(), on the new span's tick.
                self.pending[t] = Some((tk.group, tk.pass + 1));
            }
        }
    }

    fn run(&mut self, map: &TempoMap, span: &Span, f: u64, off: usize, len: usize, input: &[f32]) {
        let recording_state = span.state == State::Recording;
        let rolling = matches!(span.state, State::Playing | State::Recording);
        let tick0 = span.tick_at(f);
        for t in 0..MAX_TRACKS {
            let want = self.active && recording_state && self.tracks[t].armed;
            if !want {
                // A recording cut short waits for the next punch in.
                self.tracks[t].blocked = false;
            }
            match (self.tracks[t].rec.is_some(), want) {
                (false, true) if !self.tracks[t].blocked => {
                    let (group, pass) = self.pending[t].take().unwrap_or_else(|| {
                        self.groups = self.groups.wrapping_add(1);
                        (self.groups, 0)
                    });
                    self.punch_in(t, f, tick0, span.bpm, group, pass);
                }
                (true, false) => {
                    self.punch_out(t, map, f, Some(tick0));
                    self.pending[t] = None;
                }
                _ => {
                    if !want {
                        self.pending[t] = None;
                    }
                }
            }
            // Tails: input that belongs before a punch out.
            if let Some(tail) = self.tracks[t].tail {
                let upto = tail.stop_at.saturating_sub(f).min(len as u64) as usize;
                let id = tail.take as usize;
                let sel = self.input_of(id);
                if upto > 0 {
                    self.write(id, &input[..2 * upto], sel);
                }
                if f + len as u64 >= tail.stop_at
                    || !self.takes[id].recording
                    || self.takes[id].cut_short
                {
                    self.close(id);
                    self.tracks[t].tail = None;
                }
            }
            if let Some(rec) = self.tracks[t].rec {
                let id = rec.take as usize;
                let sel = self.input_of(id);
                if !self.write(id, input, sel) {
                    // The memory is full: the take ends where its audio
                    // does.
                    self.stats.full += 1;
                    self.tracks[t].blocked = true;
                    let g = rec.g0 + self.takes[id].frames as u64;
                    self.punch_out(t, map, g, None);
                }
            }
            let tr = &self.tracks[t];
            let hear_input = match tr.monitor {
                Monitor::Off => false,
                Monitor::In => true,
                Monitor::Auto => tr.armed && (span.state != State::Playing || !self.active),
            };
            if rolling && self.active && tr.rec.is_none() {
                self.play(t, map, span, tick0, off, len);
            }
            if hear_input {
                let tr = &mut self.tracks[t];
                let sel = tr.input;
                let dst = &mut tr.buf[2 * off..2 * (off + len)];
                for (d, s) in dst.chunks_exact_mut(2).zip(input.chunks_exact(2)) {
                    match sel {
                        Input::Stereo => {
                            d[0] += s[0];
                            d[1] += s[1];
                        }
                        Input::Left => {
                            d[0] += s[0];
                            d[1] += s[0];
                        }
                        Input::Right => {
                            d[0] += s[1];
                            d[1] += s[1];
                        }
                        Input::Sum => {
                            let v = 0.5 * (s[0] + s[1]);
                            d[0] += v;
                            d[1] += v;
                        }
                    }
                }
            }
        }
    }

    fn input_of(&self, id: usize) -> Input {
        let tk = &self.takes[id];
        if tk.channels == 2 {
            Input::Stereo
        } else {
            match self.tracks[tk.track as usize].input {
                Input::Stereo => Input::Left,
                i => i,
            }
        }
    }

    /// Add track `t`'s regions for `len` frames from song tick `tick0`
    /// (frame `off` of the period).
    fn play(&mut self, t: usize, map: &TempoMap, span: &Span, tick0: f64, off: usize, len: usize) {
        let rate = self.rate;
        let tail = mpfree_transport::time::ticks_in(FADE as f64, span.bpm, rate) + 1.0;
        let tick1 = tick0 + mpfree_transport::time::ticks_in(len as f64, span.bpm, rate) + 1.0;
        let AudioTracks {
            tracks,
            takes,
            pool,
            fade,
            ..
        } = self;
        let tr = &mut tracks[t];
        let first = tr
            .regions()
            .partition_point(|r| r.end as f64 + tail <= tick0);
        let (regions, buf) = (&tr.regions[..tr.n], &mut tr.buf);
        for r in &regions[first..] {
            if r.start as f64 >= tick1 {
                break;
            }
            let Some(tk) = takes.get(r.take as usize).filter(|k| k.live && !k.loading) else {
                continue;
            };
            // Frames since the region's start at the run's first frame,
            // and the region's length in frames.
            let e0 = frames_from(map, r.start as f64, tick0, rate).round() as i64;
            let len_f = frames_from(map, r.start as f64, r.end as f64, rate).round() as i64;
            let stop = len_f + FADE as i64;
            if e0 >= stop || e0 + len as i64 <= 0 {
                continue;
            }
            let fi = i64::from(r.fade_in).min(len_f / 2).max(0);
            let fo = i64::from(r.fade_out).min(len_f / 2).max(0);
            let fade = &*fade;
            let gain_at = |e: i64| -> f32 {
                if e < 0 || e >= stop {
                    return 0.0;
                }
                let mut g = r.gain;
                if e < FADE as i64 {
                    g *= fade[e as usize];
                }
                if e >= len_f {
                    g *= fade[(FADE as i64 - (e - len_f)) as usize];
                }
                if fi > 0 && e < fi {
                    g *= e as f32 / fi as f32;
                }
                if fo > 0 && e >= len_f - fo && e < len_f {
                    g *= (len_f - e) as f32 / fo as f32;
                }
                g
            };
            let dst = &mut buf[2 * off..2 * (off + len)];
            let plain_from = (FADE as i64).max(fi);
            if e0 >= plain_from && e0 + len as i64 <= len_f - fo {
                // The whole run is past the fades: one gain.
                let g = r.gain;
                tk.add_into(&pool.data, r.src + e0, dst, &|_| g);
            } else {
                tk.add_into(&pool.data, r.src + e0, dst, &|j| gain_at(e0 + j as i64));
            }
        }
    }

    /// Each track's level, pan, mute and solo, ramped over the period, and
    /// the sum into `out`.
    fn mix(&mut self, n: usize, out: &mut [f32]) {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        for tr in &mut self.tracks {
            let audible = !tr.mute && (!any_solo || tr.solo);
            let lv = if audible { tr.level } else { 0.0 };
            let target = [lv * (1.0 - tr.pan).min(1.0), lv * (1.0 + tr.pan).min(1.0)];
            let from = tr.gains;
            let step = [
                (target[0] - from[0]) / n.max(1) as f32,
                (target[1] - from[1]) / n.max(1) as f32,
            ];
            for (j, (d, o)) in tr.buf[..2 * n]
                .chunks_exact_mut(2)
                .zip(out.chunks_exact_mut(2))
                .enumerate()
            {
                let k = (j + 1) as f32;
                d[0] *= from[0] + step[0] * k;
                d[1] *= from[1] + step[1] * k;
                o[0] += d[0];
                o[1] += d[1];
            }
            if n > 0 {
                tr.gains = target;
            }
        }
    }
}
