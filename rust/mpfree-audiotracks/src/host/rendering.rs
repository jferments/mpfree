//! Renders through the host: one way to finish a render (the audio tracks
//! rethink, phase 6.0.3; docs/dev/design/renders.md, "One way to finish a
//! render").
//!
//! [`AudioHost::render`] takes a [`RenderDesc`] the worker can make (a
//! recording's frames as they are, the whole recording) and runs the
//! plan's seven steps:
//!
//! 1. **Prepare and reserve:** the description is checked, what it read
//!    must still hold, the take must be here, finished and in memory, and
//!    a lasting id is drawn for what it makes.
//! 2. **Snapshot:** what it read is in the description (`reads`, with
//!    their revisions).
//! 3. **Render:** its source is read out of memory a piece a period into
//!    a buffer the worker made, then a job on the stretch scheduler
//!    ([`RenderWork`]) writes it into a hidden `.render_ID.wav.part` in
//!    the work folder, a piece a step ([`crate::files::RenderFile`]).
//! 4. **Check:** the file holds every sample; what it read still holds,
//!    when the job ends and again once its take has come in.
//! 5. **Make it durable:** synced, renamed `render_ID.wav`, the folder
//!    synced.
//! 6. **Commit:** the take comes in with its provenance, then one step in
//!    the tracks' history plays it wherever its source played
//!    ([`Command::Swap`], ratio 1), which UNDO takes back in one press.
//! 7. **Release:** the buffers go to the worker; the source stays, as the
//!    undo step names it.
//!
//! What it read changing under it commits nothing: its file stays, a
//! recording no lane plays, kept in Recordings at the next start, and the
//! outcome says what changed ([`RenderOutcome::Changed`]). Cancel, a
//! project opening, a full stick or any failure commits nothing and leaves
//! neither a `.part` nor a render file, except once its take is coming
//! in: its file is whole then, and stays as a changed render's does. A
//! crash leaves at most a `.part`, which the next start removes, or a
//! whole render file, which it keeps.
//!
//! Only the audio thread's side is here; nothing it does allocates or
//! frees (descriptions, provenance, names and buffers are moved to the
//! worker to drop).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use mpfree_stretch::sched;

use super::{AudioHost, FromWorker, ToWorker};
use crate::files::{FileView, RenderFile};
use crate::render::{changed, Dep, Provenance, Range, RenderDesc, Runs, Source};
use crate::save::TakeFile;
use crate::{Command, RecordingId, TakeInfo, MAX_TRACKS};

/// The scheduler's kind for the tracks' renders.
pub(super) const RENDER_KIND: &str = "audio tracks render";

/// Host `owner`'s render of recording `rec` on the process's one
/// scheduler. A host renders one at a time, but a cancelled one may still
/// be stopping when the next starts, so the key names the recording too.
pub(super) fn render_key(owner: u16, rec: RecordingId) -> sched::Key {
    sched::Key::new(RENDER_KIND, (rec.0 << 16) | u128::from(owner))
}

/// Why [`AudioHost::render`] didn't start a render. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderRefused {
    /// A render is under way.
    Busy,
    /// There is no worker thread.
    NoWorker,
    /// The description can't be made as it says ([`RenderDesc::check`]).
    Bad(&'static str),
    /// It needs the live engine: Bounce and Freeze (6.3, 6.4).
    NotOnWorker,
    /// Only a whole recording renders here yet; a part of one comes with
    /// 6.1's tools and their region edits.
    Part,
    /// It reads a clip, a loop, the routes, or a track the host has no
    /// number for ([`AudioHost::set_track_id`]): the host can't check it.
    Reads,
    /// Its recording isn't in these tracks.
    NotHere,
    /// Its recording is still recording.
    Recording,
    /// Its recording plays from its file (streaming): read from there in
    /// a later step.
    OnFile,
    /// What it read has changed since it was described.
    Changed(Dep),
}

/// What came of a render ([`AudioHost::render_outcome`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderOutcome {
    /// Committed: take `new`, recording `rec`, plays wherever take `old`
    /// played, as one undo step.
    Committed {
        old: u16,
        new: u16,
        rec: RecordingId,
    },
    /// What it read changed under it, as said: nothing committed; its file
    /// is kept, a recording no lane plays (in Recordings from the next
    /// start).
    Changed(Dep),
    /// It couldn't be made: nothing committed, and no file left.
    Failed(RenderFailure),
    /// Cancelled (or a project opened): nothing committed, and no file
    /// left.
    Cancelled,
}

/// Why a render couldn't be made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderFailure {
    /// No work folder: files are off.
    NoWorkFolder,
    /// The stick is full.
    Full,
    /// Writing its file failed otherwise.
    Write,
    /// The scheduler had no memory for it.
    NoMemory,
    /// Its recording couldn't be read.
    Read,
    /// No take place or memory for what it made.
    NoRoom,
    /// Nothing plays its source any more.
    NothingPlaysIt,
}

/// What a render leaves for the worker to drop: the audio thread never
/// frees them. `remove` asks for its file to go too.
pub(super) struct Leftovers {
    pub desc: Option<RenderDesc>,
    pub prov: Option<Provenance>,
    /// Held only to be dropped on the worker.
    #[allow(dead_code)]
    pub name: Option<String>,
    pub path: Option<PathBuf>,
    pub audio: Vec<f32>,
    pub remove: bool,
}

impl Leftovers {
    pub(super) fn audio(audio: Vec<f32>) -> Leftovers {
        Leftovers {
            desc: None,
            prov: None,
            name: None,
            path: None,
            audio,
            remove: false,
        }
    }

    pub(super) fn desc(desc: RenderDesc) -> Leftovers {
        Leftovers {
            desc: Some(desc),
            ..Leftovers::audio(Vec::new())
        }
    }

    /// On the worker: the file goes if asked, only a render's in the work
    /// folder `work`; the rest is dropped.
    pub(super) fn drop_on_worker(self, work: Option<&Path>) {
        if let (true, Some(p), Some(w)) = (self.remove, &self.path, work) {
            let render = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("render_") && n.ends_with(".wav"));
            if render && p.parent() == Some(w) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// A render under way, on the audio thread's side.
pub(super) enum Rendering {
    /// Waiting for the worker's buffer for its source.
    Sizing {
        desc: RenderDesc,
        take: TakeInfo,
        rec: RecordingId,
        at: u64,
    },
    /// Its source being read out of memory, `frame` frames so far.
    Reading {
        desc: RenderDesc,
        take: TakeInfo,
        rec: RecordingId,
        at: u64,
        audio: Vec<f32>,
        frame: usize,
    },
    /// Its job on the scheduler, which holds the description.
    Making { take: TakeInfo, rec: RecordingId },
    /// Its take coming in; then the commit.
    Loading {
        desc: RenderDesc,
        old: u16,
        new: u16,
        rec: RecordingId,
    },
}

/// The render side of the host.
pub(super) struct RenderSide {
    pub(super) state: Option<Rendering>,
    pub(super) outcome: Option<RenderOutcome>,
    /// How far its job has got, 0 to 1, as f32 bits.
    pub(super) progress: Arc<AtomicU32>,
    /// Each engine track's lasting number in the project (0: none), for
    /// a render's reads ([`AudioHost::set_track_id`]).
    pub(super) ids: [u32; MAX_TRACKS],
}

impl RenderSide {
    pub(super) fn new() -> RenderSide {
        RenderSide {
            state: None,
            outcome: None,
            progress: Arc::new(AtomicU32::new(0)),
            ids: [0; MAX_TRACKS],
        }
    }
}

impl AudioHost {
    /// Engine track `slot` is the project's track with lasting number
    /// `id` (0: none), so a render's reads can name it
    /// ([`Dep::Track`]). Does not allocate.
    pub fn set_track_id(&mut self, slot: usize, id: u32) {
        if let Some(s) = self.render.ids.get_mut(slot) {
            *s = id;
        }
    }

    /// The engine track the project's track `id` is, if the host knows it.
    pub fn track_slot(&self, id: u32) -> Option<usize> {
        (id != 0)
            .then(|| self.render.ids.iter().position(|&i| i == id))
            .flatten()
    }

    /// The first of `reads` that no longer holds as the tracks stand now
    /// ([`changed`], with [`AudioHost::rev`] and the track numbers set),
    /// or None. A clip, a loop or the routes never hold here (the app's).
    /// Does not allocate.
    pub fn holds(&self, reads: &[crate::render::Read]) -> Option<Dep> {
        changed(reads, |d| self.rev(d, |id| self.track_slot(id)))
    }

    /// Start rendering `desc`, made at `at` (seconds since 1970, for its
    /// provenance): see the module notes. What came of it comes in
    /// [`AudioHost::render_outcome`]. A refusal changes nothing (and the
    /// description goes to the worker to drop). Does not allocate.
    pub fn render(&mut self, desc: RenderDesc, at: u64) -> Result<(), RenderRefused> {
        match self.prepare(&desc) {
            Ok((take, frames)) => {
                let rec = self.tracks.fresh_rec();
                self.render.outcome = None;
                self.render.progress.store(0, Ordering::Relaxed);
                self.send(ToWorker::RenderBuffer {
                    n: frames * take.channels,
                    gen: self.generation,
                });
                self.render.state = Some(Rendering::Sizing {
                    desc,
                    take,
                    rec,
                    at,
                });
                Ok(())
            }
            Err(why) => {
                self.leave(Leftovers::desc(desc));
                Err(why)
            }
        }
    }

    /// Whether `desc` can start: its take and frames.
    fn prepare(&self, desc: &RenderDesc) -> Result<(TakeInfo, usize), RenderRefused> {
        use RenderRefused as No;
        if self.render.state.is_some() {
            return Err(No::Busy);
        }
        if self.tx.is_none() {
            return Err(No::NoWorker);
        }
        desc.check().map_err(No::Bad)?;
        if desc.runs != Runs::Worker {
            return Err(No::NotOnWorker);
        }
        let (Source::Recording { rec, frames }, Range::Frames { from, len }) =
            (&desc.source, desc.range)
        else {
            return Err(No::NotOnWorker);
        };
        if from != 0 || len != *frames {
            return Err(No::Part);
        }
        let known = |d: &Dep| match *d {
            Dep::Recording(_) | Dep::Project => true,
            Dep::Track(id) => self.track_slot(id).is_some(),
            Dep::Clip(_) | Dep::Loop(_) | Dep::Routes => false,
        };
        if !desc.reads.iter().all(|r| known(&r.dep)) {
            return Err(No::Reads);
        }
        let take = self
            .tracks
            .take_of(*rec)
            .and_then(|id| self.tracks.take(id))
            .ok_or(No::NotHere)?;
        if take.recording {
            return Err(No::Recording);
        }
        if self.plays_from_file(take.id) {
            return Err(No::OnFile);
        }
        if let Some(d) = self.holds(&desc.reads) {
            return Err(No::Changed(d));
        }
        if take.frames as u64 != *frames {
            return Err(No::Changed(Dep::Recording(*rec)));
        }
        Ok((take, take.frames))
    }

    /// Whether a render is under way.
    pub fn rendering(&self) -> bool {
        self.render.state.is_some()
    }

    /// How far the render under way has got, 0 to 1, roughly: reading its
    /// source is the first tenth, its job to 0.9, its take coming in the
    /// rest.
    pub fn render_progress(&self) -> Option<f32> {
        Some(match self.render.state.as_ref()? {
            Rendering::Sizing { .. } => 0.0,
            Rendering::Reading { take, frame, .. } => {
                0.1 * (*frame as f32 / take.frames.max(1) as f32).min(1.0)
            }
            Rendering::Making { .. } => {
                0.1 + 0.8 * f32::from_bits(self.render.progress.load(Ordering::Relaxed))
            }
            Rendering::Loading { .. } => 0.9,
        })
    }

    /// What came of the last render, once: taken by the asking.
    pub fn render_outcome(&mut self) -> Option<RenderOutcome> {
        self.render.outcome.take()
    }

    /// Stop the render under way: nothing is committed, and its `.part`
    /// goes (a take already coming in is forgotten; its file is kept).
    /// Does not allocate.
    pub fn cancel_render(&mut self) {
        if self.stop_render() {
            self.render.outcome = Some(RenderOutcome::Cancelled);
        }
    }

    /// Stops the render under way; whether there was one.
    pub(super) fn stop_render(&mut self) -> bool {
        match self.render.state.take() {
            None => return false,
            Some(Rendering::Sizing { desc, .. }) => self.leave(Leftovers::desc(desc)),
            Some(Rendering::Reading { desc, audio, .. }) => self.leave(Leftovers {
                audio,
                ..Leftovers::desc(desc)
            }),
            // The job says so when it stops; what it sends is left.
            Some(Rendering::Making { rec, .. }) => self.send(ToWorker::CancelRender(rec)),
            Some(Rendering::Loading { desc, new, .. }) => {
                self.tracks.command(Command::Forget(new));
                self.leave(Leftovers::desc(desc));
            }
        }
        true
    }

    /// The tracks were emptied (a project opening, New): the render under
    /// way stops, as Cancelled. A take coming in went with the rest, so
    /// nothing is forgotten here (its id may be the new project's soon).
    pub(super) fn clear_render(&mut self) {
        let stopped = match self.render.state.take() {
            None => false,
            Some(Rendering::Loading { desc, .. }) => {
                self.leave(Leftovers::desc(desc));
                true
            }
            Some(s) => {
                self.render.state = Some(s);
                self.stop_render()
            }
        };
        if stopped {
            self.render.outcome = Some(RenderOutcome::Cancelled);
        }
    }

    /// Things for the worker to drop.
    pub(super) fn leave(&self, l: Leftovers) {
        self.send(ToWorker::RenderLeftovers(l));
    }

    /// The worker made the source's buffer.
    pub(super) fn render_buffer(&mut self, audio: Vec<f32>, gen: u32) {
        match self.render.state.take() {
            Some(Rendering::Sizing {
                desc,
                take,
                rec,
                at,
            }) if gen == self.generation => {
                self.render.state = Some(Rendering::Reading {
                    desc,
                    take,
                    rec,
                    at,
                    audio,
                    frame: 0,
                });
            }
            other => {
                self.render.state = other;
                self.leave(Leftovers::audio(audio));
            }
        }
    }

    /// The source read a piece a period, then handed to the worker; a
    /// take coming in, then the commit.
    pub(super) fn tend_render(&mut self, budget: usize) {
        match self.render.state.take() {
            Some(Rendering::Reading {
                desc,
                take,
                rec,
                at,
                mut audio,
                mut frame,
            }) => {
                let ch = take.channels;
                let to = (frame + budget).min(take.frames);
                match self
                    .tracks
                    .read_take(take.id, frame, &mut audio[frame * ch..to * ch])
                {
                    Some(n) if frame + n >= take.frames => {
                        self.send(ToWorker::Render {
                            desc,
                            audio,
                            channels: ch,
                            frames: take.frames,
                            rec,
                            at,
                            gen: self.generation,
                            progress: self.render.progress.clone(),
                        });
                        self.render.state = Some(Rendering::Making { take, rec });
                    }
                    Some(n) => {
                        frame += n;
                        self.render.state = Some(Rendering::Reading {
                            desc,
                            take,
                            rec,
                            at,
                            audio,
                            frame,
                        });
                    }
                    None => {
                        self.leave(Leftovers {
                            audio,
                            ..Leftovers::desc(desc)
                        });
                        self.render.outcome = Some(RenderOutcome::Failed(RenderFailure::Read));
                    }
                }
            }
            Some(Rendering::Loading {
                desc,
                old,
                new,
                rec,
            }) => {
                if self.tracks.take(new).is_some() && self.tracks.loading() == 0 {
                    let track = self.tracks.take(old).map(|t| t.track);
                    let plays =
                        track.is_some_and(|t| self.tracks.regions(t).iter().any(|r| r.take == old));
                    self.render.outcome = Some(match self.holds(&desc.reads) {
                        Some(d) => {
                            self.tracks.command(Command::Forget(new));
                            RenderOutcome::Changed(d)
                        }
                        None if !plays => {
                            self.tracks.command(Command::Forget(new));
                            RenderOutcome::Failed(RenderFailure::NothingPlaysIt)
                        }
                        None => {
                            self.tracks.command(Command::Swap {
                                old,
                                new,
                                ratio: 1.0,
                            });
                            RenderOutcome::Committed { old, new, rec }
                        }
                    });
                    self.leave(Leftovers::desc(desc));
                } else if self.tracks.take(new).is_some() || self.coming(new) {
                    self.render.state = Some(Rendering::Loading {
                        desc,
                        old,
                        new,
                        rec,
                    });
                } else {
                    self.leave(Leftovers::desc(desc));
                    self.render.outcome = Some(RenderOutcome::Failed(RenderFailure::NoRoom));
                }
            }
            other => self.render.state = other,
        }
    }

    /// The job made the render whole and durable: check, then the take
    /// comes in with its provenance.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rendered(
        &mut self,
        rec: RecordingId,
        name: String,
        path: PathBuf,
        audio: Vec<f32>,
        desc: RenderDesc,
        prov: Provenance,
        gen: u32,
    ) {
        let mine = matches!(self.render.state, Some(Rendering::Making { rec: r, .. }) if r == rec);
        let gone = |audio, desc, prov, name, path, remove| Leftovers {
            desc: Some(desc),
            prov: Some(prov),
            name: Some(name),
            path: Some(path),
            audio,
            remove,
        };
        if gen != self.generation || !mine {
            // Cancelled, or a project opened: it commits nothing, and its
            // file goes.
            self.leave(gone(audio, desc, prov, name, path, true));
            return;
        }
        let Some(Rendering::Making { take, .. }) = self.render.state.take() else {
            return;
        };
        if let Some(d) = self.holds(&desc.reads) {
            // Kept, a recording no lane plays.
            self.leave(gone(audio, desc, prov, name, path, false));
            self.render.outcome = Some(RenderOutcome::Changed(d));
            return;
        }
        let Some(new) = self.tracks.free_take_id() else {
            self.leave(gone(audio, desc, prov, name, path, true));
            self.render.outcome = Some(RenderOutcome::Failed(RenderFailure::NoRoom));
            return;
        };
        let frames = audio.len() / take.channels.max(1);
        let k = TakeFile {
            id: new,
            rec,
            track: take.track,
            group: take.group,
            pass: take.pass,
            bpm: take.bpm,
            varied: take.varied,
            anchor: take.anchor,
            frames,
            channels: take.channels,
            file: String::new(),
            path: None,
            audio,
            prov: None,
        };
        match self.tracks.load_take(k) {
            Ok(()) => {
                self.files.views[new as usize] = Some((rec, FileView::SAVED));
                self.send(ToWorker::Known {
                    rec,
                    take: new,
                    name,
                    path: Some(path),
                    frames,
                    channels: take.channels,
                });
                self.keep_prov(rec, Some(prov));
                self.render.state = Some(Rendering::Loading {
                    desc,
                    old: take.id,
                    new,
                    rec,
                });
            }
            Err(k) => {
                self.leave(gone(k.audio, desc, prov, name, path, true));
                self.render.outcome = Some(RenderOutcome::Failed(RenderFailure::NoRoom));
            }
        }
    }

    /// Recording `rec`'s job stopped without a render (cancelled, refused,
    /// failed).
    pub(super) fn render_gone(
        &mut self,
        rec: RecordingId,
        why: RenderOutcome,
        l: Leftovers,
        gen: u32,
    ) {
        self.leave(l);
        let mine = matches!(self.render.state, Some(Rendering::Making { rec: r, .. }) if r == rec);
        if gen == self.generation && mine {
            self.render.state = None;
            self.render.outcome = Some(why);
        }
    }
}

/// A render's job on the scheduler: writes its audio to its file in the
/// work folder a piece a step, then makes it durable and hands it back.
pub(super) struct RenderWork {
    pub desc: Option<RenderDesc>,
    pub prov: Option<Provenance>,
    pub audio: Vec<f32>,
    pub channels: usize,
    pub frames: usize,
    pub rate: u32,
    pub rec: RecordingId,
    pub dir: PathBuf,
    pub file: Option<RenderFile>,
    pub put: usize,
    pub failed: Option<RenderFailure>,
    pub done: Sender<FromWorker>,
    pub gen: u32,
    pub progress: Arc<AtomicU32>,
}

/// An error writing a render's file, as a failure.
fn failure(e: &std::io::Error) -> RenderFailure {
    if e.kind() == std::io::ErrorKind::StorageFull {
        RenderFailure::Full
    } else {
        RenderFailure::Write
    }
}

impl RenderWork {
    fn leftovers(&mut self) -> Leftovers {
        Leftovers {
            desc: self.desc.take(),
            prov: self.prov.take(),
            name: None,
            path: None,
            audio: std::mem::take(&mut self.audio),
            remove: false,
        }
    }

    fn gone(mut self, why: RenderOutcome) {
        if let Some(f) = self.file.take() {
            f.abandon();
        }
        let l = self.leftovers();
        let _ = self.done.send(FromWorker::RenderGone {
            rec: self.rec,
            why,
            leftovers: l,
            gen: self.gen,
        });
    }
}

impl sched::Work for RenderWork {
    fn step(&mut self, frames: usize) -> bool {
        if self.failed.is_some() {
            return true;
        }
        if self.file.is_none() {
            match RenderFile::begin(
                mpfree_stream::fsio::StdFs::new(),
                &self.dir,
                self.rec,
                self.frames,
                self.channels,
                self.rate,
            ) {
                Ok(f) => self.file = Some(f),
                Err(e) => {
                    self.failed = Some(failure(&e));
                    return true;
                }
            }
        }
        let to = (self.put + frames.max(1) * self.channels).min(self.audio.len());
        if let Some(f) = self.file.as_mut() {
            if let Err(e) = f.write(&self.audio[self.put..to]) {
                self.failed = Some(failure(&e));
                return true;
            }
        }
        self.put = to;
        let p = self.put as f32 / self.audio.len().max(1) as f32;
        self.progress.store(p.to_bits(), Ordering::Relaxed);
        self.put >= self.audio.len()
    }

    fn progress(&self) -> f32 {
        self.put as f32 / self.audio.len().max(1) as f32
    }

    fn finish(mut self: Box<Self>) {
        if let Some(why) = self.failed {
            return self.gone(RenderOutcome::Failed(why));
        }
        let Some(file) = self.file.take() else {
            return self.gone(RenderOutcome::Failed(RenderFailure::Write));
        };
        match file.finish() {
            Ok((name, path)) => {
                let l = self.leftovers();
                let m = match (l.desc, l.prov) {
                    (Some(desc), Some(prov)) => FromWorker::Rendered {
                        rec: self.rec,
                        name,
                        path,
                        audio: l.audio,
                        desc,
                        prov,
                        gen: self.gen,
                    },
                    // Never: the job holds both until here.
                    (desc, prov) => FromWorker::RenderGone {
                        rec: self.rec,
                        why: RenderOutcome::Failed(RenderFailure::Write),
                        leftovers: Leftovers {
                            desc,
                            prov,
                            name: Some(name),
                            path: Some(path),
                            audio: l.audio,
                            remove: true,
                        },
                        gen: self.gen,
                    },
                };
                let _ = self.done.send(m);
            }
            Err(e) => self.gone(RenderOutcome::Failed(failure(&e))),
        }
    }

    fn cancelled(self: Box<Self>) {
        self.gone(RenderOutcome::Cancelled);
    }

    fn refused(self: Box<Self>, _why: String) {
        self.gone(RenderOutcome::Failed(RenderFailure::NoMemory));
    }
}
