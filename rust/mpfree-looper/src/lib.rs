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
//! - **No clicks:** the first pass keeps what was heard just before it
//!   ([`KEEP_BEFORE`]) and after it ([`KEEP_AFTER`]), and the loop point
//!   is a seam the loop plays, never written into the audio: the end
//!   fades into what came just before the start, so it flows into the
//!   start, over the loop's seam fade ([`Edit::SeamFade`]). Punch in and
//!   out, start, stop, mute, undo, delete, clear and every layer's level,
//!   pan, mute and solo fade over [`FADE`] frames, and a jump in the song
//!   position (a locate, a transport loop that is not a whole number of
//!   loops, a restart) crossfades from the old place to the new one.
//!   Commands that would cut a fade short wait for it to end (at most
//!   [`FADE`] frames).
//! - **Stopped:** the loops are silent while the transport is stopped or
//!   counting in. Once a loop has faded out, its position waits at the
//!   song position's place, so the playhead stops with the transport and
//!   shows where it will start again. Overdub pressed while stopped waits
//!   for the transport to play; stopping ends an overdub (with a fade).
//! - **Latency:** the player hears the loop late (the output buffer) and
//!   their playing reaches the input late (the input buffer). Input that
//!   arrives `latency` frames after a moment belongs to that moment, so it
//!   is written that much earlier in the loop.
//! - **Saved with the project.** A playing loop's settings, layer mixes
//!   and layers (as they play, through its edits) go into a project
//!   ([`save`]: [`Looper::loop_file`], [`Looper::read_layer`]), and come
//!   back into an empty loop with [`Looper::load`]: the audio copies in at
//!   [`LOAD_SPEED`] while the loop stays silent, then the loop fades in,
//!   in time with the song.
//! - **Layers between loops.** [`Looper::copy_layer`] copies a layer, as
//!   it plays, onto another loop of the same length in bars (in time with
//!   the song, at that loop's length in frames), or into an empty loop,
//!   which takes the layer's loop's length and start. It copies at
//!   [`COPY_SPEED`] while both loops play on, then the layer fades in. A
//!   move then fades it out of its own loop as a delete (a loop's last
//!   layer stays). Undo in either loop takes a move back in both while
//!   each is the other's last step; undoing a layer that made a loop of
//!   its own clears that loop.
//!
//! The memory is allocated once, in [`Looper::new`]. [`Looper::command`]
//! and [`Looper::process`] never allocate, lock or panic, and give the same
//! output for any period size.

#![forbid(unsafe_code)]

pub mod ends;
pub mod find;
pub mod heard;
pub mod hits;
pub mod print;
pub mod reading;
pub mod save;
mod view;

pub use ends::{Candidate, Event, EventLog, InnerBeat, NoteStart, Verdict};
pub use heard::{Take, HEARD_SECONDS};
pub use reading::{
    read_bars, read_bars_with, Reading, Sure, TempoHint, TempoKind, LOOP_MAX_BPM, LOOP_MIN_BPM,
};
pub use save::{LayerFile, LoopFile};

use ends::NoteLog;
use find::FindJob;
use heard::Heard;
use hits::{HitLog, HitTrack};

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
/// Frames a first pass keeps of what was heard before its first frame:
/// a seam's worth at the longest, so the loop point fades into what was
/// really played before the start, at any seam fade.
pub const KEEP_BEFORE: usize = MAX_SEAM;
/// Frames a first pass keeps of what it hears after its end (while the
/// loop plays on): room for the end to move later, and a seam after it.
pub const KEEP_AFTER: usize = 2 * MAX_SEAM;
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
/// A loaded loop ([`Looper::load`]) copies in at this many frames (of all
/// its layers together) for each frame that plays.
pub const LOAD_SPEED: usize = 64;
/// A layer copied from another loop ([`Looper::copy_layer`]) copies in at
/// this many frames for each frame that plays.
pub const COPY_SPEED: usize = 64;
const STEP: f32 = 1.0 / FADE as f32;
/// The fade-in of an overdub that starts where the first pass ends: the
/// input runs on without a break there, so a short fade is enough, and the
/// downbeat's attack is kept (about 0.7 ms at 44.1 kHz).
pub const HANDOFF_FADE: usize = 32;
/// A table entry with no block.
const NONE: u16 = u16::MAX;
/// An overdub whose input never went over this (-60 dB) took silence, and
/// leaves no layer ([`Overdubs`]).
pub const SILENT: f32 = 0.001;
/// A loop waiting for you to play ([`Starts::Play`]) by ear starts on the
/// first input sample at or over this (-40 dB).
pub const PLAY_LEVEL: f32 = 0.01;

/// A loop armed on the bar with the song playing starts on the first bar
/// line at least this many beats after the tap, so a tap just before a
/// bar line gives time to get ready instead of starting at once.
pub const LEAD_BEATS: i64 = 1;
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
    /// Fade out the loop, every layer and the first pass. Undo on the
    /// empty loop brings it back as it was, fading in, until the loop
    /// records, loads or takes a layer again, or another loop needs the
    /// memory it holds ([`Status::kept`]); the gauge counts that memory
    /// as free ([`Looper::memory`]).
    Clear,
    /// As Clear, with no undo: the loop and what Undo could bring back go
    /// for good (a project closing).
    Forget,
    Mute(bool),
    /// While any loop with sound is soloed, only soloed loops play (each
    /// fading out and in as a mute does). Clear lets go of the solo.
    Solo(bool),
    /// How the loop's next overdubs record (see [`Overdubs`]).
    SetOverdubs(Overdubs),
    /// Bars in the next first pass; None records until Record again.
    SetBars(Option<u32>),
    /// What the loop does when its first pass ends (see [`After`]).
    SetAfter(After),
    /// When the loop's next first pass starts once armed (see [`Starts`]).
    SetStarts(Starts),
    /// A note played on song frame `frame` (a pad hit, which the host
    /// sees before the looper hears it): starts a loop armed to start
    /// when you play ([`Starts::Play`]) that isn't listening by ear
    /// ([`Command::StartsByEar`]). For every loop.
    Played(u64),
    /// Whether a loop armed to start when you play listens to its input
    /// (true: the line input, the first sample at or over [`PLAY_LEVEL`]
    /// is the note) or waits for [`Command::Played`] (false, the default:
    /// the pads, whose sound includes the song's own). For every loop.
    StartsByEar(bool),
    /// Tempo from the first loop (for every loop; off by default). With
    /// the song stopped and every other loop empty, a loop's first pass
    /// records free: from Record (On the bar) or from your note (When I
    /// play), with no bars yet. Record again closes it on the next
    /// period's first frame, or, before it is a bar at [`LOOP_MAX_BPM`]
    /// (with Length set, that many bars), once it is. Its length then
    /// sets the tempo ([`read_bars`]): the bars Length set
    /// ([`Command::SetBars`]), "this many bars at the tempo I play", or
    /// on Free length the bars the reading picks, from the start of the
    /// bar the song stopped in. The host starts the song there, on the
    /// frame the loop comes round, at that tempo
    /// ([`Looper::take_tempo`]). The song starting under a free pass
    /// (PLAY, or a count-in) throws the pass away (counted). The host
    /// turns this off while the song can't take a tempo from a loop
    /// (another device's clock leads, its tempo map has more than one
    /// tempo, an export runs).
    TempoFromLoop(bool),
    /// The song's tempo was set by hand since the project opened (true),
    /// or was left from before (false, the default): how much it counts
    /// when a free first pass's length is read ([`TempoKind`]). For every
    /// loop.
    TempoMeant(bool),
    /// Smart loop end (for every loop; Off by default): how a free first
    /// pass's closing tap is checked against the notes you played
    /// ([`Looper::note`], [`ends`]). Off closes on the tap. Suggest closes
    /// on the tap too, then offers the end it found ([`Status::offer`]).
    /// Auto listens on to the end of the reach after the tap
    /// ([`Status::closing`]), then closes on the end it found (or the
    /// tap), with the loop and the song already that far in.
    SetSmartEnd(SmartEnd),
    /// Whether smart loop end listens for hits in what is recorded (true:
    /// the line input or a mix, [`hits`], weighed on the finder's thread,
    /// [`find`]) or weighs the notes played ([`Looper::note`]; false, the
    /// default: the pads and MIDI). The beat inside a free first loop
    /// comes from the same. For every loop.
    SmartByEar(bool),
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
    /// Empty: the last this many whole bars heard become the loop
    /// ([`heard`]), ending on the bar line just passed, or, pressed in a
    /// bar's last beat, on the one coming. Fewer bars (halving) when fewer
    /// were played since the song started or jumped, or since the input
    /// changed. It copies in at [`LOAD_SPEED`] while the loop stays
    /// silent, then plays in time with the song, fading in, as a load
    /// does ([`Status::capturing`]). Again, Undo or Clear call it off.
    /// Refused when the loop isn't empty or no whole bar was heard.
    Capture(u32),
    /// Empty: as [`Command::Capture`], but the bars end on the bar line
    /// heard on song tick `.0` (in the song as it last played on
    /// unbroken), however long ago, so What you played's window can take
    /// any bars still kept: `.1` of them, fewer (halving) when fewer fit.
    /// Refused when that bar line isn't kept. Again, Undo or Clear call
    /// it off, as for Capture.
    CaptureTo(Tick, u32),
    /// The input is another source now: Capture forgets what it heard
    /// before. For every loop.
    NewInput,
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
    /// The octaves the audio plays faster than it is ([`Status::rate`]):
    /// a print makes it so ([`print`]).
    pub rate: i8,
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
        save::wav(&self.audio, rate)
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
    /// The loop's speed, like tape, in octaves from as it was recorded:
    /// -1 is half speed (an octave down, twice the bars), 0 as recorded,
    /// 1 double speed (an octave up, half the bars). Only the length in
    /// ticks changes; the audio plays faster or slower to fit it, until a
    /// print made at that speed is in ([`Status::rate`]).
    Speed(i8),
    /// Move the loop this many frames later (above 0) or earlier in the
    /// audio its first pass kept: both boundaries shift together, so the
    /// length stays, and what plays changes. Only while the loop is its
    /// first pass alone and unedited ([`Status::bounds`]), within what it
    /// kept.
    Move(i64),
    /// Move the loop's end this many frames later (above 0) or earlier
    /// in the audio its first pass kept: the length changes and bar 1
    /// stays. The bars stay too, so the loop plays a little faster or
    /// slower to fit them until the tempo changes with it. At least a
    /// 1/16 note; as [`Edit::Move`] otherwise.
    Resize(i64),
    /// Move bar 1 this many frames later (above 0) or earlier inside the
    /// same audio, going round the loop: what plays stays, where it
    /// starts changes (as a pickup does). As [`Edit::Move`] otherwise.
    Turn(i64),
}

/// Where a first pass's loop lies in the audio it kept
/// ([`Status::bounds`]): frames from the first frame kept, at the loop's
/// rate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    /// The frame that plays on bar 1: the loop's downbeat.
    pub src: i64,
    /// The loop's first frame: `src`, or before it when the loop is
    /// turned (a pickup): the frames from here to `src` play at the
    /// loop's end, leading into bar 1.
    pub start: i64,
    /// Where the pass began and where it closed, as recorded: the
    /// boundaries go back to these.
    pub captured: (i64, i64),
    /// Frames kept: the boundaries move within 0 to this.
    pub kept: i64,
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
    /// This layer came from that loop ([`Looper::copy_layer`]).
    Added(u8, usize),
    /// This layer moved to that loop: a delete here.
    Moved(u8, usize),
    /// Smart loop end moved the loop's start and end this much, with the
    /// song's tempo, on the loop's downbeat ([`Looper::snap_at`]).
    Snap(Shift),
    /// Read as: the loop was read as this many bars, its length and its
    /// sound kept, with the song's tempo, on its downbeat ([`Snap::Bars`]).
    Bars(u32),
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
    /// Recorded with Overdubs: Replace: the layers under it are silent
    /// where it was written ([`Overdubs::Replace`]).
    pub replace: bool,
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

/// Listen: a loop, or one layer of it, playing alone from its start
/// while the song is stopped ([`Looper::listen`]).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Listen {
    n: usize,
    /// One layer (its slot), or None for the whole loop as heard.
    layer: Option<u8>,
    /// The loop frame it plays next.
    pos: usize,
    /// Faded in from 0 over [`FADE`] frames, and out once `stopping`.
    gain: f32,
    stopping: bool,
}

/// When an armed loop's first pass starts: the Looper tab's Starts
/// setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Starts {
    /// On the next bar line (stopped: on the one PLAY starts on).
    #[default]
    Bar,
    /// When you play: the loop waits for the first note (a pad hit, or
    /// the line input over [`PLAY_LEVEL`]). With the song playing, it
    /// starts on the bar line nearest that note: a note in the first half
    /// of a bar starts it on that bar's line, what came between taken
    /// from what the looper heard; a note in the second half, on the next
    /// line, and the notes before that line play at the loop's end,
    /// leading into its first bar each time round. With the song stopped
    /// the host starts the song on the note ([`Looper::take_start`]), so
    /// bar 1 is where you started. A resample starts on the bar.
    Play,
}

/// How an overdub records: the Looper tab's Overdubs setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overdubs {
    /// On top, for as long as it records (until Overdub again).
    #[default]
    Add,
    /// In place of what was there, for as long as it records: the layers
    /// under it are silent over the frames it wrote (crossfading at its
    /// ends), and the rest of the loop plays as before. Nothing is erased:
    /// Undo takes the overdub away and they are back. A second pass
    /// replaces the first.
    Replace,
    /// One pass, then the loop plays by itself: the overdub ends where it
    /// started, its end crossfading into its start.
    Once,
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

/// Smart loop end ([`Command::SetSmartEnd`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SmartEnd {
    /// The loop closes on your tap.
    #[default]
    Off,
    /// It closes on your tap, then offers the end it found.
    Suggest,
    /// It listens on to the end of the reach, then closes on the end it
    /// found.
    Auto,
}

/// What smart loop end made of a free first pass's closing tap, once
/// ([`Looper::take_smart`]), for the log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmartNote {
    pub n: usize,
    pub outcome: SmartOutcome,
    /// A smart start: frames from your press to your first hit, where
    /// Auto started the loop or Suggest offers to ([`Shift::start`]). 0
    /// when the start stays on your press.
    pub start: i64,
    /// The closing tap's frame (the song's frame clock).
    pub tap: u64,
    /// The best end and the runner-up (the tap, when it was), scored.
    pub best: Option<Candidate>,
    pub second: Option<Candidate>,
}

/// What became of the tap, in frames from it (negative: the end is
/// earlier, the tap was late).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SmartOutcome {
    /// Auto moved the end.
    Moved(i64),
    /// Suggest offers moving it ([`Status::offer`]).
    Offered(i64),
    /// The end found is within [`OFFER_MS`] of the tap: nothing worth
    /// offering.
    Close(i64),
    /// The tap stood, and why.
    Kept(&'static str),
}

/// Why Suggest kept a tap when Then: Overdub started the overdub on it
/// ([`SmartOutcome::Kept`]): Suggest can't wait for you there.
pub const OVERDUB_AT_ONCE: &str = "the overdub started at once";

/// The smallest move Suggest offers, in milliseconds: anything nearer the
/// tap can't be heard.
pub const OFFER_MS: f64 = 5.0;

/// By ear, how long Auto waits past its reach for the finder's answer
/// (ms) before it keeps your tap; the loop and the song start that far
/// in, so nothing is heard late.
pub const FIND_WAIT_AUTO_MS: f64 = 30.0;
/// How long Suggest waits for it.
pub const FIND_WAIT_SUGGEST_MS: f64 = 250.0;

/// `ms` milliseconds in frames at `rate`.
fn ms_frames(rate: u32, ms: f64) -> u64 {
    (ms * f64::from(rate) / 1000.0).round() as u64
}

/// Smart loop end by ear: a decision asked of the finder for loop `k`'s
/// take `gen` ([`Looper::find_job`], [`Looper::found`]).
#[derive(Clone, Copy, Debug)]
struct Finding {
    k: usize,
    gen: u64,
    ask: ends::Ask,
    bars: u32,
    /// The frame by which it must have answered, else the tap stands.
    due: u64,
    /// Handed to the host for the finder.
    sent: bool,
    answer: Option<ends::Decision>,
}

/// How far a free first loop's start and end are, or move, from your
/// press and your closing tap, in frames
/// ([`Status::offer`], [`Status::moved`], [`Step::Snap`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shift {
    /// The loop's start, later (above 0): a smart start puts it on your
    /// first hit.
    pub start: i64,
    /// Its end, later (above 0; negative: earlier, the tap was late).
    pub end: i64,
}

impl Shift {
    pub const NONE: Shift = Shift { start: 0, end: 0 };

    /// How much longer the loop is (the tempo changes with it).
    pub fn longer(&self) -> i64 {
        self.end - self.start
    }

    pub fn is_none(&self) -> bool {
        *self == Shift::NONE
    }
}

impl std::ops::Add for Shift {
    type Output = Shift;
    fn add(self, o: Shift) -> Shift {
        Shift {
            start: self.start + o.start,
            end: self.end + o.end,
        }
    }
}

impl std::ops::Sub for Shift {
    type Output = Shift;
    fn sub(self, o: Shift) -> Shift {
        Shift {
            start: self.start - o.start,
            end: self.end - o.end,
        }
    }
}

impl std::ops::Neg for Shift {
    type Output = Shift;
    fn neg(self) -> Shift {
        Shift {
            start: -self.start,
            end: -self.end,
        }
    }
}

/// A change of a free first loop's start or end made with the song's
/// tempo, landing on the loop's downbeat ([`Looper::snap_plan`],
/// [`Looper::snap_at`]): the loop's length and the tempo change together,
/// so its bars stay and it plays as recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Snap {
    /// Take what smart loop end found ([`Status::offer`]): the end, and
    /// the start on your first hit (a smart start).
    Found,
    /// Put the end back on your closing tap ([`Status::moved`]): Use my
    /// tap.
    Tap,
    /// Put the start back on your press: Use my press.
    Press,
    /// Read as: read the loop as this many bars of the song's meter. Its
    /// length and its sound stay; the tempo changes so they fit it.
    Bars(u32),
    /// Undo the last step, a snap ([`Step::Snap`], [`Step::Bars`]).
    Undo,
    /// Redo the next step, a snap.
    Redo,
}

/// A snap worked out ([`Looper::snap_plan`]): the host asks the time engine
/// for the tempo on `at`, and once that is committed, gives the plan back
/// ([`Looper::snap_at`]) so the loop changes on the same frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapPlan {
    pub n: usize,
    pub snap: Snap,
    /// How far the start and the end move.
    pub by: Shift,
    /// The loop's length in song ticks once it lands: Read as changes it.
    pub ticks: Tick,
    /// The tempo at which the loop's bars are its new length exactly.
    pub bpm: f64,
    /// The loop's next downbeat, as a song tick and a frame.
    pub tick: Tick,
    pub at: u64,
    /// The loop's version it was worked out at ([`Status::version`]): a
    /// loop changed since doesn't take it.
    pub version: u64,
}

/// What became of a snap the host gave ([`Looper::take_snap`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SnapDone {
    pub plan: SnapPlan,
    /// It landed on its frame; false: the loop had changed, and nothing
    /// of the loop moved.
    pub landed: bool,
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
    /// First passes thrown away: the transport jumped before the end,
    /// STOP came before a 1/16 note was played, or the pass outgrew the
    /// memory.
    pub aborted: u64,
    /// First passes that kept nothing after their end
    /// ([`KEEP_AFTER`]), for want of memory: their end can't move later.
    pub kept_short: u64,
    /// Smart loop end: closing taps that stood (no clear match, or the
    /// overdub started at once), and ends Auto moved or Suggest offered.
    pub taps_kept: u64,
    pub ends_moved: u64,
    pub ends_offered: u64,
    /// Smart starts: loops Auto started on your first hit, or Suggest
    /// offered to.
    pub starts_found: u64,
    /// Auto with Then: Overdub: overdubs that began with what you played
    /// after the loop's end while Auto listened.
    pub handoffs: u64,
    /// Smart loop end by ear: hits the hit track heard; decisions asked
    /// of the finder, answered in time, too late (the tap stood), and
    /// answers for a take that had gone (dropped).
    pub hits_heard: u64,
    pub finds_asked: u64,
    pub finds_answered: u64,
    pub finds_late: u64,
    pub finds_stale: u64,
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
    /// Launched ([`Looper::launch`]): off, the loop is silent but keeps
    /// its place.
    pub on: bool,
    /// A launch waiting for the bar line at this tick: on or off.
    pub cue: Option<(bool, Tick)>,
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
    /// A saved loop copying in ([`Looper::load`]): how much is done, 0 to
    /// 1. The loop is empty and silent until it is all in.
    pub loading: Option<f32>,
    /// A layer of another loop copying in ([`Looper::copy_layer`]): how
    /// much is done, 0 to 1.
    pub copying: Option<f32>,
    /// Frames of the first pass recorded so far, while it records (0
    /// otherwise): what [`Looper::first_pass`] reads.
    pub recorded: usize,
    /// STOP cut the first pass short: the frames of it that hold sound.
    /// The rest, to `frames`, is silent. None otherwise.
    pub cut: Option<usize>,
    /// Armed on the bar with the song playing: the bar line (song tick)
    /// the first pass starts on. Always at least a beat after the tap
    /// ([`LEAD_BEATS`]), so there is time to get ready. None otherwise
    /// (stopped: it starts on the bar PLAY starts on).
    pub starts_on: Option<Tick>,
    /// Cleared, and Undo can bring it back ([`Command::Clear`]).
    pub kept: bool,
    /// Soloed ([`Command::Solo`]).
    pub solo: bool,
    /// Silent because another loop is soloed.
    pub solo_out: bool,
    /// How its overdubs record ([`Command::SetOverdubs`]).
    pub overdubs: Overdubs,
    /// Its speed ([`Edit::Speed`]): -1 half, 0 as recorded, 1 double.
    pub speed: i8,
    /// The octaves its layers' audio plays faster than it was recorded
    /// (its speed, until a fit makes the audio play as it is: then 0).
    /// Not 0, the loop plays like tape until a print is in, and an
    /// overdub waits for that print and fits the loop to it first.
    pub rate: i8,
    /// A capture ([`Command::Capture`]): 0 while it waits for its bar
    /// line and its last frames, then how much has copied in, to 1. The
    /// loop is empty and silent until it is all in.
    pub capturing: Option<f32>,
    /// When its next first pass starts ([`Command::SetStarts`]).
    pub starts: Starts,
    /// Armed to start when you play, and no note yet: it is listening.
    /// Once a note comes it waits for its bar line (or for the song to
    /// start) as an armed loop does.
    pub listening: bool,
    /// Frames of the first pass played before its first bar line (a
    /// pickup, [`Starts::Play`]): they play at the loop's end, leading
    /// into its first bar. 0 otherwise.
    pub lead: usize,
    /// Its first pass records free and its length will set the tempo
    /// ([`Command::TempoFromLoop`]): recording so, or, with the song
    /// stopped and the setting on, empty (Record would start it so) or
    /// listening (the note it waits for would).
    pub free: bool,
    /// Where the loop lies in the audio its first pass kept, while the
    /// loop is that pass alone and unedited, so that [`Edit::Move`],
    /// [`Edit::Resize`] and [`Edit::Turn`] can be made. None otherwise
    /// (and for loops captured or opened from a save, which keep nothing
    /// around them).
    pub bounds: Option<Bounds>,
    /// Smart loop end, Auto: Record closed the free first pass and it
    /// listens on to the end of the reach before it closes (the ring says
    /// closing).
    pub closing: bool,
    /// Smart loop end: how far [`Snap::Found`] would move the loop's start
    /// and end to what it found, in frames (a negative end: earlier, the
    /// tap was late). Suggest offers it once its
    /// reach has been heard; after Use my tap or Use my press it is on
    /// offer again. It lasts until the loop's sound changes in any other
    /// way.
    pub offer: Option<Shift>,
    /// How far the loop's start and end are from your press and tap
    /// (Auto, or a snap): [`Snap::Press`] and [`Snap::Tap`] put them back. Lasts as [`Status::offer`] does.
    pub moved: Option<Shift>,
    /// A snap given to land on this frame ([`Looper::snap_at`]).
    pub snapping: Option<u64>,
}

/// The memory gauge: all the loops together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    /// Frames held by every loop, in whole blocks (not counting cleared
    /// loops kept for Undo, whose memory goes when it is needed).
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

    /// Frames `from..from + out.len()` into `out`: the same values as
    /// indexing gives, a block at a time where the audio plays whole and
    /// written. For a reader of many frames at once (the screen's
    /// waveform), which indexing frame by frame makes slow.
    pub fn read(&self, from: usize, out: &mut [[f32; 2]]) {
        assert!(
            from + out.len() <= self.len,
            "frames {from}..{} of a layer of {} frames",
            from + out.len(),
            self.len
        );
        let size = self.view.size();
        if size == 0 {
            out.fill([0.0; 2]);
            return;
        }
        let blocks = self.store.blocks_of(self.lp, self.slot);
        let (start, count) = self.valid;
        let whole = self.view.is_whole();
        if whole && count >= size {
            let mut j = 0;
            while j < out.len() {
                let q = from + j;
                let (b, o) = (q / BLOCK, q % BLOCK);
                let n = (BLOCK - o).min(out.len() - j);
                match blocks.get(b) {
                    Some(&k) => {
                        let i = 2 * (k as usize * BLOCK + o);
                        let src = &self.store.pool[i..i + 2 * n];
                        for (y, x) in out[j..j + n].iter_mut().zip(src.chunks_exact(2)) {
                            *y = [x[0], x[1]];
                        }
                    }
                    None => out[j..j + n].fill([0.0; 2]),
                }
                j += n;
            }
            return;
        }
        let mut at = Cursor::new(&self.view);
        let pieces = self.view.pieces();
        for (j, y) in out.iter_mut().enumerate() {
            let q = if whole {
                from + j
            } else {
                let (i, off) = at.locate(from + j);
                self.view.frame(&pieces[i], off as i64)
            };
            *y = if (q + size - start) % size < count {
                self.store.frame(blocks, q / BLOCK, q % BLOCK)
            } else {
                [0.0; 2]
            };
        }
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
    /// Loop `n` on or off, now or from a bar line ([`Looper::launch`]).
    Launch {
        n: usize,
        on: bool,
        at: Option<Tick>,
    },
    /// A layer of loop `from` onto loop `to` ([`Looper::copy_layer`]).
    Copy {
        from: usize,
        id: u8,
        to: usize,
        keep: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Overdub,
    Delete,
    Edit,
    Merge,
    Fit,
    /// A layer copied in from another loop.
    Added,
    /// Smart loop end's change of the start and the end, with the tempo
    /// (an [`Edit::Move`] and an [`Edit::Resize`] at once, made on the
    /// loop's downbeat).
    Snap,
    /// Read as: the loop's bars changed, its length and sound kept, with
    /// the tempo, on its downbeat.
    Bars,
}

/// The other loop's side of a layer copied between loops: the loop and
/// the layer's slot there, and whether the two steps undo together (a
/// move).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Link {
    lp: u8,
    slot: u8,
    tied: bool,
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
    /// A layer copied in from, or moved to, another loop.
    link: Option<Link>,
    /// A snap's move ([`Kind::Snap`]).
    shift: Shift,
    /// The bars Read as made ([`Kind::Bars`]).
    bars: u32,
}

const NO_ACT: Act = Act {
    kind: Kind::Overdub,
    slot: 0,
    index: 0,
    edit: None,
    ages: [1.0; MAX_LAYERS],
    link: None,
    shift: Shift::NONE,
    bars: 0,
};

impl Act {
    fn step(&self) -> Step {
        match self.kind {
            Kind::Overdub => Step::Overdub(self.slot),
            Kind::Delete => match self.link {
                Some(l) => Step::Moved(self.slot, l.lp as usize),
                None => Step::Delete(self.slot),
            },
            Kind::Added => Step::Added(self.slot, self.link.map_or(0, |l| l.lp as usize)),
            Kind::Edit => Step::Edit(self.edit.unwrap_or(Edit::Half)),
            Kind::Snap => Step::Snap(self.shift),
            Kind::Bars => Step::Bars(self.bars),
            Kind::Merge => Step::Merge(self.slot),
            Kind::Fit => Step::Fit(self.slot),
        }
    }

    /// Whether taking its shape, or giving it back, crossfades: Read as
    /// keeps every frame where it was (only the bars and the tempo change),
    /// so there is nothing to fade.
    fn fades(&self) -> bool {
        self.kind != Kind::Bars
    }

    /// Where the act keeps a shape, if it does.
    fn shape(&self) -> Option<usize> {
        match self.kind {
            Kind::Edit | Kind::Snap | Kind::Bars => Some(self.slot as usize),
            Kind::Merge | Kind::Fit => Some(self.index as usize),
            Kind::Overdub | Kind::Delete | Kind::Added => None,
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
    /// The first pass's loop in the audio it kept (frames: its first,
    /// and the one on bar 1), while layer 0 plays that region alone
    /// ([`Loop::bounds_now`]).
    region: Option<(usize, usize)>,
    speed: i8,
    rate: i8,
    views: [View; MAX_LAYERS],
    order: [u8; MAX_LAYERS],
    in_order: usize,
}

const NO_SHAPE: Shape = Shape {
    len: 0,
    len_ticks: 0,
    origin: 0,
    seam: FADE,
    region: None,
    speed: 0,
    rate: 0,
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
    /// Recorded with Overdubs: Replace: its fade-in and fade-out in frames.
    /// The layers before it in the play order are silent where it was
    /// written, crossfading over those fades ([`Slot::cover`]).
    replace: Option<(u32, u32)>,
    /// How much it silences the layers under it now, 0 to 1: it moves to
    /// 0 while the layer is muted or left out by a solo, and back, a step
    /// a frame.
    duck: f32,
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
        replace: None,
        duck: 1.0,
    };

    /// Heard: not muted, and not left out by another layer's solo.
    fn heard(&self, any_solo: bool) -> bool {
        !self.mute && (!any_solo || self.solo)
    }

    /// How far a Replace layer covers audio frame `a` of its `size`: 1
    /// where it was written, 0 where not, and between over its fades. 0
    /// for a layer that is not a Replace.
    fn cover(&self, a: usize, size: usize) -> f32 {
        let Some((fin, fout)) = self.replace else {
            return 0.0;
        };
        let (start, count) = self.valid;
        if size == 0 || count == 0 {
            return 0.0;
        }
        if count >= size {
            return 1.0;
        }
        // Frames from the first one written.
        let d = (a + size - start % size) % size;
        if d >= count {
            return 0.0;
        }
        let up = (d as f32 + 0.5) / fin.max(1) as f32;
        let down = ((count - d) as f32 - 0.5) / fout.max(1) as f32;
        up.min(down).min(1.0)
    }

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
    /// Each loop as it was when cleared, for Undo ([`Command::Clear`]):
    /// the slots whose blocks it keeps (none: nothing kept), and when it
    /// was kept (the oldest goes first when the memory is needed).
    kept: Vec<Loop>,
    kept_mask: [u16; MAX_LOOPS],
    kept_at: [u64; MAX_LOOPS],
    kept_seq: u64,
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
            kept: vec![Loop::NEW; MAX_LOOPS],
            kept_mask: [0; MAX_LOOPS],
            kept_at: [0; MAX_LOOPS],
            kept_seq: 0,
        }
    }

    #[inline]
    fn table(&self, lp: usize, slot: usize) -> usize {
        (lp * MAX_LAYERS + slot) * self.n_blocks
    }

    /// Where frame `q` of a slot is in the pool, if its block is there.
    #[inline]
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

    #[inline]
    fn get(&self, lp: usize, slot: usize, q: usize) -> [f32; 2] {
        self.at(lp, slot, q)
            .map_or([0.0; 2], |i| [self.pool[i], self.pool[i + 1]])
    }

    #[inline]
    fn set(&mut self, lp: usize, slot: usize, q: usize, x: [f32; 2]) {
        if let Some(i) = self.at(lp, slot, q) {
            self.pool[i] = x[0];
            self.pool[i + 1] = x[1];
        }
    }

    #[inline]
    fn add(&mut self, lp: usize, slot: usize, q: usize, x: [f32; 2]) {
        if let Some(i) = self.at(lp, slot, q) {
            self.pool[i] += x[0];
            self.pool[i + 1] += x[1];
        }
    }

    /// Give a slot blocks until it holds `n`. False, with what it got, if
    /// the memory ran out.
    /// A loop kept for Undo gives its blocks up first, the oldest kept
    /// first. Inline, since a first pass asks on every frame and nearly
    /// always holds them already.
    #[inline]
    fn grow(&mut self, lp: usize, slot: usize, n: usize) -> bool {
        (self.held[lp][slot] as usize) >= n || self.grow_to(lp, slot, n)
    }

    #[inline(never)]
    fn grow_to(&mut self, lp: usize, slot: usize, n: usize) -> bool {
        let t = self.table(lp, slot);
        while (self.held[lp][slot] as usize) < n {
            if self.n_free == 0 && !self.drop_oldest_kept(lp) {
                return false;
            }
            self.n_free -= 1;
            let h = self.held[lp][slot] as usize;
            self.tables[t + h] = self.free[self.n_free];
            self.held[lp][slot] += 1;
        }
        true
    }

    /// Give back a slot's blocks after its first `n`.
    fn shrink(&mut self, lp: usize, slot: usize, n: usize) {
        let t = self.table(lp, slot);
        while self.held[lp][slot] as usize > n {
            self.held[lp][slot] -= 1;
            let h = self.held[lp][slot] as usize;
            self.free[self.n_free] = self.tables[t + h];
            self.tables[t + h] = NONE;
            self.n_free += 1;
        }
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

    /// Keep loop `lp` as it is (cleared, faded out) for Undo, with the
    /// blocks its slots hold. Copies, never allocates.
    fn keep(&mut self, lp: usize, l: &Loop) {
        self.drop_kept(lp);
        self.kept[lp].clone_from(l);
        let mut mask = 0u16;
        for k in 0..MAX_LAYERS {
            if self.held[lp][k] > 0 {
                mask |= 1 << k;
            }
        }
        self.kept_seq += 1;
        self.kept_mask[lp] = mask;
        self.kept_at[lp] = self.kept_seq;
    }

    /// Loop `lp` is kept for Undo.
    fn has_kept(&self, lp: usize) -> bool {
        self.kept_mask[lp] != 0
    }

    /// Slot `k` of loop `lp` holds blocks the loop kept for Undo needs.
    fn kept_holds(&self, lp: usize, k: usize) -> bool {
        self.kept_mask[lp] & (1 << k) != 0
    }

    /// Forget loop `lp`'s kept loop: its blocks go back.
    fn drop_kept(&mut self, lp: usize) {
        let mask = std::mem::take(&mut self.kept_mask[lp]);
        for k in 0..MAX_LAYERS {
            if mask & (1 << k) != 0 {
                self.release(lp, k);
            }
        }
    }

    /// Forget the oldest kept loop but `lp`'s. False if there is none.
    #[cold]
    fn drop_oldest_kept(&mut self, lp: usize) -> bool {
        let oldest = (0..MAX_LOOPS)
            .filter(|&n| n != lp && self.kept_mask[n] != 0)
            .min_by_key(|&n| self.kept_at[n]);
        match oldest {
            Some(n) => {
                self.drop_kept(n);
                true
            }
            None => false,
        }
    }

    /// Blocks loop `lp` keeps for Undo.
    fn kept_blocks_of(&self, lp: usize) -> usize {
        (0..MAX_LAYERS)
            .filter(|&k| self.kept_holds(lp, k))
            .map(|k| self.held[lp][k] as usize)
            .sum()
    }

    /// Blocks free, or free once the kept loops give theirs up.
    fn available(&self) -> usize {
        self.n_free
            + (0..MAX_LOOPS)
                .map(|lp| self.kept_blocks_of(lp))
                .sum::<usize>()
    }
}

/// Blocks for `frames` frames.
fn blocks_for(frames: usize) -> usize {
    frames.div_ceil(BLOCK)
}

/// The blocks [`Looper::new`] makes for `seconds` at `rate`: the seconds,
/// one more per layer, and never more than a block number can name
/// (65535 blocks, about 25 minutes at 44.1 kHz).
fn n_blocks(rate: u32, seconds: u32) -> usize {
    let frames = (rate as usize) * (seconds as usize);
    (blocks_for(frames) + MAX_LAYERS).min(NONE as usize)
}

/// The memory [`Looper::new`] sets aside for `seconds` at `rate`, in
/// bytes: the sound itself and its tables of blocks, and the last
/// [`HEARD_SECONDS`] of input that Capture takes from. For showing what a
/// size costs before it is made.
pub fn bytes_for(rate: u32, seconds: u32) -> usize {
    let per_block = 2 * BLOCK * std::mem::size_of::<f32>()
        + (MAX_LOOPS * MAX_LAYERS + 1) * std::mem::size_of::<u16>();
    n_blocks(rate, seconds) * per_block
        + 2 * heard_frames(rate, seconds) * std::mem::size_of::<f32>()
}

/// The frames of input kept for Capture: [`HEARD_SECONDS`], with any
/// memory for loops at all.
fn heard_frames(rate: u32, seconds: u32) -> usize {
    if seconds > 0 {
        rate as usize * HEARD_SECONDS as usize
    } else {
        0
    }
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
    /// Launched: the loop plays (fading in and out as a mute does). Off,
    /// it is silent but keeps its place in the song, so a launch brings it
    /// back where it would have been ([`Looper::launch`]).
    on: bool,
    /// A launch waiting for a bar line: on or off from that tick.
    cue: Option<(bool, Tick)>,
    origin: Tick,
    len_ticks: Tick,
    /// Frames; 0 until the first pass ends.
    len: usize,
    /// [`Edit::Speed`]'s speed, and the octaves the audio plays faster
    /// than recorded ([`Status::rate`]).
    speed: i8,
    rate: i8,
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
    /// Frames of the first pass written so far, while it records.
    written: usize,
    end_tick: Option<Tick>,
    ending: bool,
    /// A bar's ticks when the first pass started (STOP rounds the loop to
    /// whole bars of it).
    bar: Tick,
    /// The last bar line the first pass reached (its tick and frame), and
    /// the frame the next one falls on at the tempo then.
    last_bar: (Tick, u64),
    next_bar: u64,
    /// Frames of the first pass that hold its sound: `len` unless STOP cut
    /// it short ([`Loop::stop_first`]), and then the rest is silent.
    cut: usize,
    /// Go straight into an overdub when the first pass ends.
    then_overdub: bool,
    /// The overdub opened as smart loop end's Auto closed the first pass:
    /// its first frame takes what was heard after the loop's end first
    /// ([`Loop::hand_off`]).
    od_handoff: bool,
    /// Overdubs begun with what was heard after the loop's end
    /// ([`Loop::hand_off`]), ever: [`Looper::stats`] adds them up.
    handoffs: u64,
    after: After,
    /// Fade old: the share of the older audio kept each pass while
    /// overdubbing.
    fade_old: f32,

    /// The first pass's writer is still at work (it runs `latency` frames
    /// behind).
    first_open: bool,
    /// Its writer keeps what it hears after the loop's end, up to
    /// [`KEEP_AFTER`] frames, while the loop plays.
    tail: bool,
    /// What was heard before the first pass, copying in as its first
    /// [`KEEP_BEFORE`] frames: the frame heard that is the pass's first,
    /// and the frames copied.
    before: Option<(u64, usize)>,
    /// The latency the first pass is written with (fixed when it starts).
    rec_latency: u32,
    /// What the writer took before the first pass's first frame, oldest
    /// at `ring_at` (for a resample's loop point, [`Loop::keep_pre`]).
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
    /// Frames of the writer's fade-out (an overdub of one pass ends with
    /// its fade-in's length, so its end and start crossfade).
    od_out_len: usize,
    /// How overdubs record, and whether this one ends after a pass.
    overdubs: Overdubs,
    od_once: bool,
    /// The loudest input sample this overdub took, and whether Fade old
    /// turned the older layers down during it: an overdub of silence that
    /// faded nothing leaves no layer.
    od_peak: f32,
    od_faded: bool,
    // Gains.
    gain: f32,
    clearing: bool,
    /// The clear fading out keeps the loop for Undo ([`Command::Clear`]).
    keep_clear: bool,
    /// Soloed, and silent because another loop is.
    solo: bool,
    solo_out: bool,
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
    /// The first pass's loop in the audio it kept ([`Shape::region`]).
    region: Option<(usize, usize)>,
    /// Where the first pass began and closed in the audio it kept.
    captured: (usize, usize),
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
    /// Overdub pressed at another speed before its print was in: the fit
    /// (and the overdub) start once it is.
    print_wait: bool,
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
    /// A saved loop copying in: its layers (slots 0 on, their mixes set),
    /// frames, ticks and start, frames copied of all the layers, the
    /// period it started in, and the frame it starts playing on.
    loading: bool,
    ld_layers: usize,
    ld_frames: usize,
    ld_ticks: Tick,
    ld_origin: Tick,
    ld_done: usize,
    ld_from: u64,
    ld_at: u64,
    /// A layer of another loop copying in ([`Looper::copy_layer`]): from
    /// which loop and slot, into which slot, frames copied of
    /// `ad_frames`, the period it started in and the frame it joins on
    /// (as a load's), where the other loop's audio is read for this
    /// loop's frame 0 and how many of its frames go by for each frame
    /// here, and whether it then leaves the other loop (a move). Into an
    /// empty loop, the loop's length in ticks and start once it joins.
    adding: bool,
    ad_from: usize,
    ad_src: usize,
    ad_slot: usize,
    ad_frames: usize,
    ad_done: usize,
    ad_start: u64,
    ad_at: u64,
    ad_shift: f64,
    ad_rate: f64,
    ad_move: bool,
    ad_ticks: Tick,
    ad_origin: Tick,
    /// Another loop is copying one of this loop's layers.
    lend: Option<Lend>,
    /// A capture waiting for its bar line or its last frames.
    capture: Option<Capture>,
    /// A load from what was heard (a capture copying in): the frame heard
    /// that is the loop's first.
    ld_heard: Option<u64>,
    /// When the next first pass starts ([`Starts`]), and whether a loop
    /// waiting for you to play listens to its input
    /// ([`Command::StartsByEar`]).
    starts: Starts,
    by_ear: bool,
    /// Armed to start when you play: the song frame a note was played on,
    /// not placed on a bar line yet ([`Looper::place_hits`]).
    hit: Option<u64>,
    /// A note placed: the first pass starts on the next bar line, with
    /// its first frame played on this song frame, and whether that line
    /// stays the loop's start (a pickup: the frames before it lead into
    /// the first bar) or the note is the start (the song started on it).
    pending: Option<(u64, bool)>,
    /// Armed on the bar this period ([`Command::Record`]): the next
    /// period's start notes whether the song was playing.
    just_armed: bool,
    /// Armed on the bar with the song playing: the song tick of the tap,
    /// and the one [`LEAD_BEATS`] after it. No bar line between them
    /// starts the first pass, so a tap just before a bar line waits for
    /// the next one instead of starting at once (a line on the tap's own
    /// tick, the song starting with it, still does).
    tap_lead: Option<(f64, f64)>,
    /// The song is stopped and a note came: the host is asked to start
    /// it ([`Looper::take_start`]).
    want_start: bool,
    /// The song frame the first pass's first frame was played on: its
    /// start, or before it with a pickup or a start from a note.
    rec_from: u64,
    /// A pickup's frames: the first pass's view starts that far in, so
    /// they play at the loop's end ([`Status::lead`]).
    lead: usize,
    /// The first pass's frames heard before it started, copying in from
    /// what was heard.
    fill: Option<Fill>,
    /// The first pass records free, with the song stopped: no bars yet,
    /// its length sets the tempo ([`Command::TempoFromLoop`]).
    free: bool,
    /// Smart loop end: the frame the free first pass would close on
    /// without it (the closing tap), once Record closed it and it reads.
    tap: Option<u64>,
    /// Auto: listening on past the tap to the end of the reach.
    closing: bool,
    /// Suggest: the closed pass waits to the end of the reach for its
    /// decision.
    smart_wait: Option<SmartWait>,
    /// What smart loop end found (None: nothing to change), and where the
    /// loop's start and end now are, from the press and the tap; both hold while the loop is at `smart_version` (any other
    /// change to the sound ends them).
    found: Option<Shift>,
    moved: Shift,
    smart_version: u64,
    /// A snap to land on its frame ([`Looper::snap_at`]), and what became
    /// of the last one, for the host.
    snap: Option<SnapPlan>,
    snap_done: Option<SnapDone>,
    /// The snap's frame (`u64::MAX` with none): checked frame by frame.
    snap_frame: u64,
}

/// A closed free first pass waiting for smart loop end's decision.
#[derive(Clone, Copy, Debug)]
struct SmartWait {
    tap: u64,
    reach: u64,
    start: u64,
    bar: Tick,
    beat: Tick,
    /// How the tap's length read: the bars an end found keeps.
    reading: Reading,
}

/// `like`'s bars over `len` frames instead: the reading of an end smart
/// loop end found, a few milliseconds from the tap, keeps the tap's bar
/// count (None when that tempo is out of the app's range).
fn reread(len: usize, rate: u32, bar: Tick, like: &Reading) -> Option<Reading> {
    let bpm = reading::tempo_of(len, rate, bar, like.bars);
    (bpm.is_finite() && (LOOP_MIN_BPM..=LOOP_MAX_BPM).contains(&bpm))
        .then_some(Reading { bpm, ..*like })
}

/// The start of a first pass that was heard before the pass began (its
/// bar line had passed, or a pickup came before it): copied in from what
/// was heard at [`LOAD_SPEED`], while the writer takes the input from
/// `to` on.
#[derive(Clone, Copy, Debug)]
struct Fill {
    /// The frame heard that is the pass's first.
    from: u64,
    done: usize,
    to: usize,
    /// The frame it began on (its pace counts from there).
    at: u64,
}

/// A capture waiting ([`Command::Capture`]): the bars asked for, the bar
/// line it ends on (counted as [`Heard`] counts them), whether that line
/// is still to come, and once it has come, the take.
#[derive(Clone, Copy, Debug)]
struct Capture {
    bars: u32,
    end: u64,
    wait: bool,
    take: Option<Take>,
}

/// One of a loop's layers being copied into another loop: its slot, the
/// loop and the slot it goes to, the frame the copy joins that loop on,
/// and whether it leaves this loop then (a move).
#[derive(Clone, Copy, Debug)]
struct Lend {
    slot: u8,
    to: u8,
    to_slot: u8,
    at: u64,
    drop: bool,
}

impl Loop {
    const NEW: Loop = Loop {
        phase: Phase::Empty,
        bars: Some(2),
        latency: 0,
        level: 1.0,
        muted: false,
        on: true,
        cue: None,
        origin: 0,
        len_ticks: 0,
        len: 0,
        speed: 0,
        rate: 0,
        order: [0; MAX_LAYERS],
        in_order: 0,
        slots: [Slot::NEW; MAX_LAYERS],
        done: [NO_ACT; HISTORY],
        n_done: 0,
        undone: [NO_ACT; HISTORY],
        n_undone: 0,
        start_frame: 0,
        written: 0,
        end_tick: None,
        ending: false,
        bar: 4 * PPQ as Tick,
        last_bar: (0, 0),
        next_bar: 0,
        cut: 0,
        then_overdub: false,
        od_handoff: false,
        handoffs: 0,
        after: After::Play,
        fade_old: 1.0,
        first_open: false,
        tail: false,
        before: None,
        rec_latency: 0,
        ring: [0.0; 2 * FADE],
        ring_at: 0,
        od_open: false,
        od_layer: 0,
        od_in: 0,
        od_fade: FADE,
        od_stop_in: None,
        od_out: 0,
        od_out_len: FADE,
        overdubs: Overdubs::Add,
        od_once: false,
        od_peak: 0.0,
        od_faded: false,
        gain: 0.0,
        clearing: false,
        keep_clear: false,
        solo: false,
        solo_out: false,
        pos: 0.0,
        jump: 0,
        jump_pos: 0.0,
        mixing: false,
        views: [View::EMPTY; MAX_LAYERS],
        seam: FADE,
        region: None,
        captured: (0, 0),
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
        print_wait: false,
        fit_slot: 0,
        fit_shape: 0,
        fit_done: 0,
        fit_frames: 0,
        fit_from: 0,
        fit_at: 0,
        swapping: false,
        swap_shape: 0,
        loading: false,
        ld_layers: 0,
        ld_frames: 0,
        ld_ticks: 0,
        ld_origin: 0,
        ld_done: 0,
        ld_from: 0,
        ld_at: 0,
        adding: false,
        ad_from: 0,
        ad_src: 0,
        ad_slot: 0,
        ad_frames: 0,
        ad_done: 0,
        ad_start: 0,
        ad_at: 0,
        ad_shift: 0.0,
        ad_rate: 1.0,
        ad_move: false,
        ad_ticks: 0,
        ad_origin: 0,
        lend: None,
        capture: None,
        ld_heard: None,
        starts: Starts::Bar,
        by_ear: false,
        hit: None,
        pending: None,
        just_armed: false,
        tap_lead: None,
        want_start: false,
        rec_from: 0,
        lead: 0,
        fill: None,
        free: false,
        tap: None,
        closing: false,
        smart_wait: None,
        found: None,
        moved: Shift::NONE,
        smart_version: 0,
        snap: None,
        snap_done: None,
        snap_frame: u64::MAX,
    };

    /// Armed to start when you play, and no note yet.
    fn listening(&self) -> bool {
        self.phase == Phase::Armed
            && self.starts == Starts::Play
            && !self.resample
            && self.hit.is_none()
            && self.pending.is_none()
    }

    /// Forget a note heard or placed (the loop listens again, if armed).
    fn unhear(&mut self) {
        self.hit = None;
        self.pending = None;
        self.want_start = false;
    }

    /// Recording or about to: armed, the first pass, or an overdub.
    fn recording(&self) -> bool {
        matches!(self.phase, Phase::Armed | Phase::Recording) || self.od_open
    }

    /// A capture waiting, or copying in.
    fn capturing(&self) -> bool {
        self.capture.is_some() || (self.loading && self.ld_heard.is_some())
    }

    /// The capture is called off: the loop stays empty (its memory goes
    /// back with the reclaim after the command).
    fn call_off_capture(&mut self) {
        self.capture = None;
        if self.ld_heard.is_some() {
            self.loading = false;
            self.ld_heard = None;
        }
    }
}

/// The looper: its loops, their shared memory and the command queue.
pub struct Looper {
    store: Store,
    capacity: usize,
    /// Listen: a loop (or one of its layers) played alone while the song
    /// is stopped ([`Looper::listen`]).
    listen: Option<Listen>,
    loops: Vec<Loop>,
    selected: usize,
    queue: [Queued; QUEUE],
    queued: usize,
    stats: Stats,
    /// The period's first frame.
    now: u64,
    bounce: Option<Bouncing>,
    prints: Vec<Prints>,
    /// Each loop's saved loop being loaded, or loaded and waiting for
    /// [`Looper::take_loaded`].
    loads: Vec<Option<LoopFile>>,
    /// The last [`HEARD_SECONDS`] of input and its bar lines, for Capture.
    heard: Heard,
    rate: u32,
    /// Tempo from the first loop ([`Command::TempoFromLoop`]).
    free_tempo: bool,
    /// The song's tempo was set by hand ([`Command::TempoMeant`]).
    tempo_meant: bool,
    /// The song was stopped at the period's start, and its bar's ticks
    /// then.
    stopped: bool,
    bar_ticks: Tick,
    /// A free first pass closed: its loop, its reading, the frame the
    /// song starts on and the ticks into the loop it starts at
    /// ([`Looper::take_tempo`]).
    tempo_out: Option<(usize, Reading, u64, Tick)>,
    /// A free first pass may be recording: [`Looper::close_free`] looks
    /// at the loops only then, not every period.
    free_open: bool,
    /// Smart loop end ([`Command::SetSmartEnd`]), the notes it weighs and
    /// what it made of the last tap, for the host.
    smart: SmartEnd,
    notes: NoteLog,
    smart_out: Option<SmartNote>,
    /// Smart loop end listens ([`Command::SmartByEar`]): the hit track,
    /// the hits it heard, and the finder's ask under way.
    by_ear: bool,
    track: HitTrack,
    hits: HitLog,
    find: Option<Finding>,
    find_gen: u64,
    /// The last period's last span: where the song goes from here, for a
    /// snap's downbeat ([`Looper::snap_plan`]).
    last_span: Option<Span>,
}

impl Looper {
    /// A looper with room for `seconds` of stereo audio at `rate`, shared
    /// by every loop, its first passes and its overdubs, and a little more:
    /// memory comes in blocks of [`BLOCK`] frames, and one block per layer
    /// of a loop is added so that whole layers fill what the seconds
    /// promise. [`Looper::memory`] gives the exact size. Allocates; call
    /// it before the sound starts.
    pub fn new(rate: u32, seconds: u32) -> Looper {
        let n_blocks = n_blocks(rate, seconds);
        Looper {
            store: Store::new(n_blocks),
            capacity: n_blocks * BLOCK,
            listen: None,
            loops: vec![Loop::NEW; MAX_LOOPS],
            selected: 0,
            queue: [Queued::Loop(Command::Clear); QUEUE],
            queued: 0,
            stats: Stats::default(),
            now: 0,
            bounce: None,
            prints: (0..MAX_LOOPS).map(|_| Prints::default()).collect(),
            loads: (0..MAX_LOOPS).map(|_| None).collect(),
            heard: Heard::new(heard_frames(rate, seconds)),
            rate,
            free_tempo: false,
            tempo_meant: false,
            stopped: true,
            bar_ticks: 4 * PPQ as Tick,
            tempo_out: None,
            free_open: false,
            smart: SmartEnd::Off,
            notes: NoteLog::new(),
            by_ear: false,
            track: HitTrack::new(rate),
            hits: HitLog::new(),
            find: None,
            find_gen: 0,
            smart_out: None,
            last_span: None,
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
    /// Listen: play loop `n` alone from its start, round and round, while
    /// the song is stopped: the whole loop as heard (its layers' levels,
    /// mutes and solos), or with `layer` ([`LayerInfo::id`]) that layer on
    /// its own, even when muted. It plays at the speed it was recorded,
    /// through the loop's level. It stops with [`Looper::stop_listening`],
    /// or by itself when the song starts or the loop records or empties.
    /// Refused (false) while the song plays, or for a loop with nothing
    /// to play.
    pub fn listen(&mut self, n: usize, layer: Option<u8>) -> bool {
        let Some(l) = self.loops.get(n) else {
            return false;
        };
        let playable = matches!(l.phase, Phase::Playing)
            && l.len > 0
            && !l.merging
            && layer.is_none_or(|id| l.order[..l.in_order].contains(&id));
        if !self.stopped || !playable {
            return false;
        }
        self.listen = Some(Listen {
            n,
            layer,
            pos: 0,
            gain: 0.0,
            stopping: false,
        });
        true
    }

    /// Listen fades out and stops.
    pub fn stop_listening(&mut self) {
        if let Some(l) = &mut self.listen {
            l.stopping = true;
        }
    }

    /// What Listen plays: the loop, the layer (None: all of it as heard)
    /// and where it has got to, 0 to 1.
    pub fn listening(&self) -> Option<(usize, Option<u8>, f32)> {
        let l = self.listen.filter(|l| !l.stopping)?;
        let len = self.loops[l.n].len.max(1);
        Some((l.n, l.layer, l.pos as f32 / len as f32))
    }

    /// One period of Listen, added to `out` (n frames, interleaved).
    fn play_listen(&mut self, block: &Block, n: usize, out: &mut [f32]) {
        let Some(mut ls) = self.listen else {
            return;
        };
        let stopped = block.spans().iter().all(|s| s.state == State::Stopped);
        let l = &self.loops[ls.n];
        let still = l.phase == Phase::Playing
            && l.len > 0
            && !l.merging
            && ls
                .layer
                .is_none_or(|id| l.order[..l.in_order].contains(&id));
        if !still {
            // The loop changed under it: stop at once (it is silent or
            // recording anyway).
            self.listen = None;
            return;
        }
        if !stopped {
            ls.stopping = true;
        }
        let gains = l.listen_gains(ls.layer);
        let replacing = ls.layer.is_none() && l.replacing();
        let level = l.level;
        let len = l.len;
        let step = 1.0 / FADE as f32;
        let mut f = 0;
        while f < n {
            let k = RUN.min(n - f).min(len - ls.pos);
            let mut acc = [[0.0f32; 2]; RUN];
            l.render_layers_at(&self.store, ls.n, ls.pos, &mut acc[..k], &gains, replacing);
            for (j, a) in acc[..k].iter().enumerate() {
                ls.gain = if ls.stopping {
                    (ls.gain - step).max(0.0)
                } else {
                    (ls.gain + step).min(1.0)
                };
                let g = ls.gain * level;
                out[2 * (f + j)] += a[0] * g;
                out[2 * (f + j) + 1] += a[1] * g;
            }
            f += k;
            ls.pos = (ls.pos + k) % len;
            if ls.stopping && ls.gain == 0.0 {
                self.listen = None;
                return;
            }
        }
        self.listen = Some(ls);
    }

    pub fn select_loop(&mut self, n: usize) -> bool {
        self.push(Queued::Select(n))
    }

    /// Copy layer `id` ([`LayerInfo::id`]) of loop `from` onto loop `to`
    /// for the next period; `keep` false moves it: once the copy plays it
    /// leaves `from` as a delete, unless it is that loop's last layer.
    /// The layer goes as it plays, through `from`'s edits, at its level,
    /// pan, mute and Fade old turn-down (not its solo). Loop `to` must be
    /// empty, and then takes `from`'s length and start, or play a loop
    /// of the same length in ticks: the layer goes in time with the song,
    /// made to that loop's length in frames. Both loops must be playing
    /// with nothing changing their layers, and `to` needs a layer slot
    /// and the memory. It copies at [`COPY_SPEED`] while both play on;
    /// meanwhile the commands that would change either loop's layers
    /// wait, except Undo on `to` and Clear on either, which call it off.
    /// Refused (counted) otherwise. False (and counted) if the queue is
    /// full.
    pub fn copy_layer(&mut self, from: usize, id: u8, to: usize, keep: bool) -> bool {
        self.push(Queued::Copy { from, id, to, keep })
    }

    /// Launch loop `n` (`on`) or stop it, as a clip launches: from the
    /// bar line at song tick `at` (the frame it falls on), or with `None`
    /// at once. Either way it fades in or out as a mute does, and an off
    /// loop keeps its place in the song, so it comes back in step. A new
    /// launch replaces one waiting; `on` with nothing waiting and the loop
    /// already on changes nothing. Recording a first pass turns a loop on.
    /// It doesn't wait for the selected loop's fades. False (and counted)
    /// if the queue is full.
    pub fn launch(&mut self, n: usize, on: bool, at: Option<Tick>) -> bool {
        self.push(Queued::Launch { n, on, at })
    }

    /// The loop the commands act on (as of the last period).
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Room left in the command queue this period.
    pub fn room(&self) -> usize {
        QUEUE - self.queued
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
        Stats {
            handoffs: self.loops.iter().map(|l| l.handoffs).sum(),
            hits_heard: self.track.found,
            ..self.stats
        }
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
            overdub_next: l.then_overdub || l.fit_overdub || l.print_wait,
            after: l.after,
            fade_old: l.fade_old,
            latency: l.latency,
            level: l.level,
            muted: l.muted,
            on: l.on,
            cue: l.cue,
            seam: l.seam,
            merging: l.merging.then(|| l.mg_done as f32 / l.len.max(1) as f32),
            resampling: l.resample && (l.first_in_progress() || l.phase == Phase::Armed),
            capacity: self.capacity,
            used: (self.store.blocks(n) - self.store.kept_blocks_of(n)) * BLOCK,
            version: l.version,
            printed: l.print_on > 0.0,
            fitting: l
                .fitting
                .then(|| l.fit_done as f32 / l.fit_frames.max(1) as f32),
            loading: (l.loading && l.ld_heard.is_none())
                .then(|| l.ld_done as f32 / (l.ld_frames * l.ld_layers).max(1) as f32),
            copying: l
                .adding
                .then(|| l.ad_done as f32 / l.ad_frames.max(1) as f32),
            recorded: if l.phase == Phase::Recording {
                l.written
            } else {
                0
            },
            // (Not once an edit has made it longer than the pass was.)
            cut: (l.len > 0 && l.cut < l.len && l.cut < l.captured.1 - l.captured.0)
                .then_some(l.cut),
            starts_on: (l.phase == Phase::Armed)
                .then_some(l.tap_lead)
                .flatten()
                .map(|(tap, until)| {
                    let bar = self.bar_ticks.max(1) as f64;
                    let line = (tap / bar).ceil() * bar;
                    (if line > tap && line < until {
                        (until / bar).ceil() * bar
                    } else {
                        line
                    }) as Tick
                }),
            kept: self.store.has_kept(n),
            solo: l.solo,
            solo_out: l.solo_out,
            overdubs: l.overdubs,
            speed: l.speed,
            rate: l.rate,
            capturing: if l.capture.is_some() {
                Some(0.0)
            } else {
                (l.loading && l.ld_heard.is_some())
                    .then(|| l.ld_done as f32 / l.ld_frames.max(1) as f32)
            },
            starts: l.starts,
            listening: l.listening(),
            lead: l.bounds_now().map_or(l.lead, |(start, src)| src - start),
            free: l.free
                || ((l.phase == Phase::Empty || l.listening()) && self.stopped && self.frees(n)),
            bounds: l.bounds_now().map(|(start, src)| Bounds {
                src: src as i64,
                start: start as i64,
                captured: (l.captured.0 as i64, l.captured.1 as i64),
                kept: l.slots[0].valid.1 as i64,
            }),
            closing: l.closing,
            offer: l
                .smart_now()
                .and_then(|(f, m)| f.map(|f| f - m))
                .filter(|o| !o.is_none()),
            moved: l.smart_now().map(|(_, m)| m).filter(|m| !m.is_none()),
            snapping: l.snap.map(|p| p.at),
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

    /// Loop `n` as it would be saved, without its layers' audio (read it
    /// with [`Looper::read_layer`]) and with no file names. None unless it
    /// is playing with nothing changing its layers: no overdub, first pass
    /// still being written, merge, fit or load. Allocates (the list of
    /// layers): not for the audio thread.
    pub fn loop_file(&self, n: usize) -> Option<LoopFile> {
        let l = self.loops.get(n)?;
        if !l.savable() {
            return None;
        }
        let layers = (0..l.layers())
            .filter_map(|k| l.listed(k))
            .map(|slot| {
                let s = &l.slots[slot];
                LayerFile {
                    level: s.level,
                    pan: s.pan,
                    muted: s.mute,
                    solo: s.solo,
                    faded: s.age,
                    file: String::new(),
                    audio: Vec::new(),
                }
            })
            .collect();
        Some(LoopFile {
            frames: l.len,
            ticks: l.len_ticks,
            origin: l.origin,
            level: l.level,
            muted: l.muted,
            bars: l.bars,
            after: l.after,
            starts: l.starts,
            fade_old: l.fade_old,
            speed: l.speed,
            rate: l.rate,
            layers,
        })
    }

    /// Frames `from` on of layer `k` of loop `n` (in play order) as it
    /// plays, through the loop's edits and their seam fades, at gain 1
    /// (silent where a Replace layer after it covers it, as heard),
    /// into `out` (interleaved stereo, wrapping round the loop). Gives the
    /// loop's version ([`Status::version`]), so that a save read across
    /// several periods can tell that the layers changed; None, writing
    /// nothing, when the loop could not be saved ([`Looper::loop_file`])
    /// or has no such layer. Does not allocate.
    pub fn read_layer(&self, n: usize, k: usize, from: usize, out: &mut [f32]) -> Option<u64> {
        let l = self.loops.get(n)?;
        if !l.savable() {
            return None;
        }
        let slot = l.listed(k)?;
        let blocks = self.store.blocks_of(n, slot);
        let (valid, view) = (l.slots[slot].valid, &l.views[slot]);
        // Under a Replace layer it is saved as heard: silent where that
        // covers it, so the loop loads sounding the same.
        let replacing = l.replacing();
        let any_solo = l.any_solo();
        let strength = |s: &Slot| if s.heard(any_solo) { 1.0 } else { 0.0 };
        for (i, y) in out.chunks_exact_mut(2).enumerate() {
            let p = (from + i) % l.len;
            let mut x = layer_frame(&self.store, blocks, valid, view, l.seam, p);
            if replacing {
                let d = l.ducks(&l.views, l.len, p, strength)[slot];
                x = x.map(|v| v * d);
            }
            y.copy_from_slice(&x);
        }
        Some(l.version)
    }

    /// Put a saved loop back into loop `n`, which must be empty: its
    /// settings now, and its layers once their audio has copied in (at
    /// [`LOAD_SPEED`]); then it fades in, in time with the song, from
    /// `origin`. Takes its memory now. Its commands wait while it copies,
    /// except Clear (which calls it off), Mute and the settings. The
    /// [`LoopFile`] waits in [`Looper::take_loaded`] once done, so that
    /// its audio is not freed here. Does not allocate. Gives `f` back if
    /// the loop is not empty, the last load has not been collected, the
    /// loop number is past [`MAX_LOOPS`], there are no layers or more
    /// than `MAX_LAYERS - 1`, a layer is shorter than `frames`, or the
    /// memory is short.
    pub fn load(&mut self, n: usize, f: LoopFile) -> Result<(), LoopFile> {
        let Some(l) = self.loops.get_mut(n) else {
            return Err(f);
        };
        let ok = l.phase == Phase::Empty
            && !l.clearing
            && !l.loading
            && !l.adding
            && self.loads[n].is_none()
            && f.frames > 0
            && f.ticks > 0
            && !f.layers.is_empty()
            && f.layers.len() < MAX_LAYERS
            && f.layers.iter().all(|y| y.audio.len() >= 2 * f.frames);
        if !ok {
            return Err(f);
        }
        let need = f.layers.len() * blocks_for(f.frames);
        if self.store.available() < need {
            return Err(f);
        }
        // A loaded loop takes the place of one kept for Undo.
        self.store.drop_kept(n);
        l.reclaim(&mut self.store, n);
        for (k, y) in f.layers.iter().enumerate() {
            self.store.grow(n, k, blocks_for(f.frames));
            let mut s = Slot {
                valid: (0, f.frames),
                level: y.level.clamp(0.0, 2.0),
                pan: y.pan.clamp(-1.0, 1.0),
                mute: y.muted,
                solo: y.solo,
                age: y.faded.clamp(0.0, 1.0),
                ..Slot::NEW
            };
            let any_solo = f.layers.iter().any(|y| y.solo);
            (s.gl, s.gr) = s.target(any_solo);
            l.slots[k] = s;
        }
        l.level = f.level.clamp(0.0, 2.0);
        l.muted = f.muted;
        l.bars = f.bars.filter(|&b| b > 0);
        l.after = f.after;
        l.starts = f.starts;
        l.fade_old = f.fade_old.clamp(0.0, 1.0);
        l.speed = f.speed.clamp(-1, 1);
        l.rate = f.rate.clamp(-2, 2);
        l.loading = true;
        l.ld_layers = f.layers.len();
        l.ld_frames = f.frames;
        l.ld_ticks = f.ticks;
        l.ld_origin = f.origin;
        l.ld_done = 0;
        l.ld_from = self.now;
        l.ld_at = self.now + (f.layers.len() * f.frames).div_ceil(LOAD_SPEED) as u64;
        self.loads[n] = Some(f);
        Ok(())
    }

    /// What a Capture of `bars` would take now ([`Command::Capture`]):
    /// the bars ending on the last bar line heard (pressed in a bar's last
    /// beat, it waits for the next one instead), read with the selected
    /// loop's latency. None when not even one whole bar was heard.
    pub fn capture_take(&self, bars: u32) -> Option<Take> {
        let end = self.heard.lines().checked_sub(1)?;
        self.heard
            .take(end, bars, self.loops[self.selected].latency)
    }

    /// What [`Command::CaptureTo`] would take now, for the selected loop:
    /// `bars` ending on the bar line heard on tick `end`.
    pub fn capture_take_to(&self, end: Tick, bars: u32) -> Option<Take> {
        let end = self.heard.line_at(end)?;
        self.heard
            .take(end, bars, self.loops[self.selected].latency)
    }

    /// A Capture now would wait for the bar line coming: the song plays
    /// and is in a bar's last beat.
    pub fn capture_waits(&self) -> bool {
        self.heard.in_last_beat()
    }

    /// The bar lines of the song as it last played on unbroken (since it
    /// started, jumped or looped), the newest up to `out.len()`, oldest
    /// first: each one's tick and the frame it fell on. Returns how many.
    /// With [`Looper::heard_at`], where What you played puts its bars.
    pub fn heard_lines(&self, out: &mut [(Tick, u64)]) -> usize {
        self.heard.recent(out)
    }

    /// The frame after the last one the looper heard (the end of the
    /// period it last ran).
    pub fn heard_at(&self) -> u64 {
        self.heard.at()
    }

    /// The saved loop loop `n` has loaded (or called off), to drop away
    /// from the audio thread. None while it is still copying in.
    pub fn take_loaded(&mut self, n: usize) -> Option<LoopFile> {
        if self.loops.get(n)?.loading {
            return None;
        }
        self.loads.get_mut(n)?.take()
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
            rate: l.rate,
        })
    }

    /// The memory gauge.
    pub fn memory(&self) -> Memory {
        Memory {
            used: (self.store.n_blocks - self.store.available()) * BLOCK,
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

    /// Loop `n`'s first pass as far as it has recorded, while it records
    /// ([`Status::recorded`] frames), for a waveform that grows as it
    /// records. None before the first frame and once the pass has ended.
    pub fn first_pass(&self, n: usize) -> Option<LayerView<'_>> {
        let l = self.loops.get(n)?;
        let kept = KEEP_BEFORE + l.written;
        let view = View::region(kept, KEEP_BEFORE, l.written, KEEP_BEFORE)?;
        (l.phase == Phase::Recording).then_some(LayerView {
            store: &self.store,
            lp: n,
            slot: 0,
            len: l.written,
            valid: (0, kept),
            view,
        })
    }

    /// Loop `n`'s first pass as it was kept, not through its edits:
    /// frames `from..from + out.len()` of the audio it kept (frame 0 its
    /// first, [`KEEP_BEFORE`] before Record), silence outside it; and where
    /// the loop is in it now: its first frame and its length. For the loop
    /// point view, which draws your press, your tap and what smart loop
    /// end found, with the audio round them. None (`out` silent) unless
    /// the first pass plays alone as a stretch of what it kept (an
    /// overdub, a cut or a merge ends that). Allocation-free.
    pub fn read_kept(&self, n: usize, from: i64, out: &mut [[f32; 2]]) -> Option<(usize, usize)> {
        out.fill([0.0; 2]);
        let l = self.loops.get(n)?;
        let (start, _) = l.bounds_now()?;
        let size = l.views[0].size();
        let a = from.clamp(0, size as i64) as usize;
        let b = from.saturating_add(out.len() as i64).clamp(0, size as i64) as usize;
        if b > a {
            let at = (a as i64 - from) as usize;
            LayerView {
                store: &self.store,
                lp: n,
                slot: 0,
                len: b - a,
                valid: l.slots[0].valid,
                view: View::region(size, a, b - a, a)?,
            }
            .read(0, &mut out[at..at + (b - a)]);
        }
        Some((start, l.len))
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
            replace: s.replace.is_some(),
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
        self.last_span = block.spans().last().copied();
        if let Some(s) = block.spans().first() {
            self.stopped = s.state == State::Stopped;
            self.bar_ticks = s.meter.bar_ticks();
        }
        self.heard.hear(block, input, n);
        self.apply_commands();
        // Smart loop end by ear: the hit track listens while a free first
        // pass may need it.
        if self.by_ear && self.wants_notes() {
            let latency = self.loops.first().map_or(0, |l| l.latency);
            self.track
                .hear(block.start_frame, &input[..2 * n], latency, &mut self.hits);
        }
        // A loop armed with the song playing starts no sooner than
        // LEAD_BEATS after the tap; armed while stopped or counting in,
        // on the bar PLAY starts on (with the pads' pattern, SHIFT + Arm).
        if let Some(s) = block.spans().first() {
            let rolling = matches!(s.state, State::Playing | State::Recording);
            let tap = s.tick_at(s.frame);
            let until = tap + (LEAD_BEATS * s.meter.beat_ticks()) as f64;
            for l in &mut self.loops {
                if std::mem::take(&mut l.just_armed) && l.phase == Phase::Armed {
                    l.tap_lead = rolling.then_some((tap, until));
                }
            }
        }
        self.place_hits(block);
        self.close_free(block);
        self.tend_captures();
        self.play_listen(block, n, out);
        // A loop with sound soloed: the others fall silent.
        let any_solo = self.loops.iter().any(|l| l.solo && l.phase != Phase::Empty);
        for l in &mut self.loops {
            l.solo_out = any_solo && !l.solo;
        }
        let end = block.start_frame + n as u64;
        for k in 0..self.loops.len() {
            if self.loops[k].adding {
                self.render_copy(k, end);
            }
        }
        let Looper {
            store,
            loops,
            stats,
            bounce,
            prints,
            loads,
            heard,
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
            if l.print_wait {
                l.tend_print_wait(store, stats, k, pr, block.start_frame);
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
            if l.loading {
                if let Some(f) = loads[k].as_ref() {
                    l.render_load(store, k, f, end);
                } else if l.ld_heard.is_some() {
                    l.render_heard(store, stats, k, heard, end);
                }
            }
            if l.before.is_some() {
                l.render_before(store, stats, k, heard);
            }
            if l.fill.is_some() {
                l.render_fill(store, stats, k, heard, end);
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
                .any(|l| matches!(l.phase, Phase::Armed | Phase::Recording) || l.cue.is_some());
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

    /// A loop waiting for you to play heard a note while the song was
    /// stopped: the host should start the song now (bar 1 from the bar it
    /// stopped in, at the next period's first frame), and the loop's first
    /// pass starts with it, on the note ([`Starts::Play`]). Some(loop)
    /// once per note; [`Looper::start_refused`] if the song can't start.
    pub fn take_start(&mut self) -> Option<usize> {
        let n = self.loops.iter().position(|l| l.want_start)?;
        self.loops[n].want_start = false;
        Some(n)
    }

    /// The song couldn't start on the note ([`Looper::take_start`]): the
    /// loop forgets it and listens again. (The host counts it, and says
    /// why.)
    pub fn start_refused(&mut self) {
        for l in &mut self.loops {
            if l.phase == Phase::Armed && l.pending.is_some_and(|(_, pickup)| !pickup) {
                l.unhear();
            }
        }
    }

    /// A free first pass closed ([`Command::TempoFromLoop`]): its loop, how
    /// its length read (its bars and the tempo they set, how sure, the
    /// other reading), the frame the host starts the song on at that tempo
    /// (the next period's first), and where: this many ticks after the
    /// start of the bar it stopped in (the loop's first bar). 0, where the
    /// loop comes round, unless smart loop end's Auto listened on past
    /// the end it closed on: then the loop is already that far in, and
    /// the song starts there with it. Once per pass. If the song can't
    /// start, the loop plays at the song's tempo when it does (like tape,
    /// until a print is in).
    pub fn take_tempo(&mut self) -> Option<(usize, Reading, u64, Tick)> {
        self.tempo_out.take()
    }

    /// The tempo the song takes from the next period, while
    /// [`Looper::take_tempo`] hasn't been taken: a free first pass closed
    /// in this one.
    pub fn tempo_coming(&self) -> Option<f64> {
        self.tempo_out.as_ref().map(|(_, r, _, _)| r.bpm)
    }

    /// What smart loop end made of a closing tap ([`Command::SetSmartEnd`]),
    /// once per pass: Auto's when the loop closes, Suggest's at the end of
    /// the reach after it.
    pub fn take_smart(&mut self) -> Option<SmartNote> {
        self.smart_out.take()
    }

    /// Snap `s` on loop `n` ([`Snap`]), worked out against where the song
    /// was going at the end of the last period: how far the start and the
    /// end move, the loop's length in song ticks after it (Read as's bars
    /// of the song's meter), the tempo at which those ticks are its new
    /// length exactly, and the loop's next downbeat on or after frame
    /// `after` (the transport's next frame, which may be a period on from
    /// the looper's), its tick and frame. The host asks the time engine
    /// for that tempo on that frame and, once committed, gives the plan to
    /// [`Looper::snap_at`]. Or why it can't: the song isn't playing, the
    /// loop isn't playing as it was left, it plays at another speed, it is
    /// already read as those bars, or the new length is outside what was
    /// kept or the tempo range. Changes nothing.
    pub fn snap_plan(&self, n: usize, s: Snap, after: u64) -> Result<SnapPlan, &'static str> {
        let l = self.loops.get(n).ok_or("there is no such loop")?;
        let span = self
            .last_span
            .filter(|s| matches!(s.state, State::Playing | State::Recording))
            .ok_or("the song isn't playing")?;
        if l.phase != Phase::Playing || l.od_open || l.busy() {
            return Err("the loop is busy");
        }
        if l.speed != 0 || l.rate != 0 {
            return Err("the loop plays at another speed");
        }
        let by = l.snap_by(s)?;
        let ticks = l.snap_ticks(s, span.meter.bar_ticks())?;
        let len = l.len as i64 + by.longer();
        let fits = match s {
            Snap::Found | Snap::Tap | Snap::Press => l.shifted(by).is_some(),
            Snap::Bars(_) | Snap::Undo | Snap::Redo => true,
        };
        if !fits || len <= 0 || ticks <= 0 || l.len_ticks <= 0 {
            return Err("that end is outside what the loop kept");
        }
        let bpm = 60.0 * f64::from(self.rate) * ticks as f64 / (f64::from(PPQ) * len as f64);
        if !(LOOP_MIN_BPM..=LOOP_MAX_BPM).contains(&bpm) {
            return Err("its tempo would be out of range");
        }
        // The next downbeat from the coming period's first frame on, by
        // the span's clock (as `Span::tick_at` and `Span::frame_of` work it
        // out, the frame rounded the same way, so the tempo's anchor falls
        // on that tick).
        let next = after.max(span.frame + u64::from(span.len));
        let (rate, ppq) = (f64::from(span.rate), f64::from(PPQ));
        let t = span.anchor_tick as f64
            + (next as f64 - span.anchor_frame as f64) * span.bpm * ppq / (60.0 * rate);
        let k = ((t - l.origin as f64) / l.len_ticks as f64).floor() as i64 + 1;
        let tick = l.origin + k * l.len_ticks;
        let f = (tick as f64 - span.anchor_tick as f64) * 60.0 * rate / (span.bpm * ppq);
        let at = (span.anchor_frame as i64 + f.round() as i64).max(0) as u64;
        Ok(SnapPlan {
            n,
            snap: s,
            by,
            ticks,
            bpm,
            tick,
            at: at.max(next),
            version: l.version,
        })
    }

    /// Land snap `p` ([`Looper::snap_plan`]) on its frame, now that the
    /// time engine has committed its tempo there: from then the loop
    /// waits for nothing else (commands that change it wait until it has
    /// landed). Call it before [`Looper::process`] runs the period its
    /// frame falls in. False if the loop has changed since (counted, and
    /// said by [`Looper::take_snap`]). Allocation-free.
    pub fn snap_at(&mut self, p: SnapPlan) -> bool {
        let Some(l) = self.loops.get_mut(p.n) else {
            return false;
        };
        if l.version != p.version || l.phase != Phase::Playing || l.snap.is_some() {
            self.stats.refused += 1;
            l.snap_done = Some(SnapDone {
                plan: p,
                landed: false,
            });
            return false;
        }
        l.snap = Some(p);
        l.snap_frame = p.at;
        true
    }

    /// What became of a snap given to [`Looper::snap_at`], once.
    pub fn take_snap(&mut self) -> Option<SnapDone> {
        self.loops.iter_mut().find_map(|l| l.snap_done.take())
    }

    /// A note started (a pad hit or a MIDI note-on, [`NoteStart`]), on the
    /// song's frame clock: smart loop end weighs them ([`ends`]). Give
    /// every note of a period before [`Looper::process`] runs it. Kept
    /// only while it could matter (a free first pass armed, recording or
    /// waiting for its decision); allocation-free.
    pub fn note(&mut self, n: NoteStart) {
        if self.takes_notes() {
            self.notes.push(n);
        }
    }

    /// A note started on `at` that was lost before it got here (the
    /// host had no room for it): a tap whose reach it falls in stands,
    /// as for notes the log itself had to drop.
    pub fn notes_lost(&mut self, at: u64) {
        if self.takes_notes() {
            self.notes.lost(at);
        }
    }

    /// Whether a note given now is kept: tempo from the first loop is on
    /// (the reading weighs the beat, and smart loop end the end), or a
    /// decision waits. (A note in the period
    /// that arms the loop counts: the period's end forgets what nothing
    /// needs, [`Looper::wants_notes`].)
    fn takes_notes(&self) -> bool {
        self.free_tempo || self.loops.iter().any(|l| l.smart_wait.is_some())
    }

    /// Whether the notes are wanted: a free first pass may be armed or
    /// recording (its reading weighs the beat inside it, and smart loop
    /// end, when on, its end), or one waits for smart loop end's decision.
    fn wants_notes(&self) -> bool {
        self.loops.iter().any(|l| l.smart_wait.is_some())
            || (self.free_tempo
                && self
                    .loops
                    .iter()
                    .any(|l| l.phase == Phase::Armed || (l.free && l.phase == Phase::Recording)))
    }

    /// Whether loop `k`, recording a first pass now, would record free
    /// ([`Command::TempoFromLoop`]): on, not resampling, and every other
    /// loop empty (nothing loading, copying in or capturing). Its Length
    /// doesn't matter: a bar count is the bars at the tempo you play.
    fn frees(&self, k: usize) -> bool {
        let l = &self.loops[k];
        self.free_tempo
            && !l.resample
            && self.loops.iter().enumerate().all(|(i, o)| {
                i == k
                    || (o.phase == Phase::Empty && !o.loading && !o.adding && o.capture.is_none())
            })
    }

    /// Loop `k`'s first pass starts free on song frame `rec_from` (the
    /// period's first, or a note before it, copied in from what was
    /// heard).
    fn begin_free(&mut self, k: usize, rec_from: u64) {
        let Looper {
            store,
            loops,
            now,
            bar_ticks,
            ..
        } = self;
        loops[k].begin_free(store, k, rec_from, *now, *bar_ticks);
        self.free_open = true;
        // Smart loop end's notes: from a reach before the pass (a smart
        // start looks there) on.
        let reach = ends::reach(self.rate, f64::NAN);
        self.notes.keep_from(rec_from.saturating_sub(reach));
        self.hits.keep_from(rec_from.saturating_sub(reach));
    }

    /// Each free first pass asked to close ([`Loop::ending`]) closes on
    /// the next period's first frame, once it is long enough
    /// ([`read_bars`]): its bars from the start of the bar the song is
    /// stopped in, and its reading for the host ([`Looper::take_tempo`]).
    fn close_free(&mut self, block: &Block) {
        let end = block.start_frame + u64::from(block.frames);
        self.forget_stale_find();
        self.tend_smart(end);
        if !self.wants_notes() {
            if !self.notes.is_empty() || self.notes.dropped() > 0 {
                self.notes.clear();
            }
            if !self.hits.is_empty() || self.hits.dropped() > 0 {
                self.hits.clear();
            }
        }
        if !self.free_open {
            return;
        }
        let Some(span) = block.spans().first() else {
            return;
        };
        let bar = span.meter.bar_ticks().max(1);
        let beat = span.meter.beat_ticks().max(1);
        let hint = TempoHint {
            bpm: span.bpm,
            kind: if self.tempo_meant {
                TempoKind::Set
            } else {
                TempoKind::Earlier
            },
        };
        let rate = self.rate;
        let mut open = false;
        for k in 0..self.loops.len() {
            let l = &self.loops[k];
            if !(l.free && l.phase == Phase::Recording) {
                continue;
            }
            let said = l.bars;
            let read = |len: usize| read_bars(len, rate, bar, said, Some(hint));
            let rec_from = l.rec_from;
            let len = end.saturating_sub(rec_from) as usize;
            let closing = if l.ending { read(len) } else { None };
            let Some(reading) = closing else {
                open = true;
                continue;
            };
            // The tap: the frame it closes on without smart loop end.
            let tap = *self.loops[k].tap.get_or_insert(end);
            let tap_len = (tap - rec_from) as usize;
            let tap_reading = read(tap_len).unwrap_or(reading);
            let reach = ends::reach(rate, tap_reading.bpm);
            // The reading with the beat inside the loop as evidence: the
            // notes played into it.
            let heard = |len: usize| {
                if self.by_ear {
                    let inner = InnerBeat::new(&self.hits, rate, rec_from, len, bar, beat);
                    read_bars_with(len, rate, bar, said, Some(hint), &|b| inner.fit(b))
                } else {
                    let inner = InnerBeat::new(&self.notes, rate, rec_from, len, bar, beat);
                    read_bars_with(len, rate, bar, said, Some(hint), &|b| inner.fit(b))
                }
            };
            // Auto listens on to the end of the reach first.
            let mut auto = None;
            let (len, reading, lead) = if self.smart == SmartEnd::Auto {
                // By ear, what is heard reaches the writer a latency later.
                let behind = if self.by_ear {
                    u64::from(self.loops[k].latency)
                } else {
                    0
                };
                if end < tap + reach + behind {
                    self.loops[k].closing = true;
                    open = true;
                    continue;
                }
                let ask = ends::Ask {
                    rate,
                    start: rec_from,
                    tap,
                    reach,
                    heard_to: end - behind,
                    bar,
                    beat,
                };
                let tap_reading = heard(tap_len).unwrap_or(tap_reading);
                let bars = tap_reading.bars;
                let d = if self.by_ear {
                    // By ear: the finder's answer, waited for a little.
                    let wait = ms_frames(rate, FIND_WAIT_AUTO_MS);
                    match self.find_for(k, ask, bars, end, wait) {
                        Some(d) => d,
                        None => {
                            self.loops[k].closing = true;
                            open = true;
                            continue;
                        }
                    }
                } else {
                    ends::decide(&self.notes, &ask, &|_| Some(bars))
                };
                // A smart start (your first hit came just after REC) needs
                // what was heard after the end kept: room for it.
                let room = d.lead > 0
                    && self.store.grow(
                        k,
                        0,
                        blocks_for(KEEP_BEFORE + (end - rec_from) as usize + KEEP_AFTER),
                    );
                let first = rec_from + d.lead;
                let from_hit = |to: u64| reread((to - first) as usize, rate, bar, &tap_reading);
                // Moved, the beat's fit is the loop's as it now ends (the
                // reading weighed it at the tap's length).
                let moved = (d.verdict == Verdict::Moved)
                    .then(|| from_hit(d.end))
                    .flatten()
                    .map(|r| Reading {
                        beat: r.beat.and(d.best.map(|b| b.rhythm)),
                        ..r
                    });
                // The loop's length and its start in the pass. Ends are
                // measured from the first hit: with no smart start the loop
                // is as long from the press.
                let (len, lead, reading, outcome) = match moved {
                    Some(r) => {
                        let back = if room { 0 } else { d.lead };
                        let len = (d.end - first) as usize;
                        let by = (d.end - back) as i64 - tap as i64;
                        (len, d.lead - back, r, SmartOutcome::Moved(by))
                    }
                    None => {
                        let why = SmartOutcome::Kept(if d.verdict == Verdict::Moved {
                            "too short a loop"
                        } else {
                            d.verdict.why_by(self.by_ear)
                        });
                        match from_hit(tap).filter(|_| room) {
                            // On your first hit, ending on your tap.
                            Some(r) => ((tap - first) as usize, d.lead, r, why),
                            None => (tap_len, 0, tap_reading, why),
                        }
                    }
                };
                auto = Some((outcome, d.best, d.second));
                (len, reading, lead as usize)
            } else {
                (len, heard(len).unwrap_or(reading), 0)
            };
            let pos = span.ticks().start;
            let origin = pos - pos.rem_euclid(bar);
            let ticks = i64::from(reading.bars) * bar;
            let t = origin + ticks;
            let l = &mut self.loops[k];
            l.free = false;
            l.closing = false;
            l.origin = origin;
            l.bar = bar;
            l.end_tick = Some(t);
            if self.smart == SmartEnd::Suggest {
                l.smart_wait = Some(SmartWait {
                    tap,
                    reach,
                    start: rec_from,
                    bar,
                    beat,
                    reading,
                });
            }
            if !l.finish_len(&mut self.store, &mut self.stats, k, t, len, true) {
                l.forget_smart();
                continue;
            }
            // A smart start (Auto): the loop starts on your first hit, as
            // long, ending on `stop`.
            let start = if lead > 0 && l.tail && l.start_on(lead) {
                lead
            } else {
                0
            };
            // Auto listened on past the end: the song starts as far into
            // the loop as it came round since, and with Then: Overdub,
            // what you played since begins the overdub.
            let into = end.saturating_sub(rec_from + (start + len) as u64);
            let into = (into as f64 * ticks as f64 / len.max(1) as f64).round() as Tick;
            if let Some((outcome, best, second)) = auto {
                l.od_handoff = l.od_open;
                let found = Shift {
                    start: start as i64,
                    end: (rec_from + (start + len) as u64) as i64 - tap as i64,
                };
                l.found = Some(found).filter(|f| !f.is_none());
                l.moved = found;
                l.smart_version = l.version;
                // (Where the start couldn't move, the end is as far from
                // the press.)
                let outcome = match outcome {
                    SmartOutcome::Moved(_) => SmartOutcome::Moved(found.end),
                    o => o,
                };
                self.smart_said(k, tap, outcome, start as i64, best, second);
            }
            self.tempo_out = Some((k, reading, end, into.clamp(0, ticks - 1)));
        }
        self.free_open = open;
    }

    /// Suggest: a closed free pass whose reach has been heard to its end
    /// gets its decision. With an overdub under way (the loop's Then, or
    /// Overdub pressed) the tap stands: Suggest can't wait for you there.
    fn tend_smart(&mut self, end: u64) {
        for k in 0..self.loops.len() {
            let Some(w) = self.loops[k].smart_wait else {
                continue;
            };
            if self.loops[k].od_open {
                self.loops[k].smart_wait = None;
                self.smart_said(k, w.tap, SmartOutcome::Kept(OVERDUB_AT_ONCE), 0, None, None);
                continue;
            }
            // (And for the writer, a latency behind, to have it.)
            if end < w.tap + w.reach + u64::from(self.loops[k].rec_latency) {
                continue;
            }
            let (rate, bar) = (self.rate, w.bar);
            let ask = ends::Ask {
                rate,
                start: w.start,
                tap: w.tap,
                reach: w.reach,
                // (By ear, what the writer has: a latency behind.)
                heard_to: if self.by_ear {
                    end.saturating_sub(u64::from(self.loops[k].rec_latency))
                } else {
                    end
                },
                bar,
                beat: w.beat,
            };
            let bars = w.reading.bars;
            let d = if self.by_ear {
                // By ear: the finder's answer, once it comes.
                let wait = ms_frames(rate, FIND_WAIT_SUGGEST_MS);
                match self.find_for(k, ask, bars, end, wait) {
                    Some(d) => d,
                    None => continue,
                }
            } else {
                ends::decide(&self.notes, &ask, &|_| Some(bars))
            };
            self.loops[k].smart_wait = None;
            let by = d.moved_by(w.tap);
            let audible = |f: i64| f.unsigned_abs() as f64 * 1000.0 / f64::from(rate) >= OFFER_MS;
            let first = w.start + d.lead;
            let reads = reread((d.end - first) as usize, rate, bar, &w.reading).is_some();
            let moved = d.verdict == Verdict::Moved && reads;
            // What Snap would make: the end found, and a smart start (your
            // first hit came just after your press), or else the end as far
            // from the press, when the loop kept the audio for them.
            let end = if moved && audible(by) { by } else { 0 };
            let lead = d.lead as i64;
            let start = if audible(lead) { lead } else { 0 };
            let l = &mut self.loops[k];
            let fits = |f: Shift| f.is_none() || l.shifted(f).is_some();
            let without = Shift {
                start: 0,
                end: if end != 0 { end - lead } else { 0 },
            };
            let found = [Shift { start, end }, without]
                .into_iter()
                .find(|&f| fits(f))
                .unwrap_or(Shift::NONE);
            let outcome = match d.verdict {
                Verdict::Moved if !reads => SmartOutcome::Kept("too short a loop"),
                Verdict::Moved if end == 0 => SmartOutcome::Close(by),
                Verdict::Moved if found.end == 0 => {
                    SmartOutcome::Kept("that end is outside what the loop kept")
                }
                Verdict::Moved => SmartOutcome::Offered(found.end),
                v => SmartOutcome::Kept(v.why_by(self.by_ear)),
            };
            if !found.is_none() {
                l.found = Some(found);
                l.moved = Shift::NONE;
                l.smart_version = l.version;
            }
            self.smart_said(k, w.tap, outcome, found.start, d.best, d.second);
        }
    }

    /// Smart loop end by ear: loop `k`'s decision on `ask` from the
    /// finder once it has answered, asking it first (the host hands the
    /// ask over, [`Looper::find_job`]). None while it waits, until `wait`
    /// frames past the period that asked (`now`); then your tap stands
    /// ([`find::late`]). Counted either way.
    fn find_for(
        &mut self,
        k: usize,
        ask: ends::Ask,
        bars: u32,
        now: u64,
        wait: u64,
    ) -> Option<ends::Decision> {
        match self.find {
            Some(f) if f.k == k => {
                if let Some(d) = f.answer {
                    self.find = None;
                    self.stats.finds_answered += 1;
                    Some(d)
                } else if now >= f.due {
                    self.find = None;
                    self.stats.finds_late += 1;
                    Some(find::late(&f.ask))
                } else {
                    None
                }
            }
            _ => {
                self.find_gen += 1;
                self.find = Some(Finding {
                    k,
                    gen: self.find_gen,
                    ask,
                    bars,
                    due: now + wait,
                    sent: false,
                    answer: None,
                });
                self.stats.finds_asked += 1;
                None
            }
        }
    }

    /// The finder's ask is dropped once its take no longer waits for it
    /// (cleared, thrown away, its overdub started): a late answer to it
    /// is stale.
    fn forget_stale_find(&mut self) {
        let Some(f) = self.find else {
            return;
        };
        let l = &self.loops[f.k];
        let waits = l.smart_wait.is_some() || (l.free && l.phase == Phase::Recording);
        if !waits {
            self.find = None;
        }
    }

    /// Smart loop end by ear wants the finder: `job` gets the take's
    /// generation, the ask, the hits heard, and the first pass's audio
    /// round the opening hit and round the reach of the tap, and true,
    /// once per ask. The host hands it to the finder's thread
    /// ([`find::find`]) and its answer back ([`Looper::found`]).
    /// Allocation-free.
    pub fn find_job(&mut self, job: &mut FindJob) -> bool {
        let Some(f) = self.find.as_mut().filter(|f| !f.sent) else {
            return false;
        };
        f.sent = true;
        let f = *f;
        job.gen = f.gen;
        job.ask = f.ask;
        job.bars = f.bars;
        job.hits.clone_from(&self.hits);
        job.answer = None;
        job.line_up = 0.0;
        let opening = {
            let mut buf = [hits::Hit::NONE; ends::HEAD + ends::TAIL];
            let n = self.hits.sorted(&mut buf);
            buf[..n]
                .iter()
                .map(|h| h.frame)
                .find(|&x| x >= f.ask.start && x <= f.ask.start + f.ask.reach)
                .unwrap_or(f.ask.start)
        };
        job.open_from = FindJob::open_from_for(opening);
        job.open_len = self.kept_mono(f.k, job.open_from, f.ask.heard_to, &mut job.open);
        job.near_from = FindJob::near_from_for(f.ask.tap, f.ask.reach);
        job.near_len = self.kept_mono(f.k, job.near_from, f.ask.heard_to, &mut job.near);
        true
    }

    /// Whether the looper has asked the finder something the host hasn't
    /// taken yet ([`Looper::find_job`]): an ask that waits for the job to
    /// come back from the finder's thread.
    pub fn find_wanted(&self) -> bool {
        self.find.is_some_and(|f| !f.sent)
    }

    /// The finder's answer to `job`: kept for its take if the looper still
    /// waits for it (the next period uses it), else dropped as stale.
    pub fn found(&mut self, job: &FindJob) {
        match self.find.as_mut() {
            Some(f) if f.gen == job.gen && f.answer.is_none() => f.answer = job.answer,
            _ => self.stats.finds_stale += 1,
        }
    }

    /// Loop `k`'s first pass as recorded, from song frame `from`, in
    /// `out` as mono (the channels' mean), up to song frame `to`: how many
    /// frames from the first are heard (0 before what it kept; silence
    /// after). While it records or after. Allocation-free.
    fn kept_mono(&self, k: usize, from: u64, to: u64, out: &mut [f32]) -> usize {
        out.fill(0.0);
        let Some(l) = self.loops.get(k) else {
            return 0;
        };
        // Frame 0 of what it kept is KEEP_BEFORE before its first.
        let base = l.rec_from as i64 - KEEP_BEFORE as i64;
        let (v0, vn) = if l.phase == Phase::Recording {
            (0, KEEP_BEFORE + l.written)
        } else {
            l.slots[0].valid
        };
        let blocks = self.store.blocks_of(k, 0);
        let mut heard = 0;
        for (i, y) in out.iter_mut().enumerate() {
            let f = from + i as u64;
            let q = f as i64 - base;
            if f >= to || q >= (v0 + vn) as i64 {
                break;
            }
            if q < v0 as i64 {
                continue;
            }
            let q = q as usize;
            let x = self.store.frame(blocks, q / BLOCK, q % BLOCK);
            *y = 0.5 * (x[0] + x[1]);
            heard = i + 1;
        }
        heard
    }

    /// Smart loop end's word on loop `k`'s tap, for the host, counted.
    fn smart_said(
        &mut self,
        k: usize,
        tap: u64,
        outcome: SmartOutcome,
        start: i64,
        best: Option<Candidate>,
        second: Option<Candidate>,
    ) {
        match outcome {
            SmartOutcome::Moved(_) => self.stats.ends_moved += 1,
            SmartOutcome::Offered(_) => self.stats.ends_offered += 1,
            SmartOutcome::Kept(_) => self.stats.taps_kept += 1,
            SmartOutcome::Close(_) => {}
        }
        if start != 0 {
            self.stats.starts_found += 1;
        }
        self.smart_out = Some(SmartNote {
            n: k,
            outcome,
            start,
            tap,
            best,
            second,
        });
    }

    /// Each loop waiting for you to play that a note came to: where its
    /// first pass starts ([`Starts::Play`]). Song stopped: it asks for the
    /// song to start ([`Looper::take_start`]) and starts with it. Counting
    /// in: the note leads into bar 1. Playing: on the bar line nearest the
    /// note: in the first half of a bar, that bar's line (passed: the
    /// frames since are copied in from what was heard), else the next
    /// line, with the note and what followed it as a pickup.
    fn place_hits(&mut self, block: &Block) {
        for k in 0..self.loops.len() {
            let Some(p) = self.loops[k].hit.take() else {
                continue;
            };
            if self.loops[k].phase != Phase::Armed {
                continue;
            }
            let Some(span) = block.spans().first() else {
                continue;
            };
            match span.state {
                State::Playing | State::Recording => self.place_hit(k, p, block),
                State::CountIn => self.loops[k].pending = Some((p, true)),
                // Tempo from the loop: it records free from the note.
                State::Stopped if self.frees(k) => self.begin_free(k, p),
                State::Stopped => {
                    self.loops[k].pending = Some((p, false));
                    self.loops[k].want_start = true;
                }
            }
        }
    }

    /// [`Looper::place_hits`] for loop `k`, a note played on song frame
    /// `p`, the song playing.
    fn place_hit(&mut self, k: usize, p: u64, block: &Block) {
        let Some(span) = block
            .spans()
            .iter()
            .find(|s| matches!(s.state, State::Playing | State::Recording))
        else {
            return;
        };
        let bar = span.meter.bar_ticks();
        let now = block.start_frame;
        let (before, after) = self.heard.around(p).unwrap_or((None, None));
        // The line to start on, and whether the note is a pickup before it.
        let (line, pickup) = match before {
            Some((t, f)) => {
                let next = after.unwrap_or_else(|| (t + bar, span.frame_of((t + bar) as f64)));
                if p - f >= next.1.saturating_sub(f) / 2 {
                    (next, true)
                } else if next.1 <= now {
                    // Its bar is over already (a note heard that late is
                    // not a start): listen again.
                    return;
                } else {
                    ((t, f), false)
                }
            }
            None => match after {
                Some(next) => (next, true),
                // No line heard yet in this run: the next one.
                None => {
                    self.loops[k].pending = Some((p, true));
                    return;
                }
            },
        };
        if line.1 > now {
            // Still to come: the bar line starts it.
            self.loops[k].pending = Some((p, pickup));
            return;
        }
        // Passed: it starts there now, from what was heard.
        let half = span.frame_of((line.0 + bar) as f64).saturating_sub(line.1) / 2;
        let (rec_from, lead) = if pickup {
            let early = line.1.saturating_sub(p);
            if early > half {
                // Too long before the line to be its pickup: listen again.
                return;
            }
            (line.1 - early, early as usize)
        } else {
            (line.1, 0)
        };
        let l = &mut self.loops[k];
        l.begin_first(&mut self.store, k, span, line, rec_from, lead, now);
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
                // Waiting for the print at another speed: a second press
                // calls it off.
                if l.print_wait {
                    self.loops[s].print_wait = false;
                    continue;
                }
                let p = self.prints[s].now.as_ref();
                let live = p.filter(|p| l.print_live(p, self.prints[s].next.is_some()));
                let lent = l.adding || l.lend.is_some();
                if let Some(frames) = live.filter(|_| l.print_on > 0.0 && !lent).map(|p| p.frames) {
                    let l = &mut self.loops[s];
                    l.start_fit(&mut self.store, &mut self.stats, s, frames, self.now);
                    if l.fitting {
                        l.fit_overdub = true;
                        continue;
                    }
                }
                // At another speed the audio plays faster or slower than
                // it would record: the overdub waits for the print made at
                // that speed, and fits the loop to it first.
                let l = &self.loops[s];
                if l.rate != 0 && l.phase == Phase::Playing && !l.od_open {
                    if lent {
                        self.stats.refused += 1;
                    } else {
                        self.loops[s].print_wait = true;
                    }
                    continue;
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
                        | Command::Capture(_)
                        | Command::CaptureTo(..)
                ) | Queued::Layer(_, LayerCommand::Delete)
                    | Queued::Copy { .. }
            ) || (self.loops[self.selected].busy()
                && matches!(q, Queued::Layer(..)));
            let held = match q {
                // A copy waits for both of its loops.
                Queued::Copy { from, to, .. } => [from, to]
                    .iter()
                    .any(|&k| self.loops.get(k).is_some_and(|l| l.must_wait(q))),
                _ => self.loops[self.selected].must_wait(q),
            };
            let wait = hold_all
                || (deferrable && (waiting || held))
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
            Queued::Launch { n, on, at } => match self.loops.get_mut(n) {
                Some(l) => match at {
                    Some(t) => l.cue = Some((on, t)),
                    None => {
                        l.on = on;
                        l.cue = None;
                    }
                },
                None => self.stats.no_layer += 1,
            },
            Queued::Loop(Command::SetLatency(l)) => {
                for lp in &mut self.loops {
                    lp.latency = l;
                }
            }
            Queued::Loop(Command::NewInput) => self.heard.forget(),
            Queued::Loop(Command::Played(f)) => {
                for l in &mut self.loops {
                    if l.listening() && !l.by_ear {
                        l.hit = Some(f);
                    }
                }
            }
            Queued::Loop(Command::StartsByEar(on)) => {
                for l in &mut self.loops {
                    l.by_ear = on;
                }
            }
            Queued::Loop(Command::TempoFromLoop(on)) => self.free_tempo = on,
            Queued::Loop(Command::TempoMeant(on)) => self.tempo_meant = on,
            Queued::Loop(Command::SetSmartEnd(m)) => self.smart = m,
            Queued::Loop(Command::SmartByEar(on)) => {
                if on != self.by_ear {
                    self.by_ear = on;
                    self.hits.clear();
                    self.find = None;
                }
            }
            // A capture under way: again, Undo or Clear call it off.
            Queued::Loop(
                Command::Capture(_)
                | Command::CaptureTo(..)
                | Command::Undo
                | Command::Clear
                | Command::Forget,
            ) if self.loops[s].capturing() => {
                self.loops[s].call_off_capture();
            }
            Queued::Loop(Command::Capture(bars)) => self.start_capture(s, bars, None),
            Queued::Loop(Command::CaptureTo(end, bars)) => self.start_capture(s, bars, Some(end)),
            Queued::Copy { from, id, to, keep } => self.start_copy(from, id, to, keep),
            Queued::Loop(Command::Clear | Command::Forget) if self.loops[s].loading => {
                // Called off: the memory goes back (below).
                self.loops[s].loading = false;
            }
            // A copy in or out of the loop is called off, then it clears.
            Queued::Loop(Command::Clear | Command::Forget)
                if self.loops[s].adding || self.loops[s].lend.is_some() =>
            {
                self.call_off_copy(s);
                if let Some(t) = self.loops[s].lend.map(|l| l.to as usize) {
                    self.call_off_copy(t);
                }
                self.apply(q);
                return;
            }
            // Undo calls a copy in off.
            Queued::Loop(Command::Undo) if self.loops[s].adding => self.call_off_copy(s),
            Queued::Loop(c @ (Command::Undo | Command::Redo)) if self.tied(s, c).is_some() => {
                let o = self.tied(s, c).expect("tied above");
                self.loops[s].apply(&mut self.store, &mut self.stats, s, c);
                self.loops[o].apply(&mut self.store, &mut self.stats, o, c);
                // A layer that made a loop of its own took that loop with
                // it: the other loop's step can't come back either.
                if c == Command::Undo && self.loops[s].clearing {
                    self.loops[o].n_undone -= 1;
                } else if c == Command::Undo && self.loops[o].clearing {
                    self.loops[s].n_undone -= 1;
                }
                self.loops[o].reclaim(&mut self.store, o);
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
                        self.loops[s].start_fit(
                            &mut self.store,
                            &mut self.stats,
                            s,
                            frames,
                            self.now,
                        )
                    }
                    _ => self.stats.refused += 1,
                }
            }
            Queued::Loop(c) => {
                let was = self.loops[s].recording();
                let empty = self.loops[s].phase == Phase::Empty;
                self.loops[s].apply(&mut self.store, &mut self.stats, s, c);
                if !was && self.loops[s].recording() {
                    self.others_stop_recording(s);
                }
                // Tempo from the loop, on the bar: armed with the song
                // stopped, it records free from now (there is no bar line
                // to wait for).
                let l = &self.loops[s];
                if c == Command::Record
                    && empty
                    && l.phase == Phase::Armed
                    && l.starts == Starts::Bar
                    && self.stopped
                    && self.frees(s)
                {
                    self.begin_free(s, self.now);
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

    /// Capture into loop `s` ([`Command::Capture`]): it waits for its bar
    /// line (the last one heard, or, pressed in a bar's last beat, the one
    /// coming); [`Looper::tend_captures`] takes it from there.
    /// Capture into loop `s`: `bars` ending on the last bar line (or the
    /// one coming, in a bar's last beat), or on the one heard on tick
    /// `to`.
    fn start_capture(&mut self, s: usize, bars: u32, to: Option<Tick>) {
        let (end, wait) = match to {
            None => {
                let wait = self.heard.in_last_beat();
                let lines = self.heard.lines();
                match (lines, wait) {
                    (_, true) => (Some(lines), true),
                    (0, false) => (None, false),
                    (n, false) => (Some(n - 1), false),
                }
            }
            Some(t) => (self.heard.line_at(t), false),
        };
        let l = &mut self.loops[s];
        let Some(end) = end.filter(|_| {
            l.phase == Phase::Empty
                && !l.clearing
                && !l.loading
                && !l.adding
                && self.loads[s].is_none()
                && bars > 0
        }) else {
            self.stats.refused += 1;
            return;
        };
        l.capture = Some(Capture {
            bars,
            end,
            wait,
            take: None,
        });
        // A new loop plays once it is in.
        l.on = true;
        l.cue = None;
    }

    /// Each capture waiting: once its bar line is heard, the take; once
    /// the take's last frame is heard (`latency` after the bar line), it
    /// copies in, as a load does, from what was heard.
    fn tend_captures(&mut self) {
        for k in 0..self.loops.len() {
            if let Some(c) = self.loops[k].capture {
                self.loops[k].capture = self.tend_capture(k, c);
            }
        }
    }

    /// [`Looper::tend_captures`] for loop `k`: the capture as it is now,
    /// or None once it copies in or is refused.
    fn tend_capture(&mut self, k: usize, mut c: Capture) -> Option<Capture> {
        let h = &self.heard;
        if c.wait {
            if h.lines() > c.end {
                c.wait = false;
            } else if h.playing() {
                return Some(c);
            } else if c.end > 0 {
                // The song stopped before that bar line: the bars before.
                c.wait = false;
                c.end -= 1;
            } else {
                self.stats.refused += 1;
                return None;
            }
        }
        let Some(t) = c
            .take
            .or_else(|| h.take(c.end, c.bars, self.loops[k].latency))
        else {
            self.stats.refused += 1;
            return None;
        };
        c.take = Some(t);
        if h.at() < t.start + t.frames as u64 {
            // Its last frames are still to be heard.
            return Some(c);
        }
        let need = blocks_for(t.frames);
        if self.store.available() < need {
            self.stats.refused += 1;
            return None;
        }
        // It takes the place of the loop kept for Undo.
        self.store.drop_kept(k);
        let l = &mut self.loops[k];
        l.reclaim(&mut self.store, k);
        if !self.store.grow(k, 0, need) {
            self.stats.refused += 1;
            l.reclaim(&mut self.store, k);
            return None;
        }
        l.slots[0] = Slot {
            valid: (0, t.frames),
            ..Slot::NEW
        };
        l.loading = true;
        l.ld_heard = Some(t.start);
        l.ld_layers = 1;
        l.ld_frames = t.frames;
        l.ld_ticks = t.ticks;
        l.ld_origin = t.origin;
        l.ld_done = 0;
        // From the frame its last frame was heard on, when that is in
        // this period, so the period size changes nothing.
        l.ld_from = self.now.max(t.start + t.frames as u64);
        l.ld_at = l.ld_from + t.frames.div_ceil(LOAD_SPEED) as u64;
        None
    }

    /// Start a layer's copy between loops ([`Looper::copy_layer`]).
    fn start_copy(&mut self, from: usize, id: u8, to: usize, keep: bool) {
        let slot = id as usize;
        let listed = |l: &Loop| {
            slot < MAX_LAYERS && l.order[..l.in_order].contains(&id) && !l.slots[slot].leaving
        };
        if from >= MAX_LOOPS || to >= MAX_LOOPS || !listed(&self.loops[from]) {
            self.stats.no_layer += 1;
            return;
        }
        // Nothing changing the layers, nothing fading.
        let free = |l: &Loop| l.savable() && !l.busy() && !l.fading() && l.xf == 0;
        let (src, dst) = (&self.loops[from], &self.loops[to]);
        let empty = dst.phase == Phase::Empty
            && !dst.clearing
            && !dst.loading
            && !dst.adding
            && self.loads[to].is_none();
        let onto = free(dst) && dst.len_ticks == src.len_ticks && dst.in_order < MAX_LAYERS - 1;
        if from == to || !free(src) || !(empty || onto) {
            self.stats.refused += 1;
            return;
        }
        let s = src.slots[slot];
        let (len, ticks, origin, layers) = (src.len, src.len_ticks, src.origin, src.layers());
        let (speed, rate) = (src.speed, src.rate);
        let l = &mut self.loops[to];
        let k = if empty {
            // The layer's loop takes the place of one kept for Undo.
            self.store.drop_kept(to);
            l.reclaim(&mut self.store, to);
            let need = blocks_for(len);
            (self.store.available() >= need && self.store.grow(to, 0, need)).then_some(0)
        } else {
            l.free_slot(&mut self.store, to)
        };
        let Some(k) = k else {
            l.reclaim(&mut self.store, to);
            self.stats.refused += 1;
            return;
        };
        let frames = if empty { len } else { l.len };
        if empty {
            // It takes the layer's loop's speed with its length.
            l.speed = speed;
            l.rate = rate;
        }
        let mut new = Slot {
            valid: (0, frames),
            level: s.level,
            pan: s.pan,
            mute: s.mute,
            age: s.age,
            // Into a loop that plays, it fades in; an empty loop fades in
            // as a whole.
            presence: if empty { 1.0 } else { 0.0 },
            ..Slot::NEW
        };
        (new.gl, new.gr) = new.target(!empty && l.any_solo());
        l.slots[k] = new;
        // In time with the song: this loop's frame 0 is where `from` is
        // at this loop's start.
        l.ad_shift = if empty {
            0.0
        } else {
            rem_ticks((l.origin - origin) as f64, ticks as f64) * len as f64 / ticks as f64
        };
        l.ad_rate = len as f64 / frames as f64;
        l.adding = true;
        l.ad_from = from;
        l.ad_src = slot;
        l.ad_slot = k;
        l.ad_frames = frames;
        l.ad_done = 0;
        l.ad_start = self.now;
        l.ad_at = self.now + frames.div_ceil(COPY_SPEED) as u64;
        l.ad_move = !keep && layers > 1;
        l.ad_ticks = ticks;
        l.ad_origin = origin;
        let lend = Lend {
            slot: id,
            to: to as u8,
            to_slot: k as u8,
            at: l.ad_at,
            drop: l.ad_move,
        };
        self.loops[from].lend = Some(lend);
    }

    /// Copy loop `k`'s layer in as far as the period ending before frame
    /// `end` is due: [`COPY_SPEED`] frames for each frame since it
    /// started. It reads the other loop's layer as it plays, between its
    /// frames when the lengths differ.
    fn render_copy(&mut self, k: usize, end: u64) {
        let l = &self.loops[k];
        let (from, src_slot, slot) = (l.ad_from, l.ad_src, l.ad_slot);
        let (shift, rate, frames) = (l.ad_shift, l.ad_rate, l.ad_frames);
        let due = (end - l.ad_start).saturating_mul(COPY_SPEED as u64);
        let target = frames.min(due.min(usize::MAX as u64) as usize);
        let mut done = l.ad_done;
        let src = &self.loops[from];
        let (valid, view, seam, len) = (
            src.slots[src_slot].valid,
            src.views[src_slot],
            src.seam,
            src.len,
        );
        if len == 0 {
            return;
        }
        while done < target {
            let n = RUN.min(target - done);
            let mut buf = [[0.0f32; 2]; RUN];
            let blocks = self.store.blocks_of(from, src_slot);
            for (j, y) in buf[..n].iter_mut().enumerate() {
                let x = ((done + j) as f64 * rate + shift).rem_euclid(len as f64);
                let i = (x as usize).min(len - 1);
                let frac = (x - i as f64) as f32;
                let a = layer_frame(&self.store, blocks, valid, &view, seam, i);
                *y = if frac == 0.0 {
                    a
                } else {
                    let b = layer_frame(&self.store, blocks, valid, &view, seam, (i + 1) % len);
                    [a[0] + (b[0] - a[0]) * frac, a[1] + (b[1] - a[1]) * frac]
                };
            }
            for (j, y) in buf[..n].iter().enumerate() {
                self.store.set(k, slot, done + j, *y);
            }
            done += n;
        }
        self.loops[k].ad_done = done;
    }

    /// Call off the layer copying into loop `t`, if any: its memory goes
    /// back, and the layer stays in its own loop.
    fn call_off_copy(&mut self, t: usize) {
        let l = &mut self.loops[t];
        if !l.adding {
            return;
        }
        l.adding = false;
        let from = l.ad_from;
        if self.loops[from].lend.is_some_and(|x| x.to as usize == t) {
            self.loops[from].lend = None;
        }
        self.loops[t].reclaim(&mut self.store, t);
    }

    /// The loop whose step Undo (or Redo) takes back (or brings back)
    /// with loop `s`'s: the two sides of a layer moved between them, each
    /// the last step of its loop, with both loops playing and free.
    fn tied(&self, s: usize, c: Command) -> Option<usize> {
        let top = |l: &Loop| -> Option<Act> {
            match c {
                Command::Undo => l.done[..l.n_done].last().copied(),
                _ => l.undone[..l.n_undone].last().copied(),
            }
        };
        let ready = |l: &Loop| {
            l.phase == Phase::Playing
                && !l.od_open
                && !l.busy()
                && !l.must_wait(Queued::Loop(Command::Undo))
        };
        let a = top(&self.loops[s])?;
        let link = a.link.filter(|k| k.tied)?;
        let o = link.lp as usize;
        let b = top(self.loops.get(o)?)?;
        let back = b
            .link
            .filter(|k| k.tied && k.lp as usize == s && k.slot == a.slot);
        (o != s
            && back.is_some()
            && b.slot == link.slot
            && ready(&self.loops[s])
            && ready(&self.loops[o]))
        .then_some(o)
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
                Phase::Armed => {
                    l.phase = Phase::Empty;
                    l.unhear();
                }
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
            Kind::Edit | Kind::Snap | Kind::Bars => false,
            Kind::Overdub | Kind::Delete | Kind::Added => a.slot as usize == slot,
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
                || store.kept_holds(me, k)
                || (k == 0 && self.first_in_progress())
                || (self.od_open && self.od_layer == k)
                || (self.merging && self.mg_slot == k)
                || (self.fitting && self.fit_slot == k)
                || (self.loading && k < self.ld_layers)
                || (self.adding && self.ad_slot == k);
            if !in_use && store.held[me][k] > 0 {
                store.release(me, k);
            }
        }
    }

    fn must_wait(&self, q: Queued) -> bool {
        match q {
            // A capture: again, or Undo, calls it off; the rest waits.
            Queued::Loop(Command::Capture(_) | Command::CaptureTo(..) | Command::Undo)
                if self.capturing() =>
            {
                false
            }
            _ if self.capture.is_some() => true,
            // Everything waits for a bounce (Clear stops it) and a load
            // (Clear calls it off).
            _ if self.bouncing || self.loading => true,
            // A layer copying in: Undo calls it off; the rest waits, as it
            // does while one of the loop's layers is copied out.
            Queued::Loop(Command::Undo) if self.adding => false,
            _ if self.adding || self.lend.is_some() => true,
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

    /// Merging, fitting, being bounced out or copying a layer in or out:
    /// the layers must not change.
    fn busy(&self) -> bool {
        self.snap.is_some()
            || self.merging
            || self.bouncing
            || self.fitting
            || self.swapping
            || self.adding
            || self.lend.is_some()
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
    /// going, then, once silent, waits at the song position's place (where
    /// the transport will start again), so the playhead stops with the
    /// transport. A jump starts a crossfade.
    fn follow(&mut self, tick: f64, playing: bool, first: bool) {
        if self.len == 0 {
            return;
        }
        let len = self.len as f64;
        let was = self.pos;
        let was_p = self.ppos;
        if self.p_frames > 0 {
            self.follow_print(tick, playing);
        }
        if self.jump > 0 {
            self.jump -= 1;
            self.jump_pos = (self.jump_pos + 1.0) % len;
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
        if !playing && self.gain > 0.0 {
            self.pos = (was + 1.0) % len;
            return;
        }
        let rel = rem_ticks(tick - self.origin as f64, self.len_ticks as f64);
        self.pos = (rel / self.len_ticks as f64 * len).min(len - 1e-6);
        if !playing {
            return;
        }
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
                    self.pjump_pos = (was_p + 1.0) % self.p_frames as f64;
                }
            }
        }
    }

    /// The print's place, as `follow` moves the loop's (and its jump's).
    #[cold]
    fn follow_print(&mut self, tick: f64, playing: bool) {
        let pf = self.p_frames as f64;
        if self.jump > 0 {
            self.pjump_pos = (self.pjump_pos + 1.0) % pf;
        }
        // As the loop's: on while it fades out, then at the song position.
        self.ppos = if playing || self.gain == 0.0 {
            let rel = rem_ticks(tick - self.p_origin as f64, self.p_ticks as f64);
            (rel / self.p_ticks as f64 * pf).min(pf - 1e-6)
        } else {
            (self.ppos + 1.0) % pf
        };
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
                    link: None,
                    shift: Shift::NONE,
                    bars: 0,
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
                | Command::Solo(_)
                | Command::SetOverdubs(_)
                | Command::SetBars(_)
                | Command::SetAfter(_)
                | Command::SetStarts(_)
                | Command::Played(_)
                | Command::StartsByEar(_)
                | Command::TempoFromLoop(_)
                | Command::TempoMeant(_)
                | Command::SetSmartEnd(_)
                | Command::SmartByEar(_)
                | Command::SetFadeOld(_)
                | Command::SetLatency(_)
                | Command::SetLevel(_)
                | Command::Merge
                | Command::Fit
                | Command::NewInput
        ) {
            self.touch();
        }
        match c {
            Command::Record => match self.phase {
                Phase::Empty => {
                    self.phase = Phase::Armed;
                    self.ring = [0.0; 2 * FADE];
                    self.unhear();
                    self.just_armed = true;
                    self.tap_lead = None;
                    // A new loop plays once recorded.
                    self.on = true;
                    self.cue = None;
                }
                Phase::Armed => {
                    self.phase = Phase::Empty;
                    self.resample = false;
                    self.unhear();
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
                Phase::Overdubbing | Phase::Playing if self.od_open => self.undo_open_overdub(),
                Phase::Playing if self.merging => self.merging = false,
                Phase::Playing if self.fitting => {
                    self.fitting = false;
                    self.fit_overdub = false;
                    self.print_wait = false;
                }
                // An overdub waiting for its print is called off.
                Phase::Playing if self.print_wait => self.print_wait = false,
                Phase::Playing => self.undo(),
                Phase::Empty if store.has_kept(me) && !self.loading && !self.adding => {
                    self.unclear(store, me)
                }
                _ => {}
            },
            Command::Redo => {
                if self.phase == Phase::Playing && self.n_undone > 0 && !self.od_open {
                    self.redo();
                }
            }
            Command::Clear | Command::Forget => {
                let keep = c == Command::Clear;
                if !keep {
                    store.drop_kept(me);
                }
                match self.phase {
                    Phase::Empty => {}
                    Phase::Armed | Phase::Recording => self.abort(stats),
                    Phase::Playing | Phase::Overdubbing => {
                        // An overdub under way is undone first, so UNDO
                        // brings the loop back as it was before it (a key
                        // held to clear started one on its press), and
                        // Redo then brings what it recorded.
                        if self.od_open {
                            self.undo_open_overdub();
                        }
                        self.clearing = true;
                        self.keep_clear = keep;
                    }
                }
            }
            Command::Mute(m) => self.muted = m,
            Command::Solo(o) => self.solo = o,
            Command::SetOverdubs(o) => self.overdubs = o,
            Command::SetBars(b) => self.bars = b.filter(|&b| b > 0),
            Command::SetAfter(a) => self.after = a,
            Command::SetStarts(w) => {
                self.starts = w;
                if w == Starts::Bar {
                    self.unhear();
                }
            }
            // The looper hands these to every loop.
            Command::Played(_)
            | Command::StartsByEar(_)
            | Command::TempoFromLoop(_)
            | Command::TempoMeant(_)
            | Command::SetSmartEnd(_)
            | Command::SmartByEar(_) => {}
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
            // The looper starts these, with the time, the print and what
            // it heard.
            Command::Merge
            | Command::Fit
            | Command::Capture(_)
            | Command::CaptureTo(..)
            | Command::NewInput => {}
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
            region: self.region,
            speed: self.speed,
            rate: self.rate,
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
        self.region = s.region;
        self.speed = s.speed;
        self.rate = s.rate;
        self.views = s.views;
    }

    /// Make an edit, keeping the shape before it for Undo.
    fn edit(&mut self, stats: &mut Stats, e: Edit) {
        self.make_edit(stats, e, Kind::Edit);
    }

    /// Edit `e`, as a step of `kind` Undo takes back; false (counted) if it
    /// can't be made.
    fn make_edit(&mut self, stats: &mut Stats, e: Edit, kind: Kind) -> bool {
        let new = self.editable().then(|| self.edited(e)).flatten();
        self.take_shape(
            stats,
            new,
            Act {
                kind,
                edit: Some(e),
                ..NO_ACT
            },
        )
    }

    /// A snap's move ([`Kind::Snap`]), as a step Undo takes back; false
    /// (counted) if it can't be made.
    fn make_shift(&mut self, stats: &mut Stats, by: Shift) -> bool {
        let new = self.editable().then(|| self.shifted(by)).flatten();
        self.take_shape(
            stats,
            new,
            Act {
                kind: Kind::Snap,
                shift: by,
                ..NO_ACT
            },
        )
    }

    /// Read as `bars` bars, `ticks` song ticks in all ([`Kind::Bars`]), as
    /// a step Undo takes back: the loop's length, layers and sound stay.
    /// False (counted) if it can't be made.
    fn make_bars(&mut self, stats: &mut Stats, bars: u32, ticks: Tick) -> bool {
        let new = (self.editable() && ticks > 0).then(|| Shape {
            len_ticks: ticks,
            ..self.shape()
        });
        self.take_shape(
            stats,
            new,
            Act {
                kind: Kind::Bars,
                bars,
                ..NO_ACT
            },
        )
    }

    /// Playing, nothing recording: an edit can be made.
    fn editable(&self) -> bool {
        self.phase == Phase::Playing
            && !self.od_open
            && !self.first_open
            && self.len > 0
            && self.len_ticks > 0
    }

    /// Take shape `new` (None: counted, false), crossfading from the one
    /// now, which is kept for the Undo of `act`.
    fn take_shape(&mut self, stats: &mut Stats, new: Option<Shape>, act: Act) -> bool {
        let Some(new) = new else {
            stats.refused += 1;
            return false;
        };
        self.n_undone = 0;
        let Some(i) = self.free_shape() else {
            stats.refused += 1;
            return false;
        };
        self.shapes[i] = self.shape();
        self.set_shape(&new);
        self.push_done(Act {
            slot: i as u8,
            index: 0,
            ..act
        });
        if act.fades() {
            self.crossfade_from(i);
        }
        true
    }

    /// The shape a snap's move makes: the first pass's region in the audio
    /// it kept, its start moved and its length changed by `by` (an
    /// [`Edit::Move`] and an [`Edit::Resize`] at once). None if it can't
    /// be made.
    fn shifted(&self, by: Shift) -> Option<Shape> {
        let (start, src) = self.bounds_now()?;
        let mut s = self.shape();
        let shortest = self.shortest();
        let to = (
            (start as i64).checked_add(by.start)?,
            (src as i64).checked_add(by.start)?,
            (s.len as i64).checked_add(by.longer())?,
        );
        self.rebound(&mut s, (start, src), to, shortest)?;
        Some(s)
    }

    /// A 1/16 note of the loop, in frames (at least 1).
    fn shortest(&self) -> i64 {
        let (len, ticks) = (self.len as i128, i128::from(self.len_ticks.max(1)));
        ((i128::from(SIXTEENTH) * len + ticks / 2) / ticks).max(1) as i64
    }

    /// Shape `s`'s first pass from region `from` (its first frame and the
    /// one on bar 1) to `to` (first frame, bar 1, length): within what was
    /// kept, at least `shortest` frames, and moved. None otherwise.
    fn rebound(
        &self,
        s: &mut Shape,
        from: (usize, usize),
        to: (i64, i64, i64),
        shortest: i64,
    ) -> Option<()> {
        let (a, b, n) = to;
        let kept = self.slots[0].valid.1 as i64;
        if a < 0
            || n < shortest
            || a + n > kept
            || to == (from.0 as i64, from.1 as i64, s.len as i64)
        {
            return None;
        }
        let size = s.views[0].size();
        s.views[0] = View::region(size, a as usize, n as usize, b as usize)?;
        s.region = Some((a as usize, b as usize));
        s.len = n as usize;
        Some(())
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
        if matches!(
            e,
            Edit::Half | Edit::Double | Edit::Reverse | Edit::Nudge(_) | Edit::Trim { .. }
        ) {
            // The first pass's layer plays more than its region now.
            s.region = None;
        }
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
            Edit::Move(d) | Edit::Resize(d) | Edit::Turn(d) => {
                let (start, src) = self.bounds_now()?;
                let (a, b, n) = (start as i64, src as i64, len as i64);
                let to = match e {
                    Edit::Move(_) => (a.checked_add(d)?, b.checked_add(d)?, n),
                    Edit::Resize(_) => (a, b, n.checked_add(d)?),
                    _ => (a, a + (b - a + d.rem_euclid(n)) % n, n),
                };
                // Within what was kept, at least a 1/16 note, and moved.
                let shortest = frames(SIXTEENTH).max(1) as i64;
                self.rebound(&mut s, (start, src), to, shortest)?;
            }
            Edit::SeamFade(f) => s.seam = (f as usize).min(MAX_SEAM),
            Edit::Speed(to) => {
                let to = to.clamp(-1, 1);
                let by = to - s.speed;
                if by == 0 || (s.rate + by).abs() > 2 {
                    return None;
                }
                // Faster is fewer ticks for the same audio, slower more.
                let m: Tick = 1 << by.unsigned_abs();
                if by > 0 {
                    if ticks % m != 0 || ticks / m < SIXTEENTH {
                        return None;
                    }
                    s.len_ticks = ticks / m;
                } else {
                    s.len_ticks = ticks.checked_mul(m).filter(|t| *t <= Tick::MAX / 4)?;
                }
                s.speed = to;
                s.rate += by;
            }
        }
        // Every layer plays the loop's length (a check that costs nothing).
        if (0..MAX_LAYERS).any(|k| live[k] && s.views[k].len() != s.len) {
            return None;
        }
        Some(s)
    }

    /// The first pass's loop in the audio it kept (its first frame, and
    /// the one on bar 1), while it is the loop's only layer and plays that
    /// region alone, unedited (but for its seam fade and speed): what
    /// [`Edit::Move`], [`Edit::Resize`] and [`Edit::Turn`] change.
    fn bounds_now(&self) -> Option<(usize, usize)> {
        let (start, src) = self.region?;
        let v = &self.views[0];
        let alone = self.in_order == 1
            && self.order[0] == 0
            && self.slots[0].valid.0 == 0
            && (1..MAX_LAYERS).all(|k| !self.live(k));
        (alone && View::region(v.size(), start, self.len, src).as_ref() == Some(v))
            .then_some((start, src))
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
    /// edit), crossfading from it if `fade`.
    fn swap_shape(&mut self, i: usize, fade: bool) {
        let now = self.shape();
        let kept = self.shapes[i];
        self.set_shape(&kept);
        self.shapes[i] = now;
        if fade {
            self.crossfade_from(i);
        }
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
    fn start_fit(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        frames: usize,
        now: u64,
    ) {
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

    /// Playing with nothing changing its layers, so that it can be saved.
    fn savable(&self) -> bool {
        self.phase == Phase::Playing
            && self.len > 0
            && !self.od_open
            && !self.first_open
            && !self.merging
            && !self.fitting
            && !self.swapping
            && !self.clearing
            && !self.loading
            && !self.adding
    }

    /// Copy a saved loop's layers in as far as the period ending before
    /// frame `end` is due: [`LOAD_SPEED`] frames for each frame since the
    /// load started.
    fn render_load(&mut self, store: &mut Store, me: usize, f: &LoopFile, end: u64) {
        let total = self.ld_layers * self.ld_frames;
        let due = (end - self.ld_from).saturating_mul(LOAD_SPEED as u64);
        let target = total.min(due.min(usize::MAX as u64) as usize);
        for i in self.ld_done..target {
            let (k, q) = (i / self.ld_frames, i % self.ld_frames);
            let a = &f.layers[k].audio;
            store.set(me, k, q, [a[2 * q], a[2 * q + 1]]);
        }
        self.ld_done = self.ld_done.max(target);
    }

    /// A capture copying in from what was heard, as far as the period
    /// ending before frame `end` is due. Its loop point fades into what
    /// was heard just before its first frame, as a first pass's does. Were
    /// what it reads gone by (the room [`Heard::take`] leaves keeps it),
    /// the capture would be thrown away.
    fn render_heard(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        heard: &Heard,
        end: u64,
    ) {
        let Some(from) = self.ld_heard else {
            return;
        };
        let total = self.ld_frames;
        let due = (end - self.ld_from).saturating_mul(LOAD_SPEED as u64);
        let target = total.min(due.min(usize::MAX as u64) as usize);
        if self.ld_done < target && !heard.holds(from + self.ld_done as u64) {
            stats.aborted += 1;
            self.loading = false;
            self.ld_heard = None;
            self.reclaim(store, me);
            return;
        }
        let fade = FADE.min(total / 2);
        for q in self.ld_done..target {
            let mut x = heard.frame(from + q as u64);
            if q + fade >= total {
                let k = q + fade - total;
                let w = (k as f32 + 0.5) / fade as f32;
                let p = (from + k as u64)
                    .checked_sub(fade as u64)
                    .map_or([0.0; 2], |f| heard.frame(f));
                x = [x[0] * (1.0 - w) + p[0] * w, x[1] * (1.0 - w) + p[1] * w];
            }
            store.set(me, 0, q, x);
        }
        self.ld_done = self.ld_done.max(target);
    }

    /// The saved loop (or a capture) is all in: it plays from now on
    /// (fading in), in time with the song.
    fn finish_load(&mut self) {
        self.loading = false;
        self.ld_heard = None;
        self.len = self.ld_frames;
        self.cut = self.len;
        self.len_ticks = self.ld_ticks;
        self.origin = self.ld_origin;
        self.in_order = self.ld_layers;
        for k in 0..self.ld_layers {
            self.order[k] = k as u8;
            self.views[k] = View::whole(self.len);
        }
        self.seam = FADE;
        self.n_done = 0;
        self.n_undone = 0;
        self.gain = 0.0;
        self.pos = 0.0;
        self.jump = 0;
        self.xf = 0;
        self.mixing = true;
        self.phase = Phase::Playing;
        self.touch();
    }

    /// The layer copied in joins the loop, fading in, as a step Undo
    /// takes back. Into an empty loop it is the loop: it plays from now on
    /// (fading in), in time with the song.
    fn finish_add(&mut self) {
        self.adding = false;
        if self.phase == Phase::Empty {
            self.len = self.ad_frames;
            self.len_ticks = self.ad_ticks;
            self.origin = self.ad_origin;
            self.in_order = 0;
            self.seam = FADE;
            self.n_done = 0;
            self.n_undone = 0;
            self.gain = 0.0;
            self.pos = 0.0;
            self.jump = 0;
            self.xf = 0;
            self.phase = Phase::Playing;
        }
        let k = self.ad_slot;
        self.views[k] = View::whole(self.len);
        self.order[self.in_order] = k as u8;
        self.push_done(Act {
            kind: Kind::Added,
            slot: k as u8,
            index: self.in_order as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
            link: Some(Link {
                lp: self.ad_from as u8,
                slot: self.ad_src as u8,
                tied: self.ad_move,
            }),
            shift: Shift::NONE,
            bars: 0,
        });
        self.in_order += 1;
        self.mixing = true;
        self.touch();
    }

    /// The copy of one of the loop's layers has joined the other loop: a
    /// move takes the layer out of this one, as a delete Undo takes back
    /// (with the other loop's step while each is its loop's last).
    fn end_lend(&mut self) {
        let Some(l) = self.lend.take() else {
            return;
        };
        let at = self.order[..self.in_order]
            .iter()
            .position(|&k| k == l.slot)
            .filter(|_| !self.slots[l.slot as usize].leaving);
        let Some(at) = at.filter(|_| l.drop && self.layers() > 1) else {
            return;
        };
        self.n_undone = 0;
        self.push_done(Act {
            kind: Kind::Delete,
            slot: l.slot,
            index: at as u8,
            edit: None,
            ages: [1.0; MAX_LAYERS],
            link: Some(Link {
                lp: l.to,
                slot: l.to_slot,
                tied: true,
            }),
            shift: Shift::NONE,
            bars: 0,
        });
        self.slots[l.slot as usize].leaving = true;
        self.mixing = true;
        self.touch();
    }

    /// An overdub waiting for the print at another speed
    /// ([`Loop::print_wait`]): once the print plays, the fit starts and
    /// the overdub with it; back at the speed it was recorded at (no print
    /// needed), the overdub starts at once.
    fn tend_print_wait(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        pr: &Prints,
        now: u64,
    ) {
        if self.phase != Phase::Playing || self.od_open || self.fitting {
            self.print_wait = false;
            return;
        }
        if self.adding || self.lend.is_some() || self.busy() {
            return;
        }
        let live = pr
            .now
            .as_ref()
            .filter(|p| self.print_on > 0.0 && self.print_live(p, pr.next.is_some()));
        if let Some(frames) = live.map(|p| p.frames) {
            self.print_wait = false;
            self.start_fit(store, stats, me, frames, now);
            if self.fitting {
                self.fit_overdub = true;
            }
        } else if self.rate == 0 && self.print_on == 0.0 {
            self.print_wait = false;
            self.open_overdub(store, stats, me, FADE);
        }
    }

    fn cancel_fit(&mut self, stats: &mut Stats) {
        self.fitting = false;
        self.fit_overdub = false;
        self.print_wait = false;
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
        // The print was made at the loop's speed: its audio plays as it is.
        self.rate = 0;
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
            link: None,
            shift: Shift::NONE,
            bars: 0,
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
        // One layer playing its audio as it is: all of it, or a first
        // pass's region from its start.
        let plain = self.views[first].is_whole()
            || (first == 0 && self.bounds_now().is_some_and(|(start, src)| start == src));
        let nothing = self.in_order == 1 && plain && g[first] == [1.0, 1.0];
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
        self.render_layers_at(store, me, from, acc, &self.mg_g, self.replacing());
    }

    /// [`Loop::render_layers`] at the gains `gains` (each layer slot's,
    /// left and right); `replacing` hears Replace layers over the layers
    /// under them, as they play.
    fn render_layers_at(
        &self,
        store: &Store,
        me: usize,
        from: usize,
        acc: &mut [[f32; 2]],
        gains: &[[f32; 2]; MAX_LAYERS],
        replacing: bool,
    ) {
        if replacing {
            // Frame by frame, each layer at its share under the Replace
            // layers after it, as they are heard.
            let any_solo = self.any_solo();
            let strength = |s: &Slot| if s.heard(any_solo) { 1.0 } else { 0.0 };
            for (j, a) in acc.iter_mut().enumerate() {
                let p = from + j;
                let d = self.ducks(&self.views, self.len, p, strength);
                for &k in &self.order[..self.in_order] {
                    let k = k as usize;
                    let g = gains[k];
                    let v = &self.views[k];
                    if g == [0.0, 0.0] || v.size() == 0 || d[k] == 0.0 {
                        continue;
                    }
                    let blocks = store.blocks_of(me, k);
                    let x = layer_frame(store, blocks, self.slots[k].valid, v, self.seam, p);
                    a[0] += g[0] * x[0] * d[k];
                    a[1] += g[1] * x[1] * d[k];
                }
            }
            return;
        }
        for &k in &self.order[..self.in_order] {
            let k = k as usize;
            let g = gains[k];
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
    /// The gains Listen plays at: the loop as heard, or one layer alone
    /// (`id`, a layer slot) at its level and pan, even when muted.
    fn listen_gains(&self, layer: Option<u8>) -> [[f32; 2]; MAX_LAYERS] {
        let Some(id) = layer else {
            return self.heard_gains();
        };
        let mut g = [[0.0f32; 2]; MAX_LAYERS];
        let k = id as usize;
        if self.order[..self.in_order].contains(&id) {
            let s = &self.slots[k];
            let level = s.level * s.age;
            g[k] = [
                level * (1.0 - s.pan).min(1.0),
                level * (1.0 + s.pan).min(1.0),
            ];
        }
        g
    }

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
            link: None,
            shift: Shift::NONE,
            bars: 0,
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
            link: None,
            shift: Shift::NONE,
            bars: 0,
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
            // The last layer: it made the loop, which goes with it.
            Kind::Added if self.layers() <= 1 => {
                self.od_open = false;
                self.clearing = true;
                self.keep_clear = false;
            }
            Kind::Added => self.slots[a.slot as usize].leaving = true,
            Kind::Edit | Kind::Snap | Kind::Bars => self.swap_shape(a.slot as usize, a.fades()),
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
            Kind::Overdub | Kind::Added => self.restore(a),
            Kind::Edit | Kind::Snap | Kind::Bars => self.swap_shape(a.slot as usize, a.fades()),
            Kind::Merge => self.remerge(a),
            Kind::Fit => self.start_swap(a.index as usize),
        }
    }

    /// Fade old, once: every layer but the one being recorded turns down
    /// (gliding over a few ms, as a level change does). Below -80 dB it is
    /// silent.
    fn fade_older(&mut self) {
        self.touch();
        self.od_faded = true;
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
        self.print_wait = false;
        self.swapping = false;
        self.phase = Phase::Empty;
        self.len = 0;
        self.len_ticks = 0;
        self.speed = 0;
        self.rate = 0;
        self.in_order = 0;
        self.n_done = 0;
        self.n_undone = 0;
        self.end_tick = None;
        self.ending = false;
        self.cut = 0;
        self.then_overdub = false;
        self.first_open = false;
        self.tail = false;
        self.before = None;
        self.region = None;
        self.captured = (0, 0);
        self.written = 0;
        self.od_open = false;
        self.clearing = false;
        self.keep_clear = false;
        // A cleared loop lets go of its solo, so the others play.
        self.solo = false;
        self.gain = 0.0;
        self.pos = 0.0;
        self.jump = 0;
        self.xf = 0;
        self.seam = FADE;
        self.merging = false;
        self.bouncing = false;
        self.resample = false;
        self.loading = false;
        self.ld_heard = None;
        self.capture = None;
        self.adding = false;
        self.lend = None;
        self.unhear();
        self.lead = 0;
        self.fill = None;
        self.free = false;
        self.forget_smart();
    }

    /// A smart start, as the first pass closes: the loop starts `lead`
    /// frames later in what the pass kept (on your first hit), its length
    /// kept. False (nothing changes) if that can't be made.
    fn start_on(&mut self, lead: usize) -> bool {
        let Some((start, src)) = self.region else {
            return false;
        };
        let size = self.views[0].size();
        let Some(v) = View::region(size, start + lead, self.len, src + lead) else {
            return false;
        };
        self.views[0] = v;
        self.region = Some((start + lead, src + lead));
        true
    }

    /// Smart loop end lets go of this loop's tap, wait, offer and snap.
    fn forget_smart(&mut self) {
        self.tap = None;
        self.closing = false;
        self.smart_wait = None;
        self.found = None;
        self.moved = Shift::NONE;
        self.snap = None;
        self.snap_frame = u64::MAX;
    }

    /// What was found and where the loop is from the press and the tap,
    /// while the loop is as smart loop end left it.
    fn smart_now(&self) -> Option<(Option<Shift>, Shift)> {
        (self.smart_version == self.version && self.snap.is_none() && self.phase == Phase::Playing)
            .then_some((self.found, self.moved))
    }

    /// How far a snap moves the start and the end ([`Snap`]), if it can
    /// be made now.
    fn snap_by(&self, s: Snap) -> Result<Shift, &'static str> {
        let top = |acts: &[Act]| match acts.last() {
            Some(a) if a.kind == Kind::Snap => Some(a.shift),
            Some(a) if a.kind == Kind::Bars => Some(Shift::NONE),
            _ => None,
        };
        match s {
            Snap::Bars(_) => Ok(Shift::NONE),
            Snap::Found | Snap::Tap | Snap::Press => {
                let (found, moved) = self.smart_now().ok_or("the loop has changed since")?;
                let to = match s {
                    Snap::Found => found.ok_or("smart loop end found nothing to change")?,
                    Snap::Tap => Shift {
                        start: moved.start,
                        end: 0,
                    },
                    _ => Shift {
                        start: 0,
                        end: moved.end,
                    },
                };
                let by = to - moved;
                if by.is_none() {
                    return Err(match s {
                        Snap::Found => "the loop is already there",
                        Snap::Tap => "the loop already ends on your tap",
                        _ => "the loop already starts on your press",
                    });
                }
                Ok(by)
            }
            Snap::Undo => top(&self.done[..self.n_done])
                .map(|by| -by)
                .ok_or("the last step isn't a snap"),
            Snap::Redo => top(&self.undone[..self.n_undone]).ok_or("the next step isn't a snap"),
        }
    }

    /// The loop's length in song ticks once snap `s` lands: Read as's
    /// bars of `bar` ticks each, the other side of the step for Undo and
    /// Redo (the shape it keeps), or as now. Or why Read as can't be made.
    fn snap_ticks(&self, s: Snap, bar: Tick) -> Result<Tick, &'static str> {
        let kept = |acts: &[Act]| {
            acts.last()
                .and_then(Act::shape)
                .map(|i| self.shapes[i].len_ticks)
        };
        match s {
            Snap::Bars(0) => Err("a loop is at least 1 bar"),
            Snap::Bars(b) => match Tick::from(b) * bar {
                t if t == self.len_ticks => Err("the loop is already read as that"),
                t => Ok(t),
            },
            Snap::Undo => kept(&self.done[..self.n_done]).ok_or("the last step isn't a snap"),
            Snap::Redo => kept(&self.undone[..self.n_undone]).ok_or("the next step isn't a snap"),
            Snap::Found | Snap::Tap | Snap::Press => Ok(self.len_ticks),
        }
    }

    /// Land the snap waiting, on its frame: the end moves or the loop is
    /// read as other bars (a step Undo takes back, [`Kind::Snap`],
    /// [`Kind::Bars`]), or the step is undone or redone. The tempo changed
    /// on the same frame, and bar 1 is on it from now on. Not if the loop
    /// changed since it was worked out (said, [`SnapDone`]).
    #[cold]
    #[inline(never)]
    fn land_snap(&mut self, stats: &mut Stats) {
        self.snap_frame = u64::MAX;
        let Some(p) = self.snap.take() else {
            return;
        };
        let ready = self.version == p.version && self.phase == Phase::Playing && !self.busy();
        let landed = ready
            && self.snap_by(p.snap) == Ok(p.by)
            && match p.snap {
                Snap::Found | Snap::Tap | Snap::Press => self.make_shift(stats, p.by),
                Snap::Bars(b) => self.make_bars(stats, b, p.ticks),
                Snap::Undo => {
                    self.undo();
                    true
                }
                Snap::Redo => {
                    self.redo();
                    true
                }
            };
        if landed {
            // The loop's downbeat: its new bars count from here (after a
            // reading of other bars, or its undo, the old bar 1 may not
            // fall on it).
            self.origin = p.tick;
            self.moved = self.moved + p.by;
            self.touch();
            self.smart_version = self.version;
        } else {
            stats.refused += 1;
        }
        self.snap_done = Some(SnapDone { plan: p, landed });
    }

    fn abort(&mut self, stats: &mut Stats) {
        if self.phase == Phase::Recording {
            stats.aborted += 1;
        }
        self.reset();
    }

    /// Undo on a cleared loop: it comes back as it was kept, fading in,
    /// at the song's place, with today's latency and launch. Whatever was
    /// under way when it was cleared (an overdub, a merge) stays stopped;
    /// a print is made again.
    fn unclear(&mut self, store: &mut Store, me: usize) {
        let (latency, on, cue) = (self.latency, self.on, self.cue);
        self.clone_from(&store.kept[me]);
        // The blocks are the loop's again.
        store.kept_mask[me] = 0;
        self.latency = latency;
        self.on = on;
        self.cue = cue;
        self.clearing = false;
        self.keep_clear = false;
        self.gain = 0.0;
        self.od_open = false;
        if self.phase == Phase::Overdubbing {
            self.phase = Phase::Playing;
        }
        self.jump = 0;
        self.xf = 0;
        self.merging = false;
        self.fitting = false;
        self.fit_overdub = false;
        self.print_wait = false;
        self.swapping = false;
        self.bouncing = false;
        self.loading = false;
        self.adding = false;
        self.lend = None;
        self.print_on = 0.0;
        self.p_frames = 0;
        self.mixing = true;
        self.touch();
        self.reclaim(store, me);
    }

    fn marker(&mut self, store: &mut Store, stats: &mut Stats, me: usize, mark: Mark, frame: u64) {
        // A pickup waiting for its bar line is no pickup once the song
        // stops or jumps: the loop listens again.
        if self.phase == Phase::Armed
            && self.pending.is_some_and(|(_, pickup)| pickup)
            && matches!(
                mark,
                Mark::Stopped { .. } | Mark::Located { .. } | Mark::Looped { .. } | Mark::Panic
            )
        {
            self.unhear();
        }
        // The lead counts from where the tap was in the song: once the
        // song stops or jumps, the loop starts on the next bar line it
        // reaches (on the bar PLAY starts on).
        if self.phase == Phase::Armed
            && matches!(
                mark,
                Mark::Stopped { .. } | Mark::Located { .. } | Mark::Looped { .. } | Mark::Panic
            )
        {
            self.tap_lead = None;
        }
        if self.phase != Phase::Recording {
            return;
        }
        // A free pass has no bars: the song starting under it (not from
        // its close) throws it away. Nothing else moves it while stopped.
        if self.free {
            if matches!(
                mark,
                Mark::Started { .. } | Mark::CountIn { .. } | Mark::Panic
            ) {
                stats.aborted += 1;
                self.reset();
                self.reclaim(store, me);
            }
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
            // STOP: what was played is kept, cut where STOP came.
            Mark::Stopped { tick } => self.stop_first(store, stats, me, tick, frame),
            Mark::Looped { .. } | Mark::Located { .. } | Mark::Panic => {
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
        // A launch waiting for this line (or one it passed: the song
        // jumped over it).
        if let Some((on, at)) = self.cue {
            if t >= at && matches!(span.state, State::Playing | State::Recording) {
                self.on = on;
                self.cue = None;
            }
        }
        match self.phase {
            // A free pass has no bar lines (the song is stopped under it).
            Phase::Recording if self.free => {}
            Phase::Recording if t > self.origin && (self.end_tick == Some(t) || self.ending) => {
                self.finish_first(store, stats, me, t, f);
            }
            Phase::Recording => {
                self.last_bar = (t, f);
                self.next_bar = span.frame_of((t + self.bar) as f64);
            }
            Phase::Armed if t >= 0 && matches!(span.state, State::Playing | State::Recording) => {
                if let Some((rec_from, lead)) = self.starts_on(span, t, f) {
                    self.begin_first(store, me, span, (t, f), rec_from, lead, f);
                }
            }
            _ => {}
        }
    }

    /// An armed loop at bar line `t` (frame `f`): whether its first pass
    /// starts here, and if so the song frame its first frame was played
    /// on and the frames of a pickup before the line. On the bar: here,
    /// on the line. Waiting for you to play: once a note has come (a
    /// pickup no more than half a bar before this line; a start from a
    /// note the song started on, unless the song started a quarter of a
    /// bar or more after it). A pickup from longer ago is forgotten and
    /// the loop listens again.
    fn starts_on(&mut self, span: &Span, t: Tick, f: u64) -> Option<(u64, usize)> {
        if self.starts == Starts::Bar || self.resample {
            // Too soon after the tap: the next bar line.
            if self
                .tap_lead
                .is_some_and(|(tap, until)| (t as f64) > tap && (t as f64) < until)
            {
                return None;
            }
            return Some((f, 0));
        }
        let (p, pickup) = self.pending?;
        let bar = span
            .frame_of((t + span.meter.bar_ticks()) as f64)
            .saturating_sub(f);
        let early = f.saturating_sub(p);
        if pickup && early > bar / 2 {
            self.unhear();
            return None;
        }
        Some(match pickup {
            true => (f - early, early as usize),
            false if early >= bar / 4 => (f, 0),
            false => (f - early, 0),
        })
    }

    /// The first pass starts on bar line `t` at frame `f` (`line`), its
    /// first frame played on song frame `rec_from` (`f`, or before it
    /// with a pickup of `lead` frames or a start from a note). The writer
    /// takes the input from frame `write_at` on; what was heard before
    /// that copies in ([`Fill`]).
    #[allow(clippy::too_many_arguments)]
    fn begin_first(
        &mut self,
        store: &mut Store,
        me: usize,
        span: &Span,
        line: (Tick, u64),
        rec_from: u64,
        lead: usize,
        write_at: u64,
    ) {
        let (t, f) = line;
        // The new loop takes the place of one kept for Undo.
        store.drop_kept(me);
        self.phase = Phase::Recording;
        self.origin = t;
        self.start_frame = f;
        self.bar = span.meter.bar_ticks();
        self.last_bar = (t, f);
        self.next_bar = span.frame_of((t + self.bar) as f64);
        self.end_tick = self.bars.map(|b| t + i64::from(b) * self.bar);
        self.ending = false;
        self.then_overdub = self.after == After::Overdub && !self.resample;
        self.first_open = true;
        self.rec_latency = if self.resample { 0 } else { self.latency };
        self.len = 0;
        self.written = 0;
        self.rec_from = rec_from;
        self.lead = lead;
        self.unhear();
        let from = rec_from + u64::from(self.rec_latency);
        self.keep_before(from);
        self.fill = (from < write_at).then(|| Fill {
            from,
            done: 0,
            to: (write_at - from) as usize,
            at: write_at,
        });
    }

    /// The first pass's first frame is the one heard at `from`: what was
    /// heard before it copies in as its first [`KEEP_BEFORE`] frames
    /// ([`Loop::render_before`]). Not for a resample, which records what
    /// the other loops play, not what was heard: it keeps the input
    /// before it for the loop point, as the writer saw it
    /// ([`Loop::keep_pre`]).
    fn keep_before(&mut self, from: u64) {
        self.tail = false;
        self.before = (!self.resample).then_some((from, 0));
    }

    /// The first pass starts free ([`Command::TempoFromLoop`]) on song
    /// frame `rec_from`, `now` being the period's first: no bars or bar
    /// lines until it closes ([`Looper::close_free`]). A start before
    /// `now` (a note) copies in from what was heard, as
    /// [`Loop::begin_first`]'s does.
    fn begin_free(&mut self, store: &mut Store, me: usize, rec_from: u64, now: u64, bar: Tick) {
        store.drop_kept(me);
        self.phase = Phase::Recording;
        self.free = true;
        self.forget_smart();
        self.origin = 0;
        self.start_frame = rec_from;
        self.bar = bar;
        self.last_bar = (0, rec_from);
        self.next_bar = u64::MAX;
        self.end_tick = None;
        self.ending = false;
        self.then_overdub = self.after == After::Overdub;
        self.first_open = true;
        self.rec_latency = self.latency;
        self.len = 0;
        self.written = 0;
        self.rec_from = rec_from;
        self.lead = 0;
        self.unhear();
        let from = rec_from + u64::from(self.rec_latency);
        self.keep_before(from);
        self.fill = (from < now).then(|| Fill {
            from,
            done: 0,
            to: (now - from) as usize,
            at: now,
        });
    }

    /// The first pass's frames heard before it began, copied in at
    /// [`LOAD_SPEED`] from what was heard. Thrown away (counted) if what
    /// was heard has moved past them or there is no memory.
    fn render_fill(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        heard: &Heard,
        end: u64,
    ) {
        let Some(mut fl) = self.fill else {
            return;
        };
        let due = end.saturating_sub(fl.at).saturating_mul(LOAD_SPEED as u64);
        let target = fl.to.min(due.min(usize::MAX as u64) as usize);
        if fl.done < target {
            if !heard.holds(fl.from + fl.done as u64)
                || !store.grow(me, 0, (KEEP_BEFORE + target - 1) / BLOCK + 1)
            {
                stats.aborted += 1;
                self.reset();
                self.reclaim(store, me);
                return;
            }
            for q in fl.done..target {
                store.set(me, 0, KEEP_BEFORE + q, heard.frame(fl.from + q as u64));
            }
            fl.done = target;
            self.written = self.written.max(target);
        }
        self.fill = (fl.done < fl.to).then_some(fl);
    }

    /// What was heard before the first pass, copied in as its first
    /// [`KEEP_BEFORE`] frames as it comes ([`Loop::keep_before`]):
    /// silence where nothing was heard (the input had just started). The
    /// pass is thrown away (counted) if there is no memory.
    fn render_before(&mut self, store: &mut Store, stats: &mut Stats, me: usize, heard: &Heard) {
        let Some((from, done)) = self.before else {
            return;
        };
        // Frame `q` kept is the one heard at `from - KEEP_BEFORE + q`.
        let first = from as i64 - KEEP_BEFORE as i64;
        let target = (heard.at() as i64 - first).clamp(0, KEEP_BEFORE as i64) as usize;
        if done >= target {
            return;
        }
        if !store.grow(me, 0, (target - 1) / BLOCK + 1) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        for q in done..target {
            let g = first + q as i64;
            let x = if g >= 0 {
                heard.frame(g as u64)
            } else {
                [0.0; 2]
            };
            store.set(me, 0, q, x);
        }
        self.before = (target < KEEP_BEFORE).then_some((from, target));
    }

    fn finish_first(&mut self, store: &mut Store, stats: &mut Stats, me: usize, t: Tick, f: u64) {
        let len = (f - self.start_frame) as usize;
        self.finish_len(store, stats, me, t, len, true);
    }

    /// STOP during the first pass (the transport stopped at `tick`, frame
    /// `f`): what was played is kept, and nothing after STOP. The loop is
    /// whole bars, so it stays on the song's bar lines: up to the next bar
    /// line, silent after the cut, or back to the bar line just passed when
    /// STOP came within a 1/16 note after it (pressed a moment late). With
    /// no whole bar left that way, nothing is kept.
    fn stop_first(&mut self, store: &mut Store, stats: &mut Stats, me: usize, tick: Tick, f: u64) {
        let (bar_t, bar_f) = self.last_bar;
        // The bar line just passed, or the next one (as the transport would
        // have reached it at the tempo then).
        let (end_t, end_f) = if tick - bar_t <= SIXTEENTH {
            (bar_t, bar_f)
        } else {
            (bar_t + self.bar, self.next_bar)
        };
        let len = end_f.saturating_sub(self.start_frame) as usize;
        if end_t <= self.origin || len == 0 {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        // Stopped: no overdub waits for PLAY.
        self.then_overdub = false;
        self.end_tick = Some(end_t);
        // Nothing after STOP is kept.
        if self.finish_len(store, stats, me, end_t, len, false) {
            self.cut = len.min(f.saturating_sub(self.rec_from) as usize);
            self.slots[0].valid = (0, KEEP_BEFORE + self.cut);
        }
    }

    /// The first pass ends at tick `t`, `len` frames long, keeping what
    /// follows it if `after` ([`KEEP_AFTER`]: its memory is taken now,
    /// or counted short). False if there is no memory for the pass (it is
    /// thrown away).
    fn finish_len(
        &mut self,
        store: &mut Store,
        stats: &mut Stats,
        me: usize,
        t: Tick,
        len: usize,
        after: bool,
    ) -> bool {
        self.mixing = true;
        // The loop is a region of what the pass keeps: what was heard
        // before it, the loop, and what the writer hears after it while
        // the loop plays. Past those, MAX_SEAM frames are never written,
        // so a seam that reaches past what was kept reads silence instead
        // of going round to the other end.
        let kept = KEEP_BEFORE + len;
        let size = kept + KEEP_AFTER + MAX_SEAM;
        // A pickup plays at the loop's end, leading into its first bar.
        if self.lead >= len {
            self.lead = 0;
        }
        let view = View::region(size, KEEP_BEFORE, len, KEEP_BEFORE + self.lead);
        let tail = after && view.is_some() && store.grow(me, 0, blocks_for(kept + KEEP_AFTER));
        let Some(view) = view.filter(|_| tail || store.grow(me, 0, blocks_for(kept))) else {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return false;
        };
        if after && !tail {
            stats.kept_short += 1;
        }
        // A pass cut back to a bar line it had recorded past gives back
        // the blocks after it.
        store.shrink(
            me,
            0,
            blocks_for(if tail { kept + KEEP_AFTER } else { kept }),
        );
        self.len = len;
        self.cut = len;
        self.len_ticks = t - self.origin;
        self.order[0] = 0;
        self.in_order = 1;
        // A pass that closed after its end (smart loop end's Auto listens
        // on past it) has written what it heard after the end already:
        // that is the start of what it keeps after it.
        let heard = if tail {
            (KEEP_BEFORE + self.written).clamp(kept, kept + KEEP_AFTER)
        } else {
            kept
        };
        self.slots[0] = Slot {
            valid: (0, heard),
            ..Slot::NEW
        };
        self.views[0] = view;
        self.region = Some((KEEP_BEFORE, KEEP_BEFORE + self.lead));
        self.captured = (KEEP_BEFORE, kept);
        self.tail = tail;
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
        true
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
            _ if self.first_open => f < self.rec_from + u64::from(self.rec_latency),
            _ if self.tail => false,
            _ => return,
        };
        if before {
            // Waiting for you to play, by ear: the first note over
            // PLAY_LEVEL, as played (the latency before it came in).
            if self.by_ear && self.listening() && x[0].abs().max(x[1].abs()) >= PLAY_LEVEL {
                self.hit = Some(f.saturating_sub(u64::from(self.latency)));
            }
            self.ring[2 * self.ring_at] = x[0];
            self.ring[2 * self.ring_at + 1] = x[1];
            self.ring_at = (self.ring_at + 1) % FADE;
            return;
        }
        let j = (f - self.rec_from - u64::from(self.rec_latency)) as usize;
        if j == 0 && self.resample && !self.keep_pre(store, stats, me) {
            return;
        }
        // The pass's frame `j` is the audio's `KEEP_BEFORE + j`.
        let q = KEEP_BEFORE + j;
        if self.len > 0 && j >= self.cut {
            // The pass is written once a start heard before it is in.
            if self.first_open && self.fill.is_none() && self.before.is_none() {
                self.close_first(store, me);
            }
            if self.tail && q >= self.captured.1 {
                self.keep_after(store, me, q, x);
            }
            return;
        }
        if !store.grow(me, 0, q / BLOCK + 1) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return;
        }
        store.set(me, 0, q, x);
        self.written = self.written.max(j + 1);
    }

    /// What the writer hears after the loop's end, as frame `q` of the
    /// audio, up to [`KEEP_AFTER`] frames (its memory was taken when the
    /// pass ended): what the layer holds grows by a frame. It stops at a
    /// gap (the slot went to another layer).
    fn keep_after(&mut self, store: &mut Store, me: usize, q: usize, x: [f32; 2]) {
        let end = self.captured.1 + KEEP_AFTER;
        if q >= end || self.slots[0].valid != (0, q) {
            self.tail = false;
            return;
        }
        store.set(me, 0, q, x);
        self.slots[0].valid.1 = q + 1;
        self.tail = q + 1 < end;
    }

    /// A resample's first frame: it keeps what the writer took just
    /// before it (what the other loops played, not what was heard) as the
    /// last [`FADE`] frames before the loop, for the loop point, and
    /// silence before those. False if there is no memory (the pass is
    /// thrown away). Out of line: it runs once a pass, and the writer
    /// around it runs every frame.
    #[inline(never)]
    fn keep_pre(&mut self, store: &mut Store, stats: &mut Stats, me: usize) -> bool {
        if !store.grow(me, 0, blocks_for(KEEP_BEFORE)) {
            stats.aborted += 1;
            self.reset();
            self.reclaim(store, me);
            return false;
        }
        for q in 0..KEEP_BEFORE - FADE {
            store.set(me, 0, q, [0.0; 2]);
        }
        for k in 0..FADE {
            let r = (self.ring_at + k) % FADE;
            let x = [self.ring[2 * r], self.ring[2 * r + 1]];
            store.set(me, 0, KEEP_BEFORE - FADE + k, x);
        }
        true
    }

    /// The first pass is written. Its loop point is a seam the loop plays
    /// ([`piece_frame`]: the end fades into what was heard just before
    /// the start), so nothing is written for it; a take STOP cut short
    /// fades out where STOP came.
    fn close_first(&mut self, store: &mut Store, me: usize) {
        self.first_open = false;
        // A resampled first pass is written: overdubs take the input.
        self.resample = false;
        let cut = self.cut.min(self.len);
        if cut < self.len {
            let out = FADE.min(cut / 2);
            for k in 0..out {
                let w = 1.0 - (k as f32 + 0.5) / out as f32;
                let q = KEEP_BEFORE + cut - out + k;
                let a = store.get(me, 0, q);
                store.set(me, 0, q, [a[0] * w, a[1] * w]);
            }
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
            if let Some(k) = slot.filter(|_| store.available() + redo_blocks >= need) {
                self.n_undone = 0;
                self.reclaim(store, me);
                if k == 0 {
                    // The first pass is gone for good: its writer stops.
                    self.tail = false;
                }
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
            link: None,
            shift: Shift::NONE,
            bars: 0,
        });

        self.in_order += 1;
        self.od_open = true;
        self.od_handoff = false;
        self.od_layer = k;
        self.od_in = 0;
        self.od_fade = fade;
        self.od_stop_in = None;
        self.od_once = self.overdubs == Overdubs::Once;
        // One pass ends with a fade as long as its start's: the two
        // crossfade where they meet.
        self.od_out_len = if self.od_once { fade.max(1) } else { FADE };
        if self.overdubs == Overdubs::Replace {
            self.slots[k].replace = Some((fade.max(1) as u32, self.od_out_len as u32));
        }
        self.od_out = self.od_out_len;
        self.od_peak = 0.0;
        self.od_faded = false;
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
    /// Throw the overdub away (recording, or still writing its tail): it
    /// stops and fades out, and Redo can bring back what it recorded.
    fn undo_open_overdub(&mut self) {
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

    fn drop_empty_overdub(&mut self) -> bool {
        if self.slots[self.od_layer].valid.1 != 0 {
            return false;
        }
        self.drop_overdub();
        true
    }

    /// Take the overdub's layer out of the play order and its step out of
    /// the history.
    fn drop_overdub(&mut self) {
        let k = self.od_layer;
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
    }

    fn close_overdub(&mut self, store: &mut Store, me: usize) {
        self.mixing = true;
        self.touch();
        self.od_open = false;
        // An overdub that took only silence (below [`SILENT`]) and faded
        // nothing leaves no layer and no step to undo. It goes at once:
        // what it holds is too quiet to click.
        // A Replace of silence stays: it silences what was there.
        let replace = self.slots[self.od_layer].replace.is_some();
        if !self.drop_empty_overdub() && self.od_peak < SILENT && !self.od_faded && !replace {
            self.drop_overdub();
        }
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

    /// Smart loop end's Auto with Then: Overdub, at the overdub's first
    /// frame (`q` in the loop, input `x`, `y` as the overdub's fade-in
    /// would write it): what the first pass's writer heard after the
    /// loop's end before this frame (you played on while Auto listened)
    /// becomes the overdub's first moments, written once just before `q`,
    /// where the overdub would have put them, fading in as the overdub
    /// would have; this frame follows at the gain after them. The pass's
    /// writer, still keeping what it hears after the end (`tail`), has
    /// just written this frame's input as its own last frame, so that
    /// frame says where it is. Nothing is handed on if the pass didn't
    /// keep it (its memory was short, [`Stats::kept_short`], or it heard
    /// too long past the end). Out of line, in the writer's fresh-layer
    /// branch: no frame but this one pays for it.
    #[cold]
    #[inline(never)]
    fn hand_off(&mut self, store: &mut Store, me: usize, q: usize, x: [f32; 2], y: [f32; 2]) {
        self.od_handoff = false;
        let (len, k) = (self.len, self.od_layer);
        let (v0, v1) = self.slots[0].valid;
        // The pass's frame for this input, and the frames after the
        // loop's end before it.
        let now = v1.saturating_sub(KEEP_BEFORE + 1);
        let first = self.region.map_or(0, |r| r.0.saturating_sub(KEEP_BEFORE));
        let h = now.saturating_sub(first + len).min(len.saturating_sub(1));
        if !self.tail || v0 != 0 || h == 0 {
            self.slots[k].valid = (q, 1);
            store.set(me, k, q, y);
            return;
        }
        let fade = self.od_fade;
        let gain = |i: usize| {
            if i < fade {
                (i as f32 + 0.5) / fade as f32
            } else {
                1.0
            }
        };
        let at = (q + len - h) % len;
        for i in 0..h {
            let a = store.get(me, 0, KEEP_BEFORE + now - h + i);
            let g = gain(i);
            self.od_peak = self.od_peak.max(a[0].abs()).max(a[1].abs());
            store.set(me, k, (at + i) % len, [a[0] * g, a[1] * g]);
        }
        let g = gain(h);
        store.set(me, k, q, [x[0] * g, x[1] * g]);
        self.slots[k].valid = (at, h + 1);
        self.od_in = h + 1;
        self.handoffs += 1;
    }

    fn write_overdub(&mut self, store: &mut Store, me: usize, x: [f32; 2]) {
        let len = self.len;
        // One pass: it punches out `latency` frames early, so that its
        // fade-out starts on the frame it started on.
        if self.od_once
            && self.od_stop_in.is_none()
            && self.od_in >= len.saturating_sub(self.latency as usize).max(1)
        {
            self.punch_out();
        }
        let mut g = if self.od_in < self.od_fade {
            (self.od_in as f32 + 0.5) / self.od_fade as f32
        } else {
            1.0
        };
        self.od_in += 1;
        self.od_peak = self.od_peak.max(x[0].abs()).max(x[1].abs());
        let mut last = false;
        match self.od_stop_in {
            Some(0) => {
                g *= (self.od_out as f32 - 0.5) / self.od_out_len as f32;
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
            if self.od_handoff {
                self.hand_off(store, me, q, x, y);
            } else {
                self.slots[k].valid = (q, 1);
                store.set(me, k, q, y);
            }
        } else if count < len {
            self.slots[k].valid = (start, count + 1);
            store.set(me, k, q, y);
        } else if self.slots[k].replace.is_some() {
            // Replace: a later pass takes the place of the earlier ones,
            // crossfading into them as it fades out.
            let was = store.get(me, k, q);
            store.set(
                me,
                k,
                q,
                [was[0] * (1.0 - g) + y[0], was[1] * (1.0 - g) + y[1]],
            );
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
        // A layer copying in or out between loops: only then are its
        // frames looked for, so an overdub, which goes a frame at a time,
        // doesn't pay for them.
        let copying = self.adding || self.lend.is_some();
        // The run's frames, as long as `ys`, so that indexing them by a
        // frame of it needs no check.
        let n = ys.len();
        let (ticks, input) = (&r.ticks[..n], &r.input.as_chunks::<2>().0[..n]);
        let mut j = 0;
        while j < ys.len() {
            if self.phase == Phase::Empty {
                // A load, or a layer copied in, starts playing on its frame
                // (whatever the period size), once all of it is in.
                let (at, ready) = if self.loading {
                    (self.ld_at, self.ld_done == self.ld_layers * self.ld_frames)
                } else if self.adding {
                    (self.ad_at, self.ad_done == self.ad_frames)
                } else {
                    return;
                };
                let at = at.saturating_sub(r.f) as usize;
                if !ready || at >= ys.len() {
                    return;
                }
                j = j.max(at);
                if self.loading {
                    self.finish_load();
                } else {
                    self.finish_add();
                }
                continue;
            }
            let f = r.f + j as u64;
            if f >= self.snap_frame {
                self.land_snap(stats);
            }
            if self.merging && f >= self.mg_at {
                self.finish_merge(store, me);
            }
            if copying {
                if self.adding && f >= self.ad_at && self.ad_done == self.ad_frames {
                    self.finish_add();
                }
                if self.lend.is_some_and(|l| f >= l.at) {
                    self.end_lend();
                }
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
                if self.snap_frame != u64::MAX {
                    end = end
                        .min(self.snap_frame.saturating_sub(r.f) as usize)
                        .max(j + 1);
                }
                if self.merging {
                    end = end.min((self.mg_at - r.f) as usize);
                }
                if self.fitting {
                    end = end.min(self.fit_at.saturating_sub(r.f) as usize).max(j + 1);
                }
                if copying {
                    if self.adding {
                        end = end.min(self.ad_at.saturating_sub(r.f) as usize).max(j + 1);
                    }
                    if let Some(l) = self.lend {
                        end = end.min(l.at.saturating_sub(r.f) as usize).max(j + 1);
                    }
                }
                self.still_run(store, me, print, &ticks[j..end], &mut ys[j..end]);
                j = end;
                continue;
            }
            let x = input[j];
            self.write_first(store, stats, me, f, x);
            self.follow(ticks[j], r.playing, first);
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
        // The print's share settled: all of it while live, none if not.
        let settled = if live { 1.0 } else { 0.0 };
        self.phase == Phase::Playing
            && playing
            && !self.od_open
            && !self.first_open
            && !self.tail
            && !self.mixing
            && self.gain == 1.0
            && self.on
            && !self.muted
            && !self.solo_out
            && !self.clearing
            && self.jump == 0
            && self.xf == 0
            && self.len > 0
            && self.print_on == settled
            && !self.swapping
            && !self.replacing()
    }

    /// A Replace layer plays: the layers under it are read frame by frame
    /// ([`Loop::sample_at`]), with how far it covers each.
    fn replacing(&self) -> bool {
        self.order[..self.in_order]
            .iter()
            .any(|&k| self.slots[k as usize].replace.is_some())
    }

    /// How much of each layer is heard at loop frame `p` under the Replace
    /// layers after it in the play order (1: all of it), through a shape's
    /// views; `strength` is how much each Replace layer silences.
    fn ducks(
        &self,
        views: &[View; MAX_LAYERS],
        len: usize,
        p: usize,
        strength: impl Fn(&Slot) -> f32,
    ) -> [f32; MAX_LAYERS] {
        let mut out = [1.0f32; MAX_LAYERS];
        let mut d = 1.0f32;
        for &k in self.order[..self.in_order].iter().rev() {
            let k = k as usize;
            out[k] = d;
            let s = &self.slots[k];
            if s.replace.is_none() {
                continue;
            }
            let v = &views[k];
            let (a, size) = if v.is_whole() {
                (p, len)
            } else if v.size() == 0 {
                continue;
            } else {
                (v.at(p), v.size())
            };
            d *= 1.0 - strength(s) * s.cover(a, size);
        }
        out
    }

    /// Frames of a still loop ([`Loop::still`]): the same sums as `read`
    /// and `sample_at` frame by frame, in the same order, so the same bits,
    /// but layer by layer. Out of line: called once a run, it keeps its
    /// own registers, whatever the frame-by-frame path around it needs.
    #[inline(never)]
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
            let (p_origin, p_ticks, pf) = (
                self.p_origin as f64,
                self.p_ticks as f64,
                self.p_frames as f64,
            );
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
            let whole = view.is_whole();
            if let Some(o) = if whole { None } else { view.forward() } {
                // One piece from audio frame `o` (a first pass's region):
                // as `layer_frame`, its frames straight from the audio
                // where they are all written (nearly always), but for the
                // seam's before the loop point.
                let seam = self.seam;
                let f = seam.min(len);
                let edge = if f > 0 && view.cut_after(0) {
                    len - f
                } else {
                    len
                };
                let (start, count) = s.valid;
                let plain = (start == 0 && o + len <= count) || count >= view.size();
                // The whole run before the seam (as nearly every run is):
                // no frame needs a check. Positions only go forward in a
                // run, so in one shorter than half the loop the last frame
                // is the furthest unless the run went round.
                let short = n > 0 && ticks[n - 1] - ticks[0] < len_ticks * 0.5;
                let top = if short && i0s[0] <= i0s[n - 1] {
                    i0s[n - 1]
                } else {
                    len - 1
                };
                if plain && top + 1 < edge {
                    for j in 0..n {
                        let (q0, q1) = (o + i0s[j], o + i1s[j]);
                        let a = store.frame(blocks, q0 / BLOCK, q0 % BLOCK);
                        let b = store.frame(blocks, q1 / BLOCK, q1 % BLOCK);
                        for c in 0..2 {
                            acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                        }
                    }
                    continue;
                }
                for j in 0..n {
                    let (i0, i1) = (i0s[j], i1s[j]);
                    let (a, b) = if plain && i0 < edge && i1 < edge {
                        let (q0, q1) = (o + i0, o + i1);
                        (
                            store.frame(blocks, q0 / BLOCK, q0 % BLOCK),
                            store.frame(blocks, q1 / BLOCK, q1 % BLOCK),
                        )
                    } else {
                        (
                            piece_frame(store, blocks, s.valid, view, seam, 0, i0),
                            piece_frame(store, blocks, s.valid, view, seam, 0, i1),
                        )
                    };
                    for c in 0..2 {
                        acc[j][c] += g[c] * (a[c] + (b[c] - a[c]) * fracs[j]);
                    }
                }
                continue;
            }
            if !whole {
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
            && self.on
            && !self.muted
            && !self.solo_out
            && !self.clearing
            && !self.swapping;
        if audible {
            self.gain = (self.gain + STEP).min(1.0);
        } else {
            self.gain = (self.gain - STEP).max(0.0);
        }
        if self.clearing && self.gain == 0.0 {
            if self.keep_clear {
                store.keep(me, self);
            }
            self.reset();
            self.reclaim(store, me);
            return [0.0; 2];
        }
        if self.swapping && self.gain == 0.0 {
            self.swap();
        }
        // The print's share moves only while a print is live or fading.
        let printing = live || self.print_on > 0.0;
        if printing {
            self.step_print(print, live);
        }
        self.mix_layers();
        if self.gain == 0.0 || self.len == 0 {
            return [0.0; 2];
        }
        let y = if printing && self.print_on > 0.0 {
            self.print_at(store, me, print)
        } else {
            self.layers_at(store, me)
        };
        let g = self.gain * self.level;
        [y[0] * g, y[1] * g]
    }

    /// The print's share, a step toward 1 while it is live and toward 0
    /// when not.
    #[cold]
    fn step_print(&mut self, print: Option<&Print>, live: bool) {
        let on = live && print.is_some() && self.p_frames > 0;
        self.print_on = if on {
            (self.print_on + STEP).min(1.0)
        } else {
            (self.print_on - STEP).max(0.0)
        };
    }

    /// The print at its place (through a jump's crossfade), crossfaded
    /// with the layers while it fades in or out.
    #[cold]
    fn print_at(&self, store: &Store, me: usize, print: Option<&Print>) -> [f32; 2] {
        let Some(p) = print.filter(|_| self.p_frames > 0) else {
            return self.layers_at(store, me);
        };
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
            let d = if s.heard(any_solo) { 1.0 } else { 0.0 };
            s.duck += (d - s.duck).clamp(-STEP, STEP);
            settled &= !s.leaving && s.presence == 1.0 && s.gl == l && s.gr == r && s.duck == d;
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
        // Under a Replace layer, each layer at its share of i0 and i1.
        let (d0, d1) = if self.replacing() {
            let strength = |s: &Slot| s.duck * s.presence;
            (
                self.ducks(views, len, i0, strength),
                self.ducks(views, len, i1, strength),
            )
        } else {
            ([1.0; MAX_LAYERS], [1.0; MAX_LAYERS])
        };
        for &k in &self.order[..self.in_order] {
            let k = k as usize;
            let s = &self.slots[k];
            let g = [s.gl * s.presence, s.gr * s.presence];
            if g == [0.0, 0.0] || (d0[k] == 0.0 && d1[k] == 0.0) {
                continue;
            }
            if !views[k].is_whole() {
                let view = &views[k];
                let blocks = store.blocks_of(me, k);
                // A first pass plays its audio straight through from one
                // frame (`still_run` does the same): away from the seam,
                // with every frame written, a frame is just `o + i`.
                let fast = view.forward().filter(|&o| {
                    let f = seam.min(len);
                    let edge = if f > 0 && view.cut_after(0) {
                        len - f
                    } else {
                        len
                    };
                    let (start, count) = s.valid;
                    let plain = (start == 0 && o + len <= count) || count >= view.size();
                    plain && i0 < edge && i1 < edge
                });
                let (a, b) = match fast {
                    Some(o) => {
                        let (q0, q1) = (o + i0, o + i1);
                        (
                            store.frame(blocks, q0 / BLOCK, q0 % BLOCK),
                            store.frame(blocks, q1 / BLOCK, q1 % BLOCK),
                        )
                    }
                    None => (
                        layer_frame(store, blocks, s.valid, view, seam, i0),
                        layer_frame(store, blocks, s.valid, view, seam, i1),
                    ),
                };
                let (a, b) = (a.map(|v| v * d0[k]), b.map(|v| v * d1[k]));
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
            let (a, b) = if d0[k] == 1.0 && d1[k] == 1.0 {
                (a, b)
            } else {
                (a.map(|v| v * d0[k]), b.map(|v| v * d1[k]))
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
    use super::{bytes_for, n_blocks, rem_ticks, Looper, BLOCK, NONE};

    /// `bytes_for` is what `Looper::new` really sets aside, and the
    /// longest Prefs size (20 minutes) fits in the block numbers at both
    /// rates, so it is never cut short without saying.
    #[test]
    fn bytes_for_is_the_memory_the_looper_takes() {
        for seconds in [0, 1, 7, 60] {
            let l = Looper::new(44_100, seconds);
            let s = &l.store;
            let real = s.pool.len() * 4 + s.tables.len() * 2 + s.free.len() * 2 + l.heard.bytes();
            assert_eq!(bytes_for(44_100, seconds), real, "{seconds} s");
            assert!(l.memory().capacity >= 44_100 * seconds as usize);
        }
        for rate in [44_100, 48_000] {
            let n = n_blocks(rate, 20 * 60);
            assert!(n < NONE as usize && n * BLOCK >= rate as usize * 1200);
        }
        assert_eq!(n_blocks(44_100, 3600), NONE as usize);
        // Five minutes: about 109 MB, a fifth of it per minute, and 21.2 MB
        // for the 60 seconds Capture takes from.
        assert_eq!(bytes_for(44_100, 300) / 1_000_000, 130);
    }

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
