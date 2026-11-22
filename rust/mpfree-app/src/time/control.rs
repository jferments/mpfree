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
//!
//! A step edit (a tap, a turn, a lock) that only adds and erases notes and
//! locks of a clip playing or cued is not compiled: its changes go to the
//! audio half, which makes them to its view in place, whole or not at all
//! ([`ToAudio::EditPart`], [`ToAudio::EditEnd`]; refactor A3).

use std::time::{Duration, Instant};

use mpfree_capture::{keep, moves_for_clip, Capture, KeepError, Kept, Now, Pending};
use mpfree_notefx::{Chain, TrackFx, MAX_FX};
use mpfree_perform::{Perform, SceneMorph, SongRecorder, When};
use mpfree_play::Replaced;
use mpfree_play::{
    ClipDrop, Cue, Decisions, Includes, Launch, MuteCue, MuteLanded, PlayView, SongChange, Started,
    Takes,
};
use mpfree_play::{MOVE_POINTS, TOUCH_POINTS};
use mpfree_seq::arrange::{
    list_spans, list_to_song, scene_length, scenes_to_song, song_to_scenes, Difference,
};
use mpfree_seq::loops::{self, LoopNow, LoopPlan};
use mpfree_seq::mpfree_moves::{Kind, MoveLog};
use mpfree_seq::scenes::{MAX_LIST_STEPS, MAX_TIMES};
use mpfree_seq::steps::{Edit, Grid};
use mpfree_seq::ListStep;
use mpfree_seq::{
    Clip, LaunchGrid, ModelError, Project, TouchPoint, TrackId, LOOPS, MAX_TRACKS, ROWS, SLOTS,
};
use mpfree_time_ui::editor::{EditAction, Editor};
use mpfree_time_ui::list::ListRow;
use mpfree_time_ui::view::{
    bars_label, card_rows, clips_after, grid_name, rows_named, scene_rows, AudioAction, EditInfo,
    FollowNext, MorphInfo, MuteWait, RegionId, TakeRow, TempoEdit, TimeAction, TimeState,
    TransportInfo, SCENE_NAME,
};
use mpfree_transport::time::{TempoMap, Tick};
use mpfree_transport::{Block, Command, State};

use super::follow::FollowCounts;
use super::link::{Counts, CueCounts, InPlace, PeriodEnd, Rx, ToAudio, ToControl, Tracks, Tx};
use super::{StepAction, StepClip, StepsData, LINE, NEW_CLIP_BARS, RECORD_ROOM, STEP_TICKS};

/// The clip settings card's join keys for its takes, a row each (see
/// `Takes::set_clip`).
const CARD_JOIN: u64 = 0xC1_0000;

/// The scene editor's join keys for its takes, a card row each (see
/// `Takes::set_scene`).
const SCENE_JOIN: u64 = 0x5C_0000;

/// Started cues one period may bring before the list grows.
const STARTED_ROOM: usize = 64;

/// The tracks' mutes and solos as they will be once a mute queue lands,
/// read without changing the project.
struct Landed<'a> {
    project: &'a Project,
    q: &'a MuteWait,
    soloing: bool,
}

/// The kit's pads (kit 1's, which have the mixer's pad strips).
pub const PADS: usize = 16;

/// A kit pad tapped as a mute key ([`TimeControl::pad_mute_or_solo`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PadTap {
    /// It waits in the mute queue (or its change was taken back): the
    /// line for the log.
    Waits(String),
    /// At once: the pad's mute and solo now, for the mixer.
    Now { mute: bool, solo: bool },
}

/// What a pad's change does, for the log: "solo" (a solo plays the pad
/// whatever its mute), "muted" or "plays".
fn pad_words(mute: bool, solo: bool) -> &'static str {
    match (solo, mute) {
        (true, _) => "solo",
        (false, true) => "muted",
        (false, false) => "plays",
    }
}

fn queue_landed<'a>(project: &'a Project, q: &'a MuteWait) -> Landed<'a> {
    let mut l = Landed {
        project,
        q,
        soloing: false,
    };
    l.soloing = (0..project.tracks.len()).any(|t| l.flags(t).1);
    l
}

impl Landed<'_> {
    /// Track `t`'s mute and solo then.
    fn flags(&self, t: usize) -> (bool, bool) {
        match self.q.want.iter().find(|w| w.0 == t) {
            Some(&(_, m, s)) => (m, s),
            None => self
                .project
                .tracks
                .get(t)
                .map_or((false, false), |tr| (tr.mute, tr.solo)),
        }
    }

    fn silent(&self, t: usize) -> bool {
        let (m, s) = self.flags(t);
        m || (self.soloing && !s)
    }
}

/// What compiles and step edits cost the thread the control half runs on
/// (today the audio thread), for the status line: the device's numbers
/// for the time engine's own thread (time-threads.md, step 3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Costs {
    pub compiles: u64,
    /// The slowest compile, and the notes and locks its view played.
    pub slowest_compile: Duration,
    pub slowest_compile_notes: usize,
    /// Step edits made in place, and step edits that compiled.
    pub steps_in_place: u64,
    pub steps_compiled: u64,
    /// The slowest of each kind, from the action to the audio half having
    /// taken what it sent ([`super::TimeHost::step_action`]).
    pub slowest_in_place: Duration,
    pub slowest_step_compiled: Duration,
}

impl Costs {
    /// "compiles: 12 (slowest 0.21 ms, 96 notes and locks); step edits:
    /// 40 in place (slowest 0.02 ms), 3 compiled (slowest 0.25 ms)".
    pub fn describe(&self) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        format!(
            "compiles: {} (slowest {:.2} ms, {} notes and locks); step edits: {} in place \
             (slowest {:.2} ms), {} compiled (slowest {:.2} ms)",
            self.compiles,
            ms(self.slowest_compile),
            self.slowest_compile_notes,
            self.steps_in_place,
            ms(self.slowest_in_place),
            self.steps_compiled,
            ms(self.slowest_step_compiled)
        )
    }
}

/// Perform's Rec song with REC (sequencer S4), between takes and during
/// one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SongTake {
    /// Rec song is as set by hand; in Perform the next take turns it on.
    #[default]
    Idle,
    /// The take recording turned it on: it goes off when the take ends.
    Armed,
    /// Taken off for this take (or the next): back to Idle when it ends.
    Off,
}

/// When a change waiting for `line` lands, for the log: "on the next
/// bar", "on the next 2-bar line", "on the next 1/4".
fn line_words(line: LaunchGrid) -> String {
    match line {
        LaunchGrid::Now => "at once".to_string(),
        LaunchGrid::Bars(1) => "on the next bar".to_string(),
        LaunchGrid::Bars(n) => format!("on the next {n}-bar line"),
        LaunchGrid::Ticks(t) => format!("on the next {}", grid_name(t)),
        LaunchGrid::At(_) => "at the end of its pass".to_string(),
    }
}

/// What TRACK MUTE's pad for a track shows ([`TimeControl::mute_light`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MuteLight {
    /// Muted, or silent while other tracks are soloed.
    pub silent: bool,
    /// Soloed.
    pub solo: bool,
    /// A change waits in the mute queue.
    pub waiting: bool,
    /// A clip of it is launched.
    pub launched: bool,
}

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
    /// The Arrange page shows the song list (S4), and the step picked on
    /// it.
    list_view: bool,
    list_pick: Option<usize>,
    /// The scene editor's row as picked (its ‹ ›, a scene launched, a
    /// song list step picked); none follows the scene playing.
    scene_pick: Option<usize>,
    selected: Option<usize>,
    /// The audio region picked on the Song view; it and `selected` are
    /// never both set (`TimeState::pick`).
    region: Option<RegionId>,
    /// The section picked on the Song view (by its marker), for the
    /// wheel; never set with `selected` or `region`.
    section: Option<usize>,
    /// The tempo point picked on the Song view (tempo ramps), for the
    /// wheel and the Ramp and Delete buttons; never set with `selected`,
    /// `region` or `section`.
    tempo_pick: Option<usize>,
    /// The project's tempo (the app's, [`TimeControl::project_tempo`]):
    /// the song's tempo before its first tempo point.
    project_bpm: f64,
    /// Live to song: records what the grid plays into the song.
    song_rec: SongRecorder,
    /// Perform's Rec song with REC: armed by the take, or taken off for
    /// it ([`TimeControl::take_follows`]).
    song_take: SongTake,
    /// A take records (the notes, their count-in, or a loop), as the host
    /// last said.
    taking: bool,
    /// What happened to the last recording when the transport stopped,
    /// or to a take thrown away, for the log
    /// ([`TimeControl::take_later_note`]).
    later_note: Option<String>,
    /// UNDO while recording: the take recording is thrown away once the
    /// punch out lands (`TimeAction::ThrowTake`).
    throw: bool,
    /// The take the pass recording now (or last) recorded into.
    pass_take: Option<u32>,
    scene_morph: SceneMorph,
    /// The note editor (the Sequencer tab's Notes view), while it is open.
    editor: Option<Editor>,
    /// The armed track: REC records it, and the audio half's recorder
    /// takes live notes only for it. Kept by its lasting id
    /// (`armed_id`), so a track added, taken away by UNDO or brought back
    /// by REDO never moves the arm to another track (refactor C1).
    pub(super) armed: Option<usize>,
    armed_id: TrackId,
    /// The track selected apart from the arm (`TimeAction::Select`: a
    /// track picked while REC's arm is pinned), or None while the
    /// selected track is the armed one. Kept by its lasting id
    /// (`pick_id`) too.
    track_pick: Option<usize>,
    pick_id: TrackId,
    /// The other armed tracks, a bit each (`TimeAction::ArmAlso`): a take
    /// records each one's notes too. Never a Loop track; the armed one is
    /// left out of the view's set. Cleared when the tracks change.
    also: u128,
    /// The track count `also` was set with: tracks added or taken away
    /// clear it, so a bit never names a track that moved.
    also_n: usize,
    /// Other armed tracks a take found with no clip and no empty slot to
    /// make one in: they didn't record it.
    also_full: u64,
    takes: Takes,
    /// REDOs done so far: a track that comes back with one is the one UNDO
    /// took away, and its kit comes back with it (`TrackKits::follow`).
    redos: u64,
    /// The clip COPY + a clip copied last, for COPY + an empty slot.
    clipboard: Option<Clip>,
    /// The take a step lock's turn went into last: a turn that joins it
    /// writes no line, so a knob turned through many values is one line.
    lock_take: u32,
    /// What compiles and step edits cost.
    pub(super) costs: Costs,
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
    /// What following another device's clock put right or lost.
    follow: FollowCounts,
    /// Following another device's Start and Stop while it is stopped:
    /// PLAY waits for its Start (set by the host before each action).
    pub(super) clock_waits: bool,
    /// The current period's spans, copied from the audio half's.
    block: Box<Block>,
    /// The next span starts a new period's copy.
    fresh: bool,
    spans_lost: u64,
    /// The current period's started cues.
    started: Vec<Started>,
    /// Mute and solo changes waiting for their line (sequencer S4's mute
    /// queue), and the queue's number: the player lands their cues.
    mute_queue: Option<(u32, MuteWait)>,
    mute_ids: u32,
    /// The current period's landed mute cues.
    landed: Vec<MuteLanded>,
    /// The kit's pads whose change in the mute queue came due this period
    /// (their mute and solo), for the mixer ([`TimeControl::take_pad_landings`]).
    pad_landings: [Option<(bool, bool)>; PADS],
    /// Clips' saved states (Save state, sequencer S4), as (track, slot,
    /// the clip as it was): Reload clip puts one back. Kept while the
    /// project is open, not saved with it.
    saved: Vec<(usize, usize, Clip)>,
    /// Reloads waiting for their launch line.
    reloads: Vec<ReloadWait>,
    /// The audio's sample rate, for a period's length in ticks.
    rate: u32,
    /// The bar line a cue started on in the last poll, on each loop's
    /// Loop track: its loop goes on or off there ([`TimeControl::loop_plan`]).
    loop_at: [Option<Tick>; LOOPS],
    /// The loops as the looper last said ([`TimeControl::loops_are`]).
    loops_seen: [LoopNow; LOOPS],
    /// Where each loop last went from, and the pass it had: a redo of its
    /// first pass (the same length from the same place) goes back there.
    loops_gone: [Option<((usize, usize), LoopNow)>; LOOPS],
    /// The Loop cell armed (`TimeAction::LoopCell`): the next loop
    /// recorded lands there.
    loop_cell: Option<(usize, usize)>,
    /// The loops the song's blocks play (song mode, `loops::song_plan`),
    /// as of the last period.
    song_loops: [bool; LOOPS],
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
    /// Clips recorded pressure went into since the recording stopped
    /// last, to check against `TOUCH_RECLAIM_AT` once it has (B4b).
    touch_grew: Vec<(usize, usize)>,
    /// Pressure values no note could play again, freed so far, and how
    /// many times (refactor B4b).
    touch_freed: (u64, u64),
    /// What the views include: the recorded changes made so far, the
    /// started cues applied on each track, and each track's newest
    /// decision with the frame it was made at (Arrange 2.5).
    pub(super) includes: Includes,
    /// The control half's side of the timing contract: which cues are new
    /// decisions (Arrange 2.5).
    decisions: Box<Decisions>,
    /// The absolute frame the music had reached as far as the control half
    /// has heard: the end of the last period it took.
    heard: u64,
    /// What the audio half's player counted for the contract, as last
    /// sent.
    cue_counts: CueCounts,
    tx: Tx<ToAudio>,
    rx: Rx<ToControl>,
    /// A view the audio half's full queue handed back, sent again on the
    /// next poll unless a newer one replaces it.
    unsent: Option<(Box<PlayView>, u32)>,
    /// The number of the last view sent ([`ToAudio::View`]).
    view_sent: u32,
    /// The number of the last step edit sent in place.
    edit_id: u32,
    /// The tracks' settings the audio half has.
    tracks_sent: Tracks,
    /// The tracks' MIDI effects the audio half has.
    note_fx_sent: Box<[TrackFx; MAX_TRACKS]>,
    /// Tracks the audio half has a chain of MIDI effects for (one is made
    /// here the first time a track has an effect, and it stays).
    chains_given: [bool; MAX_TRACKS],
}

/// A reload waiting for its launch line (sequencer S4's Reload clip).
#[derive(Clone, Copy, Debug, PartialEq)]
struct ReloadWait {
    track: usize,
    slot: usize,
    /// The line's song tick.
    line: Tick,
    /// Where the transport was when it was asked: a jump back (a loop,
    /// a locate) lands it at once.
    asked: Tick,
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
            list_view: false,
            list_pick: None,
            scene_pick: None,
            selected: None,
            region: None,
            section: None,
            tempo_pick: None,
            project_bpm: info.bpm,
            song_rec: SongRecorder::default(),
            song_take: SongTake::Idle,
            taking: false,
            later_note: None,
            throw: false,
            pass_take: None,
            scene_morph: SceneMorph::default(),
            editor: None,
            armed: None,
            armed_id: TrackId::NONE,
            track_pick: None,
            pick_id: TrackId::NONE,
            also: 0,
            also_n: 0,
            also_full: 0,
            takes: Takes::default(),
            redos: 0,
            clipboard: None,
            lock_take: 0,
            costs: Costs::default(),
            capture: Box::new(Capture::new()),
            move_log: Box::new(MoveLog::new()),
            pending: None,
            info,
            counts: Counts::default(),
            follow: FollowCounts::default(),
            clock_waits: false,
            block: Box::new(Block::new()),
            fresh: true,
            spans_lost: 0,
            started: Vec::with_capacity(STARTED_ROOM),
            mute_queue: None,
            mute_ids: 0,
            landed: Vec::with_capacity(MAX_TRACKS),
            pad_landings: [None; PADS],
            saved: Vec::new(),
            reloads: Vec::new(),
            rate,
            loop_at: [None; LOOPS],
            loops_seen: [LoopNow::default(); LOOPS],
            loops_gone: [None; LOOPS],
            loop_cell: None,
            song_loops: [false; LOOPS],
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
            touch_grew: Vec::new(),
            touch_freed: (0, 0),
            includes: Includes::default(),
            decisions: Box::default(),
            heard: 0,
            cue_counts: CueCounts::default(),
            tx,
            rx,
            unsent: None,
            view_sent: 0,
            edit_id: 0,
            tracks_sent: tracks,
            note_fx_sent: Box::new([TrackFx::default(); MAX_TRACKS]),
            chains_given: [false; MAX_TRACKS],
        };
        c.set_armed(tracks.armed);
        c.compile()?;
        Ok(c)
    }

    /// The armed track (the one REC records), if any.
    pub fn armed(&self) -> Option<usize> {
        self.armed
    }

    /// The selected track: the one the Sequencer's Steps, Notes and STEP
    /// SEQ edit and the pads follow (refactor C1). The armed track unless
    /// one was selected apart from it while the arm is pinned.
    pub fn selected(&self) -> Option<usize> {
        self.track_pick.or(self.armed)
    }

    /// Arm `track` (or none), by its lasting id. A track armed is selected
    /// too: the selection follows the arm again.
    fn set_armed(&mut self, track: Option<usize>) {
        self.armed = track.filter(|&t| t < self.project.tracks.len());
        self.armed_id = self.id_of(self.armed);
        self.track_pick = None;
        self.pick_id = TrackId::NONE;
    }

    /// Select `track` apart from the arm, by its lasting id; the armed
    /// track (or no such track) selects the armed one again.
    fn set_pick(&mut self, track: usize) {
        let pick = Some(track).filter(|&t| t < self.project.tracks.len() && Some(t) != self.armed);
        self.track_pick = pick;
        self.pick_id = self.id_of(pick);
    }

    /// The lasting id of track `track`, if there is one.
    fn id_of(&self, track: Option<usize>) -> TrackId {
        track
            .and_then(|t| self.project.tracks.get(t))
            .map_or(TrackId::NONE, |t| t.id)
    }

    /// After the tracks changed (one added, taken away by UNDO or brought
    /// back by REDO): the armed and the selected track found again by
    /// their lasting ids. An armed track that is gone leaves the arm on
    /// the last track that isn't a Loop track; a selected one that is
    /// gone leaves the selection on the armed track.
    fn find_arm_and_pick(&mut self) {
        let project = &self.project;
        if self.armed.is_some() {
            let found = project.track_with(self.armed_id).or_else(|| {
                // No id (only a track never given one): its place, if it
                // is still there.
                self.armed
                    .filter(|&t| !self.armed_id.is_given() && t < project.tracks.len())
            });
            let armed = found.or_else(|| project.tracks.iter().rposition(|t| !t.is_loops()));
            self.armed = armed;
            self.armed_id = self.id_of(armed);
        }
        if self.track_pick.is_some() {
            let found = self.project.track_with(self.pick_id);
            self.track_pick = found.filter(|&t| Some(t) != self.armed);
            if self.track_pick.is_none() {
                self.pick_id = TrackId::NONE;
            }
        }
    }

    /// The other armed tracks, a bit each, without the armed one
    /// (`TimeAction::ArmAlso`).
    pub fn also(&self) -> u128 {
        let main = self
            .armed
            .and_then(|t| 1u128.checked_shl(t as u32))
            .unwrap_or(0);
        self.also & !main
    }

    /// Every armed track, a bit each: the armed one and the others.
    fn armed_set(&self) -> u128 {
        let main = self
            .armed
            .and_then(|t| 1u128.checked_shl(t as u32))
            .unwrap_or(0);
        self.also() | main
    }

    /// Other armed tracks a take found with nothing to record into. The
    /// status line says it ([`TimeControl::describe`]); tests read it here.
    #[allow(dead_code)]
    pub fn also_full(&self) -> u64 {
        self.also_full
    }

    /// The note editor is open on a clip that is still there (as the
    /// screen's [`TimeState::edit`] has it).
    pub fn editing(&self) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|ed| self.project.clip(ed.track, ed.slot).is_some())
    }

    /// The Loop cell armed on the grid while it is still an empty cell of
    /// a Loop track (`TimeAction::LoopCell`): the next loop recorded
    /// lands there.
    pub fn loop_cell(&self) -> Option<(usize, usize)> {
        self.loop_cell.filter(|&(t, s)| {
            self.project
                .tracks
                .get(t)
                .is_some_and(|tr| tr.is_loops() && tr.slots.get(s).is_some_and(|c| c.is_none()))
        })
    }

    /// Fills the queue to the audio half (empty now) but for `free`
    /// places, with messages that change nothing there: for tests of a
    /// queue that fills.
    #[cfg(test)]
    pub(super) fn fill_queue_but(&mut self, free: usize) {
        let (n, armed) = (self.tracks_sent.n, self.tracks_sent.armed);
        for _ in 0..super::link::TO_AUDIO - free {
            assert!(self.tx.send(ToAudio::Tracks { n, armed }).is_ok());
        }
    }

    /// The number of the last view sent to the audio half.
    #[cfg(test)]
    pub(super) fn view_sent(&self) -> u32 {
        self.view_sent
    }

    /// A view for the audio half; one its full queue hands back waits for
    /// the next poll.
    fn send_view(&mut self, v: Box<PlayView>) {
        self.view_sent = self.view_sent.wrapping_add(1);
        self.push_view(v, self.view_sent);
    }

    /// View number `n` into the queue, or waiting for the next poll.
    fn push_view(&mut self, v: Box<PlayView>, n: u32) {
        self.unsent = None;
        if let Err(ToAudio::View(v, n)) = self.tx.send(ToAudio::View(v, n)) {
            self.unsent = Some((v, n));
        }
    }

    /// Send the tracks' settings if they changed.
    fn sync_tracks(&mut self) {
        let now = Tracks::of(&self.project, self.armed);
        let sent = &mut self.tracks_sent;
        if (now.n, now.armed) != (sent.n, sent.armed)
            && self
                .tx
                .send(ToAudio::Tracks {
                    n: now.n,
                    armed: now.armed,
                })
                .is_ok()
        {
            (sent.n, sent.armed) = (now.n, now.armed);
        }
        // Only what changed, one track a message: one that finds the
        // queue full goes at the next poll.
        for (t, (s, was)) in now.set.iter().zip(sent.set.iter_mut()).enumerate() {
            if s != was && self.tx.send(ToAudio::Track(t, *s)).is_ok() {
                *was = *s;
            }
        }
        self.sync_note_fx();
    }

    /// Send the tracks' MIDI effects where they changed, one place a
    /// message, and a chain to run them the first time a track has one
    /// (made here: it is about 400 KB). A message the full queue hands
    /// back goes at the next poll.
    fn sync_note_fx(&mut self) {
        for t in 0..MAX_TRACKS {
            let now = self
                .project
                .tracks
                .get(t)
                .map_or_else(TrackFx::default, |tr| tr.note_fx);
            if !now.is_empty() && !self.chains_given[t] {
                let chain = Box::new(Chain::new());
                self.chains_given[t] = self.tx.send(ToAudio::Chain(t as u16, chain)).is_ok();
            }
            let was = &mut self.note_fx_sent[t];
            if now == *was {
                continue;
            }
            for place in 0..MAX_FX {
                let (slot, id) = (now.slots[place], now.ids[place]);
                if (slot != was.slots[place] || id != was.ids[place])
                    && self
                        .tx
                        .send(ToAudio::NoteFx {
                            track: t as u16,
                            place: place as u8,
                            slot,
                            id,
                        })
                        .is_ok()
                {
                    was.slots[place] = slot;
                    was.ids[place] = id;
                }
            }
            if now.arp_at != was.arp_at
                && self
                    .tx
                    .send(ToAudio::NoteArp {
                        track: t as u16,
                        at: now.arp_at,
                    })
                    .is_ok()
            {
                was.arp_at = now.arp_at;
            }
        }
    }

    /// A track's MIDI effects (its Notes stage, `Track::note_fx`), as the
    /// chain editor set them: they reach the sound loop at once, changing
    /// only the places that changed (`ToAudio::NoteFx`, `ToAudio::NoteArp`).
    /// False if there is no such track, or it is a Loop track (it plays no
    /// notes).
    pub fn set_note_fx(&mut self, track: usize, fx: TrackFx) -> bool {
        let Some(t) = self.project.tracks.get_mut(track) else {
            return false;
        };
        if t.loops().is_some() {
            return false;
        }
        if t.note_fx != fx {
            t.note_fx = fx;
            self.changed = true;
            self.sync_note_fx();
        }
        true
    }

    /// Take everything the audio half sent, in order, and do each
    /// period's bookkeeping. Allocates only for an edit (a compile, a
    /// started cue, a recording kept into the song).
    pub fn poll(&mut self) {
        if let Some((v, n)) = self.unsent.take() {
            self.push_view(v, n);
        }
        self.sync_tracks();
        self.loop_at = [None; LOOPS];
        while let Some(m) = self.rx.recv() {
            match m {
                ToControl::Played(offset, event) => self.capture.input(offset, event),
                ToControl::PlayedOn(track, offset, event) => {
                    self.capture.input_on(track, offset, event);
                }
                ToControl::Moved(offset, input) => self.move_log.input(offset, input),
                ToControl::Started(st) => self.started.push(st),
                ToControl::MuteLanded(l) => self.landed.push(l),
                ToControl::Touch(points, n) => {
                    self.touch.extend_from_slice(&points[..usize::from(n)]);
                }
                ToControl::Points(points, n) => {
                    self.moves.extend_from_slice(&points[..usize::from(n)]);
                }
                ToControl::Rec(number, rec) => {
                    self.includes.recorded = number;
                    self.pass_take = Some(rec.take());
                    let done = self
                        .takes
                        .record(&mut self.project, &rec, &self.touch, &self.moves);
                    if !self.touch.is_empty() && !self.touch_grew.contains(&rec.clip()) {
                        self.touch_grew.push(rec.clip());
                    }
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
                ToControl::Follow(c) => self.follow = c,
                ToControl::Cues(c) => self.cue_counts = c,
                ToControl::Period(end) => self.period(end),
                ToControl::Done(v) => drop(v),
                ToControl::Chain(c) => drop(c),
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
        self.heard = end.start_frame + u64::from(end.frames);
        self.info = end.info;
        self.counts = end.counts;
        // Out of recording: a take UNDO threw away goes now that all of
        // it has come.
        let mut thrown = false;
        if !matches!(self.info.state, State::Recording | State::CountIn) {
            if std::mem::take(&mut self.throw) {
                thrown = self.throw_pass();
            }
            self.pass_take = None;
        }
        self.capture.process(&self.block);
        self.move_log.process(&self.block);
        // The Capture button shows what it would keep: a new phrase, or one
        // more bar of it, is a change for the screen (once a bar at most).
        let pending = self.capture.pending();
        if pending != self.pending {
            self.pending = pending;
            self.changed = true;
        }
        // Live to song follows the transport and the cues that started,
        // within the song's room.
        let song_was = self.song_rec.recording();
        self.song_rec
            .song_blocks(self.project.arrangement.placements.len());
        self.song_rec.period(&self.block, &self.started);
        if let Some(used) = self.song_rec.take_warning() {
            let limit = self.song_rec.limit();
            self.note_later(format!(
                "rec song: the song is {}% full ({used} of {limit} blocks); \
                 recording stops on a launch line before it fills",
                used * 100 / limit.max(1)
            ));
        }
        if let Some(at) = self.song_rec.take_full() {
            let bar = self.info.meter.bar_ticks().max(1);
            self.note_later(format!(
                "rec song: the song is full ({} blocks), so recording stopped on \
                 every track at bar {}; what it recorded is kept",
                self.song_rec.limit(),
                at.div_euclid(bar) + 1
            ));
            self.changed = true;
        }
        // Cues that started on a bar line become the tracks' launches: the
        // view is compiled again.
        let mut compile = std::mem::take(&mut self.compile_asked) | thrown | self.reclaim_touch();
        for i in 0..self.started.len() {
            let st = self.started[i];
            // A Loop track's loops go on or off on the cue's bar line.
            let held = loops::on_track(&self.project, st.track);
            for (n, at) in self.loop_at.iter_mut().enumerate() {
                if held & (1 << n) != 0 {
                    *at = Some(st.at);
                }
            }
            self.perform.started(&mut self.project, &st);
            st.apply(&mut self.launches, &mut self.cues);
            self.decisions.started(&st);
            if let Some(n) = self.includes.switches.get_mut(st.track) {
                *n = n.wrapping_add(1);
            }
            compile = true;
        }
        self.started.clear();
        // The mute queue landed on its line, all of it on one tick: the
        // project takes its changes, Rec song the silences, and a Loop
        // track's loops go on or off there.
        for i in 0..self.landed.len() {
            let l = self.landed[i];
            self.song_rec.mute(l.track, l.silent, l.at);
            let held = loops::on_track(&self.project, l.track);
            for (n, at) in self.loop_at.iter_mut().enumerate() {
                if held & (1 << n) != 0 {
                    *at = Some(l.at);
                }
            }
            if self.mute_queue.as_ref().is_some_and(|(id, _)| *id == l.id) {
                let line = self.queue_lines(l.at);
                self.land_queue_now();
                self.note_later(line);
                compile = true;
            }
        }
        self.landed.clear();
        // STOP lands whatever waits.
        if self.mute_queue.is_some() && !self.info.state.rolling() {
            let line = self.queue_lines(self.info.position);
            self.land_queue_now();
            self.note_later(format!("{line} (STOP)"));
            compile = true;
        }
        // The kit's pads in the queue land in the period before their
        // line: the mixer takes them before it mixes the period that holds
        // it.
        self.pads_due(end.frames);
        // Reloads land just before their line: the next period, which
        // holds the line, plays the saved state from its start.
        if !self.reloads.is_empty() {
            compile |= self.reloads_due(end.frames);
        }
        // Song mode: the Loop tracks' blocks turn their loops on and off
        // on their own ticks, on the tracks with no clip launched.
        self.song_loops = [false; LOOPS];
        if self.song {
            let (launches, project) = (&self.launches, &self.project);
            let free = |t: usize| {
                !launches.iter().any(|l| l.track == t)
                    && t < project.tracks.len()
                    && !project.silent(t)
            };
            let spans = self.block.spans().iter().map(|s| s.ticks());
            let plan = loops::song_plan(&self.project, spans, free);
            self.song_loops = plan.on;
            for (at, song) in self.loop_at.iter_mut().zip(plan.at) {
                if song.is_some() {
                    *at = song;
                }
            }
        }
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
            // A follow's or the song list's scene: its mutes for tracks it
            // gave no cue wait in the mute queue for its line.
            compile |= self.queue_scene_mutes();
        }
        let recorded = std::mem::take(&mut self.recorded_now);
        // The transport stopped: what live to song recorded goes into the
        // song (it compiles).
        if song_was && !self.song_rec.recording() && self.song_rec.waiting() {
            let line = match self.keep_song() {
                Ok(line) => format!("rec song: {line}"),
                Err(e) => e,
            };
            self.note_later(line);
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
                // The step grid shows what recording added (and, without
                // overdub, took away) as it goes, not only after it stops.
                self.steps_changed = true;
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

    /// Out of recording, each clip played into whose pressure is past
    /// half its room frees the values no note can play again, its own,
    /// its Restore's or one an undo or redo would bring back
    /// (`Takes::reclaim_touch`, refactor B4b), so long sessions of
    /// recording and undoing never fill it. Never while recording: a
    /// replace pass names the notes it erases by value, pressure run and
    /// all, as the playing view holds them. True when anything was freed:
    /// the view is compiled again with the new numbering.
    fn reclaim_touch(&mut self) -> bool {
        if matches!(self.info.state, State::Recording | State::CountIn) {
            return false;
        }
        let mut freed = false;
        for (track, slot) in std::mem::take(&mut self.touch_grew) {
            let full = self
                .project
                .clip(track, slot)
                .is_some_and(|c| c.touch.len() > mpfree_play::TOUCH_RECLAIM_AT);
            if !full {
                continue;
            }
            let n = self.takes.reclaim_touch(&mut self.project, track, slot);
            if n > 0 {
                self.touch_freed.0 += n as u64;
                self.touch_freed.1 += 1;
                freed = true;
            }
        }
        freed
    }

    /// Pressure values freed so far, and how many times (B4b).
    #[cfg(test)]
    pub(super) fn touch_freed(&self) -> (u64, u64) {
        self.touch_freed
    }

    pub(super) fn compile(&mut self) -> Result<(), String> {
        let at = Instant::now();
        // An undone capture takes its clip, and so its launch, away; an
        // undone + Track takes its track.
        let project = &self.project;
        let n = project.tracks.len();
        self.launches
            .retain(|l| project.clip(l.track, l.slot).is_some());
        self.cues.retain(|c| {
            c.track < n
                && c.slot
                    .is_none_or(|slot| project.clip(c.track, slot).is_some())
        });
        self.find_arm_and_pick();
        // Tracks added or taken away: the other armed tracks let go.
        if self.also != 0 && self.also_n != n {
            self.also = 0;
        }
        let mut v = self.view()?;
        // Each new decision in it gets its revision and the frame the
        // music had reached when it was made: a cue whose line played
        // before the view came lands late, in phase (Arrange 2.5).
        self.decisions.stamp(&mut v, self.heard, &mut self.includes);
        // Room to record into every armed track's clips without
        // allocating.
        let mut armed = self.armed_set();
        while armed != 0 {
            let t = armed.trailing_zeros() as usize;
            armed &= armed - 1;
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
        let played = v.notes() + v.locks();
        self.sync_tracks();
        self.send_view(Box::new(v));
        self.compile_asked = false;
        self.changed = true;
        self.recorded = false;
        self.steps_changed = true;
        let took = at.elapsed();
        let c = &mut self.costs;
        c.compiles += 1;
        if took > c.slowest_compile {
            (c.slowest_compile, c.slowest_compile_notes) = (took, played);
        }
        Ok(())
    }

    /// Step edit `edit` of clip `slot` of `track`, already made to the
    /// project, sent to the audio half to make to its view in place, so a
    /// tap or a turn needs no compile. A clip the view does not play, cue
    /// or (in song mode) place in the song needs nothing sent; one the
    /// song places goes into the one copy every block of it plays (Arrange
    /// 2.3). False when it must compile instead: an edit of the clip's
    /// length, pressure or moves; while recording or counting in,
    /// when the audio half edits the view itself; a view still waiting to
    /// go; a track neither armed nor selected apart from the arm, whose
    /// clips the view has no room in; an edit too big; or a queue that fills (the audio half
    /// makes none of the changes that went: an edit goes in whole or not
    /// at all, and the compiled view brings it).
    fn in_place(&mut self, track: usize, slot: usize, edit: &Edit) -> bool {
        let (notes, locks) = self
            .project
            .clip(track, slot)
            .map_or((0, 0), |c| (c.notes.len(), c.locks.len()));
        if !InPlace::fits(edit, notes, locks)
            || self.unsent.is_some()
            || (self.armed != Some(track) && self.track_pick != Some(track))
            || matches!(self.info.state, State::Recording | State::CountIn)
        {
            return false;
        }
        let shown = self
            .launches
            .iter()
            .any(|l| (l.track, l.slot) == (track, slot))
            || self
                .cues
                .iter()
                .any(|c| c.track == track && c.slot == Some(slot))
            || (self.song && self.song_places(track, slot));
        if !shown {
            return true;
        }
        let (Ok(t), Ok(s)) = (u16::try_from(track), u16::try_from(slot)) else {
            return false;
        };
        // At most MAX_IN_PLACE (64): `fits` checked it.
        let n = u8::try_from(InPlace::count(edit)).unwrap_or(u8::MAX);
        let changes = InPlace::all(edit);
        self.edit_id = self.edit_id.wrapping_add(1);
        let id = self.edit_id;
        for change in changes {
            if self.tx.send(ToAudio::EditPart { id, change }).is_err() {
                return false;
            }
        }
        let end = ToAudio::EditEnd {
            id,
            track: t,
            slot: s,
            view: self.view_sent,
            n,
        };
        self.tx.send(end).is_ok()
    }

    /// Whether a block of the song plays clip `slot` of `track`, on any
    /// lane.
    fn song_places(&self, track: usize, slot: usize) -> bool {
        let Some(id) = self.project.clip(track, slot).map(|c| c.id) else {
            return false;
        };
        id.is_given()
            && self
                .project
                .arrangement
                .placements
                .iter()
                .any(|p| p.clip == id)
    }

    /// The view of the project as it is, with what it includes and room
    /// to record into in place.
    pub(super) fn view(&self) -> Result<PlayView, String> {
        let mut v = PlayView::compile_cued(&self.project, &self.launches, &self.cues)
            .map_err(|e| format!("clips: {e:?}"))?;
        v.set_armed(self.armed);
        v.set_also(self.also);
        v.set_song(&self.project, self.song)
            .map_err(|e| format!("song: {e:?}"))?;
        v.set_tempo_map(self.song_tempo()?);
        // A scene's mutes switch with its cues; while a track is soloed,
        // one it unmutes stays silent unless it is soloed too.
        self.perform.dress(&mut v);
        let project = &self.project;
        if project.tracks.iter().any(|t| t.solo) {
            for (t, tr) in project.tracks.iter().enumerate() {
                if let Some(m) = self.perform.cue_mute(t) {
                    v.set_cue_mute(t, Some(m || !tr.solo));
                }
            }
        }
        // The mute queue: every track it changes, and every track a solo
        // in it silences or lets be heard, lands on the queue's line.
        if let Some((id, q)) = &self.mute_queue {
            let after = queue_landed(project, q);
            for t in 0..project.tracks.len() {
                let silent = after.silent(t);
                if silent != project.silent(t) || q.want.iter().any(|w| w.0 == t) {
                    v.set_mute_cue(
                        t,
                        MuteCue {
                            silent,
                            line: q.line,
                            id: *id,
                        },
                    );
                }
            }
        }
        v.set_includes(self.includes);
        v.make_room(RECORD_ROOM, RECORD_ROOM);
        // The track selected apart from the arm gets the same room, so
        // its step edits go in place too (refactor C3).
        if let Some(t) = self.track_pick {
            v.make_room_on(t, RECORD_ROOM, RECORD_ROOM);
        }
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

    /// REC with nothing to record into (a fresh project, found by the UI
    /// audit): the armed track's clip the pattern shows is launched, or a
    /// new clip goes in its first empty slot, as a tap on an empty slot on
    /// Arrange does. Without it the recorder had no clip and REC kept
    /// nothing. Returns what it did, if anything.
    /// The leader's start began a take (follow.rs): the clip it records
    /// into, as REC's punch-in readies it. A line for the log.
    pub(super) fn take_started(&mut self) -> Result<String, String> {
        Ok(match self.ready_to_record()? {
            Some(line) => format!("rec: the take started with the clock, {line}"),
            None => "rec: the take started with the clock".to_string(),
        })
    }

    /// A note started the song with a take ready (the looper's Starts:
    /// When I play, [`super::TimeHost::start_at`]): the clip it records
    /// into, as for the leader's start. A line for the log.
    pub(super) fn take_started_on_note(&mut self) -> Result<String, String> {
        Ok(match self.ready_to_record()? {
            Some(line) => format!("rec: the take started on your note, {line}"),
            None => "rec: the take started on your note".to_string(),
        })
    }

    fn ready_to_record(&mut self) -> Result<Option<String>, String> {
        let mut lines = Vec::new();
        if let Some(line) = self.ready_armed()? {
            lines.push(line);
        }
        // The other armed tracks: each records into its playing clip, else
        // a new clip in its first empty slot; one with neither is said and
        // counted, and the others still record.
        let mut also = self.also();
        let mut made = false;
        let info = self.info;
        let bar = info.meter.bar_ticks();
        let length = info.loop_range.map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
        while also != 0 {
            let track = also.trailing_zeros() as usize;
            also &= also - 1;
            if self.launches.iter().any(|l| l.track == track) {
                continue;
            }
            let Some(t) = self.project.tracks.get_mut(track) else {
                continue;
            };
            let Some(s) = t.slots.first_free() else {
                self.also_full += 1;
                lines.push(format!(
                    "{} has no clip playing and no empty slot: it doesn't record this take",
                    t.name
                ));
                continue;
            };
            let clip = Clip::new(&format!("Clip {}", s + 1), length);
            self.project
                .put_new_clip(track, s, clip)
                .map_err(|e| format!("new clip: {e:?}"))?;
            let launch = TimeAction::Launch { track, slot: s };
            (self.launches, self.cues) =
                clips_after(&self.project, &self.launches, &self.cues, &launch, &info);
            lines.push(format!("new clip in slot {} of track {}", s + 1, track + 1));
            made = true;
        }
        if made {
            self.compile()?;
        }
        Ok((!lines.is_empty()).then(|| lines.join("; ")))
    }

    /// [`TimeControl::ready_to_record`] for the armed track.
    fn ready_armed(&mut self) -> Result<Option<String>, String> {
        let Some(track) = self.armed else {
            return Ok(None);
        };
        if self.launches.iter().any(|l| l.track == track) {
            return Ok(None);
        }
        let info = self.info;
        let (slot, line) = match self.focus_of(track).1 {
            Some(s) => (s, format!("slot {} of track {} launched", s + 1, track + 1)),
            None => {
                let bar = info.meter.bar_ticks();
                let length = info.loop_range.map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t
                    .slots
                    .first_free()
                    .ok_or_else(|| format!("{} has no empty slot to record into", t.name))?;
                let clip = Clip::new(&format!("Clip {}", s + 1), length);
                self.project
                    .put_new_clip(track, s, clip)
                    .map_err(|e| format!("new clip: {e:?}"))?;
                (
                    s,
                    format!("new clip in slot {} of track {}", s + 1, track + 1),
                )
            }
        };
        let launch = TimeAction::Launch { track, slot };
        (self.launches, self.cues) =
            clips_after(&self.project, &self.launches, &self.cues, &launch, &info);
        self.compile()?;
        Ok(Some(line))
    }

    /// PLAY from stopped with no clip playing or waiting anywhere (a
    /// project just opened, at start too, or every clip stopped): the
    /// pattern the Play page shows starts with it, as it does in a new
    /// project, where its first step launched it. Without this the grid's
    /// line moved over steps that never played (the 16 Nov device round).
    /// Song mode plays the song instead, and a clip launched on Arrange
    /// is left as it is.
    fn ready_to_play(&mut self) -> Result<Option<String>, String> {
        // The Loop tracks' launches don't count: a pattern still starts
        // with the loops playing.
        let loops = |t: usize| self.project.tracks.get(t).is_some_and(|t| t.is_loops());
        if self.song
            || self.launches.iter().any(|l| !loops(l.track))
            || self.cues.iter().any(|c| !loops(c.track))
        {
            return Ok(None);
        }
        let (track, Some(slot)) = self.step_focus() else {
            return Ok(None);
        };
        let info = self.info;
        let launch = TimeAction::Launch { track, slot };
        let before = (self.launches.clone(), self.cues.clone());
        (self.launches, self.cues) =
            clips_after(&self.project, &self.launches, &self.cues, &launch, &info);
        if let Err(e) = self.compile() {
            (self.launches, self.cues) = before;
            return Err(e);
        }
        Ok(Some(format!(
            "slot {} of track {} launched (the Play page's pattern)",
            slot + 1,
            track + 1
        )))
    }

    /// A step put on the Play page's pattern while its track plays no
    /// clip and has none waiting: the pattern starts (at the next bar
    /// line while the music plays), so what was added is heard, as a new
    /// pattern's first step does. Not in song mode.
    /// True if it launched it.
    fn launch_edited(&mut self, track: usize, slot: usize) -> bool {
        if self.song
            || self.launches.iter().any(|l| l.track == track)
            || self.cues.iter().any(|c| c.track == track)
        {
            return false;
        }
        let info = self.info;
        let launch = TimeAction::Launch { track, slot };
        (self.launches, self.cues) =
            clips_after(&self.project, &self.launches, &self.cues, &launch, &info);
        true
    }

    /// "take 3 · Drums / 1", as the takes list names it: what UNDO and
    /// REDO say they did.
    /// A drop on the Clips grid went in: the launches and cues follow the
    /// clips, and the views compile. If they don't, the drop is taken
    /// back (and can't be redone) and the error says why.
    fn drop_went_in(&mut self, drop: ClipDrop) -> Result<(), String> {
        // A clip moved or copied keeps no saved state, nor what it lands
        // on; rows moved forget them all.
        match drop {
            ClipDrop::Moved(a, b) | ClipDrop::Swapped(a, b) | ClipDrop::Copied(a, b) => {
                self.forget_saved(Some(a));
                self.forget_saved(Some(b));
            }
            ClipDrop::RowMoved { .. } | ClipDrop::RowCopied { .. } => self.forget_saved(None),
        }
        let before = (self.launches.clone(), self.cues.clone());
        self.follow_launches(drop, false);
        if let Err(e) = self.compile() {
            self.takes.undo(&mut self.project);
            self.takes.forget_undone();
            (self.launches, self.cues) = before;
            let _ = self.compile();
            return Err(e);
        }
        Ok(())
    }

    /// A clip that was playing goes on playing where a drop (or its undo,
    /// `back`) put it, on its own track; moved to another track, it plays
    /// there unless that track has a clip playing. A launch waiting for
    /// the bar line follows a clip on its track, and is dropped if the
    /// clip left it.
    fn follow_launches(&mut self, drop: ClipDrop, back: bool) {
        let to = |c: (usize, usize)| {
            if back {
                drop.before(c)
            } else {
                Some(drop.after(c))
            }
        };
        let stays = |l: &Launch| to((l.track, l.slot)) == Some((l.track, l.slot));
        let mut out: Vec<Launch> = self.launches.iter().copied().filter(stays).collect();
        for l in self.launches.iter().filter(|l| !stays(l)) {
            if let Some((track, slot)) = to((l.track, l.slot)) {
                if !out.iter().any(|o| o.track == track) {
                    out.push(Launch { track, slot, ..*l });
                }
            }
        }
        out.sort_by_key(|l| l.track);
        self.launches = out;
        self.cues.retain_mut(|c| match c.slot {
            Some(s) => match to((c.track, s)) {
                Some((t, s2)) if t == c.track => {
                    c.slot = Some(s2);
                    true
                }
                _ => false,
            },
            None => true,
        });
        self.changed = true;
    }

    fn take_said(&self, row: Option<TakeRow>) -> String {
        let Some(r) = row else {
            return "take".into();
        };
        let track_name = |track: usize| {
            self.project
                .tracks
                .get(track)
                .map_or_else(|| format!("Track {}", track + 1), |t| t.name.clone())
        };
        match r.song {
            Some(
                SongChange::Moved { track, .. }
                | SongChange::Copied { track, .. }
                | SongChange::Resized { track, .. }
                | SongChange::Deleted { track, .. }
                | SongChange::Unique { track, .. },
            ) => return format!("{} · {} in the song", r.short(), track_name(track)),
            Some(SongChange::SectionMoved { .. } | SongChange::SectionCopied { .. }) => {
                return format!("{} · a section of the song", r.short())
            }
            Some(SongChange::Scenes { .. }) => return format!("{} → song", r.short()),
            Some(SongChange::List) => return format!("{} · the song list", r.short()),
            Some(SongChange::ListToSong { .. }) => return format!("{} · list → song", r.short()),
            Some(SongChange::Tempo { at }) => {
                return format!(
                    "{} · the song's tempo at {}",
                    r.short(),
                    bar_beat(at, self.info.meter)
                )
            }
            None => {}
        }
        if let Some((first, last)) = r.rows {
            return format!("{} · {}", r.short(), rows_named(first, last));
        }
        if let Some(row) = r.scene {
            return format!("{} · scene {}", r.short(), row + 1);
        }
        match r.at {
            Some((track, slot)) => {
                let name = self
                    .project
                    .tracks
                    .get(track)
                    .map_or_else(|| format!("Track {}", track + 1), |t| t.name.clone());
                format!("{} · {name} / {}", r.short(), slot + 1)
            }
            None => r.short(),
        }
    }

    /// The clip the Play page's pattern shows and the Sequencer's Steps,
    /// Notes and STEP SEQ edit: the selected track's (track 1 if none is
    /// selected), as [`TimeControl::focus_of`].
    fn step_focus(&self) -> (usize, Option<usize>) {
        self.focus_of(self.selected().unwrap_or(0))
    }

    /// Track `track`'s clip the Play page's pattern would show: its
    /// launched clip, else its first.
    fn focus_of(&self, track: usize) -> (usize, Option<usize>) {
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
                .shown()
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
            let launch = self
                .launches
                .iter()
                .find(|l| l.track == track && l.slot == slot);
            let origin = launch.map(|l| l.origin);
            let order =
                launch.and_then(|l| mpfree_play::StepOrder::of(c, track, slot, l.origin, l.start));
            let shown = grid.shown(c, &mpfree_seq::steps::PAD_ROWS);
            Some(StepClip {
                slot,
                name: c.name.clone(),
                steps: grid.steps(c.length),
                song_length: c.speed.to_song(c.length as f64),
                song_step: c.speed.to_song(STEP_TICKS as f64),
                origin,
                order,
                rows: shown.rows,
                locks: shown.locks,
                lock_values: shown.lock_values,
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
    ///
    /// One action is one transaction, for the clip and for what plays. On
    /// a track with no clip, the first step makes the clip, its notes and
    /// its launch as one take, which one UNDO takes back whole (REDO
    /// brings it back playing); an edit that changes nothing makes
    /// nothing, not even the clip. If the play view can't be built after
    /// the edit, the edit is taken back from the clip and the takes (and
    /// the launches as they were), and the error says so.
    pub fn step_action(&mut self, a: StepAction) -> Result<String, String> {
        let grid = Grid::new(STEP_TICKS).expect("a sixteenth is a step size");
        let (track, slot) = self.step_focus();
        let (slot, made) = match slot {
            Some(s) => (s, None),
            None => {
                let (StepAction::Tap { .. }
                | StepAction::Enter { n: 1.., .. }
                | StepAction::Keys { n: 1.., .. }) = a
                else {
                    return Ok(String::new());
                };
                let t = self.project.tracks.get(track).ok_or("no such track")?;
                let s = t
                    .slots
                    .first_free()
                    .ok_or_else(|| format!("{} has no empty slot", t.name))?;
                // Not in the slot yet: it goes in with its first notes.
                let name = format!("Pattern {}", s + 1);
                (s, Some(Clip::new(&name, grid.page_length())))
            }
        };
        let clip = match &made {
            Some(c) => c,
            None => self.project.clip(track, slot).ok_or("the clip is gone")?,
        };
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
            StepAction::Enter { step, keys, n } => {
                let keys = &keys[..usize::from(n).min(keys.len())];
                let pads: Vec<String> = keys.iter().map(|k| (k.0 + 1).to_string()).collect();
                (
                    grid.enter(clip, step, keys),
                    None,
                    if keys.is_empty() {
                        format!("step {} of the line", step + 1)
                    } else {
                        format!("step {} of the line: pads {}", step + 1, pads.join(", "))
                    },
                )
            }
            StepAction::Keys { steps, keys, n } => {
                let keys = &keys[..usize::from(n).min(keys.len())];
                let pads: Vec<String> = keys.iter().map(|k| (k.0 + 1).to_string()).collect();
                let list = step_list(steps);
                let names: Vec<String> = list.iter().map(|i| (i + 1).to_string()).collect();
                let s = if list.len() == 1 { "" } else { "s" };
                (
                    grid.enter_steps(clip, &list, keys),
                    Some(mpfree_seq::steps::enter_key(steps)),
                    format!(
                        "step{s} {} of the line: pads {} from the keys",
                        names.join(", "),
                        pads.join(", ")
                    ),
                )
            }
            StepAction::Change { pad, step, change } if pad == LINE => (
                grid.change_all(clip, step, change),
                Some(change.key_all(step)),
                format!("step {} of the line: {}", step + 1, change_name(change)),
            ),
            StepAction::Change { pad, step, change } => (
                grid.change(clip, pad, step, change),
                Some(change.key(pad, step)),
                format!("step {} of pad {}: {change:?}", step + 1, pad + 1),
            ),
            StepAction::ChangeSteps { pad, steps, change } => {
                let held: Vec<usize> = (0..128).filter(|i| steps & (1u128 << i) != 0).collect();
                (
                    if pad == LINE {
                        grid.change_line(clip, &held, change)
                    } else {
                        grid.change_steps(clip, pad, &held, change)
                    },
                    Some(change.key_steps(pad, steps)),
                    format!(
                        "steps {} of {}: {}",
                        held.iter()
                            .map(|i| (i + 1).to_string())
                            .collect::<Vec<_>>()
                            .join(", "),
                        row_name(pad),
                        change_name(change)
                    ),
                )
            }
            StepAction::Lock {
                pad,
                steps,
                addr,
                value,
            } => {
                let held = step_list(steps);
                let sound = (pad != LINE).then_some(pad);
                (
                    grid.lock_steps(clip, sound, &held, addr, value),
                    // A turn's detents join one take; a lock taken off
                    // (the step inspector's ×) is a take of its own, so
                    // UNDO brings the lock back rather than the sound.
                    value.map(|_| mpfree_seq::steps::lock_key_steps(sound, steps, addr)),
                    format!(
                        "{} of {}: lock {addr} {}",
                        steps_text(&held),
                        row_name(pad),
                        value.map_or("off".to_string(), |v| format!("{v}"))
                    ),
                )
            }
            StepAction::ClearLocks { pad, steps } => {
                let held = step_list(steps);
                (
                    grid.clear_step_locks(clip, (pad != LINE).then_some(pad), &held),
                    None,
                    format!("{} of {}", steps_text(&held), row_name(pad)),
                )
            }
            StepAction::ClearPad { pad } => (
                grid.clear_row(clip, pad),
                None,
                format!("pad {}'s steps", pad + 1),
            ),
            StepAction::ClearSteps { pad, from, to } => (
                if pad == LINE {
                    grid.clear_line(clip, from, to)
                } else {
                    grid.clear_steps(clip, pad, from, to)
                },
                None,
                if to == from + 1 {
                    format!("step {} of {}", from + 1, row_name(pad))
                } else {
                    format!("steps {} to {to} of {}", from + 1, row_name(pad))
                },
            ),
            StepAction::CopySteps {
                pad,
                from,
                to,
                count,
            } => (
                if pad == LINE {
                    grid.copy_line(clip, from, to, count)
                } else {
                    grid.copy_steps(clip, pad, from, to, count)
                },
                None,
                if count == 1 {
                    format!("step {} of {} to step {}", from + 1, row_name(pad), to + 1)
                } else {
                    format!(
                        "steps {} to {} of {} to steps {} to {}",
                        from + 1,
                        from + count,
                        row_name(pad),
                        to + 1,
                        to + count
                    )
                },
            ),
            StepAction::Length { steps, copy } => {
                (grid.resize(clip, steps, copy), None, clip.name.clone())
            }
        };
        // A step past the clip's end, or a change to nothing: no take, and
        // on an empty track no clip either.
        if edit.is_empty() {
            return Ok(String::new());
        }
        let before = (self.launches.clone(), self.cues.clone());
        let (edits, lock_take) = (self.takes.edits(), self.lock_take);
        let (take, made) = match made {
            Some(mut c) => {
                c.apply(&edit).map_err(|e| format!("steps: {e:?}"))?;
                let name = c.name.clone();
                let take = self
                    .takes
                    .new_clip(&mut self.project, track, slot, c, true)
                    .map_err(|e| format!("steps: {e:?}"))?;
                let info = self.info;
                let launch = TimeAction::Launch { track, slot };
                (self.launches, self.cues) =
                    clips_after(&self.project, &self.launches, &self.cues, &launch, &info);
                (take, Some(name))
            }
            None => (
                self.takes
                    .edit(&mut self.project, track, slot, &edit, join)
                    .map_err(|e| format!("steps: {e:?}"))?,
                None,
            ),
        };
        let joined = matches!(a, StepAction::Lock { .. }) && take == self.lock_take;
        if matches!(a, StepAction::Lock { .. }) {
            self.lock_take = take;
        }
        let launched = made.is_none() && !edit.add.is_empty() && self.launch_edited(track, slot);
        if made.is_none() && !launched && self.in_place(track, slot, &edit) {
            self.changed = true;
            self.steps_changed = true;
            self.costs.steps_in_place += 1;
        } else {
            self.costs.steps_compiled += 1;
            if let Err(e) = self.compile() {
                // What plays can't follow: the edit goes from the clip and
                // the takes (a new clip with it), the launches go back, and
                // the view as it was is built again.
                let fresh = self.takes.edits() != edits;
                let (edit, join) = match made {
                    Some(_) => (Edit::default(), None),
                    None => (edit, join),
                };
                let back =
                    self.takes
                        .withdraw(&mut self.project, (track, slot), &edit, join, fresh);
                (self.launches, self.cues) = before;
                self.lock_take = lock_take;
                let _ = self.compile();
                return Err(match back {
                    Ok(()) => format!("steps: {e}: the edit was taken back"),
                    Err(b) => format!("steps: {e}: the edit could not be taken back ({b:?})"),
                });
            }
        }
        let on = if edit.add.is_empty() { "off" } else { "on" };
        Ok(match (made, a) {
            (Some(name), _) => format!(
                "steps: new clip {name} in {}, {what} {on}",
                self.project.tracks[track].name
            ),
            (None, StepAction::Tap { .. } | StepAction::Enter { .. } | StepAction::Keys { .. }) => {
                format!("steps: {what} {on}")
            }
            (None, StepAction::ClearPad { .. } | StepAction::ClearSteps { .. }) => {
                format!("steps: {what} erased")
            }
            (None, StepAction::CopySteps { .. }) => format!("steps: {what} copied"),
            // One line a turn, at its first detent.
            (None, StepAction::Lock { .. }) if joined => String::new(),
            (None, StepAction::Lock { .. }) => format!("steps: {what}"),
            (None, StepAction::ClearLocks { .. }) => format!("steps: {what}: locks erased"),
            // The Note knob: a line a detent (a pad each).
            (
                None,
                StepAction::Change {
                    change: mpfree_seq::steps::Change::Note(_),
                    ..
                }
                | StepAction::ChangeSteps {
                    change: mpfree_seq::steps::Change::Note(_),
                    ..
                },
            ) => format!("steps: {what}"),
            // A step switched off and kept, or on again (the redesign's
            // step 3b): "steps: step 9 of pad 5: On(false)".
            (
                None,
                StepAction::Change {
                    change: mpfree_seq::steps::Change::On(_),
                    ..
                }
                | StepAction::ChangeSteps {
                    change: mpfree_seq::steps::Change::On(_),
                    ..
                },
            ) => format!("steps: {what}"),
            (None, StepAction::Length { steps, copy }) => {
                let bars = steps.div_ceil(16);
                let s = if bars == 1 { "" } else { "s" };
                let how = if copy {
                    " · the new steps repeat the ones before them"
                } else {
                    " · UNDO brings the rest back"
                };
                format!("steps: {what} is {steps} steps ({bars} bar{s}){how}")
            }
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
            // A track picked arms it, unless the app found REC's arm
            // pinned and sent `Select` instead (refactor C1).
            TimeAction::Pick(t) => self.act(TimeAction::Arm(Some(t))),
            // Waiting for another device's Start, a take records from it
            // (REC readies it, follow.rs starts it): not now.
            TimeAction::Transport(Command::Record)
                if self.clock_waits && info.state == State::Stopped =>
            {
                Err(
                    "transport: the take waits for the clock: REC readies it, and it \
                     records from the leader's Start"
                        .into(),
                )
            }
            TimeAction::Transport(Command::Record)
                if !matches!(info.state, State::Recording | State::CountIn) =>
            {
                let made = self.ready_to_record()?;
                if !self.command(Command::Record) {
                    return Err("transport: command queue full".into());
                }
                Ok(match made {
                    Some(line) => format!("transport: Record, {line}"),
                    None => "transport: Record".into(),
                })
            }
            TimeAction::Transport(cmd @ (Command::Play | Command::PlayStart))
                if !info.state.rolling() =>
            {
                let launched = self.ready_to_play()?;
                // Following another device's Start and Stop, its Start
                // plays what is ready (from bar 1 after PLAY START).
                if self.clock_waits {
                    if cmd == Command::PlayStart && !self.command(Command::Locate(0)) {
                        return Err("transport: command queue full".into());
                    }
                    return Ok(match launched {
                        Some(line) => format!("transport: {cmd:?} waits for the clock, {line}"),
                        None => format!("transport: {cmd:?} waits for the clock"),
                    });
                }
                if !self.command(cmd) {
                    return Err("transport: command queue full".into());
                }
                Ok(match launched {
                    Some(line) => format!("transport: {cmd:?}, {line}"),
                    None => format!("transport: {cmd:?}"),
                })
            }
            TimeAction::Transport(cmd) => {
                if !self.command(cmd) {
                    return Err("transport: command queue full".into());
                }
                Ok(format!("transport: {cmd:?}"))
            }
            TimeAction::ThrowTake => match info.state {
                State::Recording => {
                    if !self.command(Command::Record) {
                        return Err("transport: command queue full".into());
                    }
                    self.throw = true;
                    Ok("transport: punch out, the take thrown away".into())
                }
                State::CountIn => {
                    if !self.command(Command::Stop) {
                        return Err("transport: command queue full".into());
                    }
                    Ok("transport: count-in stopped, nothing recorded".into())
                }
                _ => Ok("transport: nothing records".into()),
            },
            TimeAction::Undo => match self.takes.undo(&mut self.project) {
                Some(_) => {
                    let row = self.takes.undone().last().map(TakeRow::of);
                    if let Some(d) = row.and_then(|r| r.dropped) {
                        self.follow_launches(d, true);
                    }
                    if row.is_some_and(|r| r.song.is_some()) {
                        // The song's blocks and sections changed places.
                        self.selected = None;
                        self.section = None;
                    }
                    if row.is_some_and(|r| r.captured) {
                        // It can be kept again.
                        self.capture.forget_kept();
                        self.pending = self.capture.pending();
                    }
                    let said = self.take_said(row);
                    self.compile().map(|_| format!("undo: {said}"))
                }
                None => Ok("undo: nothing to undo".into()),
            },
            TimeAction::Redo => match self.takes.redo(&mut self.project) {
                Some(_) => {
                    self.redos += 1;
                    // The Sequencer's first step launched the clip it made:
                    // back again, it plays again (unless its track has
                    // another clip playing or waiting by now).
                    let launched: Vec<(usize, usize)> = self
                        .takes
                        .done()
                        .last()
                        .filter(|t| t.launched)
                        .map(|t| t.made.iter().map(|&(tr, sl, _)| (tr, sl)).collect())
                        .unwrap_or_default();
                    for (track, slot) in launched {
                        if self.project.clip(track, slot).is_some() {
                            self.launch_edited(track, slot);
                        }
                    }
                    let row = self.takes.done().last().map(TakeRow::of);
                    if let Some(d) = row.and_then(|r| r.dropped) {
                        self.follow_launches(d, false);
                    }
                    if row.is_some_and(|r| r.song.is_some()) {
                        self.selected = None;
                        self.section = None;
                    }
                    let said = self.take_said(row);
                    self.compile().map(|_| format!("redo: {said}"))
                }
                None => Ok("redo: nothing to redo".into()),
            },
            TimeAction::Capture => self.keep(),
            TimeAction::ClipSettings {
                track,
                slot,
                row,
                settings,
            } => {
                let name = self
                    .project
                    .clip(track, slot)
                    .map(|c| c.name.clone())
                    .ok_or_else(|| {
                        format!("clip: slot {} of track {} is empty", slot + 1, track + 1)
                    })?;
                // A row turned again joins its take: the wheel through
                // many values is one undo.
                self.takes
                    .set_clip(
                        &mut self.project,
                        track,
                        slot,
                        settings,
                        Some(CARD_JOIN + u64::from(row)),
                    )
                    .map_err(|e| format!("clip: {name}: {e:?}"))?;
                self.compile()?;
                let length = self.project.clip(track, slot).map_or(0, |c| c.length);
                let said = card_rows(length, settings)
                    .get(usize::from(row))
                    .map_or_else(String::new, |r| format!("{} {}", r.name, r.value));
                Ok(format!("clip: {name} · {said} (UNDO takes it back)"))
            }
            TimeAction::SceneFocus(row) => {
                if row >= mpfree_seq::SCENES {
                    return Err(format!("no scene {}", row + 1));
                }
                self.scene_pick = Some(row);
                self.changed = true;
                Ok(format!("scene editor: scene {}", row + 1))
            }
            TimeAction::SceneSettings {
                row,
                card_row,
                settings,
            } => {
                // A row turned again joins its take: the wheel through
                // many values is one undo.
                self.takes
                    .set_scene(
                        &mut self.project,
                        row,
                        settings.clone(),
                        Some(SCENE_JOIN + u64::from(card_row)),
                    )
                    .map_err(|e| format!("scene {}: {e:?}", row + 1))?;
                self.scene_pick = Some(row);
                self.compile()?;
                // The settings as the editor now shows them.
                let st = self.state();
                let said = if card_row == SCENE_NAME {
                    match st.scene.name.as_str() {
                        "" => "name taken off".to_string(),
                        name => format!("named {name}"),
                    }
                } else {
                    scene_rows(&st)
                        .get(usize::from(card_row))
                        .map_or_else(String::new, |r| format!("{} {}", r.name, r.value))
                };
                Ok(format!("scene {} · {said} (UNDO takes it back)", row + 1))
            }
            TimeAction::ClipLength {
                track,
                slot,
                steps,
                copy,
            } => {
                if self.step_focus() != (track, Some(slot)) {
                    return Err("clip: the focused clip changed: try again".into());
                }
                self.step_action(StepAction::Length { steps, copy })
            }
            TimeAction::CopyClip { track, slot } => {
                let c = self.project.clip(track, slot).ok_or_else(|| {
                    format!("copy: slot {} of track {} is empty", slot + 1, track + 1)
                })?;
                if c.looped.is_some() {
                    return Err(
                        "copy: a loop cell copies with SHIFT + a drag onto another Loop track"
                            .into(),
                    );
                }
                let said = format!(
                    "copy: {} copied · COPY + an empty slot puts a copy there",
                    c.name
                );
                self.clipboard = Some(c.clone());
                self.changed = true;
                Ok(said)
            }
            TimeAction::PasteClip { track, slot } => {
                let c = self
                    .clipboard
                    .clone()
                    .ok_or("paste: nothing copied yet: COPY + a clip copies it")?;
                let t = self
                    .project
                    .tracks
                    .get(track)
                    .ok_or_else(|| format!("paste: no track {}", track + 1))?;
                if t.is_loops() {
                    return Err(format!("paste: {} plays loops, not clips", t.name));
                }
                let place = format!("{} / {}", t.name, slot + 1);
                self.takes
                    .paste_clip(&mut self.project, track, slot, c.clone())
                    .map_err(|e| match e {
                        ModelError::BadSlot => {
                            format!("paste: {place} holds a clip: COPY + an empty slot pastes")
                        }
                        e => format!("paste: {e:?}"),
                    })?;
                self.compile()?;
                Ok(format!(
                    "paste: a copy of {} in {place} (UNDO takes it away)",
                    c.name
                ))
            }
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
            TimeAction::RecordQuantise { track, grid } => {
                if grid.is_some_and(|g| mpfree_seq::Quantise::new(g, 100).is_none()) {
                    return Err("record quantise: not a grid".into());
                }
                let t = self
                    .project
                    .tracks
                    .get_mut(track)
                    .ok_or_else(|| format!("record quantise: no track {}", track + 1))?;
                if t.record_quantise != grid {
                    t.record_quantise = grid;
                    self.changed = true;
                    // The recorder reads the grid from the view.
                    self.compile()?;
                }
                let name = &self.project.tracks[track].name;
                Ok(match grid {
                    Some(g) => format!(
                        "record quantise: {name} records each note on the nearest {}",
                        mpfree_time_ui::view::grid_name(g)
                    ),
                    None => format!("record quantise: {name} records the notes as played"),
                })
            }
            // Live transpose (S4): the track and every one that follows
            // move; the audio half shifts their notes from the next one on.
            TimeAction::Transpose { track, to } => {
                if !to.ok() {
                    return Err(format!(
                        "transpose: {} is past {} either way",
                        to.by,
                        mpfree_seq::MAX_TRANSPOSE
                    ));
                }
                if track >= self.project.tracks.len() {
                    return Err(format!("transpose: no track {}", track + 1));
                }
                let mut with = Vec::new();
                for (i, t) in self.project.tracks.iter_mut().enumerate() {
                    if i != track && !t.follows {
                        continue;
                    }
                    if i != track {
                        with.push(t.name.clone());
                    }
                    if t.transpose != to {
                        t.transpose = to;
                        self.changed = true;
                    }
                }
                self.sync_tracks();
                let name = &self.project.tracks[track].name;
                let with = if with.is_empty() {
                    String::new()
                } else {
                    format!(", with {}", with.join(", "))
                };
                Ok(format!(
                    "transpose: {name} {}{with}",
                    mpfree_time_ui::view::transpose_text(to)
                ))
            }
            TimeAction::FollowsTranspose { track, on } => {
                let t = self
                    .project
                    .tracks
                    .get_mut(track)
                    .ok_or_else(|| format!("follows transpose: no track {}", track + 1))?;
                if t.follows != on {
                    t.follows = on;
                    self.changed = true;
                }
                let name = &t.name;
                Ok(if on {
                    format!("follows transpose: {name} moves with the armed track's transpose")
                } else {
                    format!("follows transpose: {name} keeps its own")
                })
            }
            // The project's launch quantise (S4). What waits already keeps
            // the line it had (the design's edge case).
            TimeAction::LaunchGrid(g) => {
                if !g.ok() {
                    return Err("launch: not a launch quantise".into());
                }
                let was = self.project.launch;
                if was != g {
                    for c in &self.cues {
                        let own = c
                            .slot
                            .and_then(|s| self.project.clip(c.track, s))
                            .and_then(|m| m.launch);
                        if own.is_none() {
                            self.perform.hold_line(c.track, was);
                        }
                    }
                    self.project.launch = g;
                    self.changed = true;
                    self.compile()?;
                }
                let name = mpfree_time_ui::view::launch_name(g);
                Ok(match g {
                    LaunchGrid::Now => {
                        "launch: a tapped clip, a stop or a scene starts at once".into()
                    }
                    _ => format!(
                        "launch: a tapped clip, a stop or a scene starts on the next {name}"
                    ),
                })
            }
            TimeAction::Arm(Some(t))
                if self.project.tracks.get(t).is_some_and(|t| t.is_loops()) =>
            {
                // A Loop track plays loops: the pads stay on their track.
                let name = &self.project.tracks[t].name;
                Ok(match self.armed.and_then(|a| self.project.tracks.get(a)) {
                    Some(a) => format!("armed: {name} plays loops, the pads stay on {}", a.name),
                    None => format!("armed: {name} plays loops"),
                })
            }
            TimeAction::ArmAlso(tracks) => {
                if matches!(self.info.state, State::Recording | State::CountIn) {
                    return Err("armed: finish the take first".into());
                }
                let n = self.project.tracks.len();
                let mut keep = 0u128;
                for (t, tr) in self.project.tracks.iter().enumerate() {
                    if tracks & (1 << t) != 0 && !tr.is_loops() {
                        keep |= 1 << t;
                    }
                }
                self.also = keep;
                self.also_n = n;
                self.compile()?;
                let names: Vec<&str> = (0..n)
                    .filter(|&t| self.also() & (1 << t) != 0)
                    .map(|t| self.project.tracks[t].name.as_str())
                    .collect();
                Ok(if names.is_empty() {
                    "armed: the pads' track alone".into()
                } else {
                    format!("armed too: {}", names.join(", "))
                })
            }
            TimeAction::Select(t) if self.project.tracks.get(t).is_some_and(|t| t.is_loops()) => {
                // A Loop track has no steps: the Sequencer stays where it is.
                let name = &self.project.tracks[t].name;
                Ok(
                    match self.selected().and_then(|s| self.project.tracks.get(s)) {
                        Some(s) => format!("{name} plays loops: the Sequencer stays on {}", s.name),
                        None => format!("{name} plays loops"),
                    },
                )
            }
            TimeAction::Select(t) => {
                let name = self
                    .project
                    .tracks
                    .get(t)
                    .ok_or_else(|| format!("no track {}", t + 1))?
                    .name
                    .clone();
                let was = self.track_pick;
                self.set_pick(t);
                // The steps, the note editor's clip and the pads follow.
                // A new pick compiles, so the view has room for its step
                // edits to go in place (refactor C3).
                if self.track_pick != was && self.track_pick.is_some() {
                    self.compile()?;
                }
                self.changed = true;
                self.steps_changed = true;
                Ok(match self.armed.and_then(|a| self.project.tracks.get(a)) {
                    Some(a) if self.armed != Some(t) => {
                        format!("selected: {name} (REC stays on {}, pinned)", a.name)
                    }
                    _ => format!("selected: {name}"),
                })
            }
            TimeAction::Arm(track) => {
                self.set_armed(track);
                self.compile()?;
                Ok(match self.armed {
                    Some(t) => format!("armed: {}", self.project.tracks[t].name),
                    None => "armed: none".into(),
                })
            }
            TimeAction::LaunchScene(_)
            | TimeAction::StopAll
            | TimeAction::BackToSong
            | TimeAction::ListPlay(_) => {
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
                    self.mute_queue.clone(),
                );
                let song_was = self.song;
                let (launches, cues) = (&mut self.launches, &mut self.cues);
                let line = match a {
                    TimeAction::LaunchScene(row) => {
                        // A scene by hand: the song list stops going on.
                        self.perform.end_list();
                        self.scene_pick = Some(row);
                        if !self
                            .perform
                            .launch_scene(&mut self.project, row, when, launches, cues)
                        {
                            self.perform = before.2;
                            return Err(format!("no scene {}", row + 1));
                        }
                        format!("scene {}", row + 1)
                    }
                    TimeAction::ListPlay(step) => {
                        if !self
                            .perform
                            .play_list(&mut self.project, step, when, launches, cues)
                        {
                            return Err(format!("song list: no step {}", step + 1));
                        }
                        let row = self.project.arrangement.list.steps[step].row;
                        format!("song list from step {} (row {})", step + 1, row + 1)
                    }
                    TimeAction::BackToSong => {
                        // Every clip stops on the line, and the song plays
                        // the tracks from there.
                        self.perform.stop_all(when, launches, cues);
                        self.song = true;
                        match when {
                            When::Cued => "back to song: every track plays the song from the \
                                           next launch line"
                                .to_string(),
                            When::Now(_) => "back to song".to_string(),
                        }
                    }
                    _ => {
                        self.perform.stop_all(when, launches, cues);
                        "stop all".to_string()
                    }
                };
                // The scene's mutes for tracks it gave no cue wait in the
                // mute queue for its line.
                self.queue_scene_mutes();
                if let Err(e) = self.compile() {
                    self.song = song_was;
                    (self.launches, self.cues, self.perform) = (before.0, before.1, before.2);
                    for (t, m) in self.project.tracks.iter_mut().zip(before.3) {
                        t.mute = m;
                    }
                    self.mute_queue = before.4;
                    return Err(e);
                }
                Ok(line)
            }
            TimeAction::Launch { .. } | TimeAction::StopClip(_) | TimeAction::Uncue(_) => {
                // A new tap replaces the track's cue, which lands on the
                // line Launch gives now, not one an earlier cue kept.
                let kept = match a {
                    TimeAction::Uncue(track) => {
                        self.perform.uncue(track);
                        None
                    }
                    TimeAction::Launch { track, .. } | TimeAction::StopClip(track) => {
                        Some((track, self.perform.forget_line(track)))
                    }
                    _ => None,
                };
                // While the music plays, they wait for their launch line.
                let after = clips_after(&self.project, &self.launches, &self.cues, &a, &info);
                let before = (
                    std::mem::replace(&mut self.launches, after.0),
                    std::mem::replace(&mut self.cues, after.1),
                );
                if let Err(e) = self.compile() {
                    (self.launches, self.cues) = before;
                    if let Some((track, line)) = kept {
                        self.perform.restore_line(track, line);
                    }
                    return Err(e);
                }
                Ok(format!("clips: {a:?}"))
            }
            TimeAction::NewClip { track, .. }
                if self.project.tracks.get(track).is_some_and(|t| t.is_loops()) =>
            {
                Err(format!(
                    "{} is a Loop track: record a loop on Play's loop row or the Looper tab, \
                     and it lands here",
                    self.project.tracks[track].name
                ))
            }
            TimeAction::NewClip { track, slot } => {
                let bar = info.meter.bar_ticks();
                let length = info.loop_range.map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t.slots.get_mut(slot).ok_or("no such slot")?;
                if s.is_some() {
                    return Err(format!("slot {} of {} is not empty", slot + 1, t.name));
                }
                // With its lasting id (Arrange 1.1), as every clip gets.
                self.project
                    .put_new_clip(
                        track,
                        slot,
                        Clip::new(&format!("Clip {}", slot + 1), length),
                    )
                    .map_err(|e| format!("new clip: {e:?}"))?;
                self.set_armed(Some(track));
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
            TimeAction::SaveState { track, slot } => self.save_state(track, slot),
            TimeAction::Reload { track, slot } => self.reload(track, slot),
            TimeAction::Mutate {
                track,
                slot,
                more,
                melodic,
            } => self.mutate(track, slot, more, melodic),
            TimeAction::Restore { track, slot } => self.restore(track, slot),
            TimeAction::DeleteClip { track, slot } => {
                // A clip the song plays goes to the song's own clips, so
                // the song plays on (Arrange 1.3).
                let kept = self
                    .project
                    .clip(track, slot)
                    .is_some_and(|c| self.project.arrangement.plays(c.id));
                self.takes
                    .remove_clip(&mut self.project, track, slot)
                    .map_err(|_| format!("slot {} of track {} is empty", slot + 1, track + 1))?;
                self.forget_saved(Some((track, slot)));
                // Its launch and cues go with it (compile drops them).
                self.compile()?;
                let song = if kept {
                    "; the song keeps playing it"
                } else {
                    ""
                };
                Ok(format!(
                    "erase: clip {} of track {} deleted{song} (UNDO brings it back)",
                    slot + 1,
                    track + 1
                ))
            }
            TimeAction::DropClip { from, to, copy } => {
                let loops = |t: usize| self.project.tracks.get(t).map(|t| t.is_loops());
                if loops(from.0) != loops(to.0) {
                    return Err(
                        "clips: loops stay on Loop tracks and clips on note tracks".to_string()
                    );
                }
                if copy && from.0 != to.0 && loops(from.0) == Some(true) {
                    return Err(
                        "clips: a loop's copy goes in another row of its own Loop track"
                            .to_string(),
                    );
                }
                let name = |p: &Project, (t, s): (usize, usize)| {
                    let track = p.tracks.get(t).map_or("", |t| t.name.as_str());
                    format!("{track} / {}", s + 1)
                };
                let clip = self
                    .project
                    .clip(from.0, from.1)
                    .map_or_else(String::new, |c| c.name.clone());
                let (_, drop) =
                    self.takes
                        .drop_clip(&mut self.project, from, to, copy)
                        .map_err(|e| match e {
                            ModelError::NoRoom => format!(
                                "clips: {} has a clip; a copy goes in an empty slot",
                                name(&self.project, to)
                            ),
                            ModelError::BadPlacement => {
                                format!(
                            "clips: {clip}'s song blocks would sit on {}'s: move those first",
                            self.project.tracks.get(to.0).map_or("", |t| t.name.as_str())
                        )
                            }
                            _ => "clips: nothing to move there".to_string(),
                        })?;
                self.drop_went_in(drop)?;
                let there = name(&self.project, to);
                Ok(match drop {
                    ClipDrop::Swapped(..) => {
                        let other = self
                            .project
                            .clip(from.0, from.1)
                            .map_or_else(String::new, |c| c.name.clone());
                        format!("clips: {clip} and {other} changed places (UNDO takes it back)")
                    }
                    ClipDrop::Copied(..) => {
                        format!("clips: a copy of {clip} in {there} (UNDO takes it back)")
                    }
                    _ => format!("clips: {clip} moved to {there} (UNDO takes it back)"),
                })
            }
            TimeAction::DropRow { from, to, copy } => {
                let (_, drop) = self
                    .takes
                    .drop_row(&mut self.project, from, to, copy)
                    .map_err(|e| match e {
                        ModelError::NoRoom => format!(
                            "clips: row {SLOTS} is in use, so a copy has no room: empty it first"
                        ),
                        ModelError::BadPlacement => {
                            "clips: the song's blocks would overlap".to_string()
                        }
                        _ => format!("clips: row {} stays where it is", from + 1),
                    })?;
                self.drop_went_in(drop)?;
                Ok(match drop {
                    ClipDrop::RowCopied { from, to } => format!(
                        "clips: a copy of row {} in row {}, the rows under it down one (UNDO takes it back)",
                        from + 1,
                        to + 1
                    ),
                    _ => format!(
                        "clips: row {} moved to row {} (UNDO takes it back)",
                        from + 1,
                        if to > from { to } else { to + 1 }
                    ),
                })
            }
            TimeAction::AddTrack => {
                let name = format!("Track {}", self.project.tracks.len() + 1);
                let (_, index) = self
                    .takes
                    .add_track(&mut self.project, &name)
                    .map_err(|_| format!("{MAX_TRACKS} tracks is the most a project holds"))?;
                self.set_armed(Some(index));
                self.compile()?;
                Ok(format!("{name} added and armed (UNDO takes it away)"))
            }
            TimeAction::AddLoopTrack => {
                let name = loops::new_name(&self.project);
                let track = mpfree_seq::Track::new_loops(&name, 0);
                self.takes
                    .add_track_as(&mut self.project, track)
                    .map_err(|_| format!("{MAX_TRACKS} tracks is the most a project holds"))?;
                self.compile()?;
                Ok(format!(
                    "{name} added, a Loop track: tap a cell of it to record a loop there (UNDO takes it away)"
                ))
            }
            TimeAction::LoopCell(None) => {
                if self.loop_cell.take().is_some() {
                    self.changed = true;
                }
                Ok("loop cell: none armed".into())
            }
            TimeAction::LoopCell(Some((t, s))) => {
                let tr = self
                    .project
                    .tracks
                    .get(t)
                    .filter(|tr| tr.is_loops())
                    .ok_or_else(|| format!("track {} is not a Loop track", t + 1))?;
                match tr.slots.get(s) {
                    None => return Err(format!("{} has no row {}", tr.name, s + 1)),
                    Some(Some(_)) => {
                        return Err(format!("row {} of {} holds a loop", s + 1, tr.name))
                    }
                    Some(None) => {}
                }
                self.loop_cell = Some((t, s));
                self.changed = true;
                Ok(format!(
                    "loop cell: {} row {} armed, the next loop recorded lands there",
                    tr.name,
                    s + 1
                ))
            }
            TimeAction::LoopCopy { from, to, into } => {
                let n = usize::from(into);
                let name = |p: &Project, (t, s): (usize, usize)| {
                    let track = p.tracks.get(t).map_or("", |t| t.name.as_str());
                    format!("{track} / {}", s + 1)
                };
                let clip = self
                    .project
                    .clip(from.0, from.1)
                    .filter(|c| c.looped.is_some())
                    .ok_or_else(|| format!("clips: {} holds no loop", name(&self.project, from)))?;
                let (len, was) = (clip.length, clip.name.clone());
                let there = self
                    .project
                    .tracks
                    .get(to.0)
                    .filter(|tr| tr.is_loops() && to.0 != from.0)
                    .and_then(|tr| tr.slots.get(to.1))
                    .ok_or("clips: a loop's copy onto another Loop track goes in a cell of it")?;
                if there.is_some() {
                    return Err(format!(
                        "clips: {} has a loop; a copy goes in an empty cell",
                        name(&self.project, to)
                    ));
                }
                if n >= LOOPS || loops::in_cell(&self.project, into) || self.loops_seen[n].pass {
                    return Err(format!("clips: loop {} isn't free for a copy", n + 1));
                }
                self.project
                    .put_new_clip(to.0, to.1, Clip::new_loop(into, len))
                    .map_err(|e| format!("clips: {e:?}"))?;
                self.compile()?;
                Ok(format!(
                    "clips: {was} copies into Loop {} on {}: launch it to hear it (the looper's UNDO takes it back)",
                    n + 1,
                    name(&self.project, to)
                ))
            }
            TimeAction::LoopCopyOff(into) => {
                // Only while nothing of the copy went in: a loop with a
                // pass keeps its cells (its clear empties them).
                let n = usize::from(into);
                if n < LOOPS
                    && !self.loops_seen[n].pass
                    && loops::remove(&mut self.project, into) > 0
                {
                    self.compile()?;
                    return Ok(format!(
                        "clips: Loop {}'s copy stopped: its cell is empty",
                        n + 1
                    ));
                }
                Ok(String::new())
            }
            TimeAction::LoopInput(t) => {
                let tr = self
                    .project
                    .tracks
                    .get_mut(t)
                    .ok_or_else(|| format!("no track {}", t + 1))?;
                let Some(now) = tr.loops() else {
                    return Err(format!("{} is not a Loop track", tr.name));
                };
                let all = mpfree_seq::LoopInput::ALL;
                let at = all.iter().position(|&i| i == now).unwrap_or(0);
                let next = all[(at + 1) % all.len()];
                tr.kind = mpfree_seq::TrackKind::Loops(next);
                self.changed = true;
                Ok(format!("{} records {}", tr.name, next.name()))
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
                // Off during a take that turned it on: off for that take.
                if !on && self.song_take == SongTake::Armed {
                    self.song_take = SongTake::Off;
                }
                self.arm_song(on);
                if on {
                    return Ok("rec song: what the grid plays goes into the song".into());
                }
                self.keep_song().map(|line| format!("rec song off: {line}"))
            }
            TimeAction::RecSongWithTake(on) => self.song_with_take_set(on),
            TimeAction::Morph(_)
            | TimeAction::SnapBack
            | TimeAction::KeepMorph
            | TimeAction::SaveScene(_) => {
                Err("morph: needs the mixer's mod engine (TimeHost::morph)".into())
            }
            TimeAction::Edit(e) => self.edit(e),
            TimeAction::SongView(on) => {
                self.song_view = on;
                self.list_view = false;
                self.changed = true;
                Ok(if on { "song view" } else { "clips view" }.into())
            }
            TimeAction::ListView(on) => {
                self.list_view = on;
                if on {
                    self.song_view = false;
                }
                self.changed = true;
                Ok(if on { "song list" } else { "clips view" }.into())
            }
            TimeAction::ListPick(k) => {
                self.list_pick = k.filter(|&k| k < self.project.arrangement.list.steps.len());
                if let Some(k) = self.list_pick {
                    self.scene_pick = Some(self.project.arrangement.list.steps[k].row);
                }
                self.changed = true;
                Ok(match self.list_pick {
                    Some(k) => format!("song list: step {} picked", k + 1),
                    None => "song list: nothing picked".into(),
                })
            }
            TimeAction::SongMode(on) => {
                self.song = on;
                if let Err(e) = self.compile() {
                    self.song = !on;
                    return Err(e);
                }
                Ok(if on { "song on" } else { "song off" }.into())
            }
            // Make or Perform (sequencer S4): how the app answers the
            // hands; nothing playing or waiting changes.
            TimeAction::Perform(on) => {
                if self.project.perform != on {
                    self.project.perform = on;
                    self.changed = true;
                }
                Ok(perform_line(on).into())
            }
            TimeAction::SelectPlacement(i) => {
                self.selected = i.filter(|&i| i < self.project.arrangement.placements.len());
                self.region = None;
                self.section = None;
                self.tempo_pick = None;
                self.changed = true;
                Ok(String::new())
            }
            TimeAction::PickSection(k) => {
                self.section = k.filter(|&k| k < self.project.arrangement.markers.len());
                if self.section.is_some() {
                    self.selected = None;
                    self.region = None;
                    self.tempo_pick = None;
                }
                self.changed = true;
                Ok(String::new())
            }
            // The selection's kind: a region picked drops the placement.
            TimeAction::Audio(AudioAction::Pick(r)) => {
                self.region = r;
                if r.is_some() {
                    self.selected = None;
                    self.section = None;
                    self.tempo_pick = None;
                }
                self.changed = true;
                Ok(String::new())
            }
            // The audio tracks' engine is the app's to wire (AudioHost);
            // until it takes these, say so rather than drop them quietly.
            TimeAction::Audio(a) => Err(format!("audio tracks: {a:?} is not wired yet")),
            TimeAction::SongTempo(TempoEdit::Pick(k)) => {
                self.tempo_pick = k.filter(|&k| k < self.project.arrangement.tempo.len());
                if self.tempo_pick.is_some() {
                    self.selected = None;
                    self.region = None;
                    self.section = None;
                }
                self.changed = true;
                Ok(String::new())
            }
            TimeAction::SongTempo(e) => {
                // An edit of the song alone, which UNDO takes back; the
                // wheel turning one point's tempo is one undo.
                let before = self.project.arrangement.clone();
                let (line, at, join) = match self.edit_tempo(e) {
                    Ok(done) => done,
                    Err(e) => {
                        self.project.arrangement = before;
                        return Err(e);
                    }
                };
                if self.project.arrangement == before {
                    self.changed = true;
                    return Ok(line);
                }
                // The map rides in the view, so the transport takes it
                // with the song it goes with (only song mode plays it).
                if self.song {
                    if let Err(e) = self.compile() {
                        self.project.arrangement = before;
                        self.compile()?;
                        return Err(e);
                    }
                }
                self.takes
                    .song_edit_joining(&self.project, SongChange::Tempo { at }, before, join);
                self.changed = true;
                Ok(line)
            }
            TimeAction::SongToScenes => {
                // The rows change, not the song: one edit of whole rows,
                // which UNDO takes back.
                let bar = info.meter.bar_ticks();
                let before = self.project.clone();
                let line = match self.edit_song(a, bar) {
                    Ok((line, _)) => line,
                    Err(e) => {
                        self.project = before;
                        return Err(e);
                    }
                };
                if let Err(e) = self.compile() {
                    // Back as it was, but for the clip ids it gave: an id
                    // is never given twice.
                    let next = self.project.next_clip;
                    self.project = before;
                    self.project.next_clip = next;
                    self.compile()?;
                    return Err(e);
                }
                self.takes.rows_edit(&before, &self.project);
                self.changed = true;
                Ok(line)
            }
            TimeAction::ListTimes { .. }
            | TimeAction::ListScene { .. }
            | TimeAction::ListAdd
            | TimeAction::ListRemove(_)
            | TimeAction::ListLoop(_)
            | TimeAction::ListToSong
            | TimeAction::ScenesToSong
            | TimeAction::MakeUnique(_)
            | TimeAction::DeletePlacement(_)
            | TimeAction::MovePlacement { .. }
            | TimeAction::StretchPlacement { .. }
            | TimeAction::MoveSection { .. } => {
                // Edits of the song alone, each one UNDO takes back.
                let bar = info.meter.bar_ticks();
                let before = self.project.arrangement.clone();
                let edited = if matches!(
                    a,
                    TimeAction::ListTimes { .. }
                        | TimeAction::ListScene { .. }
                        | TimeAction::ListAdd
                        | TimeAction::ListRemove(_)
                        | TimeAction::ListLoop(_)
                        | TimeAction::ListToSong
                ) {
                    self.edit_list(a, bar)
                } else {
                    self.edit_song(a, bar)
                };
                let (line, change) = match edited {
                    Ok(done) => done,
                    Err(e) => {
                        self.project.arrangement = before;
                        return Err(e);
                    }
                };
                if let Err(e) = self.compile() {
                    self.project.arrangement = before;
                    self.compile()?;
                    return Err(e);
                }
                if let Some(c) = change.filter(|_| self.project.arrangement != before) {
                    self.takes.song_edit(&self.project, c, before);
                }
                self.changed = true;
                Ok(line)
            }
        }
    }

    /// What live to song put into the song when the transport stopped
    /// (or why it could not: the recording is then kept, and goes in when
    /// Rec song is turned off), once, for the log.
    pub fn take_later_note(&mut self) -> Option<String> {
        self.later_note.take()
    }

    /// The pass just recorded, thrown away (UNDO while it recorded): its
    /// take undone and forgotten, so REDO can't bring it back. True if
    /// there was one.
    fn throw_pass(&mut self) -> bool {
        let Some(n) = self.pass_take.take() else {
            self.later_note = Some("undo: nothing was recorded".into());
            return false;
        };
        let last = self.takes.done().last();
        if !last.is_some_and(|t| t.number == n && !t.captured && !t.edited) {
            self.later_note = Some("undo: nothing was recorded".into());
            return false;
        }
        self.takes.undo(&mut self.project);
        self.takes.forget_undone();
        self.changed = true;
        self.steps_changed = true;
        self.later_note = Some(format!("undo: take {n} thrown away"));
        true
    }

    /// Rec song armed or not, with what plays now.
    fn arm_song(&mut self, on: bool) {
        let rows = (self.morph.a, self.morph.b);
        self.song_rec
            .arm(on, &self.project, &self.launches, self.morph.pos, rows);
        self.changed = true;
    }

    /// Perform, and the take recording or the next one brings Rec song
    /// (the REC chip's "+ Rec song").
    fn song_with_take(&self) -> bool {
        self.project.perform
            && match self.song_take {
                SongTake::Idle => !self.song_rec.armed(),
                SongTake::Armed => true,
                SongTake::Off => false,
            }
    }

    /// The REC chip's "+ Rec song" set (`on`) or taken off for the take
    /// (sequencer S4, Perform). A line for the log.
    fn song_with_take_set(&mut self, on: bool) -> Result<String, String> {
        if !self.project.perform {
            return Err("rec song with REC: only in Perform".into());
        }
        self.changed = true;
        if on {
            if self.song_take == SongTake::Off {
                self.song_take = SongTake::Idle;
            }
            if self.taking && !self.song_rec.armed() {
                self.arm_song(true);
                self.song_take = SongTake::Armed;
                return Ok("rec song: on with this take".into());
            }
            return Ok("rec song: on with REC".into());
        }
        let was = std::mem::replace(&mut self.song_take, SongTake::Off);
        if was == SongTake::Armed && self.song_rec.armed() {
            self.arm_song(false);
            return self
                .keep_song()
                .map(|line| format!("rec song: off for this take ({line})"));
        }
        Ok(format!(
            "rec song: off for {} take",
            if self.taking { "this" } else { "the next" }
        ))
    }

    /// The host says whether a take records (the notes, their count-in,
    /// or a loop's pass), each period. In Perform a take turns Rec song on
    /// for its length, unless it is on already or taken off for this take
    /// (sequencer S4, "Rec song is on with REC"). A line for the log when
    /// it goes on or off.
    pub fn take_follows(&mut self, taking: bool) -> Option<String> {
        let was = std::mem::replace(&mut self.taking, taking);
        if was == taking {
            return None;
        }
        if taking {
            if self.project.perform && self.song_take == SongTake::Idle && !self.song_rec.armed() {
                self.arm_song(true);
                self.song_take = SongTake::Armed;
                return Some("rec song: on with REC (Perform)".into());
            }
            return None;
        }
        // The take ended.
        let ended = std::mem::replace(&mut self.song_take, SongTake::Idle);
        self.changed = true;
        if ended != SongTake::Armed || !self.song_rec.armed() {
            return None;
        }
        self.arm_song(false);
        if !self.song_rec.waiting() {
            return Some("rec song: off with the take".into());
        }
        Some(match self.keep_song() {
            Ok(line) => format!("rec song: off with the take, {line}"),
            Err(e) => e,
        })
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
        self.section = None;
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
    /// the rows). A line for the log, and what changed for the undo list.
    /// The song's tempo map while song mode plays its tempo points, made
    /// with the project's tempo before the first; None plays the
    /// project's tempo throughout.
    fn song_tempo(&self) -> Result<Option<TempoMap>, String> {
        let song = &self.project.arrangement;
        if !self.song || song.tempo.is_empty() {
            return Ok(None);
        }
        song.tempo_map(self.project_bpm)
            .map(Some)
            .map_err(|e| format!("song: its tempo points don't make a tempo map ({e:?})"))
    }

    /// The tempo of the song's point in effect at the playhead, while
    /// song mode plays the song's tempo points; None before the first
    /// point, or when they don't play.
    pub fn tempo_here(&self) -> Option<f64> {
        let tempo = &self.project.arrangement.tempo;
        if !self.song || tempo.is_empty() {
            return None;
        }
        let i = tempo.partition_point(|p| p.at <= self.info.position);
        i.checked_sub(1).map(|i| tempo[i].bpm)
    }

    /// The project's tempo (the app's: the tempo chip, TAP TEMPO, a
    /// project opened), which the song plays before its first tempo
    /// point. A change while song mode plays points after bar 1 compiles
    /// the view again, with the new map.
    pub fn project_tempo(&mut self, bpm: f64) {
        if bpm == self.project_bpm {
            return;
        }
        self.project_bpm = bpm;
        let lead = self
            .project
            .arrangement
            .tempo
            .first()
            .is_some_and(|p| p.at > 0);
        if self.song && lead {
            if let Err(e) = self.compile() {
                self.later_note = Some(format!("tempo: {e}"));
            }
        }
    }

    /// While song mode plays the song's tempo points, the tempo turned
    /// sets the point in effect at the playhead (Jay's card, 18 Nov), and
    /// [`TimeControl::project_tempo`] does not change.
    pub fn song_tempo_plays(&self) -> bool {
        self.song && !self.project.arrangement.tempo.is_empty()
    }

    /// Edit the song's tempo points as `e` asks: what to say, the tick
    /// of the point it changed, and what joins a turn of the wheel to the
    /// take before it. The caller puts the song back on an error.
    fn edit_tempo(&mut self, e: TempoEdit) -> Result<(String, Tick, Option<u64>), String> {
        // The app's range (App::set_tempo).
        let clamp = |bpm: f64| ((bpm * 10.0).round() / 10.0).clamp(30.0, 300.0);
        // A point's own key for the wheel and the tempo turned.
        let key = |at: Tick| Some(0x7e3d_0000_0000_0000 ^ at as u64);
        let meter = self.info.meter;
        let bar = meter.bar_ticks().max(1);
        let at_bar = |t: Tick| bar_beat(t, meter);
        let base = self.project_bpm;
        let song = &mut self.project.arrangement;
        let point = |song: &mpfree_seq::arrange::Arrangement, k: usize| {
            song.tempo
                .get(k)
                .copied()
                .ok_or_else(|| "tempo: that point is gone".to_string())
        };
        let refused = |at: Tick| {
            format!(
                "tempo: no point can go at {}: points are a 1/16 apart at least, {} at most",
                at_bar(at),
                mpfree_seq::arrange::MAX_TEMPO_POINTS
            )
        };
        let picked = |song: &mpfree_seq::arrange::Arrangement, at: Tick| {
            song.tempo.iter().position(|p| p.at == at)
        };
        match e {
            TempoEdit::Pick(_) => Ok((String::new(), 0, None)),
            TempoEdit::Add(at) => {
                let at = at.max(0);
                if let Some(k) = picked(song, at) {
                    self.tempo_pick = Some(k);
                    return Ok((
                        format!("tempo: the point at {} picked", at_bar(at)),
                        at,
                        None,
                    ));
                }
                let bpm = clamp(song.tempo_map(base).map_or(base, |m| m.bpm_at(at)));
                song.set_tempo(at, bpm, Some(false))
                    .map_err(|_| refused(at))?;
                self.tempo_pick = picked(song, at);
                (self.selected, self.region, self.section) = (None, None, None);
                Ok((
                    format!(
                        "tempo: a point at {}, {bpm:.1} BPM (UNDO takes it back)",
                        at_bar(at)
                    ),
                    at,
                    None,
                ))
            }
            TempoEdit::Move { index, at } => {
                let p = point(song, index)?;
                let at = at.max(0);
                if at == p.at {
                    return Ok((String::new(), at, None));
                }
                if picked(song, at).is_some() {
                    return Err(format!("tempo: {} has a point already", at_bar(at)));
                }
                song.remove_tempo(p.at);
                song.set_tempo(at, p.bpm, Some(p.ramp))
                    .map_err(|_| refused(at))?;
                self.tempo_pick = picked(song, at);
                Ok((
                    format!(
                        "tempo: the point moved to {} (UNDO takes it back)",
                        at_bar(at)
                    ),
                    at,
                    None,
                ))
            }
            TempoEdit::Bpm { index, bpm } => {
                let p = point(song, index)?;
                let bpm = clamp(bpm);
                song.set_tempo(p.at, bpm, None).map_err(|_| refused(p.at))?;
                Ok((
                    format!("tempo: {bpm:.1} BPM from {}", at_bar(p.at)),
                    p.at,
                    key(p.at),
                ))
            }
            TempoEdit::Ramp { index, on } => {
                let p = point(song, index)?;
                song.set_tempo(p.at, p.bpm, Some(on))
                    .map_err(|_| refused(p.at))?;
                let line = match (on, song.tempo.get(index + 1)) {
                    (false, _) => format!("tempo: {} steps to its tempo", at_bar(p.at)),
                    (true, Some(next)) => format!(
                        "tempo: {} glides from {:.1} to {:.1} BPM at {}",
                        at_bar(p.at),
                        p.bpm,
                        next.bpm,
                        at_bar(next.at)
                    ),
                    (true, None) => format!(
                        "tempo: {} glides once a point after it says to what",
                        at_bar(p.at)
                    ),
                };
                Ok((line, p.at, None))
            }
            TempoEdit::Delete(index) => {
                let p = point(song, index)?;
                song.remove_tempo(p.at);
                self.tempo_pick = None;
                Ok((
                    format!(
                        "tempo: the point at {} deleted (UNDO brings it back)",
                        at_bar(p.at)
                    ),
                    p.at,
                    None,
                ))
            }
            TempoEdit::AtPlayhead(bpm) => {
                let bpm = clamp(bpm);
                let now = self.info.position.max(0);
                let i = song.tempo.partition_point(|p| p.at <= now);
                let at = match i.checked_sub(1) {
                    Some(k) => song.tempo[k].at,
                    // Before the first point: one on the playhead's bar.
                    None => now.div_euclid(bar) * bar,
                };
                song.set_tempo(at, bpm, None).map_err(|_| refused(at))?;
                Ok((
                    format!("tempo: {bpm:.1} BPM from {}", at_bar(at)),
                    at,
                    key(at),
                ))
            }
        }
    }

    fn edit_song(
        &mut self,
        a: TimeAction,
        bar: Tick,
    ) -> Result<(String, Option<SongChange>), String> {
        let meter = self.info.meter;
        let tracks = &self.project.tracks;
        let song = &mut self.project.arrangement;
        let named = |song: &mpfree_seq::arrange::Arrangement, i: usize| {
            song.placements
                .get(i)
                .and_then(|p| song.clip_of(tracks, p))
                .map_or_else(|| "the block".to_string(), |c| c.name.clone())
        };
        let track_name = |t: usize| tracks.get(t).map_or_else(String::new, |t| t.name.clone());
        let at = |t: Tick| bar_beat(t, meter);
        match a {
            TimeAction::MovePlacement {
                index,
                track,
                start,
                copy,
            } => {
                let name = named(song, index);
                let was = song.placements.get(index).map(|p| p.track);
                let j = song
                    .move_placement(tracks, index, track, start, copy)
                    .map_err(|e| match e {
                        // A loop's block stays on its Loop track's lane
                        // (loop-tracks.md part 5).
                        ModelError::BadLoop => format!(
                            "song: {name} stays on its own lane: a loop's block can't go to another track, nor a clip's onto a Loop track"
                        ),
                        _ => format!(
                            "song: {name} doesn't fit on {} at {}: it would run into another block",
                            track_name(track),
                            at(start)
                        ),
                    })?;
                // On another lane it stays linked to its clip and plays
                // through that track (Arrange 1.3).
                let across = was.is_some_and(|t| t != track);
                let own = if across {
                    format!(", playing through {}", track_name(track))
                } else {
                    String::new()
                };
                let place = if across {
                    format!("{}, {}", track_name(track), at(start))
                } else {
                    at(start)
                };
                self.selected = Some(j);
                self.section = None;
                Ok(if copy {
                    (
                        format!("song: a copy of {name} at {place}{own} (UNDO takes it back)"),
                        Some(SongChange::Copied { track, start }),
                    )
                } else {
                    (
                        format!("song: {name} moved to {place}{own} (UNDO takes it back)"),
                        Some(SongChange::Moved { track, start }),
                    )
                })
            }
            TimeAction::StretchPlacement { index, length } => {
                let name = named(song, index);
                let track = song.placements.get(index).map_or(0, |p| p.track);
                song.set_length(index, length).map_err(|_| {
                    format!("song: {name} can't be that long: it would run into the next block")
                })?;
                let bars = if length % bar.max(1) == 0 {
                    format!("{} bars", length / bar.max(1))
                } else {
                    format!("{:.1} bars", length as f64 / bar.max(1) as f64)
                };
                Ok((
                    format!("song: {name} is {bars} long now (UNDO takes it back)"),
                    Some(SongChange::Resized { track, length }),
                ))
            }
            TimeAction::MoveSection { marker, gap, copy } => {
                let name = song
                    .markers
                    .get(marker)
                    .map_or_else(String::new, |m| m.name.clone());
                match song.move_section(tracks, marker, gap, copy, bar) {
                    Ok(()) => {}
                    Err(ModelError::BadMoves) if !song.tempo.is_empty() => {
                        return Err(format!(
                            "song: {name} can't move while the song has tempo points: they would stay where they are"
                        ))
                    }
                    Err(ModelError::BadMoves) => {
                        return Err(format!(
                            "song: {name} can't move while the song has morph moves: they would stay where they are"
                        ))
                    }
                    Err(_) => return Err(format!("song: {name} can't go there")),
                }
                let now = if copy {
                    gap
                } else if gap > marker + 1 {
                    gap - 1
                } else if gap < marker {
                    gap
                } else {
                    return Ok((format!("song: {name} is already there"), None));
                };
                let start = song.markers.get(now).map_or(0, |m| m.at);
                self.section = Some(now);
                self.selected = None;
                Ok(if copy {
                    (
                        format!(
                            "song: a copy of {name} at {} (UNDO takes it back)",
                            at(start)
                        ),
                        Some(SongChange::SectionCopied { at: start }),
                    )
                } else {
                    (
                        format!(
                            "song: {name} moved to {}; the sections after it moved up (UNDO takes it back)",
                            at(start)
                        ),
                        Some(SongChange::SectionMoved { at: start }),
                    )
                })
            }
            TimeAction::ScenesToSong => {
                // Every row with a clip, in order, after the song's end
                // (every row the project keeps, not only the grid's).
                let rows: Vec<(usize, Tick)> = (0..ROWS)
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
                let (how, which) = differences_said(&self.project, &add.differences, bar);
                // New placements shift the indices.
                self.selected = None;
                let song = &mut self.project.arrangement;
                for p in add.song.placements {
                    song.place(p).map_err(|e| format!("scenes → song: {e:?}"))?;
                }
                for m in add.song.markers {
                    song.mark(m.at, &m.name)
                        .map_err(|e| format!("scenes → song: {e:?}"))?;
                }
                self.section = None;
                Ok((
                    format!(
                        "scenes → song: {} rows from bar {}, {how}{which}",
                        rows.len(),
                        at / bar.max(1) + 1
                    ),
                    Some(SongChange::Scenes { rows: rows.len() }),
                ))
            }
            TimeAction::SongToScenes => match song_to_scenes(&mut self.project, bar) {
                Ok(done) if done.rows.is_empty() => {
                    Ok(("song → scenes: the song is empty".into(), None))
                }
                Ok(done) => {
                    let (how, which) = differences_said(&self.project, &done.differences, bar);
                    let rows = &done.rows;
                    Ok((
                        format!(
                            "song → scenes: {}, {how} (UNDO takes it back){which}",
                            rows_named(rows[0], rows[rows.len() - 1])
                        ),
                        None,
                    ))
                }
                Err(ModelError::NoRoom) => {
                    Err("song → scenes: not enough empty rows in a run".into())
                }
                Err(e) => Err(format!("song → scenes: {e:?}")),
            },
            TimeAction::MakeUnique(i) => {
                let p = self
                    .project
                    .arrangement
                    .placements
                    .get(i)
                    .ok_or("make unique: no such placement")?;
                let (track, start) = (p.track, p.start);
                self.project
                    .make_unique(i)
                    .map_err(|e| format!("make unique: {e:?}"))?;
                Ok((
                    format!("placement {} is its own copy", i + 1),
                    Some(SongChange::Unique { track, start }),
                ))
            }
            TimeAction::DeletePlacement(i) => {
                let p = song.remove(i).ok_or("delete: no such placement")?;
                self.selected = None;
                Ok((
                    format!("placement {} deleted (UNDO puts it back)", i + 1),
                    Some(SongChange::Deleted {
                        track: p.track,
                        start: p.start,
                    }),
                ))
            }
            _ => Ok((String::new(), None)),
        }
    }

    /// A song list edit: a step's times or scene, a step put in or taken
    /// out, the loop switch, or List → song. A line for the log, and what
    /// changed for the take UNDO takes back.
    fn edit_list(
        &mut self,
        a: TimeAction,
        bar: Tick,
    ) -> Result<(String, Option<SongChange>), String> {
        let filled: Vec<usize> = (0..SLOTS).filter(|&r| self.project.row_filled(r)).collect();
        let n = self.project.arrangement.list.steps.len();
        let list = &mut self.project.arrangement.list;
        match a {
            TimeAction::ListTimes { step, delta } => {
                let s = list.steps.get_mut(step).ok_or("song list: no such step")?;
                let to = (i32::from(s.times) + delta).clamp(1, i32::from(MAX_TIMES)) as u16;
                if to == s.times {
                    return Ok((format!("song list: step {} ×{to}", step + 1), None));
                }
                s.times = to;
                Ok((
                    format!("song list: step {} ×{to}", step + 1),
                    Some(SongChange::List),
                ))
            }
            TimeAction::ListScene { step, delta } => {
                let s = list.steps.get_mut(step).ok_or("song list: no such step")?;
                let row = next_filled(&filled, s.row, delta).ok_or(
                    "song list: make a clip on the grid first: a step plays a row with clips",
                )?;
                if row == s.row {
                    return Err("song list: no other row has a clip".into());
                }
                s.row = row;
                Ok((
                    format!("song list: step {} plays row {}", step + 1, row + 1),
                    Some(SongChange::List),
                ))
            }
            TimeAction::ListAdd => {
                if n >= MAX_LIST_STEPS {
                    return Err(format!("song list: full ({MAX_LIST_STEPS} steps)"));
                }
                // After the pick (the end with none), the next row with a
                // clip after the scene there.
                let at = self.list_pick.map_or(n, |k| (k + 1).min(n));
                let after = at.checked_sub(1).map(|k| list.steps[k].row);
                let row = match after {
                    Some(r) => next_filled(&filled, r, 1),
                    None => filled.first().copied(),
                }
                .ok_or("song list: make a clip on the grid first: a step plays a row with clips")?;
                list.steps.insert(at, ListStep { row, times: 1 });
                self.list_pick = Some(at);
                self.perform.list_edited(at, true);
                Ok((
                    format!("song list: step {} plays row {}", at + 1, row + 1),
                    Some(SongChange::List),
                ))
            }
            TimeAction::ListRemove(step) => {
                if step >= n {
                    return Err("song list: no such step".into());
                }
                list.steps.remove(step);
                self.perform.list_edited(step, false);
                self.list_pick = (n > 1).then(|| step.min(n - 2));
                Ok((
                    format!("song list: step {} taken out (UNDO puts it back)", step + 1),
                    Some(SongChange::List),
                ))
            }
            TimeAction::ListLoop(on) => {
                list.looped = on;
                Ok((
                    if on {
                        "song list: goes round"
                    } else {
                        "song list: stops after its last step"
                    }
                    .into(),
                    Some(SongChange::List),
                ))
            }
            TimeAction::ListToSong => {
                if n == 0 {
                    return Err("list → song: the list is empty".into());
                }
                let end = self.project.arrangement.end();
                let at = (end + bar - 1) / bar.max(1) * bar;
                let add = list_to_song(&self.project, at, bar)
                    .map_err(|e| format!("list → song: {e:?}"))?;
                let (how, which) = differences_said(&self.project, &add.differences, bar);
                self.selected = None;
                self.section = None;
                let song = &mut self.project.arrangement;
                for p in add.song.placements {
                    song.place(p).map_err(|e| format!("list → song: {e:?}"))?;
                }
                for m in add.song.markers {
                    song.mark(m.at, &m.name)
                        .map_err(|e| format!("list → song: {e:?}"))?;
                }
                Ok((
                    format!(
                        "list → song: {n} steps from bar {}, {how}{which}",
                        at / bar.max(1) + 1
                    ),
                    Some(SongChange::ListToSong { steps: n }),
                ))
            }
            _ => Ok((String::new(), None)),
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

    /// CAPTURE: keep the last phrase: the pads' notes in the armed track
    /// (track 1 if none is armed), and the keys a route played on other
    /// tracks each in its own track, all one take that one UNDO takes
    /// back. Each track's notes go into its playing clip when they were
    /// played in time over it, else into a new clip in its first empty
    /// slot that plays on in its place. A track they can't go into is left
    /// out and said. A line for the log.
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
            loop_range: t.playing_loop(),
        };
        let armed = self.armed.unwrap_or(0);
        let name = format!("Capture {}", self.takes.captures() + 1);
        let mut kept: Vec<Kept> = Vec::new();
        let mut left: Vec<String> = Vec::new();
        let mut first_error = None;
        for (track, part) in phrase.split(armed) {
            let r = match kept.first().map(|k| k.take) {
                None => keep(
                    &part,
                    &mut self.project,
                    &self.launches,
                    &mut self.takes,
                    track,
                    now,
                    &name,
                ),
                // The other tracks join the first one's take.
                Some(n) => self.takes.joining(n, |takes| {
                    keep(
                        &part,
                        &mut self.project,
                        &self.launches,
                        takes,
                        track,
                        now,
                        &name,
                    )
                }),
            };
            match r {
                Ok(k) => kept.push(k),
                Err(e) => {
                    left.push(self.left_out(track, e));
                    first_error.get_or_insert(e);
                }
            }
        }
        if kept.is_empty() {
            let e = first_error.ok_or("capture: nothing played")?;
            return Err(format!("capture: {e:?}"));
        }
        // The moves made over a phrase played in time go with the armed
        // track's notes (their addresses are the armed track's); with none
        // of those kept, they stay out, and the line says so.
        let mut moves = 0;
        let mut moves_out = 0;
        if let Some(s) = phrase.song_start {
            let (from, to) = (s as f64, (s + phrase.length) as f64);
            match kept.iter().find(|k| k.track == armed) {
                Some(k) => {
                    let origin = k.launch.map(|l| l.origin).or_else(|| {
                        self.launches
                            .iter()
                            .find(|l| l.track == armed && l.slot == k.slot)
                            .map(|l| l.origin)
                    });
                    if let (Some(origin), Some(clip)) = (origin, self.project.clip(armed, k.slot)) {
                        let list = moves_for_clip(
                            &self.move_log.phrase(),
                            from,
                            to,
                            clip,
                            origin,
                            armed as u8,
                        );
                        if !list.is_empty() {
                            moves = self
                                .takes
                                .capture_moves(&mut self.project, armed, k.slot, k.take, &list)
                                .unwrap_or(0);
                        }
                    }
                    self.move_log.forget();
                }
                None => moves_out = moves_over(&self.move_log.phrase(), from, to),
            }
        }
        for l in kept.iter().filter_map(|k| k.launch) {
            self.launches.retain(|x| x.track != l.track);
            self.cues.retain(|x| x.track != l.track);
            self.launches.push(l);
        }
        self.capture.mark_kept(phrase.id);
        self.pending = self.capture.pending();
        self.compile()?;
        let moved = match moves {
            0 => String::new(),
            1 => " and 1 move".into(),
            n => format!(" and {n} moves"),
        };
        // Pressure left off is counted with the recorder's (the status
        // line), and said here.
        let mut parts = Vec::with_capacity(kept.len());
        for (i, k) in kept.iter().enumerate() {
            self.touch_lost += k.touch_lost as u64;
            let into = if k.launch.is_some() {
                "new clip in"
            } else {
                "into the clip in"
            };
            let mut pressure = String::new();
            if k.touch > 0 {
                pressure = format!(" with {} pressure values", k.touch);
            }
            if k.touch_lost > 0 {
                pressure += &format!(
                    "; {} pressure values left off, the clip has no room for them",
                    k.touch_lost
                );
            }
            if phrase.touch_cut && k.touch > 0 {
                pressure += "; the first notes' pressure was older than the history keeps";
            }
            let where_to = format!(
                "{into} slot {} of {}",
                k.slot + 1,
                self.project.tracks[k.track].name
            );
            parts.push(if i == 0 {
                format!(
                    "capture {}: {} notes{moved}{pressure}, {}, {where_to}",
                    k.take,
                    k.notes,
                    bars_label(k.bars)
                )
            } else {
                format!("{} notes{pressure}, {where_to}", k.notes)
            });
        }
        parts.extend(left);
        match moves_out {
            0 => {}
            1 => parts.push("1 move left out: it goes with the armed track's notes".into()),
            n => parts.push(format!(
                "{n} moves left out: they go with the armed track's notes"
            )),
        }
        Ok(parts.join("; "))
    }

    /// Why CAPTURE left `track`'s notes out, in words.
    fn left_out(&self, track: usize, e: KeepError) -> String {
        let name = self
            .project
            .tracks
            .get(track)
            .map_or_else(|| format!("track {}", track + 1), |t| t.name.clone());
        match e {
            KeepError::NoTrack(_) => format!("{name} left out: there is no such track"),
            KeepError::NoSlot(_) => format!("{name} left out: every slot holds a clip"),
            KeepError::Nothing => format!("{name} left out: nothing played"),
            KeepError::Model(m) => format!("{name} left out: {m:?}"),
        }
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

    /// Clips made elsewhere for several tracks at once (a MIDI file's
    /// tracks, `midi_file.rs`): each into the first empty slot of its
    /// track, as one take that one UNDO on the Clips page takes away. None
    /// is launched. `clips` are (track, clip), a track at most once. All
    /// go in or none: a track with no empty slot, or a clip the model
    /// refuses, is the error. Returns a line for the log.
    pub fn put_clips(&mut self, clips: Vec<(usize, Clip)>) -> Result<String, String> {
        let mut placed = Vec::with_capacity(clips.len());
        let mut notes = 0;
        for (track, clip) in clips {
            let t = self
                .project
                .tracks
                .get(track)
                .ok_or_else(|| format!("no track {}", track + 1))?;
            let slot = (0..SLOTS)
                .find(|&s| t.slots[s].is_none())
                .ok_or_else(|| format!("{} has no empty slot", t.name))?;
            notes += clip.notes.len();
            placed.push((track, slot, clip));
        }
        let n = placed.len();
        let first = placed.first().map(|p| (p.0, p.1));
        self.takes
            .capture_clips(&mut self.project, placed)
            .map_err(|e| format!("clips refused: {e:?}"))?;
        self.compile()?;
        let (track, slot) = first.expect("one at least, or refused");
        Ok(format!(
            "{n} clip{} with {notes} notes, from slot {} of {}",
            if n == 1 { "" } else { "s" },
            slot + 1,
            self.project.tracks[track].name
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

    /// TRACK MUTE's pad (sequencer S4): track `t`'s mute flips, or with
    /// `solo` its solo. With `wait`, while the transport rolls and Launch
    /// isn't at once, the change joins the mute queue and lands with the
    /// rest on the queue's line (a tap on a track waiting takes its change
    /// back). Otherwise it acts at once, with whatever that track had
    /// waiting.
    pub fn mute_or_solo(&mut self, t: usize, wait: bool, solo: bool) -> Result<String, String> {
        let tr = self
            .project
            .tracks
            .get(t)
            .ok_or_else(|| format!("no track {}", t + 1))?;
        let name = tr.name.clone();
        let now = (tr.mute, tr.solo);
        let queued = self
            .mute_queue
            .as_ref()
            .and_then(|(_, q)| q.want.iter().find(|w| w.0 == t))
            .map(|w| (w.1, w.2));
        let (mut m, mut s) = queued.unwrap_or(now);
        if solo {
            s = !s;
        } else {
            m = !m;
        }
        let what = match (solo, m, s) {
            (true, _, true) => "solo",
            (true, _, false) => "solo off",
            (false, true, _) => "muted",
            (false, false, _) => "plays",
        };
        let line = self
            .mute_queue
            .as_ref()
            .map_or(self.project.launch, |(_, q)| q.line);
        let before = self.mute_queue.clone();
        if let Some((_, q)) = &mut self.mute_queue {
            q.want.retain(|w| w.0 != t);
        }
        if wait && self.info.state.rolling() && line != LaunchGrid::Now {
            if (m, s) != now {
                let ids = &mut self.mute_ids;
                let (_, q) = self.mute_queue.get_or_insert_with(|| {
                    *ids += 1;
                    (*ids, MuteWait::new(line))
                });
                q.want.push((t, m, s));
            }
            if self.mute_queue.as_ref().is_some_and(|(_, q)| q.is_empty()) {
                self.mute_queue = None;
            }
            if let Err(e) = self.compile() {
                self.mute_queue = before;
                return Err(e);
            }
            return Ok(if (m, s) == now {
                format!("track mute: {name}: its change taken back")
            } else {
                format!("track mute: {name} {what} {}", line_words(line))
            });
        }
        if self.mute_queue.as_ref().is_some_and(|(_, q)| q.is_empty()) {
            self.mute_queue = None;
        }
        // At once.
        let was: Vec<bool> = (0..self.project.tracks.len())
            .map(|i| self.project.silent(i))
            .collect();
        let tr = &mut self.project.tracks[t];
        (tr.mute, tr.solo) = (m, s);
        if let Err(e) = self.compile() {
            let tr = &mut self.project.tracks[t];
            (tr.mute, tr.solo) = now;
            self.mute_queue = before;
            return Err(e);
        }
        self.song_silences(&was, self.info.position);
        Ok(format!("track mute: {name} {what}"))
    }

    /// Rec song keeps the tracks whose silence changed from `was` at song
    /// tick `at` (a muted stretch is left out of the song).
    fn song_silences(&mut self, was: &[bool], at: Tick) {
        for (t, &w) in was.iter().enumerate() {
            let now = self.project.silent(t);
            if now != w {
                self.song_rec.mute(t, now, at);
            }
        }
    }

    /// The mute queue landing at song tick `at`, for the log: "track mute:
    /// at 5.1, Track 2 muted, Track 3 solo".
    /// Save state (sequencer S4, Elektron's temp save): keep clip `slot`
    /// of `track` as it is now, for Reload clip, in place of any state
    /// kept before. A loop's cell keeps none: its layers have the
    /// looper's undo.
    fn save_state(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let c = self.project.clip(track, slot).ok_or_else(|| {
            format!(
                "save state: slot {} of track {} is empty",
                slot + 1,
                track + 1
            )
        })?;
        if c.looped.is_some() {
            return Err(
                "save state: a loop's cell keeps no state; the looper's UNDO takes layers back"
                    .to_string(),
            );
        }
        let c = c.clone();
        let name = c.name.clone();
        self.saved.retain(|s| (s.0, s.1) != (track, slot));
        self.saved.push((track, slot, c));
        self.changed = true;
        Ok(format!(
            "save state: {name} kept (Reload clip brings it back)"
        ))
    }

    /// Reload clip (sequencer S4): clip `slot` of `track` goes back to its
    /// saved state, as an edit UNDO takes back. While the clip plays and
    /// the transport rolls it waits for the clip's launch line (the
    /// project's unless it has its own), so the saved state starts on the
    /// beat; asked again while it waits, the reload is taken back.
    fn reload(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let c = self
            .project
            .clip(track, slot)
            .ok_or_else(|| format!("reload: slot {} of track {} is empty", slot + 1, track + 1))?;
        let name = c.name.clone();
        if let Some(i) = self
            .reloads
            .iter()
            .position(|r| (r.track, r.slot) == (track, slot))
        {
            self.reloads.remove(i);
            self.changed = true;
            return Ok(format!("reload: {name}'s reload taken back"));
        }
        let saved = self
            .saved
            .iter()
            .find(|s| (s.0, s.1) == (track, slot))
            .map(|s| &s.2)
            .ok_or_else(|| {
                format!("reload: {name} has no saved state: Save state on the Clip card first")
            })?;
        if c == saved {
            return Ok(format!("reload: {name} is as saved already"));
        }
        if matches!(self.info.state, State::Recording | State::CountIn) {
            return Err("reload: not while recording (UNDO takes a take back)".to_string());
        }
        let grid = c.launch.unwrap_or(self.project.launch);
        let plays = self
            .launches
            .iter()
            .any(|l| (l.track, l.slot) == (track, slot))
            || self
                .cues
                .iter()
                .any(|q| q.track == track && q.slot == Some(slot));
        let pos = self.info.position;
        if plays && self.info.state.rolling() && grid != LaunchGrid::Now {
            let line = grid.next(pos, self.info.meter.bar_ticks());
            // A line within the next period is as good as now.
            if line > pos + self.period_ticks(self.block.frames) {
                self.reloads.push(ReloadWait {
                    track,
                    slot,
                    line,
                    asked: pos,
                });
                self.changed = true;
                return Ok(format!(
                    "reload: {name} reloads at {} (Reload clip again: taken back)",
                    bar_beat(line, self.info.meter)
                ));
            }
        }
        self.reload_now(track, slot)?;
        if let Err(e) = self.compile() {
            self.takes.undo(&mut self.project);
            self.takes.forget_undone();
            return Err(e);
        }
        Ok(format!(
            "reload: {name} is back as saved (UNDO takes it back)"
        ))
    }

    /// Put clip `slot` of `track`'s saved state in its place, as an edit;
    /// the caller compiles. Its name.
    fn reload_now(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let saved = self
            .saved
            .iter()
            .find(|s| (s.0, s.1) == (track, slot))
            .map(|s| s.2.clone())
            .ok_or("reload: no saved state")?;
        let name = saved.name.clone();
        self.takes
            .reload_clip(&mut self.project, track, slot, saved)
            .map_err(|e| format!("reload: {name}: {e:?}"))?;
        Ok(name)
    }

    /// Mutate (sequencer S6, `mpfree_seq::mutate`): clip `slot` of
    /// `track` becomes a new variation of its notes, made from the clip as
    /// it was before its first Mutate, which it keeps for Restore; at
    /// once, as an edit UNDO takes back. `more` (SHIFT) varies about half
    /// its notes, else a quarter; `melodic` lets a note move to the next
    /// pitch the clip plays. Each Mutate sounds new: the seed is the edit
    /// count and the variation.
    fn mutate(
        &mut self,
        track: usize,
        slot: usize,
        more: bool,
        melodic: bool,
    ) -> Result<String, String> {
        use mpfree_seq::{Amount, Pitches};
        let c = self
            .project
            .clip(track, slot)
            .ok_or_else(|| format!("mutate: slot {} of track {} is empty", slot + 1, track + 1))?;
        let name = c.name.clone();
        if c.looped.is_some() {
            return Err("mutate: a loop's cell has no notes to vary".to_string());
        }
        if matches!(self.info.state, State::Recording | State::CountIn) {
            return Err("mutate: not while recording (UNDO takes a take back)".to_string());
        }
        let seed = (u64::from(self.takes.edits()) << 32)
            ^ (u64::from(c.variation().unwrap_or(0)) << 16)
            ^ ((track as u64) << 8)
            ^ slot as u64;
        let amount = if more { Amount::More } else { Amount::Little };
        let pitches = if melodic { Pitches::Own } else { Pitches::Pads };
        let v = c
            .mutated(seed, amount, pitches)
            .ok_or_else(|| format!("mutate: {name} has no notes on to vary"))?;
        let n = v.variation().unwrap_or(1);
        self.replace(track, slot, v, Replaced::Mutate)?;
        Ok(format!(
            "mutate: {name} · variant {n} (‹ on Mutate restores; UNDO takes it back)"
        ))
    }

    /// Restore (S6): clip `slot` of `track` back as it was before its
    /// first Mutate, at once, as an edit UNDO takes back.
    fn restore(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let c = self
            .project
            .clip(track, slot)
            .ok_or_else(|| format!("restore: slot {} of track {} is empty", slot + 1, track + 1))?;
        let name = c.name.clone();
        let r = c
            .restored()
            .ok_or_else(|| format!("restore: {name} plays its original: Mutate it first"))?;
        if matches!(self.info.state, State::Recording | State::CountIn) {
            return Err("restore: not while recording (UNDO takes a take back)".to_string());
        }
        self.replace(track, slot, r, Replaced::Restore)?;
        Ok(format!(
            "restore: {name} is back as it was before Mutate (UNDO takes it back)"
        ))
    }

    /// Put `clip` in place of clip `slot` of `track`, as an edit for
    /// `why`, and compile; on a refusal the project is as it was.
    fn replace(
        &mut self,
        track: usize,
        slot: usize,
        clip: mpfree_seq::Clip,
        why: Replaced,
    ) -> Result<(), String> {
        let name = clip.name.clone();
        let what = match why {
            Replaced::Reload => "reload",
            Replaced::Mutate => "mutate",
            Replaced::Restore => "restore",
        };
        self.takes
            .replace_clip(&mut self.project, track, slot, clip, why)
            .map_err(|e| format!("{what}: {name}: {e:?}"))?;
        if let Err(e) = self.compile() {
            self.takes.undo(&mut self.project);
            self.takes.forget_undone();
            return Err(e);
        }
        Ok(())
    }

    /// The reloads due: each whose line falls in the next period (of
    /// `frames`, as long as the last), or all of them when the transport
    /// stops or jumps back. A reload due while recording is dropped. True
    /// if one went in (the view is to be compiled).
    fn reloads_due(&mut self, frames: u32) -> bool {
        let pos = self.info.position;
        let ahead = self.period_ticks(frames);
        let stopped = !self.info.state.rolling();
        let recording = matches!(self.info.state, State::Recording | State::CountIn);
        let meter = self.info.meter;
        let mut went = false;
        let mut i = 0;
        while i < self.reloads.len() {
            let w = self.reloads[i];
            if !(stopped || w.line <= pos + ahead || pos < w.asked) {
                i += 1;
                continue;
            }
            self.reloads.remove(i);
            self.changed = true;
            let at = if stopped {
                "(STOP)".to_string()
            } else {
                format!("on {}", bar_beat(w.line, meter))
            };
            if recording {
                self.note_later(format!("reload: none {at}: recording started"));
                continue;
            }
            match self.reload_now(w.track, w.slot) {
                Ok(name) => {
                    self.note_later(format!("reload: {name} back as saved {at}"));
                    went = true;
                }
                Err(e) => self.note_later(e),
            }
        }
        went
    }

    /// Song ticks in a period of `frames` frames at the tempo now, rounded
    /// up.
    fn period_ticks(&self, frames: u32) -> Tick {
        mpfree_transport::time::ticks_in(f64::from(frames), self.info.bpm, self.rate).ceil() as Tick
    }

    /// Forget the saved state of the clip `at` (track, slot), and its
    /// reload, or of every clip with None.
    fn forget_saved(&mut self, at: Option<(usize, usize)>) {
        let keep = |t: usize, s: usize| at.is_some_and(|a| a != (t, s));
        self.saved.retain(|x| keep(x.0, x.1));
        self.reloads.retain(|r| keep(r.track, r.slot));
        self.changed = true;
    }

    fn landed_line(&self, at: Tick) -> String {
        let Some((_, q)) = &self.mute_queue else {
            return String::new();
        };
        let p = self.info.meter.position(at);
        let mut line = format!("track mute: at {}.{}", p.bar, p.beat);
        for &(t, m, s) in &q.want {
            let Some(tr) = self.project.tracks.get(t) else {
                continue;
            };
            let what = if s != tr.solo {
                if s {
                    "solo"
                } else {
                    "solo off"
                }
            } else if m {
                "muted"
            } else {
                "plays"
            };
            line.push_str(&format!(", {} {what}", tr.name));
        }
        line
    }

    /// A line for the log once this period's bookkeeping is done
    /// ([`TimeControl::take_later_note`]), after any already waiting.
    fn note_later(&mut self, line: String) {
        self.later_note = Some(match self.later_note.take() {
            Some(n) => format!("{n}; {line}"),
            None => line,
        });
    }

    /// A scene's mutes for the tracks it gave no cue (nothing playing or
    /// cued, an empty cell: `Perform::take_line_mutes`) join the mute
    /// queue, so they land on the scene's line with its cues, as TRACK
    /// MUTE's do: in song mode such a track plays the song until then.
    /// Each replaces what waited for its track (the scene is the later
    /// change), and the queue lands on the scene's line from then on,
    /// with whatever else waits in it. True when the queue changed.
    fn queue_scene_mutes(&mut self) -> bool {
        let Some((line, mutes)) = self.perform.take_line_mutes() else {
            return false;
        };
        let mut changed = false;
        for (t, m) in mutes.iter().enumerate() {
            let (Some(m), Some(tr)) = (*m, self.project.tracks.get(t)) else {
                continue;
            };
            let now = (tr.mute, tr.solo);
            let queued = self
                .mute_queue
                .as_ref()
                .and_then(|(_, q)| q.want.iter().find(|w| w.0 == t))
                .map(|w| (w.1, w.2));
            let want = (m, queued.map_or(now.1, |w| w.1));
            if queued.unwrap_or(now) == want {
                continue;
            }
            changed = true;
            if let Some((_, q)) = &mut self.mute_queue {
                q.want.retain(|w| w.0 != t);
            }
            if want != now {
                let ids = &mut self.mute_ids;
                let (_, q) = self.mute_queue.get_or_insert_with(|| {
                    *ids += 1;
                    (*ids, MuteWait::new(line))
                });
                q.want.push((t, want.0, want.1));
            }
        }
        if changed {
            if let Some((_, q)) = &mut self.mute_queue {
                q.line = line;
            }
            if self.mute_queue.as_ref().is_some_and(|(_, q)| q.is_empty()) {
                self.mute_queue = None;
            }
            self.changed = true;
        }
        changed
    }

    /// Land the mute queue at once, as STOP does with whatever waits.
    fn land_queue_now(&mut self) -> bool {
        let Some((_, q)) = self.mute_queue.take() else {
            return false;
        };
        for (t, m, s) in q.want {
            if let Some(tr) = self.project.tracks.get_mut(t) {
                (tr.mute, tr.solo) = (m, s);
            }
        }
        for (p, m, s) in q.pads {
            if let Some(l) = self.pad_landings.get_mut(p) {
                *l = Some((m, s));
            }
        }
        true
    }

    /// The kit's pads waiting in the mute queue land once their line is
    /// within the next period (`frames` long), or at once with the
    /// transport stopped: they go to [`TimeControl::take_pad_landings`]
    /// for the mixer, and the log says so.
    fn pads_due(&mut self, frames: u32) {
        let Some((_, q)) = &self.mute_queue else {
            return;
        };
        if q.pads.is_empty() {
            return;
        }
        let pos = self.info.position;
        let line = q.line.next(pos, self.info.meter.bar_ticks());
        let rolling = self.info.state.rolling();
        if rolling && line > pos + self.period_ticks(frames) {
            return;
        }
        let words = self.pads_line(if rolling { line } else { pos });
        if let Some((_, q)) = &mut self.mute_queue {
            for &(p, m, s) in &q.pads {
                if let Some(l) = self.pad_landings.get_mut(p) {
                    *l = Some((m, s));
                }
            }
            q.pads.clear();
            if q.is_empty() {
                self.mute_queue = None;
            }
        }
        self.changed = true;
        self.note_later(if rolling {
            words
        } else {
            format!("{words} (STOP)")
        });
    }

    /// The log's lines for the mute queue landing on `at`: its tracks'
    /// ([`TimeControl::landed_line`]) and its pads' ([`TimeControl::pads_line`]),
    /// each if it has any.
    fn queue_lines(&self, at: Tick) -> String {
        let Some((_, q)) = &self.mute_queue else {
            return String::new();
        };
        match (q.want.is_empty(), q.pads.is_empty()) {
            (false, true) => self.landed_line(at),
            (true, false) => self.pads_line(at),
            (false, false) => format!("{}; {}", self.landed_line(at), self.pads_line(at)),
            (true, true) => String::new(),
        }
    }

    /// The log's line for the kit's pads in the queue landing on `at`:
    /// "pad mute: at 5.1, pad 3 muted, pad 1 solo".
    fn pads_line(&self, at: Tick) -> String {
        let p = self.info.meter.position(at);
        let mut line = format!("pad mute: at {}.{}", p.bar, p.beat);
        if let Some((_, q)) = &self.mute_queue {
            for &(pad, m, s) in &q.pads {
                line.push_str(&format!(", pad {} {}", pad + 1, pad_words(m, s)));
            }
        }
        line
    }

    /// The kit's pads whose change in the mute queue came due, each with
    /// its mute and solo: the mixer takes them before its next period.
    /// Allocates nothing.
    pub fn take_pad_landings(&mut self) -> [Option<(bool, bool)>; PADS] {
        std::mem::take(&mut self.pad_landings)
    }

    /// A pad of the kit tapped as a mute key (SHIFT + TRACK MUTE, kit 1's
    /// pads): its mute toggled, or its solo with `solo`, from `now` (the
    /// mixer's mute and solo for it) or from what waits for it. With
    /// `wait` (Perform), the transport rolling and Launch not at once it
    /// joins the mute queue and lands with it ([`PadTap::Waits`]); a tap
    /// on a pad waiting takes its change back. Otherwise it acts at once
    /// ([`PadTap::Now`]: the mixer sets it).
    pub fn pad_mute_or_solo(
        &mut self,
        pad: usize,
        now: (bool, bool),
        wait: bool,
        solo: bool,
    ) -> PadTap {
        let queued = self
            .mute_queue
            .as_ref()
            .and_then(|(_, q)| q.pads.iter().find(|w| w.0 == pad))
            .map(|w| (w.1, w.2));
        let (mut m, mut s) = queued.unwrap_or(now);
        if solo {
            s = !s;
        } else {
            m = !m;
        }
        let line = self
            .mute_queue
            .as_ref()
            .map_or(self.project.launch, |(_, q)| q.line);
        if let Some((_, q)) = &mut self.mute_queue {
            q.pads.retain(|w| w.0 != pad);
        }
        let waits = wait && self.info.state.rolling() && line != LaunchGrid::Now;
        if waits && (m, s) != now {
            let ids = &mut self.mute_ids;
            let (_, q) = self.mute_queue.get_or_insert_with(|| {
                *ids += 1;
                (*ids, MuteWait::new(line))
            });
            q.pads.push((pad, m, s));
        }
        if self.mute_queue.as_ref().is_some_and(|(_, q)| q.is_empty()) {
            self.mute_queue = None;
        }
        self.changed = true;
        match (waits, (m, s) == now) {
            (true, true) => {
                PadTap::Waits(format!("pad mute: pad {}: its change taken back", pad + 1))
            }
            (true, false) => PadTap::Waits(format!(
                "pad mute: pad {} {} {}",
                pad + 1,
                match (solo, m, s) {
                    (true, _, true) => "solo",
                    (true, _, false) => "solo off",
                    (false, true, _) => "muted",
                    (false, false, _) => "plays",
                },
                line_words(line)
            )),
            (false, _) => PadTap::Now { mute: m, solo: s },
        }
    }

    /// Whether a change waits for the kit's pad `pad` in the mute queue,
    /// for its light.
    pub fn pad_waiting(&self, pad: usize) -> bool {
        self.mute_queue
            .as_ref()
            .is_some_and(|(_, q)| q.pads.iter().any(|w| w.0 == pad))
    }

    /// The mute queue waiting, for the screens.
    pub fn mute_wait(&self) -> Option<&MuteWait> {
        self.mute_queue.as_ref().map(|(_, q)| q)
    }

    /// Whether track `t` is muted (None: no such track), for TRACK MUTE's
    /// pad lights.
    pub fn muted(&self, t: usize) -> Option<bool> {
        self.project.tracks.get(t).map(|tr| tr.mute)
    }

    /// The tracks that are silent now, bit `t` for track `t`: muted, or
    /// not soloed while another track is. Never allocates.
    pub fn silenced(&self) -> u128 {
        let tracks = &self.project.tracks;
        let solo = tracks.iter().any(|t| t.solo);
        tracks
            .iter()
            .take(128)
            .enumerate()
            .filter(|(_, t)| t.mute || (solo && !t.solo))
            .fold(0, |bits, (i, _)| bits | 1 << i)
    }

    /// The tracks muted and soloed as they will be once the mute queue
    /// lands, bit `t` for track `t`: where MIDI Learn reads a learned mute
    /// or solo, so a key flips it from there.
    pub fn mute_bits(&self) -> (u128, u128) {
        let queue = self
            .mute_queue
            .as_ref()
            .map_or(&[][..], |(_, q)| &q.want[..]);
        let mut bits = (0u128, 0u128);
        for (t, tr) in self.project.tracks.iter().enumerate().take(128) {
            let (m, s) = queue
                .iter()
                .find(|w| w.0 == t)
                .map_or((tr.mute, tr.solo), |w| (w.1, w.2));
            bits.0 |= u128::from(m) << t;
            bits.1 |= u128::from(s) << t;
        }
        bits
    }

    /// What TRACK MUTE's pad for track `t` shows (None: no such track).
    pub fn mute_light(&self, t: usize) -> Option<MuteLight> {
        let tr = self.project.tracks.get(t)?;
        let waiting = self
            .mute_queue
            .as_ref()
            .is_some_and(|(_, q)| q.want.iter().any(|w| w.0 == t));
        Some(MuteLight {
            silent: self.project.silent(t),
            solo: tr.solo,
            waiting,
            launched: self.launched(t),
        })
    }

    /// Which loops the Loop tracks have on (loop-tracks.md): a loop plays
    /// while its cell is launched on a Loop track that isn't muted, while
    /// a block of it plays in song mode, or while it is in no cell. A
    /// change from the last poll's cues lands on the cue's bar line, one
    /// from a block on the block's start or end; any other at once.
    /// Allocates nothing.
    pub fn loop_plan(&self) -> LoopPlan {
        let launched = self.launches.iter().map(|l| (l.track, l.slot));
        let mut on = loops::playing(&self.project, launched);
        for (on, song) in on.iter_mut().zip(self.song_loops) {
            *on |= song;
        }
        LoopPlan {
            on,
            at: self.loop_at,
        }
    }

    /// What the looper says of its loops, each period. A loop whose first
    /// pass just ended (or that an open brought) and that is in no cell
    /// lands on a Loop track with nothing playing, in the row of the scene
    /// playing (row 1 if none), launched at once: it plays already; a redo
    /// of a first pass undone goes back to the cell it left, if it can. A
    /// loop's cells follow its length, and empty when it is cleared. Not
    /// undo steps: the looper's undo takes a loop back, and its cell goes
    /// with it. Lines for the log; allocates only when something changed.
    pub fn loops_are(&mut self, now: &[LoopNow; LOOPS]) -> Vec<String> {
        let mut lines = Vec::new();
        if *now == self.loops_seen {
            return lines;
        }
        let mut compile = false;
        for (n, (&is, was)) in now.iter().zip(self.loops_seen).enumerate() {
            if is == was {
                continue;
            }
            let k = n as u8;
            match (was.pass, is.pass) {
                (false, true) if !loops::in_cell(&self.project, k) => {
                    let row = self.perform.playing().map_or(0, |p| p.row.min(ROWS - 1));
                    let (launches, cues) = (&self.launches, &self.cues);
                    let busy = |t: usize| {
                        launches.iter().any(|l| l.track == t) || cues.iter().any(|c| c.track == t)
                    };
                    // A redo of the pass that went goes back to its cell,
                    // if that is free and its Loop track plays nothing.
                    let project = &mut self.project;
                    let back = self.loops_gone[n].take().and_then(|((t, s), went)| {
                        let free = project
                            .tracks
                            .get(t)
                            .is_some_and(|tr| tr.is_loops() && tr.slots[s].is_none());
                        (went == is && free && !busy(t)).then_some((t, s))
                    });
                    // Else the armed Loop cell takes it, if it is still
                    // empty.
                    let loop_cell = &mut self.loop_cell;
                    let armed = || {
                        loop_cell.take().filter(|&(t, s)| {
                            project
                                .tracks
                                .get(t)
                                .is_some_and(|tr| tr.is_loops() && tr.slots[s].is_none())
                        })
                    };
                    let landed = match back.or_else(armed) {
                        Some((t, s)) => {
                            let _ = project.put_new_clip(t, s, Clip::new_loop(k, is.ticks));
                            // One loop plays per Loop track: the one there
                            // stops as this one plays.
                            self.launches.retain(|l| l.track != t);
                            self.cues.retain(|c| c.track != t);
                            Some((t, s))
                        }
                        None => loops::place(project, k, is.ticks, row, busy),
                    };
                    match landed {
                        Some((track, slot)) => {
                            self.launches.push(Launch {
                                track,
                                slot,
                                origin: is.origin,
                                start: is.origin,
                            });
                            lines.push(format!(
                                "Loop {} on {}, row {}, playing",
                                n + 1,
                                self.project.tracks[track].name,
                                slot + 1
                            ));
                            compile = true;
                        }
                        None => lines.push(format!(
                            "Loop {} is on no Loop track: the project has {} tracks",
                            n + 1,
                            self.project.tracks.len()
                        )),
                    }
                }
                (_, true) => compile |= loops::set_length(&mut self.project, k, is.ticks) > 0,
                (true, false) => {
                    let first = loops::cells(&self.project, k).next();
                    self.loops_gone[n] = first.map(|cell| (cell, was));
                    if loops::remove(&mut self.project, k) > 0 {
                        lines.push(format!("Loop {} cleared: its cells are empty", n + 1));
                        compile = true;
                    }
                }
                (false, false) => {}
            }
        }
        self.loops_seen = *now;
        if compile {
            if let Err(e) = self.compile() {
                lines.push(e);
            }
        }
        lines
    }

    /// REDOs done so far (`TimeHost::redos`).
    pub fn redos(&self) -> u64 {
        self.redos
    }

    /// Whether REDO has anything to do.
    pub fn can_redo(&self) -> bool {
        !self.takes.undone().is_empty()
    }

    /// The project's tracks, for TRACK MUTE's banks.
    pub fn track_count(&self) -> usize {
        self.project.tracks.len()
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

    /// Song mode is on.
    pub fn song_on(&self) -> bool {
        self.song
    }

    /// The project is in Perform, not Make (`Project::perform`).
    pub fn perform(&self) -> bool {
        self.project.perform
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
        st.track_pick = self.track_pick;
        st.also = self.also();
        st.capture = self.pending.map(|p| bars_label(p.bars));
        st.clipboard = self.clipboard.as_ref().map(|c| c.name.clone());
        st.cues = self.cues.clone();
        st.mute_wait = self.mute_wait().cloned();
        st.saved = self.saved.iter().map(|s| (s.0, s.1)).collect();
        st.reloads = self
            .reloads
            .iter()
            .map(|r| (r.track, r.slot, r.line))
            .collect();
        st.cue_lines = self
            .cues
            .iter()
            .filter_map(|c| Some((c.track, self.perform.cue_line(c.track)?)))
            .collect();
        st.scene_playing = self.perform.playing().map(|p| p.row);
        st.scene_cued = self.perform.cued();
        st.scene_focus = self.scene_pick.or(st.scene_playing).unwrap_or(0);
        st.scene = self
            .project
            .scene(st.scene_focus)
            .map(mpfree_seq::Scene::settings)
            .unwrap_or_default();
        st.loop_cell = self.loop_cell();
        st.morph = self.morph;
        st.song_view = self.song_view;
        st.list_view = self.list_view;
        st.list = self.list_rows(&st.scenes);
        st.list_pick = self.list_pick.filter(|&k| k < st.list.len());
        st.list_looped = self.project.arrangement.list.looped;
        st.list_next = self
            .perform
            .list_next(&self.project, self.info.meter.bar_ticks());
        st.song_on = self.song;
        st.selected = self.selected;
        st.region = self.region;
        st.section = self.section;
        st.tempo_base = self.project_bpm;
        st.tempo_pick = self.tempo_pick.filter(|&k| k < st.tempo_points.len());
        st.tempo_here = self.tempo_here();
        st.song_rec = self.song_rec.armed();
        st.song_with_take = self.song_with_take();
        st.follow_next = self
            .perform
            .scene_follow(&self.project, self.info.meter.bar_ticks())
            .map(|(row, at, follow)| FollowNext { at, row, follow });
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

    /// The song list's steps as the screen shows them, with the scenes'
    /// names `scenes` (a plain row is "Row 3").
    fn list_rows(&self, scenes: &[String]) -> Vec<ListRow> {
        let bar = self.info.meter.bar_ticks().max(1);
        let now = self.info.position;
        let pass = self.perform.list_pass(&self.project, now, bar);
        let waiting = self.perform.list_waiting(now);
        let spans = list_spans(&self.project, bar);
        self.project
            .arrangement
            .list
            .steps
            .iter()
            .zip(spans)
            .enumerate()
            .map(|(k, (s, (at, len)))| ListRow {
                name: scenes
                    .get(s.row)
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("Row {}", s.row + 1)),
                row: s.row,
                times: s.times,
                bars: len / bar,
                starts: at / bar + 1,
                playing: pass.filter(|p| p.0 == k).map(|p| (p.1, p.2)),
                waiting: waiting == Some(k),
            })
            .collect()
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
        // Moments too busy (refactor A2): what gave way was new, never an
        // ending. Endings lost is a bug, said whenever it isn't 0.
        line.push_str(&format!(
            "; {} new notes or locks left out of moments too busy, {} endings lost",
            c.busy, c.ends_lost
        ));
        // Pressure no note could play again, freed out of recording (B4b).
        if self.touch_freed.1 > 0 {
            line.push_str(&format!(
                "; {} unused pressure values freed ({} times)",
                self.touch_freed.0, self.touch_freed.1
            ));
        }
        // Several armed tracks (rec-arm.md "Several tracks"): a track a take
        // left out for want of room.
        if self.also_full > 0 {
            line.push_str(&format!(
                "; {} armed tracks left out of a take with no room for a clip",
                self.also_full
            ));
        }
        // The fallbacks between the halves, when there are any.
        let lost = c.lost + self.lost();
        if lost + u64::from(c.behind + c.kept + c.refused + c.missed) > 0 {
            line.push_str(&format!(
                "; between the halves: {lost} messages lost, {} late views compiled again, \
                 {} views kept unfreed, {} transport commands refused, {} step edits \
                 compiled again",
                c.behind, c.kept, c.refused, c.missed
            ));
        }
        // The timing contract (Arrange 2.5): cues the audio half carried
        // out after their line, in phase with it, because their view came
        // late; older ones it dropped; clips a newer cue took away first.
        let q = &self.cue_counts;
        if q.late + q.stale + q.lost > 0 {
            line.push_str(&format!(
                "; cues whose view came late: {} started after their line, in time with it, \
                 {} older ones dropped, {} times a track was silent until the next view",
                q.late, q.stale, q.lost
            ));
        }
        // What compiles and step edits cost (the device's numbers for the
        // control half's own thread).
        line.push_str("; ");
        line.push_str(&self.costs.describe());
        // Following another device's clock, when it had to put anything right.
        let f = &self.follow;
        if f.any() {
            line.push_str(&format!(
                "; following a clock: {} jumps to its position, {} times its clock \
                 stopped while it played, {} commands refused",
                f.jumps, f.lost, f.refused
            ));
        }
        line
    }
}

/// "bar 9", or "bar 9 beat 3" off the bar line: song tick `t` as the Song
/// view's drags and the wheel say where a block or section went.
/// What Make or Perform switches, for the hint line (sequencer S4; the
/// design's "Perform: mutes land on the bar, deletes need a hold").
pub fn perform_line(on: bool) -> &'static str {
    if on {
        "Perform: mutes land on the bar, deletes need a hold, MAIN goes to the Clips grid"
    } else {
        "Make: mutes and deletes act at once, MAIN goes to Play"
    }
}

/// The row with a clip after `row` (`delta` > 0) or before it, going
/// round, from the rows with a clip `filled` in order; `row` itself when
/// it is the only one, None when no row has a clip.
fn next_filled(filled: &[usize], row: usize, delta: i32) -> Option<usize> {
    if delta >= 0 {
        filled
            .iter()
            .find(|&&r| r > row)
            .or_else(|| filled.first())
            .copied()
    } else {
        filled
            .iter()
            .rev()
            .find(|&&r| r < row)
            .or_else(|| filled.last())
            .copied()
    }
}

pub(super) fn bar_beat(t: Tick, meter: mpfree_transport::time::Meter) -> String {
    let bar = meter.bar_ticks().max(1);
    let beat = meter.beat_ticks().max(1);
    let n = t.div_euclid(bar) + 1;
    match t.rem_euclid(bar) / beat {
        0 => format!("bar {n}"),
        b => format!("bar {n} beat {}", b + 1),
    }
}

/// The steps whose bits are set (step 0 the lowest).
/// A step row as the log names it: "pad 5", or "the line".
fn row_name(pad: u8) -> String {
    if pad == LINE {
        "the line".to_string()
    } else {
        format!("pad {}", pad + 1)
    }
}

/// A step change for the lines: as Debug says it, but the Note knob's pad
/// counted from 1, as the lines count pads.
fn change_name(change: mpfree_seq::steps::Change) -> String {
    match change {
        mpfree_seq::steps::Change::Note(to) => format!("lowest note on pad {}", u16::from(to) + 1),
        c => format!("{c:?}"),
    }
}

fn step_list(steps: u128) -> Vec<usize> {
    (0..128).filter(|i| steps & (1u128 << i) != 0).collect()
}

/// "step 5", or "steps 1, 5".
fn steps_text(steps: &[usize]) -> String {
    let list: Vec<String> = steps.iter().map(|i| (i + 1).to_string()).collect();
    if list.len() == 1 {
        format!("step {}", list[0])
    } else {
        format!("steps {}", list.join(", "))
    }
}

/// How many of `gestures` (a move log's phrase) overlap song ticks
/// `from..to`, as `moves_for_clip` takes them.
fn moves_over(gestures: &[mpfree_seq::mpfree_moves::Gesture], from: f64, to: f64) -> usize {
    gestures
        .iter()
        .filter(|g| match (g.points.first(), g.points.last()) {
            (Some(a), Some(b)) => g.song + (b.0 - a.0) >= from && g.song < to,
            _ => false,
        })
        .count()
}

/// How a conversion played out, for its log line (plan D7): "exact", or
/// how many differences and each in words ("2 differences" and ": Bass,
/// bar 9: a note held over the row change is cut there; …").
fn differences_said(project: &Project, d: &[Difference], bar: Tick) -> (String, String) {
    match d.len() {
        0 => ("exact".into(), String::new()),
        n => {
            let each: Vec<String> = d.iter().map(|d| d.said(project, bar)).collect();
            let s = if n == 1 { "" } else { "s" };
            (
                format!("{n} difference{s}"),
                format!(": {}", each.join("; ")),
            )
        }
    }
}
