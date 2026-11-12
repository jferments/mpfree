//! The time engine's control half: the project and everything that edits
//! it. The take history, the launches and cues, scenes and follow
//! actions, live to song, the note editor, the morph's settings, the
//! selection, the armed track, CAPTURE's history and the move log, the
//! steps for the Play page, every compile of the play view, and what the
//! screens show.
//!
//! It runs at its own pace and may run a few ms behind the audio half:
//! it hears from it only through its queue (see `link`). Each period the
//! audio half sends the notes the pads played, the gestures, the cues
//! that started, the recorded changes, the period's spans and the
//! transport as the period left it. [`TimeControl::poll`] takes them in
//! order and does each period's bookkeeping on a copy of the period's
//! spans: CAPTURE's history and the move log take the notes and gestures
//! (as they did on the audio thread, so they keep the same phrase), live
//! to song follows the cues, started cues become launches, follow actions
//! coming due are cued, and recorded changes go into the clips and the
//! take history. A compile sends the view, which says what it includes.

use mpfree_capture::{keep, moves_for_clip, Capture, Now, Pending};
use mpfree_perform::{Perform, SceneMorph, SongRecorder, When};
use mpfree_play::{Cue, Includes, Launch, PlayView, Started, Takes};
use mpfree_play::{MOVE_POINTS, TOUCH_POINTS};
use mpfree_seq::arrange::{scene_length, scenes_to_song, song_to_scenes};
use mpfree_seq::mpfree_moves::{Kind, MoveLog};
use mpfree_seq::steps::Grid;
use mpfree_seq::{Clip, ModelError, Project, TouchPoint, SLOTS};
use mpfree_time_ui::editor::{EditAction, Editor};
use mpfree_time_ui::view::{
    bars_label, clips_after, AudioAction, EditInfo, MorphInfo, RegionId, TimeAction, TimeState,
    TransportInfo,
};
use mpfree_transport::time::Tick;
use mpfree_transport::{Block, Command, State};

use super::link::{Counts, PeriodEnd, Rx, ToAudio, ToControl, Tracks, Tx};
use super::{take_name, StepAction, StepClip, StepsData, NEW_CLIP_BARS, RECORD_ROOM, STEP_TICKS};

/// Started cues one period may bring before the list grows.
const STARTED_ROOM: usize = 64;

pub struct TimeControl {
    pub(super) project: Project,
    pub(super) launches: Vec<Launch>,
    /// Launches and stops waiting for the next bar line.
    pub(super) cues: Vec<Cue>,
    /// Scenes and follow actions: cues the rows and chains clips and
    /// scenes on their bar lines.
    perform: Perform,
    /// The morph bar, and the morph on the mixer's mod engine
    /// ([`TimeControl::morph`]).
    morph: MorphInfo,
    /// Song mode: the arrangement plays on the tracks with no clip
    /// launched.
    song: bool,
    /// The Arrange page shows the Song view, and the placement selected
    /// on it.
    song_view: bool,
    selected: Option<usize>,
    /// The audio region picked on the Song view; it and `selected` are
    /// never both set (`TimeState::pick`).
    region: Option<RegionId>,
    /// Live to song: records what the grid plays into the song.
    song_rec: SongRecorder,
    /// What happened to the last recording when the transport stopped,
    /// for the log ([`TimeControl::take_song_note`]).
    song_note: Option<String>,
    scene_morph: SceneMorph,
    /// The note editor on the Arrange page, while it is open.
    editor: Option<Editor>,
    pub(super) armed: Option<usize>,
    takes: Takes,
    /// CAPTURE's history and the move log, fed the audio half's notes and
    /// gestures with the spans they were played in.
    capture: Box<Capture>,
    move_log: Box<MoveLog>,
    /// What CAPTURE would keep, as the screen last showed it.
    pending: Option<Pending>,
    /// The transport as the audio half's last period left it.
    info: TransportInfo,
    /// The audio half's counts, as of its last period.
    counts: Counts,
    /// The current period's spans, copied from the audio half's.
    block: Box<Block>,
    /// The next span starts a new period's copy.
    fresh: bool,
    spans_lost: u64,
    /// The current period's started cues.
    started: Vec<Started>,
    /// The points for the next recorded change.
    touch: Vec<TouchPoint>,
    moves: Vec<(f64, f32, Kind)>,
    /// A recorded change made in this period changed a clip.
    recorded_now: bool,
    /// The audio half asked for a compile (a view without room or too
    /// late to catch up).
    compile_asked: bool,
    /// The clips, launches or takes changed since [`TimeControl::take_changed`].
    changed: bool,
    /// Recording changed the clips and takes since the screens last heard.
    recorded: bool,
    /// Frames since `changed` was last set for a recording.
    since_told: u64,
    /// Recorded changes are told this often, in frames (a quarter second).
    tell_every: u64,
    /// The steps changed since [`TimeControl::take_steps_changed`].
    steps_changed: bool,
    /// Recorded pressure a clip had no room for (`MAX_TOUCH_POINTS`).
    touch_lost: u64,
    /// What the views include: the recorded changes made so far, and the
    /// started cues applied on each track.
    pub(super) includes: Includes,
    tx: Tx<ToAudio>,
    rx: Rx<ToControl>,
    /// A view the audio half's full queue handed back, sent again on the
    /// next poll unless a newer one replaces it.
    unsent: Option<Box<PlayView>>,
    /// The tracks' settings the audio half has.
    tracks_sent: Tracks,
}

impl TimeControl {
    /// Editing `project` with track 1 armed, the transport as `info`
    /// says, sending to the audio half on `tx`, which already has the
    /// tracks' settings `tracks`. Compiles the first view.
    pub fn new(
        project: Project,
        info: TransportInfo,
        rate: u32,
        tx: Tx<ToAudio>,
        rx: Rx<ToControl>,
        tracks: Tracks,
    ) -> Result<TimeControl, String> {
        let mut c = TimeControl {
            project,
            launches: Vec::new(),
            cues: Vec::new(),
            perform: Perform::default(),
            morph: MorphInfo::default(),
            song: false,
            song_view: false,
            selected: None,
            region: None,
            song_rec: SongRecorder::default(),
            song_note: None,
            scene_morph: SceneMorph::default(),
            editor: None,
            armed: tracks.armed,
            takes: Takes::default(),
            capture: Box::new(Capture::new()),
            move_log: Box::new(MoveLog::new()),
            pending: None,
            info,
            counts: Counts::default(),
            block: Box::new(Block::new()),
            fresh: true,
            spans_lost: 0,
            started: Vec::with_capacity(STARTED_ROOM),
            touch: Vec::with_capacity(TOUCH_POINTS),
            moves: Vec::with_capacity(MOVE_POINTS),
            recorded_now: false,
            compile_asked: false,
            changed: true,
            recorded: false,
            since_told: 0,
            tell_every: u64::from(rate / 4),
            steps_changed: true,
            touch_lost: 0,
            includes: Includes::default(),
            tx,
            rx,
            unsent: None,
            tracks_sent: tracks,
        };
        c.compile()?;
        Ok(c)
    }

    /// The armed track (the one the pads, the arp panel and recording
    /// follow), if any.
    pub fn armed(&self) -> Option<usize> {
        self.armed
    }

    /// A view for the audio half; one its full queue hands back waits for
    /// the next poll.
    fn send_view(&mut self, v: Box<PlayView>) {
        self.unsent = None;
        if let Err(ToAudio::View(v)) = self.tx.send(ToAudio::View(v)) {
            self.unsent = Some(v);
        }
    }

    /// Send the tracks' settings if they changed.
    fn sync_tracks(&mut self) {
        let now = Tracks::of(&self.project, self.armed);
        if now != self.tracks_sent && self.tx.send(ToAudio::Tracks(now)).is_ok() {
            self.tracks_sent = now;
        }
    }

    /// Take everything the audio half sent, in order, and do each
    /// period's bookkeeping. Allocates only for an edit (a compile, a
    /// started cue, a recording kept into the song).
    pub fn poll(&mut self) {
        if let Some(v) = self.unsent.take() {
            self.send_view(v);
        }
        self.sync_tracks();
        while let Some(m) = self.rx.recv() {
            match m {
                ToControl::Played(offset, event) => self.capture.input(offset, event),
                ToControl::Moved(offset, input) => self.move_log.input(offset, input),
                ToControl::Started(st) => self.started.push(st),
                ToControl::Touch(points, n) => {
                    self.touch.extend_from_slice(&points[..usize::from(n)]);
                }
                ToControl::Points(points, n) => {
                    self.moves.extend_from_slice(&points[..usize::from(n)]);
                }
                ToControl::Rec(number, rec) => {
                    self.includes.recorded = number;
                    let done = self
                        .takes
                        .record(&mut self.project, &rec, &self.touch, &self.moves);
                    self.recorded_now |= done.changed;
                    self.touch_lost += done.touch_lost as u64;
                    self.touch.clear();
                    self.moves.clear();
                }
                ToControl::Compile => self.compile_asked = true,
                ToControl::Span(s) => {
                    if std::mem::take(&mut self.fresh) {
                        self.block.start_copy(0, 0);
                    }
                    if !self.block.copy_span(s) {
                        self.spans_lost += 1;
                    }
                }
                ToControl::Period(end) => self.period(end),
                ToControl::Done(v) => drop(v),
            }
        }
    }

    /// One of the audio half's periods ended: its bookkeeping, in the
    /// order the audio thread did it before the halves were split.
    fn period(&mut self, end: PeriodEnd) {
        if std::mem::take(&mut self.fresh) {
            self.block.start_copy(0, 0);
        }
        self.fresh = true;
        self.block.start_frame = end.start_frame;
        self.block.frames = end.frames;
        self.info = end.info;
        self.counts = end.counts;
        self.capture.process(&self.block);
        self.move_log.process(&self.block);
        // The Capture button shows what it would keep: a new phrase, or one
        // more bar of it, is a change for the screen (once a bar at most).
        let pending = self.capture.pending();
        if pending != self.pending {
            self.pending = pending;
            self.changed = true;
        }
        // Live to song follows the transport and the cues that started.
        let song_was = self.song_rec.recording();
        self.song_rec.period(&self.block, &self.started);
        // Cues that started on a bar line become the tracks' launches: the
        // view is compiled again.
        let mut compile = std::mem::take(&mut self.compile_asked);
        for i in 0..self.started.len() {
            let st = self.started[i];
            self.perform.started(&mut self.project, &st);
            st.apply(&mut self.launches, &mut self.cues);
            if let Some(n) = self.includes.switches.get_mut(st.track) {
                *n = n.wrapping_add(1);
            }
            compile = true;
        }
        self.started.clear();
        // Follow actions coming due are cued for their bar lines.
        if self.info.state.rolling() {
            let bar = self.info.meter.bar_ticks();
            compile |= self
                .perform
                .due(
                    &mut self.project,
                    &mut self.launches,
                    &mut self.cues,
                    self.info.position,
                    bar,
                )
                .changed();
        }
        let recorded = std::mem::take(&mut self.recorded_now);
        // The transport stopped: what live to song recorded goes into the
        // song (it compiles).
        if song_was && !self.song_rec.recording() && self.song_rec.waiting() {
            self.song_note = Some(match self.keep_song() {
                Ok(line) => format!("rec song: {line}"),
                Err(e) => e,
            });
        }
        if compile {
            // The clips only gained or lost notes and locks, and cues became
            // launches of clips they had compiled with, so this compiles.
            let _ = self.compile();
        } else {
            self.recorded |= recorded;
            self.since_told += u64::from(end.frames);
            if self.recorded && self.since_told >= self.tell_every {
                self.changed = true;
                self.recorded = false;
                self.since_told = 0;
            }
        }
    }

    /// Notes this half lost: past CAPTURE's input table. What passing
    /// between the halves lost is in [`TimeControl::describe`].
    pub fn dropped(&self) -> u64 {
        self.capture.dropped()
    }

    /// Messages this half lost on the way between the halves: to the
    /// audio half's full queue (a view among them is sent again), and
    /// spans past a period copy's room. Should stay 0.
    pub fn lost(&self) -> u64 {
        self.tx.full() + self.spans_lost
    }

    /// Recorded pressure a clip had no room for.
    pub fn touch_lost(&self) -> u64 {
        self.touch_lost
    }

    pub(super) fn compile(&mut self) -> Result<(), String> {
        // An undone capture takes its clip, and so its launch, away.
        let project = &self.project;
        self.launches
            .retain(|l| project.clip(l.track, l.slot).is_some());
        self.cues.retain(|c| {
            c.slot
                .is_none_or(|slot| project.clip(c.track, slot).is_some())
        });
        let v = self.view()?;
        // Room to record into the armed track's clips without allocating.
        if let Some(t) = self.armed {
            let slots = self
                .launches
                .iter()
                .filter(|l| l.track == t)
                .map(|l| l.slot);
            let cued = self
                .cues
                .iter()
                .filter(|c| c.track == t)
                .filter_map(|c| c.slot);
            for slot in slots.chain(cued) {
                if let Some(Some(c)) = self
                    .project
                    .tracks
                    .get_mut(t)
                    .and_then(|t| t.slots.get_mut(slot))
                {
                    c.notes.reserve(RECORD_ROOM);
                    c.locks.reserve(RECORD_ROOM);
                    c.touch.reserve(mpfree_play::TOUCH_ROOM);
                }
            }
        }
        self.sync_tracks();
        self.send_view(Box::new(v));
        self.compile_asked = false;
        self.changed = true;
        self.recorded = false;
        self.steps_changed = true;
        Ok(())
    }

    /// The view of the project as it is, with what it includes and room
    /// to record into in place.
    pub(super) fn view(&self) -> Result<PlayView, String> {
        let mut v = PlayView::compile_cued(&self.project, &self.launches, &self.cues)
            .map_err(|e| format!("clips: {e:?}"))?;
        v.set_armed(self.armed);
        v.set_song(&self.project, self.song)
            .map_err(|e| format!("song: {e:?}"))?;
        // A scene's mutes switch with its cues.
        self.perform.dress(&mut v);
        v.set_includes(self.includes);
        v.make_room(RECORD_ROOM, RECORD_ROOM);
        Ok(v)
    }

    /// "Clips through the arp" for `track` (see `Track::through_arp`).
    /// The track's arp follows from the next [`TimeHost::period_arps`],
    /// which clears it if it holds anything, so no note is left hanging.
    /// False if there is no such track.
    // The arp panel calls it (the arp thread's side); until then only
    // tests do.
    #[allow(dead_code)]
    pub fn set_through_arp(&mut self, track: usize, on: bool) -> bool {
        let Some(t) = self.project.tracks.get_mut(track) else {
            return false;
        };
        if t.through_arp != on {
            t.through_arp = on;
            self.changed = true;
            self.sync_tracks();
        }
        true
    }

    /// The clip the Play page's pattern edits: the armed track's (track 1
    /// if none is armed) launched clip, else its first clip.
    fn step_focus(&self) -> (usize, Option<usize>) {
        let track = self.armed.unwrap_or(0);
        let launched = self
            .launches
            .iter()
            .find(|l| l.track == track)
            .map(|l| l.slot);
        let first = || {
            self.project
                .tracks
                .get(track)?
                .slots
                .iter()
                .position(Option::is_some)
        };
        (track, launched.or_else(first))
    }

    /// What the Play page's pattern shows. Allocates: called on a change.
    pub fn steps(&self) -> StepsData {
        let (track, slot) = self.step_focus();
        let track_name = self
            .project
            .tracks
            .get(track)
            .map_or(String::new(), |t| t.name.clone());
        let grid = Grid::new(STEP_TICKS).expect("a sixteenth is a step size");
        let clip = slot.and_then(|slot| {
            let c = self.project.clip(track, slot)?;
            let mut pitches: Vec<u8> = (0..16).collect();
            for n in &c.notes {
                if !pitches.contains(&n.note) {
                    pitches.push(n.note);
                }
            }
            let origin = self
                .launches
                .iter()
                .find(|l| l.track == track && l.slot == slot)
                .map(|l| l.origin);
            Some(StepClip {
                slot,
                name: c.name.clone(),
                steps: grid.steps(c.length),
                song_length: c.speed.to_song(c.length as f64),
                song_step: c.speed.to_song(STEP_TICKS as f64),
                origin,
                rows: pitches.into_iter().map(|p| (p, grid.row(c, p))).collect(),
            })
        });
        StepsData {
            track,
            track_name,
            clip,
        }
    }

    /// True once after the steps the Play page shows changed.
    pub fn take_steps_changed(&mut self) -> bool {
        std::mem::take(&mut self.steps_changed)
    }

    /// A touch on the Play page's pattern, as one undoable edit (a
    /// Q-Link turned through many values joins one). A line for the log.
    pub fn step_action(&mut self, a: StepAction) -> Result<String, String> {
        let grid = Grid::new(STEP_TICKS).expect("a sixteenth is a step size");
        let (track, slot) = self.step_focus();
        let (slot, made) = match slot {
            Some(s) => (s, None),
            None => {
                let StepAction::Tap { .. } = a else {
                    return Ok(String::new());
                };
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t
                    .slots
                    .iter()
                    .position(Option::is_none)
                    .ok_or_else(|| format!("{} has no empty slot", t.name))?;
                let name = format!("Pattern {}", s + 1);
                t.slots[s] = Some(Clip::new(&name, grid.page_length()));
                let info = self.info;
                let launch = TimeAction::Launch { track, slot: s };
                (self.launches, self.cues) =
                    clips_after(&self.project, &self.launches, &self.cues, &launch, &info);
                (s, Some(name))
            }
        };
        let clip = self.project.clip(track, slot).ok_or("the clip is gone")?;
        let (edit, join, what) = match a {
            StepAction::Tap {
                pad,
                step,
                velocity,
            } => (
                grid.toggle(clip, pad, step, velocity),
                None,
                format!("step {} of pad {}", step + 1, pad + 1),
            ),
            StepAction::Change { pad, step, change } => (
                grid.change(clip, pad, step, change),
                Some(change.key(pad, step)),
                format!("step {} of pad {}: {change:?}", step + 1, pad + 1),
            ),
        };
        if edit.is_empty() && made.is_none() {
            return Ok(String::new());
        }
        self.takes
            .edit(&mut self.project, track, slot, &edit, join)
            .map_err(|e| format!("steps: {e:?}"))?;
        self.compile()?;
        let on = if edit.add.is_empty() { "off" } else { "on" };
        Ok(match (made, a) {
            (Some(name), _) => format!(
                "steps: new clip {name} in {}, {what} {on}",
                self.project.tracks[track].name
            ),
            (None, StepAction::Tap { .. }) => format!("steps: {what} {on}"),
            (None, _) => String::new(),
        })
    }

    /// A transport command for the audio half, from its next period on.
    /// False if its queue is full.
    pub fn command(&mut self, cmd: Command) -> bool {
        self.tx.send(ToAudio::Command(cmd)).is_ok()
    }

    /// A touch on the note editor. Open takes the clip the Play page's
    /// steps show (the armed track's); an edit is one undoable take (a
    /// row turned or the velocity lane dragged joins one).
    fn edit(&mut self, a: EditAction) -> Result<String, String> {
        if a == EditAction::Open {
            let (track, slot) = self.step_focus();
            let name = self
                .project
                .tracks
                .get(track)
                .map_or_else(String::new, |t| t.name.clone());
            let slot = slot.ok_or_else(|| format!("edit: {name} has no clip"))?;
            let clip = self.project.clip(track, slot).ok_or("edit: no such clip")?;
            let line = format!("edit: {} on {name}", clip.name);
            self.editor = Some(Editor::new(track, slot, clip));
            self.changed = true;
            return Ok(line);
        }
        let Some(mut ed) = self.editor.take() else {
            return Ok(String::new());
        };
        let Some(clip) = self.project.clip(ed.track, ed.slot) else {
            self.changed = true;
            return Err("edit: the clip is gone".into());
        };
        let done = ed.act(a, clip);
        self.changed = true;
        if done.close {
            return Ok(done.line);
        }
        if !done.edit.is_empty() {
            let applied = self
                .takes
                .edit(&mut self.project, ed.track, ed.slot, &done.edit, done.join)
                .map_err(|e| format!("edit: {e:?}"));
            if let Err(e) = applied {
                self.editor = Some(ed);
                return Err(e);
            }
            if let Some(clip) = self.project.clip(ed.track, ed.slot) {
                ed.applied(clip, &done);
            }
            self.editor = Some(ed);
            self.compile()?;
        } else {
            self.editor = Some(ed);
        }
        Ok(done.line)
    }

    /// Carry out a touch on the time engine's screens. Returns a line for
    /// the log.
    pub fn act(&mut self, a: TimeAction) -> Result<String, String> {
        let info = self.info;
        match a {
            TimeAction::Transport(cmd) => {
                if !self.command(cmd) {
                    return Err("transport: command queue full".into());
                }
                Ok(format!("transport: {cmd:?}"))
            }
            TimeAction::Undo => match self.takes.undo(&mut self.project) {
                Some(_) => {
                    let row = self.takes.undone().last().map(|t| (t.captured, t.number));
                    if row.is_some_and(|(captured, _)| captured) {
                        // It can be kept again.
                        self.capture.forget_kept();
                        self.pending = self.capture.pending();
                    }
                    self.compile().map(|_| format!("undo: {}", take_name(row)))
                }
                None => Ok("undo: nothing to undo".into()),
            },
            TimeAction::Redo => match self.takes.redo(&mut self.project) {
                Some(_) => {
                    let row = self.takes.done().last().map(|t| (t.captured, t.number));
                    self.compile().map(|_| format!("redo: {}", take_name(row)))
                }
                None => Ok("redo: nothing to redo".into()),
            },
            TimeAction::Capture => self.keep(),
            TimeAction::RecordPressure { track, on } => {
                if !self.set_record_touch(track, on) {
                    return Err(format!("pad pressure: no track {}", track + 1));
                }
                let name = &self.project.tracks[track].name;
                Ok(if on {
                    format!("pad pressure: {name} records it")
                } else {
                    format!("pad pressure: {name} leaves it out")
                })
            }
            TimeAction::Arm(track) => {
                self.armed = track.filter(|&t| t < self.project.tracks.len());
                self.compile()?;
                Ok(match self.armed {
                    Some(t) => format!("armed: {}", self.project.tracks[t].name),
                    None => "armed: none".into(),
                })
            }
            TimeAction::LaunchScene(_) | TimeAction::StopAll => {
                let when = if info.state.rolling() {
                    When::Cued
                } else {
                    When::Now(info.position - info.position.rem_euclid(info.meter.bar_ticks()))
                };
                let before = (
                    self.launches.clone(),
                    self.cues.clone(),
                    self.perform.clone(),
                    self.project
                        .tracks
                        .iter()
                        .map(|t| t.mute)
                        .collect::<Vec<_>>(),
                );
                let (launches, cues) = (&mut self.launches, &mut self.cues);
                let line = match a {
                    TimeAction::LaunchScene(row) => {
                        if !self
                            .perform
                            .launch_scene(&mut self.project, row, when, launches, cues)
                        {
                            return Err(format!("no scene {}", row + 1));
                        }
                        format!("scene {}", row + 1)
                    }
                    _ => {
                        self.perform.stop_all(when, launches, cues);
                        "stop all".to_string()
                    }
                };
                if let Err(e) = self.compile() {
                    (self.launches, self.cues, self.perform) = (before.0, before.1, before.2);
                    for (t, m) in self.project.tracks.iter_mut().zip(before.3) {
                        t.mute = m;
                    }
                    return Err(e);
                }
                Ok(line)
            }
            TimeAction::Launch { .. } | TimeAction::StopClip(_) | TimeAction::Uncue(_) => {
                if let TimeAction::Uncue(track) = a {
                    self.perform.uncue(track);
                }
                // While the music plays, they wait for the next bar line.
                let after = clips_after(&self.project, &self.launches, &self.cues, &a, &info);
                let before = (
                    std::mem::replace(&mut self.launches, after.0),
                    std::mem::replace(&mut self.cues, after.1),
                );
                if let Err(e) = self.compile() {
                    (self.launches, self.cues) = before;
                    return Err(e);
                }
                Ok(format!("clips: {a:?}"))
            }
            TimeAction::NewClip { track, slot } => {
                let bar = info.meter.bar_ticks();
                let length = info.loop_range.map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t.slots.get_mut(slot).ok_or("no such slot")?;
                if s.is_some() {
                    return Err(format!("slot {} of {} is not empty", slot + 1, t.name));
                }
                *s = Some(Clip::new(&format!("Clip {}", slot + 1), length));
                self.armed = Some(track);
                (self.launches, self.cues) =
                    clips_after(&self.project, &self.launches, &self.cues, &a, &info);
                self.compile()?;
                // Record into it, unless already recording or counting in.
                if !matches!(info.state, State::Recording | State::CountIn) {
                    self.command(Command::Record);
                }
                Ok(format!(
                    "new clip in slot {} of track {}, recording",
                    slot + 1,
                    track + 1
                ))
            }
            TimeAction::Chip(_) => Ok(String::new()),
            TimeAction::MorphRows { a, b } => {
                self.morph.a = a.min(SLOTS - 1);
                self.morph.b = b.min(SLOTS - 1);
                self.changed = true;
                self.song_morph();
                Ok(format!(
                    "morph: rows {} and {}",
                    self.morph.a + 1,
                    self.morph.b + 1
                ))
            }
            TimeAction::RecSong(on) => {
                let rows = (self.morph.a, self.morph.b);
                self.song_rec
                    .arm(on, &self.project, &self.launches, self.morph.pos, rows);
                self.changed = true;
                if on {
                    return Ok("rec song: what the grid plays goes into the song".into());
                }
                self.keep_song().map(|line| format!("rec song off: {line}"))
            }
            TimeAction::Morph(_)
            | TimeAction::SnapBack
            | TimeAction::KeepMorph
            | TimeAction::SaveScene(_) => {
                Err("morph: needs the mixer's mod engine (TimeHost::morph)".into())
            }
            TimeAction::Edit(e) => self.edit(e),
            TimeAction::SongView(on) => {
                self.song_view = on;
                self.changed = true;
                Ok(if on { "song view" } else { "clips view" }.into())
            }
            TimeAction::SongMode(on) => {
                self.song = on;
                if let Err(e) = self.compile() {
                    self.song = !on;
                    return Err(e);
                }
                Ok(if on { "song on" } else { "song off" }.into())
            }
            TimeAction::SelectPlacement(i) => {
                self.selected = i.filter(|&i| i < self.project.arrangement.placements.len());
                self.region = None;
                self.changed = true;
                Ok(String::new())
            }
            // The selection's kind: a region picked drops the placement.
            TimeAction::Audio(AudioAction::Pick(r)) => {
                self.region = r;
                if r.is_some() {
                    self.selected = None;
                }
                self.changed = true;
                Ok(String::new())
            }
            // The audio tracks' engine is the app's to wire (AudioHost);
            // until it takes these, say so rather than drop them quietly.
            TimeAction::Audio(a) => Err(format!("audio tracks: {a:?} is not wired yet")),
            TimeAction::ScenesToSong
            | TimeAction::SongToScenes
            | TimeAction::MakeUnique(_)
            | TimeAction::DeletePlacement(_) => {
                let bar = info.meter.bar_ticks();
                let before = self.project.clone();
                let line = match self.edit_song(a, bar) {
                    Ok(line) => line,
                    Err(e) => {
                        self.project = before;
                        return Err(e);
                    }
                };
                if let Err(e) = self.compile() {
                    self.project = before;
                    self.compile()?;
                    return Err(e);
                }
                Ok(line)
            }
        }
    }

    /// What live to song put into the song when the transport stopped
    /// (or why it could not: the recording is then kept, and goes in when
    /// Rec song is turned off), once, for the log.
    pub fn take_song_note(&mut self) -> Option<String> {
        self.song_note.take()
    }

    /// Live to song: the passes recorded so far into the song, compiled
    /// in. A line for the log.
    fn keep_song(&mut self) -> Result<String, String> {
        if !self.song_rec.waiting() {
            return Ok("nothing recorded".into());
        }
        let before = self.project.arrangement.clone();
        let r = self
            .song_rec
            .finish(&mut self.project)
            .map_err(|e| format!("rec song: {e:?}"))?;
        // New placements shift the indices.
        self.selected = None;
        if let Err(e) = self.compile() {
            self.project.arrangement = before;
            self.compile()?;
            return Err(e);
        }
        Ok(format!(
            "{} placements and {} morph points into the song",
            r.placements, r.morph_points
        ))
    }

    /// The Song view's edits of the arrangement (and, for Song → scenes,
    /// the rows). A line for the log.
    fn edit_song(&mut self, a: TimeAction, bar: Tick) -> Result<String, String> {
        let song = &mut self.project.arrangement;
        match a {
            TimeAction::ScenesToSong => {
                // Every row with a clip, in order, after the song's end.
                let rows: Vec<(usize, Tick)> = (0..SLOTS)
                    .filter(|&r| self.project.row_filled(r))
                    .map(|r| (r, scene_length(&self.project, r, bar)))
                    .collect();
                if rows.is_empty() {
                    return Err("scenes → song: no row has a clip".into());
                }
                let end = self.project.arrangement.end();
                let at = (end + bar - 1) / bar.max(1) * bar;
                let add = scenes_to_song(&self.project, &rows, at)
                    .map_err(|e| format!("scenes → song: {e:?}"))?;
                // New placements shift the indices.
                self.selected = None;
                let song = &mut self.project.arrangement;
                for p in add.placements {
                    song.place(p).map_err(|e| format!("scenes → song: {e:?}"))?;
                }
                for m in add.markers {
                    song.mark(m.at, &m.name)
                        .map_err(|e| format!("scenes → song: {e:?}"))?;
                }
                Ok(format!(
                    "scenes → song: {} rows from bar {}",
                    rows.len(),
                    at / bar.max(1) + 1
                ))
            }
            TimeAction::SongToScenes => match song_to_scenes(&mut self.project, bar) {
                Ok(rows) if rows.is_empty() => Ok("song → scenes: the song is empty".into()),
                Ok(rows) => Ok(format!(
                    "song → scenes: rows {} to {}",
                    rows[0] + 1,
                    rows[rows.len() - 1] + 1
                )),
                Err(ModelError::NoRoom) => {
                    Err("song → scenes: not enough empty rows in a run".into())
                }
                Err(e) => Err(format!("song → scenes: {e:?}")),
            },
            TimeAction::MakeUnique(i) => {
                song.make_unique(&self.project.tracks, i)
                    .map_err(|e| format!("make unique: {e:?}"))?;
                Ok(format!("placement {} is its own copy", i + 1))
            }
            TimeAction::DeletePlacement(i) => {
                song.remove(i).ok_or("delete: no such placement")?;
                self.selected = None;
                Ok(format!("placement {} deleted", i + 1))
            }
            _ => Ok(String::new()),
        }
    }

    /// The morph bar on the mixer's mod engine (`modu`): the rows, the
    /// crossfader (which loads the rows on its first move), snap back,
    /// keep and saving a row. A line for the log.
    pub fn morph(
        &mut self,
        a: TimeAction,
        modu: &mut crate::modmix::ModMix,
        mixer: &mpfree_fx::Mixer,
    ) -> Result<String, String> {
        let (ra, rb) = (self.morph.a, self.morph.b);
        let line = match a {
            TimeAction::MorphRows { .. } if !self.morph.on => return self.act(a),
            TimeAction::MorphRows { a, b } => {
                // Morphing: the new rows go on A and B at once, from the
                // same sound before.
                let (a, b) = (a.min(SLOTS - 1), b.min(SLOTS - 1));
                let l = modu
                    .morph_load(mixer, &mut self.scene_morph, &mut self.project, a, b)
                    .map_err(|e| format!("morph: {e:?}"))?;
                modu.morph_to(self.morph.pos);
                (self.morph.a, self.morph.b, self.morph.params) = (a, b, l.morphing);
                self.song_morph();
                format!(
                    "morph: rows {} and {}, {} parameters",
                    a + 1,
                    b + 1,
                    l.morphing
                )
            }
            TimeAction::Morph(x) => {
                let x = if x.is_finite() {
                    x.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let mut line = String::new();
                if !self.morph.on {
                    let l = modu
                        .morph_load(mixer, &mut self.scene_morph, &mut self.project, ra, rb)
                        .map_err(|e| format!("morph: {e:?}"))?;
                    self.morph.on = true;
                    self.morph.params = l.morphing;
                    line = format!(
                        "morph: rows {} and {}, {} parameters{}",
                        ra + 1,
                        rb + 1,
                        l.morphing,
                        if l.unknown > 0 {
                            format!(" ({} not on the mixer, left out)", l.unknown)
                        } else {
                            String::new()
                        }
                    );
                }
                modu.morph_to(x);
                self.morph.pos = x;
                self.song_morph();
                line
            }
            TimeAction::SnapBack | TimeAction::KeepMorph if !self.morph.on => {
                return Ok(String::new())
            }
            TimeAction::SnapBack | TimeAction::KeepMorph => {
                let keep = a == TimeAction::KeepMorph;
                modu.morph_end(&mut self.scene_morph, &mut self.project, keep);
                self.morph.on = false;
                self.morph.pos = 0.0;
                self.morph.params = 0;
                if keep {
                    "morph: kept where it was".into()
                } else {
                    "morph: snapped back".into()
                }
            }
            TimeAction::SaveScene(row) => match modu.save_scene(&mut self.project, row) {
                Some(n) => format!("morph: row {} stores {n} values", row + 1),
                None => return Err(format!("morph: row {} cannot store them", row + 1)),
            },
            other => return self.act(other),
        };
        self.changed = true;
        Ok(line)
    }

    /// Live to song: the crossfader's position and the rows it morphs
    /// between, which go into the song's morph lane while Rec song records.
    fn song_morph(&mut self) {
        let at = self.info.position;
        let rows = (self.morph.a, self.morph.b);
        self.song_rec.morph(at, self.morph.pos, rows);
    }

    /// CAPTURE: keep the last phrase in the armed track (track 1 if none
    /// is armed): into its playing clip when it was played in time over
    /// it, else as a new clip in its first empty slot that plays on in its
    /// place. A line for the log.
    fn keep(&mut self) -> Result<String, String> {
        if self.capture.pending().is_none() {
            return Ok("capture: nothing new to keep".into());
        }
        let phrase = self.capture.phrase().ok_or("capture: nothing played")?;
        let t = &self.info;
        let now = Now {
            position: t.position,
            rolling: t.state.rolling(),
            meter: t.meter,
            loop_range: t.loop_range,
        };
        let track = self.armed.unwrap_or(0);
        let name = format!("Capture {}", self.takes.captures() + 1);
        let k = keep(
            &phrase,
            &mut self.project,
            &self.launches,
            &mut self.takes,
            track,
            now,
            &name,
        )
        .map_err(|e| format!("capture: {e:?}"))?;
        // The moves made over a phrase played in time go with it.
        let origin = k.launch.map(|l| l.origin).or_else(|| {
            self.launches
                .iter()
                .find(|l| l.track == track && l.slot == k.slot)
                .map(|l| l.origin)
        });
        let mut moves = 0;
        if let (Some(s), Some(origin)) = (phrase.song_start, origin) {
            if let Some(clip) = self.project.clip(track, k.slot) {
                let list = moves_for_clip(
                    &self.move_log.phrase(),
                    s as f64,
                    (s + phrase.length) as f64,
                    clip,
                    origin,
                    track as u8,
                );
                if !list.is_empty() {
                    moves = self
                        .takes
                        .capture_moves(&mut self.project, track, k.slot, k.take, &list)
                        .unwrap_or(0);
                }
            }
            self.move_log.forget();
        }
        if let Some(l) = k.launch {
            self.launches.retain(|x| x.track != l.track);
            self.cues.retain(|x| x.track != l.track);
            self.launches.push(l);
        }
        self.capture.mark_kept(phrase.id);
        self.pending = self.capture.pending();
        self.compile()?;
        let into = if k.launch.is_some() {
            "new clip in"
        } else {
            "into the clip in"
        };
        let moved = match moves {
            0 => String::new(),
            1 => " and 1 move".into(),
            n => format!(" and {n} moves"),
        };
        Ok(format!(
            "capture {}: {} notes{moved}, {}, {into} slot {} of {}",
            k.take,
            k.notes,
            bars_label(k.bars),
            k.slot + 1,
            self.project.tracks[track].name
        ))
    }

    /// A clip made elsewhere (the Sample page's chop to a phrase) into the
    /// first empty slot of the armed track (track 1 if none is armed), as
    /// a take that UNDO on the Clips page takes away again. It isn't
    /// launched: a tap on the Clips page plays it like any clip. Returns a
    /// line for the log.
    pub fn put_clip(&mut self, clip: Clip) -> Result<String, String> {
        let track = self.armed.unwrap_or(0);
        let t = self.project.tracks.get(track).ok_or("no such track")?;
        let name = t.name.clone();
        let slot = (0..SLOTS)
            .find(|&s| t.slots[s].is_none())
            .ok_or_else(|| format!("{name} has no empty slot for the phrase"))?;
        let notes = clip.notes.len();
        self.takes
            .capture_clip(&mut self.project, track, slot, clip)
            .map_err(|e| format!("phrase refused: {e:?}"))?;
        self.compile()?;
        Ok(format!(
            "phrase: {notes} notes in slot {} of {name}",
            slot + 1
        ))
    }

    /// "Record pad pressure" for `track` (`Track::record_touch`). False if
    /// there is no such track.
    pub fn set_record_touch(&mut self, track: usize, on: bool) -> bool {
        let Some(t) = self.project.tracks.get_mut(track) else {
            return false;
        };
        if t.record_touch != on {
            t.record_touch = on;
            self.changed = true;
            self.sync_tracks();
        }
        true
    }

    /// TRACK MUTE's keys (control map decision 5): mute track `t` now, or
    /// let it play again. Its clip keeps its place, so it comes back in
    /// time; a scene's mutes set it again when the scene starts.
    pub fn toggle_mute(&mut self, t: usize) -> Result<String, String> {
        let Some(track) = self.project.tracks.get_mut(t) else {
            return Err(format!("no track {}", t + 1));
        };
        track.mute = !track.mute;
        if let Err(e) = self.compile() {
            self.project.tracks[t].mute ^= true;
            return Err(e);
        }
        let track = &self.project.tracks[t];
        Ok(format!(
            "track mute: {} {}",
            track.name,
            if track.mute { "muted" } else { "plays" }
        ))
    }

    /// Whether track `t` is muted (None: no such track), for TRACK MUTE's
    /// pad lights.
    pub fn muted(&self, t: usize) -> Option<bool> {
        self.project.tracks.get(t).map(|tr| tr.mute)
    }

    /// Whether track `t` has a clip launched (playing, or waiting for the
    /// transport), for TRACK MUTE's pad lights.
    pub fn launched(&self, t: usize) -> bool {
        self.launches.iter().any(|l| l.track == t)
    }

    /// True once after the clips, launches or takes changed. Notes and
    /// locks being recorded are told at most every quarter second.
    pub fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    /// Everything the screens show.
    pub fn state(&self) -> TimeState {
        let mut st = TimeState::from_engine(
            self.info,
            &self.project,
            &self.launches,
            self.armed,
            &self.takes,
        );
        st.capture = self.pending.map(|p| bars_label(p.bars));
        st.cues = self.cues.clone();
        st.scene_playing = self.perform.playing().map(|p| p.row);
        st.scene_cued = self.perform.cued();
        st.morph = self.morph;
        st.song_view = self.song_view;
        st.song_on = self.song;
        st.selected = self.selected;
        st.region = self.region;
        st.song_rec = self.song_rec.armed();
        st.edit = self.editor.as_ref().and_then(|ed| {
            let clip = self.project.clip(ed.track, ed.slot)?;
            let bar = clip.speed.to_clip(self.info.meter.bar_ticks() as f64);
            Some(Box::new(EditInfo {
                editor: ed.clone(),
                clip: clip.clone(),
                track_name: self.project.tracks[ed.track].name.clone(),
                bar: bar.round() as Tick,
            }))
        });
        st
    }

    /// The time engine's part of the status line, with the audio half's
    /// counts as of its last period.
    pub fn describe(&self) -> String {
        let t = &self.info;
        let c = &self.counts;
        let mut line = format!(
            "time: {:?} at {}, {:.1} BPM, {} takes, {} notes or locks lost to full tables, \
             {} pressure values lost",
            t.state,
            t.meter.position(t.position),
            t.bpm,
            self.takes.done().len(),
            c.dropped + self.dropped(),
            c.touch_dropped + self.touch_lost
        );
        // The fallbacks between the halves, when there are any.
        let lost = c.lost + self.lost();
        if lost + c.behind + c.kept + c.refused > 0 {
            line.push_str(&format!(
                "; between the halves: {lost} messages lost, {} late views compiled again, \
                 {} views kept unfreed, {} transport commands refused",
                c.behind, c.kept, c.refused
            ));
        }
        line
    }
}
