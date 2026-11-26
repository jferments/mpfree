//! What the looper tells: each loop's phase and status, its layers
//! and their history, the memory and the counters.

use super::*;

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
    pub(crate) done: [Step; HISTORY],
    pub(crate) n_done: usize,
    pub(crate) redo: [Step; HISTORY],
    pub(crate) n_redo: usize,
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

/// A step of a loop's history by its transaction, for the one UNDO
/// across the engines (audio tracks phase 4d: [`Looper::loop_steps`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepTxn {
    pub txn: TransactionId,
    /// The step; None for the loop's Clear, kept for Undo.
    pub step: Option<Step>,
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
    /// Which layer this is, for a host that keeps it as a recording (audio
    /// tracks phase 4c): a new number each time a layer is made (a first
    /// pass, an overdub, a merge, a fit, a copy, a capture or a load),
    /// never given twice in this looper, and the same through edits,
    /// mutes, undo and redo. Its audio as written ([`Looper::raw_layer`])
    /// never changes once it is `closed`.
    pub born: u64,
    /// All of its audio is written: not being recorded, and for a first
    /// pass, no longer keeping what it hears after its loop's end.
    pub closed: bool,
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
    /// Overdubs on a loop on a track that punched out because the track
    /// stopped playing it (audio tracks phase 4b), each its layer kept.
    pub track_punch_outs: u64,
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
    /// Until tap (no bars set): Record was pressed again, the pass ends on
    /// the next bar.
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
    /// The clock it keeps time with ([`Command::SetClock`]). A free loop's
    /// `origin`, `ticks`, `starts_on` and `cue` are the free clock's ticks
    /// ([`Looper::free_clock`]).
    pub clock: Clock,
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
    /// listening (the note it waits for would). A free loop
    /// ([`Clock::Free`]) records so with nothing playing and no free clock
    /// yet, whatever the setting, and its length sets the free clock's
    /// tempo, not the song's.
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
    /// What smart loop end found, from your press and tap (not from where
    /// the loop is, as [`Status::offer`] is): a loop moved but not there
    /// was moved by hand ([`Snap::To`]). Lasts as [`Status::offer`] does.
    pub found: Option<Shift>,
    /// A snap given to land on this frame ([`Looper::snap_at`]).
    pub snapping: Option<u64>,
    /// The first pass closed on bars Length didn't say, because Length's
    /// would be slower than [`LOOP_MIN_BPM`]: how it read (its `said` is
    /// Length's). It waits, silent, with no tempo set, until
    /// [`Command::UseReading`] plays it as read or Clear throws it away
    /// (Length's bars are final: the looper asks; the looper refactor's
    /// Finding C).
    pub asks: Option<Reading>,
    /// A/B on the seam runs ([`Command::AbSeam`]): whether this time
    /// round plays the other side's seam. None while it is off.
    pub ab: Option<bool>,
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
    pub(crate) store: &'a Store,
    pub(crate) lp: usize,
    pub(crate) slot: usize,
    pub(crate) len: usize,
    /// The frames of its audio written: (first, count), wrapping.
    pub(crate) valid: (usize, usize),
    /// How the loop's edits play its audio.
    pub(crate) view: View,
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
