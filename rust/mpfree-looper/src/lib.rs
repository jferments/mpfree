//! mpfree-looper: the audio looper.
//!
//! Up to [`MAX_LOOPS`] stereo loops on the transport, recorded from the
//! line input, sharing one memory:
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
//! - **Several loops.** [`Looper::select_loop`] picks the loop the commands
//!   after it act on; every loop plays. One loop records at a time:
//!   starting to record in one ends the recording in the others (an
//!   overdub punches out, a first pass ends on the next bar line, an armed
//!   loop disarms).
//! - **One memory.** Loops take memory in blocks of [`BLOCK`] frames from
//!   one pool, as they need it, and give it back when a layer can no
//!   longer come back. Nothing is ever moved, so the memory never
//!   fragments. [`Looper::memory`] is the gauge.
//! - **No clicks:** the loop point is crossfaded with the input from just
//!   before the first pass, so the end flows into the start. Punch in and
//!   out, start, stop, mute, undo, delete, clear and every layer's level,
//!   pan, mute and solo fade over [`FADE`] frames, and a jump in the song
//!   position (a locate, a transport loop that is not a whole number of
//!   loops, a restart) crossfades from the old place to the new one.
//!   Commands that would cut a fade short wait for it to end (at most
//!   [`FADE`] frames).
//! - **Stopped:** the loops are silent while the transport is stopped or
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

use std::ops::Index;

use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Mark, Span, State};

/// Frames of every fade: the loop point, punch in and out, start, stop,
/// mute, undo and clear (about 5.8 ms at 44.1 kHz).
pub const FADE: usize = 256;
/// Layers at most in one loop: the first pass and 15 overdubs.
pub const MAX_LAYERS: usize = 16;
/// Loops at most.
pub const MAX_LOOPS: usize = 8;
/// Frames in a block of memory (about 23 ms at 44.1 kHz). A layer takes
/// whole blocks.
pub const BLOCK: usize = 1024;
/// Commands waiting for the next period.
const QUEUE: usize = 32;
/// The most frames the loops run in one go (a period is cut into runs at
/// bar lines, markers and this many frames).
const RUN: usize = 128;
/// Overdubs and deletes Undo can take back, per loop.
const HISTORY: usize = 64;
const STEP: f32 = 1.0 / FADE as f32;
/// The fade-in of an overdub that starts where the first pass ends: the
/// input runs on without a break there, so a short fade is enough, and the
/// downbeat's attack is kept (about 0.7 ms at 44.1 kHz).
pub const HANDOFF_FADE: usize = 32;
/// A table entry with no block.
const NONE: u16 = u16::MAX;
static SILENCE: f32 = 0.0;

/// What the looper's buttons and screen ask for. Each applies to the
/// selected loop at the start of the next period.
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
    /// For every loop.
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
    /// recorded throws it away, as Undo does.
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
    /// Overdubs refused because there was no memory or no layer slot.
    pub refused: u64,
    /// Layer commands for a layer that is not there, and loop numbers past
    /// [`MAX_LOOPS`].
    pub no_layer: u64,
    /// First passes thrown away: the transport stopped or jumped before
    /// the end, or the pass outgrew the memory.
    pub aborted: u64,
}

/// What the screen shows of one loop.
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
    /// The memory all the loops share, in frames.
    pub capacity: usize,
    /// Frames of memory this loop holds: its layers and the ones Undo or
    /// Redo can bring back, in whole blocks.
    pub used: usize,
}

/// The memory gauge: all the loops together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    /// Frames held by every loop, in whole blocks.
    pub used: usize,
    /// Frames the loops share.
    pub capacity: usize,
}

/// A layer's audio, interleaved stereo, the loop's length: index it like
/// a slice (`layer[2 * frame + channel]`). Frames an overdub did not
/// reach read as silence.
pub struct LayerView<'a> {
    store: &'a Store,
    lp: usize,
    slot: usize,
    len: usize,
    /// The frames written: (first, count), wrapping.
    valid: (usize, usize),
}

impl LayerView<'_> {
    /// Samples (twice the frames).
    pub fn len(&self) -> usize {
        2 * self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Index<usize> for LayerView<'_> {
    type Output = f32;

    fn index(&self, i: usize) -> &f32 {
        assert!(
            i < 2 * self.len,
            "sample {i} of a layer of {} frames",
            self.len
        );
        let q = i / 2;
        let (start, count) = self.valid;
        if (q + self.len - start) % self.len >= count {
            return &SILENCE;
        }
        self.store
            .sample(self.lp, self.slot, q, i % 2)
            .unwrap_or(&SILENCE)
    }
}

/// A run of frames, the same for every loop.
struct Run<'a> {
    /// The run's first frame, counted in its span.
    from: usize,
    /// Each frame's song position.
    ticks: &'a [f64],
    /// The line input, interleaved stereo.
    input: &'a [f32],
    /// The run's first frame.
    f: u64,
    playing: bool,
}

#[derive(Clone, Copy)]
enum Queued {
    Loop(Command),
    Layer(u8, LayerCommand),
    Select(usize),
}

/// An overdub added or a layer deleted, for Undo and Redo: the layer's
/// slot, and its place in the play order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Act {
    deleted: bool,
    slot: u8,
    index: u8,
}

const NO_ACT: Act = Act {
    deleted: false,
    slot: 0,
    index: 0,
};

/// A layer's place in its loop and its mix. Its audio is in the store.
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
    const NEW: Slot = Slot {
        valid: (0, 0),
        level: 1.0,
        pan: 0.0,
        mute: false,
        solo: false,
        gl: 1.0,
        gr: 1.0,
        presence: 1.0,
        leaving: false,
    };

    /// The gains the settings ask for.
    fn target(&self, any_solo: bool) -> (f32, f32) {
        if self.mute || (any_solo && !self.solo) {
            return (0.0, 0.0);
        }
        (
            self.level * (1.0 - self.pan).min(1.0),
            self.level * (1.0 + self.pan).min(1.0),
        )
    }
}

/// The shared memory: blocks of BLOCK stereo frames, and for every layer
/// slot of every loop a table of its blocks. A slot's blocks are always
/// the first entries of its table.
struct Store {
    pool: Vec<f32>,
    /// MAX_LOOPS * MAX_LAYERS tables of `n_blocks` entries.
    tables: Vec<u16>,
    /// Blocks in each slot's table.
    held: [[u16; MAX_LAYERS]; MAX_LOOPS],
    /// Free blocks, a stack.
    free: Vec<u16>,
    n_free: usize,
    n_blocks: usize,
}

impl Store {
    fn new(n_blocks: usize) -> Store {
        Store {
            pool: vec![0.0; 2 * BLOCK * n_blocks],
            tables: vec![NONE; MAX_LOOPS * MAX_LAYERS * n_blocks],
            held: [[0; MAX_LAYERS]; MAX_LOOPS],
            free: (0..n_blocks as u16).rev().collect(),
            n_free: n_blocks,
            n_blocks,
        }
    }

    fn table(&self, lp: usize, slot: usize) -> usize {
        (lp * MAX_LAYERS + slot) * self.n_blocks
    }

    /// Where frame `q` of a slot is in the pool, if its block is there.
    fn at(&self, lp: usize, slot: usize, q: usize) -> Option<usize> {
        let b = q / BLOCK;
        if b >= self.held[lp][slot] as usize {
            return None;
        }
        Some(2 * (self.tables[self.table(lp, slot) + b] as usize * BLOCK + q % BLOCK))
    }

    fn sample(&self, lp: usize, slot: usize, q: usize, c: usize) -> Option<&f32> {
        self.at(lp, slot, q).map(|i| &self.pool[i + c])
    }

    /// The blocks a slot holds, in order.
    #[inline]
    fn blocks_of(&self, lp: usize, slot: usize) -> &[u16] {
        let t = self.table(lp, slot);
        &self.tables[t..t + self.held[lp][slot] as usize]
    }

    /// Frame `o` of block `b` of a slot whose blocks are `blocks`
    /// (silence if it does not hold that block).
    #[inline]
    fn frame(&self, blocks: &[u16], b: usize, o: usize) -> [f32; 2] {
        match blocks.get(b) {
            Some(&n) => {
                let i = 2 * (n as usize * BLOCK + o);
                let p = &self.pool[i..i + 2];
                [p[0], p[1]]
            }
            None => [0.0; 2],
        }
    }

    fn get(&self, lp: usize, slot: usize, q: usize) -> [f32; 2] {
        self.at(lp, slot, q)
            .map_or([0.0; 2], |i| [self.pool[i], self.pool[i + 1]])
    }

    fn set(&mut self, lp: usize, slot: usize, q: usize, x: [f32; 2]) {
        if let Some(i) = self.at(lp, slot, q) {
            self.pool[i] = x[0];
            self.pool[i + 1] = x[1];
        }
    }

    fn add(&mut self, lp: usize, slot: usize, q: usize, x: [f32; 2]) {
        if let Some(i) = self.at(lp, slot, q) {
            self.pool[i] += x[0];
            self.pool[i + 1] += x[1];
        }
    }

    /// Give a slot blocks until it holds `n`. False, with what it got, if
    /// the memory ran out.
    fn grow(&mut self, lp: usize, slot: usize, n: usize) -> bool {
        let t = self.table(lp, slot);
        while (self.held[lp][slot] as usize) < n {
            if self.n_free == 0 {
                return false;
            }
            self.n_free -= 1;
            let h = self.held[lp][slot] as usize;
            self.tables[t + h] = self.free[self.n_free];
            self.held[lp][slot] += 1;
        }
        true
    }

    /// Give back every block of a slot.
    fn release(&mut self, lp: usize, slot: usize) {
        let t = self.table(lp, slot);
        for h in 0..self.held[lp][slot] as usize {
            self.free[self.n_free] = self.tables[t + h];
            self.tables[t + h] = NONE;
            self.n_free += 1;
        }
        self.held[lp][slot] = 0;
    }

    fn blocks(&self, lp: usize) -> usize {
        self.held[lp].iter().map(|&h| h as usize).sum()
    }
}

/// Blocks for `frames` frames.
fn blocks_for(frames: usize) -> usize {
    frames.div_ceil(BLOCK)
}

/// `x.rem_euclid(y)` bit for bit, for `y` a whole number of ticks above 0,
/// without the cost of `fmod` (a loop of integer steps in software, which
/// was an eighth of the looper's time with 8 loops playing).
///
/// For |x| below 2^31, `n * y` is a whole number and so a multiple of x's
/// last place, which makes `x - n * y` exact; `n` can be one too many where
/// `x / y` rounds up to a whole number, and adding `y` back is exact too,
/// as the result (fmod's) is representable.
fn rem_ticks(x: f64, y: f64) -> f64 {
    // Below 2^31 the quotient fits an i32, whose conversion is one
    // instruction (`trunc` is a call on the MPC's ARMv7 and on base x86-64).
    if x.abs() >= 2.0e9 || x.is_nan() {
        return x.rem_euclid(y);
    }
    let n = f64::from((x / y) as i32);
    let mut r = x - n * y;
    if x >= 0.0 {
        if r < 0.0 {
            r += y;
        } else if r >= y {
            r -= y;
        }
    } else if r > 0.0 {
        r -= y;
    } else if r <= -y {
        r += y;
    }
    // fmod: a zero result has x's sign.
    if r == 0.0 {
        r = 0.0f64.copysign(x);
    }
    if r < 0.0 {
        r + y
    } else {
        r
    }
}

/// One loop: its state, its layers and its history. Its audio is in the
/// store, under its number.
#[derive(Clone)]
struct Loop {
    phase: Phase,
    bars: Option<u32>,
    latency: u32,
    level: f32,
    muted: bool,
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
    /// A layer's gains or presence may still be moving (cleared when they
    /// have all settled, so a still mix costs nothing per frame).
    mixing: bool,
}

impl Loop {
    const NEW: Loop = Loop {
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
        mixing: false,
    };

    /// Recording or about to: armed, the first pass, or an overdub.
    fn recording(&self) -> bool {
        matches!(self.phase, Phase::Armed | Phase::Recording) || self.od_open
    }
}

/// The looper: its loops, their shared memory and the command queue.
pub struct Looper {
    store: Store,
    capacity: usize,
    loops: Vec<Loop>,
    selected: usize,
    queue: [Queued; QUEUE],
    queued: usize,
    stats: Stats,
}

impl Looper {
    /// A looper with room for `seconds` of stereo audio at `rate`, shared
    /// by every loop, its first passes and its overdubs, and a little more:
    /// memory comes in blocks of [`BLOCK`] frames, and one block per layer
    /// of a loop is added so that whole layers fill what the seconds
    /// promise. [`Looper::memory`] gives the exact size. Allocates; call
    /// it before the sound starts.
    pub fn new(rate: u32, seconds: u32) -> Looper {
        let frames = (rate as usize) * (seconds as usize);
        let n_blocks = (blocks_for(frames) + MAX_LAYERS).min(NONE as usize);
        Looper {
            store: Store::new(n_blocks),
            capacity: n_blocks * BLOCK,
            loops: vec![Loop::NEW; MAX_LOOPS],
            selected: 0,
            queue: [Queued::Loop(Command::Clear); QUEUE],
            queued: 0,
            stats: Stats::default(),
        }
    }

    /// Queue a command for the selected loop for the next period. False
    /// (and counted) if the queue is full.
    pub fn command(&mut self, c: Command) -> bool {
        self.push(Queued::Loop(c))
    }

    /// Queue a command for layer `id` ([`LayerInfo::id`]) of the selected
    /// loop for the next period. False (and counted) if the queue is full.
    pub fn layer_command(&mut self, id: u8, c: LayerCommand) -> bool {
        self.push(Queued::Layer(id, c))
    }

    /// Select loop `n` (0 to [`MAX_LOOPS`] - 1) for the commands queued
    /// after this. False (and counted) if the queue is full.
    pub fn select_loop(&mut self, n: usize) -> bool {
        self.push(Queued::Select(n))
    }

    /// The loop the commands act on (as of the last period).
    pub fn selected(&self) -> usize {
        self.selected
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

    /// The selected loop.
    pub fn status(&self) -> Status {
        self.loop_status(self.selected)
            .expect("the selected loop is there")
    }

    /// Loop `n`, or None past [`MAX_LOOPS`].
    pub fn loop_status(&self, n: usize) -> Option<Status> {
        let l = self.loops.get(n)?;
        Some(Status {
            phase: l.phase,
            layers: l.layers(),
            redo: l.n_undone,
            bars: l.bars,
            frames: l.len,
            ticks: l.len_ticks,
            origin: l.origin,
            position: if l.len > 0 {
                (l.pos / l.len as f64) as f32
            } else {
                0.0
            },
            ending: l.ending,
            overdub_next: l.then_overdub,
            latency: l.latency,
            level: l.level,
            muted: l.muted,
            capacity: self.capacity,
            used: self.store.blocks(n) * BLOCK,
        })
    }

    /// The memory gauge.
    pub fn memory(&self) -> Memory {
        Memory {
            used: (self.store.n_blocks - self.store.n_free) * BLOCK,
            capacity: self.capacity,
        }
    }

    /// Layer `k` of the selected loop, counting in play order: its audio,
    /// for drawing the waveform. None if there is no such layer.
    pub fn layer(&self, k: usize) -> Option<LayerView<'_>> {
        self.loop_layer(self.selected, k)
    }

    /// Layer `k` of loop `n`, counting in play order.
    pub fn loop_layer(&self, n: usize, k: usize) -> Option<LayerView<'_>> {
        let l = self.loops.get(n)?;
        let slot = l.listed(k)?;
        let valid = l.slots[slot].valid;
        (l.len > 0).then_some(LayerView {
            store: &self.store,
            lp: n,
            slot,
            len: l.len,
            valid,
        })
    }

    /// Layer `k` of the selected loop's id and mix, counting in play order.
    /// None if there is no such layer.
    pub fn layer_info(&self, k: usize) -> Option<LayerInfo> {
        self.loop_layer_info(self.selected, k)
    }

    /// Layer `k` of loop `n`, counting in play order.
    pub fn loop_layer_info(&self, n: usize, k: usize) -> Option<LayerInfo> {
        let l = self.loops.get(n)?;
        let slot = l.listed(k)?;
        let s = &l.slots[slot];
        let any_solo = l.any_solo();
        Some(LayerInfo {
            id: slot as u8,
            level: s.level,
            pan: s.pan,
            muted: s.mute,
            solo: s.solo,
            audible: !s.mute && (!any_solo || s.solo),
            recording: l.od_open && l.od_layer == slot,
        })
    }

    /// Run one period: `input` is the line input and `out` the mix, both
    /// interleaved stereo with `block.frames` frames. The loops are added
    /// to `out`; nothing else is changed.
    pub fn process(&mut self, block: &Block, input: &[f32], out: &mut [f32]) {
        let n = (block.frames as usize)
            .min(input.len() / 2)
            .min(out.len() / 2);
        self.apply_commands();
        let Looper {
            store,
            loops,
            stats,
            ..
        } = self;
        let markers = block.markers();
        let mut m = 0;
        for span in block.spans() {
            let bar = span.meter.bar_ticks();
            let watch_bars = loops
                .iter()
                .any(|l| matches!(l.phase, Phase::Armed | Phase::Recording));
            let mut lines = span.lines(bar);
            let mut next = if watch_bars { lines.next() } else { None };
            // Playing or recording: the loops sound. Counting in: the clock
            // runs, but nothing plays back.
            let playing = matches!(span.state, State::Playing | State::Recording);
            let span_len = span.len as usize;
            let mut i = 0;
            while i < span_len {
                let off = span.offset as usize + i;
                if off >= n {
                    return;
                }
                let f = span.frame + i as u64;
                while m < markers.len() && markers[m].frame <= f {
                    for (k, l) in loops.iter_mut().enumerate() {
                        l.marker(store, stats, k, markers[m].mark, markers[m].frame);
                    }
                    m += 1;
                }
                while let Some((t, o)) = next {
                    if o as usize > off {
                        break;
                    }
                    if o as usize == off {
                        for (k, l) in loops.iter_mut().enumerate() {
                            l.bar_line(store, stats, k, span, t, f);
                        }
                    }
                    next = lines.next();
                }
                // A run of frames up to the next bar line or marker: the
                // loops are independent in it, so each runs it in turn.
                let mut end = span_len.min(i + RUN).min(n - span.offset as usize);
                if let Some((_, o)) = next {
                    end = end.min(o as usize - span.offset as usize);
                }
                if m < markers.len() {
                    end = end.min((markers[m].frame - span.frame) as usize);
                }
                let frames = end - i;
                let mut ticks = [0.0f64; RUN];
                for (j, t) in ticks[..frames].iter_mut().enumerate() {
                    *t = span.tick_at(f + j as u64);
                }
                let mut ys = [[0.0f32; 2]; RUN];
                let run = Run {
                    from: i,
                    ticks: &ticks[..frames],
                    input: &input[2 * off..2 * (off + frames)],
                    f,
                    playing,
                };
                for (k, l) in loops.iter_mut().enumerate() {
                    l.run(store, stats, k, &run, &mut ys[..frames]);
                }
                for (j, y) in ys[..frames].iter().enumerate() {
                    out[2 * (off + j)] += y[0];
                    out[2 * (off + j) + 1] += y[1];
                }
                i = end;
            }
        }
        while m < markers.len() {
            for (k, l) in loops.iter_mut().enumerate() {
                l.marker(store, stats, k, markers[m].mark, markers[m].frame);
            }
            m += 1;
        }
    }

    fn apply_commands(&mut self) {
        // Commands that would cut a fade short wait for it, in order. Once
        // a selection waits, everything after it waits too, so no command
        // reaches the wrong loop.
        let mut kept = 0;
        let mut waiting = false;
        let mut hold_all = false;
        for k in 0..self.queued {
            let q = self.queue[k];
            let deferrable = matches!(
                q,
                Queued::Loop(Command::Record | Command::Overdub | Command::Undo | Command::Redo)
                    | Queued::Layer(_, LayerCommand::Delete)
            );
            let wait = hold_all
                || (deferrable && (waiting || self.loops[self.selected].must_wait(q)))
                || (waiting && matches!(q, Queued::Select(_)));
            if wait {
                self.queue[kept] = q;
                kept += 1;
                waiting = true;
                hold_all |= matches!(q, Queued::Select(_));
            } else {
                self.apply(q);
            }
        }
        self.queued = kept;
    }

    fn apply(&mut self, q: Queued) {
        let s = self.selected;
        match q {
            Queued::Select(n) if n < MAX_LOOPS => self.selected = n,
            Queued::Select(_) => self.stats.no_layer += 1,
            Queued::Loop(Command::SetLatency(l)) => {
                for lp in &mut self.loops {
                    lp.latency = l;
                }
            }
            Queued::Loop(c) => {
                let was = self.loops[s].recording();
                self.loops[s].apply(&mut self.store, &mut self.stats, s, c);
                if !was && self.loops[s].recording() {
                    self.others_stop_recording(s);
                }
            }
            Queued::Layer(id, c) => {
                let was = self.loops[s].recording();
                self.loops[s].apply_layer(&mut self.store, &mut self.stats, s, id, c);
                if !was && self.loops[s].recording() {
                    self.others_stop_recording(s);
                }
            }
        }
        self.loops[s].reclaim(&mut self.store, s);
    }

    /// One input, one recording: loop `s` starts recording, so the others
    /// stop. An overdub punches out, a first pass ends on the next bar line
    /// (on its bar line if free length), an armed loop disarms.
    fn others_stop_recording(&mut self, s: usize) {
        for (k, l) in self.loops.iter_mut().enumerate() {
            if k == s {
                continue;
            }
            match l.phase {
                Phase::Armed => l.phase = Phase::Empty,
                Phase::Recording => {
                    l.ending = true;
                    l.end_tick = None;
                    l.then_overdub = false;
                }
                _ if l.od_open => l.punch_out(),
                _ => {}
            }
            l.reclaim(&mut self.store, k);
        }
    }
}

impl Loop {
    /// Layers in play order, not counting ones fading away.
    fn layers(&self) -> usize {
        self.order[..self.in_order]
            .iter()
            .filter(|&&k| !self.slots[k as usize].leaving)
            .count()
    }

    /// The slot of the `k`th layer not fading away.
    fn listed(&self, k: usize) -> Option<usize> {
        self.order[..self.in_order]
            .iter()
            .map(|&s| s as usize)
            .filter(|&s| !self.slots[s].leaving)
            .nth(k)
    }

    fn any_solo(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].solo && !self.slots[k as usize].leaving)
    }

    /// Whether `slot` is playing, or Undo or Redo can bring it back.
    fn holds(&self, slot: usize) -> bool {
        self.live(slot)
            || self.undone[..self.n_undone]
                .iter()
                .any(|a| a.slot as usize == slot)
    }

    /// Whether `slot` is playing, or Undo can bring it back.
    fn live(&self, slot: usize) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| k as usize == slot)
            || self.done[..self.n_done]
                .iter()
                .any(|a| a.slot as usize == slot)
    }

    /// The first pass is being recorded into slot 0.
    fn first_in_progress(&self) -> bool {
        matches!(self.phase, Phase::Recording) || self.first_open
    }

    /// Give back the memory of every slot that can no longer come back.
    fn reclaim(&self, store: &mut Store, me: usize) {
        for k in 0..MAX_LAYERS {
            let in_use = self.holds(k)
                || (k == 0 && self.first_in_progress())
                || (self.od_open && self.od_layer == k);
            if !in_use && store.held[me][k] > 0 {
                store.release(me, k);
            }
        }
    }

    fn must_wait(&self, q: Queued) -> bool {
        match q {
            Queued::Loop(Command::Undo) => self.leaving() || self.clearing,
            // A new overdub waits while the last one's tail is written, so
            // that tail still fades out.
            Queued::Loop(Command::Record | Command::Overdub)
                if self.od_open && self.phase == Phase::Playing =>
            {
                true
            }
            _ => self.fading() || self.clearing,
        }
    }

    /// A layer is fading out of the play order.
    fn leaving(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].leaving)
    }

    /// A layer is fading in or out.
    fn fading(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].leaving || self.slots[k as usize].presence < 1.0)
    }

    /// Move the loop's position on a frame; `tick` is that frame's song
    /// position. While playing it is the song position's place in the
    /// loop; while not, it runs on so the sound fades out where it was
    /// going. A jump starts a crossfade.
    fn follow(&mut self, tick: f64, playing: bool, first: bool) {
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
        let rel = rem_ticks(tick - self.origin as f64, self.len_ticks as f64);
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

    fn apply_layer(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        id: u8,
        c: LayerCommand,
    ) {
        self.mixing = true;
        let slot = id as usize;
        let at = self.order[..self.in_order].iter().position(|&k| k == id);
        let Some(at) = at.filter(|_| !self.slots[slot].leaving) else {
            stats.no_layer += 1;
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
            LayerCommand::Delete if self.od_open && self.od_layer == slot => {
                self.apply(store, stats, me, Command::Undo)
            }
            LayerCommand::Delete if self.layers() > 1 => {
                self.n_undone = 0;
                self.push_done(Act {
                    deleted: true,
                    slot: id,
                    index: at as u8,
                });
                self.slots[slot].leaving = true;
            }
            LayerCommand::Delete => {}
        }
    }

    fn apply(&mut self, store: &mut Store, stats: &mut Stats, me: usize, c: Command) {
        self.mixing = true;
        match c {
            Command::Record => match self.phase {
                Phase::Empty => {
                    self.phase = Phase::Armed;
                    self.ring = [0.0; 2 * FADE];
                }
                Phase::Armed => self.phase = Phase::Empty,
                Phase::Recording => self.ending = true,
                Phase::Playing | Phase::Overdubbing => {
                    self.apply(store, stats, me, Command::Overdub)
                }
            },
            Command::Overdub => match self.phase {
                Phase::Playing => self.open_overdub(store, stats, me, FADE),
                Phase::Overdubbing => self.punch_out(),
                Phase::Recording => self.then_overdub = !self.then_overdub,
                _ => {}
            },
            Command::Undo => match self.phase {
                Phase::Armed | Phase::Recording => self.abort(stats),
                Phase::Overdubbing | Phase::Playing if self.od_open => {
                    // Throw the overdub away (recording, or still writing
                    // its tail): it stops and fades out, and Redo can
                    // bring back what it recorded.
                    self.od_open = false;
                    self.phase = Phase::Playing;
                    let slot = self.od_layer;
                    if !self.drop_empty_overdub() {
                        if let Some(i) = self.done[..self.n_done]
                            .iter()
                            .rposition(|a| !a.deleted && a.slot as usize == slot)
                        {
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
                Phase::Armed | Phase::Recording => self.abort(stats),
                Phase::Playing | Phase::Overdubbing => {
                    self.od_open = false;
                    self.clearing = true;
                }
            },
            Command::Mute(m) => self.muted = m,
            Command::SetBars(b) => self.bars = b.filter(|&b| b > 0),
            Command::SetLatency(l) => self.latency = l,
            Command::SetLevel(v) => {
                self.level = if v.is_finite() {
                    v.clamp(0.0, 2.0)
                } else {
                    self.level
                }
            }
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
        self.mixing = true;
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
        self.mixing = true;
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

    /// Back to empty, keeping the settings. The memory goes back when the
    /// command is done (`reclaim`).
    fn reset(&mut self) {
        self.mixing = true;
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

    fn abort(&mut self, stats: &mut Stats) {
        if self.phase == Phase::Recording {
            stats.aborted += 1;
        }
        self.reset();
    }

    fn marker(&mut self, store: &mut Store, stats: &mut Stats, me: usize, mark: Mark, frame: u64) {
        if self.phase != Phase::Recording {
            return;
        }
        match mark {
            // The transport looped back at `end`: if that is where the pass
            // ends (or Record asked for the next bar), it ends here.
            Mark::Looped { end, .. }
                if self.end_tick == Some(end) || (self.ending && self.end_tick.is_none()) =>
            {
                self.end_tick = Some(end);
                self.finish_first(store, stats, me, end, frame);
            }
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Stopped { .. } | Mark::Panic => {
                stats.aborted += 1;
                self.reset();
                self.reclaim(store, me);
            }
            _ => {}
        }
    }

    fn bar_line(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        span: &Span,
        t: Tick,
        f: u64,
    ) {
        match self.phase {
            Phase::Recording if t > self.origin && (self.end_tick == Some(t) || self.ending) => {
                self.finish_first(store, stats, me, t, f);
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

    fn finish_first(&mut self, store: &mut Store, stats: &mut Stats, me: usize, t: Tick, f: u64) {
        self.mixing = true;
        let len = (f - self.start_frame) as usize;
        if len == 0 || !store.grow(me, 0, blocks_for(len)) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        self.len = len;
        self.len_ticks = t - self.origin;
        self.order[0] = 0;
        self.in_order = 1;
        self.slots[0] = Slot {
            valid: (0, len),
            ..Slot::NEW
        };
        self.n_done = 0;
        self.n_undone = 0;
        self.ending = false;
        self.pos = 0.0;
        self.jump = 0;
        self.phase = Phase::Playing;
        if self.then_overdub {
            self.then_overdub = false;
            self.open_overdub(store, stats, me, HANDOFF_FADE);
        }
    }

    /// The first pass's writer: frame `f`'s input belongs `latency` frames
    /// earlier, so the loop's frame `j` is written at `start + latency + j`.
    fn write_first(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        f: u64,
        x: [f32; 2],
    ) {
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
            self.seal(store, me);
            return;
        }
        if !store.grow(me, 0, j / BLOCK + 1) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        store.set(me, 0, j, x);
    }

    /// The loop point: the end fades into the input from just before the
    /// start, which flows on into the start.
    fn seal(&mut self, store: &mut Store, me: usize) {
        self.first_open = false;
        let fade = FADE.min(self.len / 2);
        for k in 0..fade {
            let w = (k as f32 + 0.5) / fade as f32;
            let p = FADE - fade + k;
            let idx = self.len - fade + k;
            let a = store.get(me, 0, idx);
            let y = [
                a[0] * (1.0 - w) + self.pre[2 * p] * w,
                a[1] * (1.0 - w) + self.pre[2 * p + 1] * w,
            ];
            store.set(me, 0, idx, y);
        }
    }

    /// A slot for a new overdub, with the memory for a whole pass: one no
    /// layer plays and Undo cannot bring back (what Redo could bring back
    /// goes, as a new overdub ends redo). With no room, the history up to
    /// its oldest delete is forgotten, so that layer's memory can be used.
    /// None if there is still no room.
    fn free_slot(&mut self, store: &mut Store, me: usize) -> Option<usize> {
        let need = blocks_for(self.len);
        loop {
            // Blocks free now, plus what dropping redo would give back.
            let redo_blocks: usize = (0..MAX_LAYERS)
                .filter(|&k| !self.live(k))
                .map(|k| store.held[me][k] as usize)
                .sum();
            let slot = (0..MAX_LAYERS).find(|&k| !self.live(k));
            if let Some(k) = slot.filter(|_| store.n_free + redo_blocks >= need) {
                self.n_undone = 0;
                self.reclaim(store, me);
                return store.grow(me, k, need).then_some(k);
            }
            let i = self.done[..self.n_done].iter().position(|a| a.deleted)?;
            self.done.copy_within(i + 1..self.n_done, 0);
            self.n_done -= i + 1;
        }
    }

    /// A new layer on top, written from now on, fading in over `fade`
    /// frames.
    fn open_overdub(&mut self, store: &mut Store, stats: &mut Stats, me: usize, fade: usize) {
        self.mixing = true;
        let Some(k) = self.free_slot(store, me) else {
            stats.refused += 1;
            return;
        };
        self.slots[k] = Slot::NEW;
        self.order[self.in_order] = k as u8;
        self.push_done(Act {
            deleted: false,
            slot: k as u8,
            index: self.in_order as u8,
        });
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
        if let Some(at) = self.order[..self.in_order]
            .iter()
            .position(|&s| s as usize == k)
        {
            self.remove_at(at);
        }
        if let Some(i) = self.done[..self.n_done]
            .iter()
            .rposition(|a| !a.deleted && a.slot as usize == k)
        {
            self.take_done(i);
        }
        true
    }

    fn close_overdub(&mut self, store: &mut Store, me: usize) {
        self.mixing = true;
        self.od_open = false;
        self.drop_empty_overdub();
        if self.phase == Phase::Overdubbing {
            self.phase = Phase::Playing;
        }
        self.reclaim(store, me);
    }

    /// One frame of the overdub writer. It waits for the first pass's
    /// writer to finish (it runs `latency` frames behind, so this input is
    /// still the first pass), and while the transport is stopped (Overdub
    /// pressed before Play); it fades out when the transport stops.
    fn overdub_frame(&mut self, store: &mut Store, me: usize, x: [f32; 2], playing: bool) {
        if self.first_open {
            return;
        }
        if !playing {
            if self.od_in == 0 {
                if self.od_stop_in.is_some() {
                    // Punched out before it began.
                    self.close_overdub(store, me);
                }
                return;
            }
            self.od_stop_in = Some(0);
        }
        self.write_overdub(store, me, x);
    }

    fn write_overdub(&mut self, store: &mut Store, me: usize, x: [f32; 2]) {
        let len = self.len;
        let mut g = if self.od_in < self.od_fade {
            (self.od_in as f32 + 0.5) / self.od_fade as f32
        } else {
            1.0
        };
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
        let q = ((self.pos - f64::from(self.latency))
            .rem_euclid(len as f64)
            .round() as usize)
            % len;
        let k = self.od_layer;
        let (start, count) = self.slots[k].valid;
        let y = [x[0] * g, x[1] * g];
        if count == 0 {
            self.slots[k].valid = (q, 1);
            store.set(me, k, q, y);
        } else if count < len {
            self.slots[k].valid = (start, count + 1);
            store.set(me, k, q, y);
        } else {
            store.add(me, k, q, y);
        }
        if last {
            self.close_overdub(store, me);
        }
    }

    /// The loop's part of a run: each frame's output is added to `ys`.
    /// Frame by frame while anything moves (writers, fades, a jump); once
    /// the loop is still, the rest of the run in one go.
    fn run(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        r: &Run,
        ys: &mut [[f32; 2]],
    ) {
        for j in 0..ys.len() {
            if self.phase == Phase::Empty {
                return;
            }
            let first = r.from + j == 0;
            if !first && self.still(r.playing) {
                self.still_run(store, me, &r.ticks[j..], &mut ys[j..]);
                return;
            }
            let x = [r.input[2 * j], r.input[2 * j + 1]];
            self.write_first(store, stats, me, r.f + j as u64, x);
            self.follow(r.ticks[j], r.playing, first);
            let v = self.read(store, me, r.playing);
            ys[j][0] += v[0];
            ys[j][1] += v[1];
            if self.od_open {
                self.overdub_frame(store, me, x, r.playing);
            }
        }
    }

    /// Playing at full gain with nothing moving: no writer, fade, mix
    /// change or jump. Only the position changes from frame to frame
    /// (a jump can only come at a span's first frame).
    fn still(&self, playing: bool) -> bool {
        self.phase == Phase::Playing
            && playing
            && self.gain == 1.0
            && !self.muted
            && !self.clearing
            && !self.od_open
            && !self.first_open
            && !self.mixing
            && self.jump == 0
            && self.len > 0
    }

    /// Frames of a still loop ([`Loop::still`]): the same sums as `read`
    /// and `sample_at` frame by frame, in the same order, so the same bits,
    /// but layer by layer.
    fn still_run(&mut self, store: &Store, me: usize, ticks: &[f64], ys: &mut [[f32; 2]]) {
        let len = self.len;
        let n = ys.len();
        let mut i0s = [0usize; RUN];
        let mut i1s = [0usize; RUN];
        let mut fracs = [0.0f32; RUN];
        for j in 0..n {
            self.follow(ticks[j], true, false);
            let i0 = (self.pos as usize).min(len - 1);
            fracs[j] = (self.pos - i0 as f64) as f32;
            i0s[j] = i0;
            i1s[j] = if i0 + 1 == len { 0 } else { i0 + 1 };
        }
        let mut acc = [[0.0f32; 2]; RUN];
        for &k in &self.order[..self.in_order] {
            let k = k as usize;
            let s = &self.slots[k];
            let g = [s.gl * s.presence, s.gr * s.presence];
            if g == [0.0, 0.0] {
                continue;
            }
            let (start, count) = s.valid;
            let full = count >= len;
            let blocks = store.blocks_of(me, k);
            for j in 0..n {
                let (i0, i1) = (i0s[j], i1s[j]);
                let in0 = full || (i0 + len - start) % len < count;
                let in1 = full || (i1 + len - start) % len < count;
                if !in0 && !in1 {
                    continue;
                }
                let a = if in0 {
                    store.frame(blocks, i0 / BLOCK, i0 % BLOCK)
                } else {
                    [0.0; 2]
                };
                let b = if in1 {
                    store.frame(blocks, i1 / BLOCK, i1 % BLOCK)
                } else {
                    [0.0; 2]
                };
                for c in 0..2 {
                    acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                }
            }
        }
        // `read`: the gain stays at 1 (it only rises to 1 while audible).
        let g = self.gain * self.level;
        for (y, a) in ys.iter_mut().zip(&acc) {
            y[0] += a[0] * g;
            y[1] += a[1] * g;
        }
    }

    fn read(&mut self, store: &mut Store, me: usize, playing: bool) -> [f32; 2] {
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
            self.reclaim(store, me);
            return [0.0; 2];
        }
        self.mix_layers();
        if self.gain == 0.0 || self.len == 0 {
            return [0.0; 2];
        }
        let mut y = self.sample_at(store, me, self.pos);
        if self.jump > 0 {
            let w = self.jump as f32 * STEP;
            let old = self.sample_at(store, me, self.jump_pos);
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
        if !self.mixing {
            return;
        }
        let any_solo = self.any_solo();
        let mut settled = true;
        let mut p = 0;
        while p < self.in_order {
            let s = &mut self.slots[self.order[p] as usize];
            if s.leaving {
                s.presence = (s.presence - STEP).max(0.0);
                if s.presence == 0.0 {
                    s.leaving = false;
                    self.remove_at(p);
                    settled = false;
                    continue;
                }
            } else if s.presence < 1.0 {
                s.presence = (s.presence + STEP).min(1.0);
            }
            let (l, r) = s.target(any_solo);
            s.gl += (l - s.gl).clamp(-STEP, STEP);
            s.gr += (r - s.gr).clamp(-STEP, STEP);
            settled &= !s.leaving && s.presence == 1.0 && s.gl == l && s.gr == r;
            p += 1;
        }
        self.mixing = !settled;
    }

    /// Every layer at `pos`, read between frames, at its gains. Only the
    /// frames a layer has written count.
    fn sample_at(&self, store: &Store, me: usize, pos: f64) -> [f32; 2] {
        let len = self.len;
        let i0 = (pos as usize).min(len - 1);
        let frac = (pos - i0 as f64) as f32;
        let i1 = if i0 + 1 == len { 0 } else { i0 + 1 };
        let (b0, o0, b1, o1) = (i0 / BLOCK, i0 % BLOCK, i1 / BLOCK, i1 % BLOCK);
        let mut y = [0.0f32; 2];
        for &k in &self.order[..self.in_order] {
            let k = k as usize;
            let s = &self.slots[k];
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
            let blocks = store.blocks_of(me, k);
            let a = if in0 {
                store.frame(blocks, b0, o0)
            } else {
                [0.0; 2]
            };
            let b = if in1 {
                store.frame(blocks, b1, o1)
            } else {
                [0.0; 2]
            };
            for (c, v) in y.iter_mut().enumerate() {
                *v += g[c] * (a[c] + (b[c] - a[c]) * frac);
            }
        }
        y
    }
}

#[cfg(test)]
mod tests {
    use super::rem_ticks;

    /// Bit for bit what `rem_euclid` gives, over song positions and loop
    /// lengths of every size, whole multiples and the places either side.
    #[test]
    fn rem_ticks_is_rem_euclid() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut checked = 0;
        for _ in 0..200_000 {
            let y = (next() % 2_000_000 + 1) as f64;
            let scale = [1.0, 10.0, 1e3, 1e5, 1e7, 1.9e9][(next() % 6) as usize];
            let unit = (next() % 1_000_000) as f64 / 1_000_000.0;
            let sign = if next() % 2 == 0 { 1.0 } else { -1.0 };
            let x = sign * unit * scale;
            let k = (next() % 1000) as f64 * sign;
            for x in [
                x,
                k * y,
                f64::from_bits((k * y).to_bits() + 1),
                f64::from_bits((k * y).to_bits().saturating_sub(1)),
                x / 7.0,
            ] {
                let (a, b) = (rem_ticks(x, y), x.rem_euclid(y));
                assert_eq!(a.to_bits(), b.to_bits(), "{x} rem {y}: {a} against {b}");
                checked += 1;
            }
        }
        for (x, y) in [
            (0.0, 3840.0),
            (-0.0, 3840.0),
            (-1e-300, 7680.0),
            (f64::from_bits(3840.0f64.to_bits() - 1), 3840.0),
            (2.5e9, 3840.0),
            (-2.5e9, 3.0),
        ] {
            assert_eq!(
                rem_ticks(x, y).to_bits(),
                x.rem_euclid(y).to_bits(),
                "{x} rem {y}"
            );
        }
        assert_eq!(checked, 1_000_000);
    }
}
