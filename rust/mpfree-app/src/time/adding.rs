//! Add a part in the time engine (composing tools S2.6): a part made for
//! a row's empty cell, heard there with the row from the next bar line,
//! kept as one take (with any chords picked on the panel) that UNDO takes
//! back, or left with Back. And the row's chords outside the panel: a
//! chord set is one take, the kept clips it leaves off the chords are
//! marked, and Fit moves the ones asked as one take.
//!
//! As with Continue (`compose.rs`), the composer never runs on the time
//! engine's threads: an ask copies just the row's clips ([`AddPartJob`])
//! to a thread of its own, and the answer is taken on the control half's
//! next poll. The control half (`TimeControl`'s Add a part section) does
//! what each step asks of the play view:
//!
//! - a try in while the row plays: it comes in on the next bar line, its
//!   tick 0 where the row's clips started, so it lines up with the row
//!   as the reading heard it; while the row doesn't play it waits, and
//!   comes in when the row does;
//! - ‹ › or Try again: the new try on the next bar line, the last one
//!   playing until then;
//! - chords picked on the panel: every try's notes move to fit them at
//!   once (as Fit moves a kept clip's), and the next try is made over
//!   them;
//! - Back: the cell quiet again on the next bar line;
//! - Keep: the try written in its cell, with the panel's chords, as one
//!   take ([`mpfree_play::Takes::add_part`]); the track's launch moves to
//!   it, so nothing is heard to change;
//! - a change to a clip the reading heard, or to the cell (an edit, a
//!   recording, UNDO): the tries are dropped and the cell is quiet at
//!   once. REC stops a try playing at once.
//!
//! What gave way is counted for the status line ([`AddCounts`]).

use std::sync::mpsc::TryRecvError;
use std::sync::Arc;

use mpfree_compose::{
    fit_clip, marks, revision_in, AddAsk, AddDone, AddPartJob, InstrumentKind, PartMade, RowChords,
};
use mpfree_context::chord::ChordTimeline;
use mpfree_context::harmony::RowHarmony;
use mpfree_context::{PartRole, ReadBudget};
use mpfree_seq::{row_harmony, scene_length, Clip, Holds, Project};
use mpfree_time_ui::adding::{AddState, Adding as AddingScreen, PartAsk, Unfit};
use mpfree_transport::time::{Meter, Swing, Tick};

use super::compose::{LineWait, Worker};

/// How many tries the panel keeps, as Continue keeps its results.
const HISTORY: usize = mpfree_compose::preview::HISTORY;

/// What Add a part gave way on, since the start. The status line says the
/// ones that aren't 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AddCounts {
    /// Add a part and Try again asked.
    pub asked: u64,
    /// Tries that came in.
    pub made: u64,
    /// Asks the composer refused (nothing to answer, no drum pads).
    pub refused: u64,
    /// Tries a newer ask, or Back, had overtaken: dropped unheard.
    pub late: u64,
    /// Tries made over clips that changed while they were being made, or
    /// dropped because a clip they heard changed later.
    pub stale: u64,
    /// Readings that left a clip out (gone, or past the reading's budget).
    pub left_out: u64,
    /// Keeps refused (the cell filled, a clip changed, the clip's limits).
    pub keeps_refused: u64,
    /// Tries REC stopped while they played.
    pub stopped_by_rec: u64,
    /// Kept clips Fit moved to new chords.
    pub fitted: u64,
}

impl AddCounts {
    /// For the status line: None while nothing was asked.
    pub fn describe(&self) -> Option<String> {
        if self.asked == 0 && self.fitted == 0 {
            return None;
        }
        let mut line = format!("add a part: {} asked, {} made", self.asked, self.made);
        for (n, what) in [
            (self.refused, "refused"),
            (self.late, "overtaken"),
            (self.stale, "made over clips that changed"),
            (self.left_out, "clips left out of readings"),
            (self.keeps_refused, "keeps refused"),
            (self.stopped_by_rec, "stopped by REC"),
            (self.fitted, "clips fitted to new chords"),
        ] {
            if n > 0 {
                line.push_str(&format!(", {n} {what}"));
            }
        }
        Some(line)
    }
}

/// One try, as the panel keeps it.
struct Tried {
    /// Its own number, never reused: what the play view plays is named by
    /// it.
    id: u32,
    made: Arc<PartMade>,
    /// The part as heard: the try's clip, its notes moved to the panel's
    /// chords when chords were picked after it was made.
    clip: Arc<Clip>,
    /// The chords it plays over now, over the row's length.
    chords: ChordTimeline,
}

/// A try in the play view, in its empty cell: on New track (S2.6b), in
/// the cell of a track the view has and the project hasn't yet, named
/// `new_track`.
#[derive(Clone, Debug)]
pub(super) struct AddInView {
    pub id: u32,
    pub track: usize,
    pub row: usize,
    pub clip: Arc<Clip>,
    pub new_track: Option<String>,
}

/// What Keep writes.
pub(super) struct Keeping {
    pub track: usize,
    pub row: usize,
    pub clip: Clip,
    /// New track: the track Keep adds with the part, named so; it gets
    /// index `track`.
    pub new_track: Option<String>,
    /// The chords to put on the row with the part, if any.
    pub line: Option<ChordTimeline>,
    /// The chords the part was made over, as the row was heard.
    pub heard: ChordTimeline,
}

/// What an ask needs from the time engine besides the panel's choices.
pub(super) struct Hearing {
    /// The row's other clips, for the reading.
    pub parts: Vec<mpfree_compose::ToRead>,
    pub swing: Swing,
    pub transport_swing: Swing,
    pub meter: Meter,
    pub bpm: f64,
}

/// Add a part's side of the time engine's control half.
pub(super) struct Adding {
    worker: Option<Worker<AddPartJob, AddDone>>,
    /// The panel's last ask: from the first ask until Keep or Back.
    asked: Option<PartAsk>,
    tries: Vec<Tried>,
    /// The shown try, an index into `tries`.
    shown: usize,
    /// The request a try is being made for.
    waiting: Option<u32>,
    requests: u32,
    next_id: u32,
    /// Asks so far: each one's attempt, so each sounds new.
    attempts: u16,
    /// Chords picked on the panel and not kept yet.
    pub(super) picked: Option<ChordTimeline>,
    /// The try the play view sent last plays in its cell, or None.
    pub(super) in_view: Option<AddInView>,
    /// The change waiting for its bar line.
    pub(super) line: Option<LineWait>,
    /// Why nothing can be heard or kept, for the screens.
    words: Option<String>,
    pub(super) counts: AddCounts,
    /// Hearing a row's chords for the screens: its own thread, so a
    /// hearing never overtakes a try being made.
    hear_worker: Option<Worker<AddPartJob, RowChords>>,
    /// The row the screens asked to hear.
    pub(super) hearing: Option<usize>,
    /// What the last hearing was asked on: the row and its clips' and
    /// chords' fingerprint, so it is asked again only when they change.
    heard_on: Option<(usize, u64)>,
    hear_waiting: Option<(u32, usize)>,
    /// The row heard last and its chords.
    pub(super) heard: Option<(usize, Arc<RowChords>)>,
}

impl Adding {
    pub(super) fn new() -> Adding {
        Adding {
            worker: None,
            asked: None,
            tries: Vec::new(),
            shown: 0,
            waiting: None,
            requests: 0,
            next_id: 0,
            attempts: 0,
            picked: None,
            in_view: None,
            line: None,
            words: None,
            counts: AddCounts::default(),
            hear_worker: None,
            hearing: None,
            heard_on: None,
            hear_waiting: None,
            heard: None,
        }
    }

    /// Hear row `row` for the screens, unless its clips and chords are as
    /// when it was last asked. `hearing` is what the reading needs; its
    /// parts are the row's clips.
    pub(super) fn hear(
        &mut self,
        project: &Project,
        row: usize,
        hearing: Hearing,
    ) -> Result<(), String> {
        let print = fingerprint(project, row, &hearing.parts);
        if self.heard_on == Some((row, print)) {
            return Ok(());
        }
        let job = hear_job(project, row, hearing);
        if self.hear_worker.is_none() {
            self.hear_worker = Some(Worker::start("chords", AddPartJob::hear)?);
        }
        self.requests = self.requests.wrapping_add(1);
        let n = self.requests;
        let worker = self.hear_worker.as_ref().expect("started above");
        if worker.jobs.send((n, Box::new(job))).is_err() {
            self.hear_worker = None;
            return Err("chords: the composer stopped".into());
        }
        self.heard_on = Some((row, print));
        self.hear_waiting = Some((n, row));
        Ok(())
    }

    /// Stop hearing: the screens no longer show a row's chords.
    pub(super) fn stop_hearing(&mut self) {
        self.hearing = None;
        self.heard_on = None;
        self.hear_waiting = None;
        self.heard = None;
    }

    /// Take the hearing thread's answers. True if the row's chords came.
    pub(super) fn heard_answers(&mut self) -> bool {
        let Some(w) = &self.hear_worker else {
            return false;
        };
        let mut changed = false;
        loop {
            match w.done.try_recv() {
                Ok((n, Some(line))) => {
                    if let Some((m, row)) = self.hear_waiting {
                        if m == n {
                            self.hear_waiting = None;
                            if self.hearing == Some(row) {
                                self.heard = Some((row, Arc::new(*line)));
                                changed = true;
                            }
                        }
                    }
                }
                Ok((_, None)) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.hear_worker = None;
                    self.heard_on = None;
                    break;
                }
            }
        }
        changed
    }

    /// The panel is open, or a try is still in the view.
    pub(super) fn active(&self) -> bool {
        self.asked.is_some() || self.in_view.is_some()
    }

    /// The cell a part is under way for: (track, row).
    pub(super) fn target(&self) -> Option<(usize, usize)> {
        self.asked.as_ref().map(|a| (a.track, a.row))
    }

    /// New track's name, while the part is under way for a track Keep
    /// adds (S2.6b).
    fn new_track(&self) -> Option<&str> {
        self.asked
            .as_ref()
            .and_then(|a| a.new_track.as_ref())
            .map(|n| n.name.as_str())
    }

    fn shown_try(&self) -> Option<&Tried> {
        self.tries.get(self.shown)
    }

    /// Ask for a try: the job goes to the composer's thread. A different
    /// cell than the last ask's starts the tries again. The request's
    /// number.
    pub(super) fn ask(
        &mut self,
        project: &Project,
        ask: PartAsk,
        hearing: Hearing,
    ) -> Result<u32, String> {
        // An empty cell, or a blank clip that makes way at Keep (the
        // Sequencer's empty clip, S2.6); on New track, the track Keep will
        // add (S2.6b).
        let empty = match ask.new_track {
            Some(_) => project.tracks.len() == ask.track,
            None => project
                .tracks
                .get(ask.track)
                .and_then(|t| t.slots.get(ask.row))
                .is_some_and(|s| s.as_ref().is_none_or(mpfree_seq::Clip::is_blank)),
        };
        if !empty {
            return Err(format!(
                "add a part: row {}'s cell on track {} isn't empty",
                ask.row + 1,
                ask.track + 1
            ));
        }
        // Another cell, or New track asked over a track (or a track over
        // New track): the tries start again.
        let was_new = self.new_track().is_some();
        if self.target().is_some_and(|t| t != (ask.track, ask.row))
            || (self.asked.is_some() && was_new != ask.new_track.is_some())
        {
            self.tries.clear();
            self.shown = 0;
            self.picked = None;
        }
        let job = AddAsk {
            row: ask.row,
            parts: hearing.parts,
            lanes: ask.lanes.clone(),
            part: ask.part,
            relation: ask.relation,
            kind: ask.kind,
            pads: ask.pads,
            swing: hearing.swing,
            transport_swing: hearing.transport_swing,
            meter: hearing.meter,
            bpm: hearing.bpm,
            budget: ReadBudget::default(),
            settings: ask.settings,
            bars: ask.bars,
            attempt: self.attempts,
            phrase: ask.phrase.as_ref().map(|(_, p)| *p),
        };
        self.attempts = self.attempts.wrapping_add(1);
        let mut job = AddPartJob::new(project, job);
        if let Some(line) = &self.picked {
            job = job.with_line(line.clone());
        }
        if self.worker.is_none() {
            self.worker = Some(Worker::start("add a part", AddPartJob::run)?);
        }
        self.requests = self.requests.wrapping_add(1);
        let n = self.requests;
        let worker = self.worker.as_ref().expect("started above");
        if worker.jobs.send((n, Box::new(job))).is_err() {
            // The thread is gone: start another next time.
            self.worker = None;
            return Err("add a part: the composer stopped: ask again".into());
        }
        self.counts.asked += 1;
        self.asked = Some(ask);
        self.waiting = Some(n);
        self.words = None;
        Ok(n)
    }

    /// Whether the clips `made` heard are all as they were, and its cell
    /// still empty (or a blank clip, which makes way at Keep). For New
    /// track (`new`), while the project's tracks are as many as when it
    /// was asked: a track added or taken away since would make Keep add
    /// the track at another index than the one its synth plays.
    fn fresh(project: &Project, made: &PartMade, track: usize, row: usize, new: bool) -> bool {
        let empty = if new {
            project.tracks.len() == track
        } else {
            project
                .tracks
                .get(track)
                .and_then(|t| t.slots.get(row))
                .is_some_and(|s| s.as_ref().is_none_or(mpfree_seq::Clip::is_blank))
        };
        empty
            && made
                .read
                .iter()
                .all(|r| revision_in(project, r.clip) == Some(r.revision))
    }

    /// Take the composer's answers, each checked against the project as
    /// it is now. True if anything changed for the screens; a line for the
    /// log about each try that came in or was refused.
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
        let new = self.new_track().is_some();
        if gone {
            self.worker = None;
            if self.waiting.take().is_some() {
                self.words = Some("The composer stopped: ask again.".into());
                log.push("add a part: the composer stopped".into());
                changed = true;
            }
        }
        for (n, done) in got {
            let (Some(done), true) = (done, self.waiting == Some(n)) else {
                self.counts.late += 1;
                continue;
            };
            self.waiting = None;
            changed = true;
            self.counts.left_out += done.left_out as u64;
            let Some((track, row)) = self.target() else {
                self.counts.late += 1;
                continue;
            };
            match done.result {
                Err(r) => {
                    self.counts.refused += 1;
                    self.words = Some(r.words());
                    log.push(format!("add a part: nothing made: {}", r.words()));
                }
                Ok(made) if !Self::fresh(project, &made, track, row, new) => {
                    self.counts.stale += 1;
                    self.words = Some(Self::stale_words(project, track, new).into());
                }
                Ok(made) => {
                    self.counts.made += 1;
                    if self.tries.len() == HISTORY {
                        self.tries.remove(0);
                    }
                    self.next_id = self.next_id.wrapping_add(1);
                    let name = made.clip.name.clone();
                    self.tries.push(Tried {
                        id: self.next_id,
                        clip: Arc::new(made.clip.clone()),
                        chords: made.chords.clone(),
                        made: Arc::new(made),
                    });
                    self.shown = self.tries.len() - 1;
                    log.push(format!("add a part: {name} ready for row {}", row + 1));
                }
            }
        }
        changed
    }

    /// A clip a try heard, or its cell, changed (an edit, a recording,
    /// UNDO): the tries go, and the cell is quiet at once. True if one
    /// was dropped.
    pub(super) fn follow(&mut self, project: &Project) -> bool {
        let Some((track, row)) = self.target() else {
            return false;
        };
        let new = self.new_track().is_some();
        let before = self.tries.len();
        self.tries
            .retain(|t| Self::fresh(project, &t.made, track, row, new));
        if self.tries.len() == before {
            return false;
        }
        self.counts.stale += (before - self.tries.len()) as u64;
        self.shown = self.tries.len().saturating_sub(1);
        if self
            .in_view
            .as_ref()
            .is_some_and(|v| !self.tries.iter().any(|t| t.id == v.id))
        {
            self.in_view = None;
            self.line = None;
        }
        self.words = Some(Self::stale_words(project, track, new).into());
        true
    }

    /// Why the tries went: New track's tracks changed (a + Track, UNDO),
    /// or a clip a try heard did.
    fn stale_words(project: &Project, track: usize, new: bool) -> &'static str {
        match project.tracks.len() {
            n if new && n > track => "A track was added: Try again",
            n if new && n < track => "A track was taken away: Try again",
            _ => "A clip it listened to changed. Try again?",
        }
    }

    /// What the play view should play in the cell once the next change
    /// lands: the shown try while the panel is open (not while recording);
    /// None, the cell quiet.
    pub(super) fn wanted(&self, recording: bool) -> Option<u32> {
        if recording || self.asked.is_none() {
            return None;
        }
        self.shown_try().map(|t| t.id)
    }

    pub(super) fn in_view_id(&self) -> Option<u32> {
        self.in_view.as_ref().map(|v| v.id)
    }

    /// The cell whose music changes next: the panel's, or the one a try
    /// still plays in after Back.
    pub(super) fn changing(&self) -> Option<(usize, usize)> {
        self.target()
            .or_else(|| self.in_view.as_ref().map(|v| (v.track, v.row)))
    }

    /// The change landed: the play view plays try `want` in its cell, or
    /// nothing there.
    pub(super) fn landed(&mut self, want: Option<u32>) {
        self.line = None;
        let new_track = self.new_track().map(str::to_string);
        self.in_view = match (want, self.target()) {
            (Some(id), Some((track, row))) => {
                self.tries.iter().find(|t| t.id == id).map(|t| AddInView {
                    id,
                    track,
                    row,
                    clip: t.clip.clone(),
                    new_track,
                })
            }
            _ => None,
        };
    }

    /// ‹ (`back`) or ›: the try before or after the shown one. False if
    /// there is none.
    pub(super) fn step(&mut self, back: bool) -> bool {
        let to = if back {
            self.shown.checked_sub(1)
        } else {
            Some(self.shown + 1).filter(|&i| i < self.tries.len())
        };
        match to {
            Some(i) => {
                self.shown = i;
                true
            }
            None => false,
        }
    }

    /// Chords picked on the panel: every try's notes move to fit them (a
    /// drum part stays as it is), the next try is made over them, and
    /// Keep keeps them. A try in the view plays its fitted notes at once.
    pub(super) fn pick_line(&mut self, line: ChordTimeline) {
        let Some((part, kind)) = self.asked.as_ref().map(|a| (a.part, a.kind)) else {
            return;
        };
        for t in &mut self.tries {
            let length = t.chords.len();
            let old = RowHarmony {
                line: t.chords.clone(),
                length,
                suggested: false,
            };
            let new = RowHarmony {
                line: line.looped(length),
                length,
                suggested: false,
            };
            if let Ok(clip) = fit_clip(&t.clip, part, kind, &old, &new, None, 0) {
                t.clip = Arc::new(clip);
            }
            t.chords = new.line;
        }
        if let Some(v) = &mut self.in_view {
            if let Some(t) = self.tries.iter().find(|t| t.id == v.id) {
                v.clip = t.clip.clone();
            }
        }
        self.picked = Some(line);
    }

    /// Keep: the shown try for its cell, with the chords to put on the
    /// row (the panel's, or the ones suggested for a row with none), and
    /// the chords it was made over (the row's as heard). Not over clips
    /// that changed.
    pub(super) fn keep(&self, project: &Project) -> Result<Keeping, String> {
        let (track, row) = self
            .target()
            .ok_or_else(|| "add a part: nothing to keep".to_string())?;
        let t = self
            .shown_try()
            .ok_or_else(|| "add a part: nothing made yet".to_string())?;
        let new_track = self.new_track().map(str::to_string);
        if !Self::fresh(project, &t.made, track, row, new_track.is_some()) {
            return Err(match new_track {
                Some(_) => "add a part: the tracks changed since the try: Try again".into(),
                None => "add a part: a clip it listened to changed: Try again".into(),
            });
        }
        Ok(Keeping {
            track,
            row,
            clip: (*t.clip).clone(),
            new_track,
            line: self.picked.clone().or_else(|| t.made.line.clone()),
            heard: t.made.chords.clone(),
        })
    }

    /// Keep went in (`Ok`) or was refused.
    pub(super) fn kept(&mut self, r: &Result<u32, mpfree_seq::ModelError>) {
        match r {
            Ok(_) => self.close(),
            Err(e) => {
                self.counts.keeps_refused += 1;
                self.words = Some(format!("Keep was refused ({e:?}): nothing written."));
            }
        }
    }

    /// Close the panel: the tries and the picked chords go. A try in the
    /// view plays on until the change lands.
    fn close(&mut self) {
        self.asked = None;
        self.tries.clear();
        self.shown = 0;
        self.waiting = None;
        self.picked = None;
        self.words = None;
    }

    /// Back: nothing written; the cell goes quiet on the next bar line.
    pub(super) fn back(&mut self) {
        self.close();
    }

    /// REC: a try playing stops at once. True if one did.
    pub(super) fn stop_for_rec(&mut self) -> bool {
        if self.in_view.take().is_none() {
            return false;
        }
        self.line = None;
        self.counts.stopped_by_rec += 1;
        true
    }

    /// Add a part as the screens show it.
    pub(super) fn screen(&self) -> Option<Arc<AddingScreen>> {
        let a = self.asked.as_ref()?;
        let shown = self.shown_try();
        let playing = shown.is_some_and(|t| self.in_view_id() == Some(t.id));
        let state = if self.waiting.is_some() {
            AddState::Making
        } else if playing {
            AddState::Playing
        } else if shown.is_some() {
            AddState::Ready
        } else {
            AddState::Nothing
        };
        let made = shown.map(|t| {
            let mut m = (*t.made).clone();
            m.clip = (*t.clip).clone();
            m.chords = t.chords.clone();
            Arc::new(m)
        });
        Some(Arc::new(AddingScreen {
            row: a.row,
            track: a.track,
            state,
            made,
            position: shown.map(|_| (self.shown + 1, self.tries.len())),
            line: self.line.map(|w| w.line),
            picked: self.picked.clone(),
            words: self.words.clone(),
            part: a.part,
            relation: a.relation,
        }))
    }
}

/// A fingerprint of what hearing row `row` reads: each clip it hears at
/// its revision, and the row's own chords.
/// A job that hears row `row`'s chords: the reading's ask with what
/// `hearing` says, on the row's clips (the screens' strip, and the Song
/// view's Chords lane, `song_chords.rs`).
pub(super) fn hear_job(project: &Project, row: usize, hearing: Hearing) -> AddPartJob {
    let ask = AddAsk {
        row,
        parts: hearing.parts,
        lanes: mpfree_compose::Lanes::default(),
        part: PartRole::Melody,
        relation: mpfree_compose::Relation::Support,
        kind: InstrumentKind::OneNote,
        pads: [None; 4],
        swing: hearing.swing,
        transport_swing: hearing.transport_swing,
        meter: hearing.meter,
        bpm: hearing.bpm,
        budget: ReadBudget::default(),
        settings: mpfree_compose::Settings::default(),
        bars: None,
        attempt: 0,
        phrase: None,
    };
    AddPartJob::new(project, ask)
}

/// Row `row`'s clips' and chords' fingerprint: a hearing is asked again
/// only when it changes.
pub(super) fn fingerprint(project: &Project, row: usize, parts: &[mpfree_compose::ToRead]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        p.track.hash(&mut h);
        if let Some(c) = project.clip(p.track, p.slot) {
            mpfree_compose::revision(c).0.hash(&mut h);
        }
    }
    if let Some(s) = project.scene(row) {
        format!("{:?}", s.chords).hash(&mut h);
    }
    h.finish()
}

/// What a clip is in the music, for Fit and its marks: drums for pad
/// hits, else what the track's instrument plays.
pub(super) fn role_of(clip: &Clip, kind: InstrumentKind) -> PartRole {
    if clip.holds == Holds::Hits {
        return PartRole::Drums;
    }
    match kind {
        InstrumentKind::Bass => PartRole::Bass,
        InstrumentKind::Chords => PartRole::Chords,
        InstrumentKind::Percussive => PartRole::Drums,
        InstrumentKind::OneNote | InstrumentKind::Chordal => PartRole::Melody,
    }
}

/// The rows whose chords changed since their kept clips fitted them:
/// `fitted` holds each such row's chords as they were (as the time engine
/// keeps them), `bar` the bar's length, and `kind` what each track plays. A row whose clips all
/// still fit is left out.
pub(super) fn unfit(
    project: &Project,
    fitted: &[(usize, ChordTimeline)],
    bar: Tick,
    kind: impl Fn(usize, usize) -> InstrumentKind,
) -> Vec<Unfit> {
    fitted
        .iter()
        .filter_map(|(row, before)| {
            let new = row_harmony(project, *row, bar)?;
            let old = RowHarmony {
                line: before.clone(),
                length: scene_length(project, *row, bar),
                suggested: false,
            };
            let clips: Vec<(usize, usize)> = project
                .tracks
                .iter()
                .enumerate()
                .filter_map(|(t, tr)| {
                    let c = tr.slots.get(*row)?.as_ref()?;
                    if c.looped.is_some() {
                        return None;
                    }
                    let n = marks(c, role_of(c, kind(t, *row)), &old, &new).len();
                    (n > 0).then_some((t, n))
                })
                .collect();
            (!clips.is_empty()).then(|| Unfit {
                row: *row,
                before: before.clone(),
                clips,
            })
        })
        .collect()
}

/// The marked clips of row `row` on `tracks`, fitted from `before` to the
/// row's chords now, for one Fit take. A clip that can't be fitted (drums,
/// passes that meet different chords) is left out, with why.
pub(super) fn fitted_clips(
    project: &Project,
    row: usize,
    before: &ChordTimeline,
    tracks: &[usize],
    bar: Tick,
    kind: impl Fn(usize, usize) -> InstrumentKind,
) -> (Vec<(usize, usize, Clip)>, Vec<String>) {
    let Some(new) = row_harmony(project, row, bar) else {
        return (Vec::new(), vec!["the row has no chords".into()]);
    };
    let old = RowHarmony {
        line: before.clone(),
        length: scene_length(project, row, bar),
        suggested: false,
    };
    let mut out = Vec::new();
    let mut why = Vec::new();
    for &t in tracks {
        let Some(c) = project.clip(t, row) else {
            continue;
        };
        let k = kind(t, row);
        let role = role_of(c, k);
        if marks(c, role, &old, &new).is_empty() {
            continue;
        }
        match fit_clip(c, role, k, &old, &new, None, 0) {
            Ok(fitted) => out.push((t, row, fitted)),
            Err(r) => why.push(format!("{}: {}", c.name, r.words())),
        }
    }
    (out, why)
}
