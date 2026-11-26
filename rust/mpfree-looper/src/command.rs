//! What the looper is told: a loop's and a layer's commands, the
//! edits, and how a loop starts, overdubs and keeps time with a print
//! or a bounce.

use super::*;

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
    /// The clock the loop keeps time with (see [`Clock`]): the song's, or
    /// the looper's free clock. Only while the loop is empty (refused,
    /// counted, once it is armed or holds a loop); it stays through Clear,
    /// as Starts does.
    SetClock(Clock),
    /// Stop the free loops ([`Clock::Free`]) on the period's first frame:
    /// the free clock stops and goes back to its bar 1, so they stop at
    /// their tops, fading out. A first pass under way keeps what was
    /// played, as STOP keeps it on a song loop; an overdub ends; an armed
    /// free loop lets go; a free first pass in your own time takes it as
    /// its closing tap, and the free clock it sets starts stopped. STOP
    /// with the song stopped does the same. For every free loop.
    FreeStop,
    /// Play the stopped free loops from their tops: on the period's first
    /// frame with the song stopped, or while it plays on its next bar line,
    /// no sooner than [`LEAD_BEATS`] after (the song stopping first starts
    /// them then). PLAY starts them with the song. For every free loop.
    FreePlay,
    /// A first pass waiting on a Length it couldn't keep ([`Status::asks`]):
    /// play it as it read, from its top: a free loop starts the free clock
    /// (as [`Command::FreePlay`]), a song loop sets the song's tempo and
    /// starts it ([`Looper::take_tempo`]), on the next period's first
    /// frame. Refused (counted) with nothing waiting.
    UseReading,
    /// A/B on the seam, on or off, for the picked loop while smart loop
    /// end has another side to play ([`Status::offer`], or
    /// [`Status::moved`]: the snap from your tap, or your tap from the
    /// snap). Each time round, from half way, the loop plays the other
    /// side's last beat (and its first, when the start moves) at its own
    /// length and tempo, one cut's crossfade a beat before the seam; the
    /// next time round its own. Nothing else changes: no snap, no tempo,
    /// no clock, and the version stays ([`Status::ab`]). Off at the next
    /// half way, on its own side; it ends at once when the loop changes
    /// in any other way (a snap ends it as it lands). Refused (counted)
    /// with no other side, while busy or overdubbing, at another speed,
    /// with a print playing, or in a loop shorter than three beats.
    AbSeam(bool),
    /// A/B on the seam against a move of your own (Move start / Move
    /// end, [`Snap::To`]): as [`Command::AbSeam`]`(true)`, but the other
    /// side is the start and end here, from your press and tap. While it
    /// runs, another `AbMove` changes the other side from the next half
    /// way; `AbSeam(false)` ends it as for the snap's.
    AbMove(Shift),
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
    /// on Until tap (no bars set) the bars the reading picks, from the
    /// start of the bar the song stopped in. The host starts the song
    /// there, on the frame the loop comes round, at that tempo
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
    ///
    /// A free loop ([`Clock::Free`]) takes the free clock's bars while
    /// the free clock is set: those of its run since it last started (as
    /// far as it stopped, once stopped), worked out from the clock, so
    /// nothing more is kept; then it plays on the free clock. With no
    /// free clock it takes the song's bars and sets the free clock from
    /// them: a copy of the song's while the song plays on as it did over
    /// them, so the loop plays as a song loop would until the song stops
    /// or changes; else at the tempo they make, from their end. Refused
    /// when the free clock there is hasn't run since it was set.
    Capture(u32),
    /// Empty: as [`Command::Capture`], but the bars end on the bar line
    /// heard on tick `.0` (of the song as it last played on unbroken, or
    /// of the free clock's run for a free loop), however long ago, so
    /// What you played's window can take any bars still kept: `.1` of
    /// them, fewer (halving) when fewer fit. Refused when that bar line
    /// isn't kept. Again, Undo or Clear call it off, as for Capture.
    CaptureTo(Tick, u32),
    /// The input is another source now: Capture forgets what it heard
    /// before. For every loop.
    NewInput,
    /// Forget every step Redo could bring back, in every loop (a loop
    /// cleared and kept for Undo too): a new step elsewhere took their
    /// place in the one UNDO across the engines (audio tracks phase 4d.3).
    /// Their memory goes back.
    ForgetRedo,
    /// Let go of every step Undo could take back made at or before this
    /// transaction, from each loop's oldest, and of a loop kept for Undo
    /// cleared at or before it: another engine let go of a step that old
    /// (audio tracks phase 4d.3). Each loop stays as it is; the memory only
    /// those steps held goes back. A loop busy (a merge, a fit, a load, a
    /// copy, a bounce) is passed over, to be asked again.
    RetireThrough(TransactionId),
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
    pub(crate) fn frame_at(&self, p: f64) -> [f32; 2] {
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
pub(crate) struct Prints {
    pub(crate) now: Option<Print>,
    pub(crate) next: Option<Option<Print>>,
    pub(crate) old: Option<Print>,
}

impl Bounce {
    /// The audio as a WAV file (32-bit float, stereo, at `rate`), which
    /// the sampler reads. Allocates: not for the audio thread.
    pub fn wav(&self, rate: u32) -> Vec<u8> {
        save::wav(&self.audio, rate)
    }
}

/// A bounce in progress: the loop, its buffer and frames rendered.
pub(crate) struct Bouncing {
    pub(crate) n: usize,
    pub(crate) buf: Vec<f32>,
    pub(crate) done: usize,
    pub(crate) complete: Option<bool>,
    /// Without the loop's level, at [`PRINT_SPEED`] (a print's bounce).
    pub(crate) dry: bool,
    pub(crate) version: u64,
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
    /// Loop memory was full when the pass closed ([`Stats::kept_short`]):
    /// nothing after its end was kept, so the end can move earlier, not
    /// later ([`MEMORY_FULL`]). Not saved: a loop opened from a save says
    /// false.
    pub short: bool,
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
pub(crate) struct Listen {
    pub(crate) n: usize,
    /// One layer (its slot), or None for the whole loop as heard.
    pub(crate) layer: Option<u8>,
    /// The loop frame it plays next.
    pub(crate) pos: usize,
    /// Faded in from 0 over [`FADE`] frames, and out once `stopping`.
    pub(crate) gain: f32,
    pub(crate) stopping: bool,
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

#[derive(Clone, Copy)]
pub(crate) enum Queued {
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
