//! mpfree-looper: the audio looper.
//!
//! One stereo loop on the transport, recorded from the line input:
//!
//! - **The first pass** starts on the next bar line after Record, and ends
//!   after the set number of bars, or (free length) on the bar line after
//!   Record is pressed again. Its length in frames is what was recorded;
//!   its place in the song is a whole number of bars from `origin`.
//! - **Playback** follows the song position: the loop's position is the
//!   song position's place in the loop, so it stays locked to the bars
//!   through stops, locates and transport loops. At the tempo it was
//!   recorded at it plays sample for sample; at another tempo it plays
//!   faster or slower, like tape (no time-stretch yet).
//! - **Overdubs** are layers on top of the first pass, recorded from the
//!   moment Overdub is pressed until it is pressed again, over as many
//!   passes as you like. Undo fades out and removes the newest layer; Redo
//!   brings it back, until a new overdub starts.
//! - **Each layer has its own level, pan, mute and solo**
//!   ([`Looper::layer_command`]), and can be deleted. A delete is undone
//!   and redone like an overdub: Undo and Redo walk one history of both.
//!   A layer keeps its id ([`LayerInfo::id`]) while it exists, whatever
//!   comes and goes around it.
//! - **No clicks:** the loop point is crossfaded with the input from just
//!   before the first pass, so the end flows into the start. Punch in and
//!   out, start, stop, mute, undo, delete, clear and every layer's level,
//!   pan, mute and solo fade over [`FADE`] frames, and
//!   a jump in the song position (a locate, a transport loop that is not a
//!   whole number of loops, a restart) crossfades from the old place to the
//!   new one. Commands that would cut a fade short wait for it to end (at
//!   most [`FADE`] frames).
//! - **Stopped:** the loop is silent while the transport is stopped or
//!   counting in. Overdub pressed while stopped waits for the transport to
//!   play; stopping ends an overdub (with a fade).
//! - **Latency:** the player hears the loop late (the output buffer) and
//!   their playing reaches the input late (the input buffer). Input that
//!   arrives `latency` frames after a moment belongs to that moment, so it
//!   is written that much earlier in the loop.
//!
//! The memory is allocated once, in [`Looper::new`]. [`Looper::command`]
//! and [`Looper::process`] never allocate, lock or panic, and give the same
//! output for any period size.

#![forbid(unsafe_code)]

use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Mark, Span, State};

/// Frames of every fade: the loop point, punch in and out, start, stop,
/// mute, undo and clear (about 5.8 ms at 44.1 kHz).
pub const FADE: usize = 256;
/// Layers at most: the first pass and 15 overdubs.
pub const MAX_LAYERS: usize = 16;
/// Commands waiting for the next period.
const QUEUE: usize = 32;
/// Overdubs and deletes Undo can take back.
const HISTORY: usize = 64;
const STEP: f32 = 1.0 / FADE as f32;
/// The fade-in of an overdub that starts where the first pass ends: the
/// input runs on without a break there, so a short fade is enough, and the
/// downbeat's attack is kept (about 0.7 ms at 44.1 kHz).
pub const HANDOFF_FADE: usize = 32;

/// What the looper's buttons and screen ask for. Each applies at the start
/// of the next period.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    /// Empty: arm, to record the first pass from the next bar line. Armed:
    /// cancel. Recording the first pass: end it on the next bar line.
    /// Playing: start an overdub. Overdubbing: end it.
    Record,
    /// Playing: start an overdub. Overdubbing: end it. Recording the first
    /// pass: go straight into an overdub when it ends (press again to
    /// cancel).
    Overdub,
    /// Remove the newest overdub (fading it out). While overdubbing, the
    /// overdub in progress. While recording the first pass, throw it away.
    Undo,
    /// Bring back the overdub undone last.
    Redo,
    /// Fade out and forget the loop.
    Clear,
    Mute(bool),
    /// Bars in the next first pass; None records until Record again.
    SetBars(Option<u32>),
    /// Input latency to make up for, in frames (output buffer plus input
    /// buffer plus converters; measure it with a cable from out to in).
    SetLatency(u32),
    /// The loop's level, 0 to 2 (1 is as recorded).
    SetLevel(f32),
}

/// What one layer's controls ask for ([`Looper::layer_command`]). Each
/// applies at the start of the next period, after the commands before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerCommand {
    /// 0 to 2 (1 is as recorded).
    Level(f32),
    /// -1 (left) to 1 (right); 0 is as recorded. A balance: the other side
    /// turns down, this one stays.
    Pan(f32),
    Mute(bool),
    /// While any layer is soloed, only soloed layers play.
    Solo(bool),
    /// Fade the layer out and take it away; Undo brings it back. The last
    /// layer stays (Clear removes the loop). Deleting the overdub being
    /// recorded ends it first.
    Delete,
}

/// One layer as the screen shows it, in play order (the first pass first).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayerInfo {
    /// The layer's id for [`Looper::layer_command`]: the same while the
    /// layer exists, through deletes, undo and redo around it.
    pub id: u8,
    pub level: f32,
    pub pan: f32,
    pub muted: bool,
    pub solo: bool,
    /// Heard: not muted and not left out by another layer's solo.
    pub audible: bool,
    /// Being recorded now.
    pub recording: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Empty,
    /// Waiting for the next bar line to record the first pass.
    Armed,
    /// Recording the first pass.
    Recording,
    Playing,
    Overdubbing,
}

/// Counted, never silent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Commands that did not fit the queue.
    pub dropped_commands: u64,
    /// Overdubs refused because every layer slot was in use.
    pub refused: u64,
    /// Layer commands for a layer that is not there.
    pub no_layer: u64,
    /// First passes thrown away: the transport stopped or jumped before
    /// the end, or the pass outgrew the memory.
    pub aborted: u64,
}

/// What the screen shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Status {
    pub phase: Phase,
    /// Layers playing (the first pass is layer 1).
    pub layers: usize,
    /// Layers Redo can bring back.
    pub redo: usize,
    /// The bars setting for the next first pass.
    pub bars: Option<u32>,
    /// The loop's length, once known (0 before).
    pub frames: usize,
    pub ticks: Tick,
    /// The song position where the loop starts.
    pub origin: Tick,
    /// Where the loop is, 0 to 1.
    pub position: f32,
    /// Free length: Record was pressed again, the pass ends on the next bar.
    pub ending: bool,
    /// Overdub was pressed during the first pass: it starts when it ends.
    pub overdub_next: bool,
    pub latency: u32,
    pub level: f32,
    pub muted: bool,
    /// The longest first pass that fits, in frames.
    pub capacity: usize,
    /// Frames of memory the loop holds: its layers and the ones Undo or
    /// Redo can bring back.
    pub used: usize,
}

#[derive(Clone, Copy)]
enum Queued {
    Loop(Command),
    Layer(u8, LayerCommand),
}

/// An overdub added or a layer deleted, for Undo and Redo: the layer's
/// slot, and its place in the play order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Act {
    deleted: bool,
    slot: u8,
    index: u8,
}

const NO_ACT: Act = Act { deleted: false, slot: 0, index: 0 };

/// A layer's memory slot and its mix.
#[derive(Clone, Copy, Debug)]
struct Slot {
    /// (first position written, frames written).
    valid: (usize, usize),
    level: f32,
    pan: f32,
    mute: bool,
    solo: bool,
    /// Gains now, left and right: they move to the settings over FADE.
    gl: f32,
    gr: f32,
    /// 0 to 1: fades in when the layer comes back, out when it goes.
    presence: f32,
    /// Fading out, to leave the play order when silent.
    leaving: bool,
}

impl Slot {
    const NEW: Slot =
        Slot { valid: (0, 0), level: 1.0, pan: 0.0, mute: false, solo: false, gl: 1.0, gr: 1.0, presence: 1.0, leaving: false };

    /// The gains the settings ask for.
    fn target(&self, any_solo: bool) -> (f32, f32) {
        if self.mute || (any_solo && !self.solo) {
            return (0.0, 0.0);
        }
        (self.level * (1.0 - self.pan).min(1.0), self.level * (1.0 + self.pan).min(1.0))
    }
}

pub struct Looper {
    /// Interleaved stereo; layer `k` is frames `k * len .. (k + 1) * len`.
    pool: Vec<f32>,
    capacity: usize,
    phase: Phase,
    bars: Option<u32>,
    latency: u32,
    level: f32,
    muted: bool,
    // The loop.
    origin: Tick,
    len_ticks: Tick,
    /// Frames; 0 until the first pass ends.
    len: usize,
    /// The play order: slots, first pass first. Leaving layers stay until
    /// they are silent.
    order: [u8; MAX_LAYERS],
    in_order: usize,
    slots: [Slot; MAX_LAYERS],
    done: [Act; HISTORY],
    n_done: usize,
    undone: [Act; HISTORY],
    n_undone: usize,
    // The first pass.
    start_frame: u64,
    end_tick: Option<Tick>,
    ending: bool,
    /// Go straight into an overdub when the first pass ends.
    then_overdub: bool,
    /// The first pass's writer is still at work (it runs `latency` frames
    /// behind, and ends by crossfading the loop point).
    first_open: bool,
    /// The latency the first pass is written with (fixed when it starts).
    rec_latency: u32,
    /// Input just before the loop's first frame, oldest first.
    pre: [f32; 2 * FADE],
    ring: [f32; 2 * FADE],
    ring_at: usize,
    // The overdub writer.
    od_open: bool,
    /// The slot it writes.
    od_layer: usize,
    od_in: usize,
    /// Frames of the writer's fade-in.
    od_fade: usize,
    od_stop_in: Option<u32>,
    od_out: usize,
    // Gains.
    gain: f32,
    clearing: bool,
    pos: f64,
    /// A jump's crossfade: frames left, and where the old place is now.
    jump: usize,
    jump_pos: f64,
    queue: [Queued; QUEUE],
    queued: usize,
    stats: Stats,
}

impl Looper {
    /// A looper with room for `seconds` of stereo audio at `rate`, shared
    /// by the first pass and its overdubs. Allocates; call it before the
    /// sound starts.
    pub fn new(rate: u32, seconds: u32) -> Looper {
        let capacity = (rate as usize) * (seconds as usize);
        Looper {
            pool: vec![0.0; 2 * capacity],
            capacity,
            phase: Phase::Empty,
            bars: Some(2),
            latency: 0,
            level: 1.0,
            muted: false,
            origin: 0,
            len_ticks: 0,
            len: 0,
            order: [0; MAX_LAYERS],
            in_order: 0,
            slots: [Slot::NEW; MAX_LAYERS],
            done: [NO_ACT; HISTORY],
            n_done: 0,
            undone: [NO_ACT; HISTORY],
            n_undone: 0,
            start_frame: 0,
            end_tick: None,
            ending: false,
            then_overdub: false,
            first_open: false,
            rec_latency: 0,
            pre: [0.0; 2 * FADE],
            ring: [0.0; 2 * FADE],
            ring_at: 0,
            od_open: false,
            od_layer: 0,
            od_in: 0,
            od_fade: FADE,
            od_stop_in: None,
            od_out: 0,
            gain: 0.0,
            clearing: false,
            pos: 0.0,
            jump: 0,
            jump_pos: 0.0,
            queue: [Queued::Loop(Command::Clear); QUEUE],
            queued: 0,
            stats: Stats::default(),
        }
    }

    /// Queue a command for the next period. False (and counted) if the
    /// queue is full.
    pub fn command(&mut self, c: Command) -> bool {
        self.push(Queued::Loop(c))
    }

    /// Queue a command for layer `id` ([`LayerInfo::id`]) for the next
    /// period. False (and counted) if the queue is full.
    pub fn layer_command(&mut self, id: u8, c: LayerCommand) -> bool {
        self.push(Queued::Layer(id, c))
    }

    fn push(&mut self, q: Queued) -> bool {
        if self.queued == QUEUE {
            self.stats.dropped_commands += 1;
            return false;
        }
        self.queue[self.queued] = q;
        self.queued += 1;
        true
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    pub fn status(&self) -> Status {
        Status {
            phase: self.phase,
            layers: self.layers(),
            redo: self.n_undone,
            bars: self.bars,
            frames: self.len,
            ticks: self.len_ticks,
            origin: self.origin,
            position: if self.len > 0 { (self.pos / self.len as f64) as f32 } else { 0.0 },
            ending: self.ending,
            overdub_next: self.then_overdub,
            latency: self.latency,
            level: self.level,
            muted: self.muted,
            capacity: self.capacity,
            used: self.held() * self.len,
        }
    }

    /// Layer `k`'s audio (interleaved stereo, the loop's length), counting
    /// in play order, for drawing the waveform. None if there is no such
    /// layer.
    pub fn layer(&self, k: usize) -> Option<&[f32]> {
        let slot = self.listed(k)?;
        (self.len > 0).then(|| &self.pool[2 * slot * self.len..2 * (slot + 1) * self.len])
    }

    /// Layer `k`'s id and mix, counting in play order. None if there is no
    /// such layer.
    pub fn layer_info(&self, k: usize) -> Option<LayerInfo> {
        let slot = self.listed(k)?;
        let s = &self.slots[slot];
        let any_solo = self.any_solo();
        Some(LayerInfo {
            id: slot as u8,
            level: s.level,
            pan: s.pan,
            muted: s.mute,
            solo: s.solo,
            audible: !s.mute && (!any_solo || s.solo),
            recording: self.od_open && self.od_layer == slot,
        })
    }

    /// Layers in play order, not counting ones fading away.
    fn layers(&self) -> usize {
        self.order[..self.in_order].iter().filter(|&&k| !self.slots[k as usize].leaving).count()
    }

    /// The slot of the `k`th layer not fading away.
    fn listed(&self, k: usize) -> Option<usize> {
        self.order[..self.in_order].iter().map(|&s| s as usize).filter(|&s| !self.slots[s].leaving).nth(k)
    }

    fn any_solo(&self) -> bool {
        self.order[..self.in_order].iter().any(|&k| self.slots[k as usize].solo && !self.slots[k as usize].leaving)
    }

    /// Whether `slot` is playing, or Undo or Redo can bring it back.
    fn holds(&self, slot: usize) -> bool {
        self.order[..self.in_order].iter().any(|&k| k as usize == slot)
            || self.done[..self.n_done].iter().chain(&self.undone[..self.n_undone]).any(|a| a.slot as usize == slot)
    }

    /// Slots held.
    fn held(&self) -> usize {
        (0..MAX_LAYERS).filter(|&k| self.holds(k)).count()
    }

    /// Run one period: `input` is the line input and `out` the mix, both
    /// interleaved stereo with `block.frames` frames. The loop is added to
    /// `out`; nothing else is changed.
    pub fn process(&mut self, block: &Block, input: &[f32], out: &mut [f32]) {
        let n = (block.frames as usize).min(input.len() / 2).min(out.len() / 2);
        self.apply_commands();
        let markers = block.markers();
        let mut m = 0;
        for span in block.spans() {
            let bar = span.meter.bar_ticks();
            let watch_bars = matches!(self.phase, Phase::Armed | Phase::Recording);
            let mut lines = span.lines(bar);
            let mut next = if watch_bars { lines.next() } else { None };
            // Playing or recording: the loop sounds. Counting in: the clock
            // runs, but nothing plays back.
            let playing = matches!(span.state, State::Playing | State::Recording);
            for i in 0..span.len as usize {
                let off = span.offset as usize + i;
                if off >= n {
                    return;
                }
                let f = span.frame + i as u64;
                while m < markers.len() && markers[m].frame <= f {
                    self.marker(markers[m].mark, markers[m].frame);
                    m += 1;
                }
                while let Some((t, o)) = next {
                    if o as usize > off {
                        break;
                    }
                    if o as usize == off {
                        self.bar_line(span, t, f);
                    }
                    next = lines.next();
                }
                let x = [input[2 * off], input[2 * off + 1]];
                self.write_first(f, x);
                self.follow(span, f, playing, i == 0);
                let y = self.read(playing);
                out[2 * off] += y[0];
                out[2 * off + 1] += y[1];
                if self.od_open {
                    self.overdub_frame(x, playing);
                }
            }
        }
        while m < markers.len() {
            self.marker(markers[m].mark, markers[m].frame);
            m += 1;
        }
    }

    /// Move the loop's position to frame `f`. While playing it is the song
    /// position's place in the loop; while not, it runs on so the sound
    /// fades out where it was going. A jump starts a crossfade.
    fn follow(&mut self, span: &Span, f: u64, playing: bool, first: bool) {
        if self.len == 0 {
            return;
        }
        let len = self.len as f64;
        let was = self.pos;
        if self.jump > 0 {
            self.jump -= 1;
            self.jump_pos = (self.jump_pos + 1.0) % len;
        }
        if !playing {
            self.pos = (was + 1.0) % len;
            return;
        }
        let rel = (span.tick_at(f) - self.origin as f64).rem_euclid(self.len_ticks as f64);
        self.pos = (rel / self.len_ticks as f64 * len).min(len - 1e-6);
        // Within a span the song position is continuous; a jump can only
        // come at a span's first frame.
        if first && self.gain > 0.0 {
            let mut d = (self.pos - was).rem_euclid(len);
            if d > len / 2.0 {
                d -= len;
            }
            if d.abs() > 4.0 {
                self.jump = FADE;
                self.jump_pos = (was + 1.0) % len;
            }
        }
    }

    fn apply_commands(&mut self) {
        // Commands that would cut a fade short wait for it, in order.
        let mut kept = 0;
        let mut waiting = false;
        for k in 0..self.queued {
            let q = self.queue[k];
            let deferrable = matches!(
                q,
                Queued::Loop(Command::Record | Command::Overdub | Command::Undo | Command::Redo)
                    | Queued::Layer(_, LayerCommand::Delete)
            );
            if deferrable && (waiting || self.must_wait(q)) {
                self.queue[kept] = q;
                kept += 1;
                waiting = true;
            } else {
                match q {
                    Queued::Loop(c) => self.apply(c),
                    Queued::Layer(id, c) => self.apply_layer(id, c),
                }
            }
        }
        self.queued = kept;
    }

    fn must_wait(&self, q: Queued) -> bool {
        match q {
            Queued::Loop(Command::Undo) => self.leaving() || self.clearing,
            _ => self.fading() || self.clearing,
        }
    }

    /// A layer is fading out of the play order.
    fn leaving(&self) -> bool {
        self.order[..self.in_order].iter().any(|&k| self.slots[k as usize].leaving)
    }

    /// A layer is fading in or out.
    fn fading(&self) -> bool {
        self.order[..self.in_order].iter().any(|&k| self.slots[k as usize].leaving || self.slots[k as usize].presence < 1.0)
    }

    fn apply_layer(&mut self, id: u8, c: LayerCommand) {
        let slot = id as usize;
        let at = self.order[..self.in_order].iter().position(|&k| k == id);
        let Some(at) = at.filter(|_| !self.slots[slot].leaving) else {
            self.stats.no_layer += 1;
            return;
        };
        let s = &mut self.slots[slot];
        match c {
            LayerCommand::Level(v) if v.is_finite() => s.level = v.clamp(0.0, 2.0),
            LayerCommand::Pan(v) if v.is_finite() => s.pan = v.clamp(-1.0, 1.0),
            LayerCommand::Level(_) | LayerCommand::Pan(_) => {}
            LayerCommand::Mute(m) => s.mute = m,
            LayerCommand::Solo(o) => s.solo = o,
            // The overdub being recorded is thrown away, as Undo does.
            LayerCommand::Delete if self.od_open && self.od_layer == slot => self.apply(Command::Undo),
            LayerCommand::Delete if self.layers() > 1 => {
                self.n_undone = 0;
                self.push_done(Act { deleted: true, slot: id, index: at as u8 });
                self.slots[slot].leaving = true;
            }
            LayerCommand::Delete => {}
        }
    }

    /// Undo the newest overdub or delete.
    fn undo(&mut self) {
        if self.n_done == 0 {
            return;
        }
        let a = self.take_done(self.n_done - 1);
        self.push_undone(a);
        if a.deleted {
            self.restore(a);
        } else {
            self.slots[a.slot as usize].leaving = true;
        }
    }

    /// Redo what was undone last.
    fn redo(&mut self) {
        self.n_undone -= 1;
        let a = self.undone[self.n_undone];
        self.push_done(a);
        if a.deleted {
            self.slots[a.slot as usize].leaving = true;
        } else {
            self.restore(a);
        }
    }

    /// Put an act's layer back in its place, fading in.
    fn restore(&mut self, a: Act) {
        let at = (a.index as usize).min(self.in_order);
        self.order.copy_within(at..self.in_order, at + 1);
        self.order[at] = a.slot;
        self.in_order += 1;
        let s = &mut self.slots[a.slot as usize];
        s.presence = 0.0;
        s.leaving = false;
    }

    /// Take the layer at place `at` out of the play order.
    fn remove_at(&mut self, at: usize) {
        self.order.copy_within(at + 1..self.in_order, at);
        self.in_order -= 1;
    }

    /// Remember an act for Undo; the oldest is forgotten when the history
    /// is full.
    fn push_done(&mut self, a: Act) {
        if self.n_done == HISTORY {
            self.take_done(0);
        }
        self.done[self.n_done] = a;
        self.n_done += 1;
    }

    fn push_undone(&mut self, a: Act) {
        if self.n_undone == HISTORY {
            self.undone.copy_within(1..HISTORY, 0);
            self.n_undone -= 1;
        }
        self.undone[self.n_undone] = a;
        self.n_undone += 1;
    }

    fn take_done(&mut self, i: usize) -> Act {
        let a = self.done[i];
        self.done.copy_within(i + 1..self.n_done, i);
        self.n_done -= 1;
        a
    }

    fn apply(&mut self, c: Command) {
        match c {
            Command::Record => match self.phase {
                Phase::Empty => {
                    self.phase = Phase::Armed;
                    self.ring = [0.0; 2 * FADE];
                }
                Phase::Armed => self.phase = Phase::Empty,
                Phase::Recording => self.ending = true,
                Phase::Playing | Phase::Overdubbing => self.apply(Command::Overdub),
            },
            Command::Overdub => match self.phase {
                Phase::Playing => self.open_overdub(FADE),
                Phase::Overdubbing => self.punch_out(),
                Phase::Recording => self.then_overdub = !self.then_overdub,
                _ => {}
            },
            Command::Undo => match self.phase {
                Phase::Armed | Phase::Recording => self.abort(),
                Phase::Overdubbing | Phase::Playing if self.od_open => {
                    // Throw the overdub away (recording, or still writing
                    // its tail): it stops and fades out, and Redo can
                    // bring back what it recorded.
                    self.od_open = false;
                    self.phase = Phase::Playing;
                    let slot = self.od_layer;
                    if !self.drop_empty_overdub() {
                        if let Some(i) = self.done[..self.n_done].iter().rposition(|a| !a.deleted && a.slot as usize == slot) {
                            let a = self.take_done(i);
                            self.push_undone(a);
                        }
                        self.slots[slot].leaving = true;
                    }
                }
                Phase::Playing => self.undo(),
                _ => {}
            },
            Command::Redo => {
                if self.phase == Phase::Playing && self.n_undone > 0 && !self.od_open {
                    self.redo();
                }
            }
            Command::Clear => match self.phase {
                Phase::Empty => {}
                Phase::Armed | Phase::Recording => self.abort(),
                Phase::Playing | Phase::Overdubbing => {
                    self.od_open = false;
                    self.clearing = true;
                }
            },
            Command::Mute(m) => self.muted = m,
            Command::SetBars(b) => self.bars = b.filter(|&b| b > 0),
            Command::SetLatency(l) => self.latency = l,
            Command::SetLevel(v) => self.level = if v.is_finite() { v.clamp(0.0, 2.0) } else { self.level },
        }
    }

    /// Back to empty, keeping the settings.
    fn reset(&mut self) {
        self.phase = Phase::Empty;
        self.len = 0;
        self.len_ticks = 0;
        self.in_order = 0;
        self.n_done = 0;
        self.n_undone = 0;
        self.end_tick = None;
        self.ending = false;
        self.then_overdub = false;
        self.first_open = false;
        self.od_open = false;
        self.clearing = false;
        self.gain = 0.0;
        self.pos = 0.0;
        self.jump = 0;
    }

    fn abort(&mut self) {
        if self.phase == Phase::Recording {
            self.stats.aborted += 1;
        }
        self.reset();
    }

    fn marker(&mut self, mark: Mark, frame: u64) {
        if self.phase != Phase::Recording {
            return;
        }
        match mark {
            // The transport looped back at `end`: if that is where the pass
            // ends (or Record asked for the next bar), it ends here.
            Mark::Looped { end, .. } if self.end_tick == Some(end) || (self.ending && self.end_tick.is_none()) => {
                self.end_tick = Some(end);
                self.finish_first(end, frame);
            }
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Stopped { .. } | Mark::Panic => {
                self.stats.aborted += 1;
                self.reset();
            }
            _ => {}
        }
    }

    fn bar_line(&mut self, span: &Span, t: Tick, f: u64) {
        match self.phase {
            Phase::Recording if t > self.origin && (self.end_tick == Some(t) || self.ending) => {
                self.finish_first(t, f);
            }
            Phase::Armed if t >= 0 && matches!(span.state, State::Playing | State::Recording) => {
                self.phase = Phase::Recording;
                self.origin = t;
                self.start_frame = f;
                self.end_tick = self.bars.map(|b| t + i64::from(b) * span.meter.bar_ticks());
                self.ending = false;
                self.then_overdub = false;
                self.first_open = true;
                self.rec_latency = self.latency;
                self.len = 0;
            }
            _ => {}
        }
    }

    fn finish_first(&mut self, t: Tick, f: u64) {
        let len = (f - self.start_frame) as usize;
        if len == 0 || len > self.capacity {
            self.stats.aborted += 1;
            self.reset();
            return;
        }
        self.len = len;
        self.len_ticks = t - self.origin;
        self.order[0] = 0;
        self.in_order = 1;
        self.slots[0] = Slot { valid: (0, len), ..Slot::NEW };
        self.n_done = 0;
        self.n_undone = 0;
        self.ending = false;
        self.pos = 0.0;
        self.jump = 0;
        self.phase = Phase::Playing;
        if self.then_overdub {
            self.then_overdub = false;
            self.open_overdub(HANDOFF_FADE);
        }
    }

    /// The first pass's writer: frame `f`'s input belongs `latency` frames
    /// earlier, so the loop's frame `j` is written at `start + latency + j`.
    fn write_first(&mut self, f: u64, x: [f32; 2]) {
        let before = match self.phase {
            Phase::Armed => true,
            _ if self.first_open => f < self.start_frame + u64::from(self.rec_latency),
            _ => return,
        };
        if before {
            self.ring[2 * self.ring_at] = x[0];
            self.ring[2 * self.ring_at + 1] = x[1];
            self.ring_at = (self.ring_at + 1) % FADE;
            return;
        }
        let j = (f - self.start_frame - u64::from(self.rec_latency)) as usize;
        if j == 0 {
            for k in 0..FADE {
                let r = (self.ring_at + k) % FADE;
                self.pre[2 * k] = self.ring[2 * r];
                self.pre[2 * k + 1] = self.ring[2 * r + 1];
            }
        }
        if self.len > 0 && j >= self.len {
            self.seal();
            return;
        }
        if j >= self.capacity {
            self.stats.aborted += 1;
            self.reset();
            return;
        }
        self.pool[2 * j] = x[0];
        self.pool[2 * j + 1] = x[1];
    }

    /// The loop point: the end fades into the input from just before the
    /// start, which flows on into the start.
    fn seal(&mut self) {
        self.first_open = false;
        let fade = FADE.min(self.len / 2);
        for k in 0..fade {
            let w = (k as f32 + 0.5) / fade as f32;
            let p = FADE - fade + k;
            let idx = self.len - fade + k;
            for c in 0..2 {
                let a = self.pool[2 * idx + c];
                self.pool[2 * idx + c] = a * (1.0 - w) + self.pre[2 * p + c] * w;
            }
        }
    }

    /// Layer slots the memory has room for.
    fn slot_count(&self) -> usize {
        self.capacity.checked_div(self.len).map_or(0, |n| MAX_LAYERS.min(n))
    }

    /// A slot for a new overdub: one no layer plays and Undo cannot bring
    /// back (what Redo could bring back goes, as a new overdub ends
    /// redo). With none, the history up to its oldest delete is forgotten,
    /// so that delete's slot can be used. None if every slot plays.
    fn free_slot(&mut self) -> Option<usize> {
        loop {
            let live = |k: usize| {
                self.order[..self.in_order].iter().any(|&s| s as usize == k)
                    || self.done[..self.n_done].iter().any(|a| a.slot as usize == k)
            };
            if let Some(k) = (0..self.slot_count()).find(|&k| !live(k)) {
                return Some(k);
            }
            let i = self.done[..self.n_done].iter().position(|a| a.deleted)?;
            self.done.copy_within(i + 1..self.n_done, 0);
            self.n_done -= i + 1;
        }
    }

    /// A new layer on top, written from now on, fading in over `fade`
    /// frames.
    fn open_overdub(&mut self, fade: usize) {
        let Some(k) = self.free_slot() else {
            self.stats.refused += 1;
            return;
        };
        self.n_undone = 0;
        self.slots[k] = Slot::NEW;
        self.order[self.in_order] = k as u8;
        self.push_done(Act { deleted: false, slot: k as u8, index: self.in_order as u8 });
        self.in_order += 1;
        self.od_open = true;
        self.od_layer = k;
        self.od_in = 0;
        self.od_fade = fade;
        self.od_stop_in = None;
        self.od_out = FADE;
        self.phase = Phase::Overdubbing;
    }

    /// End the overdub: the writer goes on for `latency` frames (that input
    /// belongs before the press), then fades out.
    fn punch_out(&mut self) {
        if self.od_stop_in.is_none() {
            self.od_stop_in = Some(self.latency);
        }
        self.phase = Phase::Playing;
    }

    /// An overdub that never wrote a frame leaves no layer behind, and
    /// nothing to undo. True if it was dropped.
    fn drop_empty_overdub(&mut self) -> bool {
        let k = self.od_layer;
        if self.slots[k].valid.1 != 0 {
            return false;
        }
        if let Some(at) = self.order[..self.in_order].iter().position(|&s| s as usize == k) {
            self.remove_at(at);
        }
        if let Some(i) = self.done[..self.n_done].iter().rposition(|a| !a.deleted && a.slot as usize == k) {
            self.take_done(i);
        }
        true
    }

    fn close_overdub(&mut self) {
        self.od_open = false;
        self.drop_empty_overdub();
        if self.phase == Phase::Overdubbing {
            self.phase = Phase::Playing;
        }
    }

    /// One frame of the overdub writer. It waits for the first pass's
    /// writer to finish (it runs `latency` frames behind, so this input is
    /// still the first pass), and while the transport is stopped (Overdub
    /// pressed before Play); it fades out when the transport stops.
    fn overdub_frame(&mut self, x: [f32; 2], playing: bool) {
        if self.first_open {
            return;
        }
        if !playing {
            if self.od_in == 0 {
                if self.od_stop_in.is_some() {
                    // Punched out before it began.
                    self.close_overdub();
                }
                return;
            }
            self.od_stop_in = Some(0);
        }
        self.write_overdub(x);
    }

    fn write_overdub(&mut self, x: [f32; 2]) {
        let len = self.len;
        let mut g = if self.od_in < self.od_fade { (self.od_in as f32 + 0.5) / self.od_fade as f32 } else { 1.0 };
        self.od_in += 1;
        let mut last = false;
        match self.od_stop_in {
            Some(0) => {
                g *= (self.od_out as f32 - 0.5) * STEP;
                self.od_out -= 1;
                last = self.od_out == 0;
            }
            Some(left) => self.od_stop_in = Some(left - 1),
            None => {}
        }
        let q = ((self.pos - f64::from(self.latency)).rem_euclid(len as f64).round() as usize) % len;
        let k = self.od_layer;
        let (start, count) = self.slots[k].valid;
        let idx = 2 * (k * len + q);
        if count == 0 {
            self.slots[k].valid = (q, 1);
            self.pool[idx] = x[0] * g;
            self.pool[idx + 1] = x[1] * g;
        } else if count < len {
            self.slots[k].valid = (start, count + 1);
            self.pool[idx] = x[0] * g;
            self.pool[idx + 1] = x[1] * g;
        } else {
            self.pool[idx] += x[0] * g;
            self.pool[idx + 1] += x[1] * g;
        }
        if last {
            self.close_overdub();
        }
    }

    fn read(&mut self, playing: bool) -> [f32; 2] {
        let audible = matches!(self.phase, Phase::Playing | Phase::Overdubbing)
            && playing
            && !self.muted
            && !self.clearing;
        if audible {
            self.gain = (self.gain + STEP).min(1.0);
        } else {
            self.gain = (self.gain - STEP).max(0.0);
        }
        if self.clearing && self.gain == 0.0 {
            self.reset();
            return [0.0; 2];
        }
        self.mix_layers();
        if self.gain == 0.0 || self.len == 0 {
            return [0.0; 2];
        }
        let mut y = self.sample_at(self.pos);
        if self.jump > 0 {
            let w = self.jump as f32 * STEP;
            let old = self.sample_at(self.jump_pos);
            for (v, o) in y.iter_mut().zip(old) {
                *v = *v * (1.0 - w) + o * w;
            }
        }
        let g = self.gain * self.level;
        [y[0] * g, y[1] * g]
    }

    /// One frame of every layer's fades: coming and going, and its gains
    /// moving to its level, pan, mute and solo. A layer that has faded
    /// away leaves the play order.
    fn mix_layers(&mut self) {
        let any_solo = self.any_solo();
        let mut p = 0;
        while p < self.in_order {
            let s = &mut self.slots[self.order[p] as usize];
            if s.leaving {
                s.presence = (s.presence - STEP).max(0.0);
                if s.presence == 0.0 {
                    s.leaving = false;
                    self.remove_at(p);
                    continue;
                }
            } else if s.presence < 1.0 {
                s.presence = (s.presence + STEP).min(1.0);
            }
            let (l, r) = s.target(any_solo);
            s.gl += (l - s.gl).clamp(-STEP, STEP);
            s.gr += (r - s.gr).clamp(-STEP, STEP);
            p += 1;
        }
    }

    /// Every layer at `pos`, read between frames, at its gains. Only the
    /// frames a layer has written count.
    fn sample_at(&self, pos: f64) -> [f32; 2] {
        let len = self.len;
        let i0 = (pos as usize).min(len - 1);
        let frac = (pos - i0 as f64) as f32;
        let i1 = if i0 + 1 == len { 0 } else { i0 + 1 };
        let mut y = [0.0f32; 2];
        for &k in &self.order[..self.in_order] {
            let s = &self.slots[k as usize];
            let g = [s.gl * s.presence, s.gr * s.presence];
            if g == [0.0, 0.0] {
                continue;
            }
            let (start, count) = s.valid;
            let full = count >= len;
            let in0 = full || (i0 + len - start) % len < count;
            let in1 = full || (i1 + len - start) % len < count;
            if !in0 && !in1 {
                continue;
            }
            let base = 2 * k as usize * len;
            for (c, v) in y.iter_mut().enumerate() {
                let v0 = if in0 { self.pool[base + 2 * i0 + c] } else { 0.0 };
                let v1 = if in1 { self.pool[base + 2 * i1 + c] } else { 0.0 };
                *v += g[c] * (v0 + (v1 - v0) * frac);
            }
        }
        y
    }
}
