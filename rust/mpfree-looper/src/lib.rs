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
//!   faster or slower, like tape, until a print for that tempo is in.
//! - **Prints: another tempo, the same pitch.** A print ([`Print`]) is one
//!   pass of the loop's layers as heard, stretched to the loop's length at
//!   another tempo with its pitch kept ([`print`] makes one from a
//!   [`Looper::bounce_print`], on a worker). Handed in with
//!   [`Looper::print`], it plays in place of the layers while the loop is
//!   as it was when bounced ([`Status::version`]), crossfading in and out
//!   over [`FADE`] frames. Any change to the layers (an overdub, an undo,
//!   an edit, a layer's level) stops it until a new one comes.
//! - **Fit.** Overdub (or Record) while a print plays first fits the loop
//!   to the tempo: the print becomes the loop's one layer, at its length,
//!   and the overdub then starts, sample for sample at the new tempo. It
//!   copies at [`FIT_SPEED`], so the overdub waits about a 64th of the
//!   loop. Undo takes the fit back (the loop dips over [`FADE`] frames and
//!   comes back with its layers as they were). [`Command::Fit`] fits
//!   without overdubbing.
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

pub mod print;
mod view;

use view::{Cursor, View};

use std::ops::Index;

use mpfree_transport::time::{Tick, PPQ};
use mpfree_transport::{Block, Mark, Span, State};

/// Frames of every fade: the loop point, punch in and out, start, stop,
/// mute, undo and clear (about 5.8 ms at 44.1 kHz).
pub const FADE: usize = 256;
/// Layer slots in one loop. A loop plays at most one less (the first pass
/// and 14 overdubs), so that a merge always has a slot to render into.
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
/// Overdubs, deletes and edits Undo can take back, per loop.
pub const HISTORY: usize = 64;
/// Edits Undo can take back, per loop (each keeps the loop's shape before
/// it).
const EDITS: usize = 16;
/// The longest seam fade, in frames (about 186 ms at 44.1 kHz).
pub const MAX_SEAM: usize = 8192;
/// A 1/16 note, in ticks: the nudge step and the shortest loop an edit
/// makes.
const SIXTEENTH: Tick = PPQ as Tick / 4;
/// A merge renders this many frames of the merged layer for each frame
/// that plays, so it is done in half the loop's length.
pub const MERGE_SPEED: usize = 2;
/// A print's bounce ([`Looper::bounce_print`]) renders this many frames
/// for each frame that plays.
pub const PRINT_SPEED: usize = 8;
/// A fit copies the print into the loop at this many frames for each
/// frame that plays (a plain copy).
pub const FIT_SPEED: usize = 64;
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
    /// What the loop does when its first pass ends (see [`After`]).
    SetAfter(After),
    /// Fade old: the share of the older audio that stays each time the loop
    /// comes round while overdubbing, 0 to 1 (1, the default, fades
    /// nothing; 0.88 keeps 88 % a pass). The older layers turn down by it
    /// (each layer's [`LayerInfo::faded`]; their own levels stay as set),
    /// and the layer being recorded keeps that share of its earlier
    /// passes. Undo of the overdub turns the older layers back up.
    SetFadeOld(f32),
    /// Input latency to make up for, in frames (output buffer plus input
    /// buffer plus converters; measure it with a cable from out to in).
    /// For every loop.
    SetLatency(u32),
    /// The loop's level, 0 to 2 (1 is as recorded).
    SetLevel(f32),
    /// Change the loop: see [`Edit`]. Undo and Redo take edits back and
    /// forth with the overdubs and deletes.
    Edit(Edit),
    /// Bounce the layers into one, as heard: through their edits, at
    /// their levels, pans, mutes and solos (the loop's own level and mute
    /// stay outside). It takes memory for one layer and renders in the
    /// background at [`MERGE_SPEED`] while the loop plays on; then the
    /// merged layer takes the layers' place, which sounds the same. Undo
    /// brings the layers back (and, while it renders, stops it). Commands
    /// for the loop wait while it renders, except Undo, Clear, Mute and
    /// the loop's level. Refused when the loop is not playing, is
    /// recording, has nothing to merge or has no memory for it.
    Merge,
    /// As Record, but the first pass records the other loops as they are
    /// heard (after their levels and mutes), sample for sample, instead of
    /// the input, with no latency to make up for. Refused once the loop
    /// has a first pass.
    Resample,
    /// Fit the loop to the tempo its print plays at: the print becomes
    /// the loop's one layer, at the print's length (see the module notes).
    /// Undo takes it back. Refused when no print plays.
    Fit,
}

/// A loop rendered out ([`Looper::bounce`]), for a pad or a WAV file.
#[derive(Debug)]
pub struct Bounce {
    /// The loop it came from.
    pub from_loop: usize,
    /// One pass of the loop, interleaved stereo, as heard: its layers
    /// through their edits and mixes, at the loop's level (not muted).
    pub audio: Vec<f32>,
    /// Frames in `audio`.
    pub frames: usize,
    /// The loop's length in ticks, and where it starts in the song.
    pub ticks: Tick,
    pub origin: Tick,
    /// False if the loop was cleared before it was done: `audio` then
    /// holds what was rendered, and silence after.
    pub complete: bool,
    /// The loop's version when it was bounced ([`Status::version`]).
    pub version: u64,
    /// A print's bounce ([`Looper::bounce_print`]): without the loop's
    /// level.
    pub dry: bool,
}

/// A loop made to fit another tempo with its pitch kept: one pass of its
/// layers as heard (without the loop's own level and mute), stretched to
/// `frames` frames. [`print`] makes one; [`Looper::print`] hands it in.
/// It plays from `origin`, `ticks` long, like the loop, so at the tempo it
/// was made for it plays sample for sample.
#[derive(Debug)]
pub struct Print {
    /// Interleaved stereo, at least `frames` long.
    pub audio: Vec<f32>,
    pub frames: usize,
    /// The loop's length in ticks and its start, when it was bounced.
    pub ticks: Tick,
    pub origin: Tick,
    /// The loop's version it was made from ([`Status::version`]).
    pub version: u64,
}

impl Print {
    /// Frame `p` of the print, read between frames like the layers.
    #[inline]
    fn frame_at(&self, p: f64) -> [f32; 2] {
        let len = self.frames;
        let i0 = (p as usize).min(len - 1);
        let frac = (p - i0 as f64) as f32;
        let i1 = if i0 + 1 == len { 0 } else { i0 + 1 };
        let a = [self.audio[2 * i0], self.audio[2 * i0 + 1]];
        let b = [self.audio[2 * i1], self.audio[2 * i1 + 1]];
        [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
    }
}

/// A loop's prints: the one playing (or fading out), the next one waiting
/// for it to fade out (`Some(None)` takes it away), and the last one taken
/// out, for the host to drop ([`Looper::take_print`]).
#[derive(Debug, Default)]
struct Prints {
    now: Option<Print>,
    next: Option<Option<Print>>,
    old: Option<Print>,
}

impl Bounce {
    /// The audio as a WAV file (32-bit float, stereo, at `rate`), which
    /// the sampler reads. Allocates: not for the audio thread.
    pub fn wav(&self, rate: u32) -> Vec<u8> {
        let data = 4 * self.audio.len();
        let mut b = Vec::with_capacity(44 + data);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data as u32).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&3u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 8).to_le_bytes());
        b.extend_from_slice(&8u16.to_le_bytes());
        b.extend_from_slice(&32u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data as u32).to_le_bytes());
        for v in &self.audio {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b
    }
}

/// A bounce in progress: the loop, its buffer and frames rendered.
struct Bouncing {
    n: usize,
    buf: Vec<f32>,
    done: usize,
    complete: Option<bool>,
    /// Without the loop's level, at [`PRINT_SPEED`] (a print's bounce).
    dry: bool,
    version: u64,
}

/// The loop edits. None of them changes the audio: each layer plays its
/// audio through a view, which an edit changes, so an edit costs no memory
/// and no time, and Undo brings back exactly what was there. Every layer
/// of the loop is edited together, and overdubs recorded afterwards fit
/// the edited loop. An edit crossfades from the loop before it over
/// [`FADE`] frames. Edits apply while the loop plays and is not recording
/// (counted in [`Stats::refused`] otherwise, or when an edit cannot be
/// made).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Keep the first half: half the length (at least a 1/16 note).
    Half,
    /// Twice the length, the loop played twice: overdubs can then make the
    /// two halves differ.
    Double,
    /// Play the loop backwards.
    Reverse,
    /// Move the audio by this many 1/16 notes, later (above 0) or earlier,
    /// going round the loop.
    Nudge(i32),
    /// Keep ticks `from..to` of the loop (from its start), at least a 1/16
    /// note: that becomes the loop, and it stays where it was in the song.
    Trim { from: Tick, to: Tick },
    /// Frames of the crossfade at every cut the edits make (where one piece
    /// of audio jumps to another, the loop point of a trimmed loop
    /// included), up to [`MAX_SEAM`]; 0 is a straight cut. [`FADE`] until
    /// changed.
    SeamFade(u32),
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

/// One step of a loop's history ([`Looper::history`]), as the Looper tab's
/// undo history shows it. Layers are named by their id, as in
/// [`LayerInfo`]. The first pass is not a step: Clear takes it away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// An overdub, which added this layer (it may still be recording).
    Overdub(u8),
    /// This layer was deleted.
    Delete(u8),
    Edit(Edit),
    /// The layers were merged into this one.
    Merge(u8),
    /// The loop was fitted to another tempo: its print became this layer.
    Fit(u8),
}

/// A loop's history: the steps Undo takes back and the ones Redo brings
/// back. It holds the last [`HISTORY`] steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct History {
    done: [Step; HISTORY],
    n_done: usize,
    redo: [Step; HISTORY],
    n_redo: usize,
}

impl History {
    /// The steps that can be undone, oldest first: Undo takes back the
    /// last.
    pub fn done(&self) -> &[Step] {
        &self.done[..self.n_done]
    }

    /// The steps that can be redone, the next Redo first.
    pub fn redo(&self) -> &[Step] {
        &self.redo[..self.n_redo]
    }
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
    /// How far Fade old has turned it down, 0 to 1 (1: not at all). It
    /// plays at `level * faded`.
    pub faded: f32,
}

/// What a loop does when its first pass ends: the Looper tab's "After
/// recording" setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum After {
    /// Play the loop.
    #[default]
    Play,
    /// Go straight into an overdub (Overdub during the first pass
    /// cancels it, and presses again bring it back). Not after a
    /// resample, whose overdub would take the input.
    Overdub,
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
    /// Overdubs refused because there was no memory or no layer slot,
    /// edits refused (the loop was recording, the result would be too
    /// short, or too cut up: a view holds at most 8 pieces), and merges
    /// refused (not playing, recording, nothing to merge, no memory).
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
    /// An overdub starts when the first pass ends: Overdub was pressed
    /// during it, or the loop is set to ([`After::Overdub`]).
    pub overdub_next: bool,
    pub after: After,
    /// Fade old ([`Command::SetFadeOld`]).
    pub fade_old: f32,
    pub latency: u32,
    pub level: f32,
    pub muted: bool,
    /// Frames of the crossfade at the cuts edits make.
    pub seam: usize,
    /// A merge rendering: how much is done, 0 to 1.
    pub merging: Option<f32>,
    /// Armed or recording a first pass from the other loops
    /// ([`Command::Resample`]).
    pub resampling: bool,
    /// The memory all the loops share, in frames.
    pub capacity: usize,
    /// Frames of memory this loop holds: its layers and the ones Undo or
    /// Redo can bring back, in whole blocks.
    pub used: usize,
    /// Changes whenever the layers' sound does (an overdub, an undo, an
    /// edit, a layer's mix): a print made at one version plays only while
    /// the loop is still at it.
    pub version: u64,
    /// A print plays in place of the layers (fully, or fading in or out).
    pub printed: bool,
    /// A fit copying the print in: how much is done, 0 to 1.
    pub fitting: Option<f32>,
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
    /// The frames of its audio written: (first, count), wrapping.
    valid: (usize, usize),
    /// How the loop's edits play its audio.
    view: View,
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
        let size = self.view.size();
        if size == 0 {
            return &SILENCE;
        }
        let q = if self.view.is_whole() {
            i / 2
        } else {
            self.view.at(i / 2)
        };
        let (start, count) = self.valid;
        if (q + size - start) % size >= count {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Overdub,
    Delete,
    Edit,
    Merge,
    Fit,
}

/// An overdub added, a layer deleted, an edit or a merge, for Undo and
/// Redo: for an overdub or a delete, the layer's slot and its place in the
/// play order; for an edit, where the loop's shape before it is kept; for
/// a merge, the merged layer's slot, and where the play order it did not
/// choose (the layers before it, or after an undo the merged one) is kept.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Act {
    kind: Kind,
    slot: u8,
    index: u8,
    /// The edit, for the history the screen shows.
    edit: Option<Edit>,
    /// An overdub's: every layer's Fade old turn-down from the other side
    /// of it (before it while done, after it while undone).
    ages: [f32; MAX_LAYERS],
}

const NO_ACT: Act = Act {
    kind: Kind::Overdub,
    slot: 0,
    index: 0,
    edit: None,
    ages: [1.0; MAX_LAYERS],
};

impl Act {
    fn step(&self) -> Step {
        match self.kind {
            Kind::Overdub => Step::Overdub(self.slot),
            Kind::Delete => Step::Delete(self.slot),
            Kind::Edit => Step::Edit(self.edit.unwrap_or(Edit::Half)),
            Kind::Merge => Step::Merge(self.slot),
            Kind::Fit => Step::Fit(self.slot),
        }
    }

    /// Where the act keeps a shape, if it does.
    fn shape(&self) -> Option<usize> {
        match self.kind {
            Kind::Edit => Some(self.slot as usize),
            Kind::Merge | Kind::Fit => Some(self.index as usize),
            Kind::Overdub | Kind::Delete => None,
        }
    }
}

/// What an edit changes: the loop's length and place, its seam fade, and
/// every layer's view. A merge keeps a play order in one.
#[derive(Clone, Copy)]
struct Shape {
    len: usize,
    len_ticks: Tick,
    origin: Tick,
    seam: usize,
    views: [View; MAX_LAYERS],
    order: [u8; MAX_LAYERS],
    in_order: usize,
}

const NO_SHAPE: Shape = Shape {
    len: 0,
    len_ticks: 0,
    origin: 0,
    seam: FADE,
    views: [View::EMPTY; MAX_LAYERS],
    order: [0; MAX_LAYERS],
    in_order: 0,
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
    /// How far Fade old has turned it down: 1 not at all.
    age: f32,
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
        age: 1.0,
    };

    /// The gains the settings ask for.
    fn target(&self, any_solo: bool) -> (f32, f32) {
        if self.mute || (any_solo && !self.solo) {
            return (0.0, 0.0);
        }
        let level = self.level * self.age;
        (
            level * (1.0 - self.pan).min(1.0),
            level * (1.0 + self.pan).min(1.0),
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

/// Playback frame `p` of a layer through its view: the audio frame the
/// view plays there, crossfaded into the next piece over `seam` frames
/// before each cut. Only the audio frames the layer has written count.
#[inline]
fn layer_frame(
    store: &Store,
    blocks: &[u16],
    valid: (usize, usize),
    v: &View,
    seam: usize,
    p: usize,
) -> [f32; 2] {
    if v.size() == 0 {
        return [0.0; 2];
    }
    let (i, off) = v.locate(p);
    piece_frame(store, blocks, valid, v, seam, i, off)
}

/// [`layer_frame`] at frame `off` of piece `i` (where `View::locate`
/// puts it), for a view of some audio.
#[inline(always)]
fn piece_frame(
    store: &Store,
    blocks: &[u16],
    valid: (usize, usize),
    v: &View,
    seam: usize,
    i: usize,
    off: usize,
) -> [f32; 2] {
    let size = v.size();
    let (start, count) = valid;
    let fetch = |a: usize| {
        // `(a + size - start) % size`: frames from the first one written.
        let d = a + size - start;
        let d = if d < size { d } else { d - size };
        if count >= size || d < count {
            store.frame(blocks, a / BLOCK, a % BLOCK)
        } else {
            [0.0; 2]
        }
    };
    let pieces = v.pieces();
    let piece = &pieces[i];
    let x = fetch(v.frame(piece, off as i64));
    let f = seam.min(piece.len as usize);
    // Frames to the piece's end, 1 on its last.
    let left = piece.len as usize - off;
    if f > 0 && left <= f && v.cut_after(i) {
        // Into what comes before the next piece, which flows on into it.
        let w = ((f - left) as f32 + 0.5) / f as f32;
        let next = &pieces[(i + 1) % pieces.len()];
        let y = fetch(v.frame(next, -(left as i64)));
        return [x[0] * (1.0 - w) + y[0] * w, x[1] * (1.0 - w) + y[1] * w];
    }
    x
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
    after: After,
    /// Fade old: the share of the older audio kept each pass while
    /// overdubbing.
    fade_old: f32,

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
    /// How each layer plays its audio (the edits), and the crossfade at
    /// the cuts they make.
    views: [View; MAX_LAYERS],
    seam: usize,
    /// The shapes before the edits in the history.
    shapes: [Shape; EDITS],
    /// An edit's crossfade: frames left, the shape it comes from, and
    /// where that shape is now.
    xf: usize,
    xf_shape: usize,
    xf_pos: f64,
    /// A merge rendering: into which slot, the shape kept for its Undo,
    /// frames rendered, the period it started in, and the frame the merged
    /// layer takes over on (a frame by which it is surely rendered, so
    /// that the period size changes nothing).
    merging: bool,
    mg_slot: usize,
    mg_shape: usize,
    mg_done: usize,
    mg_from: u64,
    mg_at: u64,
    /// Each layer's gains in the merge or bounce (what it is heard at).
    mg_g: [[f32; 2]; MAX_LAYERS],
    /// Being bounced out: its commands wait.
    bouncing: bool,
    /// The first pass armed or recording records the other loops.
    resample: bool,
    /// Moves on whenever the layers' sound changes ([`Status::version`]).
    version: u64,
    /// How much of the print plays, 0 to 1 (the layers play the rest).
    print_on: f32,
    /// The print the looper holds for this loop: its frames (0 for none),
    /// ticks, origin and version, where it is, and where it was before a
    /// jump (as `jump_pos`).
    p_frames: usize,
    p_ticks: Tick,
    p_origin: Tick,
    p_version: u64,
    ppos: f64,
    pjump_pos: f64,
    /// A fit copying the print in: into which slot, the shape kept for its
    /// Undo, frames copied of `fit_frames`, the period it started in, and
    /// the frame it takes over on (as a merge's).
    fitting: bool,
    /// An overdub starts when the fit takes over.
    fit_overdub: bool,
    fit_slot: usize,
    fit_shape: usize,
    fit_done: usize,
    fit_frames: usize,
    fit_from: u64,
    fit_at: u64,
    /// A fit's undo or redo: the loop dips out, swaps to the shape kept at
    /// `swap_shape`, and comes back.
    swapping: bool,
    swap_shape: usize,
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
        after: After::Play,
        fade_old: 1.0,
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
        views: [View::EMPTY; MAX_LAYERS],
        seam: FADE,
        shapes: [NO_SHAPE; EDITS],
        xf: 0,
        xf_shape: 0,
        xf_pos: 0.0,
        merging: false,
        mg_slot: 0,
        mg_shape: 0,
        mg_done: 0,
        mg_from: 0,
        mg_at: 0,
        mg_g: [[0.0; 2]; MAX_LAYERS],
        bouncing: false,
        resample: false,
        version: 0,
        print_on: 0.0,
        p_frames: 0,
        p_ticks: 0,
        p_origin: 0,
        p_version: 0,
        ppos: 0.0,
        pjump_pos: 0.0,
        fitting: false,
        fit_overdub: false,
        fit_slot: 0,
        fit_shape: 0,
        fit_done: 0,
        fit_frames: 0,
        fit_from: 0,
        fit_at: 0,
        swapping: false,
        swap_shape: 0,
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
    /// The period's first frame.
    now: u64,
    bounce: Option<Bouncing>,
    prints: Vec<Prints>,
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
            now: 0,
            bounce: None,
            prints: (0..MAX_LOOPS).map(|_| Prints::default()).collect(),
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
            overdub_next: l.then_overdub || l.fit_overdub,
            after: l.after,
            fade_old: l.fade_old,
            latency: l.latency,
            level: l.level,
            muted: l.muted,
            seam: l.seam,
            merging: l.merging.then(|| l.mg_done as f32 / l.len.max(1) as f32),
            resampling: l.resample && (l.first_in_progress() || l.phase == Phase::Armed),
            capacity: self.capacity,
            used: self.store.blocks(n) * BLOCK,
            version: l.version,
            printed: l.print_on > 0.0,
            fitting: l
                .fitting
                .then(|| l.fit_done as f32 / l.fit_frames.max(1) as f32),
        })
    }

    /// Hand loop `n` a print ([`Print`]) for the tempo it should play at,
    /// or `None` to take its print away. It takes over once the print
    /// playing has faded out (at most [`FADE`] frames), and plays while
    /// the loop is at the print's version and length in ticks. The print
    /// it replaces waits in [`Looper::take_print`], so that it is not
    /// freed here. Does not allocate. Gives `p` back when another print is
    /// still waiting to take over, the loop number is past [`MAX_LOOPS`],
    /// or the print is empty or shorter than its frames.
    pub fn print(&mut self, n: usize, p: Option<Print>) -> Result<(), Option<Print>> {
        let Some(slot) = self.prints.get_mut(n) else {
            return Err(p);
        };
        let ok = p
            .as_ref()
            .is_none_or(|p| p.frames > 0 && p.audio.len() >= 2 * p.frames && p.ticks > 0);
        if !ok || slot.next.is_some() {
            return Err(p);
        }
        slot.next = Some(p);
        Ok(())
    }

    /// The print loop `n` no longer holds, if any, to drop away from the
    /// audio thread.
    pub fn take_print(&mut self, n: usize) -> Option<Print> {
        self.prints.get_mut(n)?.old.take()
    }

    /// The print loop `n` holds now (frames, ticks, version), if any.
    pub fn print_info(&self, n: usize) -> Option<(usize, Tick, u64)> {
        let p = self.prints.get(n)?.now.as_ref()?;
        Some((p.frames, p.ticks, p.version))
    }

    /// As [`Looper::bounce`], for a print: the layers as heard without the
    /// loop's level, at [`PRINT_SPEED`]; the [`Bounce`] carries the loop's
    /// version. The commands of the loop wait while it renders.
    pub fn bounce_print(&mut self, n: usize, buf: Vec<f32>) -> Result<(), Vec<f32>> {
        self.start_bounce(n, buf, true)
    }

    /// Render loop `n` out into `buf`, for a pad or a WAV file: one pass,
    /// as heard ([`Bounce`]). `buf` needs room for the loop (twice its
    /// frames) and is never grown, so this does not allocate. It renders
    /// in the next periods at [`MERGE_SPEED`]; the loop plays on, and its
    /// commands wait until it is done (Clear stops it). One at a time.
    /// `buf` comes back as the error when the loop is not playing, is
    /// recording or merging, `buf` is too small, or a bounce is not yet
    /// taken.
    pub fn bounce(&mut self, n: usize, buf: Vec<f32>) -> Result<(), Vec<f32>> {
        self.start_bounce(n, buf, false)
    }

    fn start_bounce(&mut self, n: usize, mut buf: Vec<f32>, dry: bool) -> Result<(), Vec<f32>> {
        let Some(l) = self.loops.get_mut(n) else {
            return Err(buf);
        };
        let ready = self.bounce.is_none()
            && l.phase == Phase::Playing
            && !l.od_open
            && !l.first_open
            && !l.busy()
            && !l.clearing
            && !l.fitting
            && !l.swapping
            && l.len > 0
            && buf.capacity() >= 2 * l.len;
        if !ready {
            return Err(buf);
        }
        buf.clear();
        buf.resize(2 * l.len, 0.0);
        l.mg_g = l.heard_gains();
        l.bouncing = true;
        self.bounce = Some(Bouncing {
            n,
            buf,
            done: 0,
            complete: None,
            dry,
            version: l.version,
        });
        Ok(())
    }

    /// How far the bounce is, 0 to 1 (None if there is none).
    pub fn bounce_progress(&self) -> Option<f32> {
        self.bounce
            .as_ref()
            .map(|b| b.done as f32 / (b.buf.len() / 2).max(1) as f32)
    }

    /// The finished bounce, with its buffer; None while it renders or if
    /// there is none. Moves the buffer out without allocating.
    pub fn take_bounce(&mut self) -> Option<Bounce> {
        let complete = self.bounce.as_ref()?.complete?;
        let b = self.bounce.take()?;
        let l = &self.loops[b.n];
        Some(Bounce {
            from_loop: b.n,
            frames: b.buf.len() / 2,
            audio: b.buf,
            ticks: l.len_ticks,
            origin: l.origin,
            complete,
            version: b.version,
            dry: b.dry,
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
            view: l.views[slot],
        })
    }

    /// The selected loop's history, for the Looper tab's undo history.
    pub fn history(&self) -> History {
        self.loop_history(self.selected)
            .expect("the selected loop exists")
    }

    /// Loop `n`'s history. None if there is no such loop.
    pub fn loop_history(&self, n: usize) -> Option<History> {
        let l = self.loops.get(n)?;
        let mut h = History {
            done: [Step::Merge(0); HISTORY],
            n_done: l.n_done,
            redo: [Step::Merge(0); HISTORY],
            n_redo: l.n_undone,
        };
        for (to, a) in h.done.iter_mut().zip(&l.done[..l.n_done]) {
            *to = a.step();
        }
        for (to, a) in h.redo.iter_mut().zip(l.undone[..l.n_undone].iter().rev()) {
            *to = a.step();
        }
        Some(h)
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
            faded: s.age,
        })
    }

    /// Run one period: `input` is the line input and `out` the mix, both
    /// interleaved stereo with `block.frames` frames. The loops are added
    /// to `out`; nothing else is changed.
    pub fn process(&mut self, block: &Block, input: &[f32], out: &mut [f32]) {
        let n = (block.frames as usize)
            .min(input.len() / 2)
            .min(out.len() / 2);
        self.now = block.start_frame;
        self.apply_commands();
        let Looper {
            store,
            loops,
            stats,
            bounce,
            prints,
            ..
        } = self;
        let end = block.start_frame + n as u64;
        for (k, l) in loops.iter_mut().enumerate() {
            let pr = &mut prints[k];
            // A print waiting takes over once the one playing has faded
            // out (and the last one taken out has been collected).
            if pr.next.is_some() && (l.print_on == 0.0 || pr.now.is_none()) && pr.old.is_none() {
                pr.old = pr.now.take();
                pr.now = pr.next.take().flatten();
                l.take_print(pr.now.as_ref());
            }
            if l.merging {
                l.render_merge(store, k, end);
            }
            if l.fitting {
                match pr.now.as_ref().filter(|p| l.print_live(p, false)) {
                    Some(p) => l.render_fit(store, k, p, end),
                    // The print went: no fit.
                    None => l.cancel_fit(stats),
                }
            }
        }
        if let Some(b) = bounce.as_mut().filter(|b| b.complete.is_none()) {
            let l = &mut loops[b.n];
            if !l.bouncing {
                // Cleared: what was rendered goes back as it is.
                b.complete = Some(false);
            } else {
                let len = b.buf.len() / 2;
                let speed = if b.dry { PRINT_SPEED } else { MERGE_SPEED };
                let to = len.min(b.done + speed * n);
                let level = if b.dry { 1.0 } else { l.level };
                l.render_out(store, b.n, b.done, to, level, &mut b.buf);
                b.done = to;
                if to == len {
                    l.bouncing = false;
                    b.complete = Some(true);
                }
            }
        }
        // Loops resampling run last in each run, on what the others played
        // in it.
        let rs = loops
            .iter()
            .enumerate()
            .fold(0u32, |m, (k, l)| m | (u32::from(l.resample) << k));
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
                    if rs & (1 << k) == 0 {
                        let (p, waiting) = (prints[k].now.as_ref(), prints[k].next.is_some());
                        l.run(store, stats, k, &run, p, waiting, &mut ys[..frames]);
                    }
                }
                if rs != 0 {
                    let mut heard = [0.0f32; 2 * RUN];
                    for (j, y) in ys[..frames].iter().enumerate() {
                        heard[2 * j] = y[0];
                        heard[2 * j + 1] = y[1];
                    }
                    let run = Run {
                        input: &heard[..2 * frames],
                        ..run
                    };
                    for (k, l) in loops.iter_mut().enumerate() {
                        if rs & (1 << k) != 0 {
                            let (p, waiting) = (prints[k].now.as_ref(), prints[k].next.is_some());
                            l.run(store, stats, k, &run, p, waiting, &mut ys[..frames]);
                        }
                    }
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
            // Overdub (or Record into one) over a print: fit the loop to
            // the print's tempo first; the overdub waits for it.
            // It starts on the frame the fit takes over (a second press
            // while it copies calls the overdub off).
            if !waiting && matches!(q, Queued::Loop(Command::Overdub | Command::Record)) {
                let s = self.selected;
                let l = &self.loops[s];
                if l.fitting && l.fit_overdub {
                    self.loops[s].fit_overdub = false;
                    continue;
                }
                let p = self.prints[s].now.as_ref();
                let live = p.filter(|p| l.print_live(p, self.prints[s].next.is_some()));
                if let Some(frames) = live.filter(|_| l.print_on > 0.0).map(|p| p.frames) {
                    let l = &mut self.loops[s];
                    l.start_fit(&mut self.store, &mut self.stats, s, frames, self.now);
                    if l.fitting {
                        l.fit_overdub = true;
                        continue;
                    }
                }
            }
            let deferrable = matches!(
                q,
                Queued::Loop(
                    Command::Record
                        | Command::Overdub
                        | Command::Undo
                        | Command::Redo
                        | Command::Edit(_)
                        | Command::Merge
                        | Command::Resample
                ) | Queued::Layer(_, LayerCommand::Delete)
            ) || (self.loops[self.selected].busy()
                && matches!(q, Queued::Layer(..)));
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
            Queued::Loop(Command::Merge) => {
                self.loops[s].merge(&mut self.store, &mut self.stats, s, self.now)
            }
            Queued::Loop(Command::Fit) => {
                let l = &self.loops[s];
                let p = self.prints[s].now.as_ref();
                match p.filter(|p| l.print_live(p, self.prints[s].next.is_some())) {
                    Some(p) if l.print_on > 0.0 => {
                        let frames = p.frames;
                        self.loops[s].start_fit(&mut self.store, &mut self.stats, s, frames, self.now)
                    }
                    _ => self.stats.refused += 1,
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
                .any(|a| self.brings_back(a, slot))
    }

    /// Whether `slot` is playing, or Undo can bring it back.
    fn live(&self, slot: usize) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| k as usize == slot)
            || self.done[..self.n_done]
                .iter()
                .any(|a| self.brings_back(a, slot))
    }

    /// Whether undoing or redoing `a` can bring back `slot`'s layer.
    fn brings_back(&self, a: &Act, slot: usize) -> bool {
        match a.kind {
            Kind::Edit => false,
            Kind::Overdub | Kind::Delete => a.slot as usize == slot,
            Kind::Merge | Kind::Fit => {
                let kept = &self.shapes[a.index as usize];
                a.slot as usize == slot
                    || kept.order[..kept.in_order]
                        .iter()
                        .any(|&k| k as usize == slot)
            }
        }
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
                || (self.od_open && self.od_layer == k)
                || (self.merging && self.mg_slot == k)
                || (self.fitting && self.fit_slot == k);
            if !in_use && store.held[me][k] > 0 {
                store.release(me, k);
            }
        }
    }

    fn must_wait(&self, q: Queued) -> bool {
        match q {
            // Everything waits for a bounce (Clear stops it).
            _ if self.bouncing => true,
            Queued::Loop(Command::Undo) => {
                self.leaving() || self.clearing || self.xf > 0 || self.swapping
            }
            // A new overdub, an edit or a merge waits while the last
            // overdub's tail is written, so that tail still fades out.
            Queued::Loop(
                Command::Record | Command::Overdub | Command::Edit(_) | Command::Merge,
            ) if self.od_open && self.phase == Phase::Playing => true,
            // Everything else waits for a merge or a fit (Undo stops
            // them), and for a fit's undo or redo to swap.
            _ if self.merging || self.fitting || self.swapping => true,
            _ => self.fading() || self.clearing || self.xf > 0,
        }
    }

    /// Merging, fitting or being bounced out: the layers must not change.
    fn busy(&self) -> bool {
        self.merging || self.bouncing || self.fitting || self.swapping
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
        let was_p = self.ppos;
        let pf = self.p_frames as f64;
        if self.jump > 0 {
            self.jump -= 1;
            self.jump_pos = (self.jump_pos + 1.0) % len;
            if self.p_frames > 0 {
                self.pjump_pos = (self.pjump_pos + 1.0) % pf;
            }
        }
        if self.p_frames > 0 {
            self.ppos = if playing {
                let rel = rem_ticks(tick - self.p_origin as f64, self.p_ticks as f64);
                (rel / self.p_ticks as f64 * pf).min(pf - 1e-6)
            } else {
                (was_p + 1.0) % pf
            };
        }
        if self.xf > 0 {
            self.xf -= 1;
            let old = &self.shapes[self.xf_shape];
            let old_len = old.len as f64;
            self.xf_pos = if playing {
                let rel = rem_ticks(tick - old.origin as f64, old.len_ticks as f64);
                (rel / old.len_ticks as f64 * old_len).min(old_len - 1e-6)
            } else {
                (self.xf_pos + 1.0) % old_len
            };
        }
        if !playing {
            self.pos = (was + 1.0) % len;
            return;
        }
        let rel = rem_ticks(tick - self.origin as f64, self.len_ticks as f64);
        self.pos = (rel / self.len_ticks as f64 * len).min(len - 1e-6);
        // Fade old: the loop came round (forwards, a frame or so on, not a
        // jump) while overdubbing.
        if self.fade_old < 1.0
            && self.phase == Phase::Overdubbing
            && self.pos < was
            && len - (was - self.pos) <= 4.0
        {
            self.fade_older();
        }
        // Within a span the song position is continuous; a jump can only
        // come at a span's first frame. (An edit's crossfade covers the
        // change of place an edit makes.)
        if first && self.gain > 0.0 && self.xf == 0 {
            let mut d = (self.pos - was).rem_euclid(len);
            if d > len / 2.0 {
                d -= len;
            }
            if d.abs() > 4.0 {
                self.jump = FADE;
                self.jump_pos = (was + 1.0) % len;
                if self.p_frames > 0 {
                    self.pjump_pos = (was_p + 1.0) % pf;
                }
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
        self.touch();
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
                    kind: Kind::Delete,
                    slot: id,
                    index: at as u8,
                    edit: None,
                    ages: [1.0; MAX_LAYERS],
                });
                self.slots[slot].leaving = true;
            }
            LayerCommand::Delete => {}
        }
    }

    fn apply(&mut self, store: &mut Store, stats: &mut Stats, me: usize, c: Command) {
        self.mixing = true;
        // Commands that leave the layers' sound as it is keep the version
        // (and so the print); a merge sounds the same as its layers.
        if !matches!(
            c,
            Command::Mute(_)
                | Command::SetBars(_)
                | Command::SetAfter(_)
                | Command::SetFadeOld(_)
                | Command::SetLatency(_)
                | Command::SetLevel(_)
                | Command::Merge
                | Command::Fit
        ) {
            self.touch();
        }
        match c {
            Command::Record => match self.phase {
                Phase::Empty => {
                    self.phase = Phase::Armed;
                    self.ring = [0.0; 2 * FADE];
                }
                Phase::Armed => {
                    self.phase = Phase::Empty;
                    self.resample = false;
                }
                Phase::Recording => self.ending = true,
                Phase::Playing | Phase::Overdubbing => {
                    self.apply(store, stats, me, Command::Overdub)
                }
            },
            Command::Overdub => match self.phase {
                Phase::Playing => self.open_overdub(store, stats, me, FADE),
                Phase::Overdubbing => self.punch_out(),
                // (Not after a resample: the overdub would take the input.)
                Phase::Recording if !self.resample => self.then_overdub = !self.then_overdub,
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
                            .rposition(|a| a.kind == Kind::Overdub && a.slot as usize == slot)
                        {
                            let mut a = self.take_done(i);
                            self.swap_ages(&mut a);
                            self.push_undone(a);
                        }
                        self.slots[slot].leaving = true;
                    }
                }
                Phase::Playing if self.merging => self.merging = false,
                Phase::Playing if self.fitting => {
                    self.fitting = false;
                    self.fit_overdub = false;
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
            Command::SetAfter(a) => self.after = a,
            Command::SetFadeOld(k) if k.is_finite() => self.fade_old = k.clamp(0.0, 1.0),
            Command::SetFadeOld(_) => {}
            Command::SetLatency(l) => self.latency = l,
            Command::SetLevel(v) => {
                self.level = if v.is_finite() {
                    v.clamp(0.0, 2.0)
                } else {
                    self.level
                }
            }
            Command::Edit(e) => self.edit(stats, e),
            // The looper starts these, with the time and the print.
            Command::Merge | Command::Fit => {}
            Command::Resample => match self.phase {
                Phase::Empty => {
                    self.apply(store, stats, me, Command::Record);
                    self.resample = true;
                }
                Phase::Armed | Phase::Recording if self.resample => {
                    self.apply(store, stats, me, Command::Record)
                }
                _ => stats.refused += 1,
            },
        }
    }

    /// The loop's shape now.
    fn shape(&self) -> Shape {
        Shape {
            len: self.len,
            len_ticks: self.len_ticks,
            origin: self.origin,
            seam: self.seam,
            views: self.views,
            order: self.order,
            in_order: self.in_order,
        }
    }

    fn set_shape(&mut self, s: &Shape) {
        self.len = s.len;
        self.len_ticks = s.len_ticks;
        self.origin = s.origin;
        self.seam = s.seam;
        self.views = s.views;
    }

    /// Make an edit, keeping the shape before it for Undo.
    fn edit(&mut self, stats: &mut Stats, e: Edit) {
        let ready = self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && self.len > 0
            && self.len_ticks > 0;
        let Some(new) = ready.then(|| self.edited(e)).flatten() else {
            stats.refused += 1;
            return;
        };
        self.n_undone = 0;
        let Some(i) = self.free_shape() else {
            stats.refused += 1;
            return;
        };
        self.shapes[i] = self.shape();
        self.set_shape(&new);
        self.push_done(Act {
            kind: Kind::Edit,
            slot: i as u8,
            index: 0,
            edit: Some(e),
            ages: [1.0; MAX_LAYERS],
        });
        self.crossfade_from(i);
    }

    /// The shape an edit makes, or None if it cannot be made.
    fn edited(&self, e: Edit) -> Option<Shape> {
        let mut s = self.shape();
        let (len, ticks) = (s.len, s.len_ticks);
        // Ticks of the loop to frames, rounded.
        let frames = |t: Tick| {
            ((i128::from(t) * len as i128 + i128::from(ticks) / 2) / i128::from(ticks)) as usize
        };
        let live: [bool; MAX_LAYERS] = std::array::from_fn(|k| self.live(k));
        let mut each = |f: &dyn Fn(&View) -> Option<View>| -> Option<()> {
            for k in (0..MAX_LAYERS).filter(|&k| live[k]) {
                s.views[k] = f(&s.views[k])?;
            }
            Some(())
        };
        match e {
            Edit::Half => {
                if ticks % 2 != 0 || ticks / 2 < SIXTEENTH || len < 2 {
                    return None;
                }
                each(&|v| v.cut(0, len / 2))?;
                s.len = len / 2;
                s.len_ticks = ticks / 2;
            }
            Edit::Double => {
                if len > u32::MAX as usize / 2 || ticks > Tick::MAX / 4 {
                    return None;
                }
                each(&|v| v.then(v))?;
                s.len = 2 * len;
                s.len_ticks = 2 * ticks;
            }
            Edit::Reverse => {
                each(&|v| Some(v.reverse()))?;
            }
            Edit::Nudge(n) => {
                let by =
                    (f64::from(n) * SIXTEENTH as f64 * len as f64 / ticks as f64).round() as i64;
                if by.rem_euclid(len as i64) == 0 {
                    return None;
                }
                each(&|v| v.rotate(by))?;
            }
            Edit::Trim { from, to } => {
                if from < 0 || to > ticks || to - from < SIXTEENTH {
                    return None;
                }
                let (a, b) = (frames(from), frames(to));
                if b <= a {
                    return None;
                }
                each(&|v| v.cut(a, b))?;
                s.len = b - a;
                s.len_ticks = to - from;
                s.origin += from;
            }
            Edit::SeamFade(f) => s.seam = (f as usize).min(MAX_SEAM),
        }
        // Every layer plays the loop's length (a check that costs nothing).
        if (0..MAX_LAYERS).any(|k| live[k] && s.views[k].len() != s.len) {
            return None;
        }
        Some(s)
    }

    /// A place to keep a shape for Undo. When all are taken, the history
    /// up to its oldest edit or merge is forgotten.
    fn free_shape(&mut self) -> Option<usize> {
        loop {
            let taken = |i: usize| {
                self.done[..self.n_done]
                    .iter()
                    .chain(&self.undone[..self.n_undone])
                    .any(|a| a.shape() == Some(i))
                    || (self.xf > 0 && self.xf_shape == i)
                    || (self.merging && self.mg_shape == i)
                    || (self.fitting && self.fit_shape == i)
                    || (self.swapping && self.swap_shape == i)
            };
            if let Some(i) = (0..EDITS).find(|&i| !taken(i)) {
                return Some(i);
            }
            let i = self.done[..self.n_done]
                .iter()
                .position(|a| a.shape().is_some())?;
            self.done.copy_within(i + 1..self.n_done, 0);
            self.n_done -= i + 1;
        }
    }

    /// Swap the shape now with the one kept at `i` (Undo and Redo of an
    /// edit), crossfading from it.
    fn swap_shape(&mut self, i: usize) {
        let now = self.shape();
        let kept = self.shapes[i];
        self.set_shape(&kept);
        self.shapes[i] = now;
        self.crossfade_from(i);
    }

    /// Crossfade from the shape kept at `i` (the loop's position is still
    /// that shape's).
    fn crossfade_from(&mut self, i: usize) {
        self.xf = FADE;
        self.xf_shape = i;
        self.xf_pos = self.pos;
    }

    /// The layers' sound changed: a print made before plays no more.
    fn touch(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    /// The print in the looper's hands changed: its numbers, for `follow`.
    fn take_print(&mut self, p: Option<&Print>) {
        match p {
            Some(p) => {
                self.p_frames = p.frames;
                self.p_ticks = p.ticks;
                self.p_origin = p.origin;
                self.p_version = p.version;
                self.ppos = 0.0;
                self.pjump_pos = 0.0;
            }
            None => self.p_frames = 0,
        }
        self.print_on = 0.0;
    }

    /// The loop can play a print now: playing, nothing being written or
    /// changed.
    fn print_ok(&self) -> bool {
        self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.clearing
            && !self.swapping
            && self.len > 0
    }

    /// Print `p` is for the loop as it is now (and no other print waits to
    /// take over: `waiting`).
    fn print_live(&self, p: &Print, waiting: bool) -> bool {
        !waiting
            && self.p_frames == p.frames
            && p.version == self.version
            && p.ticks == self.len_ticks
            && p.origin == self.origin
            && self.print_ok()
    }

    /// Start fitting the loop to its print (`frames` long) in the period
    /// starting at `now`: the print is copied into a new layer at
    /// [`FIT_SPEED`], which then takes the layers' place.
    fn start_fit(&mut self, store: &mut Store, stats: &mut Stats, me: usize, frames: usize, now: u64) {
        let ready = self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.bouncing
            && !self.fitting
            && !self.swapping
            && !self.clearing
            && !self.fading()
            && self.xf == 0
            && self.len > 0
            && self.in_order > 0
            && frames > 0;
        if !ready {
            stats.refused += 1;
            return;
        }
        let Some(k) = self.free_slot_for(store, me, frames) else {
            stats.refused += 1;
            return;
        };
        let Some(i) = self.free_shape() else {
            stats.refused += 1;
            return;
        };
        self.slots[k] = Slot {
            valid: (0, frames),
            ..Slot::NEW
        };
        self.fitting = true;
        self.fit_slot = k;
        self.fit_shape = i;
        self.fit_done = 0;
        self.fit_frames = frames;
        self.fit_from = now;
        self.fit_at = now + frames.div_ceil(FIT_SPEED) as u64;
    }

    /// Copy the print as far as the period ending before frame `end` is
    /// due: [`FIT_SPEED`] frames for each frame since the fit started.
    fn render_fit(&mut self, store: &mut Store, me: usize, p: &Print, end: u64) {
        let due = (end - self.fit_from).saturating_mul(FIT_SPEED as u64);
        let target = self.fit_frames.min(due.min(usize::MAX as u64) as usize);
        for q in self.fit_done..target {
            store.set(me, self.fit_slot, q, [p.audio[2 * q], p.audio[2 * q + 1]]);
        }
        self.fit_done = self.fit_done.max(target);
    }

    fn cancel_fit(&mut self, stats: &mut Stats) {
        self.fitting = false;
        self.fit_overdub = false;
        stats.refused += 1;
    }

    /// The fitted layer takes the layers' place, at the print's length:
    /// it sounds as the print did, which fades out as the loop is no
    /// longer at the print's version. Undo brings the layers back.
    fn finish_fit(&mut self, store: &mut Store, stats: &mut Stats, me: usize, p: Option<&Print>) {
        let Some(p) = p.filter(|p| self.print_live(p, false)) else {
            self.cancel_fit(stats);
            return;
        };
        self.fit_from = 0;
        self.render_fit(store, me, p, u64::MAX / (2 * FIT_SPEED as u64));
        self.fitting = false;
        let (k, i) = (self.fit_slot, self.fit_shape);
        self.shapes[i] = self.shape();
        let old = self.len as f64;
        self.len = self.fit_frames;
        self.views[k] = View::whole(self.len);
        self.pos = (self.pos / old * self.len as f64).min(self.len as f64 - 1e-6);
        self.order[0] = k as u8;
        self.in_order = 1;
        self.mixing = true;
        self.n_undone = 0;
        self.touch();
        self.push_done(Act {
            kind: Kind::Fit,
            slot: k as u8,
            index: i as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
        });
        if std::mem::take(&mut self.fit_overdub) {
            self.open_overdub(store, stats, me, FADE);
        }
    }

    /// A fit's undo or redo: dip out, then swap (`swap`).
    fn start_swap(&mut self, i: usize) {
        self.swapping = true;
        self.swap_shape = i;
        self.mixing = true;
    }

    /// Swap the loop's shape and layers with the ones kept at
    /// `swap_shape`, while it is silent.
    fn swap(&mut self) {
        let i = self.swap_shape;
        let now = self.shape();
        let kept = self.shapes[i];
        let old = self.len as f64;
        self.set_shape(&kept);
        self.order = kept.order;
        self.in_order = kept.in_order;
        self.shapes[i] = now;
        if self.len > 0 {
            self.pos = (self.pos / old * self.len as f64).min(self.len as f64 - 1e-6);
        }
        self.jump = 0;
        self.xf = 0;
        self.swapping = false;
        self.mixing = true;
        self.touch();
    }

    /// Start a merge ([`Command::Merge`]) in the period starting at `now`.
    fn merge(&mut self, store: &mut Store, stats: &mut Stats, me: usize, now: u64) {
        let ready = self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.bouncing
            && self.len > 0
            && self.in_order > 0;
        let g = self.heard_gains();
        let first = self.order[0] as usize;
        let nothing = self.in_order == 1 && self.views[first].is_whole() && g[first] == [1.0, 1.0];
        if !ready || nothing {
            stats.refused += 1;
            return;
        }
        let Some(k) = self.free_slot(store, me) else {
            stats.refused += 1;
            return;
        };
        let Some(i) = self.free_shape() else {
            stats.refused += 1;
            return;
        };
        self.slots[k] = Slot {
            valid: (0, self.len),
            ..Slot::NEW
        };
        self.views[k] = View::whole(self.len);
        self.mg_g = g;
        self.merging = true;
        self.mg_slot = k;
        self.mg_shape = i;
        self.mg_done = 0;
        self.mg_from = now;
        self.mg_at = now + self.len.div_ceil(MERGE_SPEED) as u64;
    }

    /// Render the merge up to what the period ending before frame `end`
    /// is due: [`MERGE_SPEED`] frames for each frame since it started.
    fn render_merge(&mut self, store: &mut Store, me: usize, end: u64) {
        let due = (end - self.mg_from).saturating_mul(MERGE_SPEED as u64);
        let target = self.len.min(due.min(usize::MAX as u64) as usize);
        while self.mg_done < target {
            let from = self.mg_done;
            let n = RUN.min(target - from);
            let mut acc = [[0.0f32; 2]; RUN];
            self.render_layers(store, me, from, &mut acc[..n]);
            for (j, a) in acc[..n].iter().enumerate() {
                store.set(me, self.mg_slot, from + j, *a);
            }
            self.mg_done += n;
        }
    }

    /// Frames `from..` of every layer, as heard (at the gains in `mg_g`),
    /// added into `acc`.
    fn render_layers(&self, store: &Store, me: usize, from: usize, acc: &mut [[f32; 2]]) {
        for &k in &self.order[..self.in_order] {
            let k = k as usize;
            let g = self.mg_g[k];
            let v = &self.views[k];
            if g == [0.0, 0.0] || v.size() == 0 {
                continue;
            }
            let blocks = store.blocks_of(me, k);
            let valid = self.slots[k].valid;
            let mut at = Cursor::new(v);
            for (j, a) in acc.iter_mut().enumerate() {
                let (i, off) = at.locate(from + j);
                let x = piece_frame(store, blocks, valid, v, self.seam, i, off);
                a[0] += g[0] * x[0];
                a[1] += g[1] * x[1];
            }
        }
    }

    /// The gains each layer is heard at, for a merge or a bounce.
    fn heard_gains(&self) -> [[f32; 2]; MAX_LAYERS] {
        let any_solo = self.any_solo();
        let mut g = [[0.0f32; 2]; MAX_LAYERS];
        for &k in &self.order[..self.in_order] {
            let (l, r) = self.slots[k as usize].target(any_solo);
            g[k as usize] = [l, r];
        }
        g
    }

    /// A bounce's frames `from..to`, at `level`, into `out` (interleaved).
    fn render_out(
        &self,
        store: &Store,
        me: usize,
        from: usize,
        to: usize,
        level: f32,
        out: &mut [f32],
    ) {
        let mut p = from;
        while p < to {
            let n = RUN.min(to - p);
            let mut acc = [[0.0f32; 2]; RUN];
            self.render_layers(store, me, p, &mut acc[..n]);
            for (j, a) in acc[..n].iter().enumerate() {
                out[2 * (p + j)] = a[0] * level;
                out[2 * (p + j) + 1] = a[1] * level;
            }
            p += n;
        }
    }

    /// The merged layer takes the layers' place at once (it sounds the
    /// same); Undo brings them back.
    fn finish_merge(&mut self, store: &mut Store, me: usize) {
        self.render_merge(store, me, u64::MAX / 4);
        self.merging = false;
        let (k, i) = (self.mg_slot, self.mg_shape);
        self.shapes[i] = self.shape();
        self.order[0] = k as u8;
        self.in_order = 1;
        self.mixing = true;
        self.n_undone = 0;
        self.push_done(Act {
            kind: Kind::Merge,
            slot: k as u8,
            index: i as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
        });
    }

    /// Undo a merge: the merged layer fades out, and the layers it took
    /// the place of (kept at `a.index`) fade back in, in their order.
    fn unmerge(&mut self, a: Act) {
        self.mixing = true;
        let i = a.index as usize;
        self.slots[a.slot as usize].leaving = true;
        let kept = self.shapes[i];
        for &k in &kept.order[..kept.in_order] {
            self.order[self.in_order] = k;
            self.in_order += 1;
            let s = &mut self.slots[k as usize];
            s.presence = 0.0;
            s.leaving = false;
        }
        self.shapes[i].order[0] = a.slot;
        self.shapes[i].in_order = 1;
    }

    /// Redo a merge: the layers fade out (kept at `a.index` again), and
    /// the merged layer fades in.
    fn remerge(&mut self, a: Act) {
        let i = a.index as usize;
        let n = self.in_order;
        self.shapes[i].order = self.order;
        self.shapes[i].in_order = n;
        for &k in &self.order[..n] {
            self.slots[k as usize].leaving = true;
        }
        self.restore(Act {
            kind: Kind::Overdub,
            slot: a.slot,
            index: n as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
        });
    }

    /// Undo the newest overdub, delete, edit or merge.
    fn undo(&mut self) {
        if self.n_done == 0 {
            return;
        }
        let mut a = self.take_done(self.n_done - 1);
        self.swap_ages(&mut a);
        self.push_undone(a);
        match a.kind {
            Kind::Delete => self.restore(a),
            Kind::Overdub => self.slots[a.slot as usize].leaving = true,
            Kind::Edit => self.swap_shape(a.slot as usize),
            Kind::Merge => self.unmerge(a),
            Kind::Fit => self.start_swap(a.index as usize),
        }
    }

    /// Redo what was undone last.
    fn redo(&mut self) {
        self.n_undone -= 1;
        let mut a = self.undone[self.n_undone];
        self.swap_ages(&mut a);
        self.push_done(a);
        match a.kind {
            Kind::Delete => self.slots[a.slot as usize].leaving = true,
            Kind::Overdub => self.restore(a),
            Kind::Edit => self.swap_shape(a.slot as usize),
            Kind::Merge => self.remerge(a),
            Kind::Fit => self.start_swap(a.index as usize),
        }
    }

    /// Fade old, once: every layer but the one being recorded turns down
    /// (gliding over a few ms, as a level change does). Below -80 dB it is
    /// silent.
    fn fade_older(&mut self) {
        self.touch();
        let k = self.od_layer;
        for &s in &self.order[..self.in_order] {
            if s as usize != k {
                let a = &mut self.slots[s as usize].age;
                *a *= self.fade_old;
                if *a < 1e-4 {
                    *a = 0.0;
                }
            }
        }
        self.mixing = true;
    }

    /// An overdub's undo or redo: the layers' Fade old turn-down goes back
    /// to the other side of it, and the act keeps this side.
    fn swap_ages(&mut self, a: &mut Act) {
        if a.kind != Kind::Overdub {
            return;
        }
        for (x, s) in a.ages.iter_mut().zip(self.slots.iter_mut()) {
            std::mem::swap(x, &mut s.age);
        }
        self.mixing = true;
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
        self.touch();
        self.print_on = 0.0;
        self.fitting = false;
        self.fit_overdub = false;
        self.swapping = false;
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
        self.xf = 0;
        self.seam = FADE;
        self.merging = false;
        self.bouncing = false;
        self.resample = false;
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
                self.then_overdub = self.after == After::Overdub && !self.resample;
                self.first_open = true;
                self.rec_latency = if self.resample { 0 } else { self.latency };
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
        self.views[0] = View::whole(len);
        self.n_done = 0;
        self.n_undone = 0;
        self.ending = false;
        self.pos = 0.0;
        self.jump = 0;
        self.phase = Phase::Playing;
        self.touch();
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
        // A resampled first pass is written: overdubs take the input.
        self.resample = false;
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
    /// its oldest delete or merge is forgotten, so that memory can be used.
    /// None if there is still no room.
    fn free_slot(&mut self, store: &mut Store, me: usize) -> Option<usize> {
        self.free_slot_for(store, me, self.len)
    }

    /// A free slot with memory for `frames` frames (see `free_slot`).
    fn free_slot_for(&mut self, store: &mut Store, me: usize, frames: usize) -> Option<usize> {
        let need = blocks_for(frames);
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
            let i = self.done[..self.n_done]
                .iter()
                .position(|a| matches!(a.kind, Kind::Delete | Kind::Merge | Kind::Fit))?;
            self.done.copy_within(i + 1..self.n_done, 0);
            self.n_done -= i + 1;
        }
    }

    /// A new layer on top, written from now on, fading in over `fade`
    /// frames.
    fn open_overdub(&mut self, store: &mut Store, stats: &mut Stats, me: usize, fade: usize) {
        self.mixing = true;
        // One slot is kept back, so that a merge always has one.
        if self.in_order >= MAX_LAYERS - 1 {
            stats.refused += 1;
            return;
        }
        let Some(k) = self.free_slot(store, me) else {
            stats.refused += 1;
            return;
        };
        self.slots[k] = Slot::NEW;
        self.order[self.in_order] = k as u8;
        self.views[k] = View::whole(self.len);
        let mut ages = [1.0; MAX_LAYERS];
        for (a, s) in ages.iter_mut().zip(&self.slots) {
            *a = s.age;
        }
        self.push_done(Act {
            kind: Kind::Overdub,
            slot: k as u8,
            index: self.in_order as u8,
            edit: None,
            ages,
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
            .rposition(|a| a.kind == Kind::Overdub && a.slot as usize == k)
        {
            self.take_done(i);
        }
        true
    }

    fn close_overdub(&mut self, store: &mut Store, me: usize) {
        self.mixing = true;
        self.touch();
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
        } else if self.fade_old < 1.0 {
            // Fade old: this layer's earlier passes keep their share.
            let was = store.get(me, k, q);
            let f = self.fade_old;
            store.set(me, k, q, [was[0] * f + y[0], was[1] * f + y[1]]);
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
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        r: &Run,
        print: Option<&Print>,
        waiting: bool,
        ys: &mut [[f32; 2]],
    ) {
        let mut j = 0;
        while j < ys.len() {
            if self.phase == Phase::Empty {
                return;
            }
            let f = r.f + j as u64;
            if self.merging && f >= self.mg_at {
                self.finish_merge(store, me);
            }
            let live = print.is_some_and(|p| self.print_live(p, waiting));
            if self.fitting && f >= self.fit_at && self.jump == 0 && self.xf == 0 {
                self.finish_fit(store, stats, me, print);
            }
            let first = r.from + j == 0;
            if !first && self.still(r.playing, live) {
                // Still to the run's end, or to where a merge or a fit
                // takes over.
                let mut end = ys.len();
                if self.merging {
                    end = end.min((self.mg_at - r.f) as usize);
                }
                if self.fitting {
                    end = end.min(self.fit_at.saturating_sub(r.f) as usize).max(j + 1);
                }
                self.still_run(store, me, print, &r.ticks[j..end], &mut ys[j..end]);
                j = end;
                continue;
            }
            let x = [r.input[2 * j], r.input[2 * j + 1]];
            self.write_first(store, stats, me, f, x);
            self.follow(r.ticks[j], r.playing, first);
            let v = self.read(store, me, r.playing, print, live);
            ys[j][0] += v[0];
            ys[j][1] += v[1];
            if self.od_open {
                self.overdub_frame(store, me, x, r.playing);
            }
            j += 1;
        }
    }

    /// Playing at full gain with nothing moving: no writer, fade, mix
    /// change or jump. Only the position changes from frame to frame
    /// (a jump can only come at a span's first frame).
    fn still(&self, playing: bool, live: bool) -> bool {
        let settled = if live {
            self.print_on == 1.0
        } else {
            self.print_on == 0.0
        };
        settled
            && !self.swapping
            && self.phase == Phase::Playing
            && playing
            && self.gain == 1.0
            && !self.muted
            && !self.clearing
            && !self.od_open
            && !self.first_open
            && !self.mixing
            && self.jump == 0
            && self.xf == 0
            && self.len > 0
    }

    /// Frames of a still loop ([`Loop::still`]): the same sums as `read`
    /// and `sample_at` frame by frame, in the same order, so the same bits,
    /// but layer by layer.
    fn still_run(
        &mut self,
        store: &Store,
        me: usize,
        print: Option<&Print>,
        ticks: &[f64],
        ys: &mut [[f32; 2]],
    ) {
        if let Some(p) = print.filter(|_| self.print_on == 1.0) {
            // The print plays alone: as `follow` and `read` frame by frame.
            let (origin, len_ticks, lenf) =
                (self.origin as f64, self.len_ticks as f64, self.len as f64);
            let (p_origin, p_ticks, pf) =
                (self.p_origin as f64, self.p_ticks as f64, self.p_frames as f64);
            let g = self.gain * self.level;
            for (y, &t) in ys.iter_mut().zip(ticks) {
                let rel = rem_ticks(t - origin, len_ticks);
                self.pos = (rel / len_ticks * lenf).min(lenf - 1e-6);
                let rel = rem_ticks(t - p_origin, p_ticks);
                self.ppos = (rel / p_ticks * pf).min(pf - 1e-6);
                let v = p.frame_at(self.ppos);
                y[0] += v[0] * g;
                y[1] += v[1] * g;
            }
            return;
        }
        let len = self.len;
        let n = ys.len();
        let mut i0s = [0usize; RUN];
        let mut i1s = [0usize; RUN];
        let mut fracs = [0.0f32; RUN];
        // `follow` for a still loop (playing, no jump or crossfade): the
        // song position's place in the loop.
        let (origin, len_ticks, lenf) = (self.origin as f64, self.len_ticks as f64, len as f64);
        for j in 0..n {
            let rel = rem_ticks(ticks[j] - origin, len_ticks);
            self.pos = (rel / len_ticks * lenf).min(lenf - 1e-6);
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
            let blocks = store.blocks_of(me, k);
            let view = &self.views[k];
            if !view.is_whole() {
                // As `layer_frame`, finding the pieces with a cursor, and
                // reading a frame once when it is one frame's second and
                // the next one's first.
                let empty = view.size() == 0;
                let mut at = Cursor::new(view);
                let seam = self.seam;
                let mut last = (usize::MAX, [0.0f32; 2]);
                for j in 0..n {
                    let (a, b) = if empty {
                        ([0.0; 2], [0.0; 2])
                    } else {
                        let a = if i0s[j] == last.0 {
                            last.1
                        } else {
                            let (i, off) = at.locate(i0s[j]);
                            piece_frame(store, blocks, s.valid, view, seam, i, off)
                        };
                        let (i, off) = at.locate(i1s[j]);
                        let b = piece_frame(store, blocks, s.valid, view, seam, i, off);
                        last = (i1s[j], b);
                        (a, b)
                    };
                    for c in 0..2 {
                        acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                    }
                }
                continue;
            }
            let (start, count) = s.valid;
            let full = count >= len;
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

    fn read(
        &mut self,
        store: &mut Store,
        me: usize,
        playing: bool,
        print: Option<&Print>,
        live: bool,
    ) -> [f32; 2] {
        let audible = matches!(self.phase, Phase::Playing | Phase::Overdubbing)
            && playing
            && !self.muted
            && !self.clearing
            && !self.swapping;
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
        if self.swapping && self.gain == 0.0 {
            self.swap();
        }
        let print = print.filter(|_| self.p_frames > 0);
        self.print_on = match (live && print.is_some(), self.print_on) {
            (true, on) => (on + STEP).min(1.0),
            (false, on) => (on - STEP).max(0.0),
        };
        self.mix_layers();
        if self.gain == 0.0 || self.len == 0 {
            return [0.0; 2];
        }
        let y = match print.filter(|_| self.print_on > 0.0) {
            Some(p) => {
                let mut py = p.frame_at(self.ppos);
                if self.jump > 0 {
                    let w = self.jump as f32 * STEP;
                    let old = p.frame_at(self.pjump_pos);
                    for (v, o) in py.iter_mut().zip(old) {
                        *v = *v * (1.0 - w) + o * w;
                    }
                }
                if self.print_on < 1.0 {
                    let w = self.print_on;
                    let ly = self.layers_at(store, me);
                    [ly[0] * (1.0 - w) + py[0] * w, ly[1] * (1.0 - w) + py[1] * w]
                } else {
                    py
                }
            }
            None => self.layers_at(store, me),
        };
        let g = self.gain * self.level;
        [y[0] * g, y[1] * g]
    }

    /// The layers at the loop's place, through a jump's or an edit's
    /// crossfade.
    fn layers_at(&self, store: &Store, me: usize) -> [f32; 2] {
        let now = (self.len, self.seam, &self.views);
        let mut y = self.sample_at(store, me, self.pos, now);
        if self.jump > 0 {
            let w = self.jump as f32 * STEP;
            let old = self.sample_at(store, me, self.jump_pos, now);
            for (v, o) in y.iter_mut().zip(old) {
                *v = *v * (1.0 - w) + o * w;
            }
        }
        if self.xf > 0 {
            let w = self.xf as f32 * STEP;
            let sh = &self.shapes[self.xf_shape];
            let old = self.sample_at(store, me, self.xf_pos, (sh.len, sh.seam, &sh.views));
            for (v, o) in y.iter_mut().zip(old) {
                *v = *v * (1.0 - w) + o * w;
            }
        }
        y
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

    /// Every layer at `pos`, read between frames, at its gains, through
    /// a shape's views (the loop's `len`, `seam` and `views`, now or before
    /// an edit). Only the frames a layer has written count.
    fn sample_at(
        &self,
        store: &Store,
        me: usize,
        pos: f64,
        shape: (usize, usize, &[View; MAX_LAYERS]),
    ) -> [f32; 2] {
        let (len, seam, views) = shape;
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
            if !views[k].is_whole() {
                let blocks = store.blocks_of(me, k);
                let a = layer_frame(store, blocks, s.valid, &views[k], seam, i0);
                let b = layer_frame(store, blocks, s.valid, &views[k], seam, i1);
                for (c, v) in y.iter_mut().enumerate() {
                    *v += g[c] * (a[c] + (b[c] - a[c]) * frac);
                }
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
