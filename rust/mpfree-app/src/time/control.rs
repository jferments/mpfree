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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mpfree_audiotracks::{Piece, Retime, SongStep};
use mpfree_capture::{keep, moves_for_clip, Capture, KeepError, Kept, Now, Pending};
use mpfree_notefx::{Chain, TrackFx, MAX_FX};
use mpfree_perform::{Perform, SceneMorph, SongRecorder, When};
use mpfree_play::Replaced;
use mpfree_time_ui::adding::{AddAction, ChordsAction};
use mpfree_time_ui::continuing::{ContinueAction, ContinueAsk, Parts};

use super::adding::{fitted_clips, unfit, Adding, Hearing};
use super::compose::{more_bars, Composing, LineWait};
use mpfree_play::{
    ClipDrop, Cue, Decisions, Includes, Launch, MuteCue, MuteLanded, PlayView, SceneTempo,
    SongChange, Started, Takes,
};
use mpfree_play::{MOVE_POINTS, TOUCH_POINTS};
use mpfree_seq::arrange::{
    list_spans, put_scene_tempo, scene_title, song_sections, song_to_scenes, Arrangement,
    Difference, Differs, ScenesRefused, SongSound,
};
use mpfree_seq::loops::{self, LoopNow, LoopPlan};
use mpfree_seq::mpfree_moves::{Input, Kind, MoveLog};
use mpfree_seq::notes::{self, CarryError, Copied};
use mpfree_seq::scenes::{MAX_LIST_STEPS, MAX_TIMES};
use mpfree_seq::steps::{size_name, Edit, Grid, DEFAULT_SIZE};
use mpfree_seq::time_edit::Refused;
use mpfree_seq::ListStep;
use mpfree_seq::TimeEdit;
use mpfree_seq::{
    Clip, ClipId, ClipRegion, Holds, LaunchGrid, Lock, LoopId, ModelError, Project, RecId,
    TouchPoint, Track, TrackId, TrackKind, LOOPS, MAX_TRACKS, ROWS,
};
use mpfree_time_ui::editor::compose::Compose;
use mpfree_time_ui::editor::{Done, EditAction, Editor, ScaleOffers, ToolKind, CONTINUE_ROW};
use mpfree_time_ui::list::ListRow;
use mpfree_time_ui::view::pad_name;
use mpfree_time_ui::view::{
    bars_beats, bars_label, card_rows, clips_after, gain_words, grid_name, rows_named, scene_rows,
    AudioAction, AudioSet, BarsEdit, Converting, EditAnswer, EditInfo, EditOutcome, FollowNext,
    Ghost, LinkFrom, MarkerEdit, MorphInfo, MuteWait, RegionId, Report, ReportRow, TakeRow,
    TempoEdit, TimeAction, TimeState, TransportInfo, AUDIO_GAIN, SCENE_NAME,
};
use mpfree_transport::time::{TempoMap, Tick, PPQ};
use mpfree_transport::{Block, Command, State, TransactionId};

use super::audio_wait::{AudioNeed, AudioWaits, Ctx as WaitCtx, CueKey, Wait};
use super::follow::FollowCounts;
use super::link::{
    Counts, CueCounts, InPlace, PeriodEnd, Rx, StripCounts, ToAudio, ToControl, Tracks, Tx,
};
use super::timing::next_timeline;
use super::{
    InstRef, PadRows, SilentLocks, StepAction, StepClip, StepsData, LINE, NEW_CLIP_BARS,
    RECORD_ROOM,
};

mod linked;
mod parts;
mod sections;
mod styles;
pub use linked::edits_steps;

/// What a cue waiting for its audio needs from the control half
/// ([`WaitCtx`]), borrowed field by field so the waits can change beside
/// it.
macro_rules! wait_ctx {
    ($c:expr) => {
        WaitCtx {
            project: &$c.project,
            cues: &$c.cues,
            launches: &$c.launches,
            perform: &$c.perform,
            queue: $c.mute_queue.as_ref().map(|(_, q)| q),
            info: &$c.info,
            rate: $c.rate,
            heard: $c.heard,
        }
    };
}

/// The clip settings card's join keys for its takes, a row each (see
/// `Takes::set_clip`).
const CARD_JOIN: u64 = 0xC1_0000;

/// The scene editor's join keys for its takes, a card row each (see
/// `Takes::set_scene`).
const SCENE_JOIN: u64 = 0x5C_0000;

/// An audio clip's card's join keys for its Length and Gain takes, a row
/// each (see `Takes::set_audio_clip`).
const AUDIO_CARD_JOIN: u64 = 0xA1 << 16;

/// Started cues one period may bring before the list grows.
const STARTED_ROOM: usize = 64;

/// The tracks' mutes and solos as they will be once a mute queue lands,
/// read without changing the project.
struct Landed<'a> {
    project: &'a Project,
    q: &'a MuteWait,
    soloing: bool,
}

/// A kit pad tapped as a mute key ([`TimeControl::pad_mute_or_solo`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PadTap {
    /// It waits in the mute queue (or its change was taken back): the
    /// line for the log.
    Waits(String),
    /// At once: the pad's mute and solo now on its track; the line for
    /// the log.
    Now(String),
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
    /// Note editor edits made in place, and those that compiled (piano
    /// roll plan 0C.1), with the slowest of each: the control half's time
    /// from the touch to the edit sent ([`TimeControl::edit`]), on
    /// whichever thread it runs.
    pub notes_in_place: u64,
    pub notes_compiled: u64,
    pub slowest_note_in_place: Duration,
    pub slowest_note_compiled: Duration,
}

impl Costs {
    /// "compiles: 12 (slowest 0.21 ms, 96 notes and locks); step edits:
    /// 40 in place (slowest 0.02 ms), 3 compiled (slowest 0.25 ms); note
    /// edits: 5 in place (slowest 0.03 ms), 1 compiled (slowest 0.30 ms)".
    /// The note edits come last, so the device checks that read the rest
    /// (`tools/mpc-one/app_checks.py`) read it as before.
    pub fn describe(&self) -> String {
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        format!(
            "compiles: {} (slowest {:.2} ms, {} notes and locks); step edits: {} in place \
             (slowest {:.2} ms), {} compiled (slowest {:.2} ms); note edits: {} in place \
             (slowest {:.2} ms), {} compiled (slowest {:.2} ms)",
            self.compiles,
            ms(self.slowest_compile),
            self.slowest_compile_notes,
            self.steps_in_place,
            ms(self.slowest_in_place),
            self.steps_compiled,
            ms(self.slowest_step_compiled),
            self.notes_in_place,
            ms(self.slowest_note_in_place),
            self.notes_compiled,
            ms(self.slowest_note_compiled)
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

/// Notes copied in the note editor (piano roll plan 1B.4): the copy, the
/// track it came from by its lasting id with its MIDI effects as they
/// were then, the instrument its instrument locks were made for (the
/// clip's `made_for`, else what the track played; refactor F1), and how
/// the hint line says it ("4 notes from Verse").
#[derive(Clone, Debug)]
struct NoteCopy {
    copied: Copied,
    from: TrackId,
    fx: TrackFx,
    inst: Option<mpfree_seq::inst::Inst>,
    said: String,
}

/// What a paste into `track` left out, for its line: how many locks, and
/// on what, since the track may not have those settings; for instrument
/// locks, why, when it is known (`inst_why`, refactor F1).
fn left_out(left: &[Lock], track: &str, inst_why: Option<&str>) -> String {
    use mpfree_seq::mpfree_param::OnTrack;
    // What each is on, as one and as many, in this order.
    let kind = |on: OnTrack| match on {
        OnTrack::Inst => 0,
        OnTrack::Pad(_) => 1,
        OnTrack::Insert(_) => 2,
        OnTrack::Midi => 3,
        _ => 4,
    };
    const ON: [(&str, &str); 5] = [
        ("the instrument", "the instrument"),
        ("a kit pad", "kit pads"),
        ("an insert", "inserts"),
        ("MIDI out", "MIDI out"),
        ("a MIDI effect", "MIDI effects"),
    ];
    let on: Vec<String> = (0..ON.len())
        .filter_map(|k| {
            let n = left.iter().filter(|l| kind(l.addr.on) == k).count();
            let (one, many) = ON[k];
            let why = match inst_why {
                Some(why) if k == 0 => format!(" ({why})"),
                _ => String::new(),
            };
            (n > 0).then(|| format!("{n} on {}{why}", if n == 1 { one } else { many }))
        })
        .collect();
    let n = left.len();
    format!(
        " · {n} lock{} left out, as {track} may not have {}: {}",
        if n == 1 { "" } else { "s" },
        if n == 1 {
            "its setting"
        } else {
            "their settings"
        },
        on.join(", ")
    )
}

/// The audio tracks, as a song time edit sees them (the Arrange refactor's
/// 3.4): what [`TimeControl::act_with_audio`] asks them before it edits
/// the song, so a refusal changes nothing anywhere, and tells them after,
/// so their regions move in the same period as the song.
pub trait SongAudio {
    /// Why they can't take any change now (a project's tracks going in, a
    /// change waiting for the period), in the musician's words.
    fn busy(&self) -> Option<String>;
    /// Why they can't take the edit now (busy, a track recording, no room
    /// for the cut regions).
    fn refused(&self, r: &Retime) -> Option<String>;
    /// Whether the edit moves any region.
    fn moves(&self, r: &Retime) -> bool;
    /// The edit, as song edit `song`'s undo step; false when it moves no
    /// region (nothing to take back).
    fn retime(&mut self, song: mpfree_transport::TransactionId, r: Retime) -> bool;
    /// Where song edit `song`'s step is, for UNDO (`undo`) or REDO.
    fn step(&self, song: mpfree_transport::TransactionId, undo: bool) -> SongStep;
    /// Take back (`undo`) or bring back song edit `song`'s steps.
    fn song_undo(&mut self, song: mpfree_transport::TransactionId, undo: bool);
    /// Each audio track's regions as the song plays them, by its lasting
    /// id: in order, none overlapping, start and end in song ticks, and
    /// the rest (recording, first frame, gain, fades and what they were
    /// cut from) exactly as they sound. Tracks with none are left out.
    /// Song → scenes turns them into audio clips (audio tracks phase 2,
    /// step 2d); nothing else asks, and nothing keeps them after.
    fn regions(&self) -> Vec<(TrackId, Vec<ClipRegion>)> {
        Vec::new()
    }
}

/// Whether track `tr` takes a loop recorded into an armed cell: a Loop
/// track, or with loops on audio tracks (`on`, Dev's switch) an audio
/// track.
fn takes_loops(tr: &Track, on: bool) -> bool {
    tr.is_loops() || (on && tr.is_audio())
}

/// The edit a bars action makes (`b` ticks a bar), with the tick it
/// starts on, how many ticks, and whether it puts bars in.
fn bars_edit(
    song: &mpfree_seq::arrange::Arrangement,
    e: BarsEdit,
    b: Tick,
) -> Result<(TimeEdit, Tick, Tick, bool), String> {
    let section = |k: usize| {
        song.section(k, b)
            .ok_or_else(|| "song: there's no such section".to_string())
    };
    Ok(match e {
        BarsEdit::Insert { at, bars } => {
            let len = Tick::from(bars) * b;
            (TimeEdit::Insert { at, len }, at, len, true)
        }
        BarsEdit::RoomBefore(k) => {
            let (at, _) = section(k)?;
            (TimeEdit::Insert { at, len: b }, at, b, true)
        }
        BarsEdit::Delete { from, to } => (TimeEdit::Delete { from, to }, from, to - from, false),
        BarsEdit::DeleteSection(k) => {
            let (from, to) = section(k)?;
            (TimeEdit::Delete { from, to }, from, to - from, false)
        }
    })
}

/// A song time edit for the audio tracks' regions: its pieces. None for
/// one that moves nothing, or with more pieces than an edit has.
/// Whether action `a` moves time in the song: a section moved or copied,
/// bars put in or taken out (with room before or a section deleted), a
/// tempo point added, moved or changed, song mode on or off. Markers,
/// blocks, clips, launches and mutes don't (agreed with the Arrange
/// refactor for step 0C).
fn moves_song_time(a: &TimeAction) -> bool {
    match a {
        TimeAction::MoveSection { .. } | TimeAction::SongBars(_) | TimeAction::SongMode(_) => true,
        TimeAction::SongTempo(e) => !matches!(e, TempoEdit::Pick(_)),
        _ => false,
    }
}

pub fn retime_of(edit: &TimeEdit) -> Option<Retime> {
    if edit.moves_nothing() {
        return None;
    }
    let pieces: Vec<Piece> = edit
        .pieces()
        .ok()?
        .into_iter()
        .map(|(from, end, to)| Piece {
            from,
            end: end.unwrap_or(Tick::MAX),
            to,
        })
        .collect();
    Retime::new(&pieces)
}

/// The XY pad's moves for the song that one period can bring (a value and
/// a let-go for each of a layout's settings, twice over).
const SONG_MOVES: usize = mpfree_xy::SLOTS * 4;

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
    /// The scene card open over the grid or the list (5.2): its row.
    scene_open: Option<usize>,
    selected: Option<usize>,
    /// The blocks picked together on the Song view (6.5), in order: two
    /// or more, `selected` being None while they are; empty otherwise.
    picks: Vec<usize>,
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
    /// The last conversion's report card, while it is open (the Arrange
    /// refactor's 4.3): set by a conversion whose result plays
    /// differently, closed by its Done (`TimeAction::CloseReport`), by
    /// another conversion and by UNDO or REDO.
    report: Option<Arc<Report>>,
    /// The project's tempo (the app's, [`TimeControl::project_tempo`]):
    /// the song's tempo before its first tempo point.
    project_bpm: f64,
    /// Live to song: records what the grid plays into the song.
    song_rec: SongRecorder,
    /// Perform's Rec song with REC: armed by the take, or taken off for
    /// it ([`TimeControl::take_follows`]).
    song_take: SongTake,
    /// A linked song as it plays (4b.4, `sections`): where the section
    /// whose mutes and program changes came in last starts, where the one
    /// cued for its first tick starts, and the playhead as the last period
    /// left it.
    section_now: Option<Tick>,
    section_next: Option<Tick>,
    section_last: Tick,
    /// The transport's loop Loop the list set on a linked list, and the
    /// musician's it kept aside.
    list_loop: Option<(Tick, Tick)>,
    loop_kept: Option<(Tick, Tick)>,
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
    /// The editor's clip as the editor last knew it, and its revision,
    /// which moves each time the clip differs from that copy (piano roll
    /// plan 0A.1): the editor's own edits, and anything else that changed
    /// it, which the picks follow ([`TimeControl::edit_follow`]). Shared
    /// with the screens: [`TimeControl::state`] sends this same copy while
    /// the clip is unchanged, so they rebuild the roll only when it moved
    /// (plan 0C.1).
    edit_seen: Option<Arc<Clip>>,
    /// The other clips the editor shows faintly behind (piano roll plan
    /// 4.1), each copy shared with the screens until its clip changes
    /// ([`TimeControl::ghosts_follow`]).
    ghost_seen: Vec<Ghost>,
    edit_rev: u64,
    /// What became of the editor's last touch, and how many touches it
    /// has had (plan 0C.2): sent with the clip ([`EditInfo::answer`]).
    edit_answer: EditAnswer,
    /// The scales the editor's Scale option offers, from the host (piano
    /// roll plan 2B.1, [`TimeControl::set_scale_offers`]).
    scale_offers: Arc<ScaleOffers>,
    /// Make a style's sheet, open from the clip card (composing step 6,
    /// board D; styles.rs).
    style_sheet: Option<Box<mpfree_time_ui::style_sheet::StyleSheet>>,
    /// The song's styles made from clips, read from `project.styles`, and
    /// the lines they were read from ([`TimeControl::sync_styles`]).
    styles: Arc<[mpfree_compose::ClipStyle]>,
    style_lines: Vec<String>,
    /// The Song view's Chords lane: each row the song's sections play,
    /// heard in turn while the Song view shows (composing step 6, board
    /// C; `song_chords.rs`).
    song_hear: super::song_chords::SongHearing,
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
    /// The clip of the selected track the Sequencer edits apart from what
    /// plays (`TimeAction::EditClip`, refactor C2): the track's lasting
    /// id, the clip's lasting id (so a scene put in above it or a clip
    /// moved to another row keeps it) and its row when it was picked (for
    /// a clip never given an id). It goes when another track is selected
    /// or the clip is gone: [`TimeControl::clip_picked`].
    clip_pick: Option<(TrackId, ClipId, usize)>,
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
    /// The tracks whose instrument plays notes, a bit each
    /// ([`TimeControl::set_notes_tracks`]).
    notes_tracks: u128,
    /// The note each track's pads play (a pad's root), for the step grid's
    /// pad rows on a clip of notes ([`TimeControl::set_pad_rows`]).
    pad_rows: Box<[PadRows; MAX_TRACKS]>,
    /// Convert to notes' preview, open (E1): Convert checks the clip and
    /// the pads are still what it read.
    convert_asked: Option<ConvertAsked>,
    takes: Takes,
    /// The song edits that moved the audio tracks' regions, by take
    /// number, with each edit made (+ Bar pressed in a row joins one take):
    /// their UNDO and REDO move the regions too (the Arrange refactor's
    /// 3.4, [`TimeControl::act_with_audio`]).
    audio_edits: HashMap<mpfree_transport::TransactionId, Vec<Retime>>,
    /// UNDOs of such an edit whose regions stayed where they were (the
    /// audio tracks' history no longer reached it): a fallback, counted
    /// and said.
    audio_stayed: u32,
    /// Takes on an audio lane over a block that couldn't be cut there
    /// (the song full of blocks), so the whole block went: a fallback,
    /// counted and said ([`TimeControl::audio_took`]).
    audio_took_whole: u32,
    /// The song's regions for the Song → scenes being made, from the audio
    /// tracks ([`SongAudio::regions`]), only while
    /// [`TimeControl::act_with_audio`] makes it.
    song_regions: Vec<(TrackId, Vec<ClipRegion>)>,
    /// What Rec song last put on audio tracks' lanes: its song edit's
    /// number and the spans of the blocks, by lasting id, for the audio
    /// tracks to replace the regions under them
    /// ([`TimeControl::take_song_placed`]).
    song_placed: Option<super::SongPlaced>,
    /// The song time edit being made moves audio regions: it is kept as a
    /// take even where the song itself doesn't change (bars put in after
    /// everything but the audio), so UNDO takes the regions back.
    audio_moves: bool,
    /// The audio regions changed since the last take's step on them: + Bar
    /// pressed again is a take of its own, since one UNDO couldn't take
    /// both presses back from the regions.
    audio_apart: bool,
    /// REDOs done so far: a track that comes back with one is the one UNDO
    /// took away, and its kit comes back with it (`TrackKits::follow`).
    redos: u64,
    /// The clip COPY + a clip copied last, for COPY + an empty slot.
    clipboard: Option<Clip>,
    /// The notes COPY + a note in the note editor copied last, for COPY +
    /// empty roll (piano roll plan 1B.4).
    notes_copy: Option<NoteCopy>,
    /// The take a step lock's turn went into last: a turn that joins it
    /// writes no line, so a knob turned through many values is one line.
    lock_take: u32,
    /// What compiles and step edits cost.
    pub(super) costs: Costs,
    /// CAPTURE's history and the move log, fed the audio half's notes and
    /// gestures with the spans they were played in.
    capture: Box<Capture>,
    move_log: Box<MoveLog>,
    /// The XY pad's moves for the song's lanes this period (offset and
    /// move), for the song recorder once the period's block is in.
    song_moves: Vec<(u32, Input)>,
    /// XY pad moves for the song with no room in one period (counted in
    /// [`TimeControl::dropped`]).
    song_moves_lost: u64,
    /// What CAPTURE would keep, as the screen last showed it.
    pending: Option<Pending>,
    /// The transport as the audio half's last period left it.
    info: TransportInfo,
    /// The audio half's counts, as of its last period.
    counts: Counts,
    /// What following another device's clock put right or lost.
    follow: FollowCounts,
    /// What clips' locks on "this track's strip" did (`strips.rs`).
    pub strips: StripCounts,
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
    /// The tempo the scene launched last sets, on the song tick its line
    /// falls on (the Arrange refactor's 5.2b), for the view; and the ids
    /// given so far.
    scene_tempo: Option<SceneTempo>,
    tempo_ids: u32,
    /// The current period's landed mute cues.
    landed: Vec<MuteLanded>,
    /// Clips' saved states (Save state, sequencer S4), as (track, slot,
    /// the clip as it was): Reload clip puts one back. Kept while the
    /// project is open, not saved with it.
    saved: Vec<(usize, usize, Clip)>,
    /// Reloads waiting for their launch line.
    reloads: Vec<ReloadWait>,
    /// Continue (composing tools): a clip carried on, heard in place of
    /// the clip until Keep or Back ([`super::compose`]).
    compose: Box<Composing>,
    /// What each track plays, as the screen last said, for the note
    /// editor's Continue (`ContinueAction::Parts`).
    compose_parts: Arc<Parts>,
    /// The note editor's Continue tool asked for what is under way: Back
    /// goes once the tool no longer shows that clip.
    compose_from_editor: bool,
    /// Add a part (composing tools S2.6): a part for a row's empty cell,
    /// heard there until Keep or Back ([`super::adding`]).
    add: Box<Adding>,
    /// The rows whose chords changed since their kept clips fitted them,
    /// each with its chords as they were: what marks a clip off the new
    /// chords and what Fit moves it from. Kept while the project is open,
    /// not saved with it.
    fitted: Vec<(usize, mpfree_context::chord::ChordTimeline)>,
    /// The kept clips off their row's chords, as of the last compile.
    unfit: Arc<[mpfree_time_ui::adding::Unfit]>,
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
    /// The free loops' lasting ids (0: not a free loop), as the looper last
    /// said (audio tracks phase 4e.4): a loop cell naming one is a free
    /// cell.
    free_ids: [u128; LOOPS],
    /// The free cells playing, as (track, slot), one a track at most
    /// (phase 4e.4). Not the arranger's: a free loop launches itself on the
    /// free clock ([`TimeControl::free_plan`]), so it plays on through
    /// STOP, locates and the song's loop; a clip starting on its track, a
    /// stop of the track or the cell going ends it.
    free_on: Vec<(usize, usize)>,
    /// The Loop cell armed (`TimeAction::LoopCell`): the next loop
    /// recorded lands there.
    loop_cell: Option<(usize, usize)>,
    /// Loops on audio tracks (Dev's switch, audio tracks phase 4b): an
    /// empty cell of an audio track arms as a Loop track's does, and the
    /// next loop recorded lands there as a loop cell
    /// ([`TimeControl::set_loops_on`]).
    loops_on: bool,
    /// Each loop's lasting id, as the loop host last said
    /// ([`TimeControl::loops_are_on`]; 0: none yet).
    loop_ids: [u128; LOOPS],
    /// The id an audio loop cell's copy gave each loop, until its copy
    /// ends ([`TimeAction::LoopCopyOff`] empties its cell); 0: none.
    copy_ids: [u128; LOOPS],
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
    /// The Continue result the steps show in place of its clip, as
    /// [`super::compose::Composing::heard_key`] names it.
    steps_heard: Option<(usize, usize, u32)>,
    /// Recorded pressure a clip had no room for (`MAX_TOUCH_POINTS`).
    touch_lost: u64,
    /// Recorded moves of an instrument's controls left out of clips whose
    /// instrument locks were made for another instrument (refactor F1).
    inst_refused: u64,
    /// Each track's instrument as last told ([`TimeControl::set_instruments`]),
    /// as the addresses of its node's info and controls and their count:
    /// a preset that brings other controls under the same kind tells
    /// again.
    inst_told: Vec<(usize, usize, usize)>,
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
    /// Cues kept back until their audio is held, while takes stream from
    /// their files (`audio_wait.rs`).
    audio_waits: AudioWaits,
    /// Audio clips whose launch was cancelled, their audio not ready, as
    /// (track, slot, ticks it waited), until launched again.
    not_ready: Vec<(usize, usize, Tick)>,
    /// The audio track armed last, by its lasting id, while no note track
    /// has been armed since: the Clip tab shows its clip (layout spec 4j).
    audio_focus: TrackId,
    /// Each audio track's regions, by its lasting id, as the spans of song
    /// ticks they cover, in order ([`TimeControl::set_audio_spans`]): no
    /// block goes over one (phase 2, step 2d).
    audio_spans: super::AudioSpans,
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
    /// The tracks' pad rows the audio half has (refactor E4).
    pad_rows_sent: Box<[PadRows; MAX_TRACKS]>,
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

/// Whether cell `slot` of track `track` names one of the free loops
/// `ids` (their lasting ids, 0 for none): a free cell (audio tracks phase
/// 4e.4).
fn is_free_cell(p: &Project, ids: &[u128; LOOPS], track: usize, slot: usize) -> bool {
    p.clip(track, slot)
        .and_then(Clip::loop_id)
        .is_some_and(|id| id.0 != 0 && ids.contains(&id.0))
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
        let style_lines = project.styles.clone();
        let styles = Arc::from(mpfree_compose::clip_styles(&style_lines));
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
            scene_open: None,
            selected: None,
            picks: Vec::new(),
            region: None,
            section: None,
            tempo_pick: None,
            report: None,
            project_bpm: info.bpm,
            song_rec: SongRecorder::default(),
            song_take: SongTake::Idle,
            section_now: None,
            section_next: None,
            section_last: 0,
            list_loop: None,
            loop_kept: None,
            taking: false,
            later_note: None,
            throw: false,
            pass_take: None,
            scene_morph: SceneMorph::default(),
            editor: None,
            edit_seen: None,
            ghost_seen: Vec::new(),
            edit_rev: 0,
            edit_answer: EditAnswer::default(),
            scale_offers: Arc::default(),
            style_sheet: None,
            styles,
            style_lines,
            song_hear: super::song_chords::SongHearing::new(),
            armed: None,
            armed_id: TrackId::NONE,
            track_pick: None,
            pick_id: TrackId::NONE,
            clip_pick: None,
            also: 0,
            also_n: 0,
            also_full: 0,
            notes_tracks: 0,
            pad_rows: Box::new([PadRows::PADS; MAX_TRACKS]),
            convert_asked: None,
            takes: Takes::default(),
            audio_edits: HashMap::new(),
            audio_stayed: 0,
            audio_took_whole: 0,
            song_regions: Vec::new(),
            song_placed: None,
            audio_moves: false,
            audio_apart: false,
            redos: 0,
            clipboard: None,
            notes_copy: None,
            lock_take: 0,
            costs: Costs::default(),
            capture: Box::new(Capture::new()),
            move_log: Box::new(MoveLog::new()),
            song_moves: Vec::with_capacity(SONG_MOVES),
            song_moves_lost: 0,
            pending: None,
            info,
            counts: Counts::default(),
            follow: FollowCounts::default(),
            strips: StripCounts::default(),
            clock_waits: false,
            block: Box::new(Block::new()),
            fresh: true,
            spans_lost: 0,
            started: Vec::with_capacity(STARTED_ROOM),
            mute_queue: None,
            mute_ids: 0,
            scene_tempo: None,
            tempo_ids: 0,
            landed: Vec::with_capacity(MAX_TRACKS),
            saved: Vec::new(),
            reloads: Vec::new(),
            compose: Box::new(Composing::new()),
            compose_parts: Arc::default(),
            compose_from_editor: false,
            add: Box::new(Adding::new()),
            fitted: Vec::new(),
            unfit: Arc::from(Vec::new()),
            rate,
            loop_at: [None; LOOPS],
            loops_seen: [LoopNow::default(); LOOPS],
            loops_gone: [None; LOOPS],
            free_ids: [0; LOOPS],
            free_on: Vec::new(),
            loop_cell: None,
            loops_on: false,
            loop_ids: [0; LOOPS],
            copy_ids: [0; LOOPS],
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
            steps_heard: None,
            touch_lost: 0,
            inst_refused: 0,
            inst_told: Vec::new(),
            touch_grew: Vec::new(),
            touch_freed: (0, 0),
            // A project's own timeline, never another's (step 0C).
            includes: Includes {
                timeline: next_timeline(),
                ..Includes::default()
            },
            decisions: Box::default(),
            heard: 0,
            cue_counts: CueCounts::default(),
            audio_waits: AudioWaits::default(),
            not_ready: Vec::new(),
            audio_focus: TrackId::NONE,
            audio_spans: Vec::new(),
            tx,
            rx,
            unsent: None,
            view_sent: 0,
            edit_id: 0,
            tracks_sent: tracks,
            pad_rows_sent: Box::new([PadRows::PADS; MAX_TRACKS]),
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

    /// The tracks whose instrument plays notes, a bit each, from the
    /// host's instruments: a new clip REC makes for a take on one holds
    /// notes (the instrument redesign's step 5c).
    pub fn set_notes_tracks(&mut self, tracks: u128) {
        self.notes_tracks = tracks;
    }

    /// The scales the note editor's Scale option offers (piano roll plan
    /// 2B.1), from the host: the selected track's pads' scale on the Synth
    /// tab, if any, and the owner's own scales (name, notes up, notes
    /// down). Called often; it changes something only when they change.
    pub fn set_scale_offers(
        &mut self,
        pads: Option<mpfree_seq::Scale>,
        own: &[(String, u16, u16)],
    ) {
        if self.scale_offers.pads == pads && self.scale_offers.own == own {
            return;
        }
        self.scale_offers = Arc::new(ScaleOffers {
            pads,
            own: own.to_vec(),
        });
        if let Some(ed) = self.editor.as_mut() {
            ed.offers = Arc::clone(&self.scale_offers);
            self.changed = true;
        }
    }

    /// The notes each of track `track`'s 16 pads plays, from the host's
    /// instruments: on a clip of notes the step grid's pad rows show each
    /// pad's root, and a tap on a pad's row edits the pad's notes (step
    /// 5c). The grid is shown again when they change.
    pub fn set_pad_rows(&mut self, track: usize, rows: PadRows) {
        if let Some(r) = self.pad_rows.get_mut(track) {
            if *r != rows {
                *r = rows;
                self.steps_changed = true;
            }
        }
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
        let was = self.selected();
        self.armed = track.filter(|&t| t < self.project.tracks.len());
        self.armed_id = self.id_of(self.armed);
        self.track_pick = None;
        self.pick_id = TrackId::NONE;
        self.selection_moved(was);
    }

    /// Select `track` apart from the arm, by its lasting id; the armed
    /// track (or no such track) selects the armed one again.
    fn set_pick(&mut self, track: usize) {
        let was = self.selected();
        let pick = Some(track).filter(|&t| t < self.project.tracks.len() && Some(t) != self.armed);
        self.track_pick = pick;
        self.pick_id = self.id_of(pick);
        self.selection_moved(was);
    }

    /// Another track selected than `was`: the Sequencer edits what plays
    /// on it, not a clip picked on the one before (refactor C2).
    fn selection_moved(&mut self, was: Option<usize>) {
        if self.selected() != was {
            self.clip_pick = None;
        }
    }

    /// The row of the selected track's clip picked for the Sequencer to
    /// edit apart from what plays (refactor C2), where that clip is now,
    /// while that track is still selected and has the clip. Found by the
    /// clip's lasting id; a clip never given one, by its row.
    pub fn clip_picked(&self) -> Option<usize> {
        let track = self.selected()?;
        let t = self.project.tracks.get(track)?;
        let (on, clip, row) = self.clip_pick?;
        if on != t.id {
            return None;
        }
        if clip.is_given() {
            t.slots.row_of(clip)
        } else {
            self.project.clip(track, row).is_some().then_some(row)
        }
    }

    /// The lasting id of track `track`, if there is one.
    fn id_of(&self, track: Option<usize>) -> TrackId {
        track
            .and_then(|t| self.project.tracks.get(t))
            .map_or(TrackId::NONE, |t| t.id)
    }

    /// `TimeAction::StepSize`: the step size of the clip the Sequencer
    /// edits (refactor D1), a clip setting UNDO takes back. The clip keeps
    /// its length and every note its time: only the grid cuts it into
    /// steps of another size (13 sixteenths show as 7 eighths, the last
    /// one short), so nothing compiles and nothing sounds different.
    fn step_size(&mut self, size: Tick) -> Result<String, String> {
        let name =
            size_name(size).ok_or_else(|| format!("steps: {size} ticks is not a step size"))?;
        let (track, slot) = self.step_focus();
        let t = self
            .project
            .tracks
            .get(track)
            .ok_or("steps: no such track")?;
        if t.is_loops() {
            return Err(format!("steps: {} plays loops, it has no steps", t.name));
        }
        let slot =
            slot.ok_or_else(|| format!("steps: {} has no clip yet: tap a step first", t.name))?;
        let c = self
            .project
            .clip(track, slot)
            .ok_or("steps: the clip is gone")?;
        let clip = c.name.clone();
        let settings = mpfree_seq::ClipSettings {
            step: size,
            ..c.settings()
        };
        self.takes
            .set_clip(&mut self.project, track, slot, settings, None)
            .map_err(|e| format!("steps: {clip}: {e:?}"))?;
        let steps = self
            .project
            .clip(track, slot)
            .map_or(0, |c| grid_of(c).steps(c.length));
        self.changed = true;
        self.steps_changed = true;
        Ok(format!("steps: {clip} in {name} steps ({steps})"))
    }

    /// `TimeAction::EditClip`: the Sequencer edits slot `slot`'s clip of
    /// the selected track without launching it, or (None) the clip that
    /// plays again. Nothing plays or stops: what plays keeps playing.
    fn edit_clip(&mut self, slot: Option<usize>) -> Result<String, String> {
        let track = self.selected().ok_or("edit: no track is selected")?;
        let t = self
            .project
            .tracks
            .get(track)
            .ok_or("edit: no such track")?;
        if t.is_loops() {
            return Err(format!("edit: {} plays loops, it has no steps", t.name));
        }
        if t.is_audio() {
            return Err(format!(
                "edit: {} is an audio track, it has no steps",
                t.name
            ));
        }
        let (id, name) = (t.id, t.name.clone());
        let playing = self
            .launches
            .iter()
            .find(|l| l.track == track)
            .and_then(|l| Some((l.slot, &self.project.clip(track, l.slot)?.name)));
        let line = match slot {
            Some(s) => {
                let clip = self
                    .project
                    .clip(track, s)
                    .ok_or_else(|| format!("edit: slot {} of {name} is empty", s + 1))?;
                let line = match playing {
                    Some((p, other)) if p != s => {
                        format!("editing {} on {name} ({other} plays)", clip.name)
                    }
                    _ => format!("editing {} on {name}", clip.name),
                };
                self.clip_pick = Some((id, clip.id, s));
                line
            }
            None => {
                self.clip_pick = None;
                match playing {
                    Some((_, clip)) => format!("editing what plays on {name}: {clip}"),
                    None => format!("editing what plays on {name}"),
                }
            }
        };
        self.changed = true;
        self.steps_changed = true;
        Ok(line)
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
            let armed = found.or_else(|| project.tracks.iter().rposition(|t| t.plays_notes()));
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
        // A clip picked to edit that is gone (or whose track is no longer
        // the selected one): the Sequencer edits what plays.
        if self.clip_picked().is_none() {
            self.clip_pick = None;
        }
    }

    /// The notes the step grid's pad rows stand for on `clip` of `track`:
    /// the pads, 0 to 15, on a clip of pad hits; the track's pads' notes
    /// on a clip of notes.
    fn rows_of(&self, track: usize, clip: &Clip) -> PadRows {
        match clip.holds {
            mpfree_seq::Holds::Hits => PadRows::PADS,
            mpfree_seq::Holds::Notes => self.pad_rows.get(track).copied().unwrap_or(PadRows::PADS),
        }
    }

    /// What a new clip for a take on `track` holds: notes when its
    /// instrument plays notes (its clips through its arp too, since step
    /// 5d), else pad hits.
    fn take_holds(&self, track: usize) -> mpfree_seq::Holds {
        if self.notes_tracks >> track.min(127) & 1 == 1 {
            mpfree_seq::Holds::Notes
        } else {
            mpfree_seq::Holds::Hits
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

    /// The note editor is open with Pads write on (piano roll plan 1C.2):
    /// the pads write at its cursor.
    pub fn pads_write(&self) -> bool {
        self.editing() && self.editor.as_ref().is_some_and(|ed| ed.pads_write)
    }

    /// The Loop cell armed on the grid while it is still an empty cell of
    /// a Loop track (`TimeAction::LoopCell`): the next loop recorded
    /// lands there.
    pub fn loop_cell(&self) -> Option<(usize, usize)> {
        self.loop_cell.filter(|&(t, s)| {
            self.project.tracks.get(t).is_some_and(|tr| {
                takes_loops(tr, self.loops_on) && tr.slots.get(s).is_some_and(|c| c.is_none())
            })
        })
    }

    /// Loops on audio tracks (Dev's switch, phase 4b), for this start: an
    /// audio track's empty cell arms for the next loop recorded
    /// (`TimeAction::LoopCell`), which lands there as a loop cell naming
    /// the loop's lasting id ([`TimeControl::loops_are_on`]).
    pub fn set_loops_on(&mut self, on: bool) {
        self.loops_on = on;
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
        for (t, (rows, was)) in self
            .pad_rows
            .iter()
            .zip(self.pad_rows_sent.iter_mut())
            .enumerate()
        {
            if rows != was && self.tx.send(ToAudio::Pads(t, *rows)).is_ok() {
                *was = *rows;
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
        if !t.plays_notes() {
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
                ToControl::Played(offset, event, took) => {
                    self.capture.input_as(offset, event, took);
                }
                ToControl::PlayedOn(track, offset, event, took) => {
                    self.capture.input_on(track, offset, event, took);
                }
                ToControl::Moved(offset, input) => self.move_log.input(offset, input),
                ToControl::SongMoved(offset, input) => {
                    if self.song_moves.len() < SONG_MOVES {
                        self.song_moves.push((offset, input));
                    } else {
                        self.song_moves_lost += 1;
                    }
                }
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
                    // A move of the instrument's control into a clip made
                    // for another instrument stays out (refactor F1): the
                    // next compile drops it from the view too.
                    if self.rec_refused(&rec) {
                        self.inst_refused += 1;
                        self.touch.clear();
                        self.moves.clear();
                        self.compile_asked = true;
                        continue;
                    }
                    let done = self
                        .takes
                        .record(&mut self.project, &rec, &self.touch, &self.moves);
                    if done.changed && rec_inst(&rec) {
                        self.tag_clips(Some(rec.clip().0));
                    }
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
                ToControl::Strips(c) => self.strips = c,
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
        // The XY pad's moves, on the song tick of their frame.
        for (offset, input) in self.song_moves.drain(..) {
            let at = self
                .block
                .spans()
                .iter()
                .filter(|s| s.state.rolling())
                .find(|s| offset < s.offset + s.len)
                .map(|s| {
                    let frame = s.frame + u64::from(offset.saturating_sub(s.offset));
                    s.tick_at(frame).floor() as Tick
                });
            if let Some(at) = at {
                self.song_rec.moved(at, input);
            }
        }
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
            // A clip starting on a free cell's track, or its stop, ends the
            // free cell there (phase 4e.4).
            self.free_on.retain(|f| f.0 != st.track);
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
        // The tracks' pads in the queue land in the period before their
        // line: the view compiled now plays the period that holds it.
        compile |= self.pads_due(end.frames);
        // Reloads land just before their line: the next period, which
        // holds the line, plays the saved state from its start.
        if !self.reloads.is_empty() {
            compile |= self.reloads_due(end.frames);
        }
        // Continue: the composer's answers, a recording that changed what
        // it listened to, and a result coming in or going on its line.
        compile |= self.continue_period(end.frames);
        // Add a part: the same, for a try in its empty cell.
        compile |= self.add_period(end.frames);
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
            // gave no cue wait in the mute queue for its line, and its
            // tempo for its tick.
            compile |= self.queue_scene_mutes();
            compile |= self.scene_tempo_on_its_line();
        }
        // Cues waiting for their audio whose line played without them move
        // on, or are cancelled.
        compile |= self.audio_waits_due(end.frames);
        // A linked song's sections bring their scenes' mutes and program
        // changes (4b.4).
        compile |= self.sections_due(self.info.meter.bar_ticks());
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

    /// Notes this half lost: past CAPTURE's input table, and the XY pad's
    /// moves for the song past a period's room. What passing between the
    /// halves lost is in [`TimeControl::describe`].
    pub fn dropped(&self) -> u64 {
        self.capture.dropped() + self.song_moves_lost
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

    /// Recorded moves of an instrument's controls left out of clips made
    /// for another instrument (refactor F1). The status line says it;
    /// tests read it here.
    #[allow(dead_code)]
    pub fn inst_refused(&self) -> u64 {
        self.inst_refused
    }

    /// The instrument each track plays now, by place (refactor F1),
    /// told every period: a track's that changed is kept on the track
    /// ([`mpfree_seq::Track::plays`]), its clips' tags are made true
    /// ([`mpfree_seq::Clip::tag_inst`]), and what plays compiles again if
    /// one of its clips there has instrument locks. Allocates only when
    /// one changes.
    pub fn set_instruments(&mut self, plays: &[Option<InstRef>]) {
        let mut compile = false;
        if self.inst_told.len() < plays.len() {
            self.inst_told.resize(plays.len(), (0, 0, 0));
        }
        for (t, now) in plays.iter().enumerate() {
            let Some(track) = self.project.tracks.get_mut(t) else {
                break;
            };
            let told = now.map_or((0, 0, 0), |(info, params)| {
                (
                    std::ptr::from_ref(info) as usize,
                    params.as_ptr() as usize,
                    params.len(),
                )
            });
            // A project loaded since has its tracks not told of.
            if self.inst_told[t] == told && track.plays.node() == now.map(|(i, _)| i.id) {
                continue;
            }
            self.inst_told[t] = told;
            track.plays = mpfree_seq::inst::Plays(now.map(|(info, params)| {
                std::sync::Arc::new(mpfree_seq::inst::Inst {
                    node: info.id.to_string(),
                    name: info.name.to_string(),
                    params: params
                        .iter()
                        .filter(|p| !p.readonly)
                        .map(|p| (p.id, p.name.to_string()))
                        .collect(),
                })
            }));
            self.tag_clips(Some(t));
            self.steps_changed = true;
            self.changed = true;
            compile |= self.inst_shown(t);
        }
        if compile {
            let _ = self.compile();
        }
    }

    /// Whether a clip of track `t` that plays or waits to has instrument
    /// locks (or move lanes), so what plays changes with its instrument.
    fn inst_shown(&self, t: usize) -> bool {
        let has = |slot: usize| {
            self.project
                .clip(t, slot)
                .is_some_and(|c| c.inst_params().next().is_some())
        };
        self.launches.iter().any(|l| l.track == t && has(l.slot))
            || self
                .cues
                .iter()
                .any(|c| c.track == t && c.slot.is_some_and(has))
            || self.song
    }

    /// Make the instrument tags true ([`mpfree_seq::Clip::tag_inst`]) on
    /// track `track`'s clips, or every track's: after a change that may
    /// have added or taken instrument locks (an edit, a take, UNDO). The
    /// Steps hear of a change.
    fn tag_clips(&mut self, track: Option<usize>) {
        let mut changed = false;
        for (t, tr) in self.project.tracks.iter_mut().enumerate() {
            if track.is_some_and(|k| k != t) {
                continue;
            }
            let plays = tr.plays.0.clone();
            for (_, c) in tr.slots.iter_mut() {
                changed |= c.tag_inst(plays.as_deref());
            }
        }
        self.steps_changed |= changed;
    }

    /// Whether recorded change `rec` would put a move of the instrument's
    /// control into a clip made for another instrument (refactor F1).
    fn rec_refused(&self, rec: &mpfree_play::Rec) -> bool {
        let Some(param) = rec_param(rec) else {
            return false;
        };
        let (t, slot) = rec.clip();
        let plays = self.project.tracks.get(t).and_then(|t| t.plays.get());
        self.project
            .clip(t, slot)
            .is_some_and(|c| c.inst_refuses(param, plays).is_some())
    }

    /// Out of recording, each clip played into (or given new pressure
    /// runs by an edit: a chop, pressure drawn) whose pressure is past
    /// half its room frees the values no note can play again, its own,
    /// its Restore's or one an undo or redo would bring back
    /// (`Takes::reclaim_touch`, refactor B4b), so long sessions of
    /// recording, drawing and undoing never fill it. The note editor on
    /// that clip keeps its picks: its notes stay where they were. Never
    /// while recording: a replace pass names the notes it erases by
    /// value, pressure run and all, as the playing view holds them. True
    /// when anything was freed: the view is compiled again with the new
    /// numbering.
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
            let editing = self
                .editor
                .as_ref()
                .is_some_and(|e| (e.track, e.slot) == (track, slot));
            if editing {
                // Whatever else changed the clip is followed first.
                self.edit_follow();
            }
            let n = self.takes.reclaim_touch(&mut self.project, track, slot);
            if n > 0 {
                self.touch_freed.0 += n as u64;
                self.touch_freed.1 += 1;
                freed = true;
                if editing {
                    // Its notes name renumbered runs: the picks stay, and
                    // the picture moves on (as a paste's retry does).
                    self.edit_seen = self.project.clip(track, slot).cloned().map(Arc::new);
                    self.edit_rev += 1;
                    self.changed = true;
                }
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
        // Free cells launch themselves (phase 4e.4): never the arranger's.
        self.free_cells_follow();
        // An audio clip not ready before, launched again.
        if !self.not_ready.is_empty() {
            let (cues, launches) = (&self.cues, &self.launches);
            self.not_ready.retain(|n| {
                !cues.iter().any(|c| (c.track, c.slot) == (n.0, Some(n.1)))
                    && !launches.iter().any(|l| (l.track, l.slot) == (n.0, n.1))
            });
        }
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
        // Cues decided since the last compile that launch audio wait for
        // it, with streaming on; cues gone leave their waits.
        let cx = wait_ctx!(self);
        self.audio_waits.decided(&cx);
        self.find_arm_and_pick();
        // Tracks added or taken away: the other armed tracks let go.
        if self.also != 0 && self.also_n != n {
            self.also = 0;
        }
        // Continue: a result whose clip, or a clip it listened to, changed
        // stops here, and the clip plays as it now is.
        if self.compose.active() && self.compose.follow(&self.project) {
            self.compose.in_view = None;
            self.compose.line = None;
        }
        // Add a part: a try whose row's clips, or cell, changed goes.
        if self.add.active() && self.add.follow(&self.project) {
            self.add.in_view = None;
            self.add.line = None;
        }
        self.refresh_unfit();
        self.hear_row();
        self.hear_song();
        self.continue_anchor();
        let mut v = self.view_with_add()?;
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

    /// The audio track armed last by its own arm, while it stays armed
    /// and no note track has been armed since ([`TimeControl::focus_audio`]):
    /// the one the Loop group's "Armed" half reaches (audio tracks phase
    /// 4e.1), where an armed audio track is meant. Note tracks are armed
    /// with [`TimeControl::armed`]; an audio track never is.
    pub fn audio_armed(&self) -> Option<usize> {
        let id = self.audio_focus;
        id.is_given()
            .then(|| {
                self.project
                    .tracks
                    .iter()
                    .position(|t| t.id == id && t.is_audio())
            })
            .flatten()
    }

    /// Audio track `track` armed (`on`) by its own arm, or disarmed: the
    /// Clip tab shows its clip until a note track is armed (layout spec
    /// 4j); disarmed, the tab goes back to the armed note track.
    pub fn focus_audio(&mut self, track: usize, on: bool) {
        let id = self
            .project
            .tracks
            .get(track)
            .filter(|t| t.is_audio())
            .map_or(TrackId::NONE, |t| t.id);
        let was = self.audio_focus;
        if on {
            self.audio_focus = id;
        } else if self.audio_focus == id {
            self.audio_focus = TrackId::NONE;
        }
        self.changed |= was != self.audio_focus;
    }

    /// Each audio track's regions, as the spans of song ticks they cover,
    /// in order, by the track's lasting id (phase 2, step 2d): the audio
    /// tracks host hands them over whenever they change, so a block
    /// dropped, moved or made longer over a region is refused.
    pub fn set_audio_spans(&mut self, spans: super::AudioSpans) {
        self.audio_spans = spans;
    }

    /// To a cell and Put in a cell (audio tracks phase 2): `clip`, an
    /// audio clip, goes in audio track `track`'s first empty cell, top
    /// down, as an edit UNDO takes away again. The row it went in, or why
    /// not: no such audio track, no empty cell, or the clip's own
    /// refusal. Nothing changes on an error.
    pub fn put_audio_clip(&mut self, track: usize, clip: Clip) -> Result<usize, String> {
        let t = self
            .project
            .tracks
            .get(track)
            .filter(|t| t.is_audio())
            .ok_or_else(|| format!("there is no audio track {}", track + 1))?;
        if clip.audio.is_none() {
            return Err(format!("{} takes audio clips only", t.name));
        }
        let slot = (0..ROWS)
            .find(|&s| t.slots.get(s).is_some_and(Option::is_none))
            .ok_or_else(|| format!("{} has no empty cell", t.name))?;
        self.takes
            .new_clip(&mut self.project, track, slot, clip, false)
            .map_err(|e| format!("the clip: {e:?}"))?;
        if let Err(e) = self.compile() {
            self.takes.undo(&mut self.project);
            self.takes.forget_undone();
            return Err(e);
        }
        Ok(slot)
    }

    /// Fit to tempo's end on an audio clip's card (audio tracks phase 2):
    /// clip `slot` of `track`, if it is still clip `id`, plays `audio`
    /// (its regions on fitted copies of its recordings, made at the
    /// project's tempo), as one edit UNDO takes back. Its name, or why
    /// not: the cell changed meanwhile, or the clip's own refusal. Nothing
    /// changes on an error.
    pub fn fit_clip(
        &mut self,
        track: usize,
        slot: usize,
        id: mpfree_seq::ClipId,
        audio: mpfree_seq::AudioClip,
    ) -> Result<String, String> {
        let c = self
            .project
            .clip(track, slot)
            .filter(|c| c.id == id && c.audio.is_some())
            .ok_or_else(|| "fit: the clip changed while it fitted".to_string())?;
        let mut clip = c.clone();
        clip.audio = Some(audio);
        let name = clip.name.clone();
        self.replace(track, slot, clip, Replaced::Fit)?;
        Ok(name)
    }

    /// An audio clip's card (layout spec 4j): clip `slot` of `track` with
    /// another length, gain or name, as one take (a row turned on joins
    /// it). A length cuts the regions past it, the one across it ending
    /// there (its frames past the end kept in its `left`, so a longer
    /// length brings them back), and lengthens the last region back into
    /// what it was cut from. The line for the hint, or why not.
    fn set_audio_clip(
        &mut self,
        track: usize,
        slot: usize,
        row: u8,
        set: AudioSet,
    ) -> Result<String, String> {
        let c = self
            .project
            .clip(track, slot)
            .filter(|c| c.audio.is_some())
            .ok_or_else(|| {
                format!(
                    "clip: cell {} of track {} holds no audio clip",
                    slot + 1,
                    track + 1
                )
            })?;
        let mut clip = c.clone();
        let join = match set {
            AudioSet::Name(_) => None,
            _ => Some(AUDIO_CARD_JOIN + u64::from(row)),
        };
        let rate = self.rate;
        let once = clip.settings().once;
        let from = clip.length;
        let a = clip.audio.as_mut().expect("checked");
        match set {
            AudioSet::Length(length) => {
                if once {
                    return Err(format!(
                        "clip: {} plays once, to the end of its sound: its length is its sound's",
                        clip.name
                    ));
                }
                if !(1..=mpfree_seq::MAX_CLIP_TICKS).contains(&length) {
                    return Err(format!("clip: {}: no clip is that long", clip.name));
                }
                cut_audio(a, from, length, rate);
                clip.length = length;
            }
            AudioSet::Gain(g) => {
                if !(0.0..=mpfree_seq::audio::MAX_GAIN).contains(&g) || !g.is_finite() {
                    return Err(format!("clip: {}: that gain is out of range", clip.name));
                }
                a.gain = g;
            }
            AudioSet::Name(name) => {
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err("clip: a clip needs a name".into());
                }
                clip.name = name;
            }
        }
        let name = clip.name.clone();
        self.takes
            .set_audio_clip(&mut self.project, track, slot, clip, join)
            .map_err(|_| format!("clip: {name}: that can't be set"))?;
        if let Err(e) = self.compile() {
            self.takes.undo(&mut self.project);
            self.takes.forget_undone();
            return Err(e);
        }
        let said = self
            .project
            .clip(track, slot)
            .map(|c| {
                let meter = self.info.meter;
                match usize::from(row) {
                    0 => format!("Length {}", bars_beats(c.length, meter)),
                    AUDIO_GAIN => format!(
                        "Gain {}",
                        gain_words(c.audio.as_ref().map_or(1.0, |a| a.gain))
                    ),
                    _ => "named".to_string(),
                }
            })
            .unwrap_or_default();
        Ok(format!("clip: {name} · {said} (UNDO takes it back)"))
    }

    /// Cues that launch audio wait until the app has held the start of
    /// their audio in memory (`on`: takes stream from their files), or go
    /// as they are (`audio_wait.rs`). Turned off, every cue waiting goes
    /// into the next view as it is. The offline copy never turns it on.
    pub fn set_audio_waits(&mut self, on: bool) {
        if self.audio_waits.set_on(on) {
            let _ = self.compile();
        }
    }

    /// What the cues waiting for their audio need held, in the order they
    /// wait. A key stays listed until its cues land or are cancelled, held
    /// or not; [`TimeState::audio_waits`]'s `needs_rev` rises whenever the
    /// list changes.
    pub fn audio_needs(&self) -> &[AudioNeed] {
        self.audio_waits.needs()
    }

    /// The app holds the audio `key`'s cues need (`ok`): they go into the
    /// view at once and land on their line (a hold in its last few ms
    /// lands late, in phase). Not `ok`: not held (yet), so they stay out,
    /// or come out again, and move on when their line plays without them.
    /// False when nothing waits under `key`.
    pub fn audio_held(&mut self, key: CueKey, ok: bool) -> bool {
        match self.audio_waits.held(key, ok) {
            None => false,
            Some(compile) => {
                if compile {
                    let _ = self.compile();
                }
                true
            }
        }
    }

    /// The app gave back the audio `key` held before its cues landed: they
    /// come out of the view and move to their next line, or are cancelled
    /// after their last move. At Now they wait again. False when nothing
    /// held waits under `key`.
    pub fn audio_given_back(&mut self, key: CueKey) -> bool {
        let cx = wait_ctx!(self);
        let Some((gone, said)) = self.audio_waits.given_back(&cx, key) else {
            return false;
        };
        if let Some(w) = gone {
            self.cancel_wait(w);
        }
        self.note_later(said);
        let _ = self.compile();
        true
    }

    /// After each period: waiting cues whose line played without them move
    /// to the next, or are cancelled. True when it must compile.
    fn audio_waits_due(&mut self, frames: u32) -> bool {
        let period = self.period_ticks(frames);
        let cx = wait_ctx!(self);
        let mut compile = false;
        for (gone, said) in self.audio_waits.period(&cx, period) {
            if let Some(w) = gone {
                self.cancel_wait(w);
                compile = true;
            }
            self.note_later(said);
        }
        compile
    }

    /// Cues that waited for their audio cancelled: they leave the cue list
    /// with their scene's program changes, the mutes they queued and the
    /// scene's cue.
    fn cancel_wait(&mut self, w: Wait) {
        // Their cells say "not ready" until launched again.
        let waited = w.waited(self.info.meter.bar_ticks().max(1));
        for c in &w.cues {
            if let Some(slot) = c.slot {
                self.not_ready.retain(|n| (n.0, n.1) != (c.track, slot));
                self.not_ready.push((c.track, slot, waited));
            }
        }
        for c in &w.cues {
            if let Some(i) = self.cues.iter().position(|x| x == c) {
                self.cues.remove(i);
            }
            self.perform.uncue(c.track);
        }
        let mut p = w.programs;
        while p != 0 {
            self.perform.uncue(p.trailing_zeros() as usize);
            p &= p - 1;
        }
        if let Some((_, q)) = &mut self.mute_queue {
            q.want.retain(|x| !w.mutes.contains(&x.0));
        }
        if self.mute_queue.as_ref().is_some_and(|(_, q)| q.is_empty()) {
            self.mute_queue = None;
        }
        if w.scene.is_some() && self.perform.cued() == w.scene {
            self.perform.forget_cued();
        }
        self.changed = true;
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
        // Locks made for another instrument are not in the view, so what
        // an edit does to them compiles (refactor F1).
        let plays = self.project.tracks.get(track).and_then(|t| t.plays.get());
        let silent = self
            .project
            .clip(track, slot)
            .is_some_and(|c| c.inst_silent(plays).is_some());
        // While Continue is under way the edit compiles, so its result
        // is checked against the clip ([`TimeControl::compile`]) and never
        // edited in place in the view's copy of it.
        if silent
            || !InPlace::fits(edit, notes, locks)
            || self.compose.active()
            || self.add.active()
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
        // A free cell playing is its track's clip (phase 4e.4), so the
        // track leaves the song as for any launch; the track player mixes
        // its loop as the free clock plays it, never the runs this gives.
        let with_free;
        let launches = if self.free_on.is_empty() {
            &self.launches
        } else {
            with_free = self
                .launches
                .iter()
                .copied()
                .chain(self.free_on.iter().map(|&(track, slot)| Launch {
                    track,
                    slot,
                    origin: 0,
                    start: 0,
                }))
                .collect::<Vec<_>>();
            &with_free
        };
        let mut v = PlayView::compile_cued(&self.project, launches, &self.cues)
            .map_err(|e| format!("clips: {e:?}"))?;
        v.set_armed(self.armed);
        v.set_also(self.also);
        v.set_song(&self.project, self.song)
            .map_err(|e| format!("song: {e:?}"))?;
        v.set_tempo_map(self.song_tempo()?);
        v.set_scene_tempo(self.scene_tempo);
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
        // Cues waiting for their audio stay out, with their scene's
        // program changes and the mute queue they joined.
        self.audio_waits.hold_back(&mut v);
        // The mute queue: every track it changes, and every track a solo
        // in it silences or lets be heard, lands on the queue's line.
        if let Some((id, q)) = self
            .mute_queue
            .as_ref()
            .filter(|_| !self.audio_waits.holds_queue())
        {
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
        let length = self.clip_loop().map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
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
            // On a track whose instrument plays notes it holds notes, as
            // the armed track's does (refactor E4).
            let mut clip = Clip::new(&format!("Clip {}", s + 1), length);
            clip.holds = self.take_holds(track);
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
                let length = self.clip_loop().map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t
                    .slots
                    .first_free()
                    .ok_or_else(|| format!("{} has no empty slot to record into", t.name))?;
                let holds = self.take_holds(track);
                let mut clip = Clip::new(&format!("Clip {}", s + 1), length);
                clip.holds = holds;
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
        // A row moved or copied: the scene playing and waiting go with
        // their rows.
        if matches!(drop, ClipDrop::RowMoved { .. } | ClipDrop::RowCopied { .. }) {
            self.perform.rows_moved(|r| to((0, r)).map(|c| c.1));
        }
        self.changed = true;
    }

    /// Rows renumbered by a row deleted, or its undo or redo: what plays
    /// and waits, the scene playing and waiting and the scene editor's
    /// row go with their rows, `to` giving each row's new place; a row it
    /// gives None for stops.
    fn rows_followed(&mut self, to: impl Fn(usize) -> Option<usize>) {
        self.launches.retain_mut(|l| match to(l.slot) {
            Some(s) => {
                l.slot = s;
                true
            }
            None => false,
        });
        self.cues.retain_mut(|c| match c.slot {
            Some(s) => match to(s) {
                Some(s2) => {
                    c.slot = Some(s2);
                    true
                }
                None => false,
            },
            None => true,
        });
        self.perform.rows_moved(&to);
        self.scene_pick = self.scene_pick.and_then(&to);
        self.changed = true;
    }

    /// The takes by their transactions ([`crate::time::TimeHost::take_steps`]).
    pub fn take_steps(
        &self,
        done: &mut Vec<TransactionId>,
        redo: &mut Vec<TransactionId>,
    ) -> TransactionId {
        done.extend(self.takes.done().iter().map(|t| t.txn));
        redo.extend(self.takes.undone().iter().rev().map(|t| t.txn));
        self.takes.let_go()
    }

    /// The one UNDO across the engines (audio tracks phase 4d.3): forget
    /// the takes REDO could bring back (`redo`), else let go of those UNDO
    /// could take back made at or before `through`
    /// ([`mpfree_play::Takes::forget_redo`],
    /// [`mpfree_play::Takes::retire_through`]). True if any went.
    pub fn forget_takes(&mut self, redo: bool, through: TransactionId) -> bool {
        let any = if redo {
            self.takes.forget_redo()
        } else {
            self.takes.retire_through(through) > 0
        };
        self.changed |= any;
        any
    }

    /// What take `txn` is, as UNDO says it
    /// ([`crate::time::TimeHost::take_words`]).
    pub fn take_words(&self, txn: TransactionId) -> Option<String> {
        let t = self
            .takes
            .done()
            .iter()
            .chain(self.takes.undone())
            .find(|t| t.txn == txn)?;
        Some(self.take_said(Some(TakeRow::of(t))))
    }

    fn take_said(&self, row: Option<TakeRow>) -> String {
        let Some(r) = row else {
            return "take".into();
        };
        if let Some((word, row)) = r.row_tool {
            return format!("{} · row {}", word, row + 1);
        }
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
                | SongChange::Unique { track, .. }
                | SongChange::Recorded { track, .. }
                | SongChange::Split {
                    track: Some(track), ..
                }
                | SongChange::Trimmed { track, .. }
                | SongChange::Slid { track, .. },
            ) => return format!("{} · {} in the song", r.short(), track_name(track)),
            Some(SongChange::Split { track: None, at }) => {
                return format!(
                    "{} · the song split at {}",
                    r.short(),
                    bar_beat(at, self.info.meter)
                )
            }
            Some(SongChange::SectionMoved { .. } | SongChange::SectionCopied { .. }) => {
                return format!("{} · a section of the song", r.short())
            }
            Some(
                SongChange::SeveralMoved { blocks, .. }
                | SongChange::SeveralCopied { blocks, .. }
                | SongChange::SeveralDeleted { blocks }
                | SongChange::SeveralSplit { blocks, .. },
            ) => return format!("{} · {blocks} blocks in the song", r.short()),
            Some(SongChange::Scenes { .. }) => return format!("{} → song", r.short()),
            Some(SongChange::RecSong { .. }) => return format!("{} · rec song", r.short()),
            Some(SongChange::List) => return format!("{} · the song list", r.short()),
            Some(SongChange::ListToSong { .. }) => return format!("{} · list → song", r.short()),
            Some(SongChange::Tempo { at }) => {
                return format!(
                    "{} · the song's tempo at {}",
                    r.short(),
                    bar_beat(at, self.info.meter)
                )
            }
            Some(SongChange::Bars { at, .. }) => {
                return format!(
                    "{} · the song's bars at {}",
                    r.short(),
                    bar_beat(at, self.info.meter)
                )
            }
            Some(SongChange::Marker { at }) => {
                return format!(
                    "{} · the song's marker at {}",
                    r.short(),
                    bar_beat(at, self.info.meter)
                )
            }
            Some(SongChange::Followed { row }) => {
                return format!("{} · the song's parts from row {}", r.short(), row + 1)
            }
            Some(SongChange::RowDeleted { row }) => {
                return format!("{} · row {}", r.short(), row + 1)
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

    /// Track `track`'s clip the Play page's pattern would show: on the
    /// selected track the clip picked to edit (refactor C2), else its
    /// launched clip, else its first.
    fn focus_of(&self, track: usize) -> (usize, Option<usize>) {
        if let Some(slot) = self
            .clip_picked()
            .filter(|_| self.selected() == Some(track))
        {
            return (track, Some(slot));
        }
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
        let clip = slot.and_then(|slot| {
            // While Continue's result for the clip is shown: the clip as
            // heard with it (composing step 4).
            let heard = self.compose.heard().filter(|h| (h.0, h.1) == (track, slot));
            let c = match heard {
                Some(h) => h.2,
                None => self.project.clip(track, slot)?,
            };
            let grid = grid_of(c);
            let new_from = heard.map(|h| (h.3 / grid.size().max(1)) as usize);
            let launch = self
                .launches
                .iter()
                .find(|l| l.track == track && l.slot == slot);
            let origin = launch.map(|l| l.origin);
            let order =
                launch.and_then(|l| mpfree_play::StepOrder::of(c, track, slot, l.origin, l.start));
            // A clip of notes shows its pads' notes on the pad rows, named
            // by the pad, so the Play page and the step pads read every clip
            // by pad (step 5c).
            let pads = self.rows_of(track, c).roots();
            let shown = grid.shown(c, &pads);
            let pad_of = |note: u8| -> u8 {
                match pads.iter().position(|&p| p == note) {
                    Some(i) if c.holds == mpfree_seq::Holds::Notes && note != LINE => i as u8,
                    _ => note,
                }
            };
            let plays = self.project.tracks.get(track).and_then(|t| t.plays.get());
            let silent = c.inst_silent(plays).map(|made| {
                let mut params: Vec<u16> = c
                    .inst_params()
                    .filter(|&id| !c.param_plays(id, plays))
                    .collect();
                params.sort_unstable();
                params.dedup();
                SilentLocks {
                    made_for: made.clone(),
                    params,
                    now: plays.map_or_else(String::new, |p| p.name.clone()),
                }
            });
            Some(StepClip {
                slot,
                name: c.name.clone(),
                steps: grid.steps(c.length),
                step: grid.size(),
                song_length: c.speed.to_song(c.length as f64),
                song_step: c.speed.to_song(grid.size() as f64),
                origin,
                order,
                rows: shown
                    .rows
                    .into_iter()
                    .map(|(p, r)| (pad_of(p), r))
                    .collect(),
                locks: shown
                    .locks
                    .into_iter()
                    .map(|(p, r)| (pad_of(p), r))
                    .collect(),
                lock_values: shown
                    .lock_values
                    .into_iter()
                    .map(|(p, i, a, v)| (pad_of(p), i, a, v))
                    .collect(),
                silent,
                new_from,
            })
        });
        let playing = self
            .launches
            .iter()
            .find(|l| l.track == track && Some(l.slot) != slot)
            .and_then(|l| Some(self.project.clip(track, l.slot)?.name.clone()));
        StepsData {
            track,
            track_name,
            clip,
            picked: slot.is_some() && slot == self.clip_picked(),
            playing,
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
        self.step_action_in(a, false)
    }

    /// [`Self::step_action`], with `sixteenths` cutting the clip into 1/16
    /// steps whatever its step size (refactor D1): the clip card's Length
    /// row counts sixteenths.
    fn step_action_in(&mut self, a: StepAction, sixteenths: bool) -> Result<String, String> {
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
                // Not in the slot yet: it goes in with its first notes. On
                // a track that plays notes it holds notes, as a take does.
                let name = format!("Pattern {}", s + 1);
                let page = Grid::new(DEFAULT_SIZE).expect("a 1/16 is a step size");
                let mut c = Clip::new(&name, page.page_length());
                c.holds = self.take_holds(track);
                (s, Some(c))
            }
        };
        let clip = match &made {
            Some(c) => c,
            None => self.project.clip(track, slot).ok_or("the clip is gone")?,
        };
        // The clip's own steps (refactor D1): its step size cuts it.
        let grid = if sixteenths {
            Grid::new(DEFAULT_SIZE).expect("a 1/16 is a step size")
        } else {
            grid_of(clip)
        };
        let speed = clip.speed;
        // A step at a time, from the clip's own length (refactor D2), so
        // detents that come before the screen hears back all count: the
        // length it comes to.
        let a = match a {
            StepAction::LengthBy { steps: by } => {
                let n = grid.steps(clip.length);
                let to = if by > 0 {
                    // A longer clip (recorded) is never cut by a +.
                    (n + by.unsigned_abs() as usize).min(super::MAX_STEPS.max(n))
                } else {
                    n.saturating_sub(by.unsigned_abs() as usize).max(1)
                };
                if to == n {
                    return Err(if by > 0 {
                        format!("steps: {} steps is the most here", super::MAX_STEPS)
                    } else {
                        "steps: 1 step is the fewest".into()
                    });
                }
                StepAction::Length {
                    steps: to,
                    copy: false,
                }
            }
            a => a,
        };
        // A lock on the instrument goes in only where the clip's
        // instrument locks were made for this one (refactor F1).
        if let StepAction::Lock {
            addr,
            value: Some(_),
            ..
        } = a
        {
            // A pad's own setting has no note to follow on a clip of notes
            // (E1): its lock would end on a pitch, not the pad.
            let pad = matches!(addr.on, mpfree_play::mpfree_param::OnTrack::Pad(_));
            if pad && clip.holds == mpfree_seq::Holds::Notes {
                return Err(
                    "steps: a pad's own setting has no note to follow on a clip of notes"
                        .to_string(),
                );
            }
            if addr.on == mpfree_play::mpfree_param::OnTrack::Inst {
                let plays = self.project.tracks.get(track).and_then(|t| t.plays.get());
                if let Some(why) = clip.inst_refuses(addr.param, plays) {
                    return Err(format!("steps: {why}"));
                }
            }
        }
        // The pads name the rows; on a clip of notes each pad's row stands
        // for the pad's notes (step 5c), and the log names its root.
        let rows = self.rows_of(track, clip);
        let notes = clip.holds == mpfree_seq::Holds::Notes;
        let row = |p: u8| row_name(p, notes);
        let (keys_are, key_name) = if notes {
            ("notes", crate::synth::note_name as fn(u8) -> String)
        } else {
            (
                "pads",
                (|p: u8| (u16::from(p) + 1).to_string()) as fn(u8) -> String,
            )
        };
        let act = |a: StepAction| match a {
            StepAction::Tap {
                pad,
                step,
                velocity,
            } => (
                grid.toggle(clip, pad, step, velocity),
                None,
                format!("step {} of {}", step + 1, row(pad)),
            ),
            StepAction::Enter { step, keys, n } => {
                let keys = &keys[..usize::from(n).min(keys.len())];
                let pads: Vec<String> = keys.iter().map(|k| key_name(k.0)).collect();
                (
                    grid.enter(clip, step, keys),
                    None,
                    if keys.is_empty() {
                        format!("step {} of the line", step + 1)
                    } else {
                        format!(
                            "step {} of the line: {keys_are} {}",
                            step + 1,
                            pads.join(", ")
                        )
                    },
                )
            }
            StepAction::Keys { steps, keys, n } => {
                let keys = &keys[..usize::from(n).min(keys.len())];
                let pads: Vec<String> = keys.iter().map(|k| key_name(k.0)).collect();
                let list = step_list(steps);
                let names: Vec<String> = list.iter().map(|i| (i + 1).to_string()).collect();
                let s = if list.len() == 1 { "" } else { "s" };
                (
                    grid.enter_steps(clip, &list, keys),
                    Some(mpfree_seq::steps::enter_key(steps)),
                    format!(
                        "step{s} {} of the line: {keys_are} {} from the keys",
                        names.join(", "),
                        pads.join(", ")
                    ),
                )
            }
            StepAction::Change { pad, step, change } if pad == LINE => (
                grid.change_all(clip, step, change),
                Some(change.key_all(step)),
                format!(
                    "step {} of the line: {}",
                    step + 1,
                    change_name(change, notes)
                ),
            ),
            StepAction::Change { pad, step, change } => (
                grid.change(clip, pad, step, change),
                Some(change.key(pad, step)),
                format!("step {} of {}: {change:?}", step + 1, row(pad)),
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
                        row(pad),
                        change_name(change, notes)
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
                        row(pad),
                        value.map_or("off".to_string(), |v| format!("{v}"))
                    ),
                )
            }
            StepAction::ClearLocks { pad, steps } => {
                let held = step_list(steps);
                (
                    grid.clear_step_locks(clip, (pad != LINE).then_some(pad), &held),
                    None,
                    format!("{} of {}", steps_text(&held), row(pad)),
                )
            }
            StepAction::ClearPad { pad } => (
                grid.clear_row(clip, pad),
                None,
                format!("{}'s steps", row(pad)),
            ),
            StepAction::ClearSteps { pad, from, to } => (
                if pad == LINE {
                    grid.clear_line(clip, from, to)
                } else {
                    grid.clear_steps(clip, pad, from, to)
                },
                None,
                if to == from + 1 {
                    format!("step {} of {}", from + 1, row(pad))
                } else {
                    format!("steps {} to {to} of {}", from + 1, row(pad))
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
                    format!("step {} of {} to step {}", from + 1, row(pad), to + 1)
                } else {
                    format!(
                        "steps {} to {} of {} to steps {} to {}",
                        from + 1,
                        from + count,
                        row(pad),
                        to + 1,
                        to + count
                    )
                },
            ),
            StepAction::Length { steps, copy } => {
                (grid.resize(clip, steps, copy), None, clip.name.clone())
            }
            // A lane's stroke (refactor D3): each step its own value, one
            // edit.
            StepAction::Values {
                pad,
                from,
                kind,
                values,
            } => {
                let changes: Vec<(usize, mpfree_seq::steps::Change)> = values
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| **v > 0)
                    .map(|(k, &v)| (from + k, kind.change(v)))
                    .filter(|(i, _)| !grid.notes(clip, pad, *i).is_empty())
                    .collect();
                let n = changes.len();
                (
                    grid.change_each(clip, pad, &changes),
                    None,
                    format!(
                        "{} of {} on {n} step{}",
                        kind.name(),
                        row(pad),
                        if n == 1 { "" } else { "s" }
                    ),
                )
            }
            // Made a Length above.
            StepAction::LengthBy { .. } => (Edit::default(), None, String::new()),
        };
        let (edit, join, what) = if notes {
            on_notes(grid, clip, a, &rows, act)
        } else {
            act(a)
        };
        let resized = edit.length;
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
        // What its instrument locks were made for, after the edit (the
        // first one's instrument; none with the last gone; refactor F1).
        self.tag_clips(Some(track));
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
            (None, StepAction::Values { .. }) => format!("steps: {what}"),
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
            (None, StepAction::Length { .. } | StepAction::LengthBy { .. }) => {
                let copy = matches!(a, StepAction::Length { copy: true, .. });
                let (old, new) = resized.unwrap_or_default();
                let steps = grid.steps(new);
                let st = if steps == 1 { "step" } else { "steps" };
                // Bars of the song's meter, at the clip's step size.
                let bar = self.info.meter.bar_ticks().max(1) as f64;
                let song = speed.to_song(new as f64);
                let bars = ((song / bar) - 1e-9).ceil().max(1.0) as usize;
                let s = if bars == 1 { "" } else { "s" };
                let how = if copy {
                    " · the new steps repeat the ones before them"
                } else if new < old {
                    " · UNDO brings the rest back"
                } else {
                    " · the new steps are empty"
                };
                format!("steps: {what} is {steps} {st} ({bars} bar{s}){how}")
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
    /// row turned or a Q-Link turn joins one; a stroke along a lane is
    /// one). A tap on a
    /// note made on an older picture of the clip is refused, and an edit
    /// what plays can't follow is taken back (piano roll plan 0A.1).
    fn edit(&mut self, a: EditAction) -> Result<String, String> {
        // Its answer, for the screen (plan 0C.2): numbered, with the
        // revision it was carried out on and what became of it.
        self.edit_follow();
        let from = self.edit_rev;
        let open = matches!(a, EditAction::Open | EditAction::Switch { .. });
        let r = self.edit_touch(a);
        // The editor opened on another clip: Continue goes back.
        if self.continue_follows_editor() {
            let _ = self.compile();
        }
        // The revision moves on Open, and on an edit that went in.
        let took = r.is_ok() && !open && self.edit_rev != from;
        let outcome = match &r {
            Err(why) => EditOutcome::Refused(why.clone()),
            Ok(_) if took => EditOutcome::Taken {
                rev: self.edit_rev,
                landed: self
                    .editor
                    .as_ref()
                    .map_or_else(Vec::new, |e| e.sel.clone()),
            },
            Ok(_) => EditOutcome::Unchanged,
        };
        self.edit_answer = EditAnswer {
            n: self.edit_answer.n + 1,
            from,
            outcome,
        };
        self.changed = true;
        r
    }

    /// [`TimeControl::edit`]'s touch itself.
    fn edit_touch(&mut self, a: EditAction) -> Result<String, String> {
        if let EditAction::Switch { track, slot } = a {
            return self.edit_switch(track, slot);
        }
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
            let mut ed = Editor::new(track, slot, clip);
            ed.offers = Arc::clone(&self.scale_offers);
            ed.styles = Arc::clone(&self.styles);
            self.editor = Some(ed);
            self.edit_seen = Some(Arc::new(clip.clone()));
            self.edit_rev += 1;
            self.changed = true;
            return Ok(line);
        }
        // `edit` has followed the clip.
        let at = Instant::now();
        if let Some(rev) = a.rev() {
            if rev != self.edit_rev {
                // The screen still shows the clip as it was: it gets the
                // new one, and the tap goes nowhere.
                self.changed = true;
                return Err(
                    "edit: the clip changed under your finger, so the tap did nothing".into(),
                );
            }
        }
        let Some(mut ed) = self.editor.take() else {
            return Ok(String::new());
        };
        // COPY + empty space: the paste is made here, where the copy is
        // kept (plan 1B.4).
        let pasted = match a {
            EditAction::Paste { tick } => Some(self.paste_notes(&ed, tick)),
            _ => None,
        };
        let Some(clip) = self.project.clip(ed.track, ed.slot) else {
            self.changed = true;
            return Err("edit: the clip is gone".into());
        };
        let before = ed.clone();
        let copying = matches!(a, EditAction::Copy { .. });
        let done = match pasted {
            Some(Ok(done)) => done,
            Some(Err(why)) => {
                self.editor = Some(ed);
                self.changed = true;
                return Err(why);
            }
            None => ed.act(a, clip),
        };
        // COPY + a note: the editor picked what goes, and the copy is
        // kept here, apart from the clip (plan 1B.4).
        let copied = copying.then(|| {
            let made = clip.made_for.clone();
            (notes::copy(clip, &ed.sel), clip.name.clone(), made)
        });
        self.changed = true;
        if let Some((copied, name, made)) = copied {
            let track = &self.project.tracks[ed.track];
            let (from, fx) = (track.id, track.note_fx);
            let inst = made.or_else(|| track.plays.get().cloned());
            self.editor = Some(ed);
            if copied.notes.is_empty() {
                return Err("copy: that note is gone, so nothing was copied".into());
            }
            let n = copied.notes.len();
            let said = format!("{n} note{} from {name}", if n == 1 { "" } else { "s" });
            let line = format!(
                "copy: {said} copied · COPY + empty space, here or in another clip, puts them there"
            );
            self.notes_copy = Some(NoteCopy {
                copied,
                from,
                fx,
                inst,
                said,
            });
            return Ok(line);
        }
        // Continue's tool (composing tools, phase 1 step 2): its asks go
        // to the composer.
        if let Some(c) = done.compose {
            let (track, slot) = (ed.track, ed.slot);
            self.editor = Some(ed);
            return self.continue_from_editor(c, track, slot, &done.line);
        }
        if done.close {
            self.edit_seen = None;
            return Ok(done.line);
        }
        // The clip's scale (plan 2B.1): a clip setting, as the step size
        // is, one take a run of turns; no note changes, so nothing
        // compiles.
        if let Some(scale) = done.scale {
            let (track, slot) = (ed.track, ed.slot);
            self.editor = Some(ed);
            let settings = mpfree_seq::ClipSettings {
                scale,
                ..self
                    .project
                    .clip(track, slot)
                    .ok_or("edit: the clip is gone")?
                    .settings()
            };
            self.takes
                .set_clip(&mut self.project, track, slot, settings, done.join)
                .map_err(|e| format!("edit: {e:?}"))?;
            self.edit_seen = self.project.clip(track, slot).cloned().map(Arc::new);
            self.edit_rev += 1;
            return Ok(format!("{} (UNDO takes it back)", done.line));
        }
        if !done.edit.is_empty() {
            let (track, slot) = (ed.track, ed.slot);
            let edits = self.takes.edits();
            let applied = self
                .takes
                .edit(&mut self.project, track, slot, &done.edit, done.join)
                .map_err(|e| format!("edit: {e:?}"));
            if let Err(e) = applied {
                self.editor = Some(ed);
                return Err(e);
            }
            if let Some(clip) = self.project.clip(track, slot) {
                ed.applied(clip, &done);
            }
            self.editor = Some(ed);
            // New pressure runs: the clip frees the ones nothing names
            // once it is past half its room (B4b).
            if !done.edit.touch.is_empty() && !self.touch_grew.contains(&(track, slot)) {
                self.touch_grew.push((track, slot));
            }
            // Into the playing clip in place when it fits, as a step edit
            // goes (plan 0C.1); else one compile.
            let in_place = self.in_place(track, slot, &done.edit);
            let built = if in_place {
                self.steps_changed = true;
                Ok(())
            } else {
                self.compile()
            };
            let (took, c) = (at.elapsed(), &mut self.costs);
            if in_place {
                c.notes_in_place += 1;
                c.slowest_note_in_place = c.slowest_note_in_place.max(took);
            } else {
                c.notes_compiled += 1;
                c.slowest_note_compiled = c.slowest_note_compiled.max(took);
            }
            if let Err(e) = built {
                // What plays can't follow: the edit goes from the clip and
                // the takes, the editor is as it was, and the view as it
                // was is built again (as a step edit does).
                let fresh = self.takes.edits() != edits;
                let back = self.takes.withdraw(
                    &mut self.project,
                    (track, slot),
                    &done.edit,
                    done.join,
                    fresh,
                );
                self.editor = Some(before);
                let _ = self.compile();
                return Err(match back {
                    Ok(()) => format!("edit: {e}: the edit was taken back"),
                    Err(b) => format!("edit: {e}: the edit could not be taken back ({b:?})"),
                });
            }
            self.edit_seen = self.project.clip(track, slot).cloned().map(Arc::new);
            self.edit_rev += 1;
        } else {
            self.editor = Some(ed);
        }
        Ok(done.line)
    }

    /// COPY + empty space in the note editor (plan 1B.4): the notes copied
    /// last into the editor's clip, the first at `tick`, as one take, with
    /// their pressure and the locks the clip's track has the setting of
    /// (the rest left out and said). Refused, with why, when nothing was
    /// copied, the clip holds the other kind (pad hits or notes) or it has
    /// no room. Short of room for the pressure, the room no take still
    /// needs is taken back first (`Takes::reclaim_touch`) and it tries
    /// once more.
    fn paste_notes(&mut self, ed: &Editor, tick: Tick) -> Result<Done, String> {
        let (track, slot) = (ed.track, ed.slot);
        let mut carried = self.carry_notes(track, slot, tick)?;
        if matches!(carried, Err(CarryError::Pressure))
            && self.takes.reclaim_touch(&mut self.project, track, slot) > 0
        {
            // The notes are where they were, naming renumbered runs: the
            // editor's picks stay, and its picture moves on.
            self.edit_seen = self.project.clip(track, slot).cloned().map(Arc::new);
            self.edit_rev += 1;
            carried = self.carry_notes(track, slot, tick)?;
        }
        let c = self
            .notes_copy
            .as_ref()
            .ok_or("paste: nothing copied yet")?;
        let (clip, to) = (
            self.project.clip(track, slot),
            self.project.tracks.get(track),
        );
        let (Some(clip), Some(to)) = (clip, to) else {
            return Err("edit: the clip is gone".into());
        };
        let carried = carried.map_err(|e| {
            let room = match e {
                CarryError::Nothing => return "paste: nothing copied yet".to_string(),
                CarryError::Holds => {
                    let (has, copy) = match clip.holds {
                        mpfree_seq::Holds::Hits => ("pad hits", "notes"),
                        mpfree_seq::Holds::Notes => ("notes", "pad hits"),
                    };
                    return format!(
                        "paste: {} holds {has} and the copy is {copy}, so nothing went in",
                        clip.name
                    );
                }
                CarryError::Groups => {
                    return format!(
                        "paste: {} has too many chords on a step the copy lands on, so nothing went in",
                        clip.name
                    );
                }
                CarryError::Notes => "notes",
                CarryError::Locks => "locks",
                CarryError::Pressure => "pad pressure",
            };
            format!(
                "paste: {} has no room for {} more {room}, so nothing went in",
                clip.name, c.said
            )
        })?;
        let mut line = format!("paste: {} into {} on {}", c.said, clip.name, to.name);
        if !carried.left.is_empty() {
            // Why instrument locks stayed out, when it is known (refactor
            // F1): made for another instrument than the track plays, or
            // the clip's own are for another.
            let plays = to.plays.get();
            let inst_why = match (c.inst.as_ref(), plays, clip.made_for.as_ref()) {
                (Some(made), Some(now), _) if made.node != now.node => Some(format!(
                    "made for {}, {} plays {}",
                    made.name, to.name, now.name
                )),
                (_, Some(now), Some(own))
                    if own.node != now.node && clip.inst_params().next().is_some() =>
                {
                    Some(format!("{}'s are made for {}", clip.name, own.name))
                }
                _ => None,
            };
            line += &left_out(&carried.left, &to.name, inst_why.as_deref());
        }
        Ok(Editor::pasted(carried.edit, line))
    }

    /// [`notes::carry`] of the notes copied last into clip `slot` of
    /// `track` as it is now (Err when nothing was copied or the clip is
    /// gone).
    fn carry_notes(
        &self,
        track: usize,
        slot: usize,
        tick: Tick,
    ) -> Result<Result<notes::Carried, CarryError>, String> {
        let c = self
            .notes_copy
            .as_ref()
            .ok_or("paste: nothing copied yet: COPY + a note copies the picked notes")?;
        let to = self
            .project
            .tracks
            .get(track)
            .ok_or("paste: no such track")?;
        let clip = self
            .project
            .clip(track, slot)
            .ok_or("edit: the clip is gone")?;
        let same = to.id == c.from;
        let plays = to.plays.get();
        let from = notes::TrackSettings {
            fx: &c.fx,
            inst: c.inst.as_ref(),
        };
        let there = notes::TrackSettings {
            fx: &to.note_fx,
            inst: plays,
        };
        // A clip's instrument locks are all made for one instrument
        // (refactor F1): one for the track's goes only into a clip whose
        // own are for it too, or that has none.
        let keep = |l: &Lock| {
            notes::same_setting(same, from, there, l.addr)
                && (l.addr.on != mpfree_seq::mpfree_param::OnTrack::Inst
                    || clip.inst_refuses(l.addr.param, plays).is_none())
        };
        Ok(notes::carry(clip, &c.copied, tick, keep))
    }

    /// The note editor's clip changed since the editor last knew it, by
    /// something other than the editor's own edit (a recorded note, UNDO,
    /// an edit from Steps, a clip moved): its picks follow their notes
    /// (cleared, and said, when one is gone) and the clip's revision
    /// moves, so a tap on the old picture is refused.
    /// A tap on another clip's faint note (piano roll plan 4.1): the
    /// editor opens track `track`'s slot `slot`, selecting the track and
    /// the clip as the Sequencer does (the pads follow), and the clip it
    /// left shows faintly in its place. Not while a tool is open.
    fn edit_switch(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let old = self.editor.as_ref().ok_or("edit: no clip is open")?;
        if let Some(tool) = old.open_tool {
            return Err(format!("edit: Done or Cancel {} first", tool.kind.name()));
        }
        let left = (old.track, old.slot);
        let mut show = old.show.clone();
        let holds = self.project.clip(left.0, left.1).map(|c| c.holds);
        let clip = self
            .project
            .clip(track, slot)
            .ok_or("edit: that clip is gone")?;
        if Some(clip.holds) != holds || (track, slot) == left {
            return Err("edit: that clip can't open here".into());
        }
        let was = self.track_pick;
        self.set_pick(track);
        if self.track_pick != was && self.track_pick.is_some() {
            self.compile()?;
        }
        self.steps_changed = true;
        self.edit_clip(Some(slot))?;
        let clip = self
            .project
            .clip(track, slot)
            .ok_or("edit: that clip is gone")?;
        match show.iter().position(|&s| s == (track, slot)) {
            Some(k) => show[k] = left,
            None => {
                show.insert(0, left);
                show.truncate(mpfree_time_ui::editor::SHOW_MAX);
            }
        }
        let name = |t: usize| {
            self.project
                .tracks
                .get(t)
                .map_or_else(String::new, |t| t.name.clone())
        };
        let line = format!(
            "edit: {} on {}; {} shows behind",
            clip.name,
            name(track),
            self.project
                .clip(left.0, left.1)
                .map_or_else(String::new, |c| c.name.clone())
        );
        let mut ed = Editor::new(track, slot, clip);
        ed.offers = Arc::clone(&self.scale_offers);
        ed.styles = Arc::clone(&self.styles);
        ed.show = show;
        self.editor = Some(ed);
        self.edit_seen = Some(Arc::new(clip.clone()));
        self.edit_rev += 1;
        self.changed = true;
        Ok(line)
    }

    /// The other clips the editor shows (piano roll plan 4.1): those still
    /// there, of its clip's kind and with notes to show, each copy kept
    /// while its clip is unchanged so the screens tell it at no cost.
    fn ghosts_follow(&mut self) {
        let Some(ed) = self.editor.as_ref() else {
            self.ghost_seen.clear();
            return;
        };
        let holds = self.project.clip(ed.track, ed.slot).map(|c| c.holds);
        let mut now = Vec::with_capacity(ed.show.len());
        for &(track, slot) in &ed.show {
            let Some(t) = self.project.tracks.get(track) else {
                continue;
            };
            let Some(clip) = self.project.clip(track, slot) else {
                continue;
            };
            if t.is_loops() || t.is_audio() || Some(clip.holds) != holds {
                continue;
            }
            let seen = self
                .ghost_seen
                .iter()
                .find(|g| (g.track, g.slot) == (track, slot) && *g.clip == *clip)
                .map(|g| Arc::clone(&g.clip));
            now.push(Ghost {
                track,
                slot,
                clip: seen.unwrap_or_else(|| Arc::new(clip.clone())),
                track_name: t.name.clone(),
            });
        }
        self.ghost_seen = now;
    }

    fn edit_follow(&mut self) {
        let Some(ed) = self.editor.as_mut() else {
            self.edit_seen = None;
            return;
        };
        let now = self.project.clip(ed.track, ed.slot);
        if now == self.edit_seen.as_deref() {
            return;
        }
        match (self.edit_seen.as_deref(), now) {
            (Some(was), Some(now)) => {
                ed.follow(was, now);
            }
            _ => ed.sel.clear(),
        }
        self.edit_seen = now.cloned().map(Arc::new);
        self.edit_rev += 1;
        self.changed = true;
    }

    /// The time edit a song action makes (a section moved or copied, bars
    /// put in or taken out, a linked list's step edited), if it makes one.
    fn time_edit_of(&self, a: &TimeAction) -> Option<TimeEdit> {
        let bar = self.info.meter.bar_ticks();
        let song = &self.project.arrangement;
        match *a {
            TimeAction::MoveSection { marker, gap, copy } => {
                song.section_edit(marker, gap, copy, bar)
            }
            TimeAction::SongBars(e) => bars_edit(song, e, bar.max(1)).ok().map(|x| x.0),
            // Song from the list's room at the song's start (4b.5).
            TimeAction::ListLink(from) => self.link_room(from, bar),
            // A linked list's edits are the song's (Arrange 4b.2).
            _ if song.list.linked && edits_steps(a) => {
                self.linked_edit(a, bar).ok().and_then(|e| e.time)
            }
            _ => None,
        }
    }

    /// [`TimeControl::act`] with the audio tracks (the Arrange refactor's
    /// 3.4): a song time edit moves their regions too, in the same period,
    /// and UNDO and REDO of one move them back and on.
    ///
    /// - Before a time edit the tracks are asked; their refusal (a track
    ///   recording, no room for the cut regions) refuses the edit and
    ///   nothing changes anywhere. After it, they get the same edit as the
    ///   song edit's undo step, numbered by its take.
    /// - UNDO of such an edit takes the tracks' step back with it. With
    ///   the tracks edited since (their step under others), UNDO waits:
    ///   it says to take those back first. With the step gone (the tracks'
    ///   history ran past it, or was cleared), the song goes back and the
    ///   regions stay where they are: said, and counted (`audio_stayed`).
    /// - REDO brings the step back with the song; gone (an audio edit
    ///   since dropped what Redo could bring), the same edit is made again
    ///   on the regions as they are now.
    pub fn act_with_audio(
        &mut self,
        a: TimeAction,
        audio: &mut dyn SongAudio,
    ) -> Result<String, String> {
        // Song edits the song's history no longer holds can't come back.
        let takes = &self.takes;
        self.audio_edits.retain(|&n, _| {
            let held = |t: &mpfree_play::Take| t.edited && t.txn == n;
            takes.done().iter().chain(takes.undone()).any(held)
        });
        if matches!(a, TimeAction::Undo | TimeAction::Redo) {
            return self.undo_with_audio(a == TimeAction::Undo, audio);
        }
        // Song → scenes turns the song's regions into audio clips too
        // (audio tracks phase 2, step 2d): asked for this one edit.
        if a == TimeAction::SongToScenes {
            self.song_regions = audio.regions();
            let r = self.act(a);
            self.song_regions = Vec::new();
            return r;
        }
        let Some(retime) = self.time_edit_of(&a).as_ref().and_then(retime_of) else {
            return self.act(a);
        };
        if let Some(why) = audio.refused(&retime) {
            return Err(format!("song: nothing moved: {why}"));
        }
        let moves = audio.moves(&retime);
        let last = self
            .takes
            .done()
            .last()
            .filter(|t| t.edited && t.song.is_some());
        let last = last
            .map(|t| t.txn)
            .filter(|n| self.audio_edits.contains_key(n));
        (self.audio_moves, self.audio_apart) = (
            moves,
            last.is_some_and(|n| audio.step(n, true) != SongStep::Next),
        );
        let r = self.act(a);
        (self.audio_moves, self.audio_apart) = (false, false);
        let said = r?;
        if !moves {
            return Ok(said);
        }
        // The edit's take: a new one, or the last one + Bar joined.
        let take = self
            .takes
            .done()
            .last()
            .filter(|t| t.edited && t.song.is_some());
        if let Some(n) = take.map(|t| t.txn) {
            if audio.retime(n, retime) {
                self.audio_edits.entry(n).or_default().push(retime);
            }
        }
        Ok(said)
    }

    /// UNDO (`undo`) or REDO with the audio tracks: see
    /// [`TimeControl::act_with_audio`].
    fn undo_with_audio(&mut self, undo: bool, audio: &mut dyn SongAudio) -> Result<String, String> {
        let a = if undo {
            TimeAction::Undo
        } else {
            TimeAction::Redo
        };
        let next = if undo {
            self.takes.done().last()
        } else {
            self.takes.undone().last()
        };
        let Some(n) = next
            .filter(|t| t.edited && t.song.is_some())
            .map(|t| t.txn)
            .filter(|n| self.audio_edits.contains_key(n))
        else {
            return self.act(a);
        };
        // Refusals don't start "undo: ", which the screen says as "Undid".
        let waits = if undo { "UNDO waits" } else { "REDO waits" };
        if let Some(why) = audio.busy() {
            return Err(format!("{waits}: {why}"));
        }
        match audio.step(n, undo) {
            SongStep::Next => {
                let said = self.act(a)?;
                audio.song_undo(n, undo);
                Ok(said)
            }
            SongStep::Under if undo => Err(format!(
                "{waits}: the audio regions changed after this song edit; take that back \
                 first (pick a region, then Undo)"
            )),
            SongStep::Under => Err(format!(
                "{waits}: the audio regions have a change to bring back first (pick a \
                 region, then SHIFT + Undo)"
            )),
            SongStep::Gone if undo => {
                let said = self.act(a)?;
                self.audio_stayed += 1;
                Ok(format!(
                    "{said}; the audio regions stay where they are (their Undo no longer \
                     reaches this edit)"
                ))
            }
            SongStep::Gone => {
                let edits = self.audio_edits.get(&n).cloned().unwrap_or_default();
                if let Some(why) = edits.first().and_then(|r| audio.refused(r)) {
                    return Err(format!("{waits}: {why}"));
                }
                let said = self.act(a)?;
                let moved = edits.into_iter().fold(false, |m, r| audio.retime(n, r) | m);
                if !moved {
                    self.audio_edits.remove(&n);
                }
                Ok(said)
            }
        }
    }

    /// Whether the song's UNDO (`undo`), or its REDO, still reaches song
    /// edit `n`: for the audio tracks' own Undo and Redo, which stop at
    /// its step while it does and forget the step once it doesn't
    /// ([`mpfree_audiotracks::Command::ForgetSong`]).
    pub fn song_reaches(&self, n: mpfree_transport::TransactionId, undo: bool) -> bool {
        let list = if undo {
            self.takes.done()
        } else {
            self.takes.undone()
        };
        list.iter()
            .any(|t| t.edited && t.song.is_some() && t.txn == n)
    }

    /// A take, or an import placing regions, has ended on audio track
    /// `track`'s lane over song ticks `from..to` (audio tracks phase 2,
    /// step 2d): a lane holds regions and blocks, never one over the
    /// other, so the blocks there are cut at the span's edges, as a new
    /// take covers the regions under it. The parts inside go and the parts
    /// outside play as before. The cut is a song edit of its own, and its
    /// number is returned for the audio tracks to tag the take's undo step
    /// with, as a time edit's [`Retime`] is: from then on the two are one
    /// for either UNDO ([`TimeControl::act_with_audio`]). None when no
    /// block was there (nothing to tie).
    ///
    /// Where cutting a block in two needs more blocks than the song holds,
    /// the recording wins: the whole block goes, said and counted
    /// (`audio_took_whole`).
    pub fn audio_took(
        &mut self,
        track: TrackId,
        (from, to): (Tick, Tick),
    ) -> Result<Option<(mpfree_transport::TransactionId, String)>, String> {
        let tracks = &self.project.tracks;
        let Some(t) = tracks.iter().position(|tr| tr.id == track && tr.is_audio()) else {
            return Ok(None);
        };
        let over = |p: &mpfree_seq::Placement| p.track == t && p.start < to && p.end() > from;
        let before = self.project.arrangement.clone();
        let Some(first) = before.placements.iter().find(|p| over(p)).map(|p| p.start) else {
            return Ok(None);
        };
        let name = tracks[t].name.clone();
        let at = |x: Tick| bar_beat(x, self.info.meter);
        let mut song = before.clone();
        let said = match song.clear(tracks, t, from, to) {
            Ok(()) => format!(
                "song: the recording on {name} at {} cut the blocks under it",
                at(from)
            ),
            Err(_) => {
                song = before.clone();
                song.placements.retain(|p| !over(p));
                self.audio_took_whole += 1;
                format!(
                    "song: the recording on {name} at {} took the whole block under it (the song has no room to cut it)",
                    at(from)
                )
            }
        };
        song.drop_unplayed();
        self.project.arrangement = song;
        if let Err(e) = self.compile() {
            self.project.arrangement = before;
            self.compile()?;
            return Err(e);
        }
        self.takes.song_edit_joining(
            &self.project,
            SongChange::Recorded {
                track: t,
                start: first.max(from),
            },
            before,
            None,
        );
        self.changed = true;
        let n = self
            .takes
            .done()
            .last()
            .map_or(mpfree_transport::TransactionId::NONE, |t| t.txn);
        // Tied: UNDO and REDO of it go through the audio tracks.
        self.audio_edits.entry(n).or_default();
        Ok(Some((n, said)))
    }

    /// Carry out a touch on the time engine's screens. Returns a line for
    /// the log.
    pub fn act(&mut self, a: TimeAction) -> Result<String, String> {
        // The song's parts follow their rows (parts.rs); undo and redo
        // bring back the song their take holds instead.
        let follows = !matches!(a, TimeAction::Undo | TimeAction::Redo);
        let rows = if follows {
            self.followed_rows()
        } else {
            Vec::new()
        };
        let edits = self.takes.edits();
        let mut r = self.act_in(a);
        self.sync_styles();
        self.hear_song();
        if let (Ok(line), false) = (&mut r, rows.is_empty()) {
            if let Some(more) = self.follow_rows(&rows, edits) {
                // Before "(UNDO takes it back)" where the line ends so.
                match line.rfind(" (UNDO") {
                    Some(k) if line.ends_with(')') => line.insert_str(k, &more),
                    _ => line.push_str(&more),
                }
            }
        }
        // An action may add or take instrument locks (UNDO, a paste, a
        // clip copied to another track): the tags follow (refactor F1).
        self.tag_clips(None);
        r
    }

    fn act_in(&mut self, a: TimeAction) -> Result<String, String> {
        let info = self.info;
        // An edit that moves time in the song gives the views compiled from
        // now on a new timeline, so a timing change made before it is
        // cancelled (step 0C, agreed with the Arrange refactor). Before the
        // edit, so the view it compiles carries it; one that fails only
        // cancels a change early.
        if moves_song_time(&a)
            || (self.project.arrangement.list.linked && edits_steps(&a))
            || a == TimeAction::ListLink(LinkFrom::List)
        {
            self.includes.timeline = next_timeline();
        }
        // Anything else done but the transport closes Convert to notes'
        // preview (E1), which STOP leaves open for its Convert.
        if self.convert_asked.is_some()
            && !matches!(
                a,
                TimeAction::ConvertPreview { .. }
                    | TimeAction::Convert { .. }
                    | TimeAction::ConvertCancel
                    | TimeAction::Transport(_)
            )
        {
            self.convert_asked = None;
            self.changed = true;
        }
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
                // Continue's new bars never play while recording: the
                // clip comes back before the take starts.
                if self.compose.stop_for_rec() {
                    self.compile()?;
                    self.note_later(
                        "continue: REC brought the clip back; the new bars stay ready".into(),
                    );
                }
                if self.add.stop_for_rec() {
                    self.compile()?;
                    self.note_later("add a part: REC quietened the try; it stays ready".into());
                }
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
                    // A conversion's card closes: what it said is gone or
                    // no longer the last thing done.
                    self.report = None;
                    let row = self.takes.undone().last().map(TakeRow::of);
                    if let Some(d) = row.and_then(|r| r.dropped) {
                        self.follow_launches(d, true);
                    }
                    if let Some(("delete", r)) = row.and_then(|t| t.row_tool) {
                        // The rows under it go back down.
                        self.rows_followed(|k| {
                            let k = if k >= r { k + 1 } else { k };
                            (k < ROWS).then_some(k)
                        });
                    }
                    if row.is_some_and(|r| r.song.is_some()) {
                        // The song's blocks and sections changed places.
                        self.selected = None;
                        self.picks.clear();
                        self.section = None;
                        self.includes.timeline = next_timeline();
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
                    self.report = None;
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
                    if let Some(("delete", r)) = row.and_then(|t| t.row_tool) {
                        // Its clips stop again; the rows under it up one.
                        self.rows_followed(|k| match k.cmp(&r) {
                            std::cmp::Ordering::Less => Some(k),
                            std::cmp::Ordering::Equal => None,
                            std::cmp::Ordering::Greater => Some(k - 1),
                        });
                    }
                    if row.is_some_and(|r| r.song.is_some()) {
                        self.selected = None;
                        self.picks.clear();
                        self.section = None;
                        self.includes.timeline = next_timeline();
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
                        *settings,
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
                self.step_action_in(StepAction::Length { steps, copy }, true)
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
                // Audio clips paste onto audio tracks only (phase 2).
                if t.is_audio() && c.audio.is_none() {
                    return Err(format!(
                        "paste: {} is an audio track: it takes audio clips only",
                        t.name
                    ));
                }
                if !t.is_audio() && c.audio.is_some() {
                    return Err(format!(
                        "paste: {} plays notes: an audio clip goes on an audio track",
                        t.name
                    ));
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
            TimeAction::Arm(Some(t))
                if self.project.tracks.get(t).is_some_and(|t| t.is_audio()) =>
            {
                // An audio track records with its own arm (its dot on the
                // Song view): the pads stay on their track.
                let name = &self.project.tracks[t].name;
                Err(format!(
                    "armed: {name} is an audio track: its arm dot on the Song view arms it"
                ))
            }
            TimeAction::ArmAlso(tracks) => {
                if matches!(self.info.state, State::Recording | State::CountIn) {
                    return Err("armed: finish the take first".into());
                }
                let n = self.project.tracks.len();
                let mut keep = 0u128;
                for (t, tr) in self.project.tracks.iter().enumerate() {
                    if tracks & (1 << t) != 0 && tr.plays_notes() {
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
            TimeAction::Select(t) if self.project.tracks.get(t).is_some_and(|t| t.is_audio()) => {
                // An audio track has no steps either.
                let name = &self.project.tracks[t].name;
                Ok(
                    match self.selected().and_then(|s| self.project.tracks.get(s)) {
                        Some(s) => format!(
                            "{name} is an audio track: the Sequencer stays on {}",
                            s.name
                        ),
                        None => format!("{name} is an audio track"),
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
            TimeAction::EditClip(slot) => self.edit_clip(slot),
            TimeAction::StepSize(size) => self.step_size(size),
            TimeAction::Arm(track) => {
                self.set_armed(track);
                // The Clip tab follows the note track armed now.
                self.audio_focus = TrackId::NONE;
                self.compile()?;
                Ok(match self.armed {
                    Some(t) => format!("armed: {}", self.project.tracks[t].name),
                    None => "armed: none".into(),
                })
            }
            // Linked to the song, the list plays the song (4b.4).
            TimeAction::ListPlay(k) if self.project.arrangement.list.linked => {
                self.play_from_part(k)
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
                // Free cells (phase 4e.4): a scene keeps the ones in its
                // row playing and stops the others; Stop all and Back to
                // song stop them all.
                let free_was = self.free_on.clone();
                match a {
                    TimeAction::LaunchScene(row) => self.free_on.retain(|f| f.1 == row),
                    TimeAction::StopAll | TimeAction::BackToSong => self.free_on.clear(),
                    _ => {}
                }
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
                            self.free_on = free_was;
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
                // mute queue for its line, and its tempo for its tick.
                self.queue_scene_mutes();
                let tempo_was = self.scene_tempo;
                self.scene_tempo_on_its_line();
                if let Err(e) = self.compile() {
                    self.song = song_was;
                    self.scene_tempo = tempo_was;
                    self.free_on = free_was;
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
                        // A stop ends a free cell at once (phase 4e.4).
                        if matches!(a, TimeAction::StopClip(_)) {
                            self.free_on.retain(|f| f.0 != track);
                        }
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
            TimeAction::NewClip { track, .. }
                if self.project.tracks.get(track).is_some_and(|t| t.is_audio()) =>
            {
                Err(format!(
                    "{} is an audio track: put audio in a cell with To a cell on the Song view, \
                     or Put in a cell on the Sample page",
                    self.project.tracks[track].name
                ))
            }
            TimeAction::NewClip { track, slot } => {
                let holds = self.take_holds(track);
                let bar = info.meter.bar_ticks();
                let length = self.clip_loop().map_or(NEW_CLIP_BARS * bar, |(a, b)| b - a);
                let t = self.project.tracks.get_mut(track).ok_or("no such track")?;
                let s = t.slots.get_mut(slot).ok_or("no such slot")?;
                if s.is_some() {
                    return Err(format!("slot {} of {} is not empty", slot + 1, t.name));
                }
                // With its lasting id (Arrange 1.1), as every clip gets.
                let mut clip = Clip::new(&format!("Clip {}", slot + 1), length);
                clip.holds = holds;
                self.project
                    .put_new_clip(track, slot, clip)
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
            TimeAction::ConvertPreview { track, slot } => self.convert_preview(track, slot),
            // The Make a phrase sheet is the arp's: the app opens it
            // (desk.rs), and nothing here changes.
            TimeAction::MakePhrase { .. } => Ok(String::new()),
            TimeAction::MakeStyle { track, slot } => self.make_style(track, slot),
            TimeAction::Style(s) => self.style_action(s),
            TimeAction::Convert { track, slot } => self.convert(track, slot),
            TimeAction::ConvertCancel => {
                self.convert_asked = None;
                self.changed = true;
                Ok("convert to notes: cancelled, nothing changed".to_string())
            }
            TimeAction::Continue(c) => self.continue_action(c),
            TimeAction::Add(a) => self.add_action(a),
            TimeAction::Chords(c) => self.chords_action(c),
            TimeAction::DeleteClip { track, slot } => {
                // A clip the song plays goes to the song's own clips, so
                // the song plays on (Arrange 1.3), but for the parts that
                // follow its row, which it leaves (parts.rs).
                let kept = self
                    .project
                    .clip(track, slot)
                    .is_some_and(|c| self.plays_apart_from_row(c.id, slot));
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
                // Audio clips stay on audio tracks, and note clips and loops
                // off them (audio tracks phase 2): the reason names the
                // track it was dropped on.
                let audio = |t: usize| self.project.tracks.get(t).map(|t| t.is_audio());
                if audio(from.0) != audio(to.0) {
                    let there = self.project.tracks.get(to.0);
                    let name = there.map_or("", |t| t.name.as_str());
                    return Err(match there {
                        Some(t) if t.is_audio() => {
                            format!("clips: {name} is an audio track: it takes audio clips only")
                        }
                        Some(t) if t.is_loops() => format!(
                            "clips: {name} is a Loop track: an audio clip goes on an audio track"
                        ),
                        _ => format!(
                            "clips: {name} plays notes: an audio clip goes on an audio track"
                        ),
                    });
                }
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
                            "clips: row {ROWS}, the last, is in use, so a copy has no room: empty it first"
                        ),
                        ModelError::BadPlacement => {
                            "clips: the song's blocks would overlap".to_string()
                        }
                        _ => format!("clips: row {} stays where it is", from + 1),
                    })?;
                self.drop_went_in(drop)?;
                // The scene editor's row and the scene card go with their
                // row, as its scene does.
                if let Some(r) = self.scene_pick {
                    self.scene_pick = Some(drop.after((0, r)).1);
                }
                if let Some(r) = self.scene_open {
                    self.scene_open = Some(drop.after((0, r)).1);
                }
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
            TimeAction::AddAudioTrack => {
                let name = loops::audio_name(&self.project);
                let track = mpfree_seq::Track::new_audio(&name, 0);
                self.takes
                    .add_track_as(&mut self.project, track)
                    .map_err(|_| format!("{MAX_TRACKS} tracks is the most a project holds"))?;
                self.compile()?;
                Ok(format!(
                    "{name} added, an audio track: arm it and REC records it in song mode (UNDO takes it away)"
                ))
            }
            TimeAction::LoopCell(None) => {
                if self.loop_cell.take().is_some() {
                    self.changed = true;
                }
                Ok("loop cell: none armed".into())
            }
            TimeAction::LoopCell(Some((t, s))) => {
                let on = self.loops_on;
                let tr = self
                    .project
                    .tracks
                    .get(t)
                    .filter(|tr| takes_loops(tr, on))
                    .ok_or_else(|| match on {
                        true => {
                            format!("track {} is neither a Loop track nor an audio track", t + 1)
                        }
                        false => format!("track {} is not a Loop track", t + 1),
                    })?;
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
            TimeAction::LoopCopy { from, to, into, id } => {
                let n = usize::from(into);
                let name = |p: &Project, (t, s): (usize, usize)| {
                    let track = p.tracks.get(t).map_or("", |t| t.name.as_str());
                    format!("{track} / {}", s + 1)
                };
                let audio = self.loops_on
                    && self
                        .project
                        .clip(from.0, from.1)
                        .is_some_and(|c| c.loop_clip.is_some());
                if audio {
                    return self.copy_audio_loop(from, to, into, id);
                }
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
                // An audio cell's copy: the cell naming its new id.
                let copy = self.copy_ids.get_mut(n).map_or(0, std::mem::take);
                let audio = |p: &mut Project| match copy {
                    0 => 0,
                    id => loops::remove_audio(p, LoopId(id)),
                };
                if n < LOOPS
                    && !self.loops_seen[n].pass
                    && loops::remove(&mut self.project, into) + audio(&mut self.project) > 0
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
                self.morph.a = a.min(ROWS - 1);
                self.morph.b = b.min(ROWS - 1);
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
            TimeAction::CloseReport => {
                // What the conversion did stays; only its card goes.
                self.report = None;
                self.changed = true;
                Ok(String::new())
            }
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
                let list = &self.project.arrangement.list;
                if list.linked {
                    // A section picked, and its scene with it if it has one.
                    let bar = self.info.meter.bar_ticks();
                    let parts = song_sections(&self.project, bar);
                    self.list_pick = k.filter(|&k| k < parts.len());
                    if let Some(from) = self.list_pick.and_then(|k| parts[k].scene) {
                        self.scene_pick = Some(from.row);
                    }
                } else {
                    self.list_pick = k.filter(|&k| k < list.steps.len());
                    if let Some(k) = self.list_pick {
                        self.scene_pick = Some(list.steps[k].row);
                    }
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
            TimeAction::PickPlacement(i) => {
                if i >= self.project.arrangement.placements.len() {
                    return Err("pick: no such block".into());
                }
                let mut set = self.picked();
                match set.iter().position(|&j| j == i) {
                    Some(k) => {
                        set.remove(k);
                    }
                    None => set.push(i),
                }
                self.set_picks(set);
                Ok(String::new())
            }
            TimeAction::PickPlacements { set, add } => {
                let blocks = self.project.arrangement.placements.len();
                let mut picked = if add { self.picked() } else { Vec::new() };
                picked.extend(set.into_iter().filter(|&i| i < blocks));
                self.set_picks(picked);
                Ok(String::new())
            }
            TimeAction::SelectPlacement(i) => {
                self.picks.clear();
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
                    self.picks.clear();
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
                    self.picks.clear();
                    self.section = None;
                    self.tempo_pick = None;
                }
                self.changed = true;
                Ok(String::new())
            }
            // The audio tracks' engine is the app's to wire (AudioHost);
            // until it takes these, say so rather than drop them quietly.
            TimeAction::Audio(a) => Err(format!("audio tracks: {a:?} is not wired yet")),
            // The fitted copies are the audio tracks' to make (the app,
            // `main::time_action`); the clip comes back through `fit_clip`.
            TimeAction::FitClip { .. } => Err("Fit to tempo goes through the audio tracks".into()),
            TimeAction::AudioClip {
                track,
                slot,
                row,
                set,
            } => self.set_audio_clip(track, slot, row, set),
            TimeAction::SongTempo(TempoEdit::Pick(k)) => {
                self.tempo_pick = k.filter(|&k| k < self.project.arrangement.tempo.len());
                if self.tempo_pick.is_some() {
                    self.selected = None;
                    self.picks.clear();
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
            TimeAction::CaptureScene => self.capture_scene(),
            TimeAction::SceneCard(None) => {
                self.scene_open = None;
                self.changed = true;
                Ok("scene card closed".into())
            }
            TimeAction::SceneCard(Some(row)) => {
                if row >= ROWS {
                    return Err(format!("no row {}", row + 1));
                }
                self.scene_pick = Some(row);
                self.scene_open = Some(row);
                self.changed = true;
                Ok(format!("scene card: scene {}", row + 1))
            }
            TimeAction::InsertRow(row) => {
                if row + 2 >= ROWS {
                    return Err(format!(
                        "clips: row {ROWS}, the grid's last, is the only row under it"
                    ));
                }
                if !self.project.row_empty(ROWS - 1) {
                    return Err(format!(
                        "clips: row {ROWS}, the last, is in use, so there's no room for another row: empty it first"
                    ));
                }
                // The grid's last row, empty, moves in under it.
                self.act_in(TimeAction::DropRow {
                    from: ROWS - 1,
                    to: row + 1,
                    copy: false,
                })?;
                Ok(format!(
                    "clips: an empty row {} under row {}, the rows under it down one (UNDO takes it back)",
                    row + 2,
                    row + 1
                ))
            }
            TimeAction::DeleteRow(row) => self.delete_row(row),
            TimeAction::SongToScenes => {
                // The rows change, not the song: one edit of whole rows,
                // which UNDO takes back. The last conversion's card closes.
                let bar = info.meter.bar_ticks();
                let before = self.project.clone();
                self.report = None;
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
                    self.report = None;
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
            | TimeAction::ListMove { .. }
            | TimeAction::ListBack(_)
            | TimeAction::ListLoop(_)
            | TimeAction::ListLink(_)
            | TimeAction::ListToSong
            | TimeAction::ScenesToSong
            | TimeAction::MakeUnique(_)
            | TimeAction::DeletePlacement(_)
            | TimeAction::SplitPlacement(_)
            | TimeAction::DuplicatePlacement(_)
            | TimeAction::TrimPlacement { .. }
            | TimeAction::SlidePlacement { .. }
            | TimeAction::MovePlacement { .. }
            | TimeAction::MovePlacements { .. }
            | TimeAction::DuplicatePlacements(_)
            | TimeAction::DeletePlacements(_)
            | TimeAction::SplitPlacements(_)
            | TimeAction::StretchPlacement { .. }
            | TimeAction::MoveSection { .. }
            | TimeAction::SongBars(_)
            | TimeAction::SongMarker(_) => {
                // Edits of the song alone, each one UNDO takes back. A
                // conversion closes the last one's card.
                let bar = info.meter.bar_ticks();
                let before = self.project.arrangement.clone();
                let converts = matches!(a, TimeAction::ListToSong | TimeAction::ScenesToSong);
                if converts {
                    self.report = None;
                }
                // + Bar pressed again on the same section joins its take:
                // presses in a row are one UNDO (not with the audio regions
                // changed in between: [`TimeControl::act_with_audio`]).
                let join = match a {
                    TimeAction::SongBars(BarsEdit::RoomBefore(k)) if !self.audio_apart => {
                        Some(ROOM_BEFORE_JOIN | k as u64)
                    }
                    _ => None,
                };
                // A time edit that moves audio regions is a take even with
                // the song unchanged ([`TimeControl::act_with_audio`]).
                let audio = self.audio_moves
                    && (matches!(
                        a,
                        TimeAction::MoveSection { .. }
                            | TimeAction::SongBars(_)
                            | TimeAction::ListLink(LinkFrom::List)
                    ) || edits_steps(&a));
                let list = edits_steps(&a)
                    || matches!(
                        a,
                        TimeAction::ListLoop(_) | TimeAction::ListToSong | TimeAction::ListLink(_)
                    );
                // The blocks picked together go with the edit, unless it
                // picks them again where they are now (6.5).
                let picks = std::mem::take(&mut self.picks);
                let edited = if list {
                    self.edit_list(a, bar)
                } else {
                    self.edit_song(a, bar)
                };
                let (line, change) = match edited {
                    Ok(done) => done,
                    Err(e) => {
                        self.project.arrangement = before;
                        self.picks = picks;
                        return Err(e);
                    }
                };
                // An edit inside a section that followed its scene makes it
                // the song's own (4b.3); a linked list's edits say what
                // they make of their sections themselves.
                let line = if list {
                    line
                } else {
                    self.mark_own(&before, bar, line, "song: ")
                };
                if let Err(e) = self.compile() {
                    self.project.arrangement = before;
                    self.picks = picks;
                    if converts {
                        self.report = None;
                    }
                    self.compile()?;
                    return Err(e);
                }
                if let Some(c) = change.filter(|_| self.project.arrangement != before || audio) {
                    self.takes.song_edit_joining(&self.project, c, before, join);
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

    /// The blocks picked on the Song view: the set picked together, else
    /// the one selected, else none (6.5).
    fn picked(&self) -> Vec<usize> {
        if self.picks.is_empty() {
            self.selected.into_iter().collect()
        } else {
            self.picks.clone()
        }
    }

    /// Blocks `set` picked (6.5): one is the block selected, as a tap
    /// picks it, and two or more are picked together, none selected. The
    /// region, section or tempo point picked is let go.
    fn set_picks(&mut self, mut set: Vec<usize>) {
        set.sort_unstable();
        set.dedup();
        self.selected = None;
        self.picks.clear();
        match set.len() {
            0 => {}
            1 => self.selected = Some(set[0]),
            _ => self.picks = set,
        }
        self.region = None;
        self.section = None;
        self.tempo_pick = None;
        self.changed = true;
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

    /// After an edit of the song that isn't a linked list's own (`before`
    /// the song before it, bars of `bar` ticks): the sections it changed
    /// inside that followed their scenes are the song's own now
    /// ([`Arrangement::mark_changed`], the Arrange refactor's 4b.3). On a
    /// linked list the log line (`line`) says so instead, after `head`,
    /// and where to take it back.
    fn mark_own(&mut self, before: &Arrangement, bar: Tick, line: String, head: &str) -> String {
        let song = &mut self.project.arrangement;
        let marked = song.mark_changed(&self.project.tracks, before, bar);
        let names: Vec<&str> = marked
            .iter()
            .map(|&k| song.markers[k].name.as_str())
            .collect();
        let who = match names.split_last() {
            _ if !song.list.linked => return line,
            None => return line,
            Some((last, [])) => format!("{last} is"),
            Some((last, rest)) => format!("{} and {last} are", rest.join(", ")),
        };
        format!(
            "{head}{who} the song's own now: Back to the scene on the song list puts the scene's clips back"
        )
    }

    /// Live to song: the passes recorded so far into the song, compiled
    /// in, as a song edit UNDO takes back (audio tracks phase 2, step 2d).
    /// The blocks it put on audio tracks' lanes wait for the audio tracks
    /// ([`TimeControl::take_song_placed`]): the regions under them go, as
    /// a step tied to this edit. A line for the log.
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
        self.picks.clear();
        self.section = None;
        // Recorded over a section that followed its scene: the song's own
        // now (4b.3).
        let mut line = format!(
            "{} placements and {} morph points into the song",
            r.placements, r.morph_points
        );
        // The XY pad's moves (phase 3): said only when it moved something.
        if r.moves + r.moves_lost > 0 {
            let s = if r.moves == 1 { "" } else { "s" };
            line.push_str(&format!(", {} XY pad move{s}", r.moves));
            if r.moves_lost > 0 {
                line.push_str(&format!(
                    " ({} left out: no room in the song's lanes)",
                    r.moves_lost
                ));
            }
        }
        let line = self.mark_own(&before, self.info.meter.bar_ticks(), line, "");
        if let Err(e) = self.compile() {
            self.project.arrangement = before;
            self.compile()?;
            return Err(e);
        }
        let placed = audio_placed(&self.project, &before);
        if self.project.arrangement != before {
            let change = SongChange::RecSong {
                placements: r.placements,
            };
            self.takes
                .song_edit_joining(&self.project, change, before, None);
            self.changed = true;
            if !placed.is_empty() {
                let n = self
                    .takes
                    .done()
                    .last()
                    .map_or(mpfree_transport::TransactionId::NONE, |t| t.txn);
                self.song_placed = Some((n, placed));
            }
        }
        Ok(line)
    }

    /// What Rec song last put on audio tracks' lanes, once: its song
    /// edit's number and each block's span, by the track's lasting id,
    /// in order, joined where they meet. The audio tracks replace the
    /// regions under them as a step of their own tagged with the number,
    /// and say so with [`TimeControl::tie_audio`].
    pub fn take_song_placed(&mut self) -> Option<super::SongPlaced> {
        self.song_placed.take()
    }

    /// The latest take's transaction (none before the first), for tests.
    #[cfg(test)]
    pub(crate) fn last_txn(&self) -> mpfree_transport::TransactionId {
        self.takes
            .done()
            .last()
            .map_or(mpfree_transport::TransactionId::NONE, |t| t.txn)
    }

    /// The audio tracks made a step tied to song edit `n` (the regions
    /// under Rec song's blocks replaced): its UNDO and REDO take that step
    /// back and bring it back, through [`TimeControl::act_with_audio`].
    /// False when the song's history no longer holds `n`.
    pub fn tie_audio(&mut self, n: mpfree_transport::TransactionId) -> bool {
        let held = self.song_reaches(n, true) || self.song_reaches(n, false);
        if held {
            self.audio_edits.entry(n).or_default();
        }
        held
    }

    /// The Song view's edits of the arrangement (and, for Song → scenes,
    /// the rows). A line for the log, and what changed for the undo list.
    /// Song → scenes ([`song_to_scenes`]) with the song's regions, when
    /// [`TimeControl::act_with_audio`] has asked the audio tracks for
    /// them, along the tempo map the song plays them on.
    fn song_to_scenes(
        &mut self,
        bar: Tick,
    ) -> Result<mpfree_seq::arrange::ToScenes, ScenesRefused> {
        let song = &self.project.arrangement;
        // As the transport plays it in song mode: its tempo points, or the
        // tempo throughout when there are none.
        let map = if song.tempo.is_empty() {
            TempoMap::constant(self.info.bpm)
        } else {
            song.tempo_map(self.project_bpm).ok()
        }
        .or_else(|| TempoMap::constant(self.project_bpm))
        .ok_or(ScenesRefused::Model(ModelError::BadPlacement))?;
        let regions = std::mem::take(&mut self.song_regions);
        let sound = SongSound {
            regions: &regions,
            map,
            rate: self.rate,
        };
        let r = song_to_scenes(&mut self.project, bar, &sound);
        self.song_regions = regions;
        r
    }

    /// Track `track` in song ticks `from..to`, in bars of `bar` ticks, as
    /// a refusal names it: "Vocal, bars 9 to 16".
    fn lane_bars(&self, track: usize, from: Tick, to: Tick, bar: Tick) -> String {
        let bar = bar.max(1);
        let name = self
            .project
            .tracks
            .get(track)
            .map_or_else(|| format!("Track {}", track + 1), |t| t.name.clone());
        let (first, last) = (from / bar + 1, ((to + bar - 1) / bar).max(from / bar + 1));
        if last > first {
            format!("{name}, bars {first} to {last}")
        } else {
            format!("{name}, bar {first}")
        }
    }

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
                self.picks.clear();
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
        let bpm = self.project_bpm;
        let playhead = self.info.position;
        let tracks = &self.project.tracks;
        let song = &mut self.project.arrangement;
        let named = |song: &mpfree_seq::arrange::Arrangement, i: usize| {
            song.placements
                .get(i)
                .and_then(|p| song.clip_of(tracks, p))
                .map_or_else(|| "the block".to_string(), |c| c.name.clone())
        };
        let track_name = |t: usize| tracks.get(t).map_or_else(String::new, |t| t.name.clone());
        let audio = |t: usize| tracks.get(t).is_some_and(|t| t.is_audio());
        let spans = &self.audio_spans;
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
                // A loop cell's block on an audio track (phase 4e.3).
                let looped = song
                    .placements
                    .get(index)
                    .and_then(|p| song.clip_of(tracks, p))
                    .is_some_and(|c| c.loop_clip.is_some());
                let j = song
                    .move_placement(tracks, index, track, start, copy)
                    .map_err(|e| match e {
                        // A loop's block stays on its Loop track's lane
                        // (loop-tracks.md part 5), or its audio track's.
                        ModelError::BadLoop | ModelError::AudioTrack if looped => format!(
                            "song: {name} stays on its own lane: a loop plays on one track, so its block can't go to another"
                        ),
                        ModelError::BadLoop => format!(
                            "song: {name} stays on its own lane: a loop's block can't go to another track, nor a clip's onto a Loop track"
                        ),
                        // An audio clip's block stays on audio tracks'
                        // lanes, and a note clip's off them (phase 2,
                        // step 2d).
                        ModelError::AudioTrack if was.is_some_and(audio) => format!(
                            "song: {name} stays where it was: an audio clip's block goes only on an audio track, and {} isn't one",
                            track_name(track)
                        ),
                        ModelError::AudioTrack => format!(
                            "song: {name} stays where it was: {} is an audio track, and it takes blocks of audio clips only",
                            track_name(track)
                        ),
                        _ => format!(
                            "song: {name} doesn't fit on {} at {}: it would run into another block",
                            track_name(track),
                            at(start)
                        ),
                    })?;
                // Never over a recording on an audio track's lane.
                // (the approved board's words, layout spec 4c).
                if let Some(over) = region_under(spans, tracks, &song.placements[j]) {
                    return Err(format!(
                        "song: {name} can't go over the region at {}: a block and a region never overlap on a lane",
                        at(over)
                    ));
                }
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
            TimeAction::SplitPlacement(Some(i)) => {
                let name = named(song, i);
                let track = song
                    .placements
                    .get(i)
                    .map(|p| p.track)
                    .ok_or("split: no such block")?;
                let j = song.split(tracks, i, playhead).map_err(|_| {
                    format!("song: move the playhead inside {name} (Split cuts at the playhead)")
                })?;
                // The part after is picked: Split again cuts it further.
                self.selected = Some(j);
                Ok((
                    format!(
                        "song: {name} split at {} (UNDO takes it back)",
                        at(playhead)
                    ),
                    Some(SongChange::Split {
                        track: Some(track),
                        at: playhead,
                    }),
                ))
            }
            TimeAction::SplitPlacement(None) => {
                let n = song
                    .split_all(tracks, playhead)
                    .map_err(|_| "song: too many blocks to split them all".to_string())?;
                if n == 0 {
                    return Err(format!(
                        "song: no block plays across {} (Split cuts at the playhead)",
                        at(playhead)
                    ));
                }
                let blocks = if n == 1 { "block" } else { "blocks" };
                Ok((
                    format!(
                        "song: {n} {blocks} split at {} (UNDO takes it back)",
                        at(playhead)
                    ),
                    Some(SongChange::Split {
                        track: None,
                        at: playhead,
                    }),
                ))
            }
            TimeAction::DuplicatePlacement(i) => {
                let name = named(song, i);
                if i >= song.placements.len() {
                    return Err("duplicate: no such block".into());
                }
                let j = song.duplicate(i).map_err(|_| {
                    format!("song: no room after {name} for its copy: a block on its lane is in the way")
                })?;
                let (track, start) = (song.placements[j].track, song.placements[j].start);
                self.selected = Some(j);
                self.section = None;
                Ok((
                    format!(
                        "song: a copy of {name} at {} (UNDO takes it back)",
                        at(start)
                    ),
                    Some(SongChange::Copied { track, start }),
                ))
            }
            TimeAction::TrimPlacement { index, start } => {
                let name = named(song, index);
                let track = song
                    .placements
                    .get(index)
                    .map(|p| p.track)
                    .ok_or("trim: no such block")?;
                song.trim_start(tracks, index, start).map_err(|_| {
                    format!(
                        "song: {name} can't start at {}: it would run into the block before, or past its own end",
                        at(start)
                    )
                })?;
                // Never back over a recording on an audio track's lane
                // (phase 4e.3: a loop block's or a take clip's).
                if let Some(over) = region_under(spans, tracks, &song.placements[index]) {
                    return Err(format!(
                        "song: {name} can't start at {}: it would go over the region at {}, and a block and a region never overlap on a lane",
                        at(start),
                        at(over)
                    ));
                }
                Ok((
                    format!(
                        "song: {name} starts at {} now, what it plays where it was (UNDO takes it back)",
                        at(start)
                    ),
                    Some(SongChange::Trimmed { track, start }),
                ))
            }
            TimeAction::SlidePlacement { index, by } => {
                let name = named(song, index);
                let (track, start) = song
                    .placements
                    .get(index)
                    .map(|p| (p.track, p.start))
                    .ok_or("slide: no such block")?;
                if by == 0 {
                    return Ok((format!("song: {name} plays as it did"), None));
                }
                song.slide(tracks, index, by)
                    .map_err(|_| "slide: no such block".to_string())?;
                let beats = by.unsigned_abs() as f64 / meter.beat_ticks().max(1) as f64;
                let way = if by > 0 { "later" } else { "earlier" };
                let amount = if beats.fract() == 0.0 {
                    let n = beats as u64;
                    format!("{n} beat{}", if n == 1 { "" } else { "s" })
                } else {
                    format!("{beats:.2} beats")
                };
                Ok((
                    format!("song: what {name} plays slid {amount} {way} (UNDO takes it back)"),
                    Some(SongChange::Slid { track, start }),
                ))
            }
            TimeAction::StretchPlacement { index, length } => {
                let name = named(song, index);
                let track = song.placements.get(index).map_or(0, |p| p.track);
                song.set_length(index, length).map_err(|_| {
                    format!("song: {name} can't be that long: it would run into the next block")
                })?;
                if let Some(over) = region_under(spans, tracks, &song.placements[index]) {
                    return Err(format!(
                        "song: {name} can't run over the region at {}: a block and a region never overlap on a lane",
                        at(over)
                    ));
                }
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
                // Its tempo, morph moves and rows go with it (Arrange 3.2).
                song.move_section(tracks, marker, gap, copy, bar, bpm)
                    .map_err(|r| match r {
                        Refused::BadRange => format!("song: {name} can't go there"),
                        r => format!("song: {name} can't go there: {}", refused_said(r, at)),
                    })?;
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
                self.picks.clear();
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
            TimeAction::SongBars(e) => {
                let b = bar.max(1);
                let (edit, from, len, inserted) = bars_edit(song, e, b)?;
                let n = if len % b == 0 {
                    format!("{}", len / b)
                } else {
                    format!("{:.1}", len as f64 / b as f64)
                };
                let bars = if n == "1" { "bar" } else { "bars" };
                // The bars taken out, by number: "bar 9", "bars 9 to 10".
                let span = if from % b == 0 && len % b == 0 && len > 0 {
                    let (first, last) = (from / b + 1, (from + len) / b);
                    if first == last {
                        format!("bar {first}")
                    } else {
                        format!("bars {first} to {last}")
                    }
                } else {
                    format!("{} to {}", at(from), at(from + len))
                };
                *song = song.edited(tracks, edit, bpm).map_err(|r| {
                    let what = if inserted {
                        format!("{n} {bars} can't go in at {}", at(from))
                    } else if len <= 0 {
                        format!("there are no bars to take out at {}", at(from))
                    } else {
                        format!("{span} can't come out")
                    };
                    match r {
                        Refused::BadRange => format!("song: {what}"),
                        r => format!("song: {what}: {}", refused_said(r, at)),
                    }
                })?;
                // + Bar keeps its section picked (the markers keep their
                // order); anything else may have moved what the picks name.
                self.section = match e {
                    BarsEdit::RoomBefore(k) => Some(k),
                    _ => None,
                };
                self.selected = None;
                self.picks.clear();
                self.tempo_pick = None;
                Ok((
                    if inserted {
                        format!(
                            "song: {n} empty {bars} at {}; everything after moved on (UNDO takes them out)",
                            at(from)
                        )
                    } else {
                        format!(
                            "song: {span} taken out; everything after moved back (UNDO puts them back)"
                        )
                    },
                    Some(SongChange::Bars {
                        at: from,
                        len,
                        inserted,
                    }),
                ))
            }
            TimeAction::SongMarker(e) => {
                let marker = |song: &mpfree_seq::arrange::Arrangement, k: usize| {
                    song.markers
                        .get(k)
                        .map(|m| (m.at, m.name.clone()))
                        .ok_or_else(|| "song: there's no such marker".to_string())
                };
                match e {
                    MarkerEdit::Add(t) => {
                        let name = song.new_marker_name();
                        let k = song.add_marker(t, &name).map_err(|_| {
                            if t < 0 {
                                "song: a marker can't go before the song's start".to_string()
                            } else if song.markers.iter().any(|m| m.at == t) {
                                format!("song: there's a marker at {} already", at(t))
                            } else {
                                "song: the song has as many markers as it can hold".to_string()
                            }
                        })?;
                        self.section = Some(k);
                        Ok((
                            format!("song: marker {name} at {} (UNDO takes it away)", at(t)),
                            Some(SongChange::Marker { at: t }),
                        ))
                    }
                    MarkerEdit::Rename { index, name } => {
                        let (t, was) = marker(song, index)?;
                        let name = name.trim().to_string();
                        song.rename_marker(index, &name)
                            .map_err(|_| format!("song: {was} needs a name"))?;
                        Ok((
                            format!("song: {was} is {name} now (UNDO gives its name back)"),
                            Some(SongChange::Marker { at: t }),
                        ))
                    }
                    MarkerEdit::Move { index, at: t } => {
                        let (_, name) = marker(song, index)?;
                        let k = song.move_marker(index, t).map_err(|_| {
                            if t < 0 {
                                format!("song: {name} can't go before the song's start")
                            } else {
                                format!("song: {name} can't go onto another marker at {}", at(t))
                            }
                        })?;
                        self.section = Some(k);
                        Ok((
                            format!(
                                "song: {name} moved to {}; only where its section starts moved (UNDO takes it back)",
                                at(t)
                            ),
                            Some(SongChange::Marker { at: t }),
                        ))
                    }
                    MarkerEdit::Delete(index) => {
                        let (t, name) = marker(song, index)?;
                        song.remove_marker(index)
                            .map_err(|_| "song: there's no such marker".to_string())?;
                        self.section = None;
                        let joins = index
                            .checked_sub(1)
                            .and_then(|k| song.markers.get(k))
                            .map_or_else(
                                || "what comes before the first marker".to_string(),
                                |m| m.name.clone(),
                            );
                        Ok((
                            format!(
                                "song: marker {name} taken away; its bars join {joins} (UNDO puts it back)"
                            ),
                            Some(SongChange::Marker { at: t }),
                        ))
                    }
                }
            }
            TimeAction::ScenesToSong => {
                // Every row with a clip, in order, once each, after the
                // song's end (every row the project keeps, not only the
                // grid's).
                let rows: Vec<ListStep> = (0..ROWS)
                    .filter(|&r| self.project.row_filled(r))
                    .map(|row| ListStep { row, times: 1 })
                    .collect();
                if rows.is_empty() {
                    return Err("scenes → song: no row has a clip".into());
                }
                let (at, how) = self
                    .rows_to_song(
                        &rows,
                        bar,
                        "SCENES → SONG",
                        "The song plays the rows, except:",
                    )
                    .map_err(|e| format!("scenes → song: {e}"))?;
                Ok((
                    format!(
                        "scenes → song: {} rows from bar {}{how}",
                        rows.len(),
                        at / bar.max(1) + 1
                    ),
                    Some(SongChange::Scenes { rows: rows.len() }),
                ))
            }
            TimeAction::SongToScenes => match self.song_to_scenes(bar) {
                // Nothing to make: said on the screen as a refusal is.
                Ok(done) if done.rows.is_empty() => Err("song → scenes: the song is empty".into()),
                Ok(done) => {
                    let how = conversion_said(&self.project, &done.differences, bar);
                    let rows = &done.rows;
                    let made = rows_named(rows[0], rows[rows.len() - 1]);
                    self.report = report(
                        "SONG → SCENES",
                        made.clone(),
                        format!(
                            "Launch row {} and the rows play the song, except:",
                            rows[0] + 1
                        ),
                        &done.differences,
                        bar,
                    );
                    Ok((format!("song → scenes: {made}{how}"), None))
                }
                Err(ScenesRefused::Model(ModelError::NoRoom)) => {
                    Err("song → scenes: not enough empty rows in a run".into())
                }
                Err(ScenesRefused::Model(e)) => Err(format!("song → scenes: {e:?}")),
                Err(ScenesRefused::Regions {
                    track,
                    from,
                    to,
                    regions,
                }) => Err(format!(
                    "song → scenes: nothing changed: {}: {regions} regions, and a clip holds {}. Split the section, or bounce it first.",
                    self.lane_bars(track, from, to, bar),
                    mpfree_seq::MAX_CLIP_REGIONS
                )),
                Err(ScenesRefused::Both { track, from, to }) => Err(format!(
                    "song → scenes: nothing changed: {} has recordings and a block, and a cell holds one clip",
                    self.lane_bars(track, from, to, bar)
                )),
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
            TimeAction::MovePlacements {
                set,
                lanes,
                by,
                copy,
            } => {
                let n = distinct(&set);
                let what = blocks_said(n);
                let moved = song
                    .move_placements(tracks, &set, lanes, by, copy)
                    .map_err(|e| match e {
                        ModelError::BadLoop => format!(
                            "song: the {what} stay where they are: a loop's block can't go to another lane (a loop plays on one track), nor a clip's onto a Loop track's"
                        ),
                        ModelError::AudioTrack => format!(
                            "song: the {what} stay where they are: one would cross between a note track's lane and an audio track's, and a block keeps to its kind of lane"
                        ),
                        _ => format!(
                            "song: the {what} don't fit there: one would run into another block, or leave the song"
                        ),
                    })?;
                // None over a recording on an audio track's lane (4e.3).
                if let Some(over) = moved
                    .iter()
                    .find_map(|&i| region_under(spans, tracks, song.placements.get(i)?))
                {
                    let stay = if n == 1 {
                        "stays where it is"
                    } else {
                        "stay where they are"
                    };
                    return Err(format!(
                        "song: the {what} {stay}: one would go over the region at {}, and a block and a region never overlap on a lane",
                        at(over)
                    ));
                }
                // They stay picked where they are now (the copies, for a
                // copy), for the wheel or another drag.
                self.set_picks(moved);
                let lanes_said = match lanes {
                    0 => String::new(),
                    1 | -1 => format!(" a lane {}", if lanes > 0 { "down" } else { "up" }),
                    _ => format!(
                        " {} lanes {}",
                        lanes.unsigned_abs(),
                        if lanes > 0 { "down" } else { "up" }
                    ),
                };
                let time_said = if by == 0 {
                    String::new()
                } else {
                    format!(
                        " {} {}",
                        ticks_said(by.abs(), meter),
                        if by > 0 { "later" } else { "earlier" }
                    )
                };
                let and = if lanes_said.is_empty() || time_said.is_empty() {
                    ""
                } else {
                    " and"
                };
                Ok(if copy {
                    (
                        format!("song: copies of {n} {what}{lanes_said}{and}{time_said} (UNDO takes them back)"),
                        Some(SongChange::SeveralCopied { blocks: n, lanes, by }),
                    )
                } else {
                    (
                        format!("song: {n} {what} moved{lanes_said}{and}{time_said} (UNDO takes it back)"),
                        Some(SongChange::SeveralMoved { blocks: n, lanes, by }),
                    )
                })
            }
            TimeAction::DuplicatePlacements(set) => {
                let n = distinct(&set);
                let what = blocks_said(n);
                let blocks: Vec<&mpfree_seq::arrange::Placement> =
                    set.iter().filter_map(|&i| song.placements.get(i)).collect();
                let by = blocks.iter().map(|p| p.end()).max().unwrap_or(0)
                    - blocks.iter().map(|p| p.start).min().unwrap_or(0);
                let copies = song.duplicate_placements(tracks, &set).map_err(|_| {
                    format!(
                        "song: no room after the {what} for their copies: a block is in the way"
                    )
                })?;
                // None over a recording on an audio track's lane (4e.3).
                if let Some(over) = copies
                    .iter()
                    .find_map(|&i| region_under(spans, tracks, song.placements.get(i)?))
                {
                    return Err(format!(
                        "song: no room after the {what} for their copies: one would go over the region at {}, and a block and a region never overlap on a lane",
                        at(over)
                    ));
                }
                // The copies are picked: Duplicate again repeats them on.
                self.set_picks(copies);
                Ok((
                    format!("song: copies of {n} {what} right after them (UNDO takes them back)"),
                    Some(SongChange::SeveralCopied {
                        blocks: n,
                        lanes: 0,
                        by,
                    }),
                ))
            }
            TimeAction::SplitPlacements(set) => {
                let after = song.split_these(tracks, &set, playhead).map_err(|_| {
                    format!(
                        "song: none of the {} picked plays across {} (Split cuts at the playhead)",
                        blocks_said(distinct(&set)),
                        at(playhead)
                    )
                })?;
                let n = after.len();
                // The parts after are picked: Split again cuts them further.
                self.set_picks(after);
                Ok((
                    format!(
                        "song: {n} {} split at {} (UNDO takes it back)",
                        blocks_said(n),
                        at(playhead)
                    ),
                    Some(SongChange::SeveralSplit {
                        blocks: n,
                        at: playhead,
                    }),
                ))
            }
            TimeAction::DeletePlacements(set) => {
                let n = song
                    .remove_placements(&set)
                    .map_err(|_| "delete: no such blocks".to_string())?;
                self.selected = None;
                self.picks.clear();
                Ok((
                    format!("song: {n} {} deleted (UNDO puts them back)", blocks_said(n)),
                    Some(SongChange::SeveralDeleted { blocks: n }),
                ))
            }
            TimeAction::DeletePlacement(i) => {
                let p = song.remove(i).ok_or("delete: no such placement")?;
                self.selected = None;
                self.picks.clear();
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

    /// A song list edit: a step's times or scene, a step put in, taken
    /// out or moved, the loop switch, or List → song. A linked list's
    /// step edits are the song's ([`linked`]), and it needs no List →
    /// song. A line for the log, and what changed for the take UNDO takes
    /// back.
    fn edit_list(
        &mut self,
        a: TimeAction,
        bar: Tick,
    ) -> Result<(String, Option<SongChange>), String> {
        if let TimeAction::ListLink(from) = a {
            return self.link(from, bar);
        }
        if self.project.arrangement.list.linked {
            if edits_steps(&a) {
                return self.edit_linked(&a, bar);
            }
            if a == TimeAction::ListToSong {
                return Err("list → song: the list is the song's parts already".into());
            }
        }
        let filled: Vec<usize> = (0..ROWS).filter(|&r| self.project.row_filled(r)).collect();
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
            TimeAction::ListMove { step, to } => {
                if step >= n || to > n {
                    return Err("song list: no such step".into());
                }
                if to == step || to == step + 1 {
                    return Ok((
                        format!("song list: step {} is already there", step + 1),
                        None,
                    ));
                }
                let now = if to > step { to - 1 } else { to };
                let s = list.steps.remove(step);
                list.steps.insert(now, s);
                self.perform.list_moved(step, now);
                self.list_pick = Some(now);
                Ok((
                    format!(
                        "song list: step {} is step {} now (UNDO takes it back)",
                        step + 1,
                        now + 1
                    ),
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
            TimeAction::ListBack(_) => {
                Err("song list: Back to the scene is for a list linked to the song".into())
            }
            TimeAction::ListToSong => {
                if n == 0 {
                    return Err("list → song: the list is empty".into());
                }
                let steps = self.project.arrangement.list.steps.clone();
                let (at, how) = self
                    .rows_to_song(
                        &steps,
                        bar,
                        "LIST → SONG",
                        "The song plays the list, except:",
                    )
                    .map_err(|e| format!("list → song: {e}"))?;
                Ok((
                    format!(
                        "list → song: {n} steps from bar {}{how}",
                        at / bar.max(1) + 1
                    ),
                    Some(SongChange::ListToSong { steps: n }),
                ))
            }
            _ => Ok((String::new(), None)),
        }
    }

    /// Scenes → song and List → song: `steps` played offline through Rec
    /// song ([`super::offline`], plan D7) from the first bar line after the
    /// song's end, and what Rec song recorded put in the song, with each
    /// step's scene's name above where it started. Where they start, and
    /// how the song plays differently from the rows (the end of the log
    /// line: [`conversion_said`]), or why nothing changed; the report card
    /// opens when it differs.
    fn rows_to_song(
        &mut self,
        steps: &[ListStep],
        bar: Tick,
        title: &str,
        lead: &str,
    ) -> Result<(Tick, String), String> {
        // After the song's end, and on a linked list after its last part
        // (one that follows its scene may run on past its blocks), so no
        // part is cut. The song's end is its audio tracks' recordings' too,
        // so the new blocks never land over a region (audio tracks phase 2,
        // step 2d).
        let recorded = self
            .audio_spans
            .iter()
            .filter_map(|(_, spans)| spans.last().map(|s| s.1))
            .max()
            .unwrap_or(0);
        let song = &self.project.arrangement;
        let parts = if song.list.linked {
            song.parts(bar).last().map_or(0, |s| s.end)
        } else {
            0
        };
        let end = song.end().max(parts).max(recorded);
        let at = (end + bar - 1) / bar.max(1) * bar;
        let played = super::offline::play_rows(&self.project, steps, at, bar)?;
        let made = format!(
            "bars {} to {}",
            at / bar.max(1) + 1,
            (played.end + bar - 1) / bar.max(1)
        );
        let mut song = self.project.arrangement.clone();
        let mut differences = Vec::new();
        for p in played.placements {
            song.place(p).map_err(|e| format!("{e:?}"))?;
        }
        for &(t, row) in &played.starts {
            let scene = self.project.scene(row);
            // The one thing the song doesn't play (plan D7).
            if scene.is_some_and(|s| !s.programs.is_empty()) {
                differences.push(Difference {
                    track: None,
                    at: t,
                    what: Differs::SceneProgram,
                });
            }
            // A scene's tempo (5.2b): a tempo point where its step starts,
            // so song mode changes tempo where the rows did.
            if let Some(bpm) = scene.and_then(|s| s.bpm()) {
                if !put_scene_tempo(&mut song, t, bpm, Some(self.project_bpm)) {
                    differences.push(Difference {
                        track: None,
                        at: t,
                        what: Differs::SceneTempo,
                    });
                }
            }
            song.mark(t, &scene_title(&self.project, row))
                .map_err(|e| format!("{e:?}"))?;
        }
        song.check(&self.project.tracks)
            .map_err(|e| format!("{e:?}"))?;
        self.project.arrangement = song;
        // New placements shift the indices.
        self.selected = None;
        self.picks.clear();
        self.section = None;
        self.report = report(title, made, lead.into(), &differences, bar);
        Ok((at, conversion_said(&self.project, &differences, bar)))
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
                let (a, b) = (a.min(ROWS - 1), b.min(ROWS - 1));
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
            // A new clip holds what a take there would (refactor E4).
            let new = self.take_holds(track);
            let r = match kept.first().map(|k| k.take) {
                None => keep(
                    &part,
                    &mut self.project,
                    &self.launches,
                    &mut self.takes,
                    track,
                    now,
                    &name,
                    new,
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
                        new,
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
            // A clip holds pads or notes, never both (refactor E4).
            if k.left_out > 0 {
                let (n, s) = (k.left_out, if k.left_out == 1 { "" } else { "s" });
                pressure += &match k.holds {
                    mpfree_seq::Holds::Notes => {
                        format!("; {n} pad hit{s} left out: no note played there")
                    }
                    mpfree_seq::Holds::Hits => {
                        format!(
                            "; {n} note{s} left out: the clip holds pads, and no pad played them"
                        )
                    }
                };
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
            KeepError::OtherKind(mpfree_seq::Holds::Notes) => {
                format!("{name} left out: its clip holds notes, and the pads played none there")
            }
            KeepError::OtherKind(mpfree_seq::Holds::Hits) => {
                format!("{name} left out: its clip holds pads, and no pad played its notes")
            }
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
        let slot = (0..ROWS)
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
            let slot = (0..ROWS)
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

    /// Convert to notes' preview (step sequencer refactor E1,
    /// `mpfree_seq::convert`): what converting clip `slot` of `track`, one
    /// that follows the pads, would make through the pads as they play
    /// now, or every reason it can't, STOP's too; nothing changes. It stays
    /// open, its Convert checking the clip and the pads again, until
    /// Cancel, Convert or another action. A kit's clip, a loop's cell or a
    /// clip of notes opens none.
    fn convert_preview(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let c = self.project.clip(track, slot).ok_or_else(|| {
            format!(
                "convert to notes: slot {} of track {} is empty",
                slot + 1,
                track + 1
            )
        })?;
        let name = c.name.clone();
        if c.looped.is_some() {
            return Err("convert to notes: a loop's cell holds a loop, not notes".to_string());
        }
        if c.holds == mpfree_seq::Holds::Notes {
            return Err(format!(
                "convert to notes: {name} uses notes already: each keeps its pitch, whatever the pads play"
            ));
        }
        if self.take_holds(track) == mpfree_seq::Holds::Hits {
            return Err(format!(
                "convert to notes: {name} is a kit's clip, and a kit's clips always follow the pads"
            ));
        }
        let rows = self.pad_rows.get(track).copied().unwrap_or(PadRows::PADS);
        let made = c.convert_to_notes(|p| rows.notes(usize::from(p)));
        let line = match &made {
            Ok(m) => {
                let s = |n: usize| if n == 1 { "" } else { "s" };
                let mut line = format!(
                    "Convert to notes: {} pad hit{} become{} {} note{}. Keeps what {name} plays now with these pads ({})",
                    m.hits,
                    s(m.hits),
                    if m.hits == 1 { "s" } else { "" },
                    m.notes,
                    s(m.notes),
                    pads_range(&rows)
                );
                if self.info.state.rolling() {
                    line.push_str(". Press STOP first, then Convert");
                }
                line
            }
            Err(why) => {
                let grid = grid_of(c);
                let mut words: Vec<String> = why
                    .iter()
                    .map(|r| refused_words(r, grid, c.length))
                    .collect();
                if self.info.state.rolling() {
                    words.push("press STOP first".to_string());
                }
                format!(
                    "Convert to notes can't keep what {name} plays yet: {}. Cancel closes this",
                    words.join("; ")
                )
            }
        };
        self.convert_asked = Some(ConvertAsked {
            track,
            slot,
            clip: c.clone(),
            rows,
            made: made.map_err(|_| ()),
            line: line.clone(),
        });
        self.changed = true;
        Ok(line)
    }

    /// Convert to notes (E1), after its preview: clip `slot` of `track`
    /// becomes what the preview showed, its notes, locks and the original
    /// Restore brings back together, as one edit UNDO takes back. Refused,
    /// and nothing changes, while the transport rolls or if the clip or
    /// the pads changed since the preview.
    fn convert(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let asked = match self.convert_asked.take() {
            Some(a) if (a.track, a.slot) == (track, slot) => a,
            other => {
                self.convert_asked = other;
                return Err(
                    "convert to notes: › on the Melody row shows what it would do first"
                        .to_string(),
                );
            }
        };
        self.changed = true;
        if self.project.clip(track, slot) != Some(&asked.clip) {
            return Err("convert to notes: the clip changed since the preview: look again".into());
        }
        if self.pad_rows.get(track) != Some(&asked.rows) {
            return Err("convert to notes: the pads changed since the preview: look again".into());
        }
        if self.info.state.rolling() {
            let line = "convert to notes: press STOP first, then Convert".to_string();
            self.convert_asked = Some(asked);
            return Err(line);
        }
        let Ok(made) = asked.made else {
            return Err("convert to notes: the preview named what to fix first".into());
        };
        let (hits, notes) = (made.hits, made.notes);
        self.replace(track, slot, made.clip, Replaced::Convert)?;
        let s = |n: usize| if n == 1 { "" } else { "s" };
        Ok(format!(
            "Converted: {hits} pad hit{} {} {notes} note{} (UNDO takes it back)",
            s(hits),
            if hits == 1 { "is" } else { "are" },
            s(notes)
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
            Replaced::Fit => "fit",
            Replaced::Settings => "settings",
            Replaced::Convert => "convert to notes",
            Replaced::Continue { .. } => "continue",
            Replaced::FitChords => "fit to the new chords",
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

    /// Continue's touches (composing tools, phase 1; [`super::compose`]).
    fn continue_action(&mut self, a: ContinueAction) -> Result<String, String> {
        let recording = matches!(self.info.state, State::Recording | State::CountIn);
        match a {
            ContinueAction::Ask(ask) => {
                let ContinueAsk {
                    track,
                    slot,
                    settings,
                    kind,
                    lanes,
                    scope,
                    locked,
                } = *ask;
                use mpfree_compose::{InstrumentKind, Refused};
                use mpfree_context::{PartRole, ReadBudget};
                if recording {
                    return Err("continue: not while recording".into());
                }
                let c = self.project.clip(track, slot).ok_or_else(|| {
                    format!(
                        "continue: slot {} of track {} is empty",
                        slot + 1,
                        track + 1
                    )
                })?;
                if c.looped.is_some() {
                    return Err("continue: a loop's cell has no notes to carry on".into());
                }
                // A clip of pad hits carries on as a beat (composing step
                // 4): on a kit, never as a tune.
                if c.holds != Holds::Notes && kind != InstrumentKind::Percussive {
                    return Err(format!("continue: {}", Refused::PadHits.words()));
                }
                let name = c.name.clone();
                let role = match kind {
                    InstrumentKind::Bass => PartRole::Bass,
                    InstrumentKind::Chords => PartRole::Chords,
                    InstrumentKind::Percussive => PartRole::Drums,
                    InstrumentKind::OneNote | InstrumentKind::Chordal => PartRole::Melody,
                };
                let ask = mpfree_compose::Ask {
                    track,
                    slot,
                    role,
                    kind,
                    others: self.row_others(track, slot),
                    lanes,
                    transport_swing: self.info.swing,
                    meter: self.info.meter,
                    bpm: self.info.bpm,
                    budget: ReadBudget::default(),
                    settings,
                    how: mpfree_compose::Making::New,
                };
                // Another clip's result playing goes at once.
                if self.compose.in_view.is_some() && self.compose.bound() != Some((track, slot)) {
                    self.compose.in_view = None;
                    self.compose.line = None;
                    self.compile()?;
                }
                let playing = self.compose.in_view.is_some();
                // The knobs a sound move may turn (S3), as the screen
                // named them for this track.
                let safe = self
                    .compose_parts
                    .sound
                    .filter(|&(t, _)| t == track)
                    .map(|(_, s)| s)
                    .unwrap_or_default();
                self.compose
                    .ask(&self.project, ask, &scope, &locked, safe)?;
                self.changed = true;
                let more = more_bars(settings.bars.max(1), &name);
                if let Some(again) = scope.again() {
                    // An aimed try changes one thing (S5), the others
                    // change only part of the try.
                    let rest = if again.aim().is_some() {
                        "only that changes"
                    } else {
                        "the rest stays as it is"
                    };
                    return Ok(format!("continue: trying {} ({rest})", again.words()));
                }
                Ok(if playing {
                    format!(
                        "continue: trying {more} again, {} (the last plays on until it's ready)",
                        settings.style.name
                    )
                } else {
                    format!("continue: {more}, {}", settings.style.name)
                })
            }
            ContinueAction::Step { back } => {
                if !self.compose.step(back) {
                    return Err(format!(
                        "continue: no {} result",
                        if back { "earlier" } else { "later" }
                    ));
                }
                self.changed = true;
                if self.continue_due(self.block.frames) {
                    self.compile()?;
                }
                Ok(self.continue_when("continue: the result shown"))
            }
            ContinueAction::Keep => self.continue_keep(recording),
            ContinueAction::Parts(p) => {
                self.compose_parts = p;
                Ok(String::new())
            }
            ContinueAction::Open { track, slot } => self.continue_open(track, slot),
            ContinueAction::Back => {
                if !self.compose.back() {
                    return Err("continue: Keep is under way".into());
                }
                self.changed = true;
                if self.continue_due(self.block.frames) {
                    self.compile()?;
                }
                Ok(self.continue_when("continue: back to the clip as it was"))
            }
        }
    }

    /// The clip card's Continue (composing tools, phase 1 step 4): the
    /// note editor on clip `slot` of `track` (its track selected and the
    /// clip the Sequencer edits), then Continue opened there, which asks
    /// at once. Refused while another tool is open; the editor already on
    /// the clip stays as it is, Continue opening in it.
    fn continue_open(&mut self, track: usize, slot: usize) -> Result<String, String> {
        let c = self.project.clip(track, slot).ok_or_else(|| {
            format!(
                "continue: slot {} of track {} is empty",
                slot + 1,
                track + 1
            )
        })?;
        if c.looped.is_some() {
            return Err("continue: a loop's cell has no notes to carry on".into());
        }
        if c.holds != Holds::Notes {
            return Err(format!(
                "continue: {}",
                mpfree_compose::Refused::PadHits.words()
            ));
        }
        if c.notes.iter().all(|n| !n.on) {
            return Err("continue: play or draw some notes to carry on first".into());
        }
        if let Some(ed) = &self.editor {
            if let Some(tool) = ed.open_tool {
                if tool.kind == ToolKind::Continue && (ed.track, ed.slot) == (track, slot) {
                    return Ok("continue: open already, in the note editor".into());
                }
                return Err(format!(
                    "continue: Done or Cancel {} in the note editor first",
                    tool.kind.name()
                ));
            }
        }
        let on_it = self
            .editor
            .as_ref()
            .is_some_and(|e| (e.track, e.slot) == (track, slot));
        if !on_it {
            if self.selected() != Some(track) {
                let was = self.track_pick;
                self.set_pick(track);
                if self.track_pick != was {
                    self.compile()?;
                }
                self.steps_changed = true;
            }
            self.edit_clip(Some(slot))
                .map_err(|e| e.replacen("edit:", "continue:", 1))?;
            self.edit(EditAction::Open)
                .map_err(|e| e.replacen("edit:", "continue:", 1))?;
        }
        self.edit(EditAction::Row {
            row: CONTINUE_ROW,
            delta: 1,
        })
    }

    /// A touch on the note editor's Continue tool (composing tools, phase
    /// 1 step 2) on clip `slot` of `track`: an ask, completed with what
    /// the track plays (the screen's [`Parts`], else read from the clip),
    /// Keep or Back. The tool closes once Keep or Back is done, or when an
    /// ask is refused with nothing under way; `said` is the editor's own
    /// line (a setting changed), put first.
    fn continue_from_editor(
        &mut self,
        c: Compose,
        track: usize,
        slot: usize,
        said: &str,
    ) -> Result<String, String> {
        let r = match &c {
            Compose::Ask(asking) => {
                // The Sound row goes with the ask (S3): the try is heard
                // at it.
                self.compose.set_level(asking.sound, &self.project);
                let ask = ContinueAsk {
                    track,
                    slot,
                    settings: asking.settings,
                    kind: self.part_kind(track, slot),
                    lanes: self.compose_parts.lanes.clone(),
                    scope: asking.scope.clone(),
                    locked: asking.locked.clone(),
                };
                self.compose_from_editor = true;
                self.continue_action(ContinueAction::Ask(Box::new(ask)))
            }
            Compose::Sound(level) => self.continue_sound(*level),
            Compose::Keep => self.continue_action(ContinueAction::Keep),
            Compose::Back if !self.compose.active() => Ok("continue: closed".into()),
            Compose::Back => self.continue_action(ContinueAction::Back),
        };
        let close = match &c {
            Compose::Ask(_) => r.is_err() && !self.compose.active(),
            Compose::Sound(_) => false,
            Compose::Keep | Compose::Back => r.is_ok(),
        };
        if close {
            self.compose_from_editor = false;
            if let Some(ed) = self.editor.as_mut() {
                ed.open_tool = None;
            }
        }
        if matches!(c, Compose::Keep) && r.is_ok() {
            self.edit_follow();
        }
        self.changed = true;
        match (said, &c) {
            ("", _) => r,
            // "continue: sound bold, heard from 5.1".
            (said, Compose::Sound(_)) => {
                r.map(|w| match w.strip_prefix("continue: the sound changed") {
                    Some(when) => format!("{said}{when}"),
                    None => said.to_string(),
                })
            }
            (said, _) => r.map(|_| format!("{said}: the next try is on its way")),
        }
    }

    /// The Sound row set to `level` (composing tools S3): the tries heard
    /// again with their sound moves at it, no new try; the shown one comes
    /// in on the next bar line while its clip plays, as a step through
    /// the tries does.
    fn continue_sound(&mut self, level: mpfree_compose::SoundLevel) -> Result<String, String> {
        if !self.compose.set_level(level, &self.project) {
            return Ok(String::new());
        }
        self.changed = true;
        if self.continue_due(self.block.frames) {
            self.compile()?;
        }
        Ok(self.continue_when("continue: the sound changed"))
    }

    /// The kind of instrument clip `slot` of `track` is carried on for:
    /// the screen's word for the track, else read from the clip's notes.
    pub(super) fn part_kind(&self, track: usize, slot: usize) -> mpfree_compose::InstrumentKind {
        match self.compose_parts.kinds.get(track) {
            Some(Some(k)) => *k,
            _ => self
                .project
                .clip(track, slot)
                .map_or(mpfree_compose::InstrumentKind::OneNote, |c| {
                    mpfree_compose::profile::kind_of_notes(&c.notes)
                }),
        }
    }

    /// While the note editor's Continue asked for what is under way: Back
    /// once the editor no longer has Continue open on its clip (it closed,
    /// or opened another clip). True if the view is to be compiled.
    fn continue_follows_editor(&mut self) -> bool {
        if !self.compose_from_editor {
            return false;
        }
        if !self.compose.active() {
            self.compose_from_editor = false;
            return false;
        }
        let on = self
            .editor
            .as_ref()
            .filter(|e| e.continuing_open())
            .map(|e| (e.track, e.slot));
        if on.is_some() && on == self.compose.changing() {
            return false;
        }
        self.compose_from_editor = false;
        if !self.compose.back() {
            return false;
        }
        self.changed = true;
        self.note_later("continue: the editor moved on, so the clip is back as it was".into());
        self.continue_due(self.block.frames)
    }

    /// `what`, and when the music changes for it: on a bar line, or at
    /// once.
    fn continue_when(&self, what: &str) -> String {
        match self.compose.line {
            Some(l) => format!("{what}, heard from {}", bar_beat(l.line, self.info.meter)),
            None => what.to_string(),
        }
    }

    /// Keep: the result written as one take, and the view compiled from
    /// it at once (the same notes, so nothing is heard to change). A
    /// refusal writes nothing and leaves no result playing.
    fn continue_keep(&mut self, recording: bool) -> Result<String, String> {
        if recording {
            return Err("continue: not while recording".into());
        }
        let (track, slot, clip, bars) = match self.compose.keep(&self.project) {
            Ok(k) => k,
            Err(e) => {
                if self.compose.in_view.is_some() && self.compose.wanted(recording).is_none() {
                    self.compose.in_view = None;
                    self.compose.line = None;
                    self.compile()?;
                }
                self.changed = true;
                return Err(e);
            }
        };
        let name = clip.name.clone();
        let r = self.takes.replace_clip(
            &mut self.project,
            track,
            slot,
            clip,
            Replaced::Continue { bars },
        );
        let refused = r.err();
        self.compose.kept(r);
        self.changed = true;
        if let Some(e) = refused {
            self.compile()?;
            return Err(format!("continue: {name}: {e:?}, nothing written"));
        }
        if let Err(e) = self.compile() {
            self.takes.undo(&mut self.project);
            self.takes.forget_undone();
            let _ = self.compile();
            return Err(e);
        }
        Ok(format!(
            "continue: {} kept (UNDO takes it back)",
            more_bars(bars, &name)
        ))
    }

    /// The rest of clip `slot`'s row, for the reading: the clip in the
    /// same row on every other heard track of notes, drums read as drums.
    fn row_others(&self, track: usize, slot: usize) -> Vec<mpfree_compose::ToRead> {
        self.row_parts(slot, Some(track))
    }

    /// Row `slot`'s clips for the reading, but `but`'s: the clip in the
    /// row on every heard track of notes, drums read as drums.
    fn row_parts(&self, slot: usize, but: Option<usize>) -> Vec<mpfree_compose::ToRead> {
        use mpfree_context::PartRole;
        let p = &self.project;
        p.tracks
            .iter()
            .enumerate()
            .filter(|&(t, tr)| Some(t) != but && tr.kind == TrackKind::Notes && !p.silent(t))
            .filter_map(|(t, tr)| {
                let c = tr.slots.get(slot)?.as_ref()?;
                if c.looped.is_some() || c.notes.is_empty() {
                    return None;
                }
                let role = if c.holds == Holds::Hits {
                    PartRole::Drums
                } else {
                    PartRole::Chords
                };
                Some(mpfree_compose::ToRead {
                    track: t,
                    slot,
                    role,
                })
            })
            .collect()
    }

    /// Whether the play view plays clip `slot` of `track` now or on its
    /// next line: launched, cued, or (in song mode) placed in the song.
    fn clip_heard(&self, track: usize, slot: usize) -> bool {
        self.launches
            .iter()
            .any(|l| (l.track, l.slot) == (track, slot))
            || self
                .cues
                .iter()
                .any(|c| c.track == track && c.slot == Some(slot))
            || (self.song && self.song_places(track, slot))
    }

    /// Continue, each period: the composer's answers, a recording that
    /// changed the clip or what it listened to, and a change coming due on
    /// its line. True if the view is to be compiled.
    fn continue_period(&mut self, frames: u32) -> bool {
        // The Steps show the shown result in place of its clip.
        let heard = self.compose.heard_key();
        if heard != self.steps_heard {
            self.steps_heard = heard;
            self.steps_changed = true;
        }
        if !self.compose.active() {
            self.compose_from_editor = false;
            return false;
        }
        let mut compile = self.continue_follows_editor();
        if self.recorded_now && self.compose.follow(&self.project) {
            self.changed = true;
            if self.compose.in_view.take().is_some() {
                compile = true;
            }
            self.compose.line = None;
        }
        let mut log = Vec::new();
        if self.compose.answers(&self.project, &mut log) {
            self.changed = true;
        }
        for line in log {
            self.note_later(line);
        }
        compile | self.continue_due(frames)
    }

    /// Bring the play view to what Continue wants: a result in place of
    /// the clip, another result, or the clip itself. While the clip plays
    /// and the transport rolls the change waits for the next bar line
    /// (one within the next period is as good as now); otherwise it lands
    /// at once. REC stops a result playing at once. True if the view is
    /// to be compiled.
    fn continue_due(&mut self, frames: u32) -> bool {
        let recording = matches!(self.info.state, State::Recording | State::CountIn);
        if recording && self.compose.stop_for_rec() {
            self.changed = true;
            self.note_later("continue: REC brought the clip back; the new bars stay ready".into());
            return true;
        }
        let want = self.compose.wanted(recording);
        if want == self.compose.in_view_id() {
            if self.compose.line.take().is_some() {
                self.changed = true;
            }
            self.compose.landed(want);
            return false;
        }
        let Some((track, slot)) = self.compose.changing() else {
            self.compose.landed(None);
            self.changed = true;
            return true;
        };
        if self.info.state.rolling() && self.clip_heard(track, slot) {
            let pos = self.info.position;
            let bar = self.info.meter.bar_ticks().max(1);
            let w = *self.compose.line.get_or_insert(LineWait {
                line: LaunchGrid::Bars(1).next(pos, bar),
                asked: pos,
            });
            self.changed = true;
            if !(w.line <= pos + self.period_ticks(frames) || pos < w.asked) {
                return false;
            }
        }
        self.compose.landed(want);
        self.changed = true;
        true
    }

    /// The launch of Continue's clip follows what the view plays: a
    /// result's tick 0 falls where the clip started, so it plays in time
    /// with what started with it; the clip as it was gets its own origin
    /// back. A launch that started again since is left as it is.
    fn continue_anchor(&mut self) {
        let Some((t, s, len, speed)) = self
            .compose
            .in_view_clip()
            .map(|(t, s, c)| (t, s, c.length, c.speed))
        else {
            if let Some((t, s, start, origin)) = self.compose.origin.take() {
                for l in self.launches.iter_mut() {
                    if (l.track, l.slot, l.start) == (t, s, start) {
                        l.origin = origin;
                    }
                }
            }
            return;
        };
        for l in self.launches.iter_mut() {
            if (l.track, l.slot) != (t, s) {
                continue;
            }
            match self.compose.origin {
                Some((ot, os, start, _)) if (ot, os, start) == (t, s, l.start) => {}
                _ => self.compose.origin = Some((t, s, l.start, l.origin)),
            }
            l.origin = mpfree_play::anchored(l.start, len, speed);
        }
    }

    /// The view of the project as it plays: [`TimeControl::view`], with
    /// Continue's result in place of its clip while one is in the view.
    fn view_with_continue(&mut self) -> Result<PlayView, String> {
        let Some((t, s, heard)) = self
            .compose
            .in_view_clip()
            .map(|(t, s, c)| (t, s, c.clone()))
        else {
            return self.view();
        };
        let Some(slot) = self
            .project
            .tracks
            .get_mut(t)
            .and_then(|tr| tr.slots.get_mut(s))
        else {
            return self.view();
        };
        let real = slot.replace(heard);
        let v = self.view();
        if let Some(slot) = self
            .project
            .tracks
            .get_mut(t)
            .and_then(|tr| tr.slots.get_mut(s))
        {
            *slot = real;
        }
        v
    }

    /// Add a part (composing tools S2.6): a touch on its panel.
    fn add_action(&mut self, a: AddAction) -> Result<String, String> {
        let recording = matches!(self.info.state, State::Recording | State::CountIn);
        match a {
            AddAction::Ask(ask) => {
                if recording {
                    return Err("add a part: not while recording".into());
                }
                if self.song_rec.armed() {
                    return Err(
                        "add a part: Rec song is on, and a try would be recorded into the song: turn it off first"
                            .into(),
                    );
                }
                let (track, row) = (ask.track, ask.row);
                // New track (S2.6b): the part is for the track Keep adds,
                // at the end; it swings as the transport does.
                let swing = if ask.new_track.is_some() {
                    if track != self.project.tracks.len() {
                        return Err("add a part: a track was added: ask again".into());
                    }
                    if self.project.tracks.len() >= MAX_TRACKS {
                        return Err(format!(
                            "add a part: {MAX_TRACKS} tracks is the most a project holds"
                        ));
                    }
                    self.info.swing
                } else {
                    let tr =
                        self.project.tracks.get(track).ok_or_else(|| {
                            format!("add a part: there is no track {}", track + 1)
                        })?;
                    if tr.kind != TrackKind::Notes {
                        return Err("add a part: only a track of notes takes a part".into());
                    }
                    tr.swing.unwrap_or(self.info.swing)
                };
                let hearing = Hearing {
                    parts: self.row_others(track, row),
                    swing,
                    transport_swing: self.info.swing,
                    meter: self.info.meter,
                    bpm: self.info.bpm,
                };
                let part = format!("{:?}", ask.part).to_lowercase();
                let what = match &ask.phrase {
                    // A phrase as the part (P2, board H).
                    Some((name, _)) => format!("{name} as the {part}"),
                    None => format!("{part} on {}", ask.relation.words()),
                };
                let playing = self.add.in_view.is_some();
                self.add.ask(&self.project, *ask, hearing)?;
                self.changed = true;
                Ok(if playing {
                    format!(
                        "add a part: trying {what} for row {} (the last try plays on until it's ready)",
                        row + 1
                    )
                } else {
                    format!("add a part: {what} for row {}", row + 1)
                })
            }
            AddAction::Step { back } => {
                if !self.add.step(back) {
                    return Err(format!(
                        "add a part: no {} try",
                        if back { "earlier" } else { "later" }
                    ));
                }
                self.changed = true;
                if self.add_due(self.block.frames) {
                    self.compile()?;
                }
                Ok(self.add_when("add a part: the try shown"))
            }
            AddAction::Line(line) => {
                if self.add.target().is_none() {
                    return Err("add a part: nothing is open".into());
                }
                self.add.pick_line(line);
                self.changed = true;
                if self.add.in_view.is_some() {
                    self.compile()?;
                }
                Ok("add a part: the try plays the picked chords (Keep keeps them)".into())
            }
            AddAction::Keep => self.add_keep(recording),
            AddAction::Back => {
                self.add.back();
                self.changed = true;
                if self.add_due(self.block.frames) {
                    self.compile()?;
                }
                Ok(self.add_when("add a part: back to the row as it was"))
            }
        }
    }

    /// When an Add a part change is heard, for its words.
    fn add_when(&self, what: &str) -> String {
        match self.add.line {
            Some(l) => format!("{what}, heard from {}", bar_beat(l.line, self.info.meter)),
            None => what.to_string(),
        }
    }

    /// Keep: the shown try written in its cell, with the panel's chords
    /// on the row, as one take; the track's launch moves to it where the
    /// try played, so nothing is heard to change. A refusal writes nothing.
    fn add_keep(&mut self, recording: bool) -> Result<String, String> {
        if recording {
            return Err("add a part: not while recording".into());
        }
        let super::adding::Keeping {
            track,
            row,
            clip,
            new_track,
            line,
            heard,
        } = self.add.keep(&self.project)?;
        let name = clip.name.clone();
        // What the row's other clips are measured against from now on:
        // its chords, or as it was heard when it had none of its own.
        let before = self
            .project
            .scene(row)
            .map(|s| s.chords.clone())
            .filter(|l| !l.is_empty())
            .unwrap_or(heard);
        let launch = self
            .launches
            .iter()
            .find(|l| l.slot == row && l.track != track)
            .copied();
        let changes_chords = line.is_some();
        let r = match &new_track {
            // New track: the track and the part, one take. Keep's check
            // made sure it gets the index the try played on.
            Some(name) => self
                .takes
                .add_part_on_new_track(
                    &mut self.project,
                    mpfree_seq::Track::new(name, 0),
                    row,
                    clip,
                    line,
                )
                .map(|(n, index)| {
                    debug_assert_eq!(index, track, "New track's index");
                    n
                }),
            None => self
                .takes
                .add_part(&mut self.project, track, row, clip, line),
        };
        self.add.kept(&r);
        self.changed = true;
        if let Err(e) = r {
            return Err(format!("add a part: {name}: {e:?}, nothing written"));
        }
        if changes_chords {
            self.note_fitted(row, before);
        }
        // The kept clip plays on where the try played.
        if let (Some(l), Some(c)) = (launch, self.project.clip(track, row)) {
            if self
                .add
                .in_view
                .as_ref()
                .is_some_and(|v| (v.track, v.row) == (track, row))
            {
                let origin = mpfree_play::anchored(l.start, c.length, c.speed);
                self.launches.retain(|x| x.track != track);
                self.launches.push(mpfree_play::Launch {
                    track,
                    slot: row,
                    origin,
                    start: l.start,
                });
            }
        }
        self.add.in_view = None;
        self.add.line = None;
        if let Err(e) = self.compile() {
            self.takes.undo(&mut self.project);
            self.takes.forget_undone();
            let _ = self.compile();
            return Err(e);
        }
        Ok(match new_track {
            Some(t) => format!(
                "add a part: {name} kept in row {} on a new track, {t} (UNDO takes both back)",
                row + 1
            ),
            None => format!(
                "add a part: {name} kept in row {} (UNDO takes it back)",
                row + 1
            ),
        })
    }

    /// Whether row `row` plays now: a clip of it launched (on a track
    /// other than `but`).
    fn row_playing(&self, row: usize, but: usize) -> Option<mpfree_play::Launch> {
        self.launches
            .iter()
            .find(|l| l.slot == row && l.track != but)
            .copied()
    }

    /// Add a part, each period: the composer's answers, a recording that
    /// changed what a try heard, and a change coming due on its line.
    /// True if the view is to be compiled.
    fn add_period(&mut self, frames: u32) -> bool {
        if self.add.heard_answers() {
            self.changed = true;
        }
        // The Song view's Chords lane: a row heard, then the next.
        if self.song_hear.answers() {
            self.changed = true;
            self.hear_song();
        }
        if !self.add.active() {
            return false;
        }
        let mut compile = false;
        if self.recorded_now && self.add.follow(&self.project) {
            self.changed = true;
            if self.add.in_view.take().is_some() {
                compile = true;
            }
            self.add.line = None;
        }
        let mut log = Vec::new();
        if self.add.answers(&self.project, &mut log) {
            self.changed = true;
        }
        for line in log {
            self.note_later(line);
        }
        compile | self.add_due(frames)
    }

    /// Bring the play view to what Add a part wants: a try in its cell,
    /// another try, or the cell quiet. While the row plays and the
    /// transport rolls, the change waits for the next bar line; a try
    /// waits for the row to play; the cell goes quiet at once when the row
    /// doesn't play. REC stops a try at once. True if the view is to be
    /// compiled.
    fn add_due(&mut self, frames: u32) -> bool {
        let recording = matches!(self.info.state, State::Recording | State::CountIn);
        if recording && self.add.stop_for_rec() {
            self.changed = true;
            return true;
        }
        let want = self.add.wanted(recording);
        if want == self.add.in_view_id() {
            if self.add.line.take().is_some() {
                self.changed = true;
            }
            return false;
        }
        let Some((track, row)) = self.add.changing() else {
            self.add.landed(None);
            self.changed = true;
            return true;
        };
        let playing = self.row_playing(row, track).is_some();
        if want.is_some() && !playing {
            // Heard once the row plays.
            if self.add.in_view.is_none() {
                return false;
            }
        } else if playing && self.info.state.rolling() {
            let pos = self.info.position;
            let bar = self.info.meter.bar_ticks().max(1);
            let w = *self.add.line.get_or_insert(LineWait {
                line: LaunchGrid::Bars(1).next(pos, bar),
                asked: pos,
            });
            self.changed = true;
            if !(w.line <= pos + self.period_ticks(frames) || pos < w.asked) {
                return false;
            }
        }
        self.add.landed(want);
        self.changed = true;
        true
    }

    /// The view of the project as it plays: [`Self::view_with_continue`],
    /// with Add a part's try in its empty cell (or in a blank clip's
    /// place) while one is in the view, launched where the row's clips
    /// started so it lines up with them (in place of what its track
    /// played). Without a clip of the row playing, the try isn't heard.
    fn view_with_add(&mut self) -> Result<PlayView, String> {
        let Some(v) = self.add.in_view.clone() else {
            return self.view_with_continue();
        };
        let Some(l) = self.row_playing(v.row, v.track) else {
            return self.view_with_continue();
        };
        if let Some(name) = &v.new_track {
            return self.view_with_new_track(&v, name, l);
        }
        let empty = self
            .project
            .tracks
            .get(v.track)
            .and_then(|t| t.slots.get(v.row))
            .is_some_and(|s| s.as_ref().is_none_or(Clip::is_blank));
        if !empty {
            return self.view_with_continue();
        }
        let launch = mpfree_play::Launch {
            track: v.track,
            slot: v.row,
            origin: mpfree_play::anchored(l.start, v.clip.length, v.clip.speed),
            start: l.start,
        };
        let was = self.project.tracks[v.track].slots[v.row].replace((*v.clip).clone());
        let saved = self.launches.clone();
        self.launches.retain(|x| x.track != v.track);
        self.launches.push(launch);
        let out = self.view_with_continue();
        self.launches = saved;
        self.project.tracks[v.track].slots[v.row] = was;
        out
    }

    /// [`Self::view_with_add`] for a try on New track (S2.6b): the view
    /// has the track Keep will add, at the end, with the try in its cell
    /// and launched where the row's clip `row_launch` started; the project
    /// gets it only at Keep. The track's notes go by its index to the
    /// instruments, where the synth made for the part plays them (its
    /// send set by the app). Nothing else of the track is set: the view
    /// is the only place it is, and only while the try plays.
    fn view_with_new_track(
        &mut self,
        v: &super::adding::AddInView,
        name: &str,
        row_launch: mpfree_play::Launch,
    ) -> Result<PlayView, String> {
        let n = self.project.tracks.len();
        if v.track != n || n >= MAX_TRACKS || v.row >= mpfree_seq::ROWS {
            return self.view_with_continue();
        }
        let mut t = mpfree_seq::Track::new(name, mpfree_play::lowest_free_dest(&self.project));
        t.slots[v.row] = Some((*v.clip).clone());
        self.project.tracks.push(t);
        let saved = self.launches.clone();
        self.launches.push(mpfree_play::Launch {
            track: v.track,
            slot: v.row,
            origin: mpfree_play::anchored(row_launch.start, v.clip.length, v.clip.speed),
            start: row_launch.start,
        });
        let out = self.view_with_continue();
        self.launches = saved;
        self.project.tracks.pop();
        out
    }

    /// Row `row`'s chords changed from `before`: unless it has a change
    /// its clips haven't fitted yet, its clips are measured against
    /// `before` from now on.
    fn note_fitted(&mut self, row: usize, before: mpfree_context::chord::ChordTimeline) {
        if !self.fitted.iter().any(|(r, _)| *r == row) {
            self.fitted.push((row, before));
        }
    }

    /// The kept clips off their row's chords, measured again: a row whose
    /// clips all fit again (Fit, UNDO, the chords put back) is let go.
    fn refresh_unfit(&mut self) {
        if self.fitted.is_empty() {
            if !self.unfit.is_empty() {
                self.unfit = Arc::from(Vec::new());
            }
            return;
        }
        let bar = self.info.meter.bar_ticks().max(1);
        let scenes = self.project.scenes.len();
        self.fitted.retain(|(r, _)| *r < scenes);
        // (a row with no scene has no chords of its own: nothing to fit)
        let found = unfit(&self.project, &self.fitted, bar, |t, s| {
            self.part_kind(t, s)
        });
        self.fitted
            .retain(|(r, _)| found.iter().any(|u| u.row == *r));
        self.unfit = Arc::from(found);
    }

    /// The row's chords outside Add a part: Use (one take) or Fit to the
    /// new chords (one take).
    fn chords_action(&mut self, c: ChordsAction) -> Result<String, String> {
        if let ChordsAction::Hear { row } = c {
            match row {
                Some(r) if r < mpfree_seq::SCENES => {
                    if self.add.hearing != Some(r) {
                        self.add.stop_hearing();
                        self.add.hearing = Some(r);
                    }
                    self.hear_row();
                }
                Some(r) => return Err(format!("chords: there is no row {}", r + 1)),
                None => self.add.stop_hearing(),
            }
            self.changed = true;
            return Ok(String::new());
        }
        if matches!(self.info.state, State::Recording | State::CountIn) {
            return Err("chords: not while recording".into());
        }
        match c {
            ChordsAction::Set { row, line, shown } => {
                if row >= mpfree_seq::SCENES {
                    return Err(format!("chords: there is no row {}", row + 1));
                }
                let before = self
                    .project
                    .scene(row)
                    .map(|s| s.chords.clone())
                    .filter(|l| !l.is_empty())
                    .unwrap_or(shown);
                self.takes
                    .set_row_chords(&mut self.project, row, line)
                    .map_err(|e| format!("chords: {e:?}, nothing written"))?;
                self.note_fitted(row, before);
                self.changed = true;
                self.compile()?;
                let n = self
                    .unfit
                    .iter()
                    .find(|u| u.row == row)
                    .map_or(0, |u| u.clips.len());
                Ok(match n {
                    0 => format!(
                        "chords: row {}'s chords set (UNDO takes them back)",
                        row + 1
                    ),
                    1 => format!(
                        "chords: row {}'s chords set; a clip doesn't fit them: Fit moves it",
                        row + 1
                    ),
                    n => format!(
                        "chords: row {}'s chords set; {n} clips don't fit them: Fit moves them",
                        row + 1
                    ),
                })
            }
            ChordsAction::Fit { row, tracks } => {
                let before = self
                    .fitted
                    .iter()
                    .find(|(r, _)| *r == row)
                    .map(|(_, b)| b.clone())
                    .ok_or_else(|| format!("chords: row {}'s clips fit its chords", row + 1))?;
                let bar = self.info.meter.bar_ticks().max(1);
                let (clips, why) =
                    fitted_clips(&self.project, row, &before, &tracks, bar, |t, s| {
                        self.part_kind(t, s)
                    });
                if clips.is_empty() {
                    return Err(match why.first() {
                        Some(w) => format!("chords: nothing to fit: {w}"),
                        None => "chords: nothing to fit".into(),
                    });
                }
                let n = clips.len();
                self.takes
                    .replace_clips(&mut self.project, clips, Replaced::FitChords)
                    .map_err(|e| format!("chords: {e:?}, nothing written"))?;
                self.add.counts.fitted += n as u64;
                self.changed = true;
                self.compile()?;
                let mut said = format!(
                    "chords: {} fitted to row {}'s chords (UNDO takes it back)",
                    if n == 1 {
                        "a clip".to_string()
                    } else {
                        format!("{n} clips")
                    },
                    row + 1
                );
                for w in why {
                    said.push_str(&format!("; {w}"));
                }
                Ok(said)
            }
            ChordsAction::Hear { .. } => unreachable!("answered above"),
        }
    }

    /// Hear the row the screens show the chords of again, if its clips or
    /// chords changed since it was last heard. A composer that can't start
    /// or stopped is said on the status line and the hearing stops: the
    /// screens show no heard chords until they ask again.
    fn hear_row(&mut self) {
        let Some(row) = self.add.hearing else {
            return;
        };
        let hearing = Hearing {
            parts: self.row_parts(row, None),
            swing: self.info.swing,
            transport_swing: self.info.swing,
            meter: self.info.meter,
            bpm: self.info.bpm,
        };
        if let Err(e) = self.add.hear(&self.project, row, hearing) {
            self.add.stop_hearing();
            self.note_later(e);
        }
    }

    /// While the Song view shows, hear the next row the song's sections
    /// play whose clips or chords changed since it was heard, for its
    /// Chords lane (composing step 6, board C). A composer that can't
    /// start is said on the status line.
    fn hear_song(&mut self) {
        if !self.song_view {
            return;
        }
        let bar = self.info.meter.bar_ticks().max(1);
        let mut rows: Vec<usize> = mpfree_seq::arrange::scene_sections(&self.project, bar)
            .iter()
            .map(|s| s.2)
            .collect();
        rows.sort_unstable();
        rows.dedup();
        let wanted = rows
            .into_iter()
            .map(|row| {
                let hearing = Hearing {
                    parts: self.row_parts(row, None),
                    swing: self.info.swing,
                    transport_swing: self.info.swing,
                    meter: self.info.meter,
                    bpm: self.info.bpm,
                };
                (row, hearing)
            })
            .collect();
        match self.song_hear.hear(&self.project, wanted) {
            Ok(true) => self.changed = true,
            Ok(false) => {}
            Err(e) => self.note_later(e),
        }
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
    /// The tempo the last launch asked for ([`mpfree_perform::Perform::take_tempo`])
    /// on the song tick its line falls on from the playhead as this half
    /// heard it, where the player lands the scene's cues: the first line
    /// at or after it (a line the audio half has already played by then
    /// makes it late, at once, as the cues are). Stopped, or with no line,
    /// at once. A launch of a scene with none, and Stop all, take a tempo
    /// still waiting back. True when the view needs compiling again.
    fn scene_tempo_on_its_line(&mut self) -> bool {
        let Some(ask) = self.perform.take_tempo() else {
            return false;
        };
        let bar = self.info.meter.bar_ticks();
        let rolling = self.info.state.rolling();
        let at = ask
            .line
            .filter(|_| rolling)
            .map(|l| l.next(self.info.position, bar));
        let was = self.scene_tempo;
        self.scene_tempo = ask.bpm.map(|bpm| {
            self.tempo_ids += 1;
            SceneTempo {
                bpm,
                at,
                from: self.project_bpm,
                id: self.tempo_ids,
            }
        });
        was.is_some() || self.scene_tempo.is_some()
    }

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
        for (t, p, m, s) in q.pads {
            self.set_pad(t, p, m, s);
        }
        true
    }

    /// Track `t`'s pad `pad` muted and soloed as `mute` and `solo` say
    /// (step 10); nothing for a track or pad out of range. The caller
    /// compiles.
    fn set_pad(&mut self, t: usize, pad: usize, mute: bool, solo: bool) {
        let (Some(tr), Some(bit)) = (
            self.project.tracks.get_mut(t),
            1u128.checked_shl(pad as u32),
        ) else {
            return;
        };
        tr.pad_mutes = (tr.pad_mutes & !bit) | if mute { bit } else { 0 };
        tr.pad_solos = (tr.pad_solos & !bit) | if solo { bit } else { 0 };
        self.changed = true;
    }

    /// The tracks' pads waiting in the mute queue land once their line is
    /// within the next period (`frames` long), or at once with the
    /// transport stopped: each track takes its pads' mutes and solos, and
    /// the log says so. True when they landed (the caller compiles).
    fn pads_due(&mut self, frames: u32) -> bool {
        let Some((_, q)) = &self.mute_queue else {
            return false;
        };
        if q.pads.is_empty() {
            return false;
        }
        let pos = self.info.position;
        let line = q.line.next(pos, self.info.meter.bar_ticks());
        let rolling = self.info.state.rolling();
        if rolling && line > pos + self.period_ticks(frames) {
            return false;
        }
        let words = self.pads_line(if rolling { line } else { pos });
        let mut pads = Vec::new();
        if let Some((_, q)) = &mut self.mute_queue {
            pads = std::mem::take(&mut q.pads);
            if q.is_empty() {
                self.mute_queue = None;
            }
        }
        for (t, p, m, s) in pads {
            self.set_pad(t, p, m, s);
        }
        self.changed = true;
        self.note_later(if rolling {
            words
        } else {
            format!("{words} (STOP)")
        });
        true
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

    /// The log's line for the tracks' pads in the queue landing on `at`:
    /// "pad mute: at 5.1, Drums A3 muted, Drums B1 solo".
    fn pads_line(&self, at: Tick) -> String {
        let p = self.info.meter.position(at);
        let mut line = format!("pad mute: at {}.{}", p.bar, p.beat);
        if let Some((_, q)) = &self.mute_queue {
            for &(t, pad, m, s) in &q.pads {
                line.push_str(&format!(", {} {}", self.pad_label(t, pad), pad_words(m, s)));
            }
        }
        line
    }

    /// Track `t`'s pad `pad` as the log names it: "Drums B5".
    fn pad_label(&self, t: usize, pad: usize) -> String {
        let name = self
            .project
            .tracks
            .get(t)
            .map_or_else(|| format!("Track {}", t + 1), |tr| tr.name.clone());
        format!("{name} {}", pad_name(pad))
    }

    /// Pad `pad` of track `t`'s kit tapped as a mute key (SHIFT + TRACK
    /// MUTE, the instrument redesign's step 10: a pad mute is the
    /// track's alone): its mute toggled, or its solo with `solo`, from
    /// the track's or from what waits for it. With `wait` (Perform), the
    /// transport rolling and Launch not at once it joins the mute queue
    /// and lands with it ([`PadTap::Waits`]); a tap on a pad waiting takes
    /// its change back. Otherwise the track takes it at once
    /// ([`PadTap::Now`]).
    pub fn pad_mute_or_solo(
        &mut self,
        t: usize,
        pad: usize,
        wait: bool,
        solo: bool,
    ) -> Result<PadTap, String> {
        let tr = self
            .project
            .tracks
            .get(t)
            .ok_or_else(|| format!("pad mute: no track {}", t + 1))?;
        let bit = 1u128
            .checked_shl(pad as u32)
            .ok_or_else(|| format!("pad mute: no pad {}", pad + 1))?;
        let now = (tr.pad_mutes & bit != 0, tr.pad_solos & bit != 0);
        let name = self.pad_label(t, pad);
        let queued = self
            .mute_queue
            .as_ref()
            .and_then(|(_, q)| q.pads.iter().find(|w| (w.0, w.1) == (t, pad)))
            .map(|w| (w.2, w.3));
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
        if let Some((_, q)) = &mut self.mute_queue {
            q.pads.retain(|w| (w.0, w.1) != (t, pad));
        }
        let waits = wait && self.info.state.rolling() && line != LaunchGrid::Now;
        if waits && (m, s) != now {
            let ids = &mut self.mute_ids;
            let (_, q) = self.mute_queue.get_or_insert_with(|| {
                *ids += 1;
                (*ids, MuteWait::new(line))
            });
            q.pads.push((t, pad, m, s));
        }
        if self.mute_queue.as_ref().is_some_and(|(_, q)| q.is_empty()) {
            self.mute_queue = None;
        }
        self.changed = true;
        if waits {
            return Ok(PadTap::Waits(if (m, s) == now {
                format!("pad mute: {name}: its change taken back")
            } else {
                format!("pad mute: {name} {what} {}", line_words(line))
            }));
        }
        self.set_pad(t, pad, m, s);
        if let Err(e) = self.compile() {
            self.set_pad(t, pad, now.0, now.1);
            return Err(e);
        }
        Ok(PadTap::Now(format!("pad mute: {name} {what}")))
    }

    /// Pad `pad` of track `t` muted or let play at once (a pattern row's
    /// speaker, step 10), its solo as it is; anything waiting for it in
    /// the mute queue is dropped. The line for the log.
    pub fn pad_mute_now(&mut self, t: usize, pad: usize, mute: bool) -> Result<String, String> {
        let tr = self
            .project
            .tracks
            .get(t)
            .ok_or_else(|| format!("pad mute: no track {}", t + 1))?;
        let bit = 1u128
            .checked_shl(pad as u32)
            .ok_or_else(|| format!("pad mute: no pad {}", pad + 1))?;
        let was = (tr.pad_mutes & bit != 0, tr.pad_solos & bit != 0);
        if let Some((_, q)) = &mut self.mute_queue {
            q.pads.retain(|w| (w.0, w.1) != (t, pad));
            if q.is_empty() {
                self.mute_queue = None;
            }
        }
        self.set_pad(t, pad, mute, was.1);
        if let Err(e) = self.compile() {
            self.set_pad(t, pad, was.0, was.1);
            return Err(e);
        }
        let what = if mute { "muted" } else { "plays" };
        Ok(format!("pad mute: {} {what}", self.pad_label(t, pad)))
    }

    /// The pad mutes and solos of each track in `tracks` (a bit each) set
    /// whole: an old project's kit 1 pad mutes, carried over as it opens
    /// (step 10). Compiled once; how many tracks took them.
    pub fn carry_pad_mutes(&mut self, tracks: u128, mutes: u128, solos: u128) -> usize {
        let mut n = 0;
        for (t, tr) in self.project.tracks.iter_mut().enumerate().take(128) {
            if tracks >> t & 1 == 1 {
                (tr.pad_mutes, tr.pad_solos) = (mutes, solos);
                n += 1;
            }
        }
        if n > 0 {
            self.changed = true;
            let _ = self.compile();
        }
        n
    }

    /// Whether a change waits for track `t`'s pad `pad` in the mute queue,
    /// for its light.
    pub fn pad_waiting(&self, t: usize, pad: usize) -> bool {
        self.mute_queue
            .as_ref()
            .is_some_and(|(_, q)| q.pads.iter().any(|w| (w.0, w.1) == (t, pad)))
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

    /// The free cells' loops (audio tracks phase 4e.4): for each loop, None
    /// when it is no free loop a cell names (or loops aren't on audio
    /// tracks), else whether one of its cells plays. The loop host follows
    /// it on the free clock: on, on the free clock's next bar line (at
    /// once with none running); off, at once (`LoopHost::follow_free`).
    pub fn free_plan(&self) -> [Option<bool>; LOOPS] {
        let p = &self.project;
        std::array::from_fn(|n| {
            let id = LoopId(self.free_ids[n]);
            if !self.loops_on || id.0 == 0 || p.loop_track(id).is_none() {
                return None;
            }
            Some(
                self.free_on
                    .iter()
                    .any(|&(t, s)| p.clip(t, s).and_then(Clip::loop_id) == Some(id)),
            )
        })
    }

    /// The free cells follow what changed (phase 4e.4), before a compile:
    /// a launch or a cue of a free cell becomes its track's free launch at
    /// once (the loop starts on its own clock), its track's clip stopping
    /// at once; a clip launched on a track with a free launch ends it (one
    /// clip a track); a free launch whose cell is no longer a free cell
    /// goes.
    fn free_cells_follow(&mut self) {
        if !self.loops_on {
            self.free_on.clear();
            return;
        }
        let (p, ids) = (&self.project, &self.free_ids);
        let mut took: Vec<(usize, usize)> = Vec::new();
        self.launches.retain(|l| {
            let free = is_free_cell(p, ids, l.track, l.slot);
            if free {
                took.push((l.track, l.slot));
            }
            !free
        });
        self.cues.retain(|c| match c.slot {
            Some(s) if is_free_cell(p, ids, c.track, s) => {
                took.push((c.track, s));
                false
            }
            _ => true,
        });
        let launches = &self.launches;
        self.free_on
            .retain(|f| !launches.iter().any(|l| l.track == f.0));
        for (t, s) in took {
            self.launches.retain(|l| l.track != t);
            self.cues.retain(|c| c.track != t);
            self.free_on.retain(|f| f.0 != t);
            self.free_on.push((t, s));
        }
        let (p, ids) = (&self.project, &self.free_ids);
        self.free_on.retain(|&(t, s)| is_free_cell(p, ids, t, s));
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
        let ids = self.loop_ids;
        self.loops_are_on(now, &ids, false)
    }

    /// A loop cell on an audio track copied onto an empty cell of another
    /// audio track (Dev's switch, phase 4b; [`TimeAction::LoopCopy`]): a
    /// copy is a new loop, so the cell there names `id`, the one the app
    /// gave loop `into`, an empty loop the looper copies the audio into.
    /// As a Loop track's copy it is not a take: the looper's UNDO takes it
    /// back, and its cell goes with it ([`TimeAction::LoopCopyOff`] while
    /// nothing went in). Refused, with why: no id, one the project names
    /// already, a loop not free, the same track, or a cell that isn't an
    /// audio track's empty one.
    fn copy_audio_loop(
        &mut self,
        from: (usize, usize),
        to: (usize, usize),
        into: u8,
        id: u128,
    ) -> Result<String, String> {
        let n = usize::from(into);
        let name = |p: &Project, (t, s): (usize, usize)| {
            let track = p.tracks.get(t).map_or("", |t| t.name.as_str());
            format!("{track} / {}", s + 1)
        };
        let clip = self
            .project
            .clip(from.0, from.1)
            .filter(|c| c.loop_clip.is_some())
            .ok_or_else(|| format!("clips: {} holds no loop", name(&self.project, from)))?;
        let (len, was) = (clip.length, clip.name.clone());
        let there = self
            .project
            .tracks
            .get(to.0)
            .filter(|tr| tr.is_audio() && to.0 != from.0)
            .and_then(|tr| tr.slots.get(to.1))
            .ok_or("clips: a loop's copy onto another audio track goes in a cell of it")?;
        if there.is_some() {
            return Err(format!(
                "clips: {} has a clip; a copy goes in an empty cell",
                name(&self.project, to)
            ));
        }
        let held = |i: u128| i != 0 && self.project.loop_track(LoopId(i)).is_some();
        if id == 0 || held(id) {
            return Err("clips: the copy has no loop of its own to name".to_string());
        }
        if n >= LOOPS
            || loops::in_cell(&self.project, into)
            || held(self.loop_ids[n])
            || self.loops_seen[n].pass
        {
            return Err(format!("clips: loop {} isn't free for a copy", n + 1));
        }
        self.project
            .put_new_clip(
                to.0,
                to.1,
                Clip::new_audio_loop(LoopId(id), &format!("Loop {}", n + 1), len),
            )
            .map_err(|e| format!("clips: {e:?}"))?;
        self.compile()?;
        self.copy_ids[n] = id;
        Ok(format!(
            "clips: {was} copies into Loop {} on {}: launch it to hear it (the looper's UNDO takes it back)",
            n + 1,
            name(&self.project, to)
        ))
    }

    /// [`TimeControl::loops_are`], with each loop's lasting id (`ids`, 0:
    /// none yet), for loops on audio tracks (Dev's switch, phase 4b): a
    /// first pass that ends with an audio track's empty cell armed lands
    /// there as a loop cell naming its id ([`Clip::new_audio_loop`]),
    /// launched from the loop's own start, so it plays straight on in that
    /// track. With no cell armed for it (recorded from the Looper tab or
    /// with Capture: phase 4e.4) it lands on an audio track all the same
    /// ([`loops::place_audio`]: the armed audio track's cell in the row
    /// playing, another audio track's, or a new audio track's while `room`,
    /// an engine track free), never on a Loop track. With no id yet it
    /// waits for one, landing nowhere; with one the project names already
    /// it lands nowhere new. An audio track's loop cells follow its loop's
    /// length and empty when it is cleared, as a Loop track's do. Each line
    /// says where a loop went.
    pub fn loops_are_on(
        &mut self,
        now: &[LoopNow; LOOPS],
        ids: &[u128; LOOPS],
        room: bool,
    ) -> Vec<String> {
        let mut lines = Vec::new();
        let loops_on = self.loops_on;
        // A free loop is in no cell while loops are on Loop tracks; on
        // audio tracks it lands in a free cell (phase 4e.4).
        let now: &[LoopNow; LOOPS] = &std::array::from_fn(|n| match now[n] {
            l if l.free && !loops_on => LoopNow::default(),
            l => l,
        });
        let was_ids = std::mem::replace(&mut self.loop_ids, *ids);
        if *now == self.loops_seen && *ids == was_ids {
            return lines;
        }
        let was_free = self.free_ids;
        self.free_ids = std::array::from_fn(|n| if now[n].free { ids[n] } else { 0 });
        let audio_armed = self.audio_armed();
        // A loop free now, or no longer: its cells follow (compile). A free
        // cell playing whose loop Make song made a song loop (phase 4d.2)
        // plays on as a song cell, launched from the loop's own origin, so
        // its track takes it straight on; made free again, its launch goes
        // back to being a free cell's (`free_cells_follow`).
        let mut compile = was_free != self.free_ids;
        for n in 0..LOOPS {
            let id = was_free[n];
            if id == 0 || self.free_ids[n] != 0 || ids[n] != id || !now[n].pass {
                continue;
            }
            let p = &self.project;
            let cells =
                |&(t, s): &(usize, usize)| p.clip(t, s).and_then(Clip::loop_id) == Some(LoopId(id));
            for (track, slot) in self
                .free_on
                .iter()
                .copied()
                .filter(cells)
                .collect::<Vec<_>>()
            {
                self.launches.retain(|l| l.track != track);
                self.launches.push(Launch {
                    track,
                    slot,
                    origin: now[n].origin,
                    start: now[n].origin,
                });
                lines.push(format!(
                    "Loop {} plays on as a song loop on {}, row {}",
                    n + 1,
                    self.project.tracks[track].name,
                    slot + 1
                ));
            }
            self.free_on.retain(|f| !cells(f));
        }
        // What is seen now, but for a first pass waiting for its id.
        let mut seen = *now;
        for (n, (&is, was)) in now.iter().zip(self.loops_seen).enumerate() {
            if is == was {
                continue;
            }
            let k = n as u8;
            // A loop whose id the project names already is that cell's
            // or block's loop (an opened project's, or one brought back):
            // it lands nowhere new, and its cells follow its length.
            let named =
                loops_on && ids[n] != 0 && self.project.loop_track(LoopId(ids[n])).is_some();
            match (was.pass, is.pass) {
                (false, true) if !loops::in_cell(&self.project, k) && !named => {
                    let row = self.perform.playing().map_or(0, |p| p.row.min(ROWS - 1));
                    let (launches, cues) = (&self.launches, &self.cues);
                    let busy = |t: usize| {
                        launches.iter().any(|l| l.track == t) || cues.iter().any(|c| c.track == t)
                    };
                    // A redo of the pass that went goes back to its cell,
                    // if that is free and its Loop track plays nothing.
                    let gone_was = self.loops_gone[n];
                    let project = &mut self.project;
                    // Its lasting id, for an audio track's cell: one the
                    // project names nowhere yet.
                    let id = Some(LoopId(ids[n]))
                        .filter(|id| loops_on && id.0 != 0 && project.loop_track(*id).is_none());
                    let takes = |tr: &Track| tr.is_loops() || (id.is_some() && tr.is_audio());
                    let back = self.loops_gone[n].take().and_then(|((t, s), went)| {
                        let free = project
                            .tracks
                            .get(t)
                            .is_some_and(|tr| takes(tr) && tr.slots[s].is_none());
                        (went == is && free && !busy(t)).then_some((t, s))
                    });
                    // Else the armed Loop cell takes it, if it is still
                    // empty.
                    let loop_cell = &mut self.loop_cell;
                    let armed = || {
                        let cell = loop_cell.take();
                        let free = cell.filter(|&(t, s)| {
                            project
                                .tracks
                                .get(t)
                                .is_some_and(|tr| takes(tr) && tr.slots[s].is_none())
                        });
                        // An audio track's cell with no id to give: the
                        // arm stays, and the loop lands as before.
                        if free.is_none() {
                            *loop_cell = cell.filter(|&(t, s)| {
                                project.tracks.get(t).is_some_and(|tr| {
                                    tr.is_audio() && loops_on && tr.slots[s].is_none()
                                })
                            });
                        }
                        free
                    };
                    let name = format!("Loop {}", n + 1);
                    let landed = match back.or_else(armed) {
                        Some((t, s)) => {
                            let clip = match id.filter(|_| project.tracks[t].is_audio()) {
                                Some(id) => Clip::new_audio_loop(id, &name, is.ticks),
                                None => Clip::new_loop(k, is.ticks),
                            };
                            let _ = project.put_new_clip(t, s, clip);
                            Some((t, s, false))
                        }
                        // No cell for it (4e.4): an audio track's, by its
                        // id, once it has one.
                        None if loops_on => match id {
                            Some(id) => loops::place_audio(
                                project,
                                id,
                                &name,
                                is.ticks,
                                row,
                                audio_armed,
                                busy,
                                room,
                            )
                            .map(|l| (l.track, l.slot, l.new_track)),
                            None => {
                                // Seen again once its id comes.
                                seen[n] = was;
                                self.loops_gone[n] = gone_was;
                                continue;
                            }
                        },
                        None => loops::place(project, k, is.ticks, row, busy)
                            .map(|(t, s)| (t, s, false)),
                    };
                    match landed {
                        Some((track, slot, new_track)) => {
                            // One clip plays a track: what it played stops
                            // as the loop plays on there.
                            self.launches.retain(|l| l.track != track);
                            self.cues.retain(|c| c.track != track);
                            if is.free {
                                // A free cell: the loop plays on, on the
                                // free clock, launching itself (4e.4).
                                self.free_on.retain(|f| f.0 != track);
                                self.free_on.push((track, slot));
                            } else {
                                self.launches.push(Launch {
                                    track,
                                    slot,
                                    origin: is.origin,
                                    start: is.origin,
                                });
                            }
                            lines.push(format!(
                                "Loop {} on {}{}, row {}, playing",
                                n + 1,
                                self.project.tracks[track].name,
                                if new_track { " (a new audio track)" } else { "" },
                                slot + 1
                            ));
                            compile = true;
                        }
                        None if loops_on => lines.push(format!(
                            "Loop {} has no cell: no audio track playing nothing has an empty cell for it, and no audio track can be added",
                            n + 1
                        )),
                        None => lines.push(format!(
                            "Loop {} is on no Loop track: the project has {} tracks",
                            n + 1,
                            self.project.tracks.len()
                        )),
                    }
                }
                (_, true) => {
                    let mut changed = loops::set_length(&mut self.project, k, is.ticks);
                    if let Some(id) = Some(LoopId(ids[n])).filter(|id| loops_on && id.0 != 0) {
                        changed += loops::set_audio_length(&mut self.project, id, is.ticks);
                    }
                    compile |= changed > 0;
                }
                (true, false) => {
                    let id = Some(LoopId(was_ids[n])).filter(|id| loops_on && id.0 != 0);
                    let first = loops::cells(&self.project, k)
                        .next()
                        .or_else(|| id.and_then(|id| loops::audio_cells(&self.project, id).next()));
                    self.loops_gone[n] = first.map(|cell| (cell, was));
                    let mut gone = loops::remove(&mut self.project, k);
                    if let Some(id) = id {
                        gone += loops::remove_audio(&mut self.project, id);
                    }
                    if gone > 0 {
                        lines.push(format!("Loop {} cleared: its cells are empty", n + 1));
                        compile = true;
                    }
                }
                (false, false) => {}
            }
        }
        self.loops_seen = seen;
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

    /// The recordings its audio clips name (the audio tracks rethink,
    /// phase 2), sorted, each once: in the project, in what UNDO or REDO
    /// could bring back, and in the clip COPY holds. The audio tracks keep
    /// their takes while they are named ([`crate::audiotracks::TrackHost::keep_named`]).
    pub fn recordings(&self) -> Vec<RecId> {
        let mut v: Vec<RecId> = self
            .project
            .recordings()
            .chain(self.takes.recordings())
            .chain(self.clipboard.iter().flat_map(Clip::recordings))
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The scene card's Delete row (5.2): row `row` goes, and the grid's
    /// rows under it move up one, the last row empty. Its clips go (the
    /// song keeps those it plays, as the song's own) and its scene with
    /// them. The song list's steps of that row go, and those of the rows
    /// that moved follow them, as the song's parts do (a part of that row
    /// is the song's own now) and the morph's rows. A launch or a cue of
    /// a clip that moved follows it; one of the row's stops there. One
    /// take, which UNDO takes back whole. The card closes.
    fn delete_row(&mut self, row: usize) -> Result<String, String> {
        if row >= ROWS {
            return Err(format!("clips: no row {}", row + 1));
        }
        let before = self.project.clone();
        let p = &mut self.project;
        let name = match p.scene(row).map(|s| s.name.as_str()) {
            Some(n) if !n.is_empty() => format!("row {} ({n})", row + 1),
            _ => format!("row {}", row + 1),
        };
        let mut kept = 0;
        for t in 0..p.tracks.len() {
            let gone = p.tracks[t].slots.get_mut(row).and_then(Option::take);
            if let Some(c) = gone {
                if p.arrangement.plays(c.id) {
                    p.arrangement
                        .clips
                        .keep(c)
                        .map_err(|_| "clips: the song has no room for its clips".to_string())?;
                    kept += 1;
                }
            }
            for s in row..ROWS - 1 {
                let next = p.tracks[t].slots.get_mut(s + 1).and_then(Option::take);
                if let Some(slot) = p.tracks[t].slots.get_mut(s) {
                    *slot = next;
                }
            }
        }
        let mut scenes = p.scenes.clone();
        scenes.resize(ROWS.max(scenes.len()), mpfree_seq::Scene::default());
        for s in row..ROWS - 1 {
            scenes[s] = scenes[s + 1].clone();
        }
        scenes[ROWS - 1] = mpfree_seq::Scene::default();
        while scenes
            .last()
            .is_some_and(|s| *s == mpfree_seq::Scene::default())
        {
            scenes.pop();
        }
        p.scenes = scenes;
        let up = |r: usize| if r > row && r < ROWS { r - 1 } else { r };
        let song = &mut p.arrangement;
        // The list's steps of the row go; the step playing keeps its place.
        let gone: Vec<usize> = (0..song.list.steps.len())
            .filter(|&k| song.list.steps[k].row == row)
            .collect();
        for &k in gone.iter().rev() {
            self.perform.list_edited(k, false);
        }
        song.list.steps.retain(|s| s.row != row);
        for step in &mut song.list.steps {
            step.row = up(step.row);
        }
        for m in &mut song.markers {
            match m.scene {
                Some(f) if f.row == row => m.scene = None,
                Some(ref mut f) => f.row = up(f.row),
                None => {}
            }
        }
        for m in &mut song.morphs {
            m.a = up(m.a);
            m.b = up(m.b);
        }
        // What plays follows the clips that moved; the row's own stop.
        self.rows_followed(|r| (r != row).then(|| up(r)));
        self.forget_saved(None);
        if let Err(e) = self.compile() {
            self.project = before;
            self.compile()?;
            return Err(e);
        }
        self.takes.rows_song_edit(
            &before,
            &self.project,
            mpfree_play::RowsWhat::Deleted { row },
            SongChange::RowDeleted { row },
        );
        self.scene_open = None;
        let song = match kept {
            0 => String::new(),
            1 => "; the song keeps playing its clip".to_string(),
            _ => "; the song keeps playing its clips".to_string(),
        };
        Ok(format!(
            "clips: {name} deleted, the rows under it up one{song} (UNDO brings it back)"
        ))
    }

    /// Capture scene (the Arrange refactor's 5.3): a copy of the clip
    /// each track plays now (its launched clip, or in song mode the
    /// song's block under the playhead; nothing from an audio track) goes
    /// into the first empty row under the scene playing, from row 1 with
    /// none playing, as one edit of whole rows that UNDO takes back. The
    /// copies are clips of their own, so what plays plays on; the scene
    /// editor shows the new row. A muted track's clip goes in too: the
    /// row keeps the combination, and its mutes are the scene's to set.
    fn capture_scene(&mut self) -> Result<String, String> {
        let playing = self.perform.playing().map(|p| p.row);
        let from = playing.map_or(0, |r| r + 1);
        let row = (from..ROWS)
            .find(|&r| self.project.row_empty(r))
            .ok_or_else(|| match playing {
                Some(r) => format!(
                    "capture: no empty row under row {}: empty one, or move a row away, first",
                    r + 1
                ),
                None => "capture: no empty row: empty one first".to_string(),
            })?;
        let now = self.info.position;
        let song = &self.project.arrangement;
        let mut copies = Vec::new();
        for (t, track) in self.project.tracks.iter().enumerate() {
            if track.is_audio() {
                continue;
            }
            let clip = match self.launches.iter().find(|l| l.track == t) {
                Some(l) => self.project.clip(t, l.slot),
                None if self.song => song
                    .at(t, now)
                    .and_then(|k| song.clip_of(&self.project.tracks, &song.placements[k])),
                None => None,
            };
            if let Some(c) = clip {
                copies.push((t, c.copied()));
            }
        }
        if copies.is_empty() {
            return Err("capture: nothing plays, so there's nothing to keep".to_string());
        }
        let names = names_of(
            copies
                .iter()
                .map(|(t, _)| self.project.tracks[*t].name.as_str()),
        );
        let before = self.project.clone();
        for (t, mut c) in copies {
            c.id = self.project.new_clip_id();
            if let Some(slot) = self
                .project
                .tracks
                .get_mut(t)
                .and_then(|tr| tr.slots.get_mut(row))
            {
                *slot = Some(c);
            }
        }
        if let Err(e) = self.compile() {
            // Back as it was, but for the clip ids it gave: an id is
            // never given twice.
            let next = self.project.next_clip;
            self.project = before;
            self.project.next_clip = next;
            self.compile()?;
            return Err(e);
        }
        self.takes.rows_edit_as(
            &before,
            &self.project,
            mpfree_play::RowsWhat::Captured { row },
        );
        self.scene_pick = Some(row);
        // The scene card, open, shows the new row.
        self.scene_open = self.scene_open.map(|_| row);
        self.changed = true;
        Ok(format!(
            "capture: {names} into row {}, as copies (UNDO takes it back)",
            row + 1
        ))
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
        if self.changed {
            // The note editor's picks follow what changed, before the
            // screens get it, and so do the clips it shows behind.
            self.edit_follow();
            self.ghosts_follow();
        }
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

    /// The song list's step playing, or waiting for its line (Scenes →
    /// song and List → song follow it, [`super::offline`]).
    pub(super) fn list_playing(&self) -> Option<mpfree_perform::ListPlaying> {
        self.perform.list()
    }

    /// Rec song is armed (it disarms itself when the song would be full).
    pub(super) fn song_rec_armed(&self) -> bool {
        self.song_rec.armed()
    }

    /// Everything the screens show.
    pub fn state(&self) -> TimeState {
        // Free cells playing show as launched (phase 4e.4).
        let shown;
        let launches = if self.free_on.is_empty() {
            &self.launches
        } else {
            shown = self
                .launches
                .iter()
                .copied()
                .chain(self.free_on.iter().map(|&(track, slot)| Launch {
                    track,
                    slot,
                    origin: 0,
                    start: 0,
                }))
                .collect::<Vec<_>>();
            &shown
        };
        let mut st =
            TimeState::from_engine(self.info, &self.project, launches, self.armed, &self.takes);
        st.loops_on = self.loops_on;
        st.track_pick = self.track_pick;
        st.clip_pick = self.clip_picked();
        st.continuing = self.compose.screen();
        st.adding = self.add.screen();
        st.unfit = self.unfit.clone();
        st.row_chords = self.add.heard.clone();
        st.styles = Arc::clone(&self.styles);
        st.song_chords = self.song_hear.shown();
        st.style_sheet = self.style_sheet.clone();
        st.also = self.also();
        st.capture = self.pending.map(|p| bars_label(p.bars));
        st.clipboard = self.clipboard.as_ref().map(|c| c.name.clone());
        st.notes_copied = self.notes_copy.as_ref().map(|c| c.said.clone());
        st.cues = self.cues.clone();
        st.mute_wait = self.mute_wait().cloned();
        st.audio_waits = self.audio_waits.counts();
        st.audio_late = self.audio_waits.late();
        st.audio_focus = self.audio_armed();
        // Launched again since: no longer "not ready".
        st.not_ready = self
            .not_ready
            .iter()
            .filter(|n| {
                !self.cues.iter().any(|c| c.track == n.0)
                    && !self
                        .launches
                        .iter()
                        .any(|l| (l.track, l.slot) == (n.0, n.1))
            })
            .copied()
            .collect();
        st.saved = self.saved.iter().map(|s| (s.0, s.1)).collect();
        st.converting = self.convert_asked.as_ref().map(|c| Converting {
            track: c.track,
            slot: c.slot,
            ok: c.made.is_ok() && !self.info.state.rolling(),
            line: c.line.clone(),
        });
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
        st.scene_open = self.scene_open;
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
        st.list_linked = self.project.arrangement.list.linked;
        st.list_next = self
            .perform
            .list_next(&self.project, self.info.meter.bar_ticks());
        st.song_on = self.song;
        st.selected = self.selected;
        let blocks = self.project.arrangement.placements.len();
        st.picks = self.picks.iter().copied().filter(|&i| i < blocks).collect();
        if st.picks.len() < 2 {
            st.picks.clear();
        }
        st.region = self.region;
        st.section = self.section;
        st.tempo_base = self.project_bpm;
        st.tempo_pick = self.tempo_pick.filter(|&k| k < st.tempo_points.len());
        st.report = self.report.clone();
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
            // The copy the editor last knew, when the clip is still that
            // (always, after `take_changed`): no copy of the clip each time.
            let clip = match &self.edit_seen {
                Some(seen) if **seen == *clip => Arc::clone(seen),
                _ => Arc::new(clip.clone()),
            };
            Some(Box::new(EditInfo {
                editor: ed.clone(),
                clip,
                rev: self.edit_rev,
                track_name: self.project.tracks[ed.track].name.clone(),
                bar: bar.round() as Tick,
                answer: self.edit_answer.clone(),
                locks: Default::default(),
                ghosts: self.ghost_seen.clone(),
            }))
        });
        st
    }

    /// The song list's steps as the screen shows them, with the scenes'
    /// names `scenes` (a plain row is "Row 3").
    fn list_rows(&self, scenes: &[String]) -> Vec<ListRow> {
        let bar = self.info.meter.bar_ticks().max(1);
        let now = self.info.position;
        if self.project.arrangement.list.linked {
            return self.linked_rows(bar, now);
        }
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
                scene: Some(s.row),
                times: s.times,
                bars: len / bar,
                starts: at / bar + 1,
                playing: pass.filter(|p| p.0 == k).map(|p| (p.1, p.2)),
                waiting: waiting == Some(k),
                lead: false,
                changed: false,
            })
            .collect()
    }

    /// A linked list's rows (the Arrange refactor's phase 4b): the song's
    /// sections ([`song_sections`]), each with its marker's name ("Song"
    /// for the bars before the first marker), its scene and its passes;
    /// the one the song plays now is lit with its pass.
    fn linked_rows(&self, bar: Tick, now: Tick) -> Vec<ListRow> {
        let song = &self.project.arrangement;
        let rolling = self.song && self.info.state.rolling();
        song_sections(&self.project, bar)
            .into_iter()
            .map(|s| {
                let span = (s.end - s.start).max(1);
                let playing = (rolling && (s.start..s.end).contains(&now)).then(|| {
                    let into = now - s.start;
                    let pass = (into / s.pass().max(1)) as u16 + 1;
                    (pass.min(s.passes), into as f32 / span as f32)
                });
                ListRow {
                    name: s
                        .marker
                        .map_or_else(|| "Song".to_string(), |k| song.markers[k].name.clone()),
                    scene: s.scene.map(|from| from.row),
                    times: s.passes,
                    bars: (span + bar - 1) / bar,
                    starts: s.start / bar + 1,
                    playing,
                    waiting: false,
                    lead: s.marker.is_none(),
                    changed: s.scene.is_some_and(|f| f.changed),
                }
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
        if let Some(c) = self.compose.counts.describe() {
            line.push_str("; ");
            line.push_str(&c);
        }
        if let Some(c) = self.add.counts.describe() {
            line.push_str("; ");
            line.push_str(&c);
        }
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
        // Recorded moves of a synth's control kept out of a clip whose
        // locks were made for another instrument (refactor F1).
        if self.inst_refused > 0 {
            line.push_str(&format!(
                "; {} recorded moves left out of patterns with locks for another instrument",
                self.inst_refused
            ));
        }
        // Clips' locks on "this track's strip" on a track whose instrument
        // sounds elsewhere (strips.rs), when there were any.
        let st = &self.strips;
        if st.any() {
            line.push_str(&format!(
                "; this track's strip: {} locks went to their instrument's strip, {} waited \
                 for a track before them, {} stayed on the track's own strip with the table full",
                st.moved, st.waited, st.full
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
        // Cues that waited for their audio (streaming on).
        let w = self.audio_waits.counts();
        if w.kept > 0 {
            line.push_str(&format!(
                "; cues waiting for their audio: {} kept back, {} held in time, {} moved to a \
                 later line, {} cancelled, {} given back, {} waiting now",
                w.kept, w.held, w.moved, w.cancelled, w.given_back, w.waiting
            ));
        }
        // Song edits UNDO took back while the audio regions stayed (their
        // history no longer held the edit; Arrange 3.4).
        if self.audio_stayed > 0 {
            line.push_str(&format!(
                "; {} song edits undone with the audio regions left where they were",
                self.audio_stayed
            ));
        }
        // Takes over a block that took the whole block, not its part
        // (audio tracks phase 2, step 2d).
        if self.audio_took_whole > 0 {
            line.push_str(&format!(
                "; {} recordings over a block took the whole block (no room to cut it)",
                self.audio_took_whole
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

/// The join key of the Song view's + Bar on section `k` (this, or `k`):
/// presses in a row on one section are one take.
const ROOM_BEFORE_JOIN: u64 = 0xBA25 << 32;

/// Why an edit of the song's time was refused, in a musician's words,
/// with `at` naming a place in the song.
fn refused_said(r: Refused, at: impl Fn(Tick) -> String) -> String {
    use mpfree_seq::arrange::{MAX_MARKERS, MAX_MORPHS, MAX_PLACEMENTS, MAX_TEMPO_POINTS};
    match r {
        Refused::BadRange => "there's no such stretch of the song".to_string(),
        Refused::Blocks => format!("the song would hold more than {MAX_PLACEMENTS} blocks"),
        Refused::Markers => format!("the song would hold more than {MAX_MARKERS} markers"),
        Refused::TempoPoints => {
            format!("the song would need more than {MAX_TEMPO_POINTS} tempo points")
        }
        Refused::TempoTooClose(t) => {
            format!("two tempo points would be closer than a 1/16 at {}", at(t))
        }
        Refused::Lanes => "the song's moves would have too many points".to_string(),
        Refused::Morphs => format!("the morph would change rows more than {MAX_MORPHS} times"),
        Refused::Check(e) => format!("the song wouldn't add up ({e:?})"),
    }
}

/// "block" or "blocks", for `n` of them.
fn blocks_said(n: usize) -> &'static str {
    if n == 1 {
        "block"
    } else {
        "blocks"
    }
}

/// A stretch of `t` song ticks (above 0) as a musician counts it: "2
/// bars", "1 beat", "3/16", else in ticks.
/// How many blocks a picked set names, an index given twice counting
/// once (as the model counts them).
fn distinct(set: &[usize]) -> usize {
    let mut set = set.to_vec();
    set.sort_unstable();
    set.dedup();
    set.len()
}

fn ticks_said(t: Tick, meter: mpfree_transport::time::Meter) -> String {
    let bar = meter.bar_ticks().max(1);
    let beat = meter.beat_ticks().max(1);
    let plural = |n: Tick, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    if t % bar == 0 {
        plural(t / bar, "bar")
    } else if t % beat == 0 {
        plural(t / beat, "beat")
    } else if beat % 4 == 0 && t % (beat / 4) == 0 {
        format!("{}/16", t / (beat / 4))
    } else {
        plural(t, "tick")
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

/// Step action `a` on a clip of notes ([`on_rows`]), with the notes it
/// puts on a pad's rows made that pad's trigger group (trigger groups,
/// `mpfree_seq::groups`): on a pad row a step is the pad's trigger, so
/// its notes take the pad's number and the root's chance, condition and
/// on, and decide together as the pad's hit does. A tap that puts a
/// chord's missing notes back joins the chord already on the step; other
/// notes put where the pad's number is taken on their tick get a free
/// one, so two gestures never merge. Refused (no change) when a tick has
/// no number left.
fn on_notes(
    grid: Grid,
    clip: &Clip,
    a: StepAction,
    rows: &PadRows,
    act: impl Fn(StepAction) -> (mpfree_seq::steps::Edit, Option<u64>, String),
) -> (mpfree_seq::steps::Edit, Option<u64>, String) {
    let (mut edit, key, line) = on_rows(grid, clip, a, rows, act);
    // The pads whose rows the action puts notes on, and whether a tap's
    // notes join the pad's chord already there.
    let (pads, join): (Vec<u8>, bool) = match a {
        StepAction::Tap { pad, .. } if pad != LINE => (vec![pad], true),
        StepAction::Change {
            change: mpfree_seq::steps::Change::Note(to),
            pad,
            ..
        }
        | StepAction::ChangeSteps {
            change: mpfree_seq::steps::Change::Note(to),
            pad,
            ..
        } if pad != LINE => (vec![to], false),
        StepAction::Change { pad, .. }
        | StepAction::ChangeSteps { pad, .. }
        | StepAction::Values { pad, .. }
        | StepAction::CopySteps { pad, .. }
            if pad != LINE =>
        {
            (vec![pad], false)
        }
        StepAction::Enter { keys, n, .. } | StepAction::Keys { keys, n, .. } => (
            keys[..usize::from(n).min(super::MAX_PEN)]
                .iter()
                .map(|k| k.0)
                .collect(),
            false,
        ),
        _ => (Vec::new(), false),
    };
    if pads.is_empty() || edit.add.is_empty() {
        return (edit, key, line);
    }
    // Each added note on a pad's rows: (index, the pad's number, whether
    // it is the pad's root).
    let mut claimed: Vec<(usize, u8, bool)> = Vec::new();
    for &pad in &pads {
        let notes = rows.notes(usize::from(pad));
        if notes.is_empty() || usize::from(pad) >= 16 {
            continue;
        }
        let root = notes.last().copied();
        for (i, n) in edit.add.iter().enumerate() {
            if notes.contains(&n.note) && !claimed.iter().any(|c| c.0 == i) {
                claimed.push((i, mpfree_seq::groups::pad_group(pad), Some(n.note) == root));
            }
        }
    }
    if claimed.is_empty() {
        return (edit, key, line);
    }
    // One trigger per pad and tick: the root's settings (the first note's
    // where the root isn't put in).
    type Shared = (u8, mpfree_seq::Cond, bool);
    let mut model: Vec<((Tick, u8), Shared)> = Vec::new();
    for pass in [true, false] {
        for &(i, g, root) in &claimed {
            let n = edit.add[i];
            if root == pass && !model.iter().any(|(k, _)| *k == (n.tick, g)) {
                model.push(((n.tick, g), (n.chance, n.cond, n.on)));
            }
        }
    }
    let mut mine: Vec<mpfree_seq::Note> = claimed
        .iter()
        .map(|&(i, g, _)| {
            let n = edit.add[i];
            let (chance, cond, on) = model
                .iter()
                .find(|(k, _)| *k == (n.tick, g))
                .map(|(_, m)| *m)
                .expect("each claimed note has its trigger");
            mpfree_seq::Note {
                group: g,
                chance,
                cond,
                on,
                ..n
            }
        })
        .collect();
    if mpfree_seq::groups::settle_new(clip, &edit.erase, &mut mine, join).is_err() {
        return (mpfree_seq::steps::Edit::default(), key, line);
    }
    for (&(i, _, _), n) in claimed.iter().zip(mine) {
        edit.add[i] = n;
    }
    (edit, key, line)
}

/// Step action `a` on a clip of notes, whose pad rows stand for the notes
/// `rows` gives each pad (step 5c), as `act` takes an action on a clip of
/// hits: on each of the pad's notes, root first, joined as one edit, with
/// the root's undo key and log line. A tap takes a chord pad's notes off
/// the step when its root is on it, else puts the missing ones on; the
/// Note knob puts the step on another pad's notes (`swap`); keys entered
/// on the line are their pads' notes. The line and the rows of notes no
/// pad plays go to `act` as they are.
fn on_rows(
    grid: Grid,
    clip: &Clip,
    a: StepAction,
    rows: &PadRows,
    act: impl Fn(StepAction) -> (mpfree_seq::steps::Edit, Option<u64>, String),
) -> (mpfree_seq::steps::Edit, Option<u64>, String) {
    use super::MAX_PEN;
    use mpfree_seq::steps::Change;
    // A row's notes, root first: a pad's, or the row's own note.
    let of = |pad: u8| -> Vec<u8> {
        match rows.notes(usize::from(pad)) {
            [] => vec![pad],
            n => n.iter().rev().copied().collect(),
        }
    };
    let has = |note: u8, i: usize| !grid.notes(clip, note, i).is_empty();
    // `with` each of `notes` (root first), joined.
    let each = |notes: &[u8], with: &dyn Fn(u8) -> StepAction| {
        let mut out = act(with(notes[0]));
        for &n in &notes[1..] {
            out.0.join(act(with(n)).0);
        }
        out
    };
    let keys = |keys: [(u8, u8); MAX_PEN], n: u8| {
        let mut out = [(0u8, 0u8); MAX_PEN];
        let mut m = 0;
        for &(pad, velocity) in &keys[..usize::from(n).min(MAX_PEN)] {
            for note in of(pad) {
                if m < MAX_PEN && !out[..m].iter().any(|k| k.0 == note) {
                    out[m] = (note, velocity);
                    m += 1;
                }
            }
        }
        (out, m as u8)
    };
    match a {
        StepAction::Tap {
            pad,
            step,
            velocity,
        } if pad != LINE => {
            let notes = of(pad);
            let on = has(notes[0], step);
            let notes: Vec<u8> = notes.into_iter().filter(|&n| has(n, step) == on).collect();
            each(&notes, &|n| StepAction::Tap {
                pad: n,
                step,
                velocity,
            })
        }
        StepAction::Change {
            pad,
            step,
            change: Change::Note(to),
        } if pad != LINE => {
            let (from, to) = (of(pad), of(to));
            let named = act(StepAction::Change {
                pad: from[0],
                step,
                change: Change::Note(to[0]),
            });
            if from.len() == 1 && to.len() == 1 {
                return named;
            }
            (swap(grid, clip, &from, &to, step), named.1, named.2)
        }
        StepAction::ChangeSteps {
            pad,
            steps,
            change: Change::Note(to),
        } if pad != LINE => {
            let (from, to) = (of(pad), of(to));
            let named = act(StepAction::ChangeSteps {
                pad: from[0],
                steps,
                change: Change::Note(to[0]),
            });
            if from.len() == 1 && to.len() == 1 {
                return named;
            }
            let mut edit = mpfree_seq::steps::Edit::default();
            for i in step_list(steps) {
                edit.join(swap(grid, clip, &from, &to, i));
            }
            (edit, named.1, named.2)
        }
        StepAction::Change { pad, step, change } if pad != LINE => {
            each(&of(pad), &|n| StepAction::Change {
                pad: n,
                step,
                change,
            })
        }
        StepAction::ChangeSteps { pad, steps, change } if pad != LINE => {
            each(&of(pad), &|n| StepAction::ChangeSteps {
                pad: n,
                steps,
                change,
            })
        }
        StepAction::Lock {
            pad,
            steps,
            addr,
            value,
        } if pad != LINE => each(&of(pad), &|n| StepAction::Lock {
            pad: n,
            steps,
            addr,
            value,
        }),
        StepAction::ClearLocks { pad, steps } if pad != LINE => {
            each(&of(pad), &|n| StepAction::ClearLocks { pad: n, steps })
        }
        // A lane's stroke (refactor D3) on each of the pad's notes.
        StepAction::Values {
            pad,
            from,
            kind,
            values,
        } if pad != LINE => each(&of(pad), &|n| StepAction::Values {
            pad: n,
            from,
            kind,
            values,
        }),
        StepAction::ClearPad { pad } if pad != LINE => {
            each(&of(pad), &|n| StepAction::ClearPad { pad: n })
        }
        StepAction::ClearSteps { pad, from, to } if pad != LINE => {
            each(&of(pad), &|n| StepAction::ClearSteps { pad: n, from, to })
        }
        StepAction::CopySteps {
            pad,
            from,
            to,
            count,
        } if pad != LINE => each(&of(pad), &|n| StepAction::CopySteps {
            pad: n,
            from,
            to,
            count,
        }),
        StepAction::Enter { step, keys: k, n } => {
            let (keys, n) = keys(k, n);
            act(StepAction::Enter { step, keys, n })
        }
        StepAction::Keys { steps, keys: k, n } => {
            let (keys, n) = keys(k, n);
            act(StepAction::Keys { steps, keys, n })
        }
        a => act(a),
    }
}

/// Step `i`'s notes of `from` (a pad's notes, root first) moved onto `to`
/// (another pad's) on a clip of notes, as the Note knob moves a step of
/// pad hits to another pad: each new note as the step's root was (its
/// velocity, length and the rest), its locks with it; the locks of a note
/// with no new one go, as step entry drops a gone sound's. Refused (no
/// change) where the step has none of `from`, or already has a note of
/// `to` that isn't moving.
fn swap(grid: Grid, clip: &Clip, from: &[u8], to: &[u8], i: usize) -> mpfree_seq::steps::Edit {
    let old: Vec<mpfree_seq::Note> = from.iter().flat_map(|&n| grid.notes(clip, n, i)).collect();
    let Some(&model) = old.first() else {
        return mpfree_seq::steps::Edit::default();
    };
    if to
        .iter()
        .any(|&m| !from.contains(&m) && !grid.notes(clip, m, i).is_empty())
    {
        return mpfree_seq::steps::Edit::default();
    }
    let mut add: Vec<mpfree_seq::Note> = to
        .iter()
        .map(|&m| mpfree_seq::Note { note: m, ..model })
        .collect();
    let mut erase = old;
    // A note already there as it would be put stays.
    erase.retain(|n| match add.iter().position(|a| a == n) {
        Some(j) => {
            add.remove(j);
            false
        }
        None => true,
    });
    let mut edit = mpfree_seq::steps::Edit {
        erase,
        add,
        ..mpfree_seq::steps::Edit::default()
    };
    for (k, &n) in from.iter().enumerate() {
        for l in grid.locks(clip, Some(n), i) {
            match to.get(k) {
                Some(&m) if m == n => {}
                Some(&m) => {
                    edit.erase_locks.push(l);
                    edit.add_locks.push(mpfree_seq::Lock {
                        sound: Some(m),
                        ..l
                    });
                }
                None => edit.erase_locks.push(l),
            }
        }
    }
    edit
}

/// The steps whose bits are set (step 0 the lowest).
/// A step row as the log names it: "pad 5", or "the line".
/// The instrument control recorded change `rec` locks or moves, if it
/// is one (refactor F1).
fn rec_param(rec: &mpfree_play::Rec) -> Option<u16> {
    match rec {
        mpfree_play::Rec::Lock(k) if mpfree_seq::inst::on_inst(&k.lock) => Some(k.lock.addr.param),
        mpfree_play::Rec::Move(k) => match k.target {
            mpfree_seq::mpfree_moves::Target::Track(a)
                if a.on == mpfree_play::mpfree_param::OnTrack::Inst =>
            {
                Some(a.param)
            }
            _ => None,
        },
        _ => None,
    }
}

/// Whether recorded change `rec` locks or moves an instrument control.
fn rec_inst(rec: &mpfree_play::Rec) -> bool {
    rec_param(rec).is_some()
}

/// On a clip of notes (`notes`) a row is its note: "note C3".
fn row_name(pad: u8, notes: bool) -> String {
    if pad == LINE {
        "the line".to_string()
    } else if notes {
        format!("note {}", crate::synth::note_name(pad))
    } else {
        format!("pad {}", pad + 1)
    }
}

/// A step change for the lines: as Debug says it, but the Note knob's pad
/// counted from 1, as the lines count pads (named as a note on a clip of
/// notes, `notes`).
fn change_name(change: mpfree_seq::steps::Change, notes: bool) -> String {
    match change {
        mpfree_seq::steps::Change::Note(to) if notes => {
            format!("lowest note {}", crate::synth::note_name(to))
        }
        mpfree_seq::steps::Change::Note(to) => format!("lowest note on pad {}", u16::from(to) + 1),
        c => format!("{c:?}"),
    }
}

fn step_list(steps: u128) -> Vec<usize> {
    (0..128).filter(|i| steps & (1u128 << i) != 0).collect()
}

/// "step 5", or "steps 1, 5".
/// Names for a log line: "Keys", "Keys and Bass", "Keys, Bass and
/// Drums", or "Keys, Bass and 3 more".
fn names_of<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let names: Vec<&str> = names.collect();
    match names.as_slice() {
        [] => String::new(),
        [a] => a.to_string(),
        [a, b] => format!("{a} and {b}"),
        [a, b, c] => format!("{a}, {b} and {c}"),
        [a, b, rest @ ..] => format!("{a}, {b} and {} more", rest.len()),
    }
}

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

/// The report card of a conversion whose result plays differently from
/// what it was made from (the Arrange refactor's 4.3): its differences in
/// song order, the song's own first in each bar. None for an exact one,
/// which says so on its line alone.
fn report(
    title: &str,
    made: String,
    lead: String,
    d: &[Difference],
    bar: Tick,
) -> Option<Arc<Report>> {
    if d.is_empty() {
        return None;
    }
    let mut rows: Vec<ReportRow> = d
        .iter()
        .map(|d| ReportRow {
            track: d.track,
            bar: d.bar(bar),
            what: d.what_said(bar),
        })
        .collect();
    rows.sort_by_key(|r| (r.bar, r.track));
    Some(Arc::new(Report {
        title: title.into(),
        made,
        lead,
        rows,
    }))
}

/// How an exact conversion's line ends: nothing opens, and the line shows
/// in place of the hint as it is (layout spec 4n, main.rs `time_said`).
pub const EXACT: &str = ", exact. UNDO takes it back.";

/// How a conversion played out, the end of its log line (plan D7):
/// [`EXACT`], or how many differences and each in words (", 2 differences
/// (UNDO takes it back): Bass, bar 9: a note held over the row change is
/// cut there; …"), which the report card shows too.
fn conversion_said(project: &Project, d: &[Difference], bar: Tick) -> String {
    match d.len() {
        0 => EXACT.into(),
        n => {
            let each: Vec<String> = d.iter().map(|d| d.said(project, bar)).collect();
            let s = if n == 1 { "" } else { "s" };
            format!(
                ", {n} difference{s} (UNDO takes it back): {}",
                each.join("; ")
            )
        }
    }
}

/// The step grid of clip `c`: steps of its own size (refactor D1). A
/// clip can't hold another size (`Clip::check`, an opened project, and
/// `Clip::set_settings` refuse it); one written past them shows 1/16
/// steps, which debug builds refuse.
pub(crate) fn grid_of(c: &Clip) -> Grid {
    debug_assert!(
        mpfree_seq::steps::SIZES.contains(&c.step),
        "step {}",
        c.step
    );
    Grid::new(c.step)
        .filter(|_| mpfree_seq::steps::SIZES.contains(&c.step))
        .or_else(|| Grid::new(DEFAULT_SIZE))
        .expect("a 1/16 is a step size")
}

/// The blocks on audio tracks' lanes in `project`'s song that weren't in
/// `before` (Rec song's, [`TimeControl::keep_song`]): each one's span by
/// the track's lasting id, in order, joined where they meet.
fn audio_placed(
    project: &Project,
    before: &mpfree_seq::arrange::Arrangement,
) -> Vec<(TrackId, Tick, Tick)> {
    let mut out: Vec<(TrackId, Tick, Tick)> = Vec::new();
    for p in &project.arrangement.placements {
        let Some(track) = project.tracks.get(p.track).filter(|t| t.is_audio()) else {
            continue;
        };
        if before.placements.contains(p) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.0 == track.id && last.2 >= p.start => last.2 = last.2.max(p.end()),
            _ => out.push((track.id, p.start, p.end())),
        }
    }
    out
}

/// Where block `p` would cover a region on its audio track's lane (from
/// `spans`, as [`TimeControl::set_audio_spans`] keeps them): the first
/// such region's start, or the block's own where it starts inside one.
/// None off audio tracks, and where the lane is clear.
fn region_under(
    spans: &[(TrackId, Vec<(Tick, Tick)>)],
    tracks: &[mpfree_seq::Track],
    p: &mpfree_seq::Placement,
) -> Option<Tick> {
    let id = tracks.get(p.track).filter(|t| t.is_audio())?.id;
    let (_, regions) = spans.iter().find(|(t, _)| *t == id)?;
    let k = regions.partition_point(|r| r.1 <= p.start);
    regions
        .get(k)
        .filter(|r| r.0 < p.end())
        .map(|r| r.0.max(p.start))
}

/// Audio clip `a`, `from` ticks long, cut or lengthened to `to` ticks
/// (its card's Length, layout spec 4j), its regions laid out at its own
/// tempo at `rate` frames a second. Regions past `to` go; the one across
/// it ends there, the frames cut off kept in its `left`. Longer, the last
/// region, if it ended at `from` and was cut, grows back into what it was
/// cut from, as far as `to`. The last region's sound then ends with it.
fn cut_audio(a: &mut mpfree_seq::AudioClip, from: Tick, to: Tick, rate: u32) {
    let per_tick = f64::from(rate) * 60.0 / (a.bpm.max(1.0) * f64::from(PPQ));
    let frames = |ticks: Tick| (ticks as f64 * per_tick).round() as i64;
    let before = a.regions.last().copied();
    if to < from {
        a.regions.retain(|r| r.start < to);
        if let Some(r) = a.regions.last_mut().filter(|r| r.end > to) {
            let cut = frames(r.end - to).clamp(0, i64::from(u32::MAX)) as u32;
            r.left = r.left.saturating_add(cut);
            r.end = to;
        }
    } else if let Some(r) = a.regions.last_mut().filter(|r| r.end == from && r.left > 0) {
        let room = frames(to - r.end).clamp(0, i64::from(u32::MAX)) as u32;
        let back = room.min(r.left);
        let ticks = (f64::from(back) / per_tick).floor() as Tick;
        let grow = ticks.min(to - r.end);
        r.end += grow;
        r.left -= frames(grow).clamp(0, i64::from(r.left)) as u32;
    }
    if let Some(r) = a.regions.last().filter(|&&r| Some(r) != before) {
        a.last_frame = r.src + frames(r.end - r.start).max(1);
    }
}

/// Convert to notes' preview, open (E1): the clip and the pads it read,
/// and whether it would convert.
struct ConvertAsked {
    track: usize,
    slot: usize,
    clip: Clip,
    rows: PadRows,
    made: Result<mpfree_seq::convert::Converted, ()>,
    /// What it said: the hint line while it is open.
    line: String,
}

/// The notes the pads play, lowest root to highest, for Convert to notes'
/// preview (E1): "C3 to D#4".
fn pads_range(rows: &PadRows) -> String {
    let roots = (0..16).filter_map(|p| rows.notes(p).last().copied());
    match (roots.clone().min(), roots.max()) {
        (Some(lo), Some(hi)) if lo != hi => format!(
            "{} to {}",
            crate::synth::note_name(lo),
            crate::synth::note_name(hi)
        ),
        (Some(n), _) => crate::synth::note_name(n),
        _ => "playing nothing".to_string(),
    }
}

/// Why Convert to notes can't keep what a clip plays, in its preview's
/// words (E1): the steps on `grid` in a clip `length` ticks long.
fn refused_words(r: &mpfree_seq::convert::Refused, grid: Grid, length: Tick) -> String {
    use mpfree_seq::convert::Refused;
    let pad = |p: u8| crate::rec_arm::pad_name(usize::from(p));
    let step = |tick: Tick| grid.place(length, tick).0 + 1;
    let kept = |original: bool| {
        if original {
            " (in the original Restore brings back)"
        } else {
            ""
        }
    };
    match *r {
        Refused::Notes => "it uses notes already".to_string(),
        Refused::Loop => "a loop's cell holds a loop".to_string(),
        Refused::PadLock {
            pad: p,
            param,
            tick,
            original,
        } => format!(
            "{}'s {} on step {}{}: a pad's own setting has no note to follow, and the lane's × removes it",
            pad(p),
            crate::sampler::PadParam::of(param).map_or("setting", |p| p.info(1).name),
            step(tick),
            kept(original)
        ),
        Refused::NoNote { pad: p } => format!(
            "{} plays no note now: take its steps off, or move the pads so it plays one",
            pad(p)
        ),
        Refused::Overlap { a, b, note, tick } => format!(
            "{} and {} both play {} and sound together on step {}: as notes, one would cut the other short",
            pad(a),
            pad(b),
            crate::synth::note_name(note),
            step(tick)
        ),
        Refused::Room { notes } => format!(
            "it would hold {notes} notes, more than a clip holds ({})",
            mpfree_seq::MAX_NOTES
        ),
        Refused::LocksMeet { a, b, tick } => format!(
            "{} and {} play the same root, so their locks on step {} would land on one row: take one off",
            pad(a),
            pad(b),
            step(tick)
        ),
        Refused::Twice { pad: p, tick } => format!(
            "{} is hit twice on step {} with a different chance or condition: as notes, a pad's notes on one step decide together, so take one off or set them alike",
            pad(p),
            step(tick)
        ),
    }
}
