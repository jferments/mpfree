//! The transport: owns the audio thread's frame counter and turns it into
//! song position.
//!
//! Each audio period, [`Transport::process`] fills a [`Block`]: the period
//! cut into [`Span`]s, each with one state and one tempo, plus [`Marker`]s
//! for what happened (start, stop, loop jump, record on ...). Every engine
//! reads the same block, so they all agree on time.
//!
//! **Same output for any period size.** Commands carry the absolute frame
//! they take effect on, and every boundary (loop end, tempo change,
//! count-in end) is computed from absolute values, never by adding up
//! periods. Cutting the same session into 1-frame or 4096-frame periods
//! gives the same tick on the same frame. On one frame, commands always
//! apply before a boundary that falls there. Only a command that arrives
//! late (its frame already played) lands at the start of the next period,
//! and that is counted in [`Stats::late`].
//!
//! **No allocation, no locks, no panics** in `process` or `command`: fixed
//! arrays only. The transport lives on the audio thread; the control
//! thread hands it commands through a lock-free queue, and the audio thread
//! calls [`Transport::command`] for each before `process`.

use std::ops::Range;

use crate::time::{
    ceil_to, ticks_in, valid_bpm, Meter, Swing, TempoMap, Tick, MIN_SPAN_TICKS, PPQ,
};

/// Longest period `process` takes, in frames.
pub const MAX_PERIOD: u32 = 8192;
/// Sample rates the transport accepts. The lower bound keeps a tick longer
/// than a frame at the fastest tempo (3.4 frames at 22.05 kHz, 400 BPM), so
/// every tick has a frame of its own.
pub const MIN_RATE: u32 = 22_050;
pub const MAX_RATE: u32 = 192_000;
/// Commands waiting in the queue.
pub const MAX_COMMANDS: usize = 64;
/// Commands applied in one period; the rest wait for the next period.
pub const MAX_COMMANDS_PER_PERIOD: usize = 16;
/// Spans and markers one block can hold. Loops and tempo segments are at
/// least a 1/16 long, which is 827 frames at 400 BPM and 22.05 kHz, so an
/// 8192-frame period crosses at most about 20 boundaries; with 16 commands
/// that is under 40 spans and under 90 markers.
pub const MAX_SPANS: usize = 64;
pub const MAX_MARKERS: usize = 128;
/// Longest count-in, in bars.
pub const MAX_COUNT_IN: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    /// Counting in before recording: the clock runs (the metronome plays)
    /// but nothing is recorded or played back.
    CountIn,
    Playing,
    Recording,
}

impl State {
    /// True when the song position moves.
    pub fn rolling(self) -> bool {
        self != State::Stopped
    }
}

/// What the front panel, the screen or MIDI asks the transport to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Continue from the current position. Does nothing while rolling.
    Play,
    /// Play from bar 1. While rolling, jumps to bar 1.
    PlayStart,
    /// While rolling: stop where we are; sound keeps its tails. While
    /// stopped: go back to bar 1 and mark a panic (cut all sound).
    Stop,
    /// Stop (if rolling) and go back to bar 1 at once, without a panic.
    StopAndRewind,
    /// Stopped: count in (if set), then record from the current position.
    /// Playing: punch in. Recording: punch out (keep playing).
    Record,
    ToggleOverdub,
    /// Move the song position. While rolling, jumps there at once.
    Locate(Tick),
    /// Stopped: the whole song takes this tempo. Rolling: the tempo changes
    /// on the next tempo-grid line (a 1/16 by default), or at the loop jump
    /// if that comes first.
    SetTempo(f64),
    SetSwing(Swing),
    /// Loop the song between two ticks, or stop looping. The loop is at
    /// least a 1/16 long. A loop that ends before the playhead does not
    /// pull it back; it takes effect the next time the playhead is inside.
    SetLoop(Option<(Tick, Tick)>),
    /// Bars of count-in before recording from stopped (0 to 4).
    SetCountIn(u8),
}

/// What happened, and where in the period.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mark {
    /// Playback starts from `tick` (after a count-in, when it ends).
    Started {
        tick: Tick,
    },
    /// Stopped; `tick` is where playback resumes.
    Stopped {
        tick: Tick,
    },
    /// STOP while stopped: cut all sound.
    Panic,
    /// Back to bar 1 while stopped.
    Rewound,
    /// The count-in starts; recording starts at `record_at`.
    CountIn {
        record_at: Tick,
    },
    RecordOn {
        tick: Tick,
    },
    RecordOff {
        tick: Tick,
    },
    Overdub(bool),
    /// Jumped from `from` to `to` (a locate while rolling).
    Located {
        from: Tick,
        to: Tick,
    },
    /// The loop jumped back: ticks from `end` on were not played.
    Looped {
        end: Tick,
        start: Tick,
    },
    /// The tempo is now this, from this frame: a tempo segment, a live
    /// change, or a jump into another segment. Comes after the marker of
    /// the jump that caused it.
    Tempo(f64),
    Swing(Swing),
    LoopSet(Option<(Tick, Tick)>),
    CountInSet(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Marker {
    /// Frame offset in the period.
    pub offset: u32,
    /// Absolute frame.
    pub frame: u64,
    pub mark: Mark,
}

/// Frame of a (possibly fractional) tick, given an anchor. The one rule
/// that places every event.
fn frame_at(anchor_frame: u64, anchor_tick: Tick, bpm: f64, rate: u32, tick: f64) -> u64 {
    let f = (tick - anchor_tick as f64) * 60.0 * f64::from(rate) / (bpm * f64::from(PPQ));
    (anchor_frame as i64 + f.round() as i64).max(0) as u64
}

/// The first whole tick at or after the anchor whose frame is at or after
/// `frame`: the next tick to play.
fn first_tick_at(anchor_frame: u64, anchor_tick: Tick, bpm: f64, rate: u32, frame: u64) -> Tick {
    let at = |t: Tick| frame_at(anchor_frame, anchor_tick, bpm, rate, t as f64);
    let rel = frame as f64 - anchor_frame as f64 - 0.5;
    let mut t = (anchor_tick + ticks_in(rel, bpm, rate).ceil() as i64).max(anchor_tick);
    // The estimate is within a tick; settle it exactly.
    while at(t) < frame {
        t += 1;
    }
    while t > anchor_tick && at(t - 1) >= frame {
        t -= 1;
    }
    t
}

/// A stretch of one period with one state and one tempo. While rolling,
/// tick `t` falls on frame `anchor_frame + round(frames from anchor_tick to
/// t)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// Frame offset in the period, and length.
    pub offset: u32,
    pub len: u32,
    /// Absolute frame at `offset`.
    pub frame: u64,
    pub state: State,
    pub overdub: bool,
    /// While rolling: the tick that falls exactly on `anchor_frame`.
    /// Stopped: `anchor_tick` is the position and `anchor_frame` is 0.
    pub anchor_frame: u64,
    pub anchor_tick: Tick,
    pub bpm: f64,
    pub rate: u32,
    pub swing: Swing,
    pub meter: Meter,
}

impl Span {
    /// Absolute frame of a (possibly fractional, e.g. swung) tick, by this
    /// span's anchor and tempo. Valid for ticks inside the span; beyond it
    /// it is a forecast that holds only if nothing changes.
    pub fn frame_of(&self, tick: f64) -> u64 {
        frame_at(
            self.anchor_frame,
            self.anchor_tick,
            self.bpm,
            self.rate,
            tick,
        )
    }

    /// Offset in the period of a tick, if it falls in this span.
    pub fn offset_of(&self, tick: f64) -> Option<u32> {
        if !self.state.rolling() || tick < self.anchor_tick as f64 {
            return None;
        }
        let f = self.frame_of(tick);
        (f >= self.frame && f < self.frame + u64::from(self.len))
            .then(|| self.offset + (f - self.frame) as u32)
    }

    /// The whole ticks whose frame falls in this span. Empty when stopped.
    /// Spans that follow each other give ranges that follow each other, so
    /// each tick is played exactly once (until a loop jump or locate).
    pub fn ticks(&self) -> Range<Tick> {
        if !self.state.rolling() {
            return self.anchor_tick..self.anchor_tick;
        }
        let first = |f| first_tick_at(self.anchor_frame, self.anchor_tick, self.bpm, self.rate, f);
        first(self.frame)..first(self.frame + u64::from(self.len))
    }

    /// The song position at an absolute frame inside the span, with the
    /// fraction (for playheads and the screen).
    pub fn tick_at(&self, frame: u64) -> f64 {
        if !self.state.rolling() {
            return self.anchor_tick as f64;
        }
        self.anchor_tick as f64
            + ticks_in(frame as f64 - self.anchor_frame as f64, self.bpm, self.rate)
    }

    /// Every multiple of `step` ticks in the span, with its offset in the
    /// period: beats for the metronome (`meter.beat_ticks()`), bars for the
    /// looper, 1/16 lines for a step sequencer.
    pub fn lines(&self, step: Tick) -> Lines {
        let step = step.max(1);
        let r = self.ticks();
        Lines {
            span: *self,
            next: ceil_to(r.start, step),
            end: r.end,
            step,
        }
    }
}

/// Grid lines in a span; see [`Span::lines`].
pub struct Lines {
    span: Span,
    next: Tick,
    end: Tick,
    step: Tick,
}

impl Iterator for Lines {
    type Item = (Tick, u32);
    fn next(&mut self) -> Option<(Tick, u32)> {
        if self.next >= self.end {
            return None;
        }
        let t = self.next;
        self.next += self.step;
        let f = self.span.frame_of(t as f64);
        Some((t, self.span.offset + (f - self.span.frame) as u32))
    }
}

/// One period's worth of transport output. Make it once and reuse it.
#[derive(Clone, Debug)]
pub struct Block {
    pub start_frame: u64,
    pub frames: u32,
    spans: [Span; MAX_SPANS],
    n_spans: usize,
    markers: [Marker; MAX_MARKERS],
    n_markers: usize,
}

const EMPTY_SPAN: Span = Span {
    offset: 0,
    len: 0,
    frame: 0,
    state: State::Stopped,
    overdub: false,
    anchor_frame: 0,
    anchor_tick: 0,
    bpm: 120.0,
    rate: 44_100,
    swing: Swing {
        grid: PPQ / 4,
        amount: 50,
    },
    meter: Meter { beats: 4, unit: 4 },
};
const EMPTY_MARKER: Marker = Marker {
    offset: 0,
    frame: 0,
    mark: Mark::Panic,
};

impl Default for Block {
    fn default() -> Self {
        Block::new()
    }
}

impl Block {
    pub fn new() -> Block {
        Block {
            start_frame: 0,
            frames: 0,
            spans: [EMPTY_SPAN; MAX_SPANS],
            n_spans: 0,
            markers: [EMPTY_MARKER; MAX_MARKERS],
            n_markers: 0,
        }
    }

    /// The period's spans, in order, covering every frame once.
    pub fn spans(&self) -> &[Span] {
        &self.spans[..self.n_spans]
    }

    /// What happened, in order. A marker on a span's first frame happened
    /// just before that span.
    pub fn markers(&self) -> &[Marker] {
        &self.markers[..self.n_markers]
    }

    /// Start a copy of another block's spans, `start_frame` and `frames`
    /// as its own, to fill with [`Block::copy_span`]: for a host whose
    /// bookkeeping runs a little later on another thread, from the spans
    /// the audio thread sent it. The markers are not copied.
    pub fn start_copy(&mut self, start_frame: u64, frames: u32) {
        self.clear(start_frame, frames);
    }

    /// The copy's next span, as the other block has it. False if the
    /// block is full.
    pub fn copy_span(&mut self, s: Span) -> bool {
        if self.n_spans == MAX_SPANS {
            return false;
        }
        self.spans[self.n_spans] = s;
        self.n_spans += 1;
        true
    }

    fn clear(&mut self, start: u64, frames: u32) {
        self.start_frame = start;
        self.frames = frames;
        self.n_spans = 0;
        self.n_markers = 0;
    }

    fn push_span(&mut self, s: Span) -> bool {
        if let Some(last) = self.n_spans.checked_sub(1).map(|i| &mut self.spans[i]) {
            let same = Span {
                offset: last.offset,
                len: last.len,
                frame: last.frame,
                ..s
            } == *last;
            if same && last.offset + last.len == s.offset {
                last.len += s.len;
                return true;
            }
        }
        if self.n_spans == MAX_SPANS {
            return false;
        }
        self.spans[self.n_spans] = s;
        self.n_spans += 1;
        true
    }

    fn push_marker(&mut self, m: Marker) -> bool {
        if self.n_markers == MAX_MARKERS {
            return false;
        }
        self.markers[self.n_markers] = m;
        self.n_markers += 1;
        true
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Commands whose frame had already passed; applied at the start of the
    /// next period.
    pub late: u64,
    /// Commands refused because the queue was full.
    pub queue_full: u64,
    /// Commands with a bad value (tempo out of range, loop too short ...).
    pub rejected: u64,
    /// Periods that hit [`MAX_COMMANDS_PER_PERIOD`] and left due commands
    /// for the next one.
    pub deferred: u64,
    /// Spans or markers that did not fit in a block, or a period cut short
    /// by the loop guard. Should stay 0.
    pub block_full: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    SampleRate,
    Tempo,
    /// The tempo map and meter can only be replaced while stopped.
    Rolling,
}

/// Boundaries inside a period. Ties on one frame go in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    CountInEnd,
    Wrap,
    PendingTempo,
    Segment,
}

/// Steps one period may take (commands, crossings, spans) before the guard
/// ends it; far above what the limits above allow.
const MAX_STEPS: usize = 4 * (MAX_SPANS + MAX_MARKERS);

pub struct Transport {
    rate: u32,
    /// First frame of the next period: frames processed since start. It
    /// never resets; an xrun loses wall-clock time but not musical time.
    frame: u64,
    state: State,
    /// Where playback starts or resumes, while stopped.
    position: Tick,
    anchor_frame: u64,
    anchor_tick: Tick,
    bpm: f64,
    map: TempoMap,
    meter: Meter,
    swing: Swing,
    loop_range: Option<(Tick, Tick)>,
    count_in: u8,
    record_at: Tick,
    overdub: bool,
    tempo_grid: Tick,
    pending_tempo: Option<(Tick, f64)>,
    queue: [(u64, Command); MAX_COMMANDS],
    queued: usize,
    stats: Stats,
}

impl Transport {
    pub fn new(rate: u32, bpm: f64) -> Result<Transport, ConfigError> {
        if !(MIN_RATE..=MAX_RATE).contains(&rate) {
            return Err(ConfigError::SampleRate);
        }
        let map = TempoMap::constant(bpm).ok_or(ConfigError::Tempo)?;
        Ok(Transport {
            rate,
            frame: 0,
            state: State::Stopped,
            position: 0,
            anchor_frame: 0,
            anchor_tick: 0,
            bpm,
            map,
            meter: Meter::default(),
            swing: Swing::default(),
            loop_range: None,
            count_in: 0,
            record_at: 0,
            overdub: false,
            tempo_grid: MIN_SPAN_TICKS,
            pending_tempo: None,
            queue: [(0, Command::Play); MAX_COMMANDS],
            queued: 0,
            stats: Stats::default(),
        })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }
    /// First frame of the next period.
    pub fn frame(&self) -> u64 {
        self.frame
    }
    pub fn state(&self) -> State {
        self.state
    }
    /// The tempo now (at the position, while stopped).
    pub fn bpm(&self) -> f64 {
        self.bpm
    }
    pub fn meter(&self) -> Meter {
        self.meter
    }
    pub fn swing(&self) -> Swing {
        self.swing
    }
    pub fn loop_range(&self) -> Option<(Tick, Tick)> {
        self.loop_range
    }
    pub fn count_in(&self) -> u8 {
        self.count_in
    }
    pub fn overdub(&self) -> bool {
        self.overdub
    }
    pub fn tempo_map(&self) -> &TempoMap {
        &self.map
    }
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// The song position at the start of the next period.
    pub fn position(&self) -> Tick {
        self.tick_now(self.frame)
    }

    /// [`Transport::position`] with the fraction (for following another
    /// device's clock, where a tick is half a millisecond).
    pub fn position_exact(&self) -> f64 {
        if !self.state.rolling() {
            return self.position as f64;
        }
        self.anchor_tick as f64
            + ticks_in(
                self.frame as f64 - self.anchor_frame as f64,
                self.bpm,
                self.rate,
            )
    }

    /// Replace the tempo map, while stopped. Building a map is not real-time
    /// work; copying one in is.
    pub fn set_tempo_map(&mut self, map: TempoMap) -> Result<(), ConfigError> {
        if self.state.rolling() {
            return Err(ConfigError::Rolling);
        }
        self.map = map;
        self.bpm = map.bpm_at(self.position);
        Ok(())
    }

    /// Meter, while stopped (it moves every bar line).
    pub fn set_meter(&mut self, meter: Meter) -> Result<(), ConfigError> {
        if self.state.rolling() {
            return Err(ConfigError::Rolling);
        }
        self.meter = meter;
        Ok(())
    }

    /// Which grid a live tempo change waits for, in ticks (at least a 1/16).
    pub fn set_tempo_grid(&mut self, ticks: Tick) {
        self.tempo_grid = ticks.max(MIN_SPAN_TICKS);
    }

    /// Queue a command for absolute frame `at`. Commands on the same frame
    /// apply in the order given. Returns false if the queue is full.
    pub fn command(&mut self, at: u64, cmd: Command) -> bool {
        if self.queued == MAX_COMMANDS {
            self.stats.queue_full += 1;
            return false;
        }
        // Keep the queue sorted by frame; equal frames keep their order.
        let mut i = self.queued;
        while i > 0 && self.queue[i - 1].0 > at {
            self.queue[i] = self.queue[i - 1];
            i -= 1;
        }
        self.queue[i] = (at, cmd);
        self.queued += 1;
        true
    }

    /// Run one period of `frames` frames (1 to [`MAX_PERIOD`]) and describe
    /// it in `out`. Returns false, and does nothing, for a bad length.
    pub fn process(&mut self, frames: u32, out: &mut Block) -> bool {
        if frames == 0 || frames > MAX_PERIOD {
            return false;
        }
        let start = self.frame;
        let end = start + u64::from(frames);
        out.clear(start, frames);
        let mut at = start;
        let mut applied = 0usize;
        let mut steps = 0usize;
        let mut bpm = self.bpm;
        while at < end {
            steps += 1;
            if steps > MAX_STEPS {
                // Never reached with the limits above; if a bug ever loops
                // here, finish the period as it stands instead of hanging
                // the audio thread.
                self.stats.block_full += 1;
                let span = self.span(start, at, end);
                out.push_span(span);
                break;
            }
            // 1. Commands due by now, in order.
            while self.queued > 0 && self.queue[0].0 <= at && applied < MAX_COMMANDS_PER_PERIOD {
                let (when, cmd) = self.pop();
                if when < start {
                    self.stats.late += 1;
                }
                self.apply(at, cmd, out);
                self.note_tempo(at, &mut bpm, out);
                applied += 1;
            }
            // 2. Then a boundary on this frame.
            let boundary = self.next_boundary(at);
            if let Some((f, kind)) = boundary {
                if f <= at {
                    self.cross(at, kind, out);
                    self.note_tempo(at, &mut bpm, out);
                    continue;
                }
            }
            // 3. Play up to the next command, boundary or the period's end.
            let mut stop_at = end;
            if self.queued > 0 && applied < MAX_COMMANDS_PER_PERIOD {
                stop_at = stop_at.min(self.queue[0].0);
            }
            if let Some((f, _)) = boundary {
                stop_at = stop_at.min(f);
            }
            let span = self.span(start, at, stop_at);
            if !out.push_span(span) {
                self.stats.block_full += 1;
            }
            at = stop_at;
        }
        if applied == MAX_COMMANDS_PER_PERIOD && self.queued > 0 && self.queue[0].0 < end {
            self.stats.deferred += 1;
        }
        self.frame = end;
        true
    }

    fn pop(&mut self) -> (u64, Command) {
        let first = self.queue[0];
        self.queue.copy_within(1..self.queued, 0);
        self.queued -= 1;
        first
    }

    fn frame_of(&self, tick: Tick) -> u64 {
        frame_at(
            self.anchor_frame,
            self.anchor_tick,
            self.bpm,
            self.rate,
            tick as f64,
        )
    }

    /// The song position at absolute frame `f`: the first whole tick not
    /// yet played.
    fn tick_now(&self, f: u64) -> Tick {
        if !self.state.rolling() {
            return self.position;
        }
        first_tick_at(self.anchor_frame, self.anchor_tick, self.bpm, self.rate, f)
    }

    fn span(&self, start: u64, from: u64, to: u64) -> Span {
        let rolling = self.state.rolling();
        Span {
            offset: (from - start) as u32,
            len: (to - from) as u32,
            frame: from,
            state: self.state,
            overdub: self.overdub,
            anchor_frame: if rolling { self.anchor_frame } else { 0 },
            anchor_tick: if rolling {
                self.anchor_tick
            } else {
                self.position
            },
            bpm: self.bpm,
            rate: self.rate,
            swing: self.swing,
            meter: self.meter,
        }
    }

    /// One `Tempo` marker whenever the tempo changes, whatever changed it:
    /// a segment, a live change, a loop jump or locate into another segment.
    fn note_tempo(&mut self, at: u64, last: &mut f64, out: &mut Block) {
        if self.bpm != *last {
            *last = self.bpm;
            self.mark(out, at, Mark::Tempo(self.bpm));
        }
    }

    fn mark(&mut self, out: &mut Block, at: u64, mark: Mark) {
        let m = Marker {
            offset: (at - out.start_frame) as u32,
            frame: at,
            mark,
        };
        if !out.push_marker(m) {
            self.stats.block_full += 1;
        }
    }

    /// Start the clock at frame `f` with tick `tick` on it.
    fn anchor(&mut self, f: u64, tick: Tick) {
        self.anchor_frame = f;
        self.anchor_tick = tick;
        self.bpm = self.map.bpm_at(tick);
    }

    /// The whole song takes one tempo (a live or stopped tempo change).
    fn set_constant_tempo(&mut self, bpm: f64) {
        if let Some(map) = TempoMap::constant(bpm) {
            self.map = map;
            self.bpm = bpm;
        }
    }

    /// The next boundary while rolling, as (frame, kind), seen from frame
    /// `at`. Every candidate is at or before the next tempo segment, so the
    /// current tempo places it exactly.
    fn next_boundary(&self, at: u64) -> Option<(u64, Boundary)> {
        if !self.state.rolling() {
            return None;
        }
        let mut best: Option<(Tick, Boundary)> = None;
        let mut offer = |t: Tick, k: Boundary| {
            if best.is_none_or(|(bt, _)| t < bt) {
                best = Some((t, k));
            }
        };
        if self.state == State::CountIn {
            offer(self.record_at, Boundary::CountInEnd);
        } else if let Some((_, end)) = self.loop_range {
            // Wrap only while the playhead has not passed the loop's end.
            if self.tick_now(at) <= end {
                offer(end, Boundary::Wrap);
            }
        }
        if let Some((t, _)) = self.pending_tempo {
            offer(t, Boundary::PendingTempo);
        }
        if let Some(t) = self.map.next_change_after(self.anchor_tick) {
            offer(t, Boundary::Segment);
        }
        best.map(|(t, k)| (self.frame_of(t), k))
    }

    fn cross(&mut self, at: u64, kind: Boundary, out: &mut Block) {
        match kind {
            Boundary::CountInEnd => {
                let t = self.record_at;
                self.state = State::Recording;
                self.anchor(at, t);
                self.mark(out, at, Mark::Started { tick: t });
                self.mark(out, at, Mark::RecordOn { tick: t });
            }
            Boundary::Wrap => {
                if let Some((start, end)) = self.loop_range {
                    self.anchor(at, start);
                    self.mark(out, at, Mark::Looped { end, start });
                    if let Some((_, bpm)) = self.pending_tempo.take() {
                        self.live_tempo(at, start, bpm);
                    }
                }
            }
            Boundary::PendingTempo => {
                if let Some((t, bpm)) = self.pending_tempo.take() {
                    self.live_tempo(at, t, bpm);
                }
            }
            Boundary::Segment => {
                if let Some(t) = self.map.next_change_after(self.anchor_tick) {
                    self.anchor(at, t);
                }
            }
        }
    }

    /// A live tempo change: the whole song takes the new tempo from tick
    /// `t` (on frame `at`) on.
    fn live_tempo(&mut self, at: u64, t: Tick, bpm: f64) {
        self.set_constant_tempo(bpm);
        self.anchor(at, t);
    }

    fn apply(&mut self, at: u64, cmd: Command, out: &mut Block) {
        let now = self.tick_now(at);
        match cmd {
            Command::Play => {
                if self.state == State::Stopped {
                    self.state = State::Playing;
                    self.anchor(at, self.position);
                    self.mark(
                        out,
                        at,
                        Mark::Started {
                            tick: self.position,
                        },
                    );
                }
            }
            Command::PlayStart => match self.state {
                State::Stopped => {
                    self.position = 0;
                    self.apply(at, Command::Play, out);
                }
                State::Playing | State::Recording => self.locate(at, now, 0, out),
                State::CountIn => {}
            },
            Command::Stop | Command::StopAndRewind => {
                if self.state.rolling() {
                    let resume = if self.state == State::CountIn {
                        self.record_at
                    } else {
                        now
                    };
                    if self.state == State::Recording {
                        self.mark(out, at, Mark::RecordOff { tick: now });
                    }
                    self.state = State::Stopped;
                    self.position = resume;
                    if let Some((_, bpm)) = self.pending_tempo.take() {
                        self.set_constant_tempo(bpm);
                    }
                    self.bpm = self.map.bpm_at(resume);
                    self.mark(out, at, Mark::Stopped { tick: resume });
                    if cmd == Command::StopAndRewind {
                        self.rewind(at, out);
                    }
                } else {
                    self.rewind(at, out);
                    if cmd == Command::Stop {
                        self.mark(out, at, Mark::Panic);
                    }
                }
            }
            Command::Record => match self.state {
                State::Stopped if self.count_in > 0 => {
                    self.state = State::CountIn;
                    self.record_at = self.position;
                    let lead = i64::from(self.count_in) * self.meter.bar_ticks();
                    self.anchor(at, self.position - lead);
                    self.mark(
                        out,
                        at,
                        Mark::CountIn {
                            record_at: self.record_at,
                        },
                    );
                }
                State::Stopped => {
                    self.state = State::Recording;
                    self.anchor(at, self.position);
                    self.mark(
                        out,
                        at,
                        Mark::Started {
                            tick: self.position,
                        },
                    );
                    self.mark(
                        out,
                        at,
                        Mark::RecordOn {
                            tick: self.position,
                        },
                    );
                }
                State::Playing => {
                    self.state = State::Recording;
                    self.mark(out, at, Mark::RecordOn { tick: now });
                }
                State::Recording => {
                    self.state = State::Playing;
                    self.mark(out, at, Mark::RecordOff { tick: now });
                }
                State::CountIn => {}
            },
            Command::ToggleOverdub => {
                self.overdub = !self.overdub;
                self.mark(out, at, Mark::Overdub(self.overdub));
            }
            Command::Locate(t) => match self.state {
                State::Stopped => {
                    self.position = t;
                    self.bpm = self.map.bpm_at(t);
                }
                State::Playing | State::Recording => self.locate(at, now, t, out),
                State::CountIn => {
                    self.record_at = t;
                    let lead = i64::from(self.count_in) * self.meter.bar_ticks();
                    self.anchor(at, t - lead);
                    self.mark(out, at, Mark::CountIn { record_at: t });
                }
            },
            Command::SetTempo(bpm) => {
                if !valid_bpm(bpm) {
                    self.stats.rejected += 1;
                } else if self.state.rolling() {
                    self.pending_tempo = Some((ceil_to(now, self.tempo_grid), bpm));
                } else {
                    self.set_constant_tempo(bpm);
                }
            }
            Command::SetSwing(s) => {
                if Swing::new(s.grid, s.amount).is_some() {
                    self.swing = s;
                    self.mark(out, at, Mark::Swing(s));
                } else {
                    self.stats.rejected += 1;
                }
            }
            Command::SetLoop(range) => {
                if range.is_none_or(|(a, b)| b - a >= MIN_SPAN_TICKS) {
                    self.loop_range = range;
                    self.mark(out, at, Mark::LoopSet(range));
                } else {
                    self.stats.rejected += 1;
                }
            }
            Command::SetCountIn(bars) => {
                if bars <= MAX_COUNT_IN {
                    self.count_in = bars;
                    self.mark(out, at, Mark::CountInSet(bars));
                } else {
                    self.stats.rejected += 1;
                }
            }
        }
    }

    /// Jump while rolling. A tempo change still waiting takes effect here.
    fn locate(&mut self, at: u64, from: Tick, to: Tick, out: &mut Block) {
        if let Some((_, bpm)) = self.pending_tempo.take() {
            self.set_constant_tempo(bpm);
        }
        self.anchor(at, to);
        self.mark(out, at, Mark::Located { from, to });
    }

    fn rewind(&mut self, at: u64, out: &mut Block) {
        self.position = 0;
        self.bpm = self.map.bpm_at(0);
        self.mark(out, at, Mark::Rewound);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(t: &mut Transport, frames: u32) -> Block {
        let mut b = Block::new();
        assert!(t.process(frames, &mut b));
        b
    }

    fn marks(b: &Block) -> Vec<(u64, Mark)> {
        b.markers().iter().map(|m| (m.frame, m.mark)).collect()
    }

    #[test]
    fn beats_land_where_the_tempo_puts_them() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Play);
        let b = run(&mut t, 8192);
        let beats: Vec<_> = b.spans().iter().flat_map(|s| s.lines(960)).collect();
        assert_eq!(beats, vec![(0, 0)]);
        let mut b = Block::new();
        let mut beats = vec![];
        while t.frame() < 44_100 * 3 {
            t.process(512, &mut b);
            beats.extend(b.spans().iter().flat_map(|s| {
                s.lines(960)
                    .map(move |(k, o)| (k, s.frame - u64::from(s.offset) + u64::from(o)))
            }));
        }
        assert_eq!(
            &beats[..4],
            &[
                (960, 22_050),
                (1920, 44_100),
                (2880, 66_150),
                (3840, 88_200)
            ]
        );
    }

    #[test]
    fn a_count_in_ends_on_the_record_tick() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::SetCountIn(1));
        t.command(100, Command::Record);
        let b = run(&mut t, 4096);
        assert_eq!(
            b.markers()[1],
            Marker {
                offset: 100,
                frame: 100,
                mark: Mark::CountIn { record_at: 0 }
            }
        );
        assert_eq!(b.spans()[1].state, State::CountIn);
        assert_eq!(b.spans()[1].ticks().start, -3840);
        let mut b = Block::new();
        while t.state() == State::CountIn {
            t.process(4096, &mut b);
        }
        // One bar at 120 BPM is 2 s.
        assert_eq!(
            marks(&b),
            vec![
                (88_300, Mark::Started { tick: 0 }),
                (88_300, Mark::RecordOn { tick: 0 })
            ]
        );
        assert_eq!(t.state(), State::Recording);
    }

    #[test]
    fn the_loop_jumps_back_on_its_end() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::SetLoop(Some((0, 3840))));
        t.command(0, Command::Play);
        let mut b = Block::new();
        let mut loops = vec![];
        while t.frame() < 88_200 * 3 + 1 {
            t.process(1000, &mut b);
            loops.extend(
                marks(&b)
                    .into_iter()
                    .filter(|m| matches!(m.1, Mark::Looped { .. }))
                    .map(|m| m.0),
            );
        }
        assert_eq!(loops, vec![88_200, 176_400, 264_600]);
    }

    #[test]
    fn a_loop_behind_the_playhead_does_not_pull_it_back() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Locate(5000));
        t.command(0, Command::Play);
        t.command(10, Command::SetLoop(Some((0, 3840))));
        let mut b = Block::new();
        for _ in 0..100 {
            t.process(1024, &mut b);
            assert!(!marks(&b).iter().any(|m| matches!(m.1, Mark::Looped { .. })));
        }
        assert!(t.position() > 5000);
    }

    #[test]
    fn a_live_tempo_change_waits_for_the_next_sixteenth() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Play);
        // Tick 100 is at frame 2297; the next 1/16 is tick 240, frame 5513.
        t.command(2297, Command::SetTempo(90.0));
        let b = run(&mut t, 8192);
        assert_eq!(
            marks(&b),
            vec![(0, Mark::Started { tick: 0 }), (5513, Mark::Tempo(90.0))]
        );
        let last = b.spans().last().unwrap();
        assert_eq!(
            (last.anchor_tick, last.anchor_frame, last.bpm),
            (240, 5513, 90.0)
        );
        assert_eq!(t.tempo_map().segments().len(), 1);
    }

    #[test]
    fn stop_twice_rewinds_and_panics() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Play);
        t.command(1000, Command::Stop);
        t.command(2000, Command::Stop);
        let b = run(&mut t, 4096);
        assert_eq!(
            marks(&b)[1..],
            [
                (1000, Mark::Stopped { tick: 44 }),
                (2000, Mark::Rewound),
                (2000, Mark::Panic)
            ]
        );
        assert_eq!(t.position(), 0);
    }

    #[test]
    fn punch_in_and_out_while_playing() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Play);
        t.command(22_050, Command::Record);
        t.command(44_100, Command::Record);
        let mut b = Block::new();
        let mut all = vec![];
        while t.frame() < 50_000 {
            t.process(4096, &mut b);
            all.extend(marks(&b));
        }
        assert_eq!(
            all[1..],
            [
                (22_050, Mark::RecordOn { tick: 960 }),
                (44_100, Mark::RecordOff { tick: 1920 })
            ]
        );
        assert_eq!(t.state(), State::Playing);
    }

    #[test]
    fn locate_while_rolling_jumps_at_once() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Play);
        t.command(1000, Command::Locate(7680));
        let b = run(&mut t, 2000);
        assert_eq!(marks(&b)[1].1, Mark::Located { from: 44, to: 7680 });
        assert_eq!(b.spans()[1].ticks().start, 7680);
        assert_eq!(b.spans()[1].frame, 1000);
    }

    #[test]
    fn tempo_segments_change_the_tempo_on_their_tick() {
        use crate::time::Segment;
        let mut t = Transport::new(44_100, 120.0).unwrap();
        let map = TempoMap::new(&[
            Segment {
                start: 0,
                bpm: 120.0,
            },
            Segment {
                start: 960,
                bpm: 60.0,
            },
        ])
        .unwrap();
        t.set_tempo_map(map).unwrap();
        t.command(0, Command::Play);
        let mut b = Block::new();
        let mut beats = vec![];
        while t.frame() < 22_050 + 44_100 * 2 + 1 {
            t.process(777, &mut b);
            assert!(t.set_tempo_map(map).is_err());
            for s in b.spans() {
                for (k, o) in s.lines(960) {
                    beats.push((k, b.start_frame + u64::from(o), s.bpm));
                }
            }
        }
        assert_eq!(
            beats,
            vec![
                (0, 0, 120.0),
                (960, 22_050, 60.0),
                (1920, 66_150, 60.0),
                (2880, 110_250, 60.0)
            ]
        );
    }

    #[test]
    fn bad_values_are_counted_not_applied() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        assert_eq!(
            Transport::new(8000, 120.0).err(),
            Some(ConfigError::SampleRate)
        );
        assert_eq!(
            Transport::new(44_100, 500.0).err(),
            Some(ConfigError::Tempo)
        );
        t.command(0, Command::SetTempo(1000.0));
        t.command(0, Command::SetLoop(Some((0, 10))));
        t.command(0, Command::SetCountIn(9));
        t.command(
            0,
            Command::SetSwing(Swing {
                grid: 7,
                amount: 60,
            }),
        );
        let b = run(&mut t, 64);
        assert!(b.markers().is_empty());
        assert_eq!(t.stats().rejected, 4);
        assert!(!t.process(0, &mut Block::new()) && !t.process(MAX_PERIOD + 1, &mut Block::new()));
        for i in 0..MAX_COMMANDS as u64 {
            assert!(t.command(1000 + i, Command::ToggleOverdub));
        }
        assert!(!t.command(5000, Command::Play));
        assert_eq!(t.stats().queue_full, 1);
    }

    #[test]
    fn late_and_excess_commands_are_counted() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        run(&mut t, 128);
        t.command(10, Command::Play);
        let b = run(&mut t, 128);
        assert_eq!(marks(&b), vec![(128, Mark::Started { tick: 0 })]);
        assert_eq!(t.stats().late, 1);
        for _ in 0..20 {
            t.command(300, Command::ToggleOverdub);
        }
        let b = run(&mut t, 128);
        assert_eq!(b.markers().len(), MAX_COMMANDS_PER_PERIOD);
        assert_eq!(t.stats().deferred, 1);
        let b = run(&mut t, 128);
        assert_eq!(b.markers().len(), 4);
        assert_eq!(b.markers()[0].frame, 384);
    }

    /// A block rebuilt from another's spans, as a host on another thread
    /// rebuilds it, has the same spans; a full one refuses more.
    #[test]
    fn a_block_copied_span_by_span_has_the_same_spans() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Play);
        t.command(300, Command::SetTempo(140.0));
        let b = run(&mut t, 8192);
        assert!(b.spans().len() >= 2, "{:?}", b.spans());
        let mut copy = Block::new();
        copy.start_copy(b.start_frame, b.frames);
        for s in b.spans() {
            assert!(copy.copy_span(*s));
        }
        assert_eq!(copy.spans(), b.spans());
        assert_eq!((copy.start_frame, copy.frames), (b.start_frame, b.frames));
        assert!(copy.markers().is_empty());
        copy.start_copy(0, 128);
        assert!(copy.spans().is_empty());
        let s = b.spans()[0];
        assert!((0..MAX_SPANS).all(|_| copy.copy_span(s)));
        assert!(!copy.copy_span(s));
    }

    #[test]
    fn the_exact_position_has_the_fraction_the_position_rounds_up() {
        let mut t = Transport::new(44_100, 120.0).unwrap();
        t.command(0, Command::Locate(1000));
        run(&mut t, 64);
        assert_eq!(t.position_exact(), 1000.0, "stopped: where it stands");
        t.command(t.frame(), Command::Play);
        run(&mut t, 100);
        // 100 frames at 120 BPM: 100 * 1920 / 44100 ticks.
        let want = 1000.0 + 100.0 * 1920.0 / 44_100.0;
        assert!((t.position_exact() - want).abs() < 1e-9);
        assert_eq!(t.position(), want.ceil() as Tick);
    }
}
