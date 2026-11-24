//! Continue in the time engine (composing tools, phase 1): one clip carried
//! on, heard in place of the clip from the next bar line, kept as one take
//! that UNDO takes back, or left as it was with Back.
//!
//! The composer never runs on the time engine's threads. A Continue copies
//! just what the reading needs ([`ContinueJob`]: the clip and the rest of
//! its row) and hands it to a thread of its own, started on the first
//! Continue, at idle priority on the helper cores
//! ([`mpfree_lanes::os::idle_job`]); its answer is taken on the control
//! half's next poll. The thread runs only the newest job waiting: a job
//! overtaken by a newer ask is skipped, and its answer would be dropped
//! anyway.
//!
//! [`Preview`] keeps the states (contract 7 of the design). [`Composing`]
//! keeps what the screens show beside them and what the play view plays;
//! the control half (`TimeControl`'s Continue section) does what each step
//! asks of the play view:
//!
//! - a result in while its clip plays: it comes in on the next bar line,
//!   in time with the row: its tick 0 falls where the clip started, so a
//!   clip carried on from 2 bars to 4 lines up with the 4-bar part that
//!   started with it, as the reading heard them (Back gives the clip its
//!   own origin back); while the clip doesn't play, or the transport is
//!   stopped, at once;
//! - ‹ › or Try again while one plays: the new one on the next bar line,
//!   the last one playing until then;
//! - Back while one plays: the clip as it was, on the next bar line;
//! - Keep: the result written as one take (`Replaced::Continue`) and the
//!   view compiled from it at once: it plays the same notes, so nothing is
//!   heard to change;
//! - any change to the clip, or to a clip the reading listened to (an
//!   edit, a recording, UNDO): the result is dropped and the clip plays as
//!   it now is, at once. REC stops a result playing at once, and none comes
//!   in while recording.
//!
//! What gave way is counted for the status line ([`ComposeCounts`]).

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::Arc;

use mpfree_compose::{
    Arrival, Ask, ContinueJob, Continued, Done, Making, Preview, Refusal, Settings, State,
};
use mpfree_seq::{Clip, Project};
use mpfree_time_ui::continuing::{Again, Continuing, Scope, Try};
use mpfree_transport::time::Tick;

/// What Continue gave way on, since the start. The status line says the
/// ones that aren't 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ComposeCounts {
    /// Continue and Try again asked.
    pub asked: u64,
    /// Results that came in.
    pub made: u64,
    /// Asks the composer refused (a setting in the way, too many notes).
    pub refused: u64,
    /// Results a newer ask, or Back, had overtaken: dropped unheard (or
    /// never made).
    pub late: u64,
    /// Results whose clip, or a clip they listened to, changed while they
    /// were being made: dropped.
    pub stale: u64,
    /// Results a change stopped while ready or playing.
    pub invalidated: u64,
    /// Readings that left a clip out (gone, or past the reading's budget).
    pub left_out: u64,
    /// Keeps refused (the clip changed, or the clip's own limits).
    pub keeps_refused: u64,
    /// Results REC stopped while they played.
    pub stopped_by_rec: u64,
}

impl ComposeCounts {
    /// For the status line: None while nothing was asked.
    pub fn describe(&self) -> Option<String> {
        if self.asked == 0 {
            return None;
        }
        let mut line = format!("continue: {} asked, {} made", self.asked, self.made);
        for (n, what) in [
            (self.refused, "refused"),
            (self.late, "overtaken"),
            (self.stale, "made from a clip that changed"),
            (self.invalidated, "stopped by a change"),
            (self.left_out, "clips left out of readings"),
            (self.keeps_refused, "keeps refused"),
            (self.stopped_by_rec, "stopped by REC"),
        ] {
            if n > 0 {
                line.push_str(&format!(", {n} {what}"));
            }
        }
        Some(line)
    }
}

/// One result, as the preview keeps it: what was made and the clip as it
/// is heard with it.
struct Made {
    /// Its own number, never reused: what the play view plays is named by
    /// it, as results come and go from the last 8.
    id: u32,
    made: Arc<Continued>,
    clip: Arc<Clip>,
    new_from: Tick,
    bars: u8,
    /// The style it was made in.
    style: &'static str,
    /// Its seed: a try on part of it keeps it, so the bars around play
    /// as they did (step 3).
    seed: u64,
    /// Made of part of the try before it: which.
    again: Option<Again>,
}

/// A composer's thread: jobs out, answers back, each with its request's
/// number. Continue and Add a part each have one.
pub(super) struct Worker<J, D> {
    pub(super) jobs: Sender<(u32, Box<J>)>,
    pub(super) done: Receiver<(u32, Option<Box<D>>)>,
}

impl<J: Send + 'static, D: Send + 'static> Worker<J, D> {
    /// Start the thread: `run` makes each job's answer. `what` names the
    /// tool in the words of a refusal ("continue", "add a part").
    pub(super) fn start(what: &str, run: fn(&J) -> D) -> Result<Worker<J, D>, String> {
        let (jobs, rx) = mpsc::channel::<(u32, Box<J>)>();
        let (tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("compose".into())
            .spawn(move || {
                // Started from the control half, which may be the audio
                // thread: off its core, and after everything else.
                let _ = crate::cores::background();
                mpfree_lanes::os::idle_job();
                while let Ok(mut job) = rx.recv() {
                    // Only the newest waiting job is worth making.
                    loop {
                        match rx.try_recv() {
                            Ok(newer) => {
                                if tx.send((job.0, None)).is_err() {
                                    return;
                                }
                                job = newer;
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                    }
                    let (n, job) = job;
                    if tx.send((n, Some(Box::new(run(&job))))).is_err() {
                        return;
                    }
                }
            })
            .map_err(|e| format!("{what}: the composer's thread didn't start: {e}"))?;
        Ok(Worker { jobs, done })
    }
}

/// A result in the play view, in place of its clip.
#[derive(Clone, Debug)]
pub(super) struct InView {
    pub id: u32,
    pub track: usize,
    pub slot: usize,
    clip: Arc<Clip>,
}

/// What the music changes to on a bar line, and when it was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LineWait {
    pub line: Tick,
    /// Where the transport was when it was asked: a jump back (a loop, a
    /// locate) lands it at once.
    pub asked: Tick,
}

/// Continue's side of the time engine's control half.
pub(super) struct Composing {
    pub(super) preview: Preview,
    worker: Option<Worker<ContinueJob, Done>>,
    /// The results the preview keeps, in its order.
    made: Vec<Made>,
    next_id: u32,
    /// The ask a result is being made for, by request number, with the
    /// clip as it was asked on (for the clip as heard).
    waiting: Option<(u32, Ask, Clip)>,
    /// The last ask's settings, for the screens.
    settings: Option<Settings>,
    /// Asks so far: each one's seed.
    seeds: u64,
    /// The result the play view sent last plays in place of its clip, or
    /// None: the clip itself. Kept apart from the preview's results, which
    /// Back clears while the result plays on to its line.
    pub(super) in_view: Option<InView>,
    /// The change waiting for its bar line.
    pub(super) line: Option<LineWait>,
    /// While a result is in the view, its clip's launch as it was:
    /// (track, slot, start, origin). The result's tick 0 falls where the
    /// clip started, so it plays in time with what started with it;
    /// the clip as it was gets its origin back.
    pub(super) origin: Option<(usize, usize, Tick, Tick)>,
    /// Why nothing can be heard or kept, for the screens.
    words: Option<String>,
    pub(super) counts: ComposeCounts,
}

impl Composing {
    pub(super) fn new() -> Composing {
        Composing {
            preview: Preview::new(),
            worker: None,
            made: Vec::new(),
            next_id: 0,
            waiting: None,
            settings: None,
            seeds: 0,
            in_view: None,
            line: None,
            origin: None,
            words: None,
            counts: ComposeCounts::default(),
        }
    }

    /// A preview is under way, or its result is still in the view.
    pub(super) fn active(&self) -> bool {
        self.preview.state() != State::Idle || self.in_view.is_some()
    }

    /// The clip a preview is under way for.
    pub(super) fn bound(&self) -> Option<(usize, usize)> {
        self.preview.binding().map(|b| (b.track, b.slot))
    }

    /// The next seed: each ask sounds new, and a session asked the same
    /// things gives the same results.
    fn seed(&mut self) -> u64 {
        self.seeds += 1;
        let mut z = self.seeds.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z ^ (z >> 31)
    }

    /// What an ask makes (step 3): everything, the locked bars of the
    /// shown result kept; or the picked bars or notes of the shown result
    /// again, with its seed. Refused for a part when nothing is shown for
    /// that clip.
    fn making(&mut self, ask: &mut Ask, scope: &Scope, locked: &[u16]) -> Result<Making, String> {
        let shown = self
            .shown()
            .filter(|_| self.bound() == Some((ask.track, ask.slot)))
            .map(|m| (m.made.clone(), m.seed, m.new_from, m.clip.clone()));
        let bar = ask.meter.bar_ticks().max(1);
        let attempt = |c: &mut Composing| {
            c.seed();
            (c.seeds % u64::from(u16::MAX)) as u16 + 1
        };
        Ok(match (scope, shown) {
            (Scope::All, Some((before, ..))) if !locked.is_empty() => {
                ask.settings.seed = self.seed();
                Making::Keeping {
                    before,
                    locked: locked.to_vec(),
                }
            }
            (Scope::All, _) => {
                ask.settings.seed = self.seed();
                Making::New
            }
            (Scope::Bars(bars), Some((before, seed, ..))) => {
                ask.settings.seed = seed;
                Making::Bars {
                    before,
                    bars: bars
                        .iter()
                        .copied()
                        .filter(|b| !locked.contains(b))
                        .collect(),
                    attempt: attempt(self),
                }
            }
            (Scope::Notes(ticks), Some((before, seed, new_from, clip))) => {
                // Each picked tick's new note, out of locked bars: a note's
                // bar is where it falls on the grid, before the groove and
                // the playing move it a few ms (S3 plays a downbeat a
                // little early on the climb).
                let at = |n: &mpfree_seq::Note| {
                    before
                        .events
                        .iter()
                        .find(|e| {
                            e.pitch == n.note
                                && mpfree_compose::timing::written(
                                    e.at, e.timing, ask.bpm, before.to,
                                ) == n.tick
                        })
                        .map_or(n.tick, |e| e.at)
                };
                let notes: Vec<(Tick, u8)> = clip
                    .notes
                    .iter()
                    .filter(|n| n.on && at(n) >= new_from && ticks.contains(&n.tick))
                    .filter(|n| !locked.contains(&u16::try_from(at(n) / bar).unwrap_or(u16::MAX)))
                    .map(|n| (n.tick, n.note))
                    .collect();
                if notes.is_empty() {
                    return Err(format!(
                        "continue: {}",
                        mpfree_compose::Refused::NoNewNotes.words()
                    ));
                }
                ask.settings.seed = seed;
                Making::Notes {
                    before,
                    notes,
                    attempt: attempt(self),
                }
            }
            (_, None) => {
                return Err("continue: there's no try to work on yet: Try again makes one".into())
            }
        })
    }

    /// Ask for a result: the preview is bound to the clip as it is now,
    /// and the job goes to the composer's thread. `scope` and `locked`
    /// say what it makes ([`Self::making`]). The request's number.
    pub(super) fn ask(
        &mut self,
        project: &Project,
        mut ask: Ask,
        scope: &Scope,
        locked: &[u16],
    ) -> Result<u32, String> {
        let b = mpfree_compose::Binding::of(project, ask.track, ask.slot)
            .ok_or_else(|| "continue: the clip has gone".to_string())?;
        let clip = project
            .clip(ask.track, ask.slot)
            .cloned()
            .ok_or_else(|| "continue: the clip has gone".to_string())?;
        ask.how = self.making(&mut ask, scope, locked)?;
        let job = ContinueJob::new(project, ask.clone()).map_err(|r| r.words())?;
        if self.worker.is_none() {
            self.worker = Some(Worker::start("continue", ContinueJob::run)?);
        }
        let n = self
            .preview
            .ask(b)
            .map_err(|r| format!("continue: {}", r.words()))?;
        self.sync_made();
        let worker = self.worker.as_ref().expect("started above");
        if worker.jobs.send((n, Box::new(job))).is_err() {
            // The thread is gone: start another next time.
            self.worker = None;
            self.preview.cancel();
            self.sync_made();
            return Err("continue: the composer stopped: Continue again".into());
        }
        self.counts.asked += 1;
        self.settings = Some(ask.settings);
        self.waiting = Some((n, ask, clip));
        self.words = None;
        Ok(n)
    }

    /// Take the composer's answers. Each is checked against the project as
    /// it is now. True if anything changed for the screens; a line for the
    /// log about each result that came in or was refused.
    pub(super) fn answers(&mut self, project: &Project, log: &mut Vec<String>) -> bool {
        let Some(w) = &self.worker else {
            return false;
        };
        let mut got = Vec::new();
        let gone = loop {
            match w.done.try_recv() {
                Ok(x) => got.push(x),
                Err(TryRecvError::Empty) => break false,
                Err(TryRecvError::Disconnected) => break true,
            }
        };
        let mut changed = false;
        if gone {
            self.worker = None;
            if self.preview.state() == State::Preparing {
                self.preview.cancel();
                self.sync_made();
                self.words = Some("The composer stopped: Continue again.".into());
                log.push("continue: the composer stopped".into());
                changed = true;
            }
        }
        for (n, done) in got {
            let waiting = self.waiting.take_if(|(m, ..)| *m == n);
            let (Some(done), Some((_, ask, clip))) = (done, waiting) else {
                self.counts.late += 1;
                continue;
            };
            self.counts.left_out += done.left_out as u64;
            changed = true;
            match done.result {
                Err(r) => {
                    self.counts.refused += 1;
                    self.preview.cancel();
                    self.sync_made();
                    self.words = Some(r.words());
                    log.push(format!("continue: nothing made: {}", r.words()));
                }
                Ok((proposal, made)) => {
                    let mut heard = clip;
                    if let Err(e) = heard.apply(&proposal.edit) {
                        // continuation() checked the limits: should not be.
                        self.counts.refused += 1;
                        self.preview.cancel();
                        self.sync_made();
                        let words = Refusal::Model(e).words();
                        log.push(format!("continue: nothing made: {words}"));
                        self.words = Some(words.to_string());
                        continue;
                    }
                    match self.preview.arrived(n, proposal, project) {
                        Arrival::Taken => {
                            self.counts.made += 1;
                            let bars = ask.settings.bars.max(1);
                            let bar = ask.meter.bar_ticks().max(1);
                            if self.made.len() == mpfree_compose::preview::HISTORY {
                                self.made.remove(0);
                            }
                            self.next_id = self.next_id.wrapping_add(1);
                            self.made.push(Made {
                                id: self.next_id,
                                new_from: made.to - Tick::from(bars) * bar,
                                made: Arc::new(made),
                                clip: Arc::new(heard),
                                bars,
                                style: ask.settings.style.name,
                                seed: ask.settings.seed,
                                again: again_of(&ask.how),
                            });
                            log.push(format!(
                                "continue: {} ready",
                                more_bars(bars, &self.made.last().expect("pushed").clip.name)
                            ));
                        }
                        Arrival::Dropped => self.counts.late += 1,
                        Arrival::Stale(_) => {
                            self.counts.stale += 1;
                            self.sync_made();
                            self.words = Some(changed_words());
                        }
                    }
                }
            }
        }
        changed
    }

    /// The preview's results went (another clip, Back, Keep, a change):
    /// so do ours.
    fn sync_made(&mut self) {
        match self.preview.position() {
            None => self.made.clear(),
            Some((_, n)) => {
                let extra = self.made.len().saturating_sub(n);
                self.made.drain(..extra);
            }
        }
    }

    /// The shown result's clip as it is heard with it, with where its
    /// new bars start, and the clip it is bound to (Steps show it).
    pub(super) fn heard(&self) -> Option<(usize, usize, &Clip, Tick)> {
        let (track, slot) = self.bound()?;
        let m = self.shown()?;
        Some((track, slot, &m.clip, m.new_from))
    }

    /// What the Steps show of Continue, to know when it changes: the
    /// bound clip and the shown result's number.
    pub(super) fn heard_key(&self) -> Option<(usize, usize, u32)> {
        let (track, slot) = self.bound()?;
        Some((track, slot, self.shown()?.id))
    }

    /// The shown result.
    fn shown(&self) -> Option<&Made> {
        let (i, _) = self.preview.position()?;
        self.made.get(i - 1)
    }

    /// What the play view should play in place of the bound clip once the
    /// next change lands: the shown result while it plays, or one ready to
    /// come in (not after a refused Keep, and not while recording); None,
    /// the clip itself.
    pub(super) fn wanted(&self, recording: bool) -> Option<u32> {
        let shown = self.shown()?.id;
        match self.preview.state() {
            State::Auditioning | State::Preparing if self.preview.plays().is_some() => Some(shown),
            State::Ready if !recording && self.preview.refused().is_none() => Some(shown),
            _ => None,
        }
    }

    /// The id of the result in the view.
    pub(super) fn in_view_id(&self) -> Option<u32> {
        self.in_view.as_ref().map(|v| v.id)
    }

    /// The clip to compile in place of its own, while a result is in the
    /// view: (track, slot, clip).
    pub(super) fn in_view_clip(&self) -> Option<(usize, usize, &Clip)> {
        self.in_view.as_ref().map(|v| (v.track, v.slot, &*v.clip))
    }

    /// The clip whose music changes next: the bound one, or the one a
    /// result still plays in place of after Back.
    pub(super) fn changing(&self) -> Option<(usize, usize)> {
        self.bound()
            .or_else(|| self.in_view.as_ref().map(|v| (v.track, v.slot)))
    }

    /// The clip's change landed: `want` is what the view plays now. A
    /// result ready starts playing; Back is done.
    pub(super) fn landed(&mut self, want: Option<u32>) {
        self.line = None;
        self.in_view = match (want, self.bound()) {
            (Some(id), Some((track, slot))) => {
                self.made.iter().find(|m| m.id == id).map(|m| InView {
                    id,
                    track,
                    slot,
                    clip: m.clip.clone(),
                })
            }
            _ => None,
        };
        if self.in_view.is_some() && self.preview.state() == State::Ready {
            let _ = self.preview.audition();
        }
        if self.preview.state() == State::Cancelling {
            self.preview.view_plain();
        }
    }

    /// ‹ or ›. False at either end, or with nothing to step through.
    pub(super) fn step(&mut self, back: bool) -> bool {
        self.preview.step(back)
    }

    /// Keep, first half: the clip to write and its bars, if the result
    /// still fits the project. The preview is then committing; say how it
    /// went with [`Composing::kept`].
    pub(super) fn keep(&mut self, project: &Project) -> Result<(usize, usize, Clip, u8), String> {
        let shown = self.shown().map(|m| ((*m.clip).clone(), m.bars));
        let b = self.preview.keep(project).map_err(|r| {
            self.counts.keeps_refused += 1;
            if let Refusal::Stale(_) = r {
                self.counts.invalidated += 1;
                self.sync_made();
                self.words = Some(changed_words());
            }
            format!("continue: {}", r.words())
        })?;
        let (clip, bars) = shown.ok_or_else(|| "continue: nothing to keep yet".to_string())?;
        Ok((b.0.track, b.0.slot, clip, bars))
    }

    /// Keep, second half: the take written, or why the clip refused it.
    /// Either way no result is in the view any more.
    pub(super) fn kept(&mut self, r: Result<u32, mpfree_seq::ModelError>) {
        if let Err(e) = &r {
            self.counts.keeps_refused += 1;
            self.words = Some(Refusal::Model(*e).words().to_string());
        }
        // Kept: the launch stays as the result had it, in time with the
        // kept clip.
        if r.is_ok() {
            self.origin = None;
        }
        self.preview.kept(r);
        self.sync_made();
        self.in_view = None;
        self.line = None;
    }

    /// Back. False while Keep is under way.
    pub(super) fn back(&mut self) -> bool {
        if !self.preview.cancel() {
            return false;
        }
        self.waiting = None;
        self.sync_made();
        self.words = None;
        true
    }

    /// After a change to the project: if the bound clip, or a clip a
    /// result listened to, changed or went, the preview is invalidated.
    /// True if it was; the caller compiles the clip as it now is.
    pub(super) fn follow(&mut self, project: &Project) -> bool {
        if !matches!(
            self.preview.state(),
            State::Preparing | State::Ready | State::Auditioning
        ) {
            return false;
        }
        if self.preview.clip_changed(project).is_none() {
            return false;
        }
        self.counts.invalidated += 1;
        self.waiting = None;
        self.sync_made();
        self.words = Some(changed_words());
        true
    }

    /// REC while a result plays: it stops at once and stays ready. True if
    /// one was playing.
    pub(super) fn stop_for_rec(&mut self) -> bool {
        if self.in_view.is_none() {
            return false;
        }
        self.preview.stop();
        self.counts.stopped_by_rec += 1;
        self.in_view = None;
        self.line = None;
        true
    }

    /// Continue as the screens show it, while one is under way.
    pub(super) fn screen(&self) -> Option<Arc<Continuing>> {
        let state = self.preview.state();
        let words = self.words.clone();
        if state == State::Idle && words.is_none() {
            return None;
        }
        let (track, slot) = self
            .bound()
            .or_else(|| self.waiting.as_ref().map(|w| (w.1.track, w.1.slot)))
            .unwrap_or_default();
        let shown = self.shown();
        Some(Arc::new(Continuing {
            track,
            slot,
            state,
            made: shown.map(|m| m.made.clone()),
            clip: shown.map(|m| m.clip.clone()),
            new_from: shown.map_or(0, |m| m.new_from),
            position: self.preview.position(),
            heard: self.in_view.is_some(),
            line: self.line.map(|l| l.line),
            words,
            settings: self.settings.unwrap_or_default(),
            tries: self
                .made
                .iter()
                .map(|m| Try {
                    style: m.style,
                    bars: m.bars,
                    again: m.again,
                })
                .collect(),
        }))
    }
}

/// What a try made of part of the one before it is, for the tries list.
fn again_of(how: &Making) -> Option<Again> {
    let n = |l: usize| u16::try_from(l).unwrap_or(u16::MAX);
    match how {
        Making::New | Making::Keeping { .. } => None,
        Making::Bars { bars, .. } if bars.len() == 1 => Some(Again::Bar(bars[0])),
        Making::Bars { bars, .. } => Some(Again::Bars(n(bars.len()))),
        Making::Notes { notes, .. } => Some(Again::Notes(n(notes.len()))),
    }
}

/// "4 more bars of riff".
pub(super) fn more_bars(bars: u8, name: &str) -> String {
    match bars {
        1 => format!("1 more bar of {name}"),
        n => format!("{n} more bars of {name}"),
    }
}

fn changed_words() -> String {
    "The clip changed: try again?".to_string()
}
