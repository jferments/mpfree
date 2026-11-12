//! The looper in the pad player: the line input (the MPC One's capture
//! device) and the `mpfree-looper` engine on the transport.
//!
//! - [`LineIn`] keeps the capture stream: closed until the Prefs row "Line
//!   input" turns it on (never on at start), opened at the stock settings
//!   (stereo S32_LE, 44.1 kHz, 128-frame periods, a 256-frame buffer) and
//!   read without waiting, so the playback loop never blocks on it. What
//!   arrives goes through a small FIFO; a period with too little input is
//!   filled with silence and counted.
//! - [`LoopHost`] runs the looper on the transport's block each period and
//!   gives the loop to the mixer's Loop track and the line input to its
//!   input strip, which is heard when Monitor is on (see `mixing.rs`). It
//!   records from a
//!   [`LoopSource`]: the pad player's own sound (the default, which opens
//!   nothing) or the line input. It logs a `looper:` line for each press and
//!   each change, and builds the loop's waveform for the Looper tab a slice
//!   at a time.
//! - [`Printer`] keeps each loop's pitch at another tempo: when a loop is
//!   playing at a tempo other than its own, the host bounces it
//!   ([`Looper::bounce_print`]), a worker thread stretches the bounce to
//!   the loop's length at the tempo (`mpfree_looper::print`, on the
//!   sampler's stretch engine), and the print goes back to the looper
//!   ([`Looper::print`]), which plays it in place of the layers. Until it
//!   is ready (a fraction of a second on the workstation, more on the MPC)
//!   the loop plays like tape. Buffers are made and dropped on the worker.
//!   Overdub over a print fits the loop to the tempo first (see
//!   `mpfree-looper`).
//!
//! Which jacks the two capture channels are is not verified (line in L/R is
//! the likely reading). Opening the capture device is a state change on the
//! MPC: it waits for the owner's go in the round 6 runbook.

use crate::alsa;
use mpfree_looper::print::{frames_at, PrintJob};
use mpfree_looper::save::Saved;
use mpfree_looper::{
    Bounce, Command, LayerCommand, LayerInfo, LoopFile, Looper, Memory, Phase, Print, Stats,
    Status, MAX_LAYERS, MAX_LOOPS,
};
use mpfree_transport::Block;
use mpfree_ui::wave::{Peaks, WaveColumn};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

pub const PERIOD: usize = 128;
/// Frames the FIFO holds: 8 periods. More than 3 periods waiting means the
/// input runs ahead; the oldest frames are dropped down to 2 periods.
const FIFO: usize = 8 * PERIOD;
/// The capture buffer the stream must get (2 periods).
pub const CAPTURE_BUFFER: u32 = 2 * PERIOD as u32;
/// Read errors in a row (not overruns) before the input is closed.
const MAX_ERRORS: u32 = 3;
/// Columns of the loop's waveform.
pub const WAVE_COLUMNS: usize = 600;
/// Loop frames turned into waveform columns per period (about 1 % of a
/// period's time on the MPC; a 4 s loop takes about 0.1 s).
const WAVE_FRAMES_PER_PERIOD: usize = 8192;

/// Where the looper records from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoopSource {
    /// The pad player's own sound (kits, grains and the synth) after the
    /// master level, before the click and the loop. Opens nothing, and
    /// needs no latency made up.
    #[default]
    Pads,
    /// The MPC's line input (Prefs, Line input).
    Line,
}

impl LoopSource {
    pub const ALL: [LoopSource; 2] = [LoopSource::Pads, LoopSource::Line];

    pub fn name(self) -> &'static str {
        match self {
            LoopSource::Pads => "Pads and synth",
            LoopSource::Line => "Line input",
        }
    }
}

/// What the app asks of the looper and the line input.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopAction {
    Command(Command),
    /// Record from the pads or the line input.
    Source(LoopSource),
    /// Hear the line input through the outputs.
    Monitor(bool),
    /// Open or close the capture device (a state change on the MPC).
    LineInput(bool),
    /// A layer's control (its speaker): the layer by its id
    /// ([`LayerInfo::id`]), which stays the same through undo and redo.
    Layer(u8, LayerCommand),
    /// Pick loop `n` (0 to [`MAX_LOOPS`] - 1): the buttons, the layers'
    /// controls and the screens act on it from the next period.
    Select(usize),
}

/// What the line input counted since it was opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LineStats {
    pub periods: u64,
    pub frames: u64,
    /// Periods that had too little input (filled with silence), after the
    /// first few.
    pub short: u64,
    /// Frames dropped because the input ran ahead.
    pub dropped: u64,
    /// The input was not read in time and the stream restarted.
    pub overruns: u64,
    pub errors: u64,
}

pub struct LineIn {
    pcm: Option<alsa::Pcm>,
    raw: Vec<i32>,
    /// Interleaved stereo, a ring of FIFO frames.
    fifo: Vec<f32>,
    head: usize,
    len: usize,
    pub stats: LineStats,
    errors_in_row: u32,
}

impl Default for LineIn {
    fn default() -> Self {
        LineIn::closed()
    }
}

impl LineIn {
    pub fn closed() -> LineIn {
        LineIn {
            pcm: None,
            raw: vec![0; 2 * FIFO],
            fifo: vec![0.0; 2 * FIFO],
            head: 0,
            len: 0,
            stats: LineStats::default(),
            errors_in_row: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.pcm.is_some()
    }

    /// Open card `card`'s capture device at the stock settings and start
    /// it. On any refusal the device is closed again and nothing else
    /// changes; the error says what the card said.
    pub fn open(&mut self, card: u32) -> Result<String, String> {
        if self.is_open() {
            return Ok("line input: already on".to_string());
        }
        let path = format!("/dev/snd/pcmC{card}D0c");
        let pcm = alsa::Pcm::open_capture(&path)
            .map_err(|e| format!("line input: {path}: {e} (it stays off)"))?;
        let mut hw = alsa::HwParams::stock_capture();
        if let Err(e) = pcm.hw_params(&mut hw) {
            let mut any = alsa::HwParams::format_only();
            let allows = match pcm.refine(&mut any) {
                Ok(()) => format!(
                    "the card allows period {}, periods {}, buffer {}",
                    any.interval(alsa::PARAM_PERIOD_SIZE).describe(),
                    any.interval(alsa::PARAM_PERIODS).describe(),
                    any.interval(alsa::PARAM_BUFFER_SIZE).describe()
                ),
                Err(e2) => format!("asking what it allows failed too: {e2}"),
            };
            return Err(format!(
                "line input: {path}: the stock capture settings were refused: {e}; {allows} \
                                (closed; it stays off)"
            ));
        }
        let buffer = hw.interval(alsa::PARAM_BUFFER_SIZE).single().unwrap_or(0);
        if buffer != CAPTURE_BUFFER || !hw.mask_has(alsa::PARAM_FORMAT, alsa::FORMAT_S32_LE) {
            return Err(format!("line input: {path}: unexpected buffer {buffer} frames or format (closed; it stays off)"));
        }
        // Wake every period; never start by itself (start() below); stop
        // (an overrun) when the buffer is full.
        let mut sw = alsa::SwParams::new(PERIOD as u32, u32::MAX >> 1, buffer);
        pcm.sw_params(&mut sw)
            .map_err(|e| format!("line input: sw params: {e} (closed; it stays off)"))?;
        pcm.prepare()
            .map_err(|e| format!("line input: prepare: {e} (closed; it stays off)"))?;
        pcm.start()
            .map_err(|e| format!("line input: start: {e} (closed; it stays off)"))?;
        self.pcm = Some(pcm);
        self.head = 0;
        self.len = 0;
        self.stats = LineStats::default();
        self.errors_in_row = 0;
        Ok(format!("line input: on: {path}, rate {}, channels {}, period {} frames, buffer {buffer} frames",
                   hw.interval(alsa::PARAM_RATE).describe(), hw.interval(alsa::PARAM_CHANNELS).describe(),
                   hw.interval(alsa::PARAM_PERIOD_SIZE).describe()))
    }

    /// Stop and close the capture device.
    pub fn close(&mut self) -> String {
        let Some(pcm) = self.pcm.take() else {
            return "line input: already off".to_string();
        };
        let _ = pcm.drop_stream();
        drop(pcm);
        self.len = 0;
        format!("line input: off ({})", self.describe())
    }

    pub fn describe(&self) -> String {
        let s = &self.stats;
        format!(
            "{} periods, {} short, {} frames dropped, {} overruns, {} errors",
            s.periods, s.short, s.dropped, s.overruns, s.errors
        )
    }

    /// Interleaved S32 frames that arrived (the stream, or a test).
    pub fn push_s32(&mut self, interleaved: &[i32]) {
        let frames = interleaved.len() / 2;
        for f in 0..frames {
            if self.len == FIFO {
                self.head = (self.head + 1) % FIFO;
                self.len -= 1;
                self.stats.dropped += 1;
            }
            let at = (self.head + self.len) % FIFO;
            self.fifo[2 * at] = interleaved[2 * f] as f32 / 2_147_483_648.0;
            self.fifo[2 * at + 1] = interleaved[2 * f + 1] as f32 / 2_147_483_648.0;
            self.len += 1;
        }
        self.stats.frames += frames as u64;
    }

    /// Read what the stream has. A note comes back if the input closed
    /// itself after errors.
    fn pull(&mut self) -> Option<String> {
        for _ in 0..2 {
            let room = FIFO - self.len;
            let pcm = self.pcm.as_ref()?;
            if room == 0 {
                return None;
            }
            let read = pcm.read_frames(&mut self.raw[..2 * room]);
            match read {
                Ok(0) => return None,
                Ok(n) => {
                    self.errors_in_row = 0;
                    let n = (n as usize).min(room);
                    let raw = std::mem::take(&mut self.raw);
                    self.push_s32(&raw[..2 * n]);
                    self.raw = raw;
                }
                Err(e) if e.raw_os_error() == Some(alsa::EPIPE) => {
                    self.stats.overruns += 1;
                    if pcm.prepare().and_then(|_| pcm.start()).is_err() {
                        self.stats.errors += 1;
                        self.errors_in_row += 1;
                    }
                    return None;
                }
                Err(e) => {
                    self.stats.errors += 1;
                    self.errors_in_row += 1;
                    if self.errors_in_row > MAX_ERRORS {
                        let n = self.errors_in_row;
                        let closed = self.close();
                        return Some(format!("line input: read failed {n} times in a row ({e}); turned off: {closed}"));
                    }
                    return None;
                }
            }
        }
        None
    }

    /// One period of input into `left` and `right` (silence while off, and
    /// for whatever has not arrived). A note comes back if something went
    /// wrong that the log should say.
    pub fn period(
        &mut self,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Option<String> {
        if !self.is_open() {
            left.fill(0.0);
            right.fill(0.0);
            return None;
        }
        let note = self.pull();
        self.stats.periods += 1;
        let take = self.len.min(PERIOD);
        for f in 0..take {
            let at = (self.head + f) % FIFO;
            left[f] = self.fifo[2 * at];
            right[f] = self.fifo[2 * at + 1];
        }
        left[take..].fill(0.0);
        right[take..].fill(0.0);
        self.head = (self.head + take) % FIFO;
        self.len -= take;
        // The first periods after the start may come before any input.
        if take < PERIOD && self.stats.periods > 4 {
            self.stats.short += 1;
        }
        if self.len > 3 * PERIOD {
            let drop = self.len - 2 * PERIOD;
            self.head = (self.head + drop) % FIFO;
            self.len -= drop;
            self.stats.dropped += drop as u64;
        }
        note
    }
}

/// The loop's waveform for the Looper tab: min, max and RMS of the playing
/// layers, summed, in [`WAVE_COLUMNS`] columns per channel; and for the
/// Play page's strip, each layer's peak (either side) per column.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoopWave {
    pub frames: u64,
    pub columns: Vec<[WaveColumn; 2]>,
    /// `layers[k][col]`: layer k's peak in that column, 0 to 1.
    pub layers: Vec<Vec<f32>>,
}

impl Peaks for LoopWave {
    fn frames(&self) -> u64 {
        self.frames
    }

    fn channels(&self) -> usize {
        2
    }

    fn column(&self, ch: usize, f0: f64, f1: f64) -> WaveColumn {
        let n = self.columns.len();
        if n == 0 || ch > 1 || f1 <= 0.0 || f0 >= self.frames as f64 {
            return WaveColumn::default();
        }
        let per = self.frames as f64 / n as f64;
        let a = ((f0.max(0.0) / per).floor() as usize).min(n - 1);
        let b = ((f1 / per).ceil() as usize).clamp(a + 1, n);
        let mut c = WaveColumn {
            min: f32::MAX,
            max: f32::MIN,
            rms: 0.0,
        };
        let mut sq = 0.0;
        for col in &self.columns[a..b] {
            c.min = c.min.min(col[ch].min);
            c.max = c.max.max(col[ch].max);
            sq += col[ch].rms * col[ch].rms;
        }
        c.rms = (sq / (b - a) as f32).sqrt();
        c
    }
}

/// Builds a [`LoopWave`] a slice at a time.
struct WaveBuilder {
    /// What the wave is of: the loop's length, layers and phase, and which
    /// loop.
    of: (usize, usize, Phase, usize),
    at: usize,
    frames: usize,
    acc: Vec<[(f32, f32, f32, u32); 2]>,
    /// Each layer's peak per column (made at the start, so the audio
    /// thread never allocates for it).
    peaks: Vec<Vec<f32>>,
    building: bool,
}

impl WaveBuilder {
    fn new() -> WaveBuilder {
        WaveBuilder {
            of: (0, 0, Phase::Empty, 0),
            at: 0,
            frames: 0,
            acc: vec![[(0.0, 0.0, 0.0, 0); 2]; WAVE_COLUMNS],
            peaks: vec![vec![0.0; WAVE_COLUMNS]; mpfree_looper::MAX_LAYERS],
            building: false,
        }
    }

    fn start(&mut self, frames: usize, of: (usize, usize, Phase, usize)) {
        self.of = of;
        self.frames = frames;
        self.at = 0;
        self.building = frames > 0;
        for c in self.acc.iter_mut() {
            *c = [(f32::MAX, f32::MIN, 0.0, 0); 2];
        }
        for l in self.peaks.iter_mut() {
            l.fill(0.0);
        }
    }

    /// One slice; the finished wave when the last slice is done.
    fn step(&mut self, looper: &Looper, layers: usize) -> Option<LoopWave> {
        if !self.building {
            return None;
        }
        let len = self.frames;
        let end = (self.at + WAVE_FRAMES_PER_PERIOD).min(len);
        for k in 0..layers.min(self.peaks.len()) {
            if let Some(l) = looper.layer(k) {
                let peaks = &mut self.peaks[k];
                for f in self.at..end {
                    let p = l[2 * f].abs().max(l[2 * f + 1].abs());
                    let col = f * WAVE_COLUMNS / len;
                    peaks[col] = peaks[col].max(p);
                }
            }
        }
        for f in self.at..end {
            let col = f * WAVE_COLUMNS / len;
            for ch in 0..2 {
                let mut v = 0.0;
                for k in 0..layers {
                    if let Some(l) = looper.layer(k) {
                        v += l[2 * f + ch];
                    }
                }
                let a = &mut self.acc[col][ch];
                a.0 = a.0.min(v);
                a.1 = a.1.max(v);
                a.2 += v * v;
                a.3 += 1;
            }
        }
        self.at = end;
        if end < len {
            return None;
        }
        self.building = false;
        let columns = self
            .acc
            .iter()
            .map(|c| {
                let col = |a: (f32, f32, f32, u32)| {
                    if a.3 == 0 {
                        WaveColumn::default()
                    } else {
                        WaveColumn {
                            min: a.0,
                            max: a.1,
                            rms: (a.2 / a.3 as f32).sqrt(),
                        }
                    }
                };
                [col(c[0]), col(c[1])]
            })
            .collect();
        let layers = self.peaks[..self.of.1.min(self.peaks.len())]
            .iter()
            .map(|l| l.iter().map(|p| p.min(1.0)).collect())
            .collect();
        Some(LoopWave {
            frames: len as u64,
            columns,
            layers,
        })
    }
}

/// What the Looper tab shows (sent with the timing, and on changes).
#[derive(Clone, Debug, PartialEq)]
pub struct LooperView {
    pub status: Status,
    pub stats: Stats,
    pub source: LoopSource,
    pub line_on: bool,
    pub line: LineStats,
    pub monitor: bool,
    /// The recorded source's peaks since the last view, dBFS (-inf when
    /// silent).
    pub meter_db: [f32; 2],
    /// The loop's place in the song: beats per bar and the frames of a
    /// beat at the tempo it was recorded at, for the bar lines.
    pub beats_per_bar: u32,
    pub frames_per_beat: f64,
    /// The loop's layers in play order (the first pass first) with their
    /// mix, for the speakers; None past the last.
    pub layers: [Option<LayerInfo>; MAX_LAYERS],
    /// Every loop, for the Looper tab's list; `status` is `loops[selected]`.
    pub loops: [Status; MAX_LOOPS],
    pub selected: usize,
    /// The memory all the loops share (frames).
    pub memory: Memory,
    /// Frames per second, for the lengths in seconds.
    pub rate: u32,
}

impl Default for LooperView {
    fn default() -> Self {
        let l = Looper::new(44_100, 0);
        LooperView {
            status: l.status(),
            stats: l.stats(),
            source: LoopSource::Pads,
            line_on: false,
            line: LineStats::default(),
            monitor: false,
            meter_db: [f32::NEG_INFINITY; 2],
            beats_per_bar: 4,
            frames_per_beat: 0.0,
            layers: [None; MAX_LAYERS],
            loops: [l.status(); MAX_LOOPS],
            selected: 0,
            memory: l.memory(),
            rate: 44_100,
        }
    }
}

pub struct LoopHost {
    pub looper: Looper,
    pub line: LineIn,
    pub monitor: bool,
    pub source: LoopSource,
    input_l: [f32; PERIOD],
    input_r: [f32; PERIOD],
    /// The pad player's sound for this period (see [`LoopHost::take_pads`]).
    pads_l: [f32; PERIOD],
    pads_r: [f32; PERIOD],
    input: Vec<f32>,
    out: Vec<f32>,
    last: Status,
    /// Periods where the loop or the input had to be kept within ±1
    /// (without the mixer: [`LoopHost::period`]).
    pub clipped: u64,
    wave: WaveBuilder,
    /// The recorded source's peaks since the last view.
    meter: [f32; 2],
    beats_per_bar: u32,
    rate: u32,
    printer: Printer,
    /// A save of the loops under way, and one done.
    saving: Option<Saving>,
    saved: Option<Saved>,
    /// Saves started again because a loop changed while it was read.
    pub save_restarts: u64,
}

/// A save under way ([`LoopHost::save`]).
enum Saving {
    /// Until no loop is recording a first pass, overdubbing, merging,
    /// fitting or loading.
    Waiting,
    /// The worker is making room for the layers' audio.
    Sizing {
        /// Each loop's (version, layers, frames) when the save began.
        at: [(u64, usize, usize); MAX_LOOPS],
    },
    /// Reading layer `k` of `saved.loops[i]` from `frame`.
    Reading {
        saved: Saved,
        at: [(u64, usize, usize); MAX_LOOPS],
        i: usize,
        k: usize,
        frame: usize,
    },
}

/// Frames of the loops' layers a save reads for each frame that plays.
pub const SAVE_SPEED: usize = 64;

/// What the worker is asked: a buffer for a bounce, a print, room for a
/// save's audio, or to drop what the audio loop is done with.
enum ToPrinter {
    Buffer(usize),
    Print(Bounce, usize),
    Drop(Option<Print>, Vec<f32>),
    Size(Saved),
    DropSaved(Saved),
    DropLoop(LoopFile),
}

enum FromPrinter {
    Buffer(Vec<f32>),
    /// The print for (loop, frames), or None if it could not be made.
    Done(usize, usize, Option<Print>),
    /// A save with room for every layer's audio.
    Sized(Saved),
}

/// One print under way: for loop `n`, `frames` long, from the loop at
/// `version`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wanted {
    n: usize,
    frames: usize,
    version: u64,
}

/// Prints made on a worker thread, one at a time (see the module notes).
pub struct Printer {
    rate: u32,
    tx: Option<Sender<ToPrinter>>,
    rx: Option<Receiver<FromPrinter>>,
    /// A buffer from the worker, for the next bounce.
    buf: Option<Vec<f32>>,
    asked: bool,
    /// The print under way, and whether its bounce is rendering.
    doing: Option<Wanted>,
    bouncing: bool,
    /// Prints handed to the looper, and ones made too late to use.
    pub made: u64,
    pub stale: u64,
    pub failed: u64,
    /// A save the worker made room for.
    sized: Option<Saved>,
}

impl Printer {
    fn new(rate: u32) -> Printer {
        let (tx, jobs) = channel::<ToPrinter>();
        let (done, rx) = channel();
        let spawned = std::thread::Builder::new()
            .name("mpfree-print".into())
            .spawn(move || {
                for m in jobs {
                    let reply = match m {
                        ToPrinter::Buffer(n) => FromPrinter::Buffer(Vec::with_capacity(n)),
                        ToPrinter::Print(b, frames) => {
                            let p = PrintJob::new(&b, rate, frames).ok().and_then(|mut j| {
                                while !j.run(1 << 15) {}
                                j.finish()
                            });
                            FromPrinter::Done(b.from_loop, frames, p)
                        }
                        ToPrinter::Drop(p, b) => {
                            drop((p, b));
                            continue;
                        }
                        ToPrinter::Size(mut saved) => {
                            for (_, f) in &mut saved.loops {
                                for y in &mut f.layers {
                                    y.audio = vec![0.0; 2 * f.frames];
                                }
                            }
                            FromPrinter::Sized(saved)
                        }
                        ToPrinter::DropSaved(saved) => {
                            drop(saved);
                            continue;
                        }
                        ToPrinter::DropLoop(f) => {
                            drop(f);
                            continue;
                        }
                    };
                    if done.send(reply).is_err() {
                        break;
                    }
                }
            })
            .is_ok();
        Printer {
            rate,
            tx: spawned.then_some(tx),
            rx: spawned.then_some(rx),
            buf: None,
            asked: false,
            doing: None,
            bouncing: false,
            made: 0,
            stale: 0,
            failed: 0,
            sized: None,
        }
    }

    fn send(&self, m: ToPrinter) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(m);
        }
    }

    /// One period's look after the looper ran at `bpm`: hand over what
    /// the worker made, take a finished bounce to it, drop old prints, and
    /// start the next print a loop needs (or take away one it no longer
    /// does). Cheap when nothing changes. It allocates nothing itself;
    /// a message to the worker can (a block of the channel, now and then),
    /// and only goes when a print starts, arrives or is dropped.
    fn tend(&mut self, looper: &mut Looper, bpm: f64) {
        if self.tx.is_none() || bpm.is_nan() || bpm <= 0.0 {
            return;
        }
        for n in 0..MAX_LOOPS {
            if let Some(p) = looper.take_print(n) {
                self.send(ToPrinter::Drop(Some(p), Vec::new()));
            }
            if let Some(f) = looper.take_loaded(n) {
                self.send(ToPrinter::DropLoop(f));
            }
        }
        while let Some(m) = self.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            match m {
                FromPrinter::Sized(saved) => self.sized = Some(saved),
                FromPrinter::Buffer(b) => {
                    self.asked = false;
                    self.buf = Some(b);
                }
                FromPrinter::Done(n, frames, p) => {
                    self.doing = None;
                    let Some(p) = p else {
                        self.failed += 1;
                        continue;
                    };
                    let now = looper.loop_status(n);
                    let wanted = now.is_some_and(|s| {
                        s.version == p.version && Some(frames) == want(&s, bpm, self.rate)
                    });
                    if !wanted {
                        self.stale += 1;
                        self.send(ToPrinter::Drop(Some(p), Vec::new()));
                        continue;
                    }
                    match looper.print(n, Some(p)) {
                        Ok(()) => self.made += 1,
                        Err(p) => self.send(ToPrinter::Drop(p, Vec::new())),
                    }
                }
            }
        }
        if self.bouncing {
            match looper.take_bounce() {
                Some(b) => {
                    self.bouncing = false;
                    let d = self.doing.expect("a print is under way");
                    if b.complete && b.version == d.version {
                        self.send(ToPrinter::Print(b, d.frames));
                    } else {
                        self.doing = None;
                        self.send(ToPrinter::Drop(None, b.audio));
                    }
                }
                None if looper.bounce_progress().is_none() => {
                    self.bouncing = false;
                    self.doing = None;
                }
                None => {}
            }
        }
        if self.doing.is_some() {
            return;
        }
        for n in 0..MAX_LOOPS {
            let Some(s) = looper.loop_status(n) else {
                break;
            };
            let have = looper.print_info(n);
            match want(&s, bpm, self.rate) {
                None if have.is_some() && s.phase == Phase::Playing => {
                    let _ = looper.print(n, None);
                }
                None => {}
                Some(frames) if have == Some((frames, s.ticks, s.version)) => {}
                Some(frames) => {
                    let need = 2 * s.frames;
                    match self.buf.take() {
                        Some(b) if b.capacity() >= need => match looper.bounce_print(n, b) {
                            Ok(()) => {
                                self.doing = Some(Wanted {
                                    n,
                                    frames,
                                    version: s.version,
                                });
                                self.bouncing = true;
                            }
                            Err(b) => self.buf = Some(b),
                        },
                        Some(b) => {
                            self.send(ToPrinter::Drop(None, b));
                            self.ask(need);
                        }
                        None => self.ask(need),
                    }
                    return;
                }
            }
        }
    }

    fn ask(&mut self, samples: usize) {
        if !self.asked {
            self.asked = true;
            self.send(ToPrinter::Buffer(samples));
        }
    }
}

/// The print loop `s` needs at `bpm`: its length then, if that is not its
/// own (within a frame) and it is playing; within the stretch's 10x.
fn want(s: &Status, bpm: f64, rate: u32) -> Option<usize> {
    if s.phase != Phase::Playing || s.frames == 0 || s.ticks <= 0 {
        return None;
    }
    let frames = frames_at(s.ticks, bpm, rate);
    let r = frames as f64 / s.frames as f64;
    (frames.abs_diff(s.frames) > 1 && (0.1..=10.0).contains(&r)).then_some(frames)
}

impl LoopHost {
    /// Room for `seconds` of stereo loop (all layers together).
    pub fn new(rate: u32, seconds: u32) -> LoopHost {
        let looper = Looper::new(rate, seconds);
        let last = looper.status();
        LoopHost {
            looper,
            line: LineIn::closed(),
            monitor: false,
            source: LoopSource::Pads,
            input_l: [0.0; PERIOD],
            input_r: [0.0; PERIOD],
            pads_l: [0.0; PERIOD],
            pads_r: [0.0; PERIOD],
            input: vec![0.0; 2 * PERIOD],
            out: vec![0.0; 2 * PERIOD],
            last,
            clipped: 0,
            wave: WaveBuilder::new(),
            meter: [0.0; 2],
            beats_per_bar: 4,
            rate,
            printer: Printer::new(rate),
            saving: None,
            saved: None,
            save_restarts: 0,
        }
    }

    /// Save every loop for a project: once no loop is recording a first
    /// pass, overdubbing, merging, fitting or loading, the worker makes
    /// room, and each loop's layers are read as they play ([`SAVE_SPEED`]
    /// frames for each frame), with no glitch. A loop that changes while
    /// it is read starts the save again. [`LoopHost::take_saved`] gives
    /// it when done, with each layer's audio: the project keeps the text
    /// (`mpfree_looper::save::to_text`, each layer naming its file) and a
    /// WAV a layer (`save::wav`). Allocates only the list of loops, here
    /// and when it starts again.
    // The project browser calls this (not wired into the app yet).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn save(&mut self) -> String {
        if self.printer.tx.is_none() {
            return "looper: cannot save, the worker did not start".into();
        }
        if let Some(s) = self.saved.take() {
            self.printer.send(ToPrinter::DropSaved(s));
        }
        if let Some(Saving::Reading { saved, .. }) = self.saving.take() {
            self.printer.send(ToPrinter::DropSaved(saved));
        }
        self.saving = Some(Saving::Waiting);
        "looper: saving the loops".into()
    }

    /// How far the save is, 0 to 1 (0 while it waits for a loop).
    // The project browser calls this (not wired into the app yet).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn saving(&self) -> Option<f32> {
        match self.saving.as_ref()? {
            Saving::Reading {
                saved, i, k, frame, ..
            } => {
                let all: usize = saved
                    .loops
                    .iter()
                    .map(|(_, f)| f.layers.len() * f.frames)
                    .sum();
                let done: usize = saved.loops[..*i]
                    .iter()
                    .map(|(_, f)| f.layers.len() * f.frames)
                    .sum::<usize>()
                    + saved.loops.get(*i).map_or(0, |(_, f)| k * f.frames)
                    + frame;
                Some(done as f32 / all.max(1) as f32)
            }
            _ => Some(0.0),
        }
    }

    /// The save, once done: drop it away from the audio loop.
    // The project browser calls this (not wired into the app yet).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn take_saved(&mut self) -> Option<Saved> {
        self.saved.take()
    }

    /// Load saved loops (`save::from_text`, each layer's audio from its
    /// WAV with `save::from_wav`) into the loops they came
    /// from, which must be empty: each copies in while silent, then fades
    /// in, in time with the song. A loop that is not empty keeps what it
    /// has (its saved one is dropped on the worker).
    // The project browser calls this (not wired into the app yet).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn load(&mut self, saved: Saved) -> String {
        let (mut done, mut kept) = (Vec::new(), Vec::new());
        for (n, f) in saved.loops {
            match self.looper.load(n, f) {
                Ok(()) => done.push((n + 1).to_string()),
                Err(f) => {
                    kept.push((n + 1).to_string());
                    self.printer.send(ToPrinter::DropLoop(f));
                }
            }
        }
        self.looper.select_loop(saved.selected);
        let mut note = format!("looper: loading loops {}", done.join(", "));
        if done.is_empty() {
            note = "looper: no loops to load".into();
        }
        if !kept.is_empty() {
            note += &format!(
                "; loops {} were not empty and kept what they had",
                kept.join(", ")
            );
        }
        if saved.rate != self.rate {
            note += &format!(
                " (saved at {} Hz: they play like tape until their prints are in)",
                saved.rate
            );
        }
        note
    }

    /// Each loop's (version, layers, frames) now.
    fn shapes(&self) -> [(u64, usize, usize); MAX_LOOPS] {
        let mut at = [(0, 0, 0); MAX_LOOPS];
        for (n, a) in at.iter_mut().enumerate() {
            if let Some(s) = self.looper.loop_status(n) {
                *a = (s.version, s.layers, s.frames);
            }
        }
        at
    }

    /// One period of a save: see [`LoopHost::save`].
    fn tend_save(&mut self) {
        let Some(state) = self.saving.take() else {
            return;
        };
        self.saving = match state {
            Saving::Waiting => {
                // Every loop empty (or armed) or still enough to read.
                let ready = (0..MAX_LOOPS).all(|n| {
                    let s = self.looper.loop_status(n).expect("a loop");
                    matches!(s.phase, Phase::Empty | Phase::Armed) && s.loading.is_none()
                        || self.looper.read_layer(n, 0, 0, &mut []).is_some()
                });
                if ready {
                    let saved = Saved {
                        rate: self.rate,
                        selected: self.looper.selected(),
                        loops: (0..MAX_LOOPS)
                            .filter_map(|n| self.looper.loop_file(n).map(|f| (n, f)))
                            .collect(),
                    };
                    self.printer.send(ToPrinter::Size(saved));
                    Some(Saving::Sizing { at: self.shapes() })
                } else {
                    Some(Saving::Waiting)
                }
            }
            Saving::Sizing { at } => match self.printer.sized.take() {
                Some(saved) => Some(Saving::Reading {
                    saved,
                    at,
                    i: 0,
                    k: 0,
                    frame: 0,
                }),
                None => Some(Saving::Sizing { at }),
            },
            Saving::Reading {
                mut saved,
                at,
                mut i,
                mut k,
                mut frame,
            } => {
                if self.shapes() != at {
                    // A loop changed: start again.
                    self.save_restarts += 1;
                    self.printer.send(ToPrinter::DropSaved(saved));
                    self.saving = Some(Saving::Waiting);
                    return;
                }
                let mut budget = SAVE_SPEED * PERIOD;
                while budget > 0 && i < saved.loops.len() {
                    let (n, f) = &mut saved.loops[i];
                    let take = budget.min(f.frames - frame);
                    let out = &mut f.layers[k].audio[2 * frame..2 * (frame + take)];
                    if self.looper.read_layer(*n, k, frame, out).is_none() {
                        // Busy for now (a merge or a fit starting): wait.
                        break;
                    }
                    budget -= take;
                    frame += take;
                    if frame == f.frames {
                        frame = 0;
                        k += 1;
                        if k == f.layers.len() {
                            k = 0;
                            i += 1;
                        }
                    }
                }
                if i == saved.loops.len() {
                    self.saved = Some(saved);
                    None
                } else {
                    Some(Saving::Reading {
                        saved,
                        at,
                        i,
                        k,
                        frame,
                    })
                }
            }
        };
    }

    /// A press or a setting, with its `looper:` line for the log.
    pub fn command(&mut self, c: Command) -> String {
        let s = self.looper.status();
        let what = match (c, s.phase) {
            (Command::Record, Phase::Empty) => format!(
                "Record: armed, the first pass starts on the next bar line ({})",
                bars_text(s.bars)
            ),
            (Command::Record, Phase::Armed) => "Record again: disarmed".to_string(),
            (Command::Record, Phase::Recording) => {
                "Record during the first pass: it ends on the next bar line".to_string()
            }
            (Command::Record | Command::Overdub, Phase::Playing) if s.printed => {
                "overdub on, once the loop is fitted to the tempo (its print becomes its layer; \
                 Undo takes the fit back)"
                    .to_string()
            }
            (Command::Record | Command::Overdub, Phase::Playing) => {
                format!("overdub on (layer {})", s.layers + 1)
            }
            (Command::Record | Command::Overdub, Phase::Overdubbing) => "overdub off".to_string(),
            (Command::Overdub, Phase::Recording) if !s.overdub_next => {
                "Overdub during the first pass: the overdub \
                                                                         starts where it ends"
                    .to_string()
            }
            (Command::Overdub, Phase::Recording) => {
                "Overdub again: no overdub after the first pass".to_string()
            }
            (Command::Overdub, _) => format!("Overdub: nothing to overdub ({:?})", s.phase),
            (Command::Undo, Phase::Armed | Phase::Recording) => {
                "Undo: first pass thrown away".to_string()
            }
            (Command::Undo, Phase::Overdubbing) => {
                "Undo: the overdub in progress fades out".to_string()
            }
            (Command::Undo, Phase::Playing) if s.layers > 1 => {
                format!("Undo: layer {} fades out", s.layers)
            }
            (Command::Undo, _) => {
                "Undo: nothing to undo (Clear removes the first pass)".to_string()
            }
            (Command::Redo, Phase::Playing) if s.redo > 0 => {
                format!("Redo: layer {} fades back in", s.layers + 1)
            }
            (Command::Redo, _) => "Redo: nothing to redo".to_string(),
            (Command::Clear, Phase::Empty) => "Clear: nothing to clear".to_string(),
            (Command::Clear, _) => format!("Clear: the loop fades out ({} layers)", s.layers),
            (Command::Mute(m), _) => {
                if m {
                    "muted".to_string()
                } else {
                    "unmuted".to_string()
                }
            }
            (Command::SetBars(b), _) => format!("first pass length {}", bars_text(b)),
            (Command::SetAfter(a), _) => format!("after recording: {a:?}"),
            (Command::SetFadeOld(k), _) => format!("fade old: {:.0} % kept a pass", 100.0 * k),
            (Command::SetLatency(l), _) => {
                format!("latency {l} frames ({:.1} ms)", l as f64 / 44.1)
            }
            (Command::SetLevel(v), _) => format!("loop level {:.1} dB", 20.0 * v.max(1e-6).log10()),
            (Command::Edit(e), Phase::Playing) => format!("edit {e:?}"),
            (Command::Edit(e), _) => format!("edit {e:?}: waits, or is refused while recording"),
            (Command::Merge, Phase::Playing) => {
                format!("merge {} layers into one, in the background", s.layers)
            }
            (Command::Merge, _) => "merge: refused while recording".to_string(),
            (Command::Resample, Phase::Empty) => {
                "Resample: armed, the first pass records the other loops".to_string()
            }
            (Command::Resample, Phase::Armed | Phase::Recording) if s.resampling => {
                "Resample again: as Record".to_string()
            }
            (Command::Resample, _) => "Resample: only on an empty loop".to_string(),
            (Command::Fit, Phase::Playing) if s.printed => format!(
                "fit to the tempo: the print becomes the loop's one layer ({} layers before; Undo takes it back)",
                s.layers
            ),
            (Command::Fit, _) => "fit: refused, no print plays".to_string(),
        };
        if !self.looper.command(c) {
            return format!("looper: {what}: NOT done, the command queue is full");
        }
        format!("looper: {what}")
    }

    /// Pick loop `n` for the presses and controls after this, with its
    /// `looper:` line for the log ("loop 2 picked (empty)").
    pub fn select(&mut self, n: usize) -> String {
        let n = n.min(MAX_LOOPS - 1);
        let what = match self.looper.loop_status(n) {
            Some(s) if s.phase == Phase::Empty => "empty".to_string(),
            Some(s) => format!(
                "{} layer{}, {:?}",
                s.layers,
                if s.layers == 1 { "" } else { "s" },
                s.phase
            ),
            None => "not there".to_string(),
        };
        if !self.looper.select_loop(n) {
            return format!(
                "looper: loop {} ({what}): NOT picked, the command queue is full",
                n + 1
            );
        }
        format!("looper: loop {} picked ({what})", n + 1)
    }

    /// A layer's control, by the layer's id, with its `looper:` line for
    /// the log ("layer 3 muted").
    pub fn layer_command(&mut self, id: u8, c: LayerCommand) -> String {
        let k = (0..MAX_LAYERS)
            .find(|&k| self.looper.layer_info(k).is_some_and(|l| l.id == id))
            .map_or_else(|| format!("layer id {id}"), |k| format!("layer {}", k + 1));
        let what = match c {
            LayerCommand::Mute(true) => "muted".to_string(),
            LayerCommand::Mute(false) => "unmuted".to_string(),
            LayerCommand::Solo(true) => "soloed".to_string(),
            LayerCommand::Solo(false) => "solo off".to_string(),
            LayerCommand::Level(v) => format!("level {:.1} dB", 20.0 * v.max(1e-6).log10()),
            LayerCommand::Pan(p) => format!("pan {p:+.2}"),
            LayerCommand::Delete => "deleted (Undo brings it back)".to_string(),
        };
        if !self.looper.layer_command(id, c) {
            return format!("looper: {k} {what}: NOT done, the command queue is full");
        }
        format!("looper: {k} {what}")
    }

    /// The pad player's sound for this period (the drums and the synth as
    /// the engines make them, before the mixer): what the looper records
    /// from [`LoopSource::Pads`]. Call it before [`LoopHost::run`] each
    /// period.
    pub fn take_pads(&mut self, left: &[f32; PERIOD], right: &[f32; PERIOD]) {
        self.pads_l = *left;
        self.pads_r = *right;
    }

    /// One period for the mixer: read the line input, run the looper on the
    /// transport's `block` with the source's sound, and write the loop's
    /// sound into `left` and `right` (the Loop track). The line input of
    /// the period is [`LoopHost::input`]: the mixer's input strip hears it,
    /// following [`LoopHost::monitor`]. A note comes back if the input had
    /// a problem.
    pub fn run(
        &mut self,
        block: &Block,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> Option<String> {
        let note = self.record(block);
        for f in 0..PERIOD {
            left[f] = self.out[2 * f];
            right[f] = self.out[2 * f + 1];
        }
        note
    }

    /// The line input of the last period: this period's only after
    /// [`LoopHost::run`] (or [`LoopHost::period`]) has run; zeros while the
    /// line input is closed; and a period with too little input arrived is
    /// padded with silence at its end ([`LineStats`]' short count says
    /// when), so a gap can reach whatever records it (the sampler's input
    /// buffer, too).
    pub fn input(&self) -> (&[f32; PERIOD], &[f32; PERIOD]) {
        (&self.input_l, &self.input_r)
    }

    /// One period without the mixer (the screen's samples): as
    /// [`LoopHost::run`], then the loop (and the line input, with Monitor
    /// on) added to the output, kept within ±1. Returns the output's peak,
    /// and a note for the log if the input had a problem.
    pub fn period(
        &mut self,
        block: &Block,
        left: &mut [f32; PERIOD],
        right: &mut [f32; PERIOD],
    ) -> (f32, Option<String>) {
        let note = self.record(block);
        let mut peak = 0f32;
        let mut over = false;
        let mon = if self.monitor { 1.0 } else { 0.0 };
        for f in 0..PERIOD {
            let l = left[f] + self.out[2 * f] + mon * self.input_l[f];
            let r = right[f] + self.out[2 * f + 1] + mon * self.input_r[f];
            over |= l.abs() > 1.0 || r.abs() > 1.0;
            left[f] = l.clamp(-1.0, 1.0);
            right[f] = r.clamp(-1.0, 1.0);
            peak = peak.max(left[f].abs()).max(right[f].abs());
        }
        if over {
            self.clipped += 1;
        }
        (peak, note)
    }

    /// Reads the line input and runs the looper into `self.out`.
    fn record(&mut self, block: &Block) -> Option<String> {
        let note = self.line.period(&mut self.input_l, &mut self.input_r);
        let (src_l, src_r) = match self.source {
            LoopSource::Pads => (&self.pads_l, &self.pads_r),
            LoopSource::Line => (&self.input_l, &self.input_r),
        };
        for f in 0..PERIOD {
            self.input[2 * f] = src_l[f];
            self.input[2 * f + 1] = src_r[f];
        }
        for (m, x) in self.meter.iter_mut().zip([src_l, src_r]) {
            *m = x.iter().fold(*m, |a, v| a.max(v.abs()));
        }
        // A period nobody handed the pads' sound to records silence.
        self.pads_l = [0.0; PERIOD];
        self.pads_r = [0.0; PERIOD];
        if let Some(span) = block.spans().first() {
            self.beats_per_bar = u32::from(span.meter.beats);
        }
        self.out.fill(0.0);
        self.looper.process(block, &self.input, &mut self.out);
        if let Some(span) = block.spans().last() {
            self.printer.tend(&mut self.looper, span.bpm);
        }
        self.tend_save();
        note
    }

    /// Lines for the log when the looper's phase or layers changed since
    /// the last call; `at` gives the song position as text. Nothing is
    /// allocated when nothing changed.
    pub fn changes(&mut self, at: impl FnOnce() -> String) -> Vec<String> {
        let s = self.looper.status();
        let was = self.last;
        self.last = s;
        let mut lines = Vec::new();
        if s.phase == was.phase && s.layers == was.layers && s.redo == was.redo {
            return lines;
        }
        let at = at();
        if s.phase != was.phase {
            lines.push(match s.phase {
                Phase::Empty if was.phase == Phase::Recording => format!("looper: first pass thrown away at {at} (the \
                                                                           transport stopped or jumped)"),
                Phase::Empty => format!("looper: empty at {at}"),
                Phase::Armed => format!("looper: armed at {at}"),
                Phase::Recording => format!("looper: recording the first pass from {at} ({})", bars_text(s.bars)),
                Phase::Playing if was.phase == Phase::Recording => {
                    format!("looper: loop of {} at {at}: {} frames ({:.2} s), playing", ticks_text(s.ticks, self.beats_per_bar),
                            s.frames, s.frames as f64 / 44_100.0)
                }
                Phase::Playing => format!("looper: playing at {at}, {} layers", s.layers),
                Phase::Overdubbing => format!("looper: overdubbing layer {} from {at}", s.layers),
            });
        } else if s.layers != was.layers || s.redo != was.redo {
            lines.push(format!(
                "looper: {} layers, {} to redo, at {at}",
                s.layers, s.redo
            ));
        }
        lines
    }

    /// The `status: looper: ...` line.
    pub fn describe(&self) -> String {
        let s = self.looper.status();
        let st = self.looper.stats();
        format!("looper: {:?}, {} layers ({} to redo), loop {}, first pass length {}, latency {} frames, level {:.1} \
                 dB{}; records from {}; line input {}{}, monitor {}; {} overdubs refused, {} first passes thrown \
                 away, {} commands dropped, {} periods kept within ±1; {}prints {} made, {} too late, {} failed",
                s.phase, s.layers, s.redo,
                if s.frames > 0 { format!("{} frames ({:.2} s)", s.frames, s.frames as f64 / 44_100.0) } else { "none".to_string() },
                bars_text(s.bars), s.latency, 20.0 * s.level.max(1e-6).log10(), if s.muted { ", muted" } else { "" },
                self.source.name().to_lowercase(), if self.line.is_open() { "on" } else { "off" },
                if self.line.is_open() || self.line.stats.periods > 0 { format!(" ({})", self.line.describe()) } else { String::new() },
                if self.monitor { "on" } else { "off" }, st.refused, st.aborted, st.dropped_commands, self.clipped,
                if s.printed { "a print plays (pitch kept at this tempo); " } else { "" },
                self.printer.made, self.printer.stale, self.printer.failed)
    }

    /// What the Looper tab shows; takes the meter's peaks.
    pub fn view(&mut self, bpm: f64) -> LooperView {
        let m = std::mem::take(&mut self.meter);
        let db = |v: f32| {
            if v > 0.0 {
                20.0 * v.log10()
            } else {
                f32::NEG_INFINITY
            }
        };
        let s = self.looper.status();
        // A beat at the tempo the loop was recorded at, from its length.
        let frames_per_beat = if s.ticks > 0 {
            s.frames as f64 * 960.0 / s.ticks as f64
        } else {
            44_100.0 * 60.0 / bpm.max(1.0)
        };
        let mut layers = [None; MAX_LAYERS];
        for (k, l) in layers.iter_mut().enumerate() {
            *l = self.looper.layer_info(k);
        }
        let mut loops = [s; MAX_LOOPS];
        for (n, l) in loops.iter_mut().enumerate() {
            if let Some(st) = self.looper.loop_status(n) {
                *l = st;
            }
        }
        LooperView {
            status: s,
            stats: self.looper.stats(),
            source: self.source,
            line_on: self.line.is_open(),
            line: self.line.stats,
            monitor: self.monitor,
            meter_db: [db(m[0]), db(m[1])],
            beats_per_bar: self.beats_per_bar,
            frames_per_beat,
            layers,
            loops,
            selected: self.looper.selected(),
            memory: self.looper.memory(),
            rate: self.rate,
        }
    }

    /// One slice of the loop's waveform; the finished wave when it is
    /// done. The wave starts again when the loop, its layers or the phase
    /// change, and every `refresh` periods while overdubbing.
    pub fn wave_step(&mut self, period: u64, refresh: u64) -> Option<Arc<LoopWave>> {
        let s = self.looper.status();
        let of = (s.frames, s.layers, s.phase, self.looper.selected());
        let overdubbing =
            s.phase == Phase::Overdubbing && refresh > 0 && period.is_multiple_of(refresh);
        if of != self.wave.of || overdubbing && !self.wave.building {
            if s.frames == 0 {
                let was_empty = self.wave.of.0 == 0;
                self.wave.start(0, of);
                return (!was_empty).then(|| Arc::new(LoopWave::default()));
            }
            self.wave.start(s.frames, of);
        }
        self.wave.step(&self.looper, s.layers).map(Arc::new)
    }
}

pub fn bars_text(b: Option<u32>) -> String {
    match b {
        Some(1) => "1 bar".to_string(),
        Some(n) => format!("{n} bars"),
        None => "free (until Record again)".to_string(),
    }
}

fn ticks_text(ticks: i64, beats_per_bar: u32) -> String {
    let bar = 960 * i64::from(beats_per_bar.max(1));
    if ticks % bar == 0 {
        bars_text(Some((ticks / bar) as u32))
    } else {
        format!("{:.2} bars", ticks as f64 / bar as f64)
    }
}

/// The latency to make up for when nothing has been measured: the output
/// buffer (`periods` of 128 frames) plus one period of input. The
/// converters add an unknown amount; measure it with a cable from out to in.
pub fn estimated_latency(periods: u32) -> u32 {
    (periods + 1) * PERIOD as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_transport::{Command as T, Transport};

    fn s32(v: f32) -> i32 {
        (v * 2_147_483_647.0) as i32
    }

    #[test]
    fn a_closed_input_gives_silence_and_counts_nothing() {
        let mut l = LineIn::closed();
        let (mut a, mut b) = ([1.0; PERIOD], [1.0; PERIOD]);
        assert!(l.period(&mut a, &mut b).is_none());
        assert!(a.iter().chain(b.iter()).all(|&v| v == 0.0));
        assert_eq!(l.stats, LineStats::default());
        assert_eq!(l.close(), "line input: already off");
    }

    #[test]
    fn opening_a_device_that_is_not_there_says_so_and_stays_off() {
        let mut l = LineIn::closed();
        let e = l.open(99).unwrap_err();
        assert!(
            e.contains("/dev/snd/pcmC99D0c") && e.contains("stays off"),
            "{e}"
        );
        assert!(!l.is_open());
    }

    #[test]
    fn the_fifo_hands_out_whole_periods_and_drops_what_runs_ahead() {
        let mut l = LineIn::closed();
        // Pretend it is open: drive the FIFO directly.
        let frames: Vec<i32> = (0..3 * PERIOD)
            .flat_map(|f| [s32(f as f32 / 1000.0), s32(-0.5)])
            .collect();
        l.push_s32(&frames);
        assert_eq!(l.len, 3 * PERIOD);
        let (mut a, mut b) = ([0.0; PERIOD], [0.0; PERIOD]);
        // `period` reads only while open, so take periods by hand as it does.
        let take = |l: &mut LineIn, a: &mut [f32; PERIOD], b: &mut [f32; PERIOD]| {
            for f in 0..PERIOD {
                let at = (l.head + f) % FIFO;
                a[f] = l.fifo[2 * at];
                b[f] = l.fifo[2 * at + 1];
            }
            l.head = (l.head + PERIOD) % FIFO;
            l.len -= PERIOD;
        };
        take(&mut l, &mut a, &mut b);
        assert!((a[5] - 0.005).abs() < 1e-6 && (b[0] + 0.5).abs() < 1e-6);
        // Overfilling drops the oldest frames, counted.
        let lots: Vec<i32> = vec![s32(0.25); 2 * FIFO];
        l.push_s32(&lots);
        assert_eq!(l.len, FIFO);
        assert_eq!(l.stats.dropped, 2 * PERIOD as u64);
        take(&mut l, &mut a, &mut b);
        assert!((a[0] - 0.25).abs() < 1e-6);
    }

    #[test]
    fn the_wave_covers_the_loop_and_answers_any_zoom() {
        let w = LoopWave {
            layers: Vec::new(),
            frames: 1000,
            columns: (0..10)
                .map(|i| {
                    let v = i as f32 / 10.0;
                    [
                        WaveColumn {
                            min: -v,
                            max: v,
                            rms: v / 2.0,
                        },
                        WaveColumn::default(),
                    ]
                })
                .collect(),
        };
        let c = w.column(0, 0.0, 1000.0);
        assert_eq!((c.min, c.max), (-0.9, 0.9));
        let c = w.column(0, 520.0, 530.0);
        assert_eq!(c.max, 0.5);
        assert_eq!(w.column(0, 2000.0, 3000.0), WaveColumn::default());
        assert_eq!(w.column(2, 0.0, 10.0), WaveColumn::default());
    }

    /// The looper on a real transport with the input closed: presses log,
    /// the phases log, and the waveform gets built.
    #[test]
    fn presses_and_changes_log_and_the_wave_is_built() {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.monitor = true;
        assert_eq!(
            h.command(Command::SetBars(Some(1))),
            "looper: first pass length 1 bar"
        );
        assert!(h
            .command(Command::Record)
            .starts_with("looper: Record: armed"));
        t.command(0, T::Play);
        let mut lines = Vec::new();
        let mut wave = None;
        for p in 0..1200u64 {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            let (peak, note) = h.period(&b, &mut l, &mut r);
            assert!(note.is_none() && peak == 0.0, "the input is off: silence");
            lines.extend(h.changes(|| format!("period {p}")));
            if let Some(w) = h.wave_step(p, 0) {
                wave = Some(w);
            }
        }
        // Armed and recording in the same period: the bar line was at once.
        assert_eq!(
            lines[0],
            "looper: recording the first pass from period 0 (1 bar)"
        );
        assert!(
            lines[1].starts_with("looper: loop of 1 bar at period 689: 88200 frames"),
            "{lines:?}"
        );
        let w = wave.expect("the wave of the (silent) loop");
        assert_eq!((w.frames, w.columns.len()), (88_200, WAVE_COLUMNS));
        let v = h.view(120.0);
        assert_eq!(
            (v.status.phase, v.status.layers, v.line_on),
            (Phase::Playing, 1, false)
        );
        assert!((v.frames_per_beat - 22_050.0).abs() < 1e-6);
        assert!(h.command(Command::Undo).contains("nothing to undo"));
        assert!(h.command(Command::Overdub).contains("overdub on (layer 2)"));
        assert!(
            h.describe().starts_with("looper: Playing, 1 layers"),
            "{}",
            h.describe()
        );
        assert!(h.describe().contains("line input off, monitor on"));
    }

    /// A layer's speaker: the mute goes to the layer by its id, the log
    /// names it, and the next view shows it muted and not heard.
    #[test]
    fn a_layer_mutes_by_its_id_and_the_view_shows_it() {
        let mut h = LoopHost::new(44_100, 8);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::SetLatency(0));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut run = |h: &mut LoopHost, periods: u64| {
            for _ in 0..periods {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.period(&b, &mut l, &mut r);
            }
        };
        run(&mut h, 700);
        assert!(h.command(Command::Overdub).contains("layer 2"));
        run(&mut h, 4);
        h.command(Command::Overdub);
        run(&mut h, 2);
        let v = h.view(120.0);
        assert_eq!(v.status.layers, 2);
        let second = v.layers[1].expect("two layers");
        assert!(v.layers[2].is_none());
        assert!(!second.muted && second.audible);
        assert_eq!(
            h.layer_command(second.id, LayerCommand::Mute(true)),
            "looper: layer 2 muted"
        );
        run(&mut h, 1);
        let v = h.view(120.0);
        let (first, second) = (v.layers[0].unwrap(), v.layers[1].unwrap());
        assert!(second.muted && !second.audible);
        assert!(!first.muted && first.audible);
        assert_eq!(
            h.layer_command(second.id, LayerCommand::Mute(false)),
            "looper: layer 2 unmuted"
        );
        run(&mut h, 1);
        assert!(h.view(120.0).layers[1].unwrap().audible);
    }

    /// The pads' sound for song frame `n`: different every frame.
    fn pads_at(n: u64) -> (f32, f32) {
        ((n % 1000) as f32 / 2000.0, -((n % 777) as f32) / 2000.0)
    }

    /// Recording the pads: the loop plays back what the pads played in the
    /// first bar, to the frame, in the bars after it. With the source on
    /// the line input (closed), the same pads record silence.
    #[test]
    fn the_pads_are_recorded_in_time_and_the_line_source_ignores_them() {
        for source in LoopSource::ALL {
            let mut h = LoopHost::new(44_100, 20);
            h.source = source;
            let mut t = Transport::new(44_100, 120.0).unwrap();
            let mut b = Block::new();
            h.command(Command::SetBars(Some(1)));
            h.command(Command::SetLatency(0));
            h.command(Command::Record);
            t.command(0, T::Play);
            const BAR: u64 = 88_200;
            let mut checked = 0;
            for p in 0..(2 * BAR / PERIOD as u64 + 2) {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for f in 0..PERIOD {
                    let n = p * PERIOD as u64 + f as u64;
                    if n < BAR {
                        (l[f], r[f]) = pads_at(n);
                    }
                }
                h.take_pads(&l, &r);
                h.period(&b, &mut l, &mut r);
                // The second bar, away from the loop point's fades.
                for f in 0..PERIOD {
                    let n = p * PERIOD as u64 + f as u64;
                    if (BAR + 1_000..2 * BAR - 1_000).contains(&n) {
                        let want = if source == LoopSource::Pads {
                            pads_at(n - BAR)
                        } else {
                            (0.0, 0.0)
                        };
                        assert!(
                            (l[f] - want.0).abs() < 1e-6 && (r[f] - want.1).abs() < 1e-6,
                            "{source:?} frame {n}: {:?}, want {want:?}",
                            (l[f], r[f])
                        );
                        checked += 1;
                    }
                }
            }
            assert_eq!(checked, BAR - 2_000);
            assert_eq!(h.looper.status().phase, Phase::Playing);
            assert!(h
                .describe()
                .contains(&format!("records from {}", source.name().to_lowercase())));
        }
    }

    /// The audio thread's part: handing over the pads, a period, the log
    /// check and the wave step allocate nothing while nothing changes.
    /// A loop recorded at 120 BPM, then the tempo at 100: it plays like
    /// tape (lower) until the worker's print is in, then at its own pitch
    /// and the new length; Overdub then fits it to the tempo.
    #[test]
    fn a_loop_at_another_tempo_gets_a_print_and_keeps_its_pitch() {
        let mut h = LoopHost::new(44_100, 30);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        // 441 Hz: 100 frames a cycle, 882 to the bar.
        let sine = |f: u64| 0.5 * (f as f64 * std::f64::consts::TAU / 100.0).sin() as f32;
        let mut f = 0u64;
        let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, rec: bool| {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            if rec {
                for i in 0..PERIOD {
                    l[i] = sine(f + i as u64);
                    r[i] = l[i];
                }
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            f += PERIOD as u64;
            l
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b, true);
        }
        assert_eq!(h.looper.status().frames, 88_200);
        t.command(t.frame(), T::SetTempo(100.0));
        // The pitch, from the first and last upward zero crossing.
        let ups = |out: &[f32]| {
            let x: Vec<usize> = (1..out.len())
                .filter(|&i| out[i - 1] < 0.0 && out[i] >= 0.0)
                .collect();
            (x.len() - 1) as f64 * 44_100.0 / (x[x.len() - 1] - x[0]) as f64
        };
        // Like tape at first, once the tempo has changed (on the next 16th,
        // up to 5,513 frames on).
        for _ in 0..50 {
            period(&mut h, &mut t, &mut b, false);
        }
        let mut out = Vec::new();
        for _ in 0..30 {
            out.extend(period(&mut h, &mut t, &mut b, false));
        }
        assert!(!h.looper.status().printed);
        let tape = ups(&out);
        assert!((tape - 367.5).abs() < 2.0, "tape {tape}");
        let start = std::time::Instant::now();
        while !h.looper.status().printed {
            assert!(
                start.elapsed().as_secs() < 120,
                "no print: {}",
                h.describe()
            );
            period(&mut h, &mut t, &mut b, false);
            std::thread::sleep(std::time::Duration::from_micros(200));
        }
        let mut out = Vec::new();
        for _ in 0..400 {
            out.extend(period(&mut h, &mut t, &mut b, false));
        }
        let kept = ups(&out[1_000..]);
        assert!((kept - 441.0).abs() < 2.0, "print {kept}");
        assert!(h.describe().contains("a print plays"), "{}", h.describe());
        assert!(h.describe().contains("prints 1 made"), "{}", h.describe());
        // Overdub fits the loop to the tempo first.
        assert!(h
            .command(Command::Overdub)
            .contains("once the loop is fitted"));
        for _ in 0..40 {
            period(&mut h, &mut t, &mut b, false);
        }
        let s = h.looper.status();
        assert_eq!((s.phase, s.frames), (Phase::Overdubbing, 105_840));
    }

    #[test]
    fn a_steady_period_never_allocates() {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let (mut l, mut r) = ([0.25; PERIOD], [-0.25; PERIOD]);
        // Record the bar and build its wave first.
        for p in 0..1_400u64 {
            assert!(t.process(PERIOD as u32, &mut b));
            h.take_pads(&l, &r);
            h.period(&b, &mut l, &mut r);
            h.changes(|| format!("period {p}"));
            h.wave_step(p, 0);
        }
        assert_eq!(h.looper.status().phase, Phase::Playing);
        let ((), calls) = crate::sys::alloc_count::calls(|| {
            for p in 1_400..1_800u64 {
                t.process(PERIOD as u32, &mut b);
                h.take_pads(&l, &r);
                h.period(&b, &mut l, &mut r);
                assert!(h.changes(|| format!("period {p}")).is_empty());
                assert!(h.wave_step(p, 0).is_none());
                h.view(120.0);
            }
        });
        assert_eq!(calls, 0, "allocations or frees on the audio thread's path");
    }

    #[test]
    fn the_latency_estimate_is_the_output_buffer_and_one_period() {
        assert_eq!(estimated_latency(3), 512);
        assert_eq!(estimated_latency(2), 384);
    }

    /// A host with a one-bar loop of two layers (the second at a quarter
    /// level), playing at 120 BPM.
    fn saving_host() -> (LoopHost, Transport, Block) {
        let mut h = LoopHost::new(44_100, 20);
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let mut b = Block::new();
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut f = 0u64;
        let mut period = |h: &mut LoopHost, t: &mut Transport, b: &mut Block, hz: f64| {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for i in 0..PERIOD {
                l[i] = 0.3
                    * ((f + i as u64) as f64 * hz * std::f64::consts::TAU / 44_100.0).sin() as f32;
                r[i] = -l[i];
            }
            h.take_pads(&l, &r);
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            f += PERIOD as u64;
        };
        while h.looper.status().phase != Phase::Playing {
            period(&mut h, &mut t, &mut b, 441.0);
        }
        h.command(Command::Overdub);
        for _ in 0..200 {
            period(&mut h, &mut t, &mut b, 882.0);
        }
        h.command(Command::Overdub);
        for _ in 0..800 {
            period(&mut h, &mut t, &mut b, 0.0);
        }
        let id = h.looper.layer_info(1).unwrap().id;
        h.looper.layer_command(id, LayerCommand::Level(0.25));
        (h, t, b)
    }

    /// `periods` periods with no input: the output, interleaved.
    fn run_host(h: &mut LoopHost, t: &mut Transport, b: &mut Block, periods: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..periods {
            assert!(t.process(PERIOD as u32, b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.period(b, &mut l, &mut r);
            out.extend(l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]));
        }
        out
    }

    /// Run until the save is done.
    fn saved_from(h: &mut LoopHost, t: &mut Transport, b: &mut Block) -> Saved {
        let start = std::time::Instant::now();
        loop {
            run_host(h, t, b, 1);
            if let Some(s) = h.take_saved() {
                return s;
            }
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60, "the save did not finish");
        }
    }

    #[test]
    fn loops_saved_with_the_project_load_back_and_play_the_same() {
        use mpfree_looper::save::{from_text, from_wav, to_text, wav};
        let (mut h, mut t, mut b) = saving_host();
        assert_eq!(h.save(), "looper: saving the loops");
        let mut saved = saved_from(&mut h, &mut t, &mut b);
        assert!(h.saving().is_none());
        assert_eq!(saved.loops.len(), 1);
        assert_eq!(saved.loops[0].1.layers.len(), 2);
        assert_eq!(saved.loops[0].1.layers[1].level, 0.25);
        // As the project keeps them: the text, and a WAV a layer.
        let mut files = Vec::new();
        for (n, f) in &mut saved.loops {
            for (k, y) in f.layers.iter_mut().enumerate() {
                y.file = format!("loop {} layer {}.wav", *n + 1, k + 1);
                files.push((y.file.clone(), wav(&y.audio[..2 * f.frames], saved.rate)));
            }
        }
        let mut back = from_text(&to_text(&saved)).unwrap();
        for (_, f) in &mut back.loops {
            for y in &mut f.layers {
                let bytes = &files.iter().find(|(n, _)| *n == y.file).unwrap().1;
                let (audio, rate) = from_wav(bytes).unwrap();
                assert_eq!(rate, 44_100);
                y.audio = audio;
            }
        }
        for ((_, a), (_, c)) in saved.loops.iter().zip(&back.loops) {
            assert_eq!(a.frames, c.frames);
            for (x, y) in a.layers.iter().zip(&c.layers) {
                assert_eq!(x.audio[..2 * a.frames], y.audio[..]);
                assert_eq!((x.level, &x.file), (y.level, &y.file));
            }
        }
        // Into a fresh host at the same song frame: it plays as the first
        // one does, a bar on.
        let (mut h2, mut t2) = (
            LoopHost::new(44_100, 20),
            Transport::new(44_100, 120.0).unwrap(),
        );
        let mut b2 = Block::new();
        t2.command(0, T::Play);
        let skip = (t.frame() / PERIOD as u64) as usize;
        run_host(&mut h2, &mut t2, &mut b2, skip);
        assert_eq!(t2.frame(), t.frame());
        assert!(h2.load(back).starts_with("looper: loading loops 1"));
        let a = run_host(&mut h, &mut t, &mut b, 1500);
        let c = run_host(&mut h2, &mut t2, &mut b2, 1500);
        assert_eq!(h2.looper.status().phase, Phase::Playing);
        let n = a.len();
        let worst = (n - 2 * 88_200..n)
            .map(|i| (a[i] - c[i]).abs())
            .fold(0.0, f32::max);
        assert!(worst < 1e-5, "differs by {worst}");
    }

    #[test]
    fn a_loop_changing_while_it_is_read_starts_the_save_again() {
        let (mut h, mut t, mut b) = saving_host();
        h.save();
        // Wait for the reading to start, then change a layer's mix.
        let start = std::time::Instant::now();
        while h.saving() == Some(0.0) {
            run_host(&mut h, &mut t, &mut b, 1);
            std::thread::sleep(std::time::Duration::from_micros(100));
            assert!(start.elapsed().as_secs() < 60);
        }
        let id = h.looper.layer_info(0).unwrap().id;
        h.looper.layer_command(id, LayerCommand::Level(0.5));
        run_host(&mut h, &mut t, &mut b, 2);
        assert_eq!(h.save_restarts, 1);
        let saved = saved_from(&mut h, &mut t, &mut b);
        assert_eq!(saved.loops[0].1.layers[0].level, 0.5);
        // A load into a loop that is not empty keeps what it has.
        assert!(h.load(saved).contains("loops 1 were not empty"));
    }
}
