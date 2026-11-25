//! Loops on audio tracks, the loop host's side (the audio tracks rethink,
//! phase 4, step 4b: docs/dev/design/loops-on-audio-tracks.md).
//!
//! The loop host keeps a table from each slot (a loop number, 0 to 7) to
//! the lasting id of the loop it holds (`mpfree_seq::LoopId`), saved with
//! the loop (`LoopFile::id`). A loop gets its id when it first has sound,
//! from the audio tracks' id maker. A slot whose loop
//! a loop cell (or a block of one) names is on that cell's track: each
//! period, before the looper runs, the track player's runs for its id go
//! to the looper ([`mpfree_looper::Looper::play`]), and after it runs its
//! own buffer goes to the track ([`OnTracks::out`]). A slot no loop cell
//! names plays on the Loop strip, launched at once.
//!
//! Which ids the project names is looked at every [`LOOK_EVERY`] periods
//! (a project's cells and blocks are walked, without allocating), as the
//! audio tracks look at the track list.

use mpfree_audiotracks::{LoopPlayer, MAX_TRACKS};
use mpfree_looper::{LayerInfo, Looper, Phase, PlayRun, MAX_LAYERS, MAX_LOOPS, MAX_PLAY_RUNS};
use mpfree_seq::{LoopId, Project};

/// Periods between looks at which loops the project names (about 23 ms at
/// 128 frames a period).
pub const LOOK_EVERY: u32 = 8;

/// The slots' loops by id, and which are on audio tracks.
#[derive(Clone, Debug, Default)]
pub struct OnTracks {
    /// Each slot's loop's lasting id (0: none yet).
    ids: [u128; MAX_LOOPS],
    /// The slots whose loop the project names, as last looked.
    named: [bool; MAX_LOOPS],
    /// The project track each slot's loop is on, as last looked (the Loop
    /// group's switch: phase 4e.1).
    track: [Option<u8>; MAX_LOOPS],
    /// The slots in a first pass last period ([`OnTracks::follow_passes`]).
    passing: [bool; MAX_LOOPS],
    /// Periods since the last look.
    since: u32,
    /// Runs left out: a loop told more than [`MAX_PLAY_RUNS`] in a period.
    pub dropped: u64,
    /// Runs the looper refused (it shouldn't: counted to show it didn't).
    pub refused: u64,
}

impl OnTracks {
    /// Slot `n`'s loop's id, if it has one.
    pub fn id(&self, n: usize) -> Option<LoopId> {
        self.ids.get(n).filter(|&&i| i != 0).map(|&i| LoopId(i))
    }

    /// The project track slot `n`'s loop is on, as last looked (up to
    /// [`LOOK_EVERY`] periods old).
    pub fn track(&self, n: usize) -> Option<usize> {
        self.track.get(n).copied().flatten().map(usize::from)
    }

    /// The slot holding loop `id`.
    pub fn slot_of(&self, id: LoopId) -> Option<usize> {
        let id = id.0;
        (id != 0).then(|| self.ids.iter().position(|&i| i == id))?
    }

    /// Every slot's loop's id (0: none yet), for the time engine
    /// (`TimeHost::loops_are_on`).
    pub fn ids(&self) -> [u128; MAX_LOOPS] {
        self.ids
    }

    /// Look at which loops the project names on the next period, not up
    /// to [`LOOK_EVERY`] on: a loop just put in a cell goes on its track
    /// at once.
    pub fn look_again(&mut self) {
        self.since = 0;
    }

    /// Slot `n`'s loop's id as saved or given (0: none).
    pub fn set(&mut self, n: usize, id: u128) {
        if let Some(i) = self.ids.get_mut(n) {
            *i = id;
        }
    }

    /// Every slot forgets its loop (an open or New project).
    pub fn forget(&mut self) {
        *self = OnTracks {
            dropped: self.dropped,
            refused: self.refused,
            ..OnTracks::default()
        };
    }

    /// A first pass starting in slot `n` (`first(n)`, as [`first_pass`]
    /// says) is a new loop: the slot forgets the id it held, so
    /// [`OnTracks::needs_id`] gives it a fresh one, and a cell or block
    /// that named the old loop never plays the new one. A cleared loop
    /// the looper keeps for UNDO keeps its id until then, and comes back
    /// with it; so does a loop loaded or copied in (not a first pass).
    /// Call it each period before [`OnTracks::needs_id`].
    pub fn follow_passes(&mut self, first: impl Fn(usize) -> bool) {
        for n in 0..MAX_LOOPS {
            let f = first(n);
            if f && !self.passing[n] {
                self.ids[n] = 0;
            }
            self.passing[n] = f;
        }
    }

    /// The first slot with sound (armed, recording or playing) and no id
    /// yet: it is given one ([`OnTracks::set`]) before it is told runs.
    pub fn needs_id(&self, looper: &Looper) -> Option<usize> {
        (0..MAX_LOOPS).find(|&n| {
            self.ids[n] == 0
                && looper
                    .loop_status(n)
                    .is_some_and(|s| s.phase != Phase::Empty)
        })
    }

    /// Before the looper's period: each slot whose loop the project names
    /// is told where its runs are ([`LoopPlayer::runs`] of every track, by
    /// its id), silent where there are none; a slot named no more leaves
    /// its track. A free loop a cell names is on its track as a free cell
    /// (phase 4e.4, [`Looper::free_on_track`]): it launches itself on the
    /// free clock, its sound for its track. One made a song loop (Make
    /// song) plays itself on until its track first tells it runs. Does
    /// not allocate.
    pub fn tell(&mut self, looper: &mut Looper, project: &Project, loops: &LoopPlayer) {
        if self.since == 0 {
            for n in 0..MAX_LOOPS {
                let id = self.ids[n];
                let t = (id != 0).then(|| project.loop_track(LoopId(id))).flatten();
                self.named[n] = t.is_some();
                self.track[n] = t.and_then(|t| u8::try_from(t).ok());
            }
        }
        self.since = (self.since + 1) % LOOK_EVERY;
        for n in 0..MAX_LOOPS {
            if !self.named[n] {
                if looper.on_track(n) || looper.free_tracked(n) {
                    looper.leave_track(n);
                }
                continue;
            }
            let free = looper
                .loop_status(n)
                .is_some_and(|s| s.clock == mpfree_looper::Clock::Free);
            if free {
                if !looper.free_tracked(n) && !looper.free_on_track(n, true) {
                    self.refused += 1;
                }
                continue;
            }
            let id = self.ids[n];
            let mut runs = [PlayRun::default(); MAX_PLAY_RUNS];
            let mut k = 0;
            for t in 0..MAX_TRACKS {
                for r in loops.runs(t).iter().filter(|r| r.id == id) {
                    if k == MAX_PLAY_RUNS {
                        self.dropped += 1;
                        continue;
                    }
                    runs[k] = PlayRun {
                        from: r.from,
                        to: r.to,
                        origin: r.origin,
                    };
                    k += 1;
                }
            }
            // A free cell's loop made a song loop goes on playing itself
            // until its track has runs for it.
            if k == 0 && looper.free_tracked(n) {
                continue;
            }
            if !looper.play(n, &runs[..k]) {
                self.refused += 1;
            }
        }
    }

    /// The free cells' loops on their tracks (phase 4e.4): (project track,
    /// lasting id) for each slot whose free loop a cell names; (0, 0)
    /// where none.
    pub fn free_cells(&self, looper: &Looper) -> [(usize, u128); MAX_LOOPS] {
        std::array::from_fn(|n| match self.track[n] {
            Some(t) if self.named[n] && looper.free_tracked(n) => (usize::from(t), self.ids[n]),
            _ => (0, 0),
        })
    }

    /// Its first pass is under way: armed or recording with no layer yet,
    /// or a Capture waiting for its bar line into an empty loop.
    pub fn first_pass(looper: &Looper, n: usize) -> bool {
        looper.loop_status(n).is_some_and(|s| {
            (matches!(s.phase, Phase::Armed | Phase::Recording) && s.layers == 0)
                || (s.phase == Phase::Empty && s.capturing.is_some())
        })
    }

    /// After the looper's period: loop `id`'s sound in it, for its track
    /// (None: no slot holds it, or it isn't on a track).
    pub fn out<'a>(&self, looper: &'a Looper, id: u128) -> Option<&'a [f32]> {
        // Listen playing a loop on a track: its frames, for that track
        // (phase 4e.2).
        if id == mpfree_audiotracks::LISTEN {
            let outs = looper.outs();
            return outs.listen().filter(|l| outs.on_track(l.0)).map(|l| l.1);
        }
        let n = self.slot_of(LoopId(id))?;
        let outs = looper.outs();
        outs.on_track(n).then(|| outs.loop_out(n))
    }
}

/// Each layer's recording (audio tracks phase 4c, step 4c.2): the id of
/// the recording that holds a layer's audio as written, given once the
/// layer is closed and kept while its slot holds it, through edits, mutes,
/// UNDO and saves. A save with recordings on writes it with the layer
/// (`rec=`) and names the layer's file by it; an open puts it back.
#[derive(Clone, Debug)]
pub struct LayerRecs {
    /// For each loop's layer slots ([`LayerInfo::id`]): the layer the
    /// recording was given to ([`LayerInfo::born`]) and its id.
    recs: [[(u64, u128); MAX_LAYERS]; MAX_LOOPS],
    /// And where its file stands (phase 4c.3, [`LayerWriter`]).
    files: [[FileState; MAX_LAYERS]; MAX_LOOPS],
    /// And, for a layer a merge or a fit made, the recordings it was made
    /// from, in play order, 0 after the last (phase 4c.5).
    from: Box<[[[u128; MAX_LAYERS]; MAX_LAYERS]; MAX_LOOPS]>,
    /// And whether this run wrote its file (so the audio tracks' worker
    /// knows where it is, and UNDO can read it back: phase 4d.4).
    here: [[bool; MAX_LAYERS]; MAX_LOOPS],
}

/// Where a layer's recording's file stands (phase 4c.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FileState {
    /// Not written: the layer's audio is only in memory.
    #[default]
    Unwritten,
    /// Being written.
    Writing,
    /// Whole and synced: in the work folder, or in the project it opened
    /// from.
    Synced,
    /// Couldn't be written (said once): not tried again for this layer.
    Failed,
}

impl Default for LayerRecs {
    fn default() -> LayerRecs {
        LayerRecs {
            recs: [[(0, 0); MAX_LAYERS]; MAX_LOOPS],
            files: [[FileState::Unwritten; MAX_LAYERS]; MAX_LOOPS],
            from: Box::new([[[0; MAX_LAYERS]; MAX_LAYERS]; MAX_LOOPS]),
            here: [[false; MAX_LAYERS]; MAX_LOOPS],
        }
    }
}

impl LayerRecs {
    /// Each closed layer with no recording yet gets one from `fresh` (the
    /// audio tracks' id maker). Each period, with recordings on. Does not
    /// allocate.
    pub fn give(&mut self, looper: &Looper, mut fresh: impl FnMut() -> u128) {
        for n in 0..MAX_LOOPS {
            for k in 0..MAX_LAYERS {
                let Some(y) = looper.loop_layer_info(n, k) else {
                    break;
                };
                let s = y.id as usize % MAX_LAYERS;
                if y.closed && y.born != 0 && self.recs[n][s].0 != y.born {
                    self.recs[n][s] = (y.born, fresh());
                    self.files[n][s] = FileState::Unwritten;
                    self.here[n][s] = false;
                    // A merge's or a fit's: the recordings of the layers
                    // it was made from, which still hold them (4c.5).
                    let mut from = [0; MAX_LAYERS];
                    let born = looper.layer_from(n, k).unwrap_or([0; MAX_LAYERS]);
                    let mut j = 0;
                    for b in born.into_iter().take_while(|&b| b != 0) {
                        if let Some(&(_, rec)) = self.recs[n].iter().find(|r| r.0 == b && r.1 != 0)
                        {
                            from[j] = rec;
                            j += 1;
                        }
                    }
                    self.from[n][s] = from;
                }
            }
        }
    }

    /// Each layer whose file this run wrote and synced, as (loop, its
    /// number): it may leave memory while only UNDO needs it, and come
    /// back from its file (phase 4d.4). Does not allocate.
    pub fn written_here(&self) -> impl Iterator<Item = (usize, u64)> + '_ {
        (0..MAX_LOOPS).flat_map(move |n| {
            (0..MAX_LAYERS).filter_map(move |s| {
                let synced = self.here[n][s] && self.files[n][s] == FileState::Synced;
                synced.then_some((n, self.recs[n][s].0))
            })
        })
    }

    /// The recording of loop `n`'s layer numbered `born` (0: none).
    pub fn rec_of(&self, n: usize, born: u64) -> u128 {
        self.recs[n % MAX_LOOPS]
            .iter()
            .find(|r| r.0 == born && born != 0)
            .map_or(0, |r| r.1)
    }

    /// Layer `y` of loop `n`'s recording (0: none yet).
    pub fn of(&self, n: usize, y: &LayerInfo) -> u128 {
        let (born, rec) = self.recs[n % MAX_LOOPS][y.id as usize % MAX_LAYERS];
        if born == y.born && born != 0 {
            rec
        } else {
            0
        }
    }

    /// Loop `n` loaded from a project, its layer `k` in slot `k` as the
    /// looper puts it: each keeps the recording it was saved with (0: it
    /// gets a fresh one once it is in), whose file is in the project, and
    /// the recordings it was made from (`from`, 4c.5).
    pub fn opened(
        &mut self,
        looper: &Looper,
        n: usize,
        recs: &[u128],
        from: &[[u128; MAX_LAYERS]],
    ) {
        for (k, &rec) in recs.iter().enumerate().take(MAX_LAYERS) {
            let born = looper.slot_born(n, k).unwrap_or(0);
            if rec != 0 && born != 0 {
                self.recs[n % MAX_LOOPS][k] = (born, rec);
                self.files[n % MAX_LOOPS][k] = FileState::Synced;
                self.here[n % MAX_LOOPS][k] = false;
                self.from[n % MAX_LOOPS][k] = from.get(k).copied().unwrap_or([0; MAX_LAYERS]);
            }
        }
    }

    /// The recordings layer `y` of loop `n` was made from (4c.5), in play
    /// order: none for a layer recorded, or with no recording yet.
    pub fn sources_of(&self, n: usize, y: &LayerInfo) -> &[u128] {
        let (n, s) = (n % MAX_LOOPS, y.id as usize % MAX_LAYERS);
        if self.recs[n][s].0 != y.born || y.born == 0 {
            return &[];
        }
        let f = &self.from[n][s];
        &f[..f.iter().position(|&r| r == 0).unwrap_or(MAX_LAYERS)]
    }

    /// Where layer `y` of loop `n`'s file stands (Unwritten for a layer
    /// with no recording yet).
    pub fn file(&self, n: usize, y: &LayerInfo) -> FileState {
        let (n, s) = (n % MAX_LOOPS, y.id as usize % MAX_LAYERS);
        if self.recs[n][s].0 == y.born && y.born != 0 {
            self.files[n][s]
        } else {
            FileState::Unwritten
        }
    }

    /// Layers of loop `n` whose audio is only in memory: not yet in a
    /// synced file (a layer still recording counts). Rule 4 says them on
    /// the loop's ring and to System status.
    pub fn unsaved(&self, looper: &Looper, n: usize) -> usize {
        (0..MAX_LAYERS)
            .map_while(|k| looper.loop_layer_info(n, k))
            .filter(|y| self.file(n, y) != FileState::Synced)
            .count()
    }

    /// The files of the recordings whose layers the looper still holds
    /// (in play order, kept for an Undo or Redo, or in a loop a Clear
    /// kept: [`Looper::holds_layer`]) and that are synced, so may be in a
    /// project: `loop_<rec>.wav` each (phase 4c.6). A save names those no
    /// layer line names on its `kept` lines, so the project's clean-up
    /// leaves them be while they can still come back.
    pub fn held_files(&self, looper: &Looper) -> Vec<String> {
        let mut out = Vec::new();
        for n in 0..MAX_LOOPS {
            for s in 0..MAX_LAYERS {
                let (born, rec) = self.recs[n][s];
                if rec != 0
                    && self.files[n][s] == FileState::Synced
                    && looper.holds_layer(n, s, born)
                {
                    out.push(format!("loop_{rec:032x}.wav"));
                }
            }
        }
        out
    }

    /// The slot holding recording `rec`, by loop and slot.
    fn find(&self, rec: u128) -> Option<(usize, usize)> {
        (0..MAX_LOOPS)
            .flat_map(|n| (0..MAX_LAYERS).map(move |s| (n, s)))
            .find(|&(n, s)| rec != 0 && self.recs[n][s].1 == rec)
    }
}

/// Where closed layers' files are written (phase 4c.3): the audio tracks'
/// worker, through [`crate::audiotracks::TrackHost`]; recordings by their
/// ids. None of these allocate.
pub trait LayerFiles {
    /// A layer's file can be written now: there is a work folder.
    fn can_write(&self) -> bool;
    /// Start recording `rec`'s file, `frames` stereo frames; false if it
    /// can't be now.
    fn begin(&mut self, rec: u128, frames: usize) -> bool;
    /// An empty buffer for the next piece (its length is the room), if
    /// one is back.
    fn buffer(&mut self) -> Option<Vec<f32>>;
    /// The first `len` samples of `buf` are `rec`'s next piece.
    fn piece(&mut self, rec: u128, buf: Vec<f32>, len: usize);
    /// Every piece of `rec` is given.
    fn end(&mut self, rec: u128);
    /// Stop writing `rec`; nothing of it is kept.
    fn drop_layer(&mut self, rec: u128);
    /// What came of a file, oldest first: synced (None), or why not.
    fn written(&mut self) -> Option<(u128, Option<String>)>;
}

/// Closed layers written to their recordings' files soon after they close
/// (audio tracks phase 4c.3, docs/dev/design/loops-on-audio-tracks.md
/// rule 4), one at a time, read from the looper a piece at a time
/// ([`Looper::raw_of`]) with the loops playing on: a closed layer's audio
/// never changes, so its pieces hold together whatever the loop does
/// meanwhile. A layer that goes before its file is whole (cleared, UNDO)
/// is dropped, and written again if it comes back. Each period, with
/// recordings on; allocates nothing.
#[derive(Debug, Default)]
pub struct LayerWriter {
    job: Option<Job>,
    /// Files that couldn't be written, counted, and the last reason,
    /// until the app takes it for the log ([`LayerWriter::take_failure`]).
    pub failed: u64,
    why: Option<String>,
}

/// The layer being written: loop `n`'s slot `slot`, numbered `born`, as
/// recording `rec`, `size` frames, read up to `from`; the buffer filling
/// and its samples; and whether every piece is given (then it waits for
/// its file to be synced).
#[derive(Debug)]
struct Job {
    n: usize,
    slot: usize,
    born: u64,
    rec: u128,
    size: usize,
    from: usize,
    buf: Option<Vec<f32>>,
    fill: usize,
    ended: bool,
}

impl LayerWriter {
    /// One period: what the files said, then up to `budget` frames of the
    /// layer being written, or the next closed layer with an unwritten
    /// recording started.
    pub fn tend(
        &mut self,
        recs: &mut LayerRecs,
        looper: &Looper,
        files: &mut dyn LayerFiles,
        mut budget: usize,
    ) {
        while let Some((rec, why)) = files.written() {
            if let Some((n, s)) = recs.find(rec) {
                if recs.files[n][s] == FileState::Writing {
                    recs.files[n][s] = if why.is_none() {
                        FileState::Synced
                    } else {
                        FileState::Failed
                    };
                    recs.here[n][s] = why.is_none();
                }
            }
            if let Some(why) = why {
                self.failed += 1;
                self.why = Some(why);
            }
            if self.job.as_ref().is_some_and(|j| j.rec == rec) {
                self.job = None;
            }
        }
        if let Some(j) = self.job.as_mut().filter(|j| !j.ended) {
            if looper.raw_of(j.n, j.born, 0, &mut []).is_none() {
                // Its layer went (cleared, undone, the loop loaded over):
                // its file goes, and its buffer back.
                files.drop_layer(j.rec);
                if let Some(b) = j.buf.take() {
                    files.piece(j.rec, b, 0);
                }
                if recs.recs[j.n][j.slot] == (j.born, j.rec) {
                    recs.files[j.n][j.slot] = FileState::Unwritten;
                }
                self.job = None;
            }
        }
        if let Some(j) = self.job.as_mut().filter(|j| !j.ended) {
            while budget > 0 && j.from < j.size {
                if j.buf.is_none() {
                    j.buf = files.buffer();
                    j.fill = 0;
                }
                let Some(buf) = j.buf.as_mut() else { break };
                let room = (buf.len() - j.fill) / 2;
                let n = room.min(j.size - j.from).min(budget);
                if looper
                    .raw_of(j.n, j.born, j.from, &mut buf[j.fill..j.fill + 2 * n])
                    .is_none()
                {
                    break;
                }
                j.from += n;
                j.fill += 2 * n;
                budget -= n;
                if j.fill + 2 > buf.len() || j.from == j.size {
                    let b = j.buf.take().expect("filled");
                    files.piece(j.rec, b, j.fill);
                    j.fill = 0;
                }
            }
            if j.from == j.size && j.buf.is_none() {
                files.end(j.rec);
                j.ended = true;
            }
        }
        if self.job.is_none() && files.can_write() {
            self.start_next(recs, looper, files);
        }
    }

    /// Start the first closed layer, loop by loop in play order, with a
    /// recording and no file yet.
    fn start_next(&mut self, recs: &mut LayerRecs, looper: &Looper, files: &mut dyn LayerFiles) {
        for n in 0..MAX_LOOPS {
            for k in 0..MAX_LAYERS {
                let Some(y) = looper.loop_layer_info(n, k) else {
                    break;
                };
                let s = y.id as usize % MAX_LAYERS;
                let (born, rec) = recs.recs[n][s];
                if !y.closed || born != y.born || rec == 0 {
                    continue;
                }
                if recs.files[n][s] != FileState::Unwritten {
                    continue;
                }
                let Some(size) = looper.raw_of(n, born, 0, &mut []) else {
                    continue;
                };
                if !files.begin(rec, size) {
                    return;
                }
                recs.files[n][s] = FileState::Writing;
                self.job = Some(Job {
                    n,
                    slot: s,
                    born,
                    rec,
                    size,
                    from: 0,
                    buf: None,
                    fill: 0,
                    ended: size == 0,
                });
                if size == 0 {
                    files.end(rec);
                }
                return;
            }
        }
    }

    /// The last file that couldn't be written's reason, once, for the log.
    pub fn take_failure(&mut self) -> Option<String> {
        self.why.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::looping::PERIOD;
    use mpfree_seq::{Clip, Project, Track, TrackId};

    #[test]
    fn a_slot_is_found_by_its_loops_id_and_forgets_it_for_a_new_project() {
        let mut o = OnTracks::default();
        assert_eq!(o.id(3), None);
        o.set(3, 0xabc);
        assert_eq!(o.id(3), Some(LoopId(0xabc)));
        assert_eq!(o.slot_of(LoopId(0xabc)), Some(3));
        assert_eq!(o.slot_of(LoopId(0)), None, "no id is no slot");
        assert_eq!(o.slot_of(LoopId(7)), None);
        o.dropped = 2;
        o.forget();
        assert_eq!(o.id(3), None);
        assert_eq!(o.dropped, 2, "the counts are kept");
    }

    /// A first pass in a slot is a new loop: its old id goes when the pass
    /// starts (once, not every period of it), and a loop that comes back
    /// without one (UNDO of a clear, a load, a copy) keeps its id.
    #[test]
    fn a_new_first_pass_takes_a_new_id_and_a_loop_brought_back_keeps_its_own() {
        let mut o = OnTracks::default();
        o.set(2, 0xa);
        o.set(5, 0xb);
        o.follow_passes(|_| false);
        assert_eq!(o.id(2), Some(LoopId(0xa)), "nothing started");
        o.follow_passes(|n| n == 2);
        assert_eq!((o.id(2), o.id(5)), (None, Some(LoopId(0xb))));
        o.set(2, 0xc);
        o.follow_passes(|n| n == 2);
        assert_eq!(
            o.id(2),
            Some(LoopId(0xc)),
            "given once, kept through the pass"
        );
        o.follow_passes(|_| false);
        o.follow_passes(|n| n == 2);
        assert_eq!(o.id(2), None, "the next pass is a new loop again");
    }

    #[test]
    fn only_a_loop_the_project_names_goes_on_a_track() {
        let mut looper = Looper::new(44_100, 4);
        let loops = LoopPlayer::new();
        let mut p = Project::default();
        let mut o = OnTracks::default();
        o.set(0, 5);
        o.set(1, 6);
        o.tell(&mut looper, &p, &loops);
        assert!(!looper.on_track(0) && !looper.on_track(1));
        // A loop cell on an audio track names loop 6: slot 1 goes on it
        // at the next look (told no runs, so silent).
        let mut a = Track::new_audio("Audio 1", 0);
        a.id = TrackId(p.next_track.max(1));
        p.next_track = a.id.0 + 1;
        p.tracks.push(a);
        let at = p.tracks.len() - 1;
        p.tracks[at].slots[0] = Some(Clip::new_audio_loop(LoopId(6), "Loop", 3_840));
        p.give_clip_ids();
        for _ in 0..LOOK_EVERY {
            o.tell(&mut looper, &p, &loops);
        }
        assert!(looper.on_track(1) && !looper.on_track(0));
        assert_eq!(o.refused, 0);
        // Named no more: it leaves the track at the next look.
        p.tracks[at].slots[0] = None;
        for _ in 0..LOOK_EVERY {
            o.tell(&mut looper, &p, &loops);
        }
        assert!(!looper.on_track(1));
    }

    /// The whole chain as the audio thread runs it (main.rs): a loop
    /// recorded on the looper gets an id, a loop cell on an audio track
    /// names it, and while the arranger says the cell plays the loop
    /// sounds in that track's channel and not on the Loop strip. With the
    /// cell gone it plays on the Loop strip again.
    #[test]
    fn a_loop_cell_plays_its_loop_in_its_track_and_not_on_the_loop_track() {
        use crate::audiotracks::TrackHost;
        use crate::looping::LoopHost;
        use crate::looping::PERIOD;
        use mpfree_looper::{Command, Phase};
        use mpfree_play::{AudioAnswer, AudioPlays};
        use mpfree_transport::time::PPQ;
        use mpfree_transport::{Block, Command as T, Transport};

        const RATE: u32 = 44_100;
        let mut h = LoopHost::new(RATE, 20);
        let mut t = Transport::new(RATE, 120.0).unwrap();
        let mut b = Block::new();
        let mut tracks = TrackHost::new(RATE, 10);
        let mut p = Project::new();
        let mut a = Track::new_audio("Audio 1", 0);
        p.give_track_id(&mut a);
        let audio_id = a.id;
        p.tracks.push(a);
        tracks.sync(&p, 0, true);
        let slot = tracks.slots.slot_of(audio_id).expect("an engine track");

        // Loop 1: one bar of a 441 Hz tone from the pads, then playing.
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        t.command(0, T::Play);
        let mut f = 0u64;
        while h.looper.status().phase != Phase::Playing {
            assert!(t.process(PERIOD as u32, &mut b));
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            for i in 0..PERIOD {
                let g = (f + i as u64) as f64;
                l[i] = 0.3 * (g * 441.0 * std::f64::consts::TAU / RATE as f64).sin() as f32;
                r[i] = -l[i];
            }
            h.take_pads(&l, &r);
            h.run(&b, &mut [0.0; PERIOD], &mut [0.0; PERIOD]);
            f += PERIOD as u64;
        }
        h.give_loop_id(|| tracks.fresh_loop_id());
        let id = h.on_tracks.id(0).expect("a loop with sound gets an id");
        assert_ne!(id.0, 0);

        // Its loop cell on the audio track, playing from the loop's own
        // origin, as the arranger would answer once it's launched.
        p.tracks[0].slots[0] = Some(Clip::new_audio_loop(id, "Loop 1", i64::from(4 * PPQ)));
        p.give_clip_ids();
        let c = p.tracks[0].slots[0].as_ref().unwrap();
        let mut playing = AudioAnswer::default();
        playing.start = AudioPlays::Loop {
            id,
            origin: h.looper.loop_status(0).unwrap().origin as f64,
            len: c.length,
            clip: c.id,
            stamp: 1,
        };

        // `n` periods as main.rs runs them: the peaks of the Loop strip's
        // output and the audio track's channel over the last 20.
        let mut run = |h: &mut LoopHost, tracks: &mut TrackHost, p: &Project, n: usize| {
            let ans = if p.loop_track(id).is_some() {
                playing
            } else {
                AudioAnswer::default()
            };
            let (mut on_loop, mut on_track) = (0.0f32, 0.0f32);
            for k in 0..n {
                assert!(t.process(PERIOD as u32, &mut b));
                h.take_pads(&[0.0; PERIOD], &[0.0; PERIOD]);
                tracks.sync(p, 0, false);
                tracks.plan_loops(&b, &|_| ans);
                h.give_loop_id(|| tracks.fresh_loop_id());
                h.tell_tracks(p, tracks.loop_player());
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.run(&b, &mut l, &mut r);
                let map = *t.tempo_map();
                let src = |i: u128| h.track_loop(i);
                tracks.period_with_loops(&b, &map, true, h.input(), &|_| ans, &src);
                if k + 20 >= n {
                    on_loop = l.iter().chain(&r).fold(on_loop, |m, v| m.max(v.abs()));
                    let out = tracks.host.tracks.track_out(slot, PERIOD);
                    on_track = out.iter().fold(on_track, |m, v| m.max(v.abs()));
                }
            }
            (on_loop, on_track)
        };

        let periods = 2 * LOOK_EVERY as usize + 400;
        let (on_loop, on_track) = run(&mut h, &mut tracks, &p, periods);
        assert!(h.looper.on_track(0));
        assert!(on_loop < 1e-6, "the Loop strip is silent: {on_loop}");
        assert!(on_track > 0.1, "the audio track plays the loop: {on_track}");
        assert_eq!((h.on_tracks.dropped, h.on_tracks.refused), (0, 0));

        p.tracks[0].slots[0] = None;
        let (on_loop, on_track) = run(&mut h, &mut tracks, &p, periods);
        assert!(!h.looper.on_track(0));
        assert!(on_loop > 0.1, "back on the Loop strip: {on_loop}");
        assert!(on_track < 1e-6, "the audio track is silent: {on_track}");
    }

    /// Phase 4e.1's proof, the Play page's Loop group on the looper and
    /// the track player as main.rs runs them, a loop cell playing on Audio
    /// 1, which then switches to a take clip (silent here):
    ///
    /// - At rest the track's sound is, to the bit, what it is with no
    ///   group at all, and nothing is added to its Send A.
    /// - Turned (Level -12 dB, Delay 50 %), the track's sound is the loop's
    ///   at -12 dB once the first period's glide is done, and Send A has
    ///   0.5 on top of its own value, which is never written.
    /// - The switch to the take clip leaves the group behind: the outgoing
    ///   loop keeps -12 dB to the end of its fade, then nothing of the loop
    ///   is heard and nothing is added to Send A.
    /// - With the switch on another track (or none armed) the group
    ///   reaches nothing here: the sound is as at rest.
    #[test]
    fn the_loop_group_at_rest_leaves_the_track_as_it_was_and_a_take_clip_leaves_it_behind() {
        use crate::audiotracks::TrackHost;
        use crate::loop_group::{reaches, GroupSet, LoopGroup, SEND_A};
        use crate::looping::LoopHost;
        use mpfree_fx::{Mixer, MixerConfig, StripId};
        use mpfree_looper::Command;
        use mpfree_play::{AudioAnswer, AudioPlays};
        use mpfree_seq::ClipId;
        use mpfree_transport::time::PPQ;
        use mpfree_transport::{Block, Command as T, Transport};

        const RATE: u32 = 44_100;
        let (loop_periods, take_periods) = (2 * LOOK_EVERY as usize + 400, 40);
        let send = mpfree_fx::ids::mixer::send(SEND_A);
        // One run: Loop 1 recorded (one bar of a tone from the pads), its
        // cell on Audio 1, then the take clip; `group` None runs without
        // one. Audio 1's left side each period, and Send A's added value
        // and own value after each.
        let run = |group: Option<GroupSet>, armed: Option<usize>| {
            let mut h = LoopHost::new(RATE, 20);
            let mut t = Transport::new(RATE, 120.0).unwrap();
            let mut b = Block::new();
            let mut tracks = TrackHost::new(RATE, 10);
            let mut mixer = Mixer::new(
                MixerConfig {
                    inputs: 2,
                    pads: 4,
                    tracks: 4,
                    slots: 4,
                    max_latency_ms: 50.0,
                },
                RATE,
                PERIOD,
            );
            let mut lg = LoopGroup::new(RATE);
            if let Some(set) = group {
                lg.set_to(set);
            }
            let mut p = Project::new();
            let mut a = Track::new_audio("Audio 1", 0);
            p.give_track_id(&mut a);
            let audio_id = a.id;
            p.tracks.push(a);
            tracks.sync(&p, 0, true);
            let slot = tracks.slots.slot_of(audio_id).expect("an engine track");
            h.command(Command::SetBars(Some(1)));
            h.command(Command::Record);
            t.command(0, T::Play);
            let mut f = 0u64;
            while h.looper.status().phase != Phase::Playing {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for i in 0..PERIOD {
                    let g = (f + i as u64) as f64;
                    l[i] = 0.3 * (g * 441.0 * std::f64::consts::TAU / RATE as f64).sin() as f32;
                    r[i] = -l[i];
                }
                h.take_pads(&l, &r);
                h.run(&b, &mut [0.0; PERIOD], &mut [0.0; PERIOD]);
                f += PERIOD as u64;
            }
            h.give_loop_id(|| tracks.fresh_loop_id());
            let id = h.on_tracks.id(0).expect("a loop with sound gets an id");
            p.tracks[0].slots[0] = Some(Clip::new_audio_loop(id, "Loop 1", i64::from(4 * PPQ)));
            p.give_clip_ids();
            let c = p.tracks[0].slots[0].as_ref().unwrap();
            let mut on_loop = AudioAnswer::default();
            on_loop.start = AudioPlays::Loop {
                id,
                origin: h.looper.loop_status(0).unwrap().origin as f64,
                len: c.length,
                clip: c.id,
                stamp: 1,
            };
            let mut on_take = AudioAnswer::default();
            on_take.start = AudioPlays::Clip {
                regions: &[],
                origin: 0.0,
                len: c.length,
                last_frame: 0,
                gain: 1.0,
                clip: ClipId(c.id.0 + 1),
                stamp: 2,
            };
            let own = mixer.param(StripId::Track(0), send);
            let (mut heard, mut added) = (Vec::new(), Vec::new());
            for k in 0..loop_periods + take_periods {
                let ans = if k < loop_periods { on_loop } else { on_take };
                assert!(t.process(PERIOD as u32, &mut b));
                h.take_pads(&[0.0; PERIOD], &[0.0; PERIOD]);
                tracks.sync(&p, 0, false);
                tracks.plan_loops(&b, &|_| ans);
                h.give_loop_id(|| tracks.fresh_loop_id());
                h.tell_tracks(&p, tracks.loop_player());
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                h.run(&b, &mut l, &mut r);
                let map = *t.tempo_map();
                if group.is_some() {
                    let set = lg.set();
                    lg.run(
                        PERIOD,
                        |n| h.slot_out(n),
                        |n| reaches(&set, armed, h.slot_track(n)),
                    );
                    let src = |i: u128| {
                        h.slot_of(i)
                            .and_then(|n| lg.out(n))
                            .or_else(|| h.track_loop(i))
                    };
                    tracks.period_with_loops(&b, &map, true, h.input(), &|_| ans, &src);
                    // Audio 1 on the mixer's first track channel.
                    let plays: [Option<(usize, bool)>; MAX_TRACKS] = std::array::from_fn(|s| {
                        let p = tracks.place(s)?;
                        let on = !tracks.loop_player().runs(s).is_empty();
                        Some((p, on && reaches(&set, armed, Some(p))))
                    });
                    lg.sends(&mut mixer, |s| plays[s]);
                } else {
                    let src = |i: u128| h.track_loop(i);
                    tracks.period_with_loops(&b, &map, true, h.input(), &|_| ans, &src);
                }
                let out = tracks.host.tracks.track_out(slot, PERIOD);
                heard.push(
                    out.iter()
                        .step_by(2)
                        .take(PERIOD)
                        .copied()
                        .collect::<Vec<f32>>(),
                );
                added.push(mixer.send_add(StripId::Track(0), SEND_A));
                assert_eq!(mixer.param(StripId::Track(0), send), own, "never written");
            }
            assert!(h.looper.on_track(0));
            (heard, added)
        };
        let peak = |v: &[f32]| v.iter().fold(0.0f32, |m, x| m.max(x.abs()));

        let (plain, _) = run(None, None);
        let (rest, added) = run(Some(GroupSet::default()), Some(0));
        assert_eq!(rest, plain, "at rest, to the bit");
        assert!(added.iter().all(|&a| a == 0.0));
        let steady = loop_periods - 20;
        assert!(peak(&plain[steady]) > 0.1, "the loop plays on Audio 1");

        let turned = GroupSet {
            level_db: -12.0,
            delay: 0.5,
            ..GroupSet::default()
        };
        let gain = turned.gain();
        let (down, added) = run(Some(turned), None);
        for k in [1, steady, loop_periods - 1] {
            for (d, p) in down[k].iter().zip(&plain[k]) {
                assert!((d - p * gain).abs() < 1e-5, "period {k}: {d} for {p}");
            }
        }
        assert_eq!(added[steady], 0.5, "the Delay on Send A");
        // The switch: the fade's frames still at -12 dB, then nothing.
        let at = loop_periods;
        let fade = mpfree_audiotracks::FADE;
        assert!(peak(&plain[at][..fade]) > 0.01, "the loop fades out");
        for (d, p) in down[at][..fade].iter().zip(&plain[at][..fade]) {
            assert!((d - p * gain).abs() < 1e-5, "in the fade: {d} for {p}");
        }
        for k in at + 1..at + take_periods {
            assert_eq!(peak(&down[k]), 0.0, "period {k}: the loop is gone");
            assert_eq!(peak(&plain[k]), 0.0);
            assert_eq!(added[k], 0.0, "period {k}: nothing on Send A");
        }
        assert_eq!(added[at], 0.5, "while the loop fades");

        // The switch on the armed track: Audio 1 (track 0) armed, reached;
        // another armed, or none, as at rest.
        let armed_only = GroupSet {
            armed_only: true,
            ..turned
        };
        let (on_armed, _) = run(Some(armed_only), Some(0));
        assert_eq!(on_armed[steady], down[steady]);
        for armed in [Some(1), None] {
            let (other, added) = run(Some(armed_only), armed);
            assert_eq!(other[steady], plain[steady], "{armed:?}");
            assert!(added.iter().all(|&a| a == 0.0));
        }
    }

    /// Phase 4e.2, Listen through the track, as main.rs runs it: a loop
    /// cell playing on Audio 1, the song stopped, and Listen on the loop.
    /// What it plays is heard in Audio 1, through the track, and not on
    /// the Loop strip; Listen stopped, it fades out and both are silent.
    /// Without the track told, nothing is heard: the looper leaves a loop
    /// on a track out of the Loop strip, Listen too.
    #[test]
    fn listen_on_a_loop_cell_is_heard_through_its_track_not_the_loop_strip() {
        use crate::audiotracks::TrackHost;
        use crate::looping::LoopHost;
        use mpfree_looper::Command;
        use mpfree_play::{AudioAnswer, AudioPlays};
        use mpfree_transport::time::PPQ;
        use mpfree_transport::{Block, Command as T, Transport};

        const RATE: u32 = 44_100;
        // One run: Loop 1 recorded and playing in its cell on Audio 1, then
        // the song stopped and Listen on it for `listen` periods, then
        // stopped. The peaks of the Loop strip and Audio 1 in each part.
        let run = |tell: bool| {
            let mut h = LoopHost::new(RATE, 20);
            let mut t = Transport::new(RATE, 120.0).unwrap();
            let mut b = Block::new();
            let mut tracks = TrackHost::new(RATE, 10);
            let mut p = Project::new();
            let mut a = Track::new_audio("Audio 1", 0);
            p.give_track_id(&mut a);
            let audio_id = a.id;
            p.tracks.push(a);
            tracks.sync(&p, 0, true);
            let slot = tracks.slots.slot_of(audio_id).expect("an engine track");
            h.command(Command::SetBars(Some(1)));
            h.command(Command::Record);
            t.command(0, T::Play);
            let mut f = 0u64;
            while h.looper.status().phase != Phase::Playing {
                assert!(t.process(PERIOD as u32, &mut b));
                let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                for i in 0..PERIOD {
                    let g = (f + i as u64) as f64;
                    l[i] = 0.3 * (g * 441.0 * std::f64::consts::TAU / RATE as f64).sin() as f32;
                    r[i] = -l[i];
                }
                h.take_pads(&l, &r);
                h.run(&b, &mut [0.0; PERIOD], &mut [0.0; PERIOD]);
                f += PERIOD as u64;
            }
            h.give_loop_id(|| tracks.fresh_loop_id());
            let id = h.on_tracks.id(0).expect("a loop with sound gets an id");
            p.tracks[0].slots[0] = Some(Clip::new_audio_loop(id, "Loop 1", i64::from(4 * PPQ)));
            p.give_clip_ids();
            let c = p.tracks[0].slots[0].as_ref().unwrap();
            let mut playing = AudioAnswer::default();
            playing.start = AudioPlays::Loop {
                id,
                origin: h.looper.loop_status(0).unwrap().origin as f64,
                len: c.length,
                clip: c.id,
                stamp: 1,
            };
            let mut period =
                |h: &mut LoopHost, tracks: &mut TrackHost, ans: AudioAnswer<'static>| {
                    if matches!(ans.start, AudioPlays::Nothing) {
                        // Stopped: the arranger answers nothing.
                        t.command(0, T::Stop);
                    }
                    assert!(t.process(PERIOD as u32, &mut b));
                    h.take_pads(&[0.0; PERIOD], &[0.0; PERIOD]);
                    tracks.sync(&p, 0, false);
                    tracks.plan_loops(&b, &|_| ans);
                    h.give_loop_id(|| tracks.fresh_loop_id());
                    h.tell_tracks(&p, tracks.loop_player());
                    let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
                    h.run(&b, &mut l, &mut r);
                    if tell {
                        tracks.loop_listen(h.listen_track());
                    }
                    let map = *t.tempo_map();
                    let src = |i: u128| h.track_loop(i);
                    tracks.period_with_loops(&b, &map, false, h.input(), &|_| ans, &src);
                    let strip = l.iter().chain(&r).fold(0.0f32, |m, v| m.max(v.abs()));
                    let out = tracks.host.tracks.track_out(slot, PERIOD);
                    (strip, out.iter().fold(0.0f32, |m, v| m.max(v.abs())))
                };
            for _ in 0..2 * LOOK_EVERY as usize + 200 {
                period(&mut h, &mut tracks, playing);
            }
            assert!(h.looper.on_track(0));
            let quiet = AudioAnswer::default();
            for _ in 0..20 {
                period(&mut h, &mut tracks, quiet);
            }
            let said = h.listen(Some(None));
            assert!(said.starts_with("Listen: Loop 1 plays"), "{said}");
            let mut on = (0.0f32, 0.0f32);
            for _ in 0..200 {
                let (s, a) = period(&mut h, &mut tracks, quiet);
                on = (on.0.max(s), on.1.max(a));
            }
            assert!(h.listen_track().is_some());
            h.listen(None);
            for _ in 0..10 {
                period(&mut h, &mut tracks, quiet);
            }
            let mut after = (0.0f32, 0.0f32);
            for _ in 0..20 {
                let (s, a) = period(&mut h, &mut tracks, quiet);
                after = (after.0.max(s), after.1.max(a));
            }
            (on, after)
        };
        let ((strip, track), after) = run(true);
        assert_eq!(strip, 0.0, "not on the Loop strip");
        assert!(track > 0.1, "through Audio 1: {track}");
        assert_eq!(after, (0.0, 0.0), "stopped");
        let ((strip, track), _) = run(false);
        assert_eq!((strip, track), (0.0, 0.0), "untold: nowhere");
    }

    /// Making a loop cell, as main.rs runs it: an audio track's empty
    /// cell armed, a loop recorded on the looper gets its lasting id while
    /// it records, and when its first pass ends the time engine puts it in
    /// that cell, launched from the loop's own start; from the next period
    /// it plays in the track and not on the Loop strip. Straight on: the
    /// Loop strip and the track together are, to float rounding, the Loop
    /// strip alone with the loop never put on a track, from the first
    /// pass's first frame on, its own fade in included. With no cell armed
    /// for it (phase 4e.4) it lands in the audio track's first cell all the
    /// same, and plays on there just as straight.
    #[test]
    fn a_loop_recorded_into_an_armed_audio_cell_plays_on_in_that_track() {
        let plain = record_a_loop(Lands::Nowhere);
        assert!(!plain.in_cell && !plain.on_track);
        for into in [Lands::Armed, Lands::NoCell] {
            heard_straight_on(record_a_loop(into), &plain);
        }
    }

    /// `armed` landed in Audio 1's first cell and plays on there exactly as
    /// `plain` (the loop on no track) did on the Loop strip.
    fn heard_straight_on(armed: Heard, plain: &Heard) {
        let k = armed.landed;
        assert_eq!(armed.said, "Loop 1 on Audio 1, row 1, playing");
        assert_eq!(plain.landed, k, "the pass ends on the same period");
        assert!(armed.in_cell && armed.launched && armed.on_track);
        // After the hand-off the Loop strip is silent and the track plays.
        let after = (k + 1) * PERIOD;
        let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(
            peak(&armed.strip[after..]) == 0.0,
            "the Loop strip is silent"
        );
        assert!(
            peak(&armed.track[after..]) > 0.25,
            "the track plays the loop"
        );
        assert!(peak(&plain.track) == 0.0);
        assert!(peak(&plain.strip[after..]) > 0.25);
        // The hand-off is in the loop's own fade in (it began playing in
        // period `k`), and nothing of it shows in the sum.
        assert!(peak(&armed.strip[k * PERIOD..after]) > 0.05);
        for (i, (&a, &b)) in armed.strip.iter().zip(&armed.track).enumerate() {
            let want = plain.strip[i];
            assert!(
                (a + b - want).abs() < 1e-6,
                "frame {i}: {} + {} vs {want}",
                a,
                b
            );
        }
    }

    /// Free cells (the rethink's phase 4e.4), as main.rs runs them: a free
    /// loop (Sync: Free) recorded with no cell
    /// lands in Audio 1's first cell, not launched by the arranger; from
    /// then it is heard through Audio 1 and not on the Loop strip, with
    /// nothing lost or doubled at the hand-off; it plays on through STOP;
    /// the cell's stop silences it; launched again it comes back on the
    /// free clock's next bar line.
    #[test]
    fn a_free_loop_lands_in_a_free_cell_and_plays_through_its_track_on_the_free_clock() {
        use crate::audiotracks::TrackHost;
        use crate::looping::LoopHost;
        use crate::time::{Pads, TimeHost};
        use mpfree_looper::{Clock, Command, Phase};
        use mpfree_time_ui::TimeAction;
        use mpfree_transport::Command as T;

        struct Quiet;
        impl Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        struct Rig {
            time: TimeHost,
            h: LoopHost,
            tracks: TrackHost,
            slot: usize,
            frame: u64,
            lines: Vec<String>,
        }
        const RATE: u32 = 44_100;
        let tone = |f: u64| {
            let t = f as f64 / RATE as f64;
            (0.3 * (std::f64::consts::TAU * 441.0 * t).sin()) as f32
        };
        // A period as main.rs runs it: the Loop strip's and Audio 1's
        // loudest samples (left side).
        let period = |r: &mut Rig| {
            let (time, h, tracks) = (&mut r.time, &mut r.h, &mut r.tracks);
            time.advance(PERIOD as u32);
            time.period(PERIOD as u32, &mut Quiet);
            let _ = h.follow_untracked();
            let (mut l, mut rr) = ([0.0; PERIOD], [0.0; PERIOD]);
            if h.looper.status().phase != Phase::Playing {
                for i in 0..PERIOD {
                    l[i] = tone(r.frame + i as u64);
                    rr[i] = -l[i];
                }
            }
            h.take_pads(&l, &rr);
            tracks.sync(time.project(), time.silenced(), false);
            tracks.loops_moving(&h.moving_ids());
            tracks.plan_loops(time.block(), &|q| time.audio(q));
            h.give_loop_id(|| tracks.fresh_loop_id());
            h.tell_tracks(time.project(), tracks.loop_player());
            let _ = h.follow_free(&time.free_plan());
            let (mut l, mut rr) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.run(time.block(), &mut l, &mut rr);
            tracks.loop_free(&h.free_cells());
            let src = |i: u128| h.track_loop(i);
            tracks.period_with_loops(
                time.block(),
                time.tempo_map(),
                time.song_on(),
                h.input(),
                &|q| time.audio(q),
                &src,
            );
            let peak = |x: &mut dyn Iterator<Item = f32>| x.fold(0.0f32, |m, v| m.max(v.abs()));
            let strip = peak(&mut l.iter().copied());
            let out = tracks.host.tracks.track_out(r.slot, PERIOD);
            let track = peak(&mut out.iter().step_by(2).take(PERIOD).copied());
            if let Some(now) = h.grid_now() {
                let lines = time.loops_are_on(&now, &h.loop_ids(), true);
                if !lines.is_empty() {
                    h.look_again();
                    tracks.sync(time.project(), time.silenced(), true);
                    r.lines.extend(lines);
                }
            }
            r.frame += PERIOD as u64;
            (strip, track)
        };
        let mut p = Project::new();
        let mut a = Track::new_audio("Audio 1", 0);
        p.give_track_id(&mut a);
        let audio_id = a.id;
        p.tracks.push(a);
        let audio = p.tracks.len() - 1;
        let time = TimeHost::with_project(RATE, 120.0, p).unwrap();
        let h = LoopHost::new(RATE, 20);
        let mut tracks = TrackHost::new(RATE, 10);
        tracks.sync(time.project(), 0, true);
        let slot = tracks.slots.slot_of(audio_id).expect("an engine track");
        let mut r = Rig {
            time,
            h,
            tracks,
            slot,
            frame: 0,
            lines: Vec::new(),
        };
        r.h.command(Command::SetClock(Clock::Free));
        r.h.command(Command::SetBars(Some(1)));
        r.h.command(Command::Record);
        assert!(r.time.command(T::Play));
        // Recorded with the song (a bar at 120 BPM: 2 s), then landed.
        let mut heard = Vec::new();
        let mut landed = None;
        for k in 0..(4 * RATE as usize / PERIOD) {
            heard.push(period(&mut r));
            if landed.is_none() && !r.lines.is_empty() {
                landed = Some(k);
            }
        }
        let k = landed.expect("the free loop landed");
        assert_eq!(r.lines, ["Loop 1 on Audio 1, row 1, playing"]);
        let s = r.h.looper.loop_status(0).unwrap();
        assert_eq!(s.clock, Clock::Free);
        let cell = r.time.project().clip(audio, 0).expect("its cell");
        assert_eq!(cell.loop_id().map(|i| i.0), Some(r.h.loop_ids()[0]));
        assert!(!r.time.launched(audio), "not the arranger's");
        assert_eq!(r.time.free_plan()[0], Some(true));
        // From the hand-off on, through Audio 1 and not the Loop strip, and
        // never both nor neither.
        let on = heard[k + LOOK_EVERY as usize + 1..]
            .iter()
            .all(|&(strip, track)| strip == 0.0 && track > 0.0);
        assert!(on, "through Audio 1 only");
        assert!(r.h.looper.free_tracked(0));
        assert!(
            heard[k + 2..]
                .iter()
                .all(|&(s, t)| s.max(t) > 0.05 && s.min(t) == 0.0),
            "no gap at the hand-off, nor both"
        );
        // STOP: it plays on, on the free clock.
        assert!(r.time.command(T::Stop));
        let after: Vec<_> = (0..200).map(|_| period(&mut r)).collect();
        assert!(after[20..].iter().all(|&(s, t)| s == 0.0 && t > 0.05));
        // The cell's stop: silent.
        r.time.act(TimeAction::StopClip(audio)).unwrap();
        assert_eq!(r.time.free_plan()[0], Some(false));
        let after: Vec<_> = (0..40).map(|_| period(&mut r)).collect();
        assert!(after[10..].iter().all(|&(s, t)| s == 0.0 && t == 0.0));
        assert!(!r.h.looper.loop_status(0).unwrap().on);
        // Launched again: back on the free clock's next bar line (a bar
        // at most), through Audio 1.
        r.time
            .act(TimeAction::Launch {
                track: audio,
                slot: 0,
            })
            .unwrap();
        assert_eq!(r.time.free_plan()[0], Some(true));
        assert!(!r.time.launched(audio));
        let bar = 2 * RATE as usize / PERIOD + 2;
        let after: Vec<_> = (0..2 * bar).map(|_| period(&mut r)).collect();
        let back = after.iter().position(|&(_, t)| t > 0.0).expect("back");
        assert!(back <= bar, "on the next line: period {back}");
        assert!(after[back + 10..]
            .iter()
            .all(|&(s, t)| s == 0.0 && t > 0.05));
    }

    /// Make song on a free cell (phase 4d.2), as main.rs runs it: a free
    /// loop playing in Audio 1's free cell with the song
    /// stopped becomes a song loop on the free clock's bar line, the song
    /// starting there, and its cell a song cell launched from the loop's
    /// origin, which Audio 1 takes over; what Audio 1 plays is what it
    /// plays with the loop left free, to under 1e-5 (no gap, no step), and
    /// never on the Loop strip. It is a step on the one history: UNDO
    /// makes the loop free and its cell a free cell again, the song
    /// stopped; REDO makes the song again.
    #[test]
    fn make_song_on_a_free_cell_plays_on_through_its_track_and_the_one_undo_takes_it_back() {
        use crate::audiotracks::TrackHost;
        use crate::looping::LoopHost;
        use crate::one_undo::{OneUndo, Part};
        use crate::time::{Pads, TimeHost};
        use mpfree_looper::{Clock, Command, Phase};
        use mpfree_transport::Command as T;

        struct Quiet;
        impl Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }
        struct Rig {
            time: TimeHost,
            h: LoopHost,
            tracks: TrackHost,
            slot: usize,
            frame: u64,
            lines: Vec<String>,
        }
        const RATE: u32 = 44_100;
        let tone = |f: u64| {
            let t = f as f64 / RATE as f64;
            (0.3 * (std::f64::consts::TAU * 441.0 * t).sin()) as f32
        };
        // A period as main.rs runs it: the Loop strip's loudest sample and
        // Audio 1's left side.
        let period = |r: &mut Rig| {
            let (time, h, tracks) = (&mut r.time, &mut r.h, &mut r.tracks);
            time.advance(PERIOD as u32);
            time.period(PERIOD as u32, &mut Quiet);
            let _ = h.follow_untracked();
            let (mut l, mut rr) = ([0.0; PERIOD], [0.0; PERIOD]);
            if h.looper.status().phase != Phase::Playing {
                for i in 0..PERIOD {
                    l[i] = tone(r.frame + i as u64);
                    rr[i] = -l[i];
                }
            }
            h.take_pads(&l, &rr);
            if let (Some((line, _)), _) = h.tend_song(time, false) {
                r.lines.push(line);
            }
            tracks.sync(time.project(), time.silenced(), false);
            tracks.loops_moving(&h.moving_ids());
            tracks.plan_loops(time.block(), &|q| time.audio(q));
            h.give_loop_id(|| tracks.fresh_loop_id());
            h.tell_tracks(time.project(), tracks.loop_player());
            let _ = h.follow_free(&time.free_plan());
            let (mut l, mut rr) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.run(time.block(), &mut l, &mut rr);
            if let Some((line, _)) = h.take_song() {
                r.lines.push(line);
            }
            tracks.loop_free(&h.free_cells());
            let src = |i: u128| h.track_loop(i);
            tracks.period_with_loops(
                time.block(),
                time.tempo_map(),
                time.song_on(),
                h.input(),
                &|q| time.audio(q),
                &src,
            );
            let strip = l.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let out = tracks.host.tracks.track_out(r.slot, PERIOD);
            let track: Vec<f32> = out.iter().step_by(2).take(PERIOD).copied().collect();
            if let Some(now) = h.grid_now() {
                let lines = time.loops_are_on(&now, &h.loop_ids(), true);
                if !lines.is_empty() {
                    h.look_again();
                    tracks.sync(time.project(), time.silenced(), true);
                    r.lines.extend(lines);
                }
            }
            r.frame += PERIOD as u64;
            (strip, track)
        };
        // A free loop recorded with the song, landed in Audio 1's free
        // cell, playing on with the song stopped.
        let setup = || {
            let mut p = Project::new();
            let mut a = Track::new_audio("Audio 1", 0);
            p.give_track_id(&mut a);
            let audio_id = a.id;
            p.tracks.push(a);
            let time = TimeHost::with_project(RATE, 120.0, p).unwrap();
            let mut tracks = TrackHost::new(RATE, 10);
            tracks.sync(time.project(), 0, true);
            let slot = tracks.slots.slot_of(audio_id).expect("an engine track");
            let mut r = Rig {
                time,
                h: LoopHost::new(RATE, 20),
                tracks,
                slot,
                frame: 0,
                lines: Vec::new(),
            };
            r.h.command(Command::SetClock(Clock::Free));
            r.h.command(Command::SetBars(Some(1)));
            r.h.command(Command::Record);
            assert!(r.time.command(T::Play));
            for _ in 0..(3 * RATE as usize / PERIOD) {
                period(&mut r);
            }
            assert!(r.time.command(T::Stop));
            for _ in 0..(RATE as usize / PERIOD) {
                period(&mut r);
            }
            assert!(r.h.looper.free_tracked(0));
            assert_eq!(r.time.free_plan()[0], Some(true));
            r.lines.clear();
            r
        };
        let audio = 0;
        let periods = 4 * RATE as usize / PERIOD;
        let mut free = setup();
        let left_free: Vec<(f32, Vec<f32>)> = (0..periods).map(|_| period(&mut free)).collect();
        let mut r = setup();
        r.h.make_song();
        let made: Vec<(f32, Vec<f32>)> = (0..periods).map(|_| period(&mut r)).collect();
        assert!(
            r.lines
                .iter()
                .any(|l| l.contains("Made a song from 1 free loop")),
            "{:?}",
            r.lines
        );
        assert!(
            r.lines
                .contains(&"Loop 1 plays on as a song loop on Audio 1, row 1".to_string()),
            "{:?}",
            r.lines
        );
        let s = r.h.looper.loop_status(0).unwrap();
        assert_eq!(s.clock, Clock::Song);
        assert!(
            r.h.looper.on_track(0) && !r.h.looper.free_tracked(0),
            "Audio 1's"
        );
        assert!(r.time.launched(audio), "a song cell, launched");
        assert_eq!(r.time.free_plan()[0], None);
        assert!(r.time.info().state.rolling(), "the song plays");
        // What Audio 1 played: the same as left free, never on the strip.
        let mut most = 0.0f32;
        for ((s0, a), (s1, b)) in left_free.iter().zip(&made) {
            assert_eq!((*s0, *s1), (0.0, 0.0), "never on the Loop strip");
            for (x, y) in a.iter().zip(b) {
                most = most.max((x - y).abs());
            }
        }
        assert!(made.iter().all(|(_, t)| t.iter().any(|v| v.abs() > 0.05)));
        assert!(most < 1e-5, "Audio 1 plays on as it did: {most}");
        // The one history: Make song is the step UNDO takes back.
        let mut one = OneUndo::default();
        one.follow(&r.time, &r.tracks, &r.h);
        let e = one.next(false).expect("a step");
        assert_eq!(
            (e.part, e.words.as_str()),
            (Part::Song, "Make song · Loop 1")
        );
        assert_eq!(
            r.h.song_undo(false),
            "looper: UNDO: Make song, the loops free again"
        );
        let undone: Vec<_> = (0..periods / 2).map(|_| period(&mut r)).collect();
        assert_eq!(r.h.looper.loop_status(0).unwrap().clock, Clock::Free);
        assert!(r.h.looper.free_tracked(0) && !r.h.looper.on_track(0));
        assert_eq!(r.time.free_plan()[0], Some(true), "a free cell again");
        assert!(!r.time.launched(audio));
        assert!(!r.time.info().state.rolling(), "the song stopped");
        assert!(undone
            .iter()
            .all(|(s, t)| *s == 0.0 && t.iter().any(|v| v.abs() > 0.05)));
        one.follow(&r.time, &r.tracks, &r.h);
        assert_eq!(one.next(false).map(|e| e.part), None);
        assert_eq!(one.next(true).map(|e| e.part), Some(Part::Song));
        // REDO: the song again, the same step.
        assert_eq!(r.h.song_undo(true), "looper: REDO: Make song");
        for _ in 0..periods / 2 {
            period(&mut r);
        }
        assert_eq!(r.h.looper.loop_status(0).unwrap().clock, Clock::Song);
        assert!(r.time.launched(audio));
        one.follow(&r.time, &r.tracks, &r.h);
        assert_eq!(one.next(false).map(|e| e.part), Some(Part::Song));
        assert_eq!(one.next(true).map(|e| e.part), None);
    }

    /// What [`a_loop_recorded_into_an_armed_audio_cell_plays_on_in_that_track`]
    /// heard.
    struct Heard {
        strip: Vec<f32>,
        track: Vec<f32>,
        landed: usize,
        said: String,
        in_cell: bool,
        launched: bool,
        on_track: bool,
    }

    /// Where [`record_a_loop`]'s loop is to land.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Lands {
        /// In Audio 1's first cell, armed for it.
        Armed,
        /// No cell armed (phase 4e.4: in Audio 1's first cell all the same).
        NoCell,
        /// Never told to the time engine: it plays on the Loop strip, on no
        /// track (its "landed" is the period its first pass ends in).
        Nowhere,
    }

    /// One bar of a tone recorded on the looper from the pads, landing
    /// `into`, and the periods as main.rs runs them: the left side of the
    /// Loop strip and of Audio 1, and the period the first pass landed
    /// in.
    fn record_a_loop(into: Lands) -> Heard {
        use crate::audiotracks::TrackHost;
        use crate::looping::LoopHost;
        use crate::time::{Pads, TimeHost};
        use mpfree_looper::{Command, Phase};
        use mpfree_time_ui::TimeAction;
        use mpfree_transport::Command as T;

        struct Quiet;
        impl Pads for Quiet {
            fn hit_at(&mut self, _: u16, _: usize, _: u8, _: u32) -> bool {
                true
            }
            fn release_at(&mut self, _: u16, _: usize, _: u32) {}
            fn stop_all(&mut self) {}
            fn pressure_at(&mut self, _: u16, _: usize, _: u8, _: u32) {}
        }

        const RATE: u32 = 44_100;
        let mut p = Project::new();
        let mut a = Track::new_audio("Audio 1", 0);
        p.give_track_id(&mut a);
        let audio_id = a.id;
        p.tracks.push(a);
        let audio = p.tracks.len() - 1;
        let mut time = TimeHost::with_project(RATE, 120.0, p).unwrap();
        if into == Lands::Armed {
            time.act(TimeAction::LoopCell(Some((audio, 0)))).unwrap();
        }
        let mut h = LoopHost::new(RATE, 20);
        let mut tracks = TrackHost::new(RATE, 10);
        tracks.sync(time.project(), 0, true);
        let slot = tracks.slots.slot_of(audio_id).expect("an engine track");

        // A 441 Hz tone, its phase swaying slowly (no two places alike).
        let tone = |f: u64| {
            let t = f as f64 / RATE as f64;
            let tau = std::f64::consts::TAU;
            (0.3 * (tau * 441.0 * t + 0.5 * (tau * 0.7 * t).sin()).sin()) as f32
        };
        h.command(Command::SetBars(Some(1)));
        h.command(Command::Record);
        assert!(time.command(T::Play));
        let periods = 2 * 2 * RATE as usize / PERIOD + 40 * LOOK_EVERY as usize;
        let (mut strip, mut track) = (Vec::new(), Vec::new());
        let mut landed = None;
        for k in 0..periods {
            let f = (k * PERIOD) as u64;
            time.advance(PERIOD as u32);
            time.period(PERIOD as u32, &mut Quiet);
            for line in h.follow_untracked() {
                assert!(!line.contains("refused"), "{line}");
            }
            // The pads play the tone until the first pass ends.
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            if h.looper.status().phase != Phase::Playing {
                for i in 0..PERIOD {
                    l[i] = tone(f + i as u64);
                    r[i] = -l[i];
                }
            }
            h.take_pads(&l, &r);
            tracks.sync(time.project(), time.silenced(), false);
            tracks.loops_moving(&h.moving_ids());
            tracks.plan_loops(time.block(), &|q| time.audio(q));
            h.give_loop_id(|| tracks.fresh_loop_id());
            h.tell_tracks(time.project(), tracks.loop_player());
            let (mut l, mut r) = ([0.0; PERIOD], [0.0; PERIOD]);
            h.run(time.block(), &mut l, &mut r);
            let src = |i: u128| h.track_loop(i);
            tracks.period_with_loops(
                time.block(),
                time.tempo_map(),
                time.song_on(),
                h.input(),
                &|q| time.audio(q),
                &src,
            );
            strip.extend_from_slice(&l);
            // (Interleaved: its left side.)
            let out = tracks.host.tracks.track_out(slot, PERIOD);
            track.extend(out.iter().step_by(2).take(PERIOD));
            if into == Lands::Nowhere {
                if landed.is_none() && h.looper.status().phase == Phase::Playing {
                    landed = Some((k, String::new()));
                }
            } else if let Some(now) = h.grid_now() {
                let lines = time.loops_are_on(&now, &h.loop_ids(), true);
                if !lines.is_empty() {
                    h.look_again();
                    if landed.is_none() {
                        landed = Some((k, lines.join("; ")));
                    }
                }
            }
        }
        let (landed, said) = landed.expect("the first pass ended and landed");
        let id = h.on_tracks.id(0).expect("an id while it recorded");
        let in_cell = time.project().tracks[audio].slots[0]
            .as_ref()
            .is_some_and(|c| c.loop_id() == Some(id));
        Heard {
            strip,
            track,
            landed,
            said,
            in_cell,
            launched: time.launched(audio),
            on_track: h.looper.on_track(0),
        }
    }
}
