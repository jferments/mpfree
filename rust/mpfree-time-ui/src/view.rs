//! The time engine's screens: what they show, worked out from the engine,
//! and what their touches mean.
//!
//! - The **transport bar** sits under the header on the Arrange page:
//!   stop, play, record, replace or overdub, bar.beat.tick, tempo, meter,
//!   swing, loop and Undo (status chips on the right where a layout wants
//!   them; on Arrange the app's header carries the arp's chip).
//! - The **Arrange page** (the Clips page until it was renamed) is the
//!   launch grid (tracks across, their 16 clip slots down), the record
//!   settings, and the takes with Capture, Undo and Redo. Capture keeps
//!   what was just played (`mpfree-capture`). Play's Q-Link column runs
//!   down its right edge (layout spec 4c, Jay's option E): the app fills
//!   the cards and routes the knobs.
//!
//! The screens do not own the engine. The audio thread copies a
//! [`TransportInfo`] after each period; the control thread builds a
//! [`TimeState`] from it, the project, the launches and the takes, and
//! calls [`TimePage::update`]. Touches come back as [`TimeAction`]s for the
//! app to apply: transport commands go to the audio thread's queue, the
//! rest are edits on the control thread ([`clips_after`] works out the new
//! launches and cues). The header, the pads and the hint line belong to the
//! app.
//!
//! **Launching.** While the music plays, tapping a clip cues it: it starts
//! on the next bar line (its cell shows "from 6.1" in amber), and the
//! track's clip plays until then. Tapping the playing clip cues a stop the
//! same way. Tapping what is cued takes the cue back. While stopped, a tap
//! launches or stops at once.

use crate::editor::{
    drop_of, edit_rows, pads, step_name, EditAction, Editor, Lens, LockMark, NoteBox, NoteGrid,
    PEN, STEP_FALLBACK,
};
use crate::list::{
    ListRow, SongListView, LIST_ADD, LIST_CLIPS, LIST_LOOP, LIST_PLAY, LIST_REMOVE, LIST_SONG,
    LIST_TO_SONG, SCENE_STEP,
};
use crate::song::{
    AudioLane, SongBlocks, SongLanes, ADD_AUDIO, AUDIO_BUTTONS, BARS_BACK, BARS_ON, INPUTS,
    MONITORS, NONE, PANEL, PANEL_CLOSE, PANEL_INPUT, PANEL_MONITOR, PANEL_MUTE, PANEL_SOLO,
    PANEL_TAKES, REC, REGION_SLOTS, SECTION_TAP, TEMPO_ROW,
};
use crate::widgets::{
    BarButton, BarItem, CellState, ClipGrid, GridCell, GridTrack, MorphBar, Run, TakeItem,
    TakeList, TransportBar, ADD_TRACK, BACK_TO_SONG, GRID_COLS, PAGE_BACK, PAGE_ON, ROWS_BACK,
    ROWS_ON, ROW_DROP,
};
use mpfree_audiotracks::host::TakeWave;
use mpfree_audiotracks::{Input, Monitor};
use mpfree_play::mpfree_param::clips::{self, Clips};
use mpfree_play::mpfree_param::{Owner, ParamAddr};
use mpfree_play::{anchored, ClipDrop, Cue, Launch, Replaced, SongChange, Take, Takes};
use mpfree_seq::scenes::MAX_FOLLOW_BARS;
use mpfree_seq::{
    direction_steps, Clip, ClipProgram, ClipSettings, Direction, Follow, FollowAction, LaunchGrid,
    Project, Quantise, SceneSettings, Speed, TrackProgram, Transpose, LAUNCH_GRIDS, MAX_CLIP_TICKS,
    MAX_TRACKS, MAX_TRANSPOSE, SLOTS,
};
use mpfree_transport::time::{Meter, Swing, Tick, MAX_BPM, MIN_BPM, PPQ};
use mpfree_transport::transport::MAX_COUNT_IN;
use mpfree_transport::{Command, State, Transport};
use mpfree_ui::qlink::QLinkColumn;
use mpfree_ui::widgets::{Chip, OptionButton, OptionList, OptionRow};
use mpfree_ui::{Color, EventKind, Tone, Ui, UiEvent};
use std::sync::Arc;

/// Widget names the page adds. The app's own are `header` and `hint`.
pub const TRANSPORT: &str = "transport";
pub const CLIPS: &str = "clips";
pub const RECORD: &str = "record";
/// The clip settings card: the Record panel's Clip tab (sequencer S3).
pub const CLIP_CARD: &str = "clip-card";
/// The scene editor: the Record panel's Scene tab (sequencer S4).
pub const SCENE_CARD: &str = "scene-card";
pub const TAKES: &str = "takes";
pub const MORPH: &str = "morph";
pub const SONG: &str = "song";
/// The song list (sequencer S4): Arrange's third view.
pub const SONG_LIST: &str = "song-list";
/// Play's Q-Link column, at Arrange's right edge too: the app's
/// `QLinkColumn` (its `play_page::QLINKS`), one widget on both pages.
pub const QLINKS: &str = "qlinks";
/// The width of the panel beside the grid, the song or the note editor
/// (Record or Edit over Takes), narrowed for the Q-Link column.
pub const SIDE_W: i32 = 250;
/// The note editor and its Edit options.
pub const NOTES: &str = "notes";
pub const EDIT_OPTIONS: &str = "edit-options";
/// The page's name, for the header tab.
pub const CLIPS_PAGE: &str = "Clips";
/// The bar in a layout spec: 44 pixels under the header.
pub const BAR_SPEC: &str = "transport:44";
/// The Clips view: the bar over the grid and the scene morph, and beside
/// them the record settings over the takes; Play's Q-Link column (128 px,
/// `mpfree_ui::qlink::COLUMN_WIDTH`) down the right edge, from under the
/// header to the hint line, as on Play.
pub const CLIPS_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, col(record:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The same page in its Song view: the timeline in place of the grid.
pub const SONG_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, col(record:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The Clips view, the Song view and the song list with the Record panel
/// on its Scene tab: the scene editor in the record settings' place.
pub const CLIPS_SCENE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, col(scene-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const SONG_SCENE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, col(scene-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const LIST_SCENE_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, col(scene-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The hint line with the Record panel's Scene tab.
pub const SCENE_HINT: &str =
    "Scene: the one playing, or pick one on its first row   ·   wheel: pick a row, - / +: change it   ·   Mutes now: the mutes as they are now   ·   UNDO takes a change back";
/// The same page with the song list in place of the grid and the morph
/// (mockup D), and with the Record panel on its Clip tab.
pub const LIST_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, col(record:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const LIST_CARD_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(song-list:*, col(clip-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The hint line under the song list.
pub const LIST_HINT: &str =
    "Tap a step to pick it   ·   ‹ ›: its scene   ·   – / +: times   ·   + Add a scene: after the pick   ·   Play from here: on the next launch line   ·   List → song: onto the timeline";
/// The Clips view and the Song view with the Record panel on its Clip tab:
/// the clip settings card in the record settings' place.
pub const CLIPS_CARD_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, col(clip-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const SONG_CARD_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, col(clip-card:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
pub const CLIPS_HINT: &str =
    "Tap a clip: it plays from the next Launch line, or stops   ·   drag it: move it (SHIFT as you lift: copy)   ·   a row's number plays the row   ·   + records   ·   a track's name arms it";
/// The hint line under the Clips grid with the Record panel's Clip tab.
pub const CLIPS_CARD_HINT: &str =
    "Clip: the armed track's playing clip   ·   wheel: pick a row, - / +: change it   ·   SHIFT + ‹ ›: Length and Speed halve or double   ·   COPY + a clip, then an empty slot: a copy";

/// The hint line while COPY is held on the Clips grid: what a tap copies
/// or pastes.
pub fn copy_hint(st: &TimeState) -> String {
    match &st.clipboard {
        Some(name) => format!(
            "COPY held: tap an empty slot, on any track: a copy of {name} goes there   ·   tap a clip: copy that one instead   ·   UNDO takes a paste back"
        ),
        None => "COPY held: tap a clip to copy it   ·   then, COPY held again, an empty slot on any track: the copy goes there".to_string(),
    }
}

/// The hint line under the Song view.
pub const SONG_HINT: &str =
    "Tap a block to pick it   ·   drag a block or a section's name: move it (SHIFT as you lift: copy)   ·   Song on: the song plays where no clip is launched   ·   SHIFT + wheel: bars";
/// The hint line under the Song view with a block picked.
pub const SONG_BLOCK_HINT: &str =
    "Drag the block: move it a bar at a time (SHIFT as you lift: copy)   ·   drag its end: longer or shorter   ·   push the wheel: move it a beat at a time   ·   SHIFT + wheel: bars";
/// The hint line under the Song view with a section picked.
pub const SONG_SECTION_HINT: &str =
    "Drag the section's name: move it, the others close up (SHIFT as you lift: copy)   ·   push the wheel: move it a section at a time   ·   SHIFT + wheel: bars";
/// The hint line under the Song view with a tempo point picked.
pub const SONG_TEMPO_HINT: &str =
    "Drag the point: move it a bar at a time   ·   push the wheel: change its tempo   ·   Ramp: glide to the next point   ·   SHIFT + wheel: bars";

/// The hint line under the Song view for what is picked.
pub fn song_hint(st: &TimeState) -> &'static str {
    if st.tempo_pick.is_some() {
        SONG_TEMPO_HINT
    } else if st.selected.is_some() {
        SONG_BLOCK_HINT
    } else if st.section.is_some() {
        SONG_SECTION_HINT
    } else {
        SONG_HINT
    }
}
/// The same page with the note editor (a piano roll or a list) in place
/// of the grid, and the Edit options in place of the record settings.
pub const EDIT_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(notes:*, col(edit-options:*, takes:240):250):*):*, qlinks:128):*, hint:26)";
/// The hint line under the note editor's piano roll.
pub const EDIT_HINT: &str =
    "Tap a note to pick it, drag to move   ·   Tool: select, draw, erase   ·   TC corrects the picked notes (all if none)   ·   wheel: pick an option, - / +: change it";
/// The hint line under the note editor's list.
pub const EDIT_LIST_HINT: &str =
    "Tap a row to pick its note   ·   SHIFT + wheel: scroll   ·   TC corrects the picked notes (all if none)   ·   wheel: pick an option, - / +: change it";
/// The hint line under the note editor's tracker.
pub const EDIT_TRACKER_HINT: &str =
    "Tap a row to pick its note   ·   Draw: tap an empty step   ·   SHIFT + wheel: scroll   ·   wheel: pick an option, - / +: change it";

/// The hint line under the note editor showing `lens`.
pub fn edit_hint(lens: Lens) -> &'static str {
    match lens {
        Lens::Grid => EDIT_HINT,
        Lens::List => EDIT_LIST_HINT,
        Lens::Tracker => EDIT_TRACKER_HINT,
    }
}
/// The record settings, top to bottom (short, to fit the 250 px panel).
pub const RECORD_ROWS: [&str; 10] = [
    "Armed",
    "Mode",
    "Count-in",
    "Loop",
    "Tempo",
    "Pressure",
    "Quantise",
    "Launch",
    "Transpose",
    "Follows",
];
/// The Record tab's Transpose and Follows rows (sequencer S4, live
/// transpose), for the armed track.
pub const TRANSPOSE_ROW: usize = 8;
pub const FOLLOWS_ROW: usize = 9;
/// The scene editor's rows, top to bottom. Program and Bank are the
/// program change the scene sends the armed track's gear (sequencer S5);
/// Sent to lists every track it sends one.
pub const SCENE_ROWS: [&str; 9] = [
    "Scene", "Follow", "After", "Chance", "Or", "Mutes", "Program", "Bank", "Sent to",
];
/// The scene editor's Program and Bank rows.
pub const SCENE_PROGRAM: usize = 6;
pub const SCENE_BANK: usize = 7;
/// `TimeAction::SceneSettings`'s card row for a rename.
pub const SCENE_NAME: u8 = SCENE_ROWS.len() as u8;
/// The scene editor's buttons, under its rows: `Toggle(k)`.
pub const SCENE_RENAME: usize = 0;
pub const SCENE_MUTES_NOW: usize = 1;
/// The other actions the scene editor's Or row steps through: Stay,
/// then the follow actions.
pub const OTHERS: [FollowAction; 9] = [
    FollowAction::Stay,
    FollowAction::Next,
    FollowAction::Previous,
    FollowAction::First,
    FollowAction::Last,
    FollowAction::Any,
    FollowAction::Other,
    FollowAction::Again,
    FollowAction::Stop,
];
/// The clip settings card's rows, top to bottom.
pub const CLIP_ROWS: [&str; 13] = [
    "Length",
    "Speed",
    "Swing",
    "Quantise",
    "Strength",
    "Follow",
    "After",
    "Launch",
    "Mode",
    "Program",
    "Bank",
    "Direction",
    "Mutate",
];
/// The clip card's Direction row (S6).
pub const CLIP_DIRECTION: usize = 11;
/// The clip card's Mutate row (S6): › a new variation of the clip's
/// notes, SHIFT + › a bolder one, ‹ the original back.
pub const CLIP_MUTATE: usize = 12;

/// A play direction (S6) as the clip card names it.
pub fn direction_name(d: Direction) -> &'static str {
    match d {
        Direction::Forward => "Forward",
        Direction::Reverse => "Reverse",
        Direction::PingPong => "Ping pong",
        Direction::Random => "Random",
    }
}
/// The clip card's Mode row: Restart, Legato (takes over the playing
/// clip's place), Once (a one-shot: one pass, then the track stops).
pub const MODES: [&str; 3] = ["Restart", "Legato", "Once"];

/// A clip's settings' place in [`MODES`] (one-shot over legato).
pub fn mode_of(s: &ClipSettings) -> usize {
    if s.once {
        2
    } else if s.legato {
        1
    } else {
        0
    }
}
/// The speeds the card's Speed row steps through, slowest first, as
/// (clip ticks, song ticks): x1/8 to x4.
pub const SPEEDS: [(u8, u8); 12] = [
    (1, 8),
    (1, 4),
    (1, 3),
    (1, 2),
    (2, 3),
    (3, 4),
    (1, 1),
    (4, 3),
    (3, 2),
    (2, 1),
    (3, 1),
    (4, 1),
];
/// The grids the card's Quantise row steps through after off, coarsest
/// first: 1/4, 1/8, 1/8 triplets, 1/16, 1/16 triplets, 1/32.
pub const QUANTISE_GRIDS: [u32; 6] = [PPQ, PPQ / 2, PPQ / 3, PPQ / 4, PPQ / 6, PPQ / 8];
/// The follow actions the card's Follow row steps through after off.
pub const FOLLOWS: [FollowAction; 8] = [
    FollowAction::Next,
    FollowAction::Previous,
    FollowAction::First,
    FollowAction::Last,
    FollowAction::Any,
    FollowAction::Other,
    FollowAction::Again,
    FollowAction::Stop,
];
/// The swing the card's Swing row steps through after the track's: 50 %
/// (straight) to 75 %, on 1/16 notes.
pub const CLIP_SWING: std::ops::RangeInclusive<u8> = 50..=75;
/// A step of the card's Length row: a sixteenth, as on Beats.
pub const CARD_STEP: Tick = PPQ as Tick / 4;
/// The loop lengths the Loop row steps through, in bars from bar 1 (0 is off).
pub const LOOP_BARS: [i64; 6] = [0, 1, 2, 4, 8, 16];
/// Track colours, by track number.
pub const TRACK_COLORS: [u32; 8] = [
    0xE8A33D, 0x3DDC97, 0x6EA8FE, 0xFF8A3D, 0xC77DFF, 0xFF6FA8, 0x2EC4B6, 0x9AA8FF,
];

/// `layout` with the transport bar under the header, for the app's Play
/// page and the arp's layouts: `col(header:60, ...` becomes
/// `col(header:60, transport:44, ...`. None if it does not start that way.
pub fn with_transport(layout: &str) -> Option<String> {
    const HEAD: &str = "col(header:60, ";
    let rest = layout.strip_prefix(HEAD)?;
    Some(format!("{HEAD}{BAR_SPEC}, {rest}"))
}

pub fn track_color(track: usize) -> Color {
    Color::rgb(TRACK_COLORS[track % TRACK_COLORS.len()])
}

/// Audio tracks' colours, by audio track: the track colours in another
/// order (teal and amber first, as the mockups have them).
pub fn audio_color(track: usize) -> Color {
    const ORDER: [usize; 8] = [6, 0, 2, 5, 1, 4, 3, 7];
    track_color(ORDER[track % ORDER.len()])
}

// ---------------------------------------------------------------- state

/// The transport as the screen shows it. Plain values: the audio thread
/// copies one after each period without allocating.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransportInfo {
    pub state: State,
    /// The song position at the start of the next period.
    pub position: Tick,
    pub bpm: f64,
    pub meter: Meter,
    pub swing: Swing,
    /// The loop set here. While [`TransportInfo::follows_position`] it
    /// is held off, not played ([`TransportInfo::playing_loop`]).
    pub loop_range: Option<(Tick, Tick)>,
    pub overdub: bool,
    /// Bars of count-in before recording from stopped.
    pub count_in: u8,
    /// Following another device's position (Sync's Position in the I/O
    /// view): the leader owns the timeline, so the loop is off, greyed,
    /// and comes back when following stops (a loop set meanwhile too).
    pub follows_position: bool,
}

impl Default for TransportInfo {
    fn default() -> Self {
        TransportInfo {
            state: State::Stopped,
            position: 0,
            bpm: 120.0,
            meter: Meter::default(),
            swing: Swing::default(),
            loop_range: None,
            overdub: false,
            count_in: 0,
            follows_position: false,
        }
    }
}

impl TransportInfo {
    /// The loop the transport plays: none while following another
    /// device's position, which holds the loop set here off.
    pub fn playing_loop(&self) -> Option<(Tick, Tick)> {
        self.loop_range.filter(|_| !self.follows_position)
    }

    pub fn of(t: &Transport) -> TransportInfo {
        TransportInfo {
            state: t.state(),
            position: t.position(),
            bpm: t.bpm(),
            meter: t.meter(),
            swing: t.swing(),
            loop_range: t.loop_range(),
            overdub: t.overdub(),
            count_in: t.count_in(),
            follows_position: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipInfo {
    pub name: String,
    /// One pass, in the clip's own ticks.
    pub length: Tick,
    pub speed: Speed,
    pub notes: usize,
    /// A Loop track's cell: the looper's loop it launches (`Clip::looped`).
    pub looped: Option<u8>,
    /// Its speed, swing, quantise and follow action, for the clip
    /// settings card.
    pub settings: ClipSettings,
    /// The variation of its notes it plays, kept beside its original
    /// (`Clip::variation`, S6's Mutate); None when it keeps none.
    pub variation: Option<u32>,
}

/// What the looper says of a loop, for its cells on the Clips grid: the
/// app fills it from the Looper's view ([`TimeState::loops`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopFacts {
    /// It records a layer now.
    pub overdubbing: bool,
    /// Another loop's layers copy into it (a loop cell copied onto
    /// another Loop track, loop-tracks.md part 6).
    pub copying: bool,
    /// Its small wave for the cells ([`crate::widgets::GridCell::wave`]).
    pub wave: [u8; crate::widgets::CELL_WAVE],
}

/// The loops for the grid: each loop's facts, how many hold a loop, and
/// the share of the loop memory in use, in percent. The app fills it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LoopsInfo {
    pub loops: [LoopFacts; mpfree_seq::LOOPS],
    pub in_use: usize,
    pub memory_pct: u8,
    /// A new loop's first pass waits for PLAY or the bar line, or
    /// records: the armed Loop cell says so.
    pub waiting: bool,
    pub recording: bool,
}

impl ClipInfo {
    /// One pass, in song ticks.
    pub fn song_len(&self) -> f64 {
        self.speed.to_song(self.length as f64)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TrackInfo {
    pub name: String,
    /// What it plays, under its name ("Kit", "FM · Glass Keys"); empty
    /// shows only the name.
    pub sound: String,
    pub mute: bool,
    /// Soloed (`Track::solo`, sequencer S4).
    pub solo: bool,
    /// Silent: muted, or not soloed while another track is.
    pub silent: bool,
    /// It records pad pressure (`Track::record_touch`).
    pub pressure: bool,
    /// Its record quantise grid (`Track::record_quantise`), or none.
    pub record_quantise: Option<u32>,
    /// A Loop track, and what its new loops record (`Track::kind`).
    pub loops: Option<mpfree_seq::LoopInput>,
    /// Its live transpose and "Follows transpose" (`Track::transpose`,
    /// `Track::follows`, sequencer S4).
    pub transpose: Transpose,
    pub follows: bool,
    /// It plays notes, not a kit's pads: its own instrument, or the synth
    /// on the pads. Only such a track transposes. The screen fills it in.
    pub melodic: bool,
    pub slots: Vec<Option<ClipInfo>>,
}

/// One take as the list shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TakeRow {
    pub number: u32,
    /// Made by CAPTURE.
    pub captured: bool,
    /// An edit of a clip's notes (the step sequencer).
    pub edited: bool,
    /// It made a new clip.
    pub made: bool,
    /// The track and slot it recorded into (of its first note).
    pub at: Option<(usize, usize)>,
    pub added: usize,
    pub erased: usize,
    /// It took a clip out of its slot (ERASE + a clip).
    pub deleted: bool,
    /// It added this track (+ Track).
    pub new_track: Option<usize>,
    /// A drop on the Clips grid: a clip or a row moved, swapped or copied.
    pub dropped: Option<ClipDrop>,
    /// A Song view edit: a block moved, copied, made longer or deleted, a
    /// section moved.
    pub song: Option<SongChange>,
    /// A clip's settings changed (the clip settings card).
    pub settings: bool,
    /// COPY + an empty slot pasted a clip there.
    pub pasted: bool,
    /// A whole clip went in place of the clip there, and why: Reload
    /// clip (sequencer S4), Mutate or Restore (S6).
    pub replaced: Option<Replaced>,
    /// The variation a Mutate made ("variant 3").
    pub variant: Option<u32>,
    /// The scene editor changed this row's scene (its name, mutes or
    /// follow action).
    pub scene: Option<usize>,
    /// Song → scenes filled these rows, as (first, last).
    pub rows: Option<(usize, usize)>,
}

impl TakeRow {
    pub fn of(t: &Take) -> TakeRow {
        let made = t.made.first().map(|(track, slot, _)| (*track, *slot));
        let removed = t.removed.first().map(|(track, slot, _)| (*track, *slot));
        let at = match t.dropped {
            // Named by where the clip came from; a row by its number.
            Some(ClipDrop::Moved(a, _) | ClipDrop::Swapped(a, _) | ClipDrop::Copied(a, _)) => {
                Some(a)
            }
            Some(_) => None,
            None => made
                .or(removed)
                .or_else(|| {
                    t.added
                        .first()
                        .or(t.erased.first())
                        .map(|&(track, slot, _)| (track, slot))
                })
                .or_else(|| t.settings.map(|(track, slot, ..)| (track, slot))),
        };
        let added = t.added.len() + t.made.iter().map(|(_, _, c)| c.notes.len()).sum::<usize>();
        TakeRow {
            number: t.number,
            captured: t.captured,
            edited: t.edited,
            made: made.is_some(),
            at,
            added,
            erased: t.erased.len(),
            deleted: removed.is_some(),
            new_track: t.track_added.as_ref().map(|(i, _)| *i),
            dropped: t.dropped,
            song: t.song.as_ref().map(|e| e.change),
            settings: t.settings.is_some(),
            pasted: t.pasted,
            replaced: t.replaced,
            variant: t.made.first().and_then(|(_, _, c)| c.variation()),
            scene: t.scene.as_ref().map(|e| e.row),
            rows: t
                .rows
                .as_ref()
                .and_then(|e| Some((*e.rows.first()?, *e.rows.last()?))),
        }
    }

    /// What it is: "take", "capture", "edit", "delete", "paste",
    /// "settings", "reload", "mutate", "restore", for a drop "move",
    /// "swap" or "copy", and for a Song view edit "move", "copy",
    /// "length", "delete", "unique", "scenes" or "tempo", "list" for the
    /// song list, "scene" for the scene editor and "song → scenes".
    pub fn word(&self) -> &'static str {
        if self.rows.is_some() {
            return "song → scenes";
        }
        if let Some(c) = self.song {
            return match c {
                SongChange::Moved { .. } | SongChange::SectionMoved { .. } => "move",
                SongChange::Copied { .. } | SongChange::SectionCopied { .. } => "copy",
                SongChange::Resized { .. } => "length",
                SongChange::Deleted { .. } => "delete",
                SongChange::Unique { .. } => "unique",
                SongChange::Scenes { .. } => "scenes",
                SongChange::List | SongChange::ListToSong { .. } => "list",
                SongChange::Tempo { .. } => "tempo",
            };
        }
        if let Some(d) = self.dropped {
            return match d {
                ClipDrop::Moved(..) | ClipDrop::RowMoved { .. } => "move",
                ClipDrop::Swapped(..) => "swap",
                ClipDrop::Copied(..) | ClipDrop::RowCopied { .. } => "copy",
            };
        }
        if self.scene.is_some() {
            "scene"
        } else if let Some(r) = self.replaced {
            match r {
                Replaced::Reload => "reload",
                Replaced::Mutate => "mutate",
                Replaced::Restore => "restore",
            }
        } else if self.deleted {
            "delete"
        } else if self.pasted {
            "paste"
        } else if self.settings {
            "settings"
        } else if self.captured {
            "capture"
        } else if self.edited {
            "edit"
        } else if self.number & mpfree_play::ERASE_TAKES != 0 {
            // ERASE + a pad while only playing (live erase's own takes).
            "erase"
        } else {
            "take"
        }
    }

    /// Its number as shown: live erase's takes count from 1 too, without
    /// the top bit their numbers carry (`mpfree_play::ERASE_TAKES`).
    pub fn shown(&self) -> u32 {
        self.number & !mpfree_play::ERASE_TAKES
    }

    /// What Undo and Redo name it: "take 3", "capture 2", "edit 5",
    /// "erase 1" (live erase while playing), "new track 9", or "move 7 ·
    /// row 3" for a row.
    pub fn short(&self) -> String {
        let n = self.shown();
        match (self.new_track, self.dropped) {
            (Some(t), _) => format!("new track {}", t + 1),
            (None, Some(ClipDrop::RowMoved { from, .. } | ClipDrop::RowCopied { from, .. })) => {
                format!("{} {n} · row {}", self.word(), from + 1)
            }
            _ => format!("{} {n}", self.word()),
        }
    }
}

/// Everything the time engine's screens show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeState {
    pub transport: TransportInfo,
    pub tracks: Vec<TrackInfo>,
    /// The looper's loops, for the Loop tracks' cells. The app sets them.
    pub loops: LoopsInfo,
    pub launches: Vec<Launch>,
    /// Launches and stops waiting for their launch line (the next bar line
    /// by default). The app sets them.
    pub cues: Vec<Cue>,
    pub armed: Option<usize>,
    /// The track selected apart from the arm ([`TimeAction::Select`]), or
    /// None while the selection is the armed track: see
    /// [`TimeState::selected`].
    pub track_pick: Option<usize>,
    /// The selected track's slot picked for the Sequencer to edit apart
    /// from what plays ([`TimeAction::EditClip`]), or None while it edits
    /// the clip that plays.
    pub clip_pick: Option<usize>,
    /// The other armed tracks, a bit each (`TimeAction::ArmAlso`): a take
    /// records their notes with the armed track's. Never the armed one.
    pub also: u128,
    /// The clip COPY + a clip copied last, by its name, for COPY + an
    /// empty slot to paste. The app sets it.
    pub clipboard: Option<String>,
    /// The takes that can be undone, oldest first, and the ones that can be
    /// redone, the next one last (as [`Takes`] keeps them).
    pub done: Vec<TakeRow>,
    pub undone: Vec<TakeRow>,
    /// Status chips on the right of the bar (the arp's).
    pub chips: Vec<Chip>,
    /// What Capture would keep now, such as "2 bars"; None when nothing
    /// was played since the last capture. The app sets it.
    pub capture: Option<String>,
    /// The scene (row) playing, and the one waiting for the next bar line
    /// (`mpfree_perform::Perform`). The app sets them.
    pub scene_playing: Option<usize>,
    pub scene_cued: Option<usize>,
    /// The Loop track cell armed: the next loop recorded lands there
    /// (`TimeAction::LoopCell`).
    pub loop_cell: Option<(usize, usize)>,
    /// Each row's scene name, as far as the project names them (empty: a
    /// plain row, shown as its number).
    pub scenes: Vec<String>,
    /// The scene morph. The app sets it.
    pub morph: MorphInfo,
    /// The scene editor's row (the Record panel's Scene tab, S4), and
    /// its name, mutes and follow action. The app sets them.
    pub scene_focus: usize,
    pub scene: SceneSettings,
    /// The page shows the song (the timeline) in place of the grid. The
    /// app sets it.
    pub song_view: bool,
    /// The page shows the song list in place of the grid (it wins over
    /// `song_view`). The app sets it.
    pub list_view: bool,
    /// The song list's steps as it shows them, the step picked, and
    /// whether it goes round. The app sets them.
    pub list: Vec<ListRow>,
    pub list_pick: Option<usize>,
    pub list_looped: bool,
    /// While the list plays: the step after the one playing (None: it
    /// stops) and the line it comes in on, for Perform's NEXT. The app
    /// sets it.
    pub list_next: Option<(Option<usize>, Tick)>,
    /// Song mode: the song plays on tracks with no clip launched. The app
    /// sets it.
    pub song_on: bool,
    /// The project is in Perform, not Make (`Project::perform`, sequencer
    /// S4).
    pub perform: bool,
    /// The project's launch quantise (`Project::launch`, S4).
    pub launch: LaunchGrid,
    /// The line each cue lands on, by track, where the time engine set
    /// it apart from the clip's own and the project's (a scene's, a
    /// follow's, one kept when Launch changed). The app sets it.
    pub cue_lines: Vec<(usize, LaunchGrid)>,
    /// Mute and solo changes waiting for their line (the mute queue). The
    /// app sets it.
    pub mute_wait: Option<MuteWait>,
    /// The clips with a saved state (Save state on the clip card, S4), as
    /// (track, slot): Reload clip brings it back.
    pub saved: Vec<(usize, usize)>,
    /// Reloads waiting for their launch line, as (track, slot, the line's
    /// song tick).
    pub reloads: Vec<(usize, usize, Tick)>,
    /// The scene playing's follow action, for Perform's NEXT (S4). The
    /// app sets it.
    pub follow_next: Option<FollowNext>,
    /// The arrangement's placements and markers. The placements are
    /// shared: the screen keeps the same list from one transport update to
    /// the next, so the Song view knows its blocks are the same without
    /// comparing them (Arrange refactor 2.4).
    pub placements: Arc<[PlacementInfo]>,
    pub markers: Vec<(Tick, String)>,
    /// The placement selected on the Song view. The app sets it.
    pub selected: Option<usize>,
    /// The section picked on the Song view (by its marker), for the
    /// wheel. The app sets it.
    pub section: Option<usize>,
    /// Live to song armed: what the grid plays goes into the song. The app
    /// sets it.
    pub song_rec: bool,
    /// Perform (sequencer S4): the take recording, or the next one, turns
    /// Rec song on for its length; the REC chip says "+ Rec song".
    pub song_with_take: bool,
    /// The note editor, when it is open. The app sets it.
    pub edit: Option<Box<EditInfo>>,
    /// The audio tracks' lanes on the Song view (layout spec 4c), under
    /// the MIDI lanes: one for each audio track with a region or armed,
    /// in order. Build their blocks with [`crate::song::audio_blocks`].
    /// The app sets them.
    pub audio: Vec<AudioLane>,
    /// The audio takes' waveforms read so far (`AudioHost::wave`), a take
    /// each. The app sets them.
    pub waves: Vec<TakeWave>,
    /// The audio region picked on the Song view. The app sets it; it and
    /// `selected` are never both set (see [`TimeState::pick`]).
    pub region: Option<RegionId>,
    /// The audio tracks' memory: seconds free and seconds in all, for the
    /// Song view's legend. The app sets it.
    pub audio_free: Option<(f64, f64)>,
    /// Audio tracks counters to show while they are not zero, such as
    /// ("dropped", 2) or ("read errors", 1): the engine's `Stats` and the
    /// streaming counts. Fallbacks and losses are always shown. The app
    /// sets them.
    pub audio_counts: Vec<(String, u32)>,
    /// The audio track whose panel is open over the lanes (layout spec
    /// 4c), if any. The app sets it.
    pub audio_panel: Option<usize>,
    /// The audio track whose take lanes show under its lane (layout spec
    /// 4c), if any. The app sets it.
    pub audio_takes: Option<usize>,
    /// The song's tempo points as (tick, tempo, glides to the next), in
    /// order, and the project's tempo before the first (tempo ramps).
    pub tempo_points: Vec<(Tick, f64, bool)>,
    pub tempo_base: f64,
    /// The tempo point picked on the Song view. The app sets it.
    pub tempo_pick: Option<usize>,
    /// While song mode plays the song's tempo points: the tempo of the
    /// point in effect at the playhead (None before the first), which the
    /// Tempo row steps from. None otherwise. The app sets it.
    pub tempo_here: Option<f64>,
}

/// An audio region: region `index` of audio track `track`
/// (`AudioTracks::regions(track)[index]`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionId {
    pub track: usize,
    pub index: usize,
}

/// What is picked on the Song view: the selection's kind decides the
/// bottom row's buttons and the Q-Links.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A placement, by its index in the arrangement.
    Placement(usize),
    Region(RegionId),
}

/// The note editor as the screens show it.
#[derive(Clone, Debug, PartialEq)]
pub struct EditInfo {
    pub editor: Editor,
    /// The clip, shared: the time engine sends the same one until the clip
    /// changes, so the screen tells an unchanged clip at no cost
    /// ([`Arc::ptr_eq`]) and rebuilds the roll only when it did.
    pub clip: Arc<Clip>,
    /// The clip's revision: it moves whenever the clip changes (an edit,
    /// a recorded note, UNDO, an edit from Steps), and a tap on a note
    /// carries the one it was made on ([`EditAction::Note`]).
    pub rev: u64,
    pub track_name: String,
    /// A bar, in the clip's ticks.
    pub bar: Tick,
}

/// A placement as the screens show it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementInfo {
    pub track: usize,
    pub start: Tick,
    pub length: Tick,
    /// Its clip's name ("(empty)" for a slot emptied since).
    pub name: String,
    pub unique: bool,
}

/// The scene morph as the screen shows it (`mpfree_perform::SceneMorph`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MorphInfo {
    /// The rows on A and B.
    pub a: usize,
    pub b: usize,
    /// The crossfader, 0 (A) to 1 (B).
    pub pos: f32,
    /// A morph is loaded: the rows' values are on A and B.
    pub on: bool,
    /// Parameters it moves.
    pub params: usize,
}

impl Default for MorphInfo {
    fn default() -> MorphInfo {
        MorphInfo {
            a: 0,
            b: 1,
            pos: 0.0,
            on: false,
            params: 0,
        }
    }
}

impl TimeState {
    /// The selected track: the one the Sequencer's Steps, Notes and STEP
    /// SEQ edit and the pads follow. The armed track unless a track was
    /// selected apart from it while the arm is pinned
    /// ([`TimeState::track_pick`]).
    pub fn selected(&self) -> Option<usize> {
        self.track_pick.or(self.armed)
    }

    pub fn from_engine(
        transport: TransportInfo,
        project: &Project,
        launches: &[Launch],
        armed: Option<usize>,
        takes: &Takes,
    ) -> TimeState {
        let tracks = project
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| TrackInfo {
                name: t.name.clone(),
                sound: String::new(),
                mute: t.mute,
                solo: t.solo,
                silent: project.silent(i),
                pressure: t.record_touch,
                record_quantise: t.record_quantise,
                loops: t.loops(),
                transpose: t.transpose,
                follows: t.follows,
                melodic: false,
                // The rows the grid shows (Arrange 1.2: a project keeps
                // more, for scene banks to come).
                slots: t
                    .slots
                    .shown()
                    .map(|s| {
                        s.as_ref().map(|c| ClipInfo {
                            name: c.name.clone(),
                            length: c.length,
                            speed: c.speed,
                            notes: c.notes.len(),
                            looped: c.looped,
                            settings: c.settings(),
                            variation: c.variation(),
                        })
                    })
                    .collect(),
            })
            .collect();
        TimeState {
            transport,
            tracks,
            loops: LoopsInfo::default(),
            launches: launches.to_vec(),
            cues: Vec::new(),
            armed,
            track_pick: None,
            clip_pick: None,
            also: 0,
            clipboard: None,
            done: takes.done().iter().map(TakeRow::of).collect(),
            undone: takes.undone().iter().map(TakeRow::of).collect(),
            chips: Vec::new(),
            capture: None,
            scene_playing: None,
            scene_cued: None,
            loop_cell: None,
            scenes: project.scenes.iter().map(|s| s.name.clone()).collect(),
            morph: MorphInfo::default(),
            scene_focus: 0,
            scene: SceneSettings::default(),
            song_view: false,
            list_view: false,
            list: Vec::new(),
            list_pick: None,
            list_looped: false,
            list_next: None,
            song_on: false,
            perform: project.perform,
            launch: project.launch,
            cue_lines: Vec::new(),
            mute_wait: None,
            saved: Vec::new(),
            reloads: Vec::new(),
            placements: project
                .arrangement
                .placements
                .iter()
                .map(|p| PlacementInfo {
                    track: p.track,
                    start: p.start,
                    length: p.length,
                    name: project
                        .block_clip(p)
                        .map_or_else(|| "(empty)".to_string(), |c| c.name.clone()),
                    // Its clip is the song's own (made unique, or a
                    // deleted clip the song still plays).
                    unique: project.arrangement.clips.get(p.clip).is_some(),
                })
                .collect::<Vec<_>>()
                .into(),
            markers: project
                .arrangement
                .markers
                .iter()
                .map(|m| (m.at, m.name.clone()))
                .collect(),
            selected: None,
            section: None,
            song_rec: false,
            song_with_take: false,
            follow_next: None,
            edit: None,
            audio: Vec::new(),
            waves: Vec::new(),
            region: None,
            audio_free: None,
            audio_counts: Vec::new(),
            audio_panel: None,
            audio_takes: None,
            tempo_points: project
                .arrangement
                .tempo
                .iter()
                .map(|p| (p.at, p.bpm, p.ramp))
                .collect(),
            tempo_base: transport.bpm,
            tempo_pick: None,
            tempo_here: None,
        }
    }

    /// What is picked on the Song view, and its kind. A region wins if
    /// both were ever set.
    pub fn pick(&self) -> Option<Pick> {
        self.region
            .map(Pick::Region)
            .or(self.selected.map(Pick::Placement))
    }

    pub fn clip(&self, track: usize, slot: usize) -> Option<&ClipInfo> {
        self.tracks.get(track)?.slots.get(slot)?.as_ref()
    }

    /// The clip launched on `track`, if any.
    pub fn launch(&self, track: usize) -> Option<&Launch> {
        self.launches.iter().find(|l| l.track == track)
    }

    /// The cue waiting on `track`, if any.
    pub fn cue(&self, track: usize) -> Option<&Cue> {
        self.cues.iter().find(|c| c.track == track)
    }

    /// The focused clip, as (track, slot): the armed track's playing clip,
    /// else its first (the first track's while none is armed). Beats, the
    /// note editor and the clip settings card all show it (the app's
    /// `TimeControl` picks the same one).
    pub fn focus(&self) -> Option<(usize, usize)> {
        let track = self.armed.unwrap_or(0);
        let tr = self.tracks.get(track)?;
        let slot = self
            .launch(track)
            .map(|l| l.slot)
            .or_else(|| tr.slots.iter().position(Option::is_some))?;
        self.clip(track, slot).is_some().then_some((track, slot))
    }

    fn track_name(&self, track: usize) -> String {
        self.tracks
            .get(track)
            .map_or_else(|| format!("Track {}", track + 1), |t| t.name.clone())
    }
}

// ---------------------------------------------------------------- actions

/// What a touch on the time engine's screens asks for. The app applies it.
/// Not `Copy`: the scene editor's settings carry a name.
#[derive(Clone, Debug, PartialEq)]
pub enum TimeAction {
    /// For the audio thread's command queue.
    Transport(Command),
    Undo,
    Redo,
    /// Arm a track for recording, or none. A track armed is selected too
    /// (the Sequencer edits it and the pads follow it), so without a
    /// pinned arm the two never part.
    Arm(Option<usize>),
    /// A track picked (its name on Arrange, the Play page's track switch,
    /// MAIN + a pad, a track on the Pads tab): selected, and armed too
    /// unless REC's arm is pinned, which the app knows; it sends
    /// [`TimeAction::Select`] then. On its own it arms, as
    /// [`TimeAction::Arm`] does.
    Pick(usize),
    /// Select a track without moving the arm (a track picked while the
    /// arm is pinned): Steps, Notes and STEP SEQ edit it and the pads
    /// follow it, while REC still records the armed track. The armed
    /// track itself selects it again.
    Select(usize),
    /// The clip the Sequencer edits on the selected track (refactor C2):
    /// `Some(slot)` edits that slot's clip without launching it, so what
    /// plays keeps playing; `None` edits the clip that plays again (its
    /// first while none does). A track selected again goes back to what
    /// plays.
    EditClip(Option<usize>),
    /// Arm these tracks too, a bit each (track 0 the lowest), for one take
    /// over several tracks (the MPC's SHIFT + a track's arm): each records
    /// what its MIDI input sends it. The armed track is left out of it.
    ArmAlso(u128),
    /// "Pressure" (pad pressure) for a track: whether recording keeps the pads'
    /// pressure on its notes (`Track::record_touch`).
    RecordPressure {
        track: usize,
        on: bool,
    },
    /// "Quantise" on the record settings (sequencer S3): the grid the
    /// track's recorded notes go in on (`Track::record_quantise`), or none.
    RecordQuantise {
        track: usize,
        grid: Option<u32>,
    },
    /// "Launch" on the record settings (sequencer S4): the project's launch
    /// quantise, when launches, stops and scenes land.
    LaunchGrid(LaunchGrid),
    /// Live transpose (sequencer S4): the pads' Transpose, or the Record
    /// tab's row. `track` (the armed one) and every melodic track that
    /// follows transpose move to `to`, from their next note on; a note
    /// already sounding keeps its pitch.
    Transpose {
        track: usize,
        to: Transpose,
    },
    /// "Follows transpose" for `track`: the pads' Transpose moves it with
    /// the armed track.
    FollowsTranspose {
        track: usize,
        on: bool,
    },
    /// Play this clip from the next bar line (see [`clips_after`]).
    Launch {
        track: usize,
        slot: usize,
    },
    /// Stop the track's clip on the next bar line.
    StopClip(usize),
    /// Take back the launch or stop waiting on the track.
    Uncue(usize),
    /// Make a clip in this empty slot, launch it and arm its track.
    NewClip {
        track: usize,
        slot: usize,
    },
    /// ERASE + a clip: take the clip out of its slot, as a take that UNDO
    /// puts back (Jay after live test 3).
    DeleteClip {
        track: usize,
        slot: usize,
    },
    /// A clip dragged on the Clips grid and dropped on slot `to`, both
    /// (track, slot) (Jay, 14 Nov: drag and drop): it moves into an
    /// empty slot, or changes places with the clip there; with `copy`
    /// (SHIFT held at the lift) a copy goes into an empty slot. One take,
    /// which UNDO takes back.
    DropClip {
        from: (usize, usize),
        to: (usize, usize),
        copy: bool,
    },
    /// A row's number dragged on the Clips grid: row `from` goes in
    /// before row `to` (a gap, 0 to the slots), the rows between moving
    /// one, or with `copy` a copy goes in and the rows under it move down
    /// one. One take, which UNDO takes back.
    DropRow {
        from: usize,
        to: usize,
        copy: bool,
    },
    /// "+ Track": a new empty track at the end (up to 128), armed, as a
    /// take that UNDO takes away again (Jay, 14 Nov: up to 128 tracks).
    AddTrack,
    /// SHIFT + "+ Track": a new empty Loop track at the end ("Loops C"),
    /// as a take that UNDO takes away again (loop-tracks.md part 4).
    AddLoopTrack,
    /// Arm an empty cell of a Loop track, or none: the next loop
    /// recorded lands there (loop-tracks.md part 4). The app arms the loop
    /// for REC with it.
    LoopCell(Option<(usize, usize)>),
    /// A Loop track's input to the next one (Pads, Line in, Resample):
    /// what a loop recorded into its cells records.
    LoopInput(usize),
    /// A loop cell copied onto an empty cell of another Loop track
    /// (loop-tracks.md part 6): the cell there takes loop `into`, an empty
    /// loop the looper copies the audio into. Not a take: the looper's
    /// UNDO takes the copy back, and its cell goes with it. The app sends
    /// it for SHIFT + a drop onto another Loop track.
    LoopCopy {
        from: (usize, usize),
        to: (usize, usize),
        into: u8,
    },
    /// Loop `n`'s copy stopped before any of it went in: its cells empty.
    LoopCopyOff(u8),
    /// Chip `i` on the bar was tapped.
    Chip(usize),
    /// Keep what was just played (CAPTURE): into the armed track's playing
    /// clip, or as a new clip (see `mpfree_capture::keep`).
    Capture,
    /// UNDO while recording: punch out and throw the take so far away (the
    /// music plays on); during the count-in, stop.
    ThrowTake,
    /// Play a whole row of the grid (a scene) from the next bar line, with
    /// its mutes (`mpfree_perform::Perform::launch_scene`).
    LaunchScene(usize),
    /// Stop every track on the next bar line.
    StopAll,
    /// Every track back to the arrangement on the launch line: song mode
    /// on, and every clip launched over the song stopped there (sequencer
    /// S4, "Back to song", Ableton's Back to Arrangement).
    BackToSong,
    /// Keep the clip as it is now (sequencer S4, Elektron's temp save):
    /// Reload clip brings this state back.
    SaveState {
        track: usize,
        slot: usize,
    },
    /// Put the clip's saved state back, on its launch line while it plays
    /// (at once otherwise), as an edit UNDO takes back. Asked again while
    /// it waits, the reload is taken back.
    Reload {
        track: usize,
        slot: usize,
    },
    /// Mutate (sequencer S6, `mpfree_seq::mutate`): a new variation of
    /// the clip's notes, made from the clip as it was before its first
    /// Mutate, which it keeps for Restore; at once, as an edit UNDO takes
    /// back. `more` (SHIFT) varies about half the notes, else a quarter;
    /// `melodic` (its track plays notes, not pads) lets a note move to the
    /// next pitch the clip plays.
    Mutate {
        track: usize,
        slot: usize,
        more: bool,
        melodic: bool,
    },
    /// Restore (S6): the clip as it was before its first Mutate, at once,
    /// as an edit UNDO takes back.
    Restore {
        track: usize,
        slot: usize,
    },
    /// Put these rows on the morph's A and B (loaded again if it is on).
    MorphRows {
        a: usize,
        b: usize,
    },
    /// Move the crossfader, 0 (A) to 1 (B); it loads the rows first if the
    /// morph is off.
    Morph(f32),
    /// Let go of the morph, every parameter back to where it was before.
    SnapBack,
    /// Let go of the morph, the parameters staying where it has them.
    KeepMorph,
    /// Store every parameter's value now in this row, for the morph.
    SaveScene(usize),
    /// Show the Song view (the timeline) or the grid.
    SongView(bool),
    /// The scene editor's row (its Scene row's ‹ ›).
    SceneFocus(usize),
    /// The scene editor: row `row` given `settings` (its name, mutes and
    /// follow action), card row `card_row` turned (`SCENE_NAME` for a
    /// rename). One take a row: turning the same row again joins it.
    SceneSettings {
        row: usize,
        card_row: u8,
        settings: SceneSettings,
    },
    /// Show the song list (Arrange's third view, sequencer S4), or (false)
    /// the grid.
    ListView(bool),
    /// Pick a step of the song list, or none.
    ListPick(Option<usize>),
    /// Play the song list from this step: its scene as a scene launch
    /// does, then each step after it in turn
    /// (`mpfree_perform::Perform::play_list`).
    ListPlay(usize),
    /// Step `step`'s times one more or fewer (1 to 99).
    ListTimes {
        step: usize,
        delta: i32,
    },
    /// Step `step`'s scene the next or the previous row with a clip.
    ListScene {
        step: usize,
        delta: i32,
    },
    /// A step after the one picked (at the end with none): the next row
    /// with a clip after its scene, two times.
    ListAdd,
    /// Take this step out of the list.
    ListRemove(usize),
    /// The list goes round after its last step, or stops.
    ListLoop(bool),
    /// The list onto the end of the song, each step its scene's clips its
    /// times (`mpfree_seq::Arrangement::list_to_song`).
    ListToSong,
    /// Song mode on or off: the song plays on tracks with no clip launched.
    SongMode(bool),
    /// Make (false) or Perform (true): the top bar's chip, or SHIFT + MAIN
    /// (sequencer S4). Saved with the project; nothing playing changes.
    Perform(bool),
    /// The rows with clips, in order, onto the end of the song
    /// (`mpfree_seq::arrange::scenes_to_song`).
    ScenesToSong,
    /// The song's sections into a run of empty rows, chained by follow
    /// actions (`mpfree_seq::arrange::song_to_scenes`).
    SongToScenes,
    /// Select a placement on the Song view, or none.
    SelectPlacement(Option<usize>),
    /// A block dragged on the Song view (Jay, 14 Nov: drag and drop), or
    /// moved a beat by the wheel: placement `index` to `track` from
    /// `start`, or with `copy` (SHIFT held as the finger lifts) a copy of
    /// it there.
    MovePlacement {
        index: usize,
        track: usize,
        start: Tick,
        copy: bool,
    },
    /// The picked block's end dragged: placement `index` made `length`
    /// song ticks long.
    StretchPlacement {
        index: usize,
        length: Tick,
    },
    /// A section's name dragged along the ruler, or moved by the wheel:
    /// section `marker` to before section `gap` (after the last for the
    /// markers' count), or with `copy` a copy of it there
    /// (`Arrangement::move_section`).
    MoveSection {
        marker: usize,
        gap: usize,
        copy: bool,
    },
    /// A section's name tapped: pick it (the wheel's push then grabs it).
    PickSection(Option<usize>),
    /// Give a placement its own copy of its clip.
    MakeUnique(usize),
    DeletePlacement(usize),
    /// Live to song on or off: while on and the transport plays, what the
    /// grid plays is recorded into the song
    /// (`mpfree_perform::SongRecorder`).
    RecSong(bool),
    /// Perform's Rec song with REC (sequencer S4): a take turns Rec song
    /// on for its length, or (false) not this take.
    RecSongWithTake(bool),
    /// The clip settings card: clip `slot` of `track` given `settings`
    /// (its speed, swing, quantise and follow action), card row `row`
    /// turned. One take a row: turning the same row again joins it.
    ClipSettings {
        track: usize,
        slot: usize,
        row: u8,
        settings: ClipSettings,
    },
    /// The card's Length row: the clip made `steps` sixteenths long, as
    /// Beats' length does (the notes past the end come back on UNDO); with
    /// `copy` (SHIFT + its ›, doubling) the new steps repeat the notes
    /// before them.
    ClipLength {
        track: usize,
        slot: usize,
        steps: usize,
        copy: bool,
    },
    /// COPY + a clip: keep a copy of it for COPY + an empty slot.
    CopyClip {
        track: usize,
        slot: usize,
    },
    /// COPY + an empty slot: the clip copied last goes there, as a take
    /// UNDO takes away.
    PasteClip {
        track: usize,
        slot: usize,
    },
    /// A touch on the note editor (`crate::editor`).
    Edit(EditAction),
    /// A touch on the Song view's audio lanes, for the audio tracks
    /// (`mpfree_audiotracks`).
    Audio(AudioAction),
    /// The song's tempo points (tempo ramps): the Song view's tempo lane,
    /// the wheel on a picked point, and the tempo turned in song mode.
    SongTempo(TempoEdit),
}

/// An edit of the song's tempo points (`mpfree_seq::arrange::TempoPoint`),
/// each one UNDO takes back.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TempoEdit {
    /// A tap on the tempo lane: a point at song tick `at` (a bar line)
    /// with the tempo playing there, picked.
    Add(Tick),
    /// Point `k` picked (the wheel then sets its tempo, buttons 5 and 6
    /// read Ramp and Delete), or none.
    Pick(Option<usize>),
    /// Point `index` dragged to song tick `at`.
    Move {
        index: usize,
        at: Tick,
    },
    /// Point `index` given the tempo `bpm` (the wheel, with it picked).
    Bpm {
        index: usize,
        bpm: f64,
    },
    /// Point `index` glides to the next point's tempo, or steps.
    Ramp {
        index: usize,
        on: bool,
    },
    Delete(usize),
    /// The tempo turned in song mode while the song's points play (Jay's
    /// card, 18 Nov): the point in effect at the playhead takes `bpm`, or
    /// before the first point a new one at the playhead's bar.
    AtPlayhead(f64),
}

/// What a touch on the Song view's audio lanes asks for (layout spec 4c).
/// The app turns them into `mpfree_audiotracks::Command`s and its panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioAction {
    /// The header's arm dot: arm or disarm the audio track.
    Arm {
        track: usize,
        on: bool,
    },
    /// The header's name or input chip: open the track's panel (Arm,
    /// Input, Monitor, level, pan, mute, solo, Takes) over the lanes, or
    /// close it if it is that track's.
    Panel(usize),
    /// The panel's Close.
    ClosePanel,
    /// The panel: what the track records from, when it hears its input,
    /// and its mute and solo.
    Input {
        track: usize,
        input: Input,
    },
    Monitor {
        track: usize,
        monitor: Monitor,
    },
    /// The panel's Mix and its From steps: record from inside the mix (the
    /// instrument redesign's step 9), stepping through what the mix
    /// offers, `step` either way (0, Mix itself: the one picked last, or
    /// the first). The app keeps the list.
    Mix {
        track: usize,
        step: i32,
    },
    /// The panel's Effects while the track records from the mix: before
    /// or after the source's effects.
    MixEffects {
        track: usize,
        after: bool,
    },
    Mute {
        track: usize,
        on: bool,
    },
    Solo {
        track: usize,
        on: bool,
    },
    /// "+ Audio": show the next audio track's lane.
    Add,
    /// Pick a region, or none. Picking one drops the placement picked.
    Pick(Option<RegionId>),
    /// The bottom row with a region picked: Split (at the playhead),
    /// Erase, Fades, Gain, Takes (the track's take lanes) and Undo (SHIFT
    /// + Undo redoes).
    Split(RegionId),
    Erase(RegionId),
    Fades(RegionId),
    Gain(RegionId),
    /// Show the track's take lanes under its lane, or hide them if they
    /// show.
    Takes(usize),
    Undo,
    /// SHIFT + Undo on the audio row (the app sets it): bring back what
    /// Undo took.
    Redo,
    /// A swipe across a take lane: play take `take` from `from` to `to`
    /// on its track, over what was there (`Command::Comp`), snapped to the
    /// grid unless `fine` (SHIFT held: the app sets it).
    Comp {
        take: u16,
        from: Tick,
        to: Tick,
        fine: bool,
    },
    /// A tap on a take lane: play the take everywhere it has audio.
    UseTake(u16),
    /// A region dragged sideways: move it to start at `to`, snapped to
    /// the grid unless `fine` (SHIFT held: the app sets it).
    Move {
        region: RegionId,
        to: Tick,
        fine: bool,
    },
    /// A Q-Link on Arrange while an audio region is picked or the track
    /// panel is open (layout spec 4c), or the panel's level and pan
    /// steps: knob 0 the track's level, 1 its pan, 2 the region's gain, 3
    /// its fades (`region`, an index into the track's regions: None when
    /// none is picked on the track); `ticks` detents, `fine` with SHIFT
    /// held. The app keeps the values and says them back in the lanes.
    Turn {
        track: usize,
        region: Option<usize>,
        knob: usize,
        ticks: i32,
        fine: bool,
    },
}

/// Where a clip launched now starts: on the next bar line while the
/// transport rolls (now, if the position is on one), else at the start of
/// the bar the position is in.
pub fn launch_origin(t: &TransportInfo) -> Tick {
    if t.state.rolling() {
        t.meter.next_bar(t.position)
    } else {
        t.position - t.position.rem_euclid(t.meter.bar_ticks())
    }
}

/// The launches and cues after `action`.
///
/// - While the transport rolls, Launch and StopClip become a [`Cue`] for
///   the next bar line, in place of the track's cue. The player starts it
///   there, and the app applies [`mpfree_play::Started`] to these lists.
/// - While it is stopped, they happen at once. A launched clip plays as if
///   it had started on the line of the bar the position is in, wherever
///   the song goes after.
/// - Uncue takes the track's cue back.
/// - NewClip launches its new clip at once from [`launch_origin`] (it
///   records from there), and takes the track's cue back.
pub fn clips_after(
    project: &Project,
    launches: &[Launch],
    cues: &[Cue],
    action: &TimeAction,
    t: &TransportInfo,
) -> (Vec<Launch>, Vec<Cue>) {
    let (mut launches, mut cues) = (launches.to_vec(), cues.to_vec());
    let rolling = t.state.rolling();
    let (track, slot) = match *action {
        TimeAction::Launch { track, slot } => (track, Some(slot)),
        TimeAction::StopClip(track) => (track, None),
        TimeAction::Uncue(track) => {
            cues.retain(|c| c.track != track);
            return (launches, cues);
        }
        TimeAction::NewClip { track, .. } => {
            cues.retain(|c| c.track != track);
            return (launches_after(&launches, action, t), cues);
        }
        _ => return (launches, cues),
    };
    cues.retain(|c| c.track != track);
    if rolling {
        cues.push(Cue { track, slot });
        return (launches, cues);
    }
    launches.retain(|l| l.track != track);
    if let Some((slot, clip)) = slot.and_then(|s| Some((s, project.clip(track, s)?))) {
        let bar = t.position - t.position.rem_euclid(t.meter.bar_ticks());
        launches.push(Launch {
            track,
            slot,
            origin: anchored(bar, clip.length, clip.speed),
            start: bar,
        });
    }
    (launches, cues)
}

/// Where a cue landing on `line` starts: its next line the playhead
/// plays. In a loop whose end comes first, that is the loop start's.
pub fn cue_tick(t: &TransportInfo, line: LaunchGrid) -> Tick {
    let bar = t.meter.bar_ticks();
    let next = line.next(t.position, bar);
    match t.playing_loop() {
        Some((a, b)) if t.state.rolling() && t.position < b && next >= b => line.next(a, bar),
        _ => next,
    }
}

impl TimeState {
    /// The line the cue on `track` lands on: as the time engine set it
    /// ([`TimeState::cue_lines`]), else the cued clip's own launch
    /// quantise, else the project's.
    pub fn cue_line(&self, track: usize) -> LaunchGrid {
        if let Some((_, g)) = self.cue_lines.iter().find(|(t, _)| *t == track) {
            return *g;
        }
        let own = self
            .cue(track)
            .and_then(|c| c.slot)
            .and_then(|s| self.clip(track, s))
            .and_then(|c| c.settings.launch);
        own.unwrap_or(self.launch)
    }
}

/// The launches after `action`, without cues: Launch replaces the track's
/// clip with one starting at [`launch_origin`]; StopClip removes it. Other
/// actions leave them as they are. The old clip stops when the new view is
/// swapped in, not on the bar: [`clips_after`] does it properly.
pub fn launches_after(launches: &[Launch], action: &TimeAction, t: &TransportInfo) -> Vec<Launch> {
    let mut out = launches.to_vec();
    match *action {
        TimeAction::Launch { track, slot } | TimeAction::NewClip { track, slot } => {
            out.retain(|l| l.track != track);
            out.push(Launch {
                track,
                slot,
                origin: launch_origin(t),
                start: launch_origin(t),
            });
        }
        TimeAction::StopClip(track) => out.retain(|l| l.track != track),
        _ => {}
    }
    out
}

/// What a touch means, given what the screen shows.
/// A tap on a track's name on the Clips grid: it picks the track
/// ([`TimeAction::Pick`]), or lets go of the arm when it has it.
pub fn name_tap(st: &TimeState, track: usize) -> TimeAction {
    if st.armed == Some(track) {
        TimeAction::Arm(None)
    } else {
        TimeAction::Pick(track)
    }
}

/// A tap on the Clips grid's cell at (`track`, `slot`): launch its clip,
/// stop it while it plays, take back a launch or stop waiting, make a
/// clip in an empty slot, or arm an empty Loop cell. None past the
/// tracks.
pub fn cell_tap(st: &TimeState, track: usize, slot: usize) -> Option<TimeAction> {
    if track >= st.tracks.len() || slot >= SLOTS {
        None
    } else if st.clip(track, slot).is_none() && st.tracks[track].loops.is_some() {
        // An empty Loop cell arms (again: lets go of) a loop there.
        let cell = (track, slot);
        Some(TimeAction::LoopCell(
            (st.loop_cell != Some(cell)).then_some(cell),
        ))
    } else if st.clip(track, slot).is_none() {
        Some(TimeAction::NewClip { track, slot })
    } else if st.cue(track).is_some_and(|c| {
        c.slot == Some(slot)
            || (c.slot.is_none() && st.launch(track).is_some_and(|l| l.slot == slot))
    }) {
        // Tapping what is cued takes it back.
        Some(TimeAction::Uncue(track))
    } else if st.launch(track).is_some_and(|l| l.slot == slot) {
        Some(TimeAction::StopClip(track))
    } else {
        Some(TimeAction::Launch { track, slot })
    }
}

/// SHIFT + a tap on the Clips grid (sequencer S5, Jay's card of 18 Nov
/// 2026): what MIDI Learn waits to give a control. A clip's cell (its
/// tap, whatever the cell holds), a row's scene button, Stop all, Back to
/// song, or a track's name (its arm). None for anything else on the grid,
/// where SHIFT keeps its job (a drag copies, "+ Track" adds a Loop track).
pub fn learn_target(ev: &UiEvent, st: &TimeState) -> Option<ParamAddr> {
    if ev.widget != CLIPS {
        return None;
    }
    let id = match ev.kind {
        EventKind::Select(i) if i / SLOTS < st.tracks.len() => {
            clips::clip((i / SLOTS) as u8, (i % SLOTS) as u8)
        }
        EventKind::Toggle(r) if r < SLOTS => clips::scene(r as u8),
        EventKind::Toggle(SLOTS) => clips::STOP_ALL,
        EventKind::Toggle(r) if r == SLOTS + BACK_TO_SONG => clips::BACK_TO_SONG,
        EventKind::Tab(t) if t < st.tracks.len() => clips::arm(t as u8),
        _ => return None,
    };
    Some(ParamAddr::new(Owner::Clips, id))
}

/// A learned control's press on [`Owner::Clips`] parameter `c` (MIDI
/// Learn's remote control): what a tap on the grid does now. None for a
/// track or row that isn't there, and for mutes and solos, which take a
/// value and go through the app's mute queue.
pub fn clips_press(st: &TimeState, c: Clips) -> Option<TimeAction> {
    let track = |t: u8| Some(usize::from(t)).filter(|&t| t < st.tracks.len());
    match c {
        Clips::StopAll => Some(TimeAction::StopAll),
        Clips::BackToSong => Some(TimeAction::BackToSong),
        Clips::Scene(r) => Some(usize::from(r))
            .filter(|&r| r < SLOTS)
            .map(TimeAction::LaunchScene),
        Clips::Clip(t, r) => cell_tap(st, usize::from(t), usize::from(r)),
        Clips::Stop(t) => track(t).map(TimeAction::StopClip),
        Clips::Arm(t) => track(t).map(|t| name_tap(st, t)),
        Clips::Mute(_) | Clips::Solo(_) => None,
    }
}

pub fn action(ev: &UiEvent, st: &TimeState) -> Option<TimeAction> {
    let t = &st.transport;
    match (ev.widget.as_str(), &ev.kind) {
        (TRANSPORT, EventKind::Tab(i)) => Some(match BarButton::ALL.get(*i)? {
            BarButton::Stop => TimeAction::Transport(Command::Stop),
            BarButton::Play => TimeAction::Transport(Command::Play),
            BarButton::Rec => TimeAction::Transport(Command::Record),
            BarButton::Mode => TimeAction::Transport(Command::ToggleOverdub),
            BarButton::Undo => TimeAction::Undo,
        }),
        (TRANSPORT, EventKind::Select(i)) => Some(TimeAction::Chip(*i)),
        (MORPH, EventKind::Step { index, delta }) => {
            let m = st.morph;
            let step = |r: usize| {
                (r as i64 + i64::from((*delta).signum())).rem_euclid(SLOTS as i64) as usize
            };
            Some(match index {
                0 => TimeAction::MorphRows {
                    a: step(m.a),
                    b: m.b,
                },
                _ => TimeAction::MorphRows {
                    a: m.a,
                    b: step(m.b),
                },
            })
        }
        (MORPH, EventKind::Value(x)) => Some(TimeAction::Morph(*x)),
        (MORPH, EventKind::Tab(0)) => st.morph.on.then_some(TimeAction::SnapBack),
        (MORPH, EventKind::Tab(1)) => st.morph.on.then_some(TimeAction::KeepMorph),
        (MORPH, EventKind::Tab(2)) => Some(TimeAction::SaveScene(if st.morph.pos < 0.5 {
            st.morph.a
        } else {
            st.morph.b
        })),
        (MORPH, EventKind::Tab(3)) => st
            .focus()
            .filter(|f| st.saved.contains(f))
            .map(|(track, slot)| TimeAction::Reload { track, slot }),
        (CLIPS, EventKind::Toggle(row)) => Some(match *row {
            r if r < SLOTS => TimeAction::LaunchScene(r),
            SLOTS => TimeAction::StopAll,
            r if r == SLOTS + 1 => TimeAction::SongView(true),
            r if r == SLOTS + 2 => TimeAction::RecSong(!st.song_rec),
            r if r == SLOTS + 3 => TimeAction::Edit(EditAction::Open),
            r if r == SLOTS + ADD_TRACK => TimeAction::AddTrack,
            r if r == SLOTS + BACK_TO_SONG => TimeAction::BackToSong,
            // The pager and the drag's edges are the page's own:
            // TimePage::page_tracks.
            _ => return None,
        }),
        // A drop on the grid: a move or a swap; the app makes it a copy
        // with SHIFT held.
        (CLIPS, EventKind::Drop { from, to }) if *from >= ROW_DROP => Some(TimeAction::DropRow {
            from: from - ROW_DROP,
            to: *to,
            copy: false,
        }),
        (CLIPS, EventKind::Drop { from, to }) => Some(TimeAction::DropClip {
            from: (from / SLOTS, from % SLOTS),
            to: (to / SLOTS, to % SLOTS),
            copy: false,
        }),
        (NOTES, kind) => Some(TimeAction::Edit(match *kind {
            EventKind::Tab(k) => EditAction::Button(k),
            EventKind::Select(i) => EditAction::Note {
                index: i,
                rev: st.edit.as_ref().map_or(0, |e| e.rev),
            },
            EventKind::Cell { row: PEN, col } => EditAction::Step(col as Tick),
            EventKind::Cell { row, col } => EditAction::Empty {
                tick: col as Tick,
                note: u8::try_from(row).ok()?,
            },
            EventKind::Marker { frame, .. } => {
                let (ticks, notes) = drop_of(frame);
                EditAction::Drop { ticks, notes }
            }
            EventKind::Value(v) => EditAction::Velocity(v),
            _ => return None,
        })),
        (EDIT_OPTIONS, EventKind::Step { index, delta }) => {
            Some(TimeAction::Edit(EditAction::Row {
                row: *index,
                delta: *delta,
            }))
        }
        (EDIT_OPTIONS, EventKind::Select(i)) => Some(TimeAction::Edit(EditAction::PickRow(*i))),
        // The tempo lane: a tap adds a point, a tap on one picks it (again:
        // lets go), a drag moves it.
        (
            SONG,
            EventKind::Place {
                index: NONE,
                row: TEMPO_ROW,
                at,
            },
        ) => Some(TimeAction::SongTempo(TempoEdit::Add(*at))),
        (
            SONG,
            EventKind::Place {
                index,
                row: TEMPO_ROW,
                at,
            },
        ) => Some(TimeAction::SongTempo(TempoEdit::Move {
            index: *index,
            at: *at,
        })),
        (
            SONG,
            EventKind::Cell {
                row: TEMPO_ROW,
                col,
            },
        ) => Some(TimeAction::SongTempo(TempoEdit::Pick(
            (st.tempo_pick != Some(*col)).then_some(*col),
        ))),
        // With a point picked, the last two buttons are Ramp and Delete.
        (SONG, EventKind::Tab(k @ (4 | 5))) if st.region.is_none() && st.tempo_pick.is_some() => {
            let index = st.tempo_pick?;
            let (_, _, ramp) = *st.tempo_points.get(index)?;
            Some(TimeAction::SongTempo(if *k == 4 {
                TempoEdit::Ramp { index, on: !ramp }
            } else {
                TempoEdit::Delete(index)
            }))
        }
        (SONG, EventKind::Tab(k)) if *k < AUDIO_BUTTONS.len() && st.region.is_some() => {
            let r = st.region?;
            Some(TimeAction::Audio(match k {
                0 => AudioAction::Split(r),
                1 => AudioAction::Erase(r),
                2 => AudioAction::Fades(r),
                3 => AudioAction::Gain(r),
                4 => AudioAction::Takes(r.track),
                _ => AudioAction::Undo,
            }))
        }
        (SONG, EventKind::Tab(k)) if (SECTION_TAP..BARS_BACK).contains(k) => {
            let k = k - SECTION_TAP;
            Some(TimeAction::PickSection(
                (st.section != Some(k)).then_some(k),
            ))
        }
        (SONG, EventKind::Place { index, row, at }) => Some(TimeAction::MovePlacement {
            index: *index,
            track: *row,
            start: *at,
            copy: false,
        }),
        (SONG, EventKind::Stretch { index, to }) => Some(TimeAction::StretchPlacement {
            index: *index,
            length: *to,
        }),
        (SONG, EventKind::Drop { from, to }) => Some(TimeAction::MoveSection {
            marker: *from,
            gap: *to,
            copy: false,
        }),
        (SONG, EventKind::Tab(ADD_AUDIO)) => Some(TimeAction::Audio(AudioAction::Add)),
        (SONG, EventKind::Tab(crate::song::ADD_TRACK)) => Some(TimeAction::AddTrack),
        (SONG, EventKind::Tab(crate::song::LIST)) => Some(TimeAction::ListView(true)),
        (SONG, EventKind::Tab(k)) if *k >= PANEL_CLOSE => {
            let track = st.audio_panel?;
            let lane = st.audio.iter().find(|l| l.track == track);
            let (mute, solo) = lane.map_or((false, false), |l| (l.mute, l.solo));
            Some(TimeAction::Audio(match *k {
                PANEL_CLOSE => AudioAction::ClosePanel,
                PANEL_MUTE => AudioAction::Mute { track, on: !mute },
                PANEL_SOLO => AudioAction::Solo { track, on: !solo },
                PANEL_TAKES => AudioAction::Takes(track),
                k if (PANEL_INPUT..PANEL_INPUT + INPUTS.len()).contains(&k) => AudioAction::Input {
                    track,
                    input: INPUTS[k - PANEL_INPUT].1,
                },
                k if (PANEL_MONITOR..PANEL_MONITOR + MONITORS.len()).contains(&k) => {
                    AudioAction::Monitor {
                        track,
                        monitor: MONITORS[k - PANEL_MONITOR].1,
                    }
                }
                crate::song::PANEL_MIX => AudioAction::Mix { track, step: 0 },
                crate::song::PANEL_FROM_LESS => AudioAction::Mix { track, step: -1 },
                crate::song::PANEL_FROM_MORE => AudioAction::Mix { track, step: 1 },
                crate::song::PANEL_BEFORE => AudioAction::MixEffects {
                    track,
                    after: false,
                },
                crate::song::PANEL_AFTER => AudioAction::MixEffects { track, after: true },
                _ => return None,
            }))
        }
        (SONG, EventKind::Marker { index, frame }) => Some(TimeAction::Audio(AudioAction::Move {
            region: RegionId {
                track: index / REGION_SLOTS,
                index: index % REGION_SLOTS,
            },
            to: Tick::try_from(*frame).ok()?,
            fine: false,
        })),
        (SONG, EventKind::Span { index, from, to }) => {
            let take = u16::try_from(*index).ok()?;
            Some(TimeAction::Audio(if from == to {
                AudioAction::UseTake(take)
            } else {
                AudioAction::Comp {
                    take,
                    from: *from,
                    to: *to,
                    fine: false,
                }
            }))
        }
        (SONG, EventKind::Step { index, delta }) => {
            let track = st.audio_panel?;
            Some(TimeAction::Audio(AudioAction::Turn {
                track,
                region: st.region.filter(|r| r.track == track).map(|r| r.index),
                knob: *index,
                ticks: *delta,
                fine: false,
            }))
        }
        (SONG, EventKind::Toggle(track)) => Some(TimeAction::Audio(AudioAction::Arm {
            track: *track,
            on: !st.audio.iter().any(|l| l.track == *track && l.armed),
        })),
        (SONG, EventKind::Cell { row, col: PANEL }) => {
            Some(TimeAction::Audio(AudioAction::Panel(*row)))
        }
        (SONG, EventKind::Cell { row, col }) => {
            Some(TimeAction::Audio(AudioAction::Pick(Some(RegionId {
                track: *row,
                index: *col,
            }))))
        }
        (SONG_LIST, EventKind::Toggle(k)) => match *k {
            LIST_LOOP => Some(TimeAction::ListLoop(!st.list_looped)),
            LIST_CLIPS => Some(TimeAction::SongView(false)),
            LIST_SONG => Some(TimeAction::SongView(true)),
            LIST_ADD => {
                (st.list.len() < mpfree_seq::scenes::MAX_LIST_STEPS).then_some(TimeAction::ListAdd)
            }
            LIST_TO_SONG => (!st.list.is_empty()).then_some(TimeAction::ListToSong),
            LIST_PLAY => st.list_pick.map(TimeAction::ListPlay),
            LIST_REMOVE => st.list_pick.map(TimeAction::ListRemove),
            _ => None,
        },
        (SONG_LIST, EventKind::Step { index, delta }) if *index >= SCENE_STEP => {
            let step = index - SCENE_STEP;
            (step < st.list.len()).then_some(TimeAction::ListScene {
                step,
                delta: *delta,
            })
        }
        (SONG_LIST, EventKind::Step { index, delta }) => {
            let r = st.list.get(*index)?;
            let to = i32::from(r.times) + delta;
            (1..=mpfree_seq::scenes::MAX_TIMES as i32)
                .contains(&to)
                .then_some(TimeAction::ListTimes {
                    step: *index,
                    delta: *delta,
                })
        }
        (SONG_LIST, EventKind::Select(k)) => {
            (*k < st.list.len()).then_some(TimeAction::ListPick(Some(*k)))
        }
        (SONG, EventKind::Tab(k)) => match k {
            0 => Some(TimeAction::SongView(false)),
            1 => Some(TimeAction::SongMode(!st.song_on)),
            2 => Some(TimeAction::ScenesToSong),
            3 => Some(TimeAction::SongToScenes),
            4 => st.selected.map(TimeAction::MakeUnique),
            5 => st.selected.map(TimeAction::DeletePlacement),
            &REC => Some(TimeAction::RecSong(!st.song_rec)),
            _ => None,
        },
        (SONG, EventKind::Select(i)) => {
            Some(TimeAction::SelectPlacement((*i != NONE).then_some(*i)))
        }
        // The sound line (`SOUND_TAP + track`) is the app's: the
        // track's instrument.
        (CLIPS, EventKind::Tab(track)) if *track >= crate::widgets::SOUND_TAP => None,
        (CLIPS, EventKind::Tab(track)) => Some(name_tap(st, *track)),
        (CLIPS, EventKind::Select(i)) => cell_tap(st, i / SLOTS, i % SLOTS),
        (RECORD, EventKind::Step { index, delta }) => {
            let d = (*delta).signum();
            match *index {
                0 => {
                    // None, then each track in turn.
                    let n = st.tracks.len() as i64 + 1;
                    let now = st.armed.map_or(0, |a| a as i64 + 1);
                    let next = (now + i64::from(d)).rem_euclid(n);
                    Some(TimeAction::Arm(if next == 0 {
                        None
                    } else {
                        Some(next as usize - 1)
                    }))
                }
                1 => Some(TimeAction::Transport(Command::ToggleOverdub)),
                2 => {
                    let bars = (i32::from(t.count_in) + d).clamp(0, i32::from(MAX_COUNT_IN)) as u8;
                    (bars != t.count_in).then_some(TimeAction::Transport(Command::SetCountIn(bars)))
                }
                3 => {
                    let bar = t.meter.bar_ticks();
                    let now = match t.loop_range {
                        None => 0,
                        Some((a, b)) => LOOP_BARS
                            .iter()
                            .position(|&n| a == 0 && b == n * bar)
                            // A loop set elsewhere steps from the nearest length.
                            .unwrap_or_else(|| {
                                LOOP_BARS
                                    .iter()
                                    .rposition(|&n| n * bar <= b - a)
                                    .unwrap_or(1)
                            }),
                    };
                    let next = (now as i32 + d).clamp(0, LOOP_BARS.len() as i32 - 1) as usize;
                    let range = (next > 0).then(|| (0, LOOP_BARS[next] * bar));
                    (range != t.loop_range)
                        .then_some(TimeAction::Transport(Command::SetLoop(range)))
                }
                // While the song's tempo points play, from the point in
                // effect at the playhead (a ramp's start, not where it
                // has glided to); the app gives it to that point.
                4 => {
                    let now = st.tempo_here.unwrap_or(t.bpm);
                    let bpm = (now + f64::from(d)).clamp(MIN_BPM, MAX_BPM);
                    (bpm != now).then_some(TimeAction::Transport(Command::SetTempo(bpm)))
                }
                // Either way flips it, for the armed track.
                5 => st.armed.and_then(|a| {
                    let on = st.tracks.get(a)?.pressure;
                    Some(TimeAction::RecordPressure { track: a, on: !on })
                }),
                // Off, then the grids from the coarsest.
                6 => st.armed.and_then(|a| {
                    let now = st.tracks.get(a)?.record_quantise;
                    let k = now.map_or(0, |g| {
                        QUANTISE_GRIDS
                            .iter()
                            .position(|&q| q == g)
                            .map_or(0, |i| i + 1)
                    });
                    let next = (k as i32 + d).clamp(0, QUANTISE_GRIDS.len() as i32) as usize;
                    let grid = (next > 0).then(|| QUANTISE_GRIDS[next - 1]);
                    (grid != now).then_some(TimeAction::RecordQuantise { track: a, grid })
                }),
                // At once, the note grids, then the bars.
                7 => {
                    let now = LAUNCH_GRIDS
                        .iter()
                        .position(|&g| g == st.launch)
                        .unwrap_or(5);
                    let next = (now as i32 + d).clamp(0, LAUNCH_GRIDS.len() as i32 - 1) as usize;
                    (next != now).then_some(TimeAction::LaunchGrid(LAUNCH_GRIDS[next]))
                }
                TRANSPOSE_ROW | FOLLOWS_ROW => transpose_step(st, *index, d).ok(),
                _ => None,
            }
        }
        (CLIP_CARD, EventKind::Step { index, delta }) => card_step(st, *index, *delta).ok(),
        (SCENE_CARD, EventKind::Step { index, delta }) => scene_step(st, *index, *delta).ok(),
        // Rename is the app's: the text keyboard.
        (SCENE_CARD, EventKind::Toggle(SCENE_MUTES_NOW)) => scene_mutes_now(st).ok(),
        (CLIP_CARD, EventKind::Toggle(b)) => {
            let (track, slot) = st.focus()?;
            match b {
                0 => Some(TimeAction::SaveState { track, slot }),
                1 => st
                    .saved
                    .contains(&(track, slot))
                    .then_some(TimeAction::Reload { track, slot }),
                _ => None,
            }
        }
        // Each only while it has something to do (it is faint otherwise).
        (TAKES, EventKind::Tab(0)) => (!st.done.is_empty()).then_some(TimeAction::Undo),
        (TAKES, EventKind::Tab(1)) => (!st.undone.is_empty()).then_some(TimeAction::Redo),
        (TAKES, EventKind::Tab(2)) => st.capture.is_some().then_some(TimeAction::Capture),
        _ => None,
    }
}

/// Why a touch that [`action`] answers with nothing changes nothing, for
/// the hint line: a faint button, or a setting already at its end (Jay, 13
/// Nov: say in place when a press would send nothing).
pub fn why_not(ev: &UiEvent, st: &TimeState) -> Option<String> {
    let end = |delta: i32, low: &str, high: &str| {
        if delta < 0 {
            low.to_string()
        } else {
            high.to_string()
        }
    };
    match (ev.widget.as_str(), &ev.kind) {
        (MORPH, EventKind::Tab(0 | 1)) if !st.morph.on => {
            Some("Move the crossfader first: Snap back and Keep work on a morph".to_string())
        }
        (MORPH, EventKind::Tab(3)) | (CLIP_CARD, EventKind::Toggle(1)) => Some(match st.focus() {
            None => "Arm a track with a clip: Reload clip reloads its clip".to_string(),
            Some(_) => {
                "Save state on the Clip card first: Reload clip brings that state back".to_string()
            }
        }),
        (SONG_LIST, EventKind::Toggle(LIST_PLAY | LIST_REMOVE)) if st.list_pick.is_none() => {
            Some("Tap a step first".to_string())
        }
        (SONG_LIST, EventKind::Toggle(LIST_TO_SONG)) => {
            Some("Add a scene to the list first".to_string())
        }
        (SONG_LIST, EventKind::Toggle(LIST_ADD)) => Some(format!(
            "The list is full: {} steps",
            mpfree_seq::scenes::MAX_LIST_STEPS
        )),
        (SONG_LIST, EventKind::Step { index, delta }) if *index < SCENE_STEP => Some(end(
            *delta,
            "Once is the fewest",
            &format!("{} times is the most", mpfree_seq::scenes::MAX_TIMES),
        )),
        (TAKES, EventKind::Tab(0)) => Some("Nothing to undo yet".to_string()),
        (TAKES, EventKind::Tab(1)) => Some("Nothing to redo: Undo something first".to_string()),
        (TAKES, EventKind::Tab(2)) => {
            Some("Nothing to capture yet: play the pads while not recording".to_string())
        }
        (SONG, EventKind::Tab(4 | 5)) if st.selected.is_none() => {
            Some("Tap a block in the song first".to_string())
        }
        (RECORD, EventKind::Step { index: 2, delta }) => Some(end(
            *delta,
            "Count-in is off already",
            &format!("Count-in is at its longest, {MAX_COUNT_IN} bars"),
        )),
        (RECORD, EventKind::Step { index: 3, delta }) => Some(end(
            *delta,
            "The loop is off already",
            &format!(
                "The loop is at its longest, {} bars",
                LOOP_BARS[LOOP_BARS.len() - 1]
            ),
        )),
        (RECORD, EventKind::Step { index: 4, delta }) => Some(end(
            *delta,
            &format!("The tempo is at its lowest, {MIN_BPM:.0} BPM"),
            &format!("The tempo is at its highest, {MAX_BPM:.0} BPM"),
        )),
        (RECORD, EventKind::Step { index: 5, .. }) if st.armed.is_none() => {
            Some("Arm a track first: pressure records on the armed track".to_string())
        }
        (RECORD, EventKind::Step { index: 6, .. }) if st.armed.is_none() => {
            Some("Arm a track first: quantise is the armed track's".to_string())
        }
        (RECORD, EventKind::Step { index: 6, delta }) => Some(end(
            *delta,
            "Quantise is off already: the notes go in as played",
            &format!(
                "Quantise is at its finest, {}",
                grid_name(QUANTISE_GRIDS[QUANTISE_GRIDS.len() - 1])
            ),
        )),
        (RECORD, EventKind::Step { index: 7, delta }) => Some(end(
            *delta,
            "Launch is at once already: a tapped clip starts on the next tick",
            "Launch is at its longest, 8 bars",
        )),
        (RECORD, EventKind::Step { index, delta })
            if *index == TRANSPOSE_ROW || *index == FOLLOWS_ROW =>
        {
            transpose_step(st, *index, *delta).err()
        }
        (CLIP_CARD, EventKind::Step { index, delta }) => card_step(st, *index, *delta).err(),
        (SCENE_CARD, EventKind::Step { index, delta }) => scene_step(st, *index, *delta).err(),
        (SCENE_CARD, EventKind::Toggle(SCENE_MUTES_NOW)) => scene_mutes_now(st).err(),
        _ => None,
    }
}

/// A note value for a grid of `ticks`: "1/16", "1/8T", or the ticks.
pub fn grid_name(ticks: u32) -> String {
    let whole = PPQ * 4;
    if ticks > 0 && whole.is_multiple_of(ticks) {
        let n = whole / ticks;
        if n.is_multiple_of(3) && (n / 3).is_power_of_two() {
            return format!("1/{}T", n * 2 / 3);
        }
        return format!("1/{n}");
    }
    format!("{ticks} ticks")
}

/// A speed as the card shows it: "x1", "x1/2", "x3/2".
pub fn speed_name(s: Speed) -> String {
    if s.den == 1 {
        format!("×{}", s.num)
    } else {
        format!("×{}/{}", s.num, s.den)
    }
}

fn follow_name(a: FollowAction) -> &'static str {
    match a {
        FollowAction::Stay => "Stay",
        FollowAction::Again => "Again",
        FollowAction::Next => "Next",
        FollowAction::Previous => "Previous",
        FollowAction::First => "First",
        FollowAction::Last => "Last",
        FollowAction::Any => "Any",
        FollowAction::Other => "Other",
        FollowAction::Stop => "Stop",
    }
}

/// The Record panel's tabs: Record, Clip with the focused clip's name
/// ("Clip · Verse B"; its track is the armed one, lit on the grid), and
/// Scene (the scene editor, S4).
pub fn panel_tabs(st: &TimeState) -> Vec<String> {
    let clip = match st.focus().and_then(|(t, s)| st.clip(t, s)) {
        Some(c) => format!("Clip · {}", c.name),
        None => "Clip".to_string(),
    };
    vec!["Record".to_string(), clip, "Scene".to_string()]
}

/// The Record panel's tab widths but the last's (Scene): Record, Clip.
pub const PANEL_TAB_WIDTHS: [i32; 2] = [66, 104];

/// The scene editor's rows for the scene in focus.
pub fn scene_rows(st: &TimeState) -> Vec<OptionRow> {
    let row = |k: usize, value: String| OptionRow {
        name: SCENE_ROWS[k].to_string(),
        value,
    };
    let s = &st.scene;
    let dash = || "—".to_string();
    let r = st.scene_focus;
    vec![
        row(
            0,
            if s.name.is_empty() {
                format!("{}", r + 1)
            } else {
                format!("{} · {}", r + 1, s.name)
            },
        ),
        row(
            1,
            s.follow
                .map_or_else(|| "off".to_string(), |f| follow_name(f.action).to_string()),
        ),
        row(
            2,
            s.follow
                .map_or_else(dash, |f| bars_label(i64::from(f.bars))),
        ),
        row(3, s.follow.map_or_else(dash, |f| format!("{} %", f.chance))),
        row(
            4,
            s.follow
                .filter(|f| f.chance < 100)
                .map_or_else(dash, |f| follow_name(f.other).to_string()),
        ),
        row(5, mutes_words(st, s.mutes)),
        // The armed track's program change (as the Record tab's rows are
        // the armed track's), shown 1 to 128 as on the gear, and its bank
        // select.
        row(
            SCENE_PROGRAM,
            st.armed.map_or_else(dash, |t| {
                s.program(t)
                    .map_or_else(|| "off".to_string(), |p| program_name(p.program))
            }),
        ),
        row(
            SCENE_BANK,
            match st.armed.and_then(|t| s.program(t)) {
                None => dash(),
                Some(ClipProgram { bank: None, .. }) => "off".to_string(),
                Some(ClipProgram { bank: Some(b), .. }) => bank_name(b),
            },
        ),
        row(8, programs_words(st, s)),
    ]
}

/// A program change as the gear shows it: 1 to 128.
pub fn program_name(program: u8) -> String {
    (u16::from(program) + 1).to_string()
}

/// The tracks a scene sends a program change, as its Sent to row says
/// them: "none", "Keys", "Keys, Bass", "Keys +2".
fn programs_words(st: &TimeState, s: &SceneSettings) -> String {
    let name = |p: &TrackProgram| st.track_name(p.track);
    match s.programs.as_slice() {
        [] => "none".to_string(),
        [a] => name(a),
        [a, b] => format!("{}, {}", name(a), name(b)),
        [a, rest @ ..] => format!("{} +{}", name(a), rest.len()),
    }
}

/// A scene's mutes as its Mutes row says them: "as they are" (it leaves
/// them), "none", "Keys, Bass", "Keys +2".
fn mutes_words(st: &TimeState, mutes: Option<u128>) -> String {
    let Some(m) = mutes else {
        return "as they are".to_string();
    };
    let names: Vec<String> = (0..st.tracks.len().min(MAX_TRACKS))
        .filter(|&t| m & (1 << t) != 0)
        .map(|t| st.track_name(t))
        .collect();
    match names.len() {
        0 => "none".to_string(),
        1 | 2 => names.join(", "),
        n => format!("{} +{}", names[0], n - 1),
    }
}

/// The scene editor's buttons: Rename and Mutes now.
pub fn scene_buttons(_st: &TimeState) -> Vec<OptionButton> {
    vec![
        OptionButton::new("Rename", true),
        OptionButton::new("Mutes now", true),
    ]
}

/// The tracks muted now, as a scene's mutes.
fn mutes_now(st: &TimeState) -> u128 {
    st.tracks
        .iter()
        .take(MAX_TRACKS)
        .enumerate()
        .filter(|(_, t)| t.mute)
        .fold(0, |m, (k, _)| m | 1 << k)
}

/// Mutes now: the scene in focus mutes the tracks muted now, and plays
/// the others, when it starts.
fn scene_mutes_now(st: &TimeState) -> Result<TimeAction, String> {
    let m = mutes_now(st);
    if st.scene.mutes == Some(m) {
        return Err(format!(
            "Scene {} mutes these tracks already",
            st.scene_focus + 1
        ));
    }
    Ok(TimeAction::SceneSettings {
        row: st.scene_focus,
        card_row: 5,
        settings: SceneSettings {
            mutes: Some(m),
            ..st.scene.clone()
        },
    })
}

/// How long the scene in focus plays in whole bars: its longest clip
/// (one bar at least), for a follow turned on.
fn scene_bars(st: &TimeState) -> u16 {
    let bar = st.transport.meter.bar_ticks().max(1) as f64;
    let longest = st
        .tracks
        .iter()
        .filter_map(|t| t.slots.get(st.scene_focus)?.as_ref())
        .map(|c| c.song_len())
        .fold(0.0, f64::max);
    ((longest / bar).ceil() as u16).clamp(1, MAX_FOLLOW_BARS)
}

/// What a step of the scene editor's row `index` asks for, or why it
/// asks for nothing (a setting at its end, or off).
fn scene_step(st: &TimeState, index: usize, delta: i32) -> Result<TimeAction, String> {
    let d = delta.signum();
    let step = |now: usize, n: usize| {
        let next = now as i64 + i64::from(d);
        (0..n as i64).contains(&next).then_some(next as usize)
    };
    let end = |low: String, high: String| Err(if d < 0 { low } else { high });
    let row = st.scene_focus;
    let mut s = st.scene.clone();
    let set = |s: SceneSettings| {
        Ok(TimeAction::SceneSettings {
            row,
            card_row: index as u8,
            settings: s,
        })
    };
    let off = || {
        format!(
            "Turn Follow on first: scene {} plays until another starts",
            row + 1
        )
    };
    match index {
        0 => match step(row, SLOTS) {
            Some(k) => Ok(TimeAction::SceneFocus(k)),
            None => end(
                "Scene 1 is the first".into(),
                format!("Scene {SLOTS} is the last"),
            ),
        },
        1 => {
            let now = s.follow.map_or(0, |f| {
                FOLLOWS
                    .iter()
                    .position(|&a| a == f.action)
                    .map_or(0, |k| k + 1)
            });
            match step(now, FOLLOWS.len() + 1) {
                Some(0) => {
                    s.follow = None;
                    set(s)
                }
                Some(k) => {
                    let action = FOLLOWS[k - 1];
                    s.follow = Some(match s.follow {
                        Some(f) => Follow { action, ..f },
                        // After one pass of the scene, to begin with.
                        None => Follow::after(scene_bars(st), action),
                    });
                    set(s)
                }
                None => end(
                    "Follow is off already".into(),
                    "Follow is at its last action, Stop".into(),
                ),
            }
        }
        2 => {
            let f = s.follow.ok_or_else(off)?;
            match step(usize::from(f.bars) - 1, usize::from(MAX_FOLLOW_BARS)) {
                Some(k) => {
                    s.follow = Some(Follow {
                        bars: k as u16 + 1,
                        ..f
                    });
                    set(s)
                }
                None => end(
                    "Follow is at its soonest, after 1 bar".into(),
                    format!("Follow is at its latest, after {MAX_FOLLOW_BARS} bars"),
                ),
            }
        }
        3 => {
            let f = s.follow.ok_or_else(off)?;
            let next = if d > 0 {
                (f.chance / 10 + 1) * 10
            } else {
                f.chance.div_ceil(10).saturating_sub(1) * 10
            };
            if next > 100 || (d < 0 && f.chance == 0) {
                return end(
                    "Chance is at its least, 0 %: the Or action always plays".into(),
                    "Chance is full already, 100 %".into(),
                );
            }
            s.follow = Some(Follow {
                chance: next.min(100),
                ..f
            });
            set(s)
        }
        4 => {
            let f = s.follow.ok_or_else(off)?;
            if f.chance >= 100 {
                return Err(
                    "Set Chance under 100 % first: then Or plays the rest of the time".into(),
                );
            }
            let now = OTHERS.iter().position(|&a| a == f.other).unwrap_or(0);
            match step(now, OTHERS.len()) {
                Some(k) => {
                    s.follow = Some(Follow {
                        other: OTHERS[k],
                        ..f
                    });
                    set(s)
                }
                None => end(
                    "Or is at its first, Stay".into(),
                    "Or is at its last action, Stop".into(),
                ),
            }
        }
        5 => match (s.mutes, d > 0) {
            (Some(_), false) => {
                s.mutes = None;
                set(s)
            }
            (None, true) => {
                s.mutes = Some(mutes_now(st));
                set(s)
            }
            (None, false) => {
                Err("Mutes is at its first: the scene leaves the mutes as they are".into())
            }
            (Some(_), true) => Err("Mutes now takes the tracks muted now".into()),
        },
        SCENE_PROGRAM | SCENE_BANK => {
            let t = st
                .armed
                .ok_or("Arm a track first: Program is the program change the scene sends it")?;
            let now = s.program(t);
            let to = if index == SCENE_BANK {
                let p = now.ok_or("Set Program first: the bank goes with its program change")?;
                Some(ClipProgram {
                    bank: bank_next(p, delta, 1)?,
                    ..p
                })
            } else {
                program_next(now, delta)?
            };
            s.set_program(t, to);
            set(s)
        }
        8 => Err(
            "Arm a track and set Program: this row lists the tracks the scene sends a program change"
                .into(),
        ),
        _ => Err(String::new()),
    }
}

/// The scene editor's step with SHIFT held or not: with SHIFT, Bank steps
/// a whole CC 0 (128), as on the clip card; the other rows step as
/// without it.
pub fn scene_action(
    st: &TimeState,
    index: usize,
    delta: i32,
    shift: bool,
) -> Result<TimeAction, String> {
    if !(shift && index == SCENE_BANK) {
        return scene_step(st, index, delta);
    }
    let t = st
        .armed
        .ok_or("Arm a track first: Program is the program change the scene sends it")?;
    let mut s = st.scene.clone();
    let p = s
        .program(t)
        .ok_or("Set Program first: the bank goes with its program change")?;
    s.set_program(
        t,
        Some(ClipProgram {
            bank: bank_next(p, delta, 128)?,
            ..p
        }),
    );
    Ok(TimeAction::SceneSettings {
        row: st.scene_focus,
        card_row: SCENE_BANK as u8,
        settings: s,
    })
}

/// A step of a Program row: off, then 1 to 128 (the bank kept).
fn program_next(now: Option<ClipProgram>, delta: i32) -> Result<Option<ClipProgram>, String> {
    let k = now.map_or(0, |p| i32::from(p.program) + 1) + delta.signum();
    match k {
        ..0 => Err("Program is off already: no program change goes out".into()),
        0 => Ok(None),
        1..=128 => Ok(Some(ClipProgram {
            program: (k - 1) as u8,
            bank: now.and_then(|p| p.bank),
        })),
        _ => Err("Program is at its last, 128".into()),
    }
}

/// A step of a Bank row by `by` (1, or 128 with SHIFT: a whole CC 0):
/// off, then 0/0 up to 127/127.
fn bank_next(p: ClipProgram, delta: i32, by: i32) -> Result<Option<u16>, String> {
    // Off is one below 0/0.
    let now = p.bank.map_or(-1, i32::from);
    let max = i32::from(ClipProgram::MAX_BANK);
    let next = if delta > 0 {
        if now >= max {
            return Err(format!(
                "Bank is at its last, {}",
                bank_name(ClipProgram::MAX_BANK)
            ));
        }
        (now + by).min(max)
    } else {
        match now {
            ..0 => {
                return Err("Bank is off already: no bank select goes before the program".into())
            }
            // 0/0, then off.
            0 => -1,
            _ => (now - by).max(0),
        }
    };
    Ok(u16::try_from(next).ok())
}

/// The clip settings card's rows, for the focused clip; none without one.
pub fn clip_rows(st: &TimeState) -> Vec<OptionRow> {
    match st.focus().and_then(|(t, s)| st.clip(t, s)) {
        Some(c) => {
            let mut rows = card_rows(c.length, c.settings);
            rows.push(OptionRow {
                name: CLIP_ROWS[CLIP_MUTATE].to_string(),
                value: mutate_value(c),
            });
            rows
        }
        None => Vec::new(),
    }
}

/// The clip card's Mutate row's value (S6): "original" until the clip is
/// mutated, then "variant" (the hint line and the Takes list number it);
/// "—" on a clip with no notes or a loop's cell.
pub fn mutate_value(c: &ClipInfo) -> String {
    match c.variation {
        _ if c.looped.is_some() || (c.notes == 0 && c.variation.is_none()) => "—".to_string(),
        Some(_) => "variant".to_string(),
        None => "original".to_string(),
    }
}

/// The clip card's buttons, under its rows (S4): Save state, and Reload
/// while the focused clip has a saved state, outlined while its reload
/// waits. None without a clip; Save state is off on a loop's cell (its
/// layers have the looper's undo).
pub fn card_buttons(st: &TimeState) -> Vec<OptionButton> {
    let Some((track, slot)) = st.focus() else {
        return Vec::new();
    };
    let looped = st.clip(track, slot).is_some_and(|c| c.looped.is_some());
    let mut reload = OptionButton::new("Reload", st.saved.contains(&(track, slot)));
    reload.waiting = st.reload_waiting((track, slot));
    vec![OptionButton::new("Save state", !looped), reload]
}

/// The card's rows for a clip `length` ticks long (its own ticks) with
/// `s`.
pub fn card_rows(length: Tick, s: ClipSettings) -> Vec<OptionRow> {
    let row = |k: usize, value: String| OptionRow {
        name: CLIP_ROWS[k].to_string(),
        value,
    };
    let steps = (length + CARD_STEP - 1) / CARD_STEP;
    let dash = "—".to_string();
    vec![
        row(
            0,
            match steps {
                1 => "1 step".to_string(),
                // Past 999 the word does not fit: whole bars as bars.
                ..=999 => format!("{steps} steps"),
                _ if steps % 16 == 0 => format!("{} bars", steps / 16),
                _ => format!("{steps} st"),
            },
        ),
        row(1, speed_name(s.speed)),
        row(
            2,
            s.swing.map_or_else(
                || "track".to_string(),
                |w| {
                    if w.grid == PPQ / 4 {
                        format!("{} %", w.amount)
                    } else {
                        format!("{}% {}", w.amount, grid_name(w.grid))
                    }
                },
            ),
        ),
        row(
            3,
            s.quantise
                .map_or_else(|| "off".to_string(), |q| grid_name(q.grid)),
        ),
        row(
            4,
            s.quantise
                .map_or_else(|| dash.clone(), |q| format!("{} %", q.strength)),
        ),
        row(
            5,
            // Its chance and other action (set by an import) are kept, not
            // shown: the box has no room.
            s.follow
                .map_or_else(|| "off".to_string(), |f| follow_name(f.action).to_string()),
        ),
        row(
            6,
            s.follow
                .map_or(dash.clone(), |f| bars_label(i64::from(f.bars))),
        ),
        // The project's launch quantise unless it has its own.
        row(
            7,
            s.launch.map_or_else(|| "project".to_string(), launch_name),
        ),
        row(8, MODES[mode_of(&s)].to_string()),
        // The program change it sends its track's outside gear as it
        // starts (S5): shown 1 to 128, as on the gear.
        row(
            9,
            s.program
                .map_or_else(|| "off".to_string(), |p| program_name(p.program)),
        ),
        // Its bank select as the two numbers sent, CC 0 / CC 32.
        row(
            10,
            match s.program {
                None => dash.clone(),
                Some(ClipProgram { bank: None, .. }) => "off".to_string(),
                Some(ClipProgram { bank: Some(b), .. }) => bank_name(b),
            },
        ),
        // The order it plays its steps in (S6); "—" on a clip it can't
        // move (not two whole 1/16 steps or more), which plays forward.
        row(
            CLIP_DIRECTION,
            match s.direction {
                d if d != Direction::Forward && direction_steps(d, length).is_none() => dash,
                d => direction_name(d).to_string(),
            },
        ),
    ]
}

/// A bank select as the gear's manual names it: CC 0's value / CC 32's.
pub fn bank_name(bank: u16) -> String {
    format!("{}/{}", bank >> 7, bank & 0x7F)
}

/// What a step of the card's row `index` asks for, with SHIFT held or
/// not, or why it asks for nothing (empty when there is nothing to say).
/// With SHIFT, Length doubles (the notes repeated) or halves, Speed
/// doubles or halves, and Bank steps a whole CC 0 (128); the other rows
/// step as without it.
pub fn card_action(
    st: &TimeState,
    index: usize,
    delta: i32,
    shift: bool,
) -> Result<TimeAction, String> {
    if shift && index == 10 {
        return bank_step(st, delta, 128);
    }
    // SHIFT + › on Mutate: a bolder variation.
    if shift && index == CLIP_MUTATE && delta > 0 {
        return match card_step(st, index, delta)? {
            TimeAction::Mutate {
                track,
                slot,
                melodic,
                ..
            } => Ok(TimeAction::Mutate {
                track,
                slot,
                more: true,
                melodic,
            }),
            a => Ok(a),
        };
    }
    if !shift || index > 1 {
        return card_step(st, index, delta);
    }
    let (track, slot) = st
        .focus()
        .ok_or("No clip to set: arm a track with a clip, or make one")?;
    let c = st.clip(track, slot).ok_or("No clip to set")?;
    if c.looped.is_some() {
        return Err("A loop cell plays its loop as recorded: only Follow and After apply".into());
    }
    let up = delta > 0;
    if index == 0 {
        let max = (MAX_CLIP_TICKS / CARD_STEP) as usize;
        let now = ((c.length + CARD_STEP - 1) / CARD_STEP) as usize;
        let steps = if up { now * 2 } else { now / 2 };
        if steps == 0 || steps > max {
            return Err(if up {
                format!("The clip can't double: {max} steps at most")
            } else {
                "The clip can't halve: it is 1 step long".to_string()
            });
        }
        return Ok(TimeAction::ClipLength {
            track,
            slot,
            steps,
            copy: up,
        });
    }
    let sp = c.settings.speed;
    let (num, den) = if up {
        (u16::from(sp.num) * 2, u16::from(sp.den))
    } else {
        (u16::from(sp.num), u16::from(sp.den) * 2)
    };
    let g = gcd(num, den);
    let speed = u8::try_from(num / g)
        .ok()
        .zip(u8::try_from(den / g).ok())
        .and_then(|(n, d)| Speed::new(n, d))
        .ok_or_else(|| {
            let ratio = f64::from(num) / f64::from(den);
            let way = if up { "double" } else { "halve" };
            match ratio {
                r if r > 4.0 => format!("{} can't double: ×4 is the fastest", speed_name(sp)),
                r if r < 0.125 => format!("{} can't halve: ×1/8 is the slowest", speed_name(sp)),
                _ => format!(
                    "{} can't {way}: ×{}/{} is not a speed (the Speed row's arrows step it)",
                    speed_name(sp),
                    num / g,
                    den / g
                ),
            }
        })?;
    Ok(TimeAction::ClipSettings {
        track,
        slot,
        row: 1,
        settings: ClipSettings {
            speed,
            ..c.settings
        },
    })
}

fn gcd(a: u16, b: u16) -> u16 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

/// What a step of the card's row `index` asks for, or why it asks for
/// nothing (no clip, a loop cell, a setting at its end).
fn card_step(st: &TimeState, index: usize, delta: i32) -> Result<TimeAction, String> {
    let (track, slot) = st
        .focus()
        .ok_or("No clip to set: arm a track with a clip, or make one")?;
    let c = st.clip(track, slot).ok_or("No clip to set")?;
    if c.looped.is_some() && !(5..8).contains(&index) {
        return Err(
            "A loop cell plays its loop as recorded: only Follow, After and Launch apply".into(),
        );
    }
    let d = delta.signum();
    // Within 0..n from `now`, or None at the end it is pushed past.
    let step = |now: usize, n: usize| {
        let next = now as i64 + i64::from(d);
        (0..n as i64).contains(&next).then_some(next as usize)
    };
    let mut s = c.settings;
    let set = |s: ClipSettings| {
        Ok(TimeAction::ClipSettings {
            track,
            slot,
            row: index as u8,
            settings: s,
        })
    };
    let end = |low: String, high: String| Err(if d < 0 { low } else { high });
    match index {
        0 => {
            let max = (MAX_CLIP_TICKS / CARD_STEP) as usize;
            let now = ((c.length + CARD_STEP - 1) / CARD_STEP) as usize;
            match step(now - 1, max) {
                Some(k) => Ok(TimeAction::ClipLength {
                    track,
                    slot,
                    steps: k + 1,
                    copy: false,
                }),
                None => end(
                    "The clip is at its shortest, 1 step".into(),
                    format!("The clip is at its longest, {max} steps"),
                ),
            }
        }
        1 => {
            let ratio = |(n, d): (u8, u8)| f64::from(n) / f64::from(d);
            let at = ratio((s.speed.num, s.speed.den));
            // A speed set elsewhere steps from the nearest.
            let now = SPEEDS
                .iter()
                .position(|&(n, d)| (n, d) == (s.speed.num, s.speed.den))
                .unwrap_or_else(|| SPEEDS.iter().rposition(|&p| ratio(p) <= at).unwrap_or(0));
            match step(now, SPEEDS.len()) {
                Some(k) => {
                    s.speed = Speed::new(SPEEDS[k].0, SPEEDS[k].1).ok_or("speed")?;
                    set(s)
                }
                None => end(
                    "The clip is at its slowest, ×1/8".into(),
                    "The clip is at its fastest, ×4".into(),
                ),
            }
        }
        2 => {
            let lo = *CLIP_SWING.start();
            let n = usize::from(CLIP_SWING.end() - lo) + 2;
            let now = s
                .swing
                .map_or(0, |w| usize::from(w.amount.saturating_sub(lo)) + 1);
            match step(now, n) {
                Some(0) => {
                    s.swing = None;
                    set(s)
                }
                Some(k) => {
                    let grid = s.swing.map_or(PPQ / 4, |w| w.grid);
                    s.swing = Some(Swing::new(grid, lo + k as u8 - 1).ok_or("swing")?);
                    set(s)
                }
                None => end(
                    "Swing is the track's already".into(),
                    format!("Swing is at its most, {} %", CLIP_SWING.end()),
                ),
            }
        }
        3 => {
            let now = s.quantise.map_or(0, |q| {
                QUANTISE_GRIDS
                    .iter()
                    .position(|&g| g == q.grid)
                    .unwrap_or_else(|| {
                        QUANTISE_GRIDS
                            .iter()
                            .rposition(|&g| g >= q.grid)
                            .unwrap_or(0)
                    })
                    + 1
            });
            match step(now, QUANTISE_GRIDS.len() + 1) {
                Some(0) => {
                    s.quantise = None;
                    set(s)
                }
                Some(k) => {
                    let strength = s.quantise.map_or(100, |q| q.strength);
                    s.quantise =
                        Some(Quantise::new(QUANTISE_GRIDS[k - 1], strength).ok_or("quantise")?);
                    set(s)
                }
                None => end(
                    "Quantise is off already".into(),
                    "Quantise is at its finest, 1/32".into(),
                ),
            }
        }
        4 => {
            let q = s.quantise.ok_or("Turn Quantise on first")?;
            let next = if d > 0 {
                (q.strength / 10 + 1) * 10
            } else {
                (q.strength.div_ceil(10).saturating_sub(1)) * 10
            };
            if !(10..=100).contains(&next) {
                return end(
                    "Strength is at its least, 10 %".into(),
                    "Strength is full already, 100 %".into(),
                );
            }
            s.quantise = Some(Quantise::new(q.grid, next).ok_or("strength")?);
            set(s)
        }
        5 => {
            let now = s.follow.map_or(0, |f| {
                FOLLOWS
                    .iter()
                    .position(|&a| a == f.action)
                    .map_or(0, |k| k + 1)
            });
            match step(now, FOLLOWS.len() + 1) {
                Some(0) => {
                    s.follow = None;
                    set(s)
                }
                Some(k) => {
                    let action = FOLLOWS[k - 1];
                    s.follow = Some(match s.follow {
                        Some(f) => Follow { action, ..f },
                        // After one pass of the clip, to begin with.
                        None => {
                            let bar = st.transport.meter.bar_ticks() as f64;
                            let bars = (c.song_len() / bar).ceil() as i64;
                            Follow::after(bars.clamp(1, i64::from(MAX_FOLLOW_BARS)) as u16, action)
                        }
                    });
                    set(s)
                }
                None => end(
                    "Follow is off already".into(),
                    "Follow is at its last action, Stop".into(),
                ),
            }
        }
        6 => {
            let f = s.follow.ok_or("Turn Follow on first")?;
            match step(usize::from(f.bars) - 1, usize::from(MAX_FOLLOW_BARS)) {
                Some(k) => {
                    s.follow = Some(Follow {
                        bars: k as u16 + 1,
                        ..f
                    });
                    set(s)
                }
                None => end(
                    "Follow is at its soonest, after 1 bar".into(),
                    format!("Follow is at its latest, after {MAX_FOLLOW_BARS} bars"),
                ),
            }
        }
        // The project's, then its own: at once, the note grids, the bars.
        7 => {
            let now = s.launch.map_or(0, |g| {
                LAUNCH_GRIDS
                    .iter()
                    .position(|&x| x == g)
                    .map_or(0, |k| k + 1)
            });
            match step(now, LAUNCH_GRIDS.len() + 1) {
                Some(k) => {
                    s.launch = k.checked_sub(1).map(|k| LAUNCH_GRIDS[k]);
                    set(s)
                }
                None => end(
                    format!(
                        "Launch is the project's already ({})",
                        launch_name(st.launch)
                    ),
                    "Launch is at its longest, 8 bars".into(),
                ),
            }
        }
        // Restart, Legato, Once.
        8 => match step(mode_of(&s), MODES.len()) {
            Some(k) => {
                s.legato = k == 1;
                s.once = k == 2;
                set(s)
            }
            None => end(
                "Mode is at its first, Restart".into(),
                "Mode is at its last, Once: the clip plays one pass, then its track stops".into(),
            ),
        },
        // Off, then 1 to 128.
        9 => {
            let now = s.program.map_or(0, |p| usize::from(p.program) + 1);
            match step(now, 129) {
                Some(0) => {
                    s.program = None;
                    set(s)
                }
                Some(k) => {
                    let bank = s.program.and_then(|p| p.bank);
                    s.program = Some(ClipProgram {
                        program: (k - 1) as u8,
                        bank,
                    });
                    set(s)
                }
                None => end(
                    "Program is off already: the clip sends no program change".into(),
                    "Program is at its last, 128".into(),
                ),
            }
        }
        10 => bank_step(st, delta, 1),
        // Forward, Reverse, Ping-pong, Random (S6). On a clip it can't
        // move, back to Forward, then why not.
        CLIP_DIRECTION if direction_steps(Direction::Reverse, c.length).is_none() => {
            if s.direction == Direction::Forward {
                return Err("Direction moves the clip's 1/16 steps: it needs a clip of 2 whole steps or more, so this one plays forward".into());
            }
            s.direction = Direction::Forward;
            set(s)
        }
        CLIP_DIRECTION => {
            let now = Direction::ALL
                .iter()
                .position(|&x| x == s.direction)
                .unwrap_or(0);
            match step(now, Direction::ALL.len()) {
                Some(k) => {
                    s.direction = Direction::ALL[k];
                    set(s)
                }
                None => end(
                    "Direction is at its first, Forward".into(),
                    "Direction is at its last, Random".into(),
                ),
            }
        }
        // › a new variation (SHIFT: a bolder one, `card_action`), ‹ the
        // original back (S6).
        CLIP_MUTATE if d > 0 && c.notes == 0 && c.variation.is_none() => {
            Err("Mutate varies the clip's notes: this clip has none yet".into())
        }
        CLIP_MUTATE if d > 0 => Ok(TimeAction::Mutate {
            track,
            slot,
            more: false,
            melodic: st.tracks.get(track).is_some_and(|t| t.melodic),
        }),
        CLIP_MUTATE if c.variation.is_some() => Ok(TimeAction::Restore { track, slot }),
        CLIP_MUTATE => {
            Err("The clip plays its original: › mutates it, and ‹ then brings this back".into())
        }
        _ => Err(String::new()),
    }
}

/// A step of the card's Bank row by `by` (1, or 128 with SHIFT: a whole
/// CC 0): off, then 0/0 up to 127/127.
fn bank_step(st: &TimeState, delta: i32, by: i32) -> Result<TimeAction, String> {
    let (track, slot) = st
        .focus()
        .ok_or("No clip to set: arm a track with a clip, or make one")?;
    let c = st.clip(track, slot).ok_or("No clip to set")?;
    if c.looped.is_some() {
        return Err(
            "A loop cell plays its loop as recorded: only Follow, After and Launch apply".into(),
        );
    }
    let mut s = c.settings;
    let p = s
        .program
        .ok_or("Set Program first: the bank goes with its program change")?;
    s.program = Some(ClipProgram {
        bank: bank_next(p, delta, by)?,
        ..p
    });
    Ok(TimeAction::ClipSettings {
        track,
        slot,
        row: 10,
        settings: s,
    })
}

// ---------------------------------------------------------------- texts

/// A clip's length as the grid shows it: "2 bars", "6 beats", "7/16".
pub fn length_label(song_ticks: f64, meter: Meter) -> String {
    let whole = |unit: i64| {
        let n = song_ticks / unit as f64;
        (n >= 1.0 && (n - n.round()).abs() < 1e-6).then_some(n.round() as i64)
    };
    let plural = |n: i64, one: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {one}s")
        }
    };
    if let Some(n) = whole(meter.bar_ticks()) {
        plural(n, "bar")
    } else if let Some(n) = whole(meter.beat_ticks()) {
        plural(n, "beat")
    } else if let Some(n) = whole(i64::from(PPQ / 4)) {
        format!("{n}/16")
    } else {
        format!("{} ticks", song_ticks.round() as i64)
    }
}

/// The loop as the bar shows it: "off", "1" (one bar), "1–4", or positions.
/// The Record tab's Loop row: the loop's bars, or, following another
/// device's position, the loop that comes back after ("1–4, off while
/// following").
fn loop_row(t: &TransportInfo) -> String {
    match (t.follows_position, t.loop_range) {
        (false, range) => loop_label(range, t.meter),
        (true, None) => "off while following".to_string(),
        (true, held) => format!("{}, off while following", loop_label(held, t.meter)),
    }
}

pub fn loop_label(range: Option<(Tick, Tick)>, meter: Meter) -> String {
    let Some((a, b)) = range else {
        return "off".to_string();
    };
    let bar = meter.bar_ticks();
    if a % bar == 0 && b % bar == 0 {
        let (first, last) = (a / bar + 1, b / bar);
        if first == last {
            format!("{first}")
        } else {
            format!("{first}–{last}")
        }
    } else {
        format!("{}–{}", meter.position(a), meter.position(b))
    }
}

fn bar_beat(tick: Tick, meter: Meter) -> String {
    let p = meter.position(tick);
    format!("{}.{}", p.bar, p.beat)
}

/// The pending capture as Capture's button shows it: "1 bar", "4 bars".
pub fn bars_label(bars: i64) -> String {
    if bars == 1 {
        "1 bar".to_string()
    } else {
        format!("{bars} bars")
    }
}

/// "row 5", or "rows 5 to 7", counted from 1.
pub fn rows_named(first: usize, last: usize) -> String {
    if first == last {
        format!("row {}", first + 1)
    } else {
        format!("rows {} to {}", first + 1, last + 1)
    }
}

pub fn take_title(st: &TimeState, row: &TakeRow) -> String {
    let mut word = row.word().to_string();
    word[..1].make_ascii_uppercase();
    match row.song {
        Some(
            SongChange::Moved { track, .. }
            | SongChange::Copied { track, .. }
            | SongChange::Resized { track, .. }
            | SongChange::Deleted { track, .. }
            | SongChange::Unique { track, .. },
        ) => return format!("{word} {} · {}", row.shown(), st.track_name(track)),
        Some(SongChange::SectionMoved { at } | SongChange::SectionCopied { at }) => {
            let name = st
                .markers
                .iter()
                .find(|m| m.0 == at)
                .map_or("section", |m| m.1.as_str());
            return format!("{word} {} · {name}", row.shown());
        }
        Some(SongChange::Tempo { at }) => {
            let bar = st.transport.meter.bar_ticks().max(1);
            return format!("{word} {} · bar {}", row.shown(), at.div_euclid(bar) + 1);
        }
        Some(SongChange::Scenes { .. } | SongChange::List | SongChange::ListToSong { .. })
        | None => {}
    }
    if row.rows.is_some() {
        return format!("{word} {}", row.shown());
    }
    if let Some(r) = row.scene {
        return format!("{word} {} · row {}", row.shown(), r + 1);
    }
    match (row.at, row.dropped) {
        (Some((track, slot)), _) => format!(
            "{word} {} · {} / {}",
            row.shown(),
            st.track_name(track),
            slot + 1
        ),
        (None, Some(ClipDrop::RowMoved { from, .. } | ClipDrop::RowCopied { from, .. })) => {
            format!("{word} {} · row {}", row.shown(), from + 1)
        }
        (None, _) => format!("{word} {}", row.shown()),
    }
}

fn take_detail(st: &TimeState, row: &TakeRow) -> String {
    if let Some((first, last)) = row.rows {
        return rows_named(first, last);
    }
    let cell = |(t, s): (usize, usize)| format!("{} / {}", st.track_name(t), s + 1);
    let bar = st.transport.meter.bar_ticks().max(1);
    let bar_of = |t: Tick| t.div_euclid(bar) + 1;
    match row.song {
        Some(
            SongChange::Moved { start, .. }
            | SongChange::Copied { start, .. }
            | SongChange::SectionMoved { at: start }
            | SongChange::SectionCopied { at: start },
        ) => return format!("to bar {}", bar_of(start)),
        Some(SongChange::Deleted { start, .. } | SongChange::Unique { start, .. }) => {
            return format!("at bar {}", bar_of(start))
        }
        Some(SongChange::Resized { length, .. }) if length % bar == 0 => {
            return format!("{} bars", length / bar)
        }
        Some(SongChange::Resized { length, .. }) => {
            return format!("{:.1} bars", length as f64 / bar as f64)
        }
        Some(SongChange::Scenes { rows }) => return format!("{rows} rows"),
        Some(SongChange::List) => return "song list".into(),
        Some(SongChange::ListToSong { steps }) => return format!("{steps} steps → song"),
        Some(SongChange::Tempo { .. }) => return "tempo point".into(),
        None => {}
    }
    match row.dropped {
        Some(ClipDrop::Moved(_, b) | ClipDrop::Copied(_, b)) => return format!("to {}", cell(b)),
        Some(ClipDrop::Swapped(_, b)) => return format!("with {}", cell(b)),
        Some(ClipDrop::RowMoved { from, to }) => {
            return format!("to row {}", if to > from { to } else { to + 1 })
        }
        Some(ClipDrop::RowCopied { to, .. }) => return format!("to row {}", to + 1),
        None => {}
    }
    if row.scene.is_some() {
        return "scene settings".into();
    }
    match (row.replaced, row.variant) {
        (Some(Replaced::Reload), _) => return "saved state".into(),
        (Some(Replaced::Mutate), Some(n)) => return format!("variant {n}"),
        (Some(Replaced::Mutate), None) => return "a variation".into(),
        (Some(Replaced::Restore), _) => return "the original".into(),
        (None, _) => {}
    }
    if row.deleted {
        return "clip".into();
    }
    if row.settings {
        return "clip settings".into();
    }
    if row.pasted {
        return "a copy".into();
    }
    let new = if row.made { "new " } else { "" };
    if row.erased > 0 {
        format!("{new}+{} −{}", row.added, row.erased)
    } else {
        format!("{new}+{}", row.added)
    }
}

// ---------------------------------------------------------------- what the widgets show

/// The bar for `st`.
pub fn bar_of(st: &TimeState) -> TransportBar {
    let t = &st.transport;
    let mut bar = TransportBar::new();
    bar.run = match t.state {
        State::Stopped => Run::Stopped,
        State::CountIn => Run::CountIn,
        State::Playing => Run::Playing,
        State::Recording => Run::Recording,
    };
    bar.position = t.meter.position(t.position).to_string();
    let swing = if t.swing.amount == 50 {
        "off".to_string()
    } else {
        format!("{}%", t.swing.amount)
    };
    bar.items = vec![];
    // The mute queue first, so it shows whatever else fits (S4).
    if let Some(w) = st.waiting_reading() {
        bar.items.push(BarItem::new("WAITING", w, Tone::Warn));
    }
    // Perform's NEXT: what comes next, and when (S4).
    if let Some(n) = st.next_reading() {
        bar.items.push(BarItem::new("NEXT", n, Tone::Accent));
    }
    bar.items.extend([
        BarItem::new("BPM", format!("{:.1}", t.bpm), Tone::Normal),
        BarItem::new(
            "",
            format!("{}/{}", t.meter.beats, t.meter.unit),
            Tone::Normal,
        ),
        // When a tapped clip, a stop or a scene lands (S4; the Record
        // tab's Launch row sets it).
        BarItem::new("Launch", launch_name(st.launch), Tone::Normal),
        BarItem::new("Swing", swing, Tone::Normal),
        // Following another device's position, its loop is off: greyed.
        BarItem::new(
            "Loop",
            loop_label(t.loop_range, t.meter),
            if t.playing_loop().is_some() {
                Tone::Accent
            } else {
                Tone::Dim
            },
        ),
    ]);
    if t.state == State::CountIn {
        let bars = if t.count_in == 1 {
            "1 bar".to_string()
        } else {
            format!("{} bars", t.count_in)
        };
        bar.items
            .insert(0, BarItem::new("Count-in", bars, Tone::Warn));
    }
    bar.overdub = t.overdub;
    bar.undo = st.done.last().map(TakeRow::short);
    bar.chips = st.chips.clone();
    bar
}

/// Where a loop sits on the Clips grid, as Play's strip and the Looper
/// tab name it: its Loop track and its row (the first, if it is in more
/// than one).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoopPlace {
    pub track: usize,
    pub slot: usize,
    /// "Loops B".
    pub track_name: String,
    /// The row's scene name, or "row 2".
    pub row: String,
}

impl LoopPlace {
    /// "Loops B · Loop 2 · Verse".
    pub fn full(&self, n: usize) -> String {
        format!("{} · Loop {} · {}", self.track_name, n + 1, self.row)
    }
}

/// Row `slot`'s name: its scene's, or "row 2".
pub fn row_name(st: &TimeState, slot: usize) -> String {
    match st.scenes.get(slot).filter(|s| !s.is_empty()) {
        Some(name) => name.clone(),
        None => format!("row {}", slot + 1),
    }
}

/// The armed Loop cell as the REC chip names it: "Loops C · Drop".
pub fn loop_cell_name(st: &TimeState) -> Option<String> {
    let (t, s) = st.loop_cell?;
    Some(format!("{} · {}", st.tracks.get(t)?.name, row_name(st, s)))
}

/// Each loop's place on the grid; None for a loop in no cell.
pub fn loop_places(st: &TimeState) -> [Option<LoopPlace>; mpfree_seq::LOOPS] {
    let mut out: [Option<LoopPlace>; mpfree_seq::LOOPS] = Default::default();
    for (track, t) in st.tracks.iter().enumerate() {
        for (slot, c) in t.slots.iter().enumerate() {
            let Some(n) = c.as_ref().and_then(|c| c.looped) else {
                continue;
            };
            let Some(place @ None) = out.get_mut(usize::from(n)) else {
                continue;
            };
            *place = Some(LoopPlace {
                track,
                slot,
                track_name: t.name.clone(),
                row: row_name(st, slot),
            });
        }
    }
    out
}

/// Cell `slot` of `track` for `st`.
pub fn cell_of(st: &TimeState, track: usize, slot: usize) -> GridCell {
    let Some(clip) = st.clip(track, slot) else {
        if st.loop_cell == Some((track, slot)) {
            // The armed Loop cell: the next loop recorded lands here.
            // Short: the column is 86 px.
            let (name, detail, foot) = if st.loops.recording {
                ("REC", "REC ends", "the pass")
            } else if st.loops.waiting {
                ("Ready", "bar line", "or PLAY")
            } else {
                ("Armed", "REC here", "")
            };
            return GridCell {
                state: CellState::Armed,
                name: name.to_string(),
                detail: detail.to_string(),
                foot: foot.to_string(),
                ..GridCell::default()
            };
        }
        return GridCell::default();
    };
    let t = &st.transport;
    let len = clip.song_len().max(1.0);
    let facts = clip
        .looped
        .and_then(|n| st.loops.loops.get(usize::from(n)))
        .copied();
    // A loop cell shows its wave in the detail's place, and the detail
    // at the foot.
    let notes = match (facts, clip.notes) {
        (Some(_), _) => String::new(),
        (None, 1) => "1 note".to_string(),
        (None, n) => format!("{n} notes"),
    };
    let mut cell = GridCell {
        state: CellState::Stopped,
        name: clip.name.clone(),
        detail: length_label(len, t.meter),
        foot: notes,
        progress: 0.0,
        wave: facts.map(|f| f.wave),
    };
    if facts.is_some_and(|f| f.copying) {
        cell.foot = "copying in".to_string();
    }
    let cue = st.cue(track);
    if cue.is_some_and(|c| c.slot == Some(slot)) {
        cell.state = CellState::Queued;
        let line = st.cue_line(track);
        let at = cue_tick(t, line);
        st.queued_foot(&mut cell, at, line.line(t.meter.bar_ticks()));
        return cell;
    }
    let Some(l) = st.launch(track).filter(|l| l.slot == slot) else {
        return cell;
    };
    if l.origin > t.position {
        cell.state = CellState::Queued;
        st.queued_foot(&mut cell, l.origin, t.meter.bar_ticks());
        return cell;
    }
    // In steps of 1/64 of a pass, so the ring is not redrawn every frame.
    let frac = ((t.position - l.origin) as f64).rem_euclid(len) / len;
    cell.progress = ((frac * 64.0).floor() / 64.0) as f32;
    // The ring takes the foot's place.
    cell.foot.clear();
    if t.state == State::Recording && st.armed == Some(track) {
        cell.state = CellState::Recording;
        cell.detail = "recording".to_string();
    } else if facts.is_some_and(|f| f.overdubbing) {
        cell.state = CellState::Recording;
        cell.detail = "overdub".to_string();
    } else {
        cell.state = CellState::Playing;
    }
    if let Some(c) = cue {
        // It plays until the cue starts.
        let word = if c.slot.is_none() { "stops" } else { "until" };
        cell.detail = format!(
            "{word} {}",
            bar_beat(cue_tick(t, st.cue_line(track)), t.meter)
        );
    }
    cell
}

/// How long `left` ticks are, as Perform's countdowns say it: whole
/// beats, rounded up, under a bar ("1 beat", "3 beats"), whole bars past
/// one ("2 bars").
pub fn countdown(left: Tick, meter: Meter) -> String {
    let left = left.max(1);
    let beat = meter.beat_ticks().max(1);
    let bar = meter.bar_ticks().max(1);
    let (n, one, many) = if left >= bar {
        ((left + bar - 1) / bar, "bar", "bars")
    } else {
        ((left + beat - 1) / beat, "beat", "beats")
    };
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// The song list's title line: "Song list · 7 steps · 76 bars · 2:27"
/// (the time at the tempo now), or "Song list" while it is empty.
pub fn list_title(st: &TimeState) -> String {
    if st.list.is_empty() {
        return "Song list".to_string();
    }
    let n = st.list.len();
    let bars: i64 = st.list.iter().map(|r| r.bars).sum();
    let t = &st.transport;
    let quarters = (bars * t.meter.bar_ticks()) as f64 / PPQ as f64;
    let secs = (quarters * 60.0 / t.bpm.max(1.0)).round() as i64;
    format!(
        "Song list · {n} {} · {bars} {} · {}:{:02}",
        if n == 1 { "step" } else { "steps" },
        if bars == 1 { "bar" } else { "bars" },
        secs / 60,
        secs % 60
    )
}

/// The scene playing's follow action, for Perform's NEXT (S4): on song
/// tick `at` it plays `follow` from row `row`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowNext {
    pub at: Tick,
    pub row: usize,
    pub follow: Follow,
}

impl TimeState {
    /// The reload of clip `at` (track, slot) waits for its line.
    pub fn reload_waiting(&self, at: (usize, usize)) -> bool {
        self.reloads.iter().any(|r| (r.0, r.1) == at)
    }

    /// Clips play over the song: song mode is on and a clip is launched,
    /// or waits to be (the grid's "Back to song", S4).
    pub fn over_song(&self) -> bool {
        self.song_on && (!self.launches.is_empty() || self.cues.iter().any(|c| c.slot.is_some()))
    }

    /// A queued cell's foot: where it starts ("from 7.1"); in Perform also
    /// its countdown, filling over the `span` ticks before the line (its
    /// launch grid's).
    fn queued_foot(&self, cell: &mut GridCell, at: Tick, span: Tick) {
        let t = &self.transport;
        cell.foot = format!("from {}", bar_beat(at, t.meter));
        if self.perform && t.state.rolling() {
            let left = (at - t.position).max(0);
            let span = span.max(1);
            let frac = 1.0 - (left.min(span) as f64 / span as f64);
            // In steps of 1/64, as a playing clip's ring; never empty, so
            // the countdown shows from the tap.
            cell.progress = ((frac * 64.0).floor().max(1.0) / 64.0) as f32;
        }
    }

    /// What Perform's NEXT says comes next, and when: a scene launched
    /// ("Drop in 2 beats"), else the clips cued ("Drop bass in 1 bar",
    /// "Bass stops in 3 beats", "Drop bass +2 in 1 bar"), else the
    /// scene playing's follow action ("Verse in 3 bars"), else in song
    /// mode the next marker ("Chorus in 4 bars"). None with nothing
    /// coming, in Make, or stopped.
    pub fn next_reading(&self) -> Option<String> {
        let t = &self.transport;
        if !self.perform || !t.state.rolling() {
            return None;
        }
        let cue_at = |c: &Cue| cue_tick(t, self.cue_line(c.track));
        let soonest = self.cues.iter().map(cue_at).min();
        let what = if let Some(row) = self.scene_cued {
            let at = soonest.unwrap_or_else(|| cue_tick(t, self.launch));
            Some((row_name(self, row), at))
        } else if let Some(at) = soonest {
            let names: Vec<String> = self
                .cues
                .iter()
                .filter(|c| cue_at(c) == at)
                .map(|c| match c.slot.and_then(|s| self.clip(c.track, s)) {
                    Some(clip) => clip.name.clone(),
                    None => format!("{} stops", self.track_name(c.track)),
                })
                .collect();
            let shown = match names.len() {
                0 | 1 => names.join(""),
                n => format!("{} +{}", names[0], n - 1),
            };
            Some((shown, at))
        } else if let Some(&(track, slot, at)) = self
            .reloads
            .iter()
            .filter(|r| r.2 > t.position)
            .min_by_key(|r| r.2)
        {
            let name = self
                .clip(track, slot)
                .map_or_else(|| self.track_name(track), |c| c.name.clone());
            Some((format!("{name} reloads"), at))
        } else if let Some((step, at)) = self.list_next.filter(|n| n.1 > t.position) {
            // The song list's next step, or its end (a list that goes
            // round names its first step).
            Some((
                match step.and_then(|k| self.list.get(k)) {
                    Some(r) => r.name.clone(),
                    None => "Stop all".to_string(),
                },
                at,
            ))
        } else if let Some(f) = self.follow_next.filter(|f| f.at > t.position) {
            self.follow_words(f).map(|w| (w, f.at))
        } else if self.song_on {
            self.markers
                .iter()
                .filter(|(at, _)| *at > t.position)
                .min_by_key(|(at, _)| *at)
                .map(|(at, name)| (name.clone(), *at))
        } else {
            None
        };
        let (name, at) = what?;
        Some(format!("{name} in {}", countdown(at - t.position, t.meter)))
    }

    /// What a scene's follow action plays, as NEXT names it: the row
    /// ("Drop"), "Verse again", "Stop all", "Any scene", "Another scene";
    /// with a chance, both ("Drop or stay"). None when it stays.
    fn follow_words(&self, f: FollowNext) -> Option<String> {
        let filled: Vec<bool> = (0..SLOTS)
            .map(|r| {
                self.tracks
                    .iter()
                    .any(|tr| tr.slots.get(r).is_some_and(Option::is_some))
            })
            .collect();
        let words = |a: FollowAction| -> Option<String> {
            Some(match a {
                FollowAction::Stay => return None,
                FollowAction::Again => format!("{} again", row_name(self, f.row)),
                FollowAction::Stop => "Stop all".to_string(),
                FollowAction::Any => "Any scene".to_string(),
                FollowAction::Other => "Another scene".to_string(),
                _ => row_name(
                    self,
                    mpfree_seq::scenes::follow_target(a, f.row, &filled, 0)?,
                ),
            })
        };
        let fw = f.follow;
        if fw.chance >= 100 || fw.action == fw.other {
            words(fw.action)
        } else if fw.chance == 0 {
            words(fw.other)
        } else {
            let one = |a| words(a).unwrap_or_else(|| "stay".to_string());
            Some(format!(
                "{} or {}",
                one(fw.action),
                one(fw.other).to_lowercase()
            ))
        }
    }
}

/// Mute and solo changes waiting for one launch line (sequencer S4's mute
/// queue): they land together on the line they had when the first one
/// joined.
#[derive(Clone, Debug, PartialEq)]
pub struct MuteWait {
    pub line: LaunchGrid,
    /// Each track that changes: its mute and solo once the queue lands.
    pub want: Vec<(usize, bool, bool)>,
    /// Each pad of the kit that changes (SHIFT + TRACK MUTE): its mute
    /// and solo once the queue lands.
    pub pads: Vec<(usize, bool, bool)>,
}

impl MuteWait {
    /// A queue for line `line` with nothing in it yet.
    pub fn new(line: LaunchGrid) -> MuteWait {
        MuteWait {
            line,
            want: Vec::new(),
            pads: Vec::new(),
        }
    }

    /// Nothing waits in it: no track and no pad.
    pub fn is_empty(&self) -> bool {
        self.want.is_empty() && self.pads.is_empty()
    }
}

impl TimeState {
    /// What waits on track `t`'s header, as the grid shows it: "mutes in
    /// 2 beats", "plays in 1 bar", "solo in 3 beats", "solo off in 1
    /// beat". None with nothing waiting there.
    pub fn mute_waiting(&self, t: usize) -> Option<String> {
        let q = self.mute_wait.as_ref()?;
        let &(_, mute, solo) = q.want.iter().find(|w| w.0 == t)?;
        let tr = self.tracks.get(t)?;
        let what = if solo != tr.solo {
            if solo {
                "solo"
            } else {
                "solo off"
            }
        } else if mute {
            "mutes"
        } else {
            "plays"
        };
        let tp = &self.transport;
        let left = cue_tick(tp, q.line) - tp.position;
        Some(format!("{what} in {}", countdown(left, tp.meter)))
    }

    /// The transport bar's WAITING reading: the tracks and the kit's pads
    /// waiting and where they land ("Hats, Vox at 18.1", "Pad 3 at 9.1").
    /// None with nothing waiting.
    pub fn waiting_reading(&self) -> Option<String> {
        let q = self.mute_wait.as_ref().filter(|q| !q.is_empty())?;
        let tracks = q.want.iter().map(|w| self.track_name(w.0));
        let pads = q.pads.iter().map(|w| format!("Pad {}", w.0 + 1));
        let names: Vec<String> = tracks.chain(pads).collect();
        let shown = if names.len() > 2 {
            format!("{}, {} +{}", names[0], names[1], names.len() - 2)
        } else {
            names.join(", ")
        };
        let at = cue_tick(&self.transport, q.line);
        Some(format!("{shown} at {}", bar_beat(at, self.transport.meter)))
    }
}

/// The grid's tracks for `st`.
pub fn grid_tracks(st: &TimeState) -> Vec<GridTrack> {
    st.tracks
        .iter()
        .enumerate()
        .map(|(i, tr)| GridTrack {
            name: tr.name.clone(),
            sound: tr.sound.clone(),
            color: track_color(i),
            armed: st.armed == Some(i),
            mute: tr.silent,
            solo: tr.solo,
            waiting: st.mute_waiting(i),
            loops: tr.loops.is_some(),
            cells: (0..tr.slots.len()).map(|s| cell_of(st, i, s)).collect(),
        })
        .collect()
}

/// The record settings' rows.
pub fn record_rows(st: &TimeState) -> Vec<OptionRow> {
    let t = &st.transport;
    let row = |name: &str, value: String| OptionRow {
        name: name.to_string(),
        value,
    };
    let armed = st
        .armed
        .map_or_else(|| "none".to_string(), |a| st.track_name(a));
    let count_in = match t.count_in {
        0 => "off".to_string(),
        1 => "1 bar".to_string(),
        n => format!("{n} bars"),
    };
    vec![
        row(RECORD_ROWS[0], armed),
        row(
            RECORD_ROWS[1],
            if t.overdub { "Overdub" } else { "Replace" }.to_string(),
        ),
        row(RECORD_ROWS[2], count_in),
        row(RECORD_ROWS[3], loop_row(t)),
        row(RECORD_ROWS[4], format!("{:.1}", t.bpm)),
        row(
            RECORD_ROWS[5],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                None => "–".to_string(),
                Some(tr) if tr.pressure => "Recorded".to_string(),
                Some(_) => "Off".to_string(),
            },
        ),
        row(
            RECORD_ROWS[6],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                None => "–".to_string(),
                Some(tr) => tr
                    .record_quantise
                    .map_or_else(|| "Off".to_string(), grid_name),
            },
        ),
        row(RECORD_ROWS[7], launch_name(st.launch)),
        row(
            RECORD_ROWS[TRANSPOSE_ROW],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                Some(tr) if tr.melodic => transpose_text(tr.transpose),
                _ => "–".to_string(),
            },
        ),
        row(
            RECORD_ROWS[FOLLOWS_ROW],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                Some(tr) if tr.melodic => if tr.follows { "On" } else { "Off" }.to_string(),
                _ => "–".to_string(),
            },
        ),
    ]
}

/// A track's live transpose as the screens say it: "as written", "+5 st",
/// "-2 st", "+3 in key" (as 16 Levels' Tune says "-8 st").
pub fn transpose_text(t: Transpose) -> String {
    match (t.by, t.in_key) {
        (0, _) => "as written".to_string(),
        (n, false) => format!("{n:+} st"),
        (n, true) => format!("{n:+} in key"),
    }
}

/// The Record tab's Transpose (– / + a semitone, or a note of the key when
/// the track moves in key, within two octaves) and Follows (either way
/// flips it) for the armed track; why not when it can't.
fn transpose_step(st: &TimeState, index: usize, delta: i32) -> Result<TimeAction, String> {
    let Some(a) = st.armed else {
        return Err("Arm a track first: transpose moves the armed track".to_string());
    };
    let tr = st.tracks.get(a).ok_or_else(String::new)?;
    if !tr.melodic {
        return Err(format!(
            "{} plays a kit, whose notes are its pads: transpose moves a track that plays notes",
            tr.name
        ));
    }
    if index == FOLLOWS_ROW {
        return Ok(TimeAction::FollowsTranspose {
            track: a,
            on: !tr.follows,
        });
    }
    let now = tr.transpose;
    let by = (i32::from(now.by) + delta.signum())
        .clamp(-i32::from(MAX_TRANSPOSE), i32::from(MAX_TRANSPOSE)) as i8;
    if by == now.by {
        let unit = if now.in_key {
            "notes of the key"
        } else {
            "semitones"
        };
        return Err(format!(
            "Transpose is at its {}, {MAX_TRANSPOSE} {unit} {}",
            if delta < 0 { "lowest" } else { "highest" },
            if delta < 0 { "down" } else { "up" }
        ));
    }
    Ok(TimeAction::Transpose {
        track: a,
        to: Transpose { by, ..now },
    })
}

/// A launch quantise as the screens say it: "at once", "1/16", "1 bar",
/// "4 bars".
pub fn launch_name(g: LaunchGrid) -> String {
    match g {
        LaunchGrid::Now => "at once".to_string(),
        LaunchGrid::Ticks(t) => grid_name(t),
        LaunchGrid::Bars(1) => "1 bar".to_string(),
        LaunchGrid::Bars(n) => format!("{n} bars"),
        LaunchGrid::At(_) => "its end".to_string(),
    }
}

/// The takes newest first: the undone ones (dimmed), then the ones Undo
/// can take back; and which row Undo takes back next.
pub fn take_items(st: &TimeState) -> (Vec<TakeItem>, Option<usize>) {
    let item = |r: &TakeRow, undone: bool| TakeItem {
        title: take_title(st, r),
        detail: take_detail(st, r),
        color: r.at.map(|(track, _)| track_color(track)),
        undone,
    };
    let mut items: Vec<TakeItem> = st.undone.iter().map(|r| item(r, true)).collect();
    items.extend(st.done.iter().rev().map(|r| item(r, false)));
    let current = (!st.done.is_empty()).then_some(st.undone.len());
    (items, current)
}

/// "0:42": seconds, to the nearest, as minutes and seconds.
pub fn min_sec(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// The Song view for `st`, from bar `first_bar` (0-based): 16 bars. The
/// waveforms are left out: [`TimePage::update`] copies `st.waves` only
/// when they change, as they can be large.
pub fn song_lanes(st: &TimeState, first_bar: i64) -> SongLanes {
    song_lanes_with(st, first_bar, None)
}

/// [`song_lanes`], with `blocks` for the blocks when they were made from
/// `st.placements` ([`SongBlocks::made_from`]), so an update that changes
/// no block copies none (Arrange refactor 2.4).
fn song_lanes_with(st: &TimeState, first_bar: i64, blocks: Option<SongBlocks>) -> SongLanes {
    let t = &st.transport;
    let bar = t.meter.bar_ticks();
    let blocks = blocks.unwrap_or_else(|| SongBlocks::of(&st.placements));
    let placed = std::iter::once(blocks.end());
    let audio = st
        .audio
        .iter()
        .flat_map(|l| l.blocks.iter().map(|b| b.end).chain(l.recording));
    let end = placed.chain(audio).max().unwrap_or(0);
    let bars = (end + bar - 1) / bar.max(1);
    let mut legend = if bars == 0 {
        "empty: Scenes → song makes one from the rows".to_string()
    } else {
        format!("{bars} bars · from bar {}", first_bar + 1)
    };
    if let Some((free, all)) = st.audio_free {
        legend += &format!(" · Audio {} of {} free", min_sec(free), min_sec(all));
    }
    for (what, n) in st.audio_counts.iter().filter(|c| c.1 > 0) {
        legend += &format!(" · {n} {what}");
    }
    let alert = st.audio_counts.iter().any(|c| c.1 > 0);
    SongLanes {
        title: "Song".to_string(),
        legend,
        alert,
        tracks: st
            .tracks
            .iter()
            .enumerate()
            .map(|(i, tr)| (tr.name.clone(), track_color(i)))
            .collect(),
        sounds: st.tracks.iter().map(|tr| tr.sound.clone()).collect(),
        blocks,
        markers: st.markers.clone(),
        bar,
        first_bar,
        bars: 16,
        playhead: Some(t.position),
        selected: st.selected,
        section: st.section,
        on: st.song_on,
        rec: st.song_rec,
        audio: st.audio.clone(),
        region: st.region.map(|r| (r.track, r.index)),
        panel: st.audio_panel,
        takes_open: st.audio_takes,
        can_add_track: st.tracks.len() < MAX_TRACKS,
        tempo: st.tempo_points.clone(),
        tempo_base: st.tempo_base,
        tempo_pick: st.tempo_pick.filter(|&k| k < st.tempo_points.len()),
        ..SongLanes::default()
    }
}

/// Where the note editor's clip plays now, in its own ticks, if it is
/// launched and the transport has started.
pub fn note_playhead(e: &EditInfo, st: &TimeState) -> Option<Tick> {
    let ed = &e.editor;
    let c = &e.clip;
    st.launches
        .iter()
        .find(|l| (l.track, l.slot) == (ed.track, ed.slot))
        .filter(|_| c.length > 0 && st.transport.position >= 0)
        .map(|l| {
            let at = c.speed.to_clip((st.transport.position - l.origin) as f64);
            (at.floor() as Tick).rem_euclid(c.length)
        })
}

/// The screen built the note editor from `a` and is asked to show `b`:
/// nothing but the playhead can have changed. The same clip (one shared
/// copy, not compared note by note), revision and editor.
fn same_edit(a: &EditInfo, b: &EditInfo) -> bool {
    Arc::ptr_eq(&a.clip, &b.clip)
        && a.rev == b.rev
        && a.editor == b.editor
        && a.bar == b.bar
        && a.track_name == b.track_name
}

/// The note editor's roll or list for `e`, with the playhead where the
/// clip plays now (if it is launched).
pub fn note_grid(e: &EditInfo, st: &TimeState) -> NoteGrid {
    let ed = &e.editor;
    let c = &e.clip;
    let playhead = note_playhead(e, st);
    let selected = |i: usize| ed.sel.contains(&i);
    let n_sel = ed.sel.iter().filter(|&&i| i < c.notes.len()).count();
    let mut g = NoteGrid::new();
    g.title = format!("Edit · {} · {}", c.name, e.track_name);
    let step = ed.grid().size();
    let lens = match ed.lens {
        Lens::Grid => "grid".to_string(),
        Lens::List => "list".to_string(),
        // The snap off or too fine: the steps are 1/16s, and it says so.
        Lens::Tracker if step == STEP_FALLBACK && ed.snap_ticks() != step => {
            format!(
                "tracker · {} steps (snap {})",
                step_name(step),
                ed.snap_name()
            )
        }
        Lens::Tracker => format!("tracker · {} steps", step_name(step)),
    };
    g.legend = if ed.cleared {
        // Something else changed the clip and took a picked note with it.
        format!(
            "{} notes · the clip changed: picks cleared · {lens}",
            c.notes.len()
        )
    } else {
        format!("{} notes · {} picked · {lens}", c.notes.len(), n_sel)
    };
    // Each note with its pressure: only the runs the notes name (the
    // clip keeps the runs of notes taken out too, for undo).
    g.notes = c
        .notes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let mut b = NoteBox::of(n, selected(i));
            let run = c.touch_of(n);
            if !run.is_empty() {
                b.touch = mpfree_seq::Touch {
                    first: g.touch.len() as u32,
                    len: run.len() as u32,
                };
                g.touch.extend_from_slice(run);
            }
            b
        })
        .collect();
    g.step = step;
    g.locks = c.locks.iter().map(LockMark::of).collect();
    g.length = c.length;
    g.bar = e.bar;
    g.low = ed.low;
    g.pads = pads(c);
    g.playhead = playhead;
    g.snap = ed.snap_ticks();
    g.snap_name = ed.snap_name().to_string();
    g.tool = ed.tool;
    g.lens = ed.lens;
    g.first = ed.first;
    g.color = track_color(ed.track);
    g
}

// ---------------------------------------------------------------- the page

/// The Clips page and the transport bar in the app's [`Ui`]: the app adds
/// them once, calls [`TimePage::update`] when the state changes, and turns
/// touches into actions with [`action`].
#[derive(Clone, Debug, Default)]
pub struct TimePage {
    /// The first slot shown (the wheel scrolls it).
    pub first_row: usize,
    /// The first track shown: a multiple of [`GRID_COLS`], paged by the
    /// pager and following the armed track.
    pub first_track: usize,
    /// The first bar the Song view shows (0-based).
    pub first_bar: i64,
    /// The armed track last shown, so a newly armed track off the page
    /// brings its page.
    armed_seen: Option<usize>,
    /// The grid's rows in view, as of the last update.
    rows_seen: usize,
    /// The Record panel shows its Clip tab: the clip settings card.
    pub clip_card: bool,
    /// The Record panel shows its Scene tab: the scene editor.
    pub scene_card: bool,
    /// The note editor shows on another page (the app's Sequencer tab):
    /// while it is open the Clips page keeps the grid, the song or the
    /// list, and the editor's layout comes back once this is false again.
    pub edit_hidden: bool,
    /// What the note editor's roll and options were last built from: an
    /// update with the same clip and editor moves only the playhead
    /// (piano roll plan 0C.1), so the playing transport costs the screen
    /// no rebuild of 8,192 notes a frame.
    edit_built: Option<Box<EditInfo>>,
}

impl TimePage {
    /// Add the bar, the grid, the record settings, the takes and the Clips
    /// page. The app's `header` and `hint` must be in `ui` already. The
    /// Q-Link column is the app's: an empty one is added only when `ui`
    /// has none yet (the samples, the tests), so the app's is never
    /// replaced.
    pub fn add_to(ui: &mut Ui) -> Result<TimePage, String> {
        if ui.get::<QLinkColumn>(QLINKS).is_none() {
            ui.add(QLINKS, QLinkColumn::new());
        }
        ui.add(TRANSPORT, TransportBar::new());
        ui.add(CLIPS, ClipGrid::new("Clips", SLOTS));
        let mut record = OptionList::new("Record");
        record.tabs = vec![
            "Record".to_string(),
            "Clip".to_string(),
            "Scene".to_string(),
        ];
        record.tab_widths = PANEL_TAB_WIDTHS.to_vec();
        let mut card = OptionList::new("Clip");
        card.tabs = record.tabs.clone();
        card.tab_widths = record.tab_widths.clone();
        card.tab = 1;
        card.right = "wheel or arrows".to_string();
        card.empty = "Arm a track with a clip".to_string();
        let mut scene = OptionList::new("Scene");
        scene.tabs = record.tabs.clone();
        scene.tab_widths = record.tab_widths.clone();
        scene.tab = 2;
        scene.right = "wheel or arrows".to_string();
        ui.add(RECORD, record);
        ui.add(CLIP_CARD, card);
        ui.add(SCENE_CARD, scene);
        ui.add(TAKES, TakeList::new("Takes"));
        ui.add(MORPH, MorphBar::new());
        ui.add(SONG, SongLanes::new());
        ui.add(SONG_LIST, SongListView::new());
        ui.add(NOTES, NoteGrid::new());
        ui.add(EDIT_OPTIONS, OptionList::new("Edit"));
        ui.add_page(CLIPS_PAGE, CLIPS_LAYOUT)?;
        Ok(TimePage::default())
    }

    /// Rows of the grid shown at once (0 if the grid is not on screen).
    pub fn visible_rows(&self, ui: &Ui) -> usize {
        match (ui.rect_of(CLIPS), ui.get::<ClipGrid>(CLIPS)) {
            (Some(r), Some(g)) => g.visible_rows(r),
            _ => 0,
        }
    }

    /// A touch the page handles itself: the grid's pager (‹ and ›, or a
    /// swipe along the track names) shows the 8 tracks before or after; a
    /// dragged clip, row, block or section held at an edge scrolls.
    /// True when `ev` was one; call [`TimePage::update`] after.
    pub fn page_tracks(&mut self, ev: &UiEvent, st: &TimeState) -> bool {
        let step = match (ev.widget.as_str(), &ev.kind) {
            (SONG, EventKind::Tab(k)) if *k == BARS_BACK || *k == BARS_ON => {
                // A dragged block or section held at the left or right
                // edge.
                let d = if *k == BARS_ON { 1 } else { -1 };
                self.first_bar = (self.first_bar + d).max(0);
                return true;
            }
            (RECORD | CLIP_CARD | SCENE_CARD, EventKind::Tab(k)) => {
                // The Record panel's tabs: the record settings, the clip
                // settings card or the scene editor.
                self.clip_card = *k == 1;
                self.scene_card = *k == 2;
                return true;
            }
            (CLIPS, EventKind::Toggle(k)) if *k == SLOTS + PAGE_BACK => -1,
            (CLIPS, EventKind::Toggle(k)) if *k == SLOTS + PAGE_ON => 1,
            (CLIPS, EventKind::Toggle(k)) if *k == SLOTS + ROWS_BACK || *k == SLOTS + ROWS_ON => {
                // A dragged clip or row held at the top or bottom edge.
                let rows = self.rows_seen.max(1);
                let last = SLOTS.saturating_sub(rows) as i64;
                let d = if *k == SLOTS + ROWS_ON { 1 } else { -1 };
                self.first_row = (self.first_row as i64 + d).clamp(0, last) as usize;
                return true;
            }
            _ => return false,
        };
        let last = st.tracks.len().saturating_sub(1) / GRID_COLS * GRID_COLS;
        self.first_track =
            (self.first_track as i64 + step * GRID_COLS as i64).clamp(0, last as i64) as usize;
        true
    }

    /// Bring the grid's cell `slot` of `track` into view: its page of
    /// tracks, and its row if it is off the rows shown (Show in Clips on
    /// the Looper tab). Call [`TimePage::update`] after.
    pub fn show_cell(&mut self, track: usize, slot: usize) {
        self.first_track = track / GRID_COLS * GRID_COLS;
        let rows = self.rows_seen.max(1);
        if slot < self.first_row || slot >= self.first_row + rows {
            self.first_row = slot.min(SLOTS.saturating_sub(rows));
        }
    }

    /// Scroll the grid `delta` rows down (up if negative), or on the Song
    /// view the timeline `delta` bars on. In the note editor the editor
    /// scrolls: the action for the app.
    pub fn scroll(&mut self, ui: &Ui, delta: i32) -> Option<TimeAction> {
        if ui.layout_of(CLIPS_PAGE) == Some(EDIT_LAYOUT) {
            return Some(TimeAction::Edit(EditAction::Scroll(delta)));
        }
        if matches!(
            ui.layout_of(CLIPS_PAGE),
            Some(SONG_LAYOUT | SONG_CARD_LAYOUT)
        ) {
            self.first_bar = (self.first_bar + i64::from(delta)).max(0);
            return None;
        }
        if matches!(
            ui.layout_of(CLIPS_PAGE),
            Some(LIST_LAYOUT | LIST_CARD_LAYOUT)
        ) {
            // The list scrolls with a drag.
            return None;
        }
        let rows = self.visible_rows(ui).max(1);
        let last = SLOTS.saturating_sub(rows) as i64;
        self.first_row = (self.first_row as i64 + i64::from(delta)).clamp(0, last) as usize;
        None
    }

    /// The options list the wheel steps: the Edit options in the note
    /// editor, the clip settings card on the Record panel's Clip tab, else
    /// the record settings.
    pub fn wheel_list(&self, ui: &Ui) -> &'static str {
        match ui.layout_of(CLIPS_PAGE) {
            Some(EDIT_LAYOUT) => EDIT_OPTIONS,
            Some(CLIPS_CARD_LAYOUT | SONG_CARD_LAYOUT | LIST_CARD_LAYOUT) => CLIP_CARD,
            Some(CLIPS_SCENE_LAYOUT | SONG_SCENE_LAYOUT | LIST_SCENE_LAYOUT) => SCENE_CARD,
            _ => RECORD,
        }
    }

    /// Show `st`. Widgets whose contents did not change are left alone, so
    /// they are not redrawn. A track newly armed off the grid's page
    /// brings its page, and the page stays within the tracks there are.
    pub fn update(&mut self, ui: &mut Ui, st: &TimeState) {
        if st.armed != self.armed_seen {
            self.armed_seen = st.armed;
            if let Some(a) = st.armed {
                if !(self.first_track..self.first_track + GRID_COLS).contains(&a) {
                    self.first_track = a / GRID_COLS * GRID_COLS;
                }
            }
        }
        let last = st.tracks.len().saturating_sub(1) / GRID_COLS * GRID_COLS;
        self.first_track = self.first_track.min(last);
        let bar = bar_of(st);
        let same_bar = ui.get::<TransportBar>(TRANSPORT).is_some_and(|b| {
            b.run == bar.run
                && b.position == bar.position
                && b.items == bar.items
                && b.overdub == bar.overdub
                && b.undo == bar.undo
                && b.chips == bar.chips
        });
        if !same_bar {
            if let Some(b) = ui.get_mut::<TransportBar>(TRANSPORT) {
                b.run = bar.run;
                b.position = bar.position;
                b.items = bar.items;
                b.overdub = bar.overdub;
                b.undo = bar.undo;
                b.chips = bar.chips;
            }
        }
        let tracks = grid_tracks(st);
        // Perform's big rows, Make's compact ones (S4): the rows in view
        // follow, kept within the slots.
        // (A look first: `get_mut` marks the grid for a paint.)
        if ui
            .get::<ClipGrid>(CLIPS)
            .is_some_and(|g| g.big != st.perform)
        {
            if let Some(g) = ui.get_mut::<ClipGrid>(CLIPS) {
                g.big = st.perform;
            }
        }
        let rows = self.visible_rows(ui).max(1);
        self.first_row = self.first_row.min(SLOTS.saturating_sub(rows));
        self.rows_seen = rows;
        // With Loop tracks, the title is the loops' gauge: how many of the
        // looper's loops hold one, and the share of its memory in use (the
        // row numbers say which rows show). Past 8 tracks the pager takes
        // the legend's place, so the gauge is not there.
        let title = if st.tracks.iter().any(|t| t.loops.is_some()) {
            let l = &st.loops;
            format!(
                "Loops {} of {} · memory {} %",
                l.in_use,
                mpfree_seq::LOOPS,
                l.memory_pct
            )
        } else {
            format!(
                "Clips · slots {}–{} of {SLOTS}",
                self.first_row + 1,
                (self.first_row + rows).min(SLOTS)
            )
        };
        let legend = "ring: one pass   ·   amber: waiting   ·   red: recording".to_string();
        let can_add = st.tracks.len() < MAX_TRACKS;
        let same_grid = ui.get::<ClipGrid>(CLIPS).is_some_and(|g| {
            g.tracks == tracks
                && g.title == title
                && g.legend == legend
                && g.first_row == self.first_row
                && g.first_track == self.first_track
                && (g.scene_playing, g.scene_cued) == (st.scene_playing, st.scene_cued)
                && g.song_rec == st.song_rec
                && g.can_add == can_add
                && g.back_to_song == st.over_song()
        });
        if !same_grid {
            if let Some(g) = ui.get_mut::<ClipGrid>(CLIPS) {
                g.tracks = tracks;
                g.title = title;
                g.legend = legend;
                g.first_row = self.first_row;
                g.first_track = self.first_track;
                g.scene_playing = st.scene_playing;
                g.scene_cued = st.scene_cued;
                g.song_rec = st.song_rec;
                g.can_add = can_add;
                g.back_to_song = st.over_song();
            }
        }
        // The grid, the song or the note editor, as the app has it.
        let editing = st.edit.is_some() && !self.edit_hidden;
        let want = match (editing, st.song_view, self.clip_card) {
            (true, ..) => EDIT_LAYOUT,
            _ if st.list_view && self.scene_card => LIST_SCENE_LAYOUT,
            _ if st.list_view && self.clip_card => LIST_CARD_LAYOUT,
            _ if st.list_view => LIST_LAYOUT,
            (false, true, _) if self.scene_card => SONG_SCENE_LAYOUT,
            (false, false, _) if self.scene_card => CLIPS_SCENE_LAYOUT,
            (false, true, false) => SONG_LAYOUT,
            (false, true, true) => SONG_CARD_LAYOUT,
            (false, false, false) => CLIPS_LAYOUT,
            (false, false, true) => CLIPS_CARD_LAYOUT,
        };
        if ui.layout_of(CLIPS_PAGE).is_some_and(|l| l != want) {
            let _ = ui.set_layout(CLIPS_PAGE, want);
        }
        // The song list.
        let title = list_title(st);
        let can_add = st.list.len() < mpfree_seq::scenes::MAX_LIST_STEPS;
        let same_list = ui.get::<SongListView>(SONG_LIST).is_some_and(|l| {
            l.rows == st.list
                && l.title == title
                && l.picked == st.list_pick
                && l.looped == st.list_looped
                && l.can_add == can_add
        });
        if !same_list {
            if let Some(l) = ui.get_mut::<SongListView>(SONG_LIST) {
                l.rows = st.list.clone();
                l.title = title;
                l.picked = st.list_pick;
                l.looped = st.list_looped;
                l.can_add = can_add;
            }
        }
        // The blocks shown, while the placements are the same list: a frame
        // that only moves the playhead or scrolls copies and compares no
        // block (Arrange refactor 2.4).
        let blocks = ui
            .get::<SongLanes>(SONG)
            .map(|l| &l.blocks)
            .filter(|b| b.made_from(&st.placements))
            .cloned();
        let kept = blocks.is_some();
        let mut lanes = song_lanes_with(st, self.first_bar, blocks);
        let (same, same_waves) = ui.get::<SongLanes>(SONG).map_or((true, true), |l| {
            let waves = crate::song::same_waves(&l.waves, &st.waves);
            lanes.keep_scroll(l);
            lanes.waves = Vec::new();
            let mut was = l.clone_without_waves();
            was.keep_scroll(l);
            (was == lanes, waves)
        });
        if !(same && same_waves) {
            if let Some(l) = ui.get_mut::<SongLanes>(SONG) {
                let waves = std::mem::take(&mut l.waves);
                *l = lanes;
                l.waves = if same_waves { waves } else { st.waves.clone() };
            }
        } else if !kept {
            // The same blocks in a new list (a new state from the engine):
            // kept as made from it, so the next frames know them in one
            // step. Nothing drawn changes.
            if let Some(l) = ui.get_mut_quiet::<SongLanes>(SONG) {
                l.blocks = lanes.blocks;
            }
        }
        let m = st.morph;
        let focus = st.focus();
        let bar = MorphBar {
            a: m.a,
            b: m.b,
            pos: m.pos,
            on: m.on,
            params: m.params,
            reload: focus.is_some_and(|f| st.saved.contains(&f)),
            reload_waiting: focus.is_some_and(|f| st.reload_waiting(f)),
        };
        if ui.get::<MorphBar>(MORPH).is_some_and(|b| *b != bar) {
            if let Some(b) = ui.get_mut::<MorphBar>(MORPH) {
                *b = bar;
            }
        }
        match &st.edit {
            Some(e) if self.edit_built.as_ref().is_some_and(|b| same_edit(b, e)) => {
                // Only the playhead can have moved.
                let at = note_playhead(e, st);
                if ui.get::<NoteGrid>(NOTES).is_some_and(|g| g.playhead != at) {
                    if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                        g.playhead = at;
                    }
                }
            }
            Some(e) => {
                let grid = note_grid(e, st);
                if ui.get::<NoteGrid>(NOTES).is_some_and(|g| !g.same(&grid)) {
                    if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                        g.show(grid);
                    }
                }
                let rows = edit_rows(&e.editor, &e.clip);
                if ui
                    .get::<OptionList>(EDIT_OPTIONS)
                    .is_some_and(|l| l.rows != rows || l.selected != e.editor.row)
                {
                    if let Some(l) = ui.get_mut::<OptionList>(EDIT_OPTIONS) {
                        l.rows = rows;
                        l.selected = e.editor.row;
                        l.right = "wheel or arrows".to_string();
                    }
                }
                self.edit_built = Some(e.clone());
            }
            None => self.edit_built = None,
        }
        let rows = record_rows(st);
        let tabs = panel_tabs(st);
        if ui
            .get::<OptionList>(RECORD)
            .is_some_and(|l| l.rows != rows || l.tabs != tabs)
        {
            if let Some(l) = ui.get_mut::<OptionList>(RECORD) {
                l.rows = rows;
                l.tabs = tabs.clone();
                l.right = "wheel or arrows".to_string();
            }
        }
        let rows = clip_rows(st);
        let buttons = card_buttons(st);
        if ui
            .get::<OptionList>(CLIP_CARD)
            .is_some_and(|l| l.rows != rows || l.tabs != tabs || l.buttons != buttons)
        {
            if let Some(l) = ui.get_mut::<OptionList>(CLIP_CARD) {
                // Another clip: its card starts at the top.
                if l.tabs != tabs {
                    l.reset_view();
                }
                l.rows = rows;
                l.tabs = tabs.clone();
                l.buttons = buttons;
            }
        }
        let rows = scene_rows(st);
        let buttons = scene_buttons(st);
        if ui
            .get::<OptionList>(SCENE_CARD)
            .is_some_and(|l| l.rows != rows || l.tabs != tabs || l.buttons != buttons)
        {
            if let Some(l) = ui.get_mut::<OptionList>(SCENE_CARD) {
                l.rows = rows;
                l.tabs = tabs;
                l.buttons = buttons;
            }
        }
        let (items, current) = take_items(st);
        let undo = st.done.last().map(TakeRow::short);
        let redo = st.undone.last().map(TakeRow::short);
        let same_takes = ui.get::<TakeList>(TAKES).is_some_and(|l| {
            l.items == items
                && l.current == current
                && l.undo == undo
                && l.redo == redo
                && l.capture == st.capture
        });
        if !same_takes {
            if let Some(l) = ui.get_mut::<TakeList>(TAKES) {
                l.items = items;
                l.current = current;
                l.undo = undo;
                l.redo = redo;
                l.capture = st.capture.clone();
                l.right = "SHIFT + UNDO: redo".to_string();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_seq::{Clip, Note, Track};
    use mpfree_ui::widgets::{Header, Label};
    use mpfree_ui::{Align, Fonts, SoftCanvas, Theme};

    const BAR: Tick = 3840;

    fn state() -> TimeState {
        let mut p = Project::new();
        for (i, name) in ["Drums", "Bass", "Stab"].iter().enumerate() {
            let mut t = Track::new(name, i as u16);
            let mut c = Clip::new("A", 2 * BAR);
            c.add_note(Note {
                tick: 0,
                len: 10,
                note: 1,
                velocity: 100,
                chance: 100,
                cond: mpfree_seq::Cond::Always,
                ratchet: 1,
                touch: mpfree_seq::Touch::NONE,
                on: true,
                ramp: 0,
                join: mpfree_seq::Join::Attack,
            })
            .unwrap();
            t.slots[0] = Some(c);
            t.slots[1] = Some(Clip::new("B", 1680));
            p.tracks.push(t);
        }
        let launches = [
            Launch {
                track: 0,
                slot: 0,
                origin: 0,
                start: 0,
            },
            Launch {
                track: 1,
                slot: 1,
                origin: 5 * BAR,
                start: 5 * BAR,
            },
        ];
        let t = TransportInfo {
            state: State::Playing,
            position: 4 * BAR + 960,
            ..TransportInfo::default()
        };
        TimeState::from_engine(t, &p, &launches, Some(2), &Takes::default())
    }

    fn ev(widget: &str, kind: EventKind) -> UiEvent {
        UiEvent {
            widget: widget.to_string(),
            kind,
        }
    }

    #[test]
    fn the_bar_goes_under_the_header() {
        let play = "col(header:60, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)";
        assert_eq!(with_transport(play).unwrap(),
                   "col(header:60, transport:44, row(pads:*, col(kits:*, level:96, timing:232):460):*, hint:26)");
        assert_eq!(with_transport("col(log:*)"), None);
    }

    #[test]
    fn the_clips_page_fits_the_screen() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        assert_eq!(ui.rect_of(TRANSPORT).map(|r| (r.y, r.h)), Some((84, 44)));
        // Five rows over the morph bar (the wheel scrolls the rest).
        assert!(
            page.visible_rows(&ui) >= 5,
            "{} {:?}",
            page.visible_rows(&ui),
            ui.rect_of(CLIPS)
        );
        let morph = ui.rect_of(MORPH).unwrap();
        assert_eq!(morph.h, 64);
        assert!(morph.w >= 800, "{morph:?}");
        // Five record settings are in view at once (58 px a row); the
        // sixth, Pressure, scrolls in when the row above it is picked (the
        // list keeps the picked row and the next one in view).
        let record = ui.rect_of(RECORD).unwrap();
        assert!(record.h - 14 * 2 - 34 >= 58 * 5, "{record:?}");
        let list = ui.get_mut::<OptionList>(RECORD).unwrap();
        list.rows = (0..RECORD_ROWS.len())
            .map(|k| OptionRow {
                name: RECORD_ROWS[k].to_string(),
                value: String::new(),
            })
            .collect();
        assert!(list.row_rects(record, 5).is_none());
        list.selected = 3;
        assert!(list.row_rects(record, 5).is_none(), "no jump yet");
        list.selected = 4;
        assert!(list.row_rects(record, 5).is_some());
        // Three takes and the buttons.
        let takes = ui.rect_of(TAKES).unwrap();
        assert!(takes.h - 14 * 2 - 34 - 48 - 8 >= 3 * 38, "{takes:?}");
    }

    /// Jay's option E (layout spec 4c): Play's Q-Link column at Arrange's
    /// right edge, from under the header to the hint line, in every view;
    /// the bar ends short of it, Record (or Edit) and Takes narrow to
    /// 250 px, and the grid, the song and the note editor keep their width.
    #[test]
    fn the_qlink_column_runs_down_arranges_right_edge() {
        use mpfree_ui::geom::Rect;
        use mpfree_ui::qlink::COLUMN_WIDTH;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        // The app's column is there first: add_to keeps it.
        let mut mine = QLinkColumn::new();
        mine.title = "Q-Links · Arrange".into();
        ui.add(QLINKS, mine);
        TimePage::add_to(&mut ui).unwrap();
        assert_eq!(
            ui.get::<QLinkColumn>(QLINKS).unwrap().title,
            "Q-Links · Arrange"
        );
        ui.show(CLIPS_PAGE).unwrap();
        for (layout, body, side) in [
            (CLIPS_LAYOUT, CLIPS, RECORD),
            (SONG_LAYOUT, SONG, RECORD),
            (CLIPS_CARD_LAYOUT, CLIPS, CLIP_CARD),
            (SONG_CARD_LAYOUT, SONG, CLIP_CARD),
            (EDIT_LAYOUT, NOTES, EDIT_OPTIONS),
        ] {
            ui.set_layout(CLIPS_PAGE, layout).unwrap();
            // Where Play has it (play_page's frame test): x 1140, y 84.
            assert_eq!(
                ui.rect_of(QLINKS),
                Some(Rect::new(1140, 84, COLUMN_WIDTH, 666)),
                "{layout}"
            );
            assert_eq!(ui.rect_of(TRANSPORT), Some(Rect::new(12, 84, 1116, 44)));
            let s = ui.rect_of(side).unwrap();
            assert_eq!((s.x, s.w), (878, SIDE_W), "{layout}");
            assert_eq!(ui.rect_of(TAKES).map(|t| (t.x, t.w)), Some((878, SIDE_W)));
            let b = ui.rect_of(body).unwrap();
            assert_eq!((b.x, b.right()), (12, 866), "{layout}");
        }
        // Without the app, the samples get an empty column.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        TimePage::add_to(&mut ui).unwrap();
        assert!(ui.get::<QLinkColumn>(QLINKS).unwrap().cards[0].is_free());
    }

    /// Every record setting, Edit option and take button reads whole in
    /// the narrowed panel (nothing is cut short with an ellipsis).
    #[test]
    fn the_narrow_panels_fit_their_words() {
        use mpfree_ui::FontId;
        let mut fonts = Fonts::embedded();
        // The row is 222 px inside the panel's padding; its value box 119.
        let (name_w, value_w) = (222 - 119 - 20, 119 - 2 * 22);
        let px = Theme::dark().size_small + 1.0;
        let st = state();
        let mut values: Vec<String> = record_rows(&st).into_iter().map(|r| r.value).collect();
        values.extend(["Overdub", "Replace", "1 bar", "16–32", "300.0"].map(String::from));
        let keys = crate::sample::keys_clip();
        let mut ed = crate::editor::Editor::new(0, 0, &keys);
        values.extend(edit_rows(&ed, &keys).into_iter().map(|r| r.value));
        ed.sel = vec![0, 1];
        values.extend(edit_rows(&ed, &keys).into_iter().map(|r| r.value));
        // The clip settings card, at its longest words.
        let mut card = ClipSettings {
            speed: Speed::new(1, 8).unwrap(),
            swing: Swing::new(PPQ / 2, 75),
            quantise: Quantise::new(PPQ / 6, 100),
            follow: Some(Follow {
                bars: MAX_FOLLOW_BARS,
                action: FollowAction::Previous,
                other: FollowAction::Stop,
                chance: 75,
            }),
            launch: None,
            legato: true,
            once: false,
            program: Some(ClipProgram {
                program: 127,
                bank: Some(ClipProgram::MAX_BANK),
            }),
            // Its longest word.
            direction: Direction::PingPong,
        };
        values.extend(card_rows(CARD_STEP, card).into_iter().map(|r| r.value));
        values.extend(card_rows(MAX_CLIP_TICKS, card).into_iter().map(|r| r.value));
        values.extend(
            card_rows(999 * CARD_STEP, card)
                .into_iter()
                .map(|r| r.value),
        );
        values.extend(
            card_rows(MAX_CLIP_TICKS - 1, card)
                .into_iter()
                .map(|r| r.value),
        );
        card.swing = Swing::new(PPQ / 6, 75);
        values.extend(
            card_rows(1001 * CARD_STEP, card)
                .into_iter()
                .map(|r| r.value),
        );
        card.speed = Speed::new(4, 3).unwrap();
        card.quantise = Quantise::new(PPQ / 3, 100);
        card.swing = None;
        card.follow = None;
        card.legato = false;
        card.once = true;
        values.extend(card_rows(CARD_STEP, card).into_iter().map(|r| r.value));
        // Every launch quantise, the project's and a clip's own.
        for g in LAUNCH_GRIDS {
            card.launch = Some(g);
            values.extend(card_rows(CARD_STEP, card).into_iter().map(|r| r.value));
        }
        // The Mutate row (S6) at its longest.
        values.extend(["original", "variant", "—"].map(String::from));
        for name in RECORD_ROWS
            .iter()
            .chain(crate::editor::EDIT_ROWS.iter())
            .chain(CLIP_ROWS.iter())
        {
            let w = fonts.measure(FontId::Bold, px, name);
            assert!(w <= name_w, "{name}: {w} > {name_w}");
        }
        for v in &values {
            let w = fonts.measure(FontId::Bold, px, v);
            assert!(w <= value_w, "{v}: {w} > {value_w}");
        }
    }

    /// The note editor gets each note's pressure, and only the runs its
    /// notes name (an erased note's run stays in the clip for undo).
    #[test]
    fn the_note_editor_gets_each_notes_pressure() {
        let point = |at, value| mpfree_seq::TouchPoint { at, value };
        let mut c = Clip::new("pressed", BAR);
        let gone = c.add_touch(&[point(0, 1), point(5, 2)]).unwrap();
        let kept = c.add_touch(&[point(0, 90), point(40, 30)]).unwrap();
        assert!(gone.len == 2);
        c.add_note(Note::new(0, 240, 1, 100)).unwrap();
        c.add_note(Note {
            touch: kept,
            ..Note::new(480, 240, 2, 100)
        })
        .unwrap();
        let e = EditInfo {
            editor: crate::editor::Editor::new(0, 0, &c),
            clip: c.into(),
            rev: 0,
            track_name: "Drums".into(),
            bar: BAR,
        };
        let g = note_grid(&e, &state());
        assert_eq!(g.touch, [point(0, 90), point(40, 30)]);
        assert!(g.touch_of(0).is_empty());
        assert_eq!(g.touch_of(1), &g.touch[..]);
    }

    /// Piano roll plan 0C.1: while the note editor's clip (one shared
    /// copy) and the editor stay the same, an update moves only the roll's
    /// playhead and builds none of its notes again; a new copy builds it.
    #[test]
    fn the_note_editor_moves_only_its_playhead_while_its_clip_is_the_same() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        let mut c = Clip::new("A", 2 * BAR);
        c.add_note(Note::new(0, 240, 1, 100)).unwrap();
        st.edit = Some(Box::new(EditInfo {
            editor: crate::editor::Editor::new(0, 0, &c),
            clip: c.into(),
            rev: 1,
            track_name: "Drums".into(),
            bar: BAR,
        }));
        page.update(&mut ui, &st);
        let e = st.edit.clone().unwrap();
        let head = note_playhead(&e, &st);
        assert!(head.is_some(), "track 1's clip plays");
        assert_eq!(ui.get::<NoteGrid>(NOTES).unwrap().playhead, head);
        // Marked, to see whether the roll is built again.
        ui.get_mut::<NoteGrid>(NOTES).unwrap().title = "kept".into();
        st.transport.position += 120;
        page.update(&mut ui, &st);
        let g = ui.get::<NoteGrid>(NOTES).unwrap();
        assert_eq!(g.title, "kept", "not built again");
        assert_eq!(g.playhead, note_playhead(&e, &st));
        assert_ne!(g.playhead, head, "the playhead moved");
        // The same notes in a new copy (the clip changed and changed
        // back): built again.
        let e = st.edit.as_mut().unwrap();
        e.clip = Arc::new((*e.clip).clone());
        page.update(&mut ui, &st);
        assert!(ui
            .get::<NoteGrid>(NOTES)
            .unwrap()
            .title
            .starts_with("Edit · A"));
        // Closed and opened again: built again too.
        let open = st.edit.take();
        page.update(&mut ui, &st);
        ui.get_mut::<NoteGrid>(NOTES).unwrap().title = "kept".into();
        st.edit = open;
        page.update(&mut ui, &st);
        assert_ne!(ui.get::<NoteGrid>(NOTES).unwrap().title, "kept");
    }

    #[test]
    fn cells_show_what_each_clip_is_doing() {
        let mut st = state();
        let c = cell_of(&st, 0, 0);
        assert_eq!(
            (c.state, c.detail.as_str(), c.foot.as_str()),
            (CellState::Playing, "2 bars", "")
        );
        assert_eq!(cell_of(&st, 1, 0).foot, "1 note");
        // Bar 5 beat 2 of a 2-bar clip from bar 1: an eighth of the way round.
        assert_eq!(c.progress, 0.125);
        let q = cell_of(&st, 1, 1);
        assert_eq!(
            (q.state, q.detail.as_str(), q.foot.as_str()),
            (CellState::Queued, "7/16", "from 6.1")
        );
        assert_eq!(cell_of(&st, 1, 0).state, CellState::Stopped);
        assert_eq!(cell_of(&st, 0, 2).state, CellState::Empty);
        st.launches.push(Launch {
            track: 2,
            slot: 0,
            origin: 0,
            start: 0,
        });
        assert_eq!(cell_of(&st, 2, 0).state, CellState::Playing);
        st.transport.state = State::Recording;
        let r = cell_of(&st, 2, 0);
        assert_eq!(
            (r.state, r.detail.as_str()),
            (CellState::Recording, "recording")
        );
        assert_eq!(
            cell_of(&st, 0, 0).state,
            CellState::Playing,
            "only the armed track records"
        );
    }

    /// Loop tracks (part 3): a loop cell draws its loop's wave in the
    /// detail's place with its length at the foot, its pass on the wave
    /// (no ring), "overdub" while it takes a layer; the header has a wave
    /// mark; the title counts the loops; each loop's place names its
    /// track and row (or scene).
    #[test]
    fn loop_cells_show_their_wave_and_loops_are_named_by_place() {
        let mut st = crate::sample::loops_playing().unwrap().state();
        st.loops.loops[1].wave = [200; crate::widgets::CELL_WAVE];
        st.loops.in_use = 4;
        st.loops.memory_pct = 38;
        let stopped = cell_of(&st, 3, 0);
        assert_eq!(
            (
                stopped.state,
                stopped.detail.as_str(),
                stopped.foot.as_str()
            ),
            (CellState::Stopped, "2 bars", "")
        );
        assert_eq!(stopped.wave, Some([0; crate::widgets::CELL_WAVE]));
        // A copy's layers copying in (part 6): its foot says so.
        st.loops.loops[0].copying = true;
        assert_eq!(cell_of(&st, 3, 0).foot, "copying in");
        st.loops.loops[0].copying = false;
        let playing = cell_of(&st, 4, 1);
        assert_eq!(playing.state, CellState::Playing);
        assert_eq!(playing.wave, Some([200; crate::widgets::CELL_WAVE]));
        st.loops.loops[1].overdubbing = true;
        let over = cell_of(&st, 4, 1);
        assert_eq!(
            (over.state, over.detail.as_str()),
            (CellState::Recording, "overdub")
        );
        assert_eq!(cell_of(&st, 0, 0).wave, None, "a note clip has no wave");
        let tracks = grid_tracks(&st);
        assert_eq!(
            tracks.iter().map(|t| t.loops).collect::<Vec<_>>(),
            [false, false, false, true, true, true]
        );
        // The places: the first row a loop is in, by its scene's name.
        st.scenes = vec![String::new(), "Verse".into()];
        let places = loop_places(&st);
        let full = |n: usize| places[n].as_ref().map(|p| p.full(n));
        assert_eq!(full(0).as_deref(), Some("Loops A · Loop 1 · row 1"));
        assert_eq!(full(1).as_deref(), Some("Loops B · Loop 2 · Verse"));
        assert_eq!(full(3).as_deref(), Some("Loops A · Loop 4 · row 4"));
        assert_eq!(places[4], None);
        let three = places[2].as_ref().unwrap();
        assert_eq!((three.track, three.slot), (4, 2));
        // The gauge, in the legend.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        page.update(&mut ui, &st);
        let title = &ui.get::<ClipGrid>(CLIPS).unwrap().title;
        assert_eq!(title, "Loops 4 of 8 · memory 38 %");
        // Show in Clips: a cell off the rows shown brings its row.
        page.show_cell(4, 14);
        assert_eq!(page.first_track, 0);
        assert!(page.first_row > 0 && page.first_row <= 14);
    }

    /// A tap on an empty Loop cell arms it and a second lets go; the
    /// armed cell says what REC does there as the first pass goes, and
    /// the REC chip names it (loop-tracks.md part 4).
    #[test]
    fn an_empty_loop_cell_arms_and_says_what_rec_does() {
        let mut st = crate::sample::loops_playing().unwrap().state();
        let tap = |st: &TimeState, track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * SLOTS + slot)), st)
        };
        assert!(st.clip(3, 5).is_none() && st.tracks[3].loops.is_some());
        assert_eq!(tap(&st, 3, 5), Some(TimeAction::LoopCell(Some((3, 5)))));
        assert!(
            matches!(tap(&st, 0, 5), Some(TimeAction::NewClip { .. })),
            "a note track's empty cell makes a clip"
        );
        assert_eq!(cell_of(&st, 3, 5).state, CellState::Empty);
        st.loop_cell = Some((3, 5));
        assert_eq!(tap(&st, 3, 5), Some(TimeAction::LoopCell(None)));
        let said = |st: &TimeState| {
            let c = cell_of(st, 3, 5);
            assert_eq!(c.state, CellState::Armed);
            format!("{} / {} / {}", c.name, c.detail, c.foot)
        };
        assert_eq!(said(&st), "Armed / REC here / ");
        st.loops.waiting = true;
        assert_eq!(said(&st), "Ready / bar line / or PLAY");
        st.loops.recording = true;
        assert_eq!(said(&st), "REC / REC ends / the pass");
        st.scenes = vec![String::new(); 6];
        st.scenes[5] = "Drop".into();
        assert_eq!(loop_cell_name(&st).as_deref(), Some("Loops A · Drop"));
        st.scenes[5].clear();
        assert_eq!(loop_cell_name(&st).as_deref(), Some("Loops A · row 6"));
    }

    #[test]
    fn taps_on_the_grid_launch_stop_make_and_arm() {
        let st = state();
        let tap = |track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * SLOTS + slot)), &st)
        };
        assert_eq!(tap(0, 0), Some(TimeAction::StopClip(0)));
        assert_eq!(tap(0, 1), Some(TimeAction::Launch { track: 0, slot: 1 }));
        assert_eq!(
            tap(1, 1),
            Some(TimeAction::StopClip(1)),
            "a queued clip stops too"
        );
        assert_eq!(tap(2, 5), Some(TimeAction::NewClip { track: 2, slot: 5 }));
        assert_eq!(tap(3, 0), None, "no such track");
        assert_eq!(
            action(&ev(CLIPS, EventKind::Tab(0)), &st),
            Some(TimeAction::Pick(0))
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(4)), &st),
            Some(TimeAction::LaunchScene(4))
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(SLOTS)), &st),
            Some(TimeAction::StopAll)
        );
    }

    /// MIDI Learn on the Clips grid (sequencer S5, Jay's card of 18 Nov):
    /// SHIFT + a tap names a cell, a scene, Stop all, Back to song or a
    /// track's arm, and a learned press later does what that tap does then.
    #[test]
    fn a_learned_press_does_what_a_tap_on_the_grid_does() {
        let st = state();
        let learn = |kind| learn_target(&ev(CLIPS, kind), &st).map(|a| (a.owner, a.param));
        let c = |id| Some((Owner::Clips, id));
        assert_eq!(learn(EventKind::Select(SLOTS + 2)), c(clips::clip(1, 2)));
        assert_eq!(
            learn(EventKind::Select(2 * SLOTS + 5)),
            c(clips::clip(2, 5)),
            "empty"
        );
        assert_eq!(learn(EventKind::Select(3 * SLOTS)), None, "no such track");
        assert_eq!(learn(EventKind::Toggle(4)), c(clips::scene(4)));
        assert_eq!(learn(EventKind::Toggle(SLOTS)), c(clips::STOP_ALL));
        assert_eq!(
            learn(EventKind::Toggle(SLOTS + BACK_TO_SONG)),
            c(clips::BACK_TO_SONG)
        );
        assert_eq!(learn(EventKind::Tab(1)), c(clips::arm(1)));
        // SHIFT keeps its job elsewhere: + Track, the pager, the sound line.
        assert_eq!(learn(EventKind::Toggle(SLOTS + ADD_TRACK)), None);
        assert_eq!(learn(EventKind::Toggle(SLOTS + PAGE_ON)), None);
        assert_eq!(learn(EventKind::Tab(crate::widgets::SOUND_TAP + 1)), None);
        assert_eq!(
            learn_target(&ev(RECORD, EventKind::Tab(1)), &st),
            None,
            "only the grid"
        );
        // Each press is the tap now, wherever the cell's clip stands.
        let tap = |track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * SLOTS + slot)), &st)
        };
        for (t, r) in [(0, 0), (0, 1), (1, 1), (2, 5)] {
            assert_eq!(clips_press(&st, Clips::Clip(t as u8, r as u8)), tap(t, r));
        }
        assert_eq!(clips_press(&st, Clips::Clip(3, 0)), None);
        assert_eq!(
            clips_press(&st, Clips::Scene(4)),
            Some(TimeAction::LaunchScene(4))
        );
        assert_eq!(clips_press(&st, Clips::StopAll), Some(TimeAction::StopAll));
        assert_eq!(
            clips_press(&st, Clips::BackToSong),
            Some(TimeAction::BackToSong)
        );
        assert_eq!(
            clips_press(&st, Clips::Stop(1)),
            Some(TimeAction::StopClip(1))
        );
        assert_eq!(
            clips_press(&st, Clips::Arm(1)),
            action(&ev(CLIPS, EventKind::Tab(1)), &st)
        );
        assert_eq!(clips_press(&st, Clips::Arm(3)), None);
        assert_eq!(clips_press(&st, Clips::Mute(0)), None, "the app's");
        assert_eq!(clips_press(&st, Clips::Solo(0)), None, "the app's");
    }

    /// More than 8 tracks (up to 128, Jay 14 Nov): the grid follows the
    /// armed track to its page, the pager steps 8 at a time within the
    /// tracks there are, and "+ Track" on the grid and the Song view adds
    /// one.
    #[test]
    fn the_grid_pages_through_24_tracks_and_plus_track_adds_one() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = crate::sample::big_playing(10).unwrap().state();
        assert_eq!(st.tracks.len(), 24);
        page.update(&mut ui, &st);
        assert_eq!(page.first_track, 8, "Vox chop's page");
        let g = ui.get::<ClipGrid>(CLIPS).unwrap();
        assert_eq!((g.first_track, g.can_add), (8, true));
        let on = ev(CLIPS, EventKind::Toggle(SLOTS + PAGE_ON));
        let back = ev(CLIPS, EventKind::Toggle(SLOTS + PAGE_BACK));
        assert_eq!(action(&on, &st), None, "the page's own");
        assert!(page.page_tracks(&on, &st));
        assert_eq!(page.first_track, 16);
        assert!(page.page_tracks(&on, &st));
        assert_eq!(page.first_track, 16, "the last page");
        page.update(&mut ui, &st);
        assert_eq!(
            page.first_track, 16,
            "paging away from the armed track stays"
        );
        assert!(page.page_tracks(&back, &st));
        assert!(page.page_tracks(&back, &st));
        assert!(page.page_tracks(&back, &st));
        assert_eq!(page.first_track, 0);
        assert!(!page.page_tracks(&ev(CLIPS, EventKind::Toggle(SLOTS)), &st));
        // Arming a track off the page brings its page.
        st.armed = Some(21);
        page.update(&mut ui, &st);
        assert_eq!(page.first_track, 16);
        // Fewer tracks (an undone + Track, a new project): the page stays
        // within them.
        st.tracks.truncate(8);
        st.armed = Some(2);
        page.update(&mut ui, &st);
        assert_eq!(page.first_track, 0);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(SLOTS + ADD_TRACK)), &st),
            Some(TimeAction::AddTrack)
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(SLOTS + 1)), &st),
            Some(TimeAction::SongView(true))
        );
        assert_eq!(
            action(&ev(SONG, EventKind::Tab(crate::song::ADD_TRACK)), &st),
            Some(TimeAction::AddTrack)
        );
        assert!(song_lanes(&st, 0).can_add_track);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(SLOTS + BACK_TO_SONG)), &st),
            Some(TimeAction::BackToSong)
        );
    }

    #[test]
    fn a_frame_that_moves_the_playhead_or_scrolls_copies_no_block() {
        // Arrange refactor 2.4: while the placements are the same list, the
        // Song view keeps its blocks, so a long song costs a frame no more
        // than a short one; new placements are new blocks.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        let place = |n: usize| -> Arc<[PlacementInfo]> {
            (0..n)
                .map(|k| PlacementInfo {
                    track: k % 3,
                    start: (k / 3) as Tick * BAR,
                    length: BAR,
                    name: format!("Clip {k}"),
                    unique: false,
                })
                .collect::<Vec<_>>()
                .into()
        };
        st.placements = place(4096);
        page.update(&mut ui, &st);
        let blocks = |ui: &Ui| ui.get::<SongLanes>(SONG).unwrap().blocks.as_ptr();
        let first = blocks(&ui);
        for k in 0..4 {
            st.transport.position += 960;
            page.update(&mut ui, &st);
            page.scroll(&ui, k);
            page.update(&mut ui, &st);
            let l = ui.get::<SongLanes>(SONG).unwrap();
            assert_eq!(l.first_bar, (0..=k).sum::<i32>() as i64);
            assert_eq!(l.playhead, Some(st.transport.position));
            assert!(
                std::ptr::eq(blocks(&ui), first),
                "frame {k} copied the blocks"
            );
        }
        // An equal list from the engine (a new state) is copied once, with
        // nothing to draw, and the frames after it copy nothing.
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        st.placements = place(4096);
        page.update(&mut ui, &st);
        assert!(ui.paint(&mut c).is_empty(), "nothing changed on the screen");
        let second = blocks(&ui);
        assert!(!std::ptr::eq(second, first));
        st.transport.position += 960;
        page.update(&mut ui, &st);
        page.update(&mut ui, &st);
        assert!(std::ptr::eq(blocks(&ui), second));
        // A changed one shows.
        st.placements = place(5);
        page.update(&mut ui, &st);
        let l = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!(l.blocks.len(), 5);
        assert_eq!(l.blocks.get(4).map(|b| b.name.as_str()), Some("Clip 4"));
    }

    #[test]
    fn the_song_view_swaps_in_for_the_grid_and_its_buttons_act() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(SLOTS + 1)), &st),
            Some(TimeAction::SongView(true))
        );
        st.song_view = true;
        st.placements = vec![PlacementInfo {
            track: 0,
            start: 3840,
            length: 7680,
            name: "Boom".into(),
            unique: false,
        }]
        .into();
        page.update(&mut ui, &st);
        assert!(ui.rect_of(SONG).is_some() && ui.rect_of(CLIPS).is_none());
        let lanes = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!(
            (lanes.blocks.len(), lanes.legend.as_str()),
            (1, "3 bars · from bar 1")
        );
        page.scroll(&ui, 4);
        assert_eq!(page.first_bar, 4, "the wheel scrolls bars");
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        assert_eq!(
            song(EventKind::Tab(0), &st),
            Some(TimeAction::SongView(false))
        );
        assert_eq!(
            song(EventKind::Tab(1), &st),
            Some(TimeAction::SongMode(true))
        );
        assert_eq!(song(EventKind::Tab(2), &st), Some(TimeAction::ScenesToSong));
        assert_eq!(song(EventKind::Tab(3), &st), Some(TimeAction::SongToScenes));
        assert_eq!(song(EventKind::Tab(4), &st), None, "nothing selected");
        assert_eq!(
            song(EventKind::Select(0), &st),
            Some(TimeAction::SelectPlacement(Some(0)))
        );
        assert_eq!(
            song(EventKind::Select(NONE), &st),
            Some(TimeAction::SelectPlacement(None))
        );
        st.selected = Some(0);
        assert_eq!(
            song(EventKind::Tab(4), &st),
            Some(TimeAction::MakeUnique(0))
        );
        assert_eq!(
            song(EventKind::Tab(5), &st),
            Some(TimeAction::DeletePlacement(0))
        );
        // Rec song, on the Song view and on the grid.
        assert_eq!(
            song(EventKind::Tab(REC), &st),
            Some(TimeAction::RecSong(true))
        );
        st.song_rec = true;
        page.update(&mut ui, &st);
        assert!(ui.get::<SongLanes>(SONG).unwrap().rec);
        st.song_view = false;
        page.update(&mut ui, &st);
        assert!(ui.rect_of(CLIPS).is_some() && ui.rect_of(SONG).is_none());
        assert!(ui.get::<ClipGrid>(CLIPS).unwrap().song_rec);
        assert_eq!(
            action(&ev(CLIPS, EventKind::Toggle(SLOTS + 2)), &st),
            Some(TimeAction::RecSong(false))
        );
    }

    /// The song list (S4): List on the Song view shows it in place of the
    /// grid and the morph; its title counts the steps, bars and time;
    /// each touch is its action or says why not; NEXT names the next
    /// step.
    #[test]
    fn the_song_list_swaps_in_and_its_touches_act() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        assert_eq!(
            action(&ev(SONG, EventKind::Tab(crate::song::LIST)), &st),
            Some(TimeAction::ListView(true))
        );
        st.list_view = true;
        page.update(&mut ui, &st);
        assert!(ui.rect_of(SONG_LIST).is_some());
        assert!(ui.rect_of(SONG).is_none() && ui.rect_of(MORPH).is_none());
        assert_eq!(
            ui.get::<SongListView>(SONG_LIST).unwrap().title,
            "Song list"
        );
        let list = |k: EventKind, st: &TimeState| action(&ev(SONG_LIST, k), st);
        let why = |k: EventKind, st: &TimeState| why_not(&ev(SONG_LIST, k), st);
        assert_eq!(
            list(EventKind::Toggle(LIST_ADD), &st),
            Some(TimeAction::ListAdd)
        );
        assert_eq!(list(EventKind::Toggle(LIST_TO_SONG), &st), None);
        assert_eq!(
            why(EventKind::Toggle(LIST_TO_SONG), &st).as_deref(),
            Some("Add a scene to the list first")
        );
        // Two steps: 2 × 4 bars and 1 × 2 bars at 120 BPM.
        st.list = vec![
            ListRow {
                name: "Intro".into(),
                row: 0,
                times: 2,
                bars: 8,
                starts: 1,
                ..ListRow::default()
            },
            ListRow {
                name: "Drop".into(),
                row: 3,
                times: 1,
                bars: 2,
                starts: 9,
                ..ListRow::default()
            },
        ];
        page.update(&mut ui, &st);
        assert_eq!(
            ui.get::<SongListView>(SONG_LIST).unwrap().title,
            "Song list · 2 steps · 10 bars · 0:20"
        );
        assert_eq!(list(EventKind::Toggle(LIST_PLAY), &st), None);
        assert_eq!(
            why(EventKind::Toggle(LIST_REMOVE), &st).as_deref(),
            Some("Tap a step first")
        );
        assert_eq!(
            list(EventKind::Select(1), &st),
            Some(TimeAction::ListPick(Some(1)))
        );
        st.list_pick = Some(1);
        assert_eq!(
            list(EventKind::Toggle(LIST_PLAY), &st),
            Some(TimeAction::ListPlay(1))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_REMOVE), &st),
            Some(TimeAction::ListRemove(1))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_TO_SONG), &st),
            Some(TimeAction::ListToSong)
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_LOOP), &st),
            Some(TimeAction::ListLoop(true))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_CLIPS), &st),
            Some(TimeAction::SongView(false))
        );
        assert_eq!(
            list(EventKind::Toggle(LIST_SONG), &st),
            Some(TimeAction::SongView(true))
        );
        assert_eq!(
            list(EventKind::Step { index: 0, delta: 1 }, &st),
            Some(TimeAction::ListTimes { step: 0, delta: 1 })
        );
        assert_eq!(
            list(
                EventKind::Step {
                    index: 1,
                    delta: -1
                },
                &st
            ),
            None
        );
        assert_eq!(
            why(
                EventKind::Step {
                    index: 1,
                    delta: -1
                },
                &st
            )
            .as_deref(),
            Some("Once is the fewest")
        );
        assert_eq!(
            list(
                EventKind::Step {
                    index: SCENE_STEP + 1,
                    delta: -1
                },
                &st
            ),
            Some(TimeAction::ListScene { step: 1, delta: -1 })
        );
        // The Clip tab keeps the list.
        page.clip_card = true;
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(LIST_CARD_LAYOUT));
        assert_eq!(page.wheel_list(&ui), CLIP_CARD);
        page.clip_card = false;
        // NEXT: the step after the one playing, then the list's end.
        st.perform = true;
        st.transport.state = State::Playing;
        let bar = st.transport.meter.bar_ticks();
        st.transport.position = 3 * bar;
        st.list_next = Some((Some(1), 4 * bar));
        assert_eq!(st.next_reading().as_deref(), Some("Drop in 1 bar"));
        st.list_next = Some((None, 4 * bar));
        assert_eq!(st.next_reading().as_deref(), Some("Stop all in 1 bar"));
        st.list_view = false;
        st.song_view = false;
        page.update(&mut ui, &st);
        assert!(ui.rect_of(CLIPS).is_some() && ui.rect_of(SONG_LIST).is_none());
    }

    /// Jay, 14 Nov: drag and drop on the Song view. Blocks, their ends
    /// and sections ask to move; a section's name picks it; held at an
    /// edge the bars scroll; the hint follows what is picked.
    #[test]
    fn song_drags_ask_to_move_and_the_hint_follows_the_pick() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        st.markers = vec![(0, "Intro".into()), (4 * 3840, "Verse".into())];
        page.update(&mut ui, &st);
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        assert_eq!(
            song(
                EventKind::Place {
                    index: 2,
                    row: 1,
                    at: 7680
                },
                &st
            ),
            Some(TimeAction::MovePlacement {
                index: 2,
                track: 1,
                start: 7680,
                copy: false
            })
        );
        assert_eq!(
            song(EventKind::Stretch { index: 2, to: 3840 }, &st),
            Some(TimeAction::StretchPlacement {
                index: 2,
                length: 3840
            })
        );
        assert_eq!(
            song(EventKind::Drop { from: 1, to: 0 }, &st),
            Some(TimeAction::MoveSection {
                marker: 1,
                gap: 0,
                copy: false
            })
        );
        assert_eq!(
            song(EventKind::Tab(SECTION_TAP + 1), &st),
            Some(TimeAction::PickSection(Some(1)))
        );
        assert_eq!(song_hint(&st), SONG_HINT);
        st.section = Some(1);
        assert_eq!(
            song(EventKind::Tab(SECTION_TAP + 1), &st),
            Some(TimeAction::PickSection(None)),
            "a second tap lets it go"
        );
        assert_eq!(song_hint(&st), SONG_SECTION_HINT);
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().section, Some(1));

        // The tempo lane (tempo ramps): a tap adds a point, a tap on one
        // picks it and a second lets it go, a drag moves it; picked, the
        // last two buttons glide it and delete it, the hint says so and
        // the lanes show the points.
        let tempo = |index, at| EventKind::Place {
            index,
            row: TEMPO_ROW,
            at,
        };
        assert_eq!(
            song(tempo(NONE, 7680), &st),
            Some(TimeAction::SongTempo(TempoEdit::Add(7680)))
        );
        st.tempo_points = vec![(3840, 100.0, false), (11_520, 140.0, true)];
        assert_eq!(
            song(tempo(0, 7680), &st),
            Some(TimeAction::SongTempo(TempoEdit::Move {
                index: 0,
                at: 7680
            }))
        );
        let tap = |k| EventKind::Cell {
            row: TEMPO_ROW,
            col: k,
        };
        assert_eq!(
            song(tap(1), &st),
            Some(TimeAction::SongTempo(TempoEdit::Pick(Some(1))))
        );
        st.tempo_pick = Some(1);
        assert_eq!(
            song(tap(1), &st),
            Some(TimeAction::SongTempo(TempoEdit::Pick(None))),
            "a second tap lets it go"
        );
        assert_eq!(
            song(EventKind::Tab(4), &st),
            Some(TimeAction::SongTempo(TempoEdit::Ramp {
                index: 1,
                on: false
            }))
        );
        assert_eq!(
            song(EventKind::Tab(5), &st),
            Some(TimeAction::SongTempo(TempoEdit::Delete(1)))
        );
        assert_eq!(song_hint(&st), SONG_TEMPO_HINT);
        st.tempo_base = 92.0;
        page.update(&mut ui, &st);
        let lanes = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!(
            (lanes.tempo.clone(), lanes.tempo_base, lanes.tempo_pick),
            (st.tempo_points.clone(), 92.0, Some(1))
        );
        st.tempo_pick = None;
        st.tempo_points.clear();
        st.section = None;
        st.selected = Some(0);
        assert_eq!(song_hint(&st), SONG_BLOCK_HINT);
        // A drag held at an edge scrolls the bars, never before bar 1.
        let edge = |k| UiEvent {
            widget: SONG.into(),
            kind: EventKind::Tab(k),
        };
        assert!(page.page_tracks(&edge(BARS_ON), &st));
        assert!(page.page_tracks(&edge(BARS_ON), &st));
        assert_eq!(page.first_bar, 2);
        assert!(page.page_tracks(&edge(BARS_BACK), &st));
        assert!(page.page_tracks(&edge(BARS_BACK), &st));
        assert!(page.page_tracks(&edge(BARS_BACK), &st));
        assert_eq!(page.first_bar, 0);
        // The undo list names a song edit by its track and bar.
        let mut takes = Takes::default();
        for _ in 0..6 {
            takes.song_edit(
                &mpfree_seq::Project::new(),
                SongChange::Scenes { rows: 1 },
                Default::default(),
            );
        }
        let edit = |change| {
            let mut t = takes.clone();
            t.song_edit(&mpfree_seq::Project::new(), change, Default::default());
            t.done().last().unwrap().clone()
        };
        let moved = TakeRow::of(&edit(SongChange::Moved {
            track: 0,
            start: 8 * 3840,
        }));
        assert_eq!(moved.short(), "move 7");
        assert_eq!(
            take_title(&st, &moved),
            format!("Move 7 · {}", st.track_name(0))
        );
        assert_eq!(take_detail(&st, &moved), "to bar 9");
        let verse = TakeRow::of(&edit(SongChange::SectionCopied { at: 4 * 3840 }));
        assert_eq!(take_title(&st, &verse), "Copy 7 · Verse");
        let longer = TakeRow::of(&edit(SongChange::Resized {
            track: 0,
            length: 6 * 3840,
        }));
        assert_eq!(take_detail(&st, &longer), "6 bars");
    }

    /// Layout spec 4c: the audio lanes' taps, the selection's kind, and
    /// the bottom row with a region picked.
    #[test]
    fn the_audio_lanes_ask_for_audio_actions() {
        use crate::song::{AudioBlock, AudioLane};
        let mut st = state();
        st.song_view = true;
        st.audio = vec![AudioLane {
            track: 2,
            name: "Vocal".into(),
            color: audio_color(2),
            armed: true,
            input: Input::Left,
            mix: None,
            level: 0.3,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: false,
            solo: false,
            blocks: vec![AudioBlock {
                start: 0,
                end: BAR,
                take: 0,
                tag: String::new(),
                src: 0,
                frames: 88_200.0,
                gain: 1.0,
                fade_in: 0,
                fade_out: 0,
            }],
            takes: Vec::new(),
            recording: None,
        }];
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        let audio = |a: AudioAction| Some(TimeAction::Audio(a));
        let r = RegionId { track: 2, index: 0 };
        assert_eq!(
            song(EventKind::Cell { row: 2, col: 0 }, &st),
            audio(AudioAction::Pick(Some(r)))
        );
        assert_eq!(
            song(EventKind::Cell { row: 2, col: PANEL }, &st),
            audio(AudioAction::Panel(2))
        );
        assert_eq!(
            song(EventKind::Toggle(2), &st),
            audio(AudioAction::Arm {
                track: 2,
                on: false
            }),
            "armed: the dot disarms"
        );
        assert_eq!(
            song(EventKind::Toggle(5), &st),
            audio(AudioAction::Arm { track: 5, on: true })
        );
        assert_eq!(
            song(EventKind::Tab(ADD_AUDIO), &st),
            audio(AudioAction::Add)
        );
        // Nothing picked: the Song view's own row.
        assert_eq!(st.pick(), None);
        assert_eq!(
            song(EventKind::Tab(0), &st),
            Some(TimeAction::SongView(false))
        );
        st.selected = Some(1);
        assert_eq!(st.pick(), Some(Pick::Placement(1)));
        // A region picked: its row, whatever was picked before.
        st.region = Some(r);
        assert_eq!(st.pick(), Some(Pick::Region(r)));
        let row = [
            AudioAction::Split(r),
            AudioAction::Erase(r),
            AudioAction::Fades(r),
            AudioAction::Gain(r),
            AudioAction::Takes(2),
            AudioAction::Undo,
        ];
        for (k, want) in row.into_iter().enumerate() {
            assert_eq!(song(EventKind::Tab(k), &st), audio(want), "button {k}");
        }
        // Rec song and empty space still do what they did.
        assert_eq!(
            song(EventKind::Tab(REC), &st),
            Some(TimeAction::RecSong(true))
        );
        assert_eq!(
            song(EventKind::Select(NONE), &st),
            Some(TimeAction::SelectPlacement(None))
        );
        // The widget shows the lane, the pick and the memory legend.
        st.audio_free = Some((31.4, 60.0));
        st.audio_counts = vec![("dropped".into(), 0), ("read errors".into(), 2)];
        let lanes = song_lanes(&st, 0);
        assert_eq!(lanes.region, Some((2, 0)));
        assert_eq!(lanes.audio.len(), 1);
        assert!(
            lanes
                .legend
                .ends_with("Audio 0:31 of 1:00 free · 2 read errors"),
            "{}",
            lanes.legend
        );
        assert!(lanes.alert, "a loss shows in the warning colour");
        assert!(lanes.waves.is_empty(), "update copies the waves");
    }

    /// Layout spec 4c: the track panel's controls ask for the open
    /// panel's track; with no panel open they ask for nothing.
    #[test]
    fn the_track_panel_asks_for_its_track() {
        use crate::song::{
            AudioLane, PANEL_CLOSE, PANEL_INPUT, PANEL_LEVEL, PANEL_MONITOR, PANEL_MUTE, PANEL_PAN,
            PANEL_SOLO, PANEL_TAKES,
        };
        let mut st = state();
        st.song_view = true;
        st.audio = vec![AudioLane {
            track: 3,
            name: "Audio 4".into(),
            color: audio_color(3),
            armed: false,
            input: Input::Left,
            mix: None,
            level: 0.0,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: true,
            solo: false,
            blocks: Vec::new(),
            takes: Vec::new(),
            recording: None,
        }];
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        let audio = |a: AudioAction| Some(TimeAction::Audio(a));
        assert_eq!(song(EventKind::Tab(PANEL_MUTE), &st), None, "no panel");
        assert_eq!(
            song(
                EventKind::Step {
                    index: PANEL_LEVEL,
                    delta: 1
                },
                &st
            ),
            None
        );
        st.audio_panel = Some(3);
        assert_eq!(lanes_panel(&st), Some(3));
        let tab = |k: usize, st: &TimeState| song(EventKind::Tab(k), st);
        assert_eq!(tab(PANEL_CLOSE, &st), audio(AudioAction::ClosePanel));
        assert_eq!(
            tab(PANEL_MUTE, &st),
            audio(AudioAction::Mute {
                track: 3,
                on: false
            }),
            "muted: Mute unmutes"
        );
        assert_eq!(
            tab(PANEL_SOLO, &st),
            audio(AudioAction::Solo { track: 3, on: true })
        );
        assert_eq!(tab(PANEL_TAKES, &st), audio(AudioAction::Takes(3)));
        assert_eq!(
            tab(PANEL_INPUT + 3, &st),
            audio(AudioAction::Input {
                track: 3,
                input: Input::Stereo
            })
        );
        assert_eq!(
            tab(PANEL_INPUT + 2, &st),
            audio(AudioAction::Input {
                track: 3,
                input: Input::Sum
            })
        );
        assert_eq!(
            tab(PANEL_MONITOR + 2, &st),
            audio(AudioAction::Monitor {
                track: 3,
                monitor: Monitor::Off
            })
        );
        // Mix and its From steps and Effects (step 9).
        use crate::song::{PANEL_AFTER, PANEL_BEFORE, PANEL_FROM_LESS, PANEL_FROM_MORE, PANEL_MIX};
        let mix = |step| audio(AudioAction::Mix { track: 3, step });
        assert_eq!(
            tab(PANEL_MONITOR + 3, &st),
            mix(0),
            "Mix follows the monitors"
        );
        assert_eq!(tab(PANEL_MIX, &st), mix(0));
        assert_eq!(tab(PANEL_FROM_LESS, &st), mix(-1));
        assert_eq!(tab(PANEL_FROM_MORE, &st), mix(1));
        let fx = |after| audio(AudioAction::MixEffects { track: 3, after });
        assert_eq!(tab(PANEL_BEFORE, &st), fx(false));
        assert_eq!(tab(PANEL_AFTER, &st), fx(true));
        assert_eq!(tab(PANEL_AFTER + 1, &st), None, "past the last");
        let step = |index, delta, st: &TimeState| song(EventKind::Step { index, delta }, st);
        let turn = |knob, ticks, region| {
            audio(AudioAction::Turn {
                track: 3,
                region,
                knob,
                ticks,
                fine: false,
            })
        };
        assert_eq!(step(PANEL_LEVEL, -1, &st), turn(0, -1, None));
        // A region picked on another track is not this track's.
        st.region = Some(RegionId { track: 1, index: 0 });
        assert_eq!(step(PANEL_PAN, 1, &st), turn(1, 1, None));
        st.region = Some(RegionId { track: 3, index: 2 });
        assert_eq!(step(PANEL_PAN, 1, &st), turn(1, 1, Some(2)));
        // The region's row still answers with the panel open.
        assert_eq!(
            song(EventKind::Tab(0), &st),
            audio(AudioAction::Split(RegionId { track: 3, index: 2 }))
        );
    }

    /// Layout spec 4c: a tap on a take lane uses the take, a swipe comps
    /// the stretch; the Song view shows the open track's take lanes.
    #[test]
    fn take_lanes_ask_to_comp() {
        let mut st = state();
        st.song_view = true;
        let song = |k: EventKind, st: &TimeState| action(&ev(SONG, k), st);
        let audio = |a: AudioAction| Some(TimeAction::Audio(a));
        assert_eq!(
            song(
                EventKind::Span {
                    index: 3,
                    from: 0,
                    to: 0
                },
                &st
            ),
            audio(AudioAction::UseTake(3))
        );
        assert_eq!(
            song(
                EventKind::Span {
                    index: 3,
                    from: 960,
                    to: 2880
                },
                &st
            ),
            audio(AudioAction::Comp {
                take: 3,
                from: 960,
                to: 2880,
                fine: false
            })
        );
        assert_eq!(
            song(
                EventKind::Span {
                    index: 70_000,
                    from: 0,
                    to: 0
                },
                &st
            ),
            None,
            "no such take"
        );
        st.audio_takes = Some(2);
        assert_eq!(song_lanes(&st, 0).takes_open, Some(2));
        // A region dragged: its track and index, and where it starts now.
        assert_eq!(
            song(
                EventKind::Marker {
                    index: 3 * crate::song::REGION_SLOTS + 4,
                    frame: 7680
                },
                &st
            ),
            audio(AudioAction::Move {
                region: RegionId { track: 3, index: 4 },
                to: 7680,
                fine: false
            })
        );
    }

    fn lanes_panel(st: &TimeState) -> Option<usize> {
        song_lanes(st, 0).panel
    }

    /// The waveforms reach the widget, and are copied again only when they
    /// change; the lanes' scroll survives an update.
    #[test]
    fn the_song_view_keeps_its_waves_and_scroll() {
        use mpfree_audiotracks::host::TakeWave;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.song_view = true;
        st.waves = vec![TakeWave {
            id: 0,
            frames: 1000,
            channels: 1,
            frames_per_column: 10,
            columns: vec![(-0.5, 0.5); 100],
        }];
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().waves.len(), 1);
        ui.get_mut::<SongLanes>(SONG).unwrap().first_lane = 1;
        st.transport.position += 100;
        page.update(&mut ui, &st);
        let l = ui.get::<SongLanes>(SONG).unwrap();
        assert_eq!((l.waves.len(), l.first_lane), (1, 1));
        assert_eq!(l.playhead, Some(st.transport.position));
        st.waves[0].frames = 1200;
        st.waves[0].columns.push((0.0, 0.0));
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<SongLanes>(SONG).unwrap().waves[0].frames, 1200);
    }

    #[test]
    fn the_morph_bar_picks_rows_moves_and_lets_go() {
        let mut st = state();
        let morph = |k: EventKind, st: &TimeState| action(&ev(MORPH, k), st);
        assert_eq!(
            morph(
                EventKind::Step {
                    index: 0,
                    delta: -1
                },
                &st
            ),
            Some(TimeAction::MorphRows { a: SLOTS - 1, b: 1 }),
            "round to the last row"
        );
        assert_eq!(
            morph(EventKind::Step { index: 1, delta: 1 }, &st),
            Some(TimeAction::MorphRows { a: 0, b: 2 })
        );
        assert_eq!(
            morph(EventKind::Value(0.3), &st),
            Some(TimeAction::Morph(0.3))
        );
        assert_eq!(
            morph(EventKind::Tab(0), &st),
            None,
            "nothing to snap back from"
        );
        assert_eq!(
            morph(EventKind::Tab(2), &st),
            Some(TimeAction::SaveScene(0))
        );
        st.morph.on = true;
        st.morph.pos = 0.8;
        assert_eq!(morph(EventKind::Tab(0), &st), Some(TimeAction::SnapBack));
        assert_eq!(morph(EventKind::Tab(1), &st), Some(TimeAction::KeepMorph));
        assert_eq!(
            morph(EventKind::Tab(2), &st),
            Some(TimeAction::SaveScene(1))
        );
        assert_eq!(
            action(&ev(CLIPS, EventKind::Tab(2)), &st),
            Some(TimeAction::Arm(None)),
            "arming again disarms"
        );
    }

    #[test]
    fn the_bar_and_takes_buttons_map_to_commands() {
        let st = state();
        let bar = |b: BarButton| action(&ev(TRANSPORT, EventKind::Tab(b.index())), &st);
        assert_eq!(
            bar(BarButton::Stop),
            Some(TimeAction::Transport(Command::Stop))
        );
        assert_eq!(
            bar(BarButton::Play),
            Some(TimeAction::Transport(Command::Play))
        );
        assert_eq!(
            bar(BarButton::Rec),
            Some(TimeAction::Transport(Command::Record))
        );
        assert_eq!(
            bar(BarButton::Mode),
            Some(TimeAction::Transport(Command::ToggleOverdub))
        );
        assert_eq!(bar(BarButton::Undo), Some(TimeAction::Undo));
        assert_eq!(
            action(&ev(TRANSPORT, EventKind::Select(0)), &st),
            Some(TimeAction::Chip(0))
        );
        // With nothing to undo, redo or capture, the take buttons are faint:
        // they send nothing and say why in place.
        for i in 0..3 {
            let e = ev(TAKES, EventKind::Tab(i));
            assert_eq!(action(&e, &st), None, "take button {i}");
            assert!(why_not(&e, &st).is_some(), "take button {i} says why");
        }
        let take = TakeRow {
            number: 1,
            captured: false,
            edited: false,
            made: true,
            at: Some((0, 0)),
            added: 4,
            erased: 0,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
        };
        let mut st = st;
        st.done = vec![take];
        st.undone = vec![take];
        st.capture = Some("2 bars".to_string());
        assert_eq!(
            action(&ev(TAKES, EventKind::Tab(0)), &st),
            Some(TimeAction::Undo)
        );
        assert_eq!(
            action(&ev(TAKES, EventKind::Tab(1)), &st),
            Some(TimeAction::Redo)
        );
        assert_eq!(
            action(&ev(TAKES, EventKind::Tab(2)), &st),
            Some(TimeAction::Capture)
        );
    }

    #[test]
    fn record_settings_step_within_their_range() {
        let mut st = state();
        let step = |st: &TimeState, index: usize, delta: i32| {
            action(&ev(RECORD, EventKind::Step { index, delta }), st)
        };
        // Armed: Stab (2) -> none -> Drums (0), and back.
        assert_eq!(step(&st, 0, 1), Some(TimeAction::Arm(None)));
        st.armed = None;
        assert_eq!(step(&st, 0, 1), Some(TimeAction::Arm(Some(0))));
        assert_eq!(step(&st, 0, -1), Some(TimeAction::Arm(Some(2))));
        assert_eq!(
            step(&st, 1, -1),
            Some(TimeAction::Transport(Command::ToggleOverdub))
        );
        assert_eq!(step(&st, 2, -1), None, "count-in is already off");
        assert_eq!(
            step(&st, 2, 1),
            Some(TimeAction::Transport(Command::SetCountIn(1)))
        );
        st.transport.count_in = MAX_COUNT_IN;
        assert_eq!(step(&st, 2, 1), None);
        assert_eq!(
            step(&st, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, BAR)))))
        );
        st.transport.loop_range = Some((0, 4 * BAR));
        assert_eq!(
            step(&st, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 8 * BAR)))))
        );
        assert_eq!(
            step(&st, 3, -1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 2 * BAR)))))
        );
        // A loop set elsewhere (bars 3 to 5) steps from the nearest length.
        st.transport.loop_range = Some((2 * BAR, 5 * BAR));
        assert_eq!(
            step(&st, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 4 * BAR)))))
        );
        // Following another device's position the transport's loop is off:
        // the row shows and steps the loop held until that stops.
        let following = TimeState {
            transport: TransportInfo {
                loop_range: Some((0, 2 * BAR)),
                follows_position: true,
                ..st.transport
            },
            ..st.clone()
        };
        assert_eq!(record_rows(&following)[3].value, "1–2, off while following");
        assert_eq!(
            step(&following, 3, 1),
            Some(TimeAction::Transport(Command::SetLoop(Some((0, 4 * BAR)))))
        );
        let none_held = TimeState {
            transport: TransportInfo {
                loop_range: None,
                ..following.transport
            },
            ..st.clone()
        };
        assert_eq!(record_rows(&none_held)[3].value, "off while following");
        assert_eq!(step(&none_held, 3, -1), None);
        assert_eq!(
            step(&st, 4, 1),
            Some(TimeAction::Transport(Command::SetTempo(121.0)))
        );
        // While the song's tempo points play, from the point in effect
        // (a ramp's start, not where it has glided to).
        st.tempo_here = Some(90.0);
        st.transport.bpm = 97.3;
        assert_eq!(
            step(&st, 4, 1),
            Some(TimeAction::Transport(Command::SetTempo(91.0)))
        );
        st.tempo_here = None;
        st.transport.bpm = MAX_BPM;
        assert_eq!(step(&st, 4, 1), None);
        // Pad pressure flips for the armed track, either way; with none
        // armed it shows a dash and does nothing.
        st.armed = Some(1);
        st.tracks[1].pressure = true;
        assert_eq!(record_rows(&st)[5].value, "Recorded");
        assert_eq!(
            step(&st, 5, -1),
            Some(TimeAction::RecordPressure {
                track: 1,
                on: false
            })
        );
        st.tracks[1].pressure = false;
        assert_eq!(record_rows(&st)[5].value, "Off");
        assert_eq!(
            step(&st, 5, 1),
            Some(TimeAction::RecordPressure { track: 1, on: true })
        );
        st.armed = None;
        assert_eq!(record_rows(&st)[5].value, "–");
        assert_eq!(step(&st, 5, 1), None);
        // Record quantise steps off, 1/4 ... 1/32 for the armed track, and
        // says why at its ends or with none armed.
        let why = |st: &TimeState, delta| {
            why_not(&ev(RECORD, EventKind::Step { index: 6, delta }), st).unwrap_or_default()
        };
        assert_eq!(record_rows(&st)[6].value, "–");
        assert_eq!(step(&st, 6, 1), None);
        assert!(why(&st, 1).starts_with("Arm a track first"));
        st.armed = Some(1);
        assert_eq!(record_rows(&st)[6].value, "Off");
        assert_eq!(step(&st, 6, -1), None);
        assert!(why(&st, -1).contains("off already"));
        assert_eq!(
            step(&st, 6, 1),
            Some(TimeAction::RecordQuantise {
                track: 1,
                grid: Some(PPQ)
            })
        );
        st.tracks[1].record_quantise = Some(PPQ / 4);
        assert_eq!(record_rows(&st)[6].value, "1/16");
        assert_eq!(
            step(&st, 6, 1),
            Some(TimeAction::RecordQuantise {
                track: 1,
                grid: Some(PPQ / 6)
            })
        );
        st.tracks[1].record_quantise = Some(PPQ);
        assert_eq!(
            step(&st, 6, -1),
            Some(TimeAction::RecordQuantise {
                track: 1,
                grid: None
            })
        );
        st.tracks[1].record_quantise = Some(PPQ / 8);
        assert_eq!(step(&st, 6, 1), None);
        assert!(why(&st, 1).contains("finest, 1/32"));
        // Launch: the project's line for tapped clips, stops and scenes,
        // from at once to 8 bars, whatever is armed.
        let why = |st: &TimeState, delta| {
            why_not(&ev(RECORD, EventKind::Step { index: 7, delta }), st).unwrap_or_default()
        };
        st.armed = None;
        assert_eq!(record_rows(&st)[7].value, "1 bar");
        assert_eq!(
            step(&st, 7, 1),
            Some(TimeAction::LaunchGrid(LaunchGrid::Bars(2)))
        );
        assert_eq!(
            step(&st, 7, -1),
            Some(TimeAction::LaunchGrid(LaunchGrid::Ticks(PPQ * 2)))
        );
        st.launch = LaunchGrid::Now;
        assert_eq!(record_rows(&st)[7].value, "at once");
        assert_eq!(step(&st, 7, -1), None);
        assert!(why(&st, -1).contains("at once already"));
        st.launch = LaunchGrid::Bars(8);
        assert_eq!(record_rows(&st)[7].value, "8 bars");
        assert_eq!(step(&st, 7, 1), None);
        assert!(why(&st, 1).contains("longest, 8 bars"));
        // One set in the file (3 bars) shows as it is and steps from 1 bar.
        st.launch = LaunchGrid::Bars(3);
        assert_eq!(record_rows(&st)[7].value, "3 bars");
        assert_eq!(
            step(&st, 7, 1),
            Some(TimeAction::LaunchGrid(LaunchGrid::Bars(2)))
        );
        // Transpose and Follows (sequencer S4, live transpose): the armed
        // track's, a semitone (or a note of the key) a step, within two
        // octaves; a dash and why not on a kit's track or with none armed.
        let why = |st: &TimeState, index, delta| {
            why_not(&ev(RECORD, EventKind::Step { index, delta }), st).unwrap_or_default()
        };
        assert_eq!(RECORD_ROWS[TRANSPOSE_ROW], "Transpose");
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "–");
        assert!(why(&st, TRANSPOSE_ROW, 1).starts_with("Arm a track first"));
        st.armed = Some(1);
        st.tracks[1].melodic = false;
        assert_eq!(record_rows(&st)[FOLLOWS_ROW].value, "–");
        assert_eq!(step(&st, TRANSPOSE_ROW, 1), None);
        assert!(why(&st, TRANSPOSE_ROW, 1).contains("plays a kit"));
        st.tracks[1].melodic = true;
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "as written");
        assert_eq!(
            step(&st, TRANSPOSE_ROW, 1),
            Some(TimeAction::Transpose {
                track: 1,
                to: Transpose::semitones(1)
            })
        );
        st.tracks[1].transpose = Transpose {
            by: -3,
            in_key: true,
        };
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "-3 in key");
        assert_eq!(
            step(&st, TRANSPOSE_ROW, -1),
            Some(TimeAction::Transpose {
                track: 1,
                to: Transpose {
                    by: -4,
                    in_key: true
                }
            })
        );
        st.tracks[1].transpose = Transpose::semitones(MAX_TRANSPOSE);
        assert_eq!(record_rows(&st)[TRANSPOSE_ROW].value, "+24 st");
        assert_eq!(step(&st, TRANSPOSE_ROW, 1), None);
        assert!(why(&st, TRANSPOSE_ROW, 1).contains("highest, 24 semitones up"));
        assert_eq!(record_rows(&st)[FOLLOWS_ROW].value, "Off");
        assert_eq!(
            step(&st, FOLLOWS_ROW, -1),
            Some(TimeAction::FollowsTranspose { track: 1, on: true })
        );
        st.tracks[1].follows = true;
        assert_eq!(record_rows(&st)[FOLLOWS_ROW].value, "On");
    }

    /// The clip settings card: the focused clip's settings, each row
    /// stepping within its range and saying why at its ends.
    #[test]
    fn the_clip_card_steps_the_focused_clips_settings() {
        let mut st = state();
        // The armed track's playing clip, else its first; the first
        // track's while none is armed.
        assert_eq!(st.focus(), Some((2, 0)));
        st.armed = None;
        assert_eq!(st.focus(), Some((0, 0)));
        st.armed = Some(1);
        assert_eq!(st.focus(), Some((1, 1)));
        assert_eq!(panel_tabs(&st), ["Record", "Clip · B", "Scene"]);
        let rows: Vec<String> = clip_rows(&st)
            .iter()
            .map(|r| format!("{} {}", r.name, r.value))
            .collect();
        assert_eq!(
            rows,
            [
                "Length 7 steps",
                "Speed ×1",
                "Swing track",
                "Quantise off",
                "Strength —",
                "Follow off",
                "After —",
                "Launch project",
                "Mode Restart",
                "Program off",
                "Bank —",
                "Direction Forward",
                "Mutate —"
            ]
        );
        let step = |st: &TimeState, index: usize, delta: i32| {
            let e = ev(CLIP_CARD, EventKind::Step { index, delta });
            (action(&e, st), why_not(&e, st))
        };
        let set = |st: &TimeState, row: u8, f: fn(&mut ClipSettings)| {
            let mut s = st.clip(1, 1).unwrap().settings;
            f(&mut s);
            (
                Some(TimeAction::ClipSettings {
                    track: 1,
                    slot: 1,
                    row,
                    settings: s,
                }),
                None,
            )
        };
        fn cs(st: &mut TimeState) -> &mut ClipSettings {
            &mut st.tracks[1].slots[1].as_mut().unwrap().settings
        }
        let ends = |got: (Option<TimeAction>, Option<String>), says: &str| {
            assert_eq!(got.0, None);
            assert!(
                got.1.as_deref().is_some_and(|w| w.contains(says)),
                "{got:?}"
            );
        };
        // Length, a sixteenth at a time.
        let length = |steps| {
            (
                Some(TimeAction::ClipLength {
                    track: 1,
                    slot: 1,
                    steps,
                    copy: false,
                }),
                None,
            )
        };
        assert_eq!(step(&st, 0, 1), length(8));
        assert_eq!(step(&st, 0, -1), length(6));
        st.tracks[1].slots[1].as_mut().unwrap().length = CARD_STEP;
        ends(step(&st, 0, -1), "shortest, 1 step");
        // Speed, from x1/8 to x4; one set elsewhere steps from the nearest.
        assert_eq!(
            step(&st, 1, 1),
            set(&st, 1, |s| s.speed = Speed::new(4, 3).unwrap())
        );
        assert_eq!(
            step(&st, 1, -1),
            set(&st, 1, |s| s.speed = Speed::new(3, 4).unwrap())
        );
        cs(&mut st).speed = Speed::new(3, 8).unwrap();
        assert_eq!(
            step(&st, 1, 1),
            set(&st, 1, |s| s.speed = Speed::new(1, 2).unwrap())
        );
        cs(&mut st).speed = Speed::new(1, 8).unwrap();
        ends(step(&st, 1, -1), "slowest");
        // Swing: the track's, then 50 to 75 %.
        ends(step(&st, 2, -1), "the track's already");
        assert_eq!(
            step(&st, 2, 1),
            set(&st, 2, |s| s.swing = Swing::new(240, 50))
        );
        cs(&mut st).swing = Swing::new(480, 75);
        assert_eq!(clip_rows(&st)[2].value, "75% 1/8");
        ends(step(&st, 2, 1), "at its most, 75 %");
        assert_eq!(
            step(&st, 2, -1),
            set(&st, 2, |s| s.swing = Swing::new(480, 74))
        );
        // Quantise: off, then 1/4 to 1/32 at full strength; strength only
        // while it is on, in tens.
        ends(step(&st, 4, 1), "Turn Quantise on first");
        ends(step(&st, 3, -1), "off already");
        assert_eq!(
            step(&st, 3, 1),
            set(&st, 3, |s| s.quantise = Quantise::new(960, 100))
        );
        cs(&mut st).quantise = Quantise::new(240, 75);
        assert_eq!(clip_rows(&st)[3].value, "1/16");
        assert_eq!(clip_rows(&st)[4].value, "75 %");
        assert_eq!(
            step(&st, 3, -1),
            set(&st, 3, |s| s.quantise = Quantise::new(320, 75))
        );
        assert_eq!(
            step(&st, 4, 1),
            set(&st, 4, |s| s.quantise = Quantise::new(240, 80))
        );
        assert_eq!(
            step(&st, 4, -1),
            set(&st, 4, |s| s.quantise = Quantise::new(240, 70))
        );
        cs(&mut st).quantise = Quantise::new(120, 100);
        ends(step(&st, 3, 1), "finest, 1/32");
        ends(step(&st, 4, 1), "full already");
        // Follow: off, then an action after one pass of the clip (7
        // sixteenths: 1 bar); After only while it is on.
        ends(step(&st, 6, 1), "Turn Follow on first");
        ends(step(&st, 5, -1), "off already");
        cs(&mut st).speed = Speed::NORMAL;
        st.tracks[1].slots[1].as_mut().unwrap().length = 1680;
        assert_eq!(
            step(&st, 5, 1),
            set(&st, 5, |s| s.follow =
                Some(Follow::after(1, FollowAction::Next)))
        );
        cs(&mut st).follow = Some(Follow::after(4, FollowAction::Next));
        assert_eq!(clip_rows(&st)[5].value, "Next");
        assert_eq!(clip_rows(&st)[6].value, "4 bars");
        assert_eq!(
            step(&st, 5, 1),
            set(&st, 5, |s| s.follow =
                Some(Follow::after(4, FollowAction::Previous)))
        );
        assert_eq!(
            step(&st, 6, 1),
            set(&st, 6, |s| s.follow =
                Some(Follow::after(5, FollowAction::Next)))
        );
        cs(&mut st).follow = Some(Follow::after(1, FollowAction::Stop));
        ends(step(&st, 5, 1), "last action, Stop");
        ends(step(&st, 6, -1), "soonest");
        // Launch: the project's, then the clip's own, at once to 8 bars.
        ends(step(&st, 7, -1), "the project's already (1 bar)");
        assert_eq!(
            step(&st, 7, 1),
            set(&st, 7, |s| s.launch = Some(LaunchGrid::Now))
        );
        cs(&mut st).launch = Some(LaunchGrid::Ticks(PPQ));
        assert_eq!(clip_rows(&st)[7].value, "1/4");
        assert_eq!(
            step(&st, 7, 1),
            set(&st, 7, |s| s.launch = Some(LaunchGrid::Ticks(PPQ * 2)))
        );
        assert_eq!(
            step(&st, 7, -1),
            set(&st, 7, |s| s.launch = Some(LaunchGrid::Ticks(PPQ / 2)))
        );
        cs(&mut st).launch = Some(LaunchGrid::Now);
        assert_eq!(clip_rows(&st)[7].value, "at once");
        assert_eq!(step(&st, 7, -1), set(&st, 7, |s| s.launch = None));
        cs(&mut st).launch = Some(LaunchGrid::Bars(8));
        assert_eq!(clip_rows(&st)[7].value, "8 bars");
        ends(step(&st, 7, 1), "longest, 8 bars");
        // Mode: Restart, Legato, Once.
        ends(step(&st, 8, -1), "first, Restart");
        assert_eq!(step(&st, 8, 1), set(&st, 8, |s| s.legato = true));
        cs(&mut st).legato = true;
        assert_eq!(clip_rows(&st)[8].value, "Legato");
        assert_eq!(step(&st, 8, -1), set(&st, 8, |s| s.legato = false));
        assert_eq!(
            step(&st, 8, 1),
            set(&st, 8, |s| {
                s.legato = false;
                s.once = true;
            })
        );
        cs(&mut st).legato = false;
        cs(&mut st).once = true;
        assert_eq!(clip_rows(&st)[8].value, "Once");
        ends(step(&st, 8, 1), "last, Once: the clip plays one pass");
        assert_eq!(
            step(&st, 8, -1),
            set(&st, 8, |s| {
                s.once = false;
                s.legato = true;
            })
        );
        cs(&mut st).once = false;
        // Program (S5): off, then 1 to 128; Bank only with a program: off,
        // then 0/0 up to 127/127, SHIFT a whole CC 0.
        ends(step(&st, 9, -1), "off already");
        ends(step(&st, 10, 1), "Set Program first");
        let prog = |program, bank| Some(ClipProgram { program, bank });
        assert_eq!(
            step(&st, 9, 1),
            set(&st, 9, |s| s.program = Some(ClipProgram {
                program: 0,
                bank: None
            }))
        );
        cs(&mut st).program = prog(4, None);
        assert_eq!(clip_rows(&st)[9].value, "5");
        assert_eq!(clip_rows(&st)[10].value, "off");
        assert_eq!(
            step(&st, 9, -1),
            set(&st, 9, |s| s.program = Some(ClipProgram {
                program: 3,
                bank: None
            }))
        );
        ends(step(&st, 10, -1), "Bank is off already");
        assert_eq!(
            step(&st, 10, 1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 4,
                bank: Some(0)
            }))
        );
        cs(&mut st).program = prog(127, Some(130));
        assert_eq!(clip_rows(&st)[9].value, "128");
        assert_eq!(clip_rows(&st)[10].value, "1/2");
        ends(step(&st, 9, 1), "last, 128");
        let shifted = |st: &TimeState, delta| (card_action(st, 10, delta, true).ok(), None);
        assert_eq!(
            shifted(&st, 1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: Some(258)
            }))
        );
        assert_eq!(
            shifted(&st, -1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: Some(2)
            }))
        );
        cs(&mut st).program = prog(127, Some(5));
        assert_eq!(
            shifted(&st, -1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: Some(0)
            }))
        );
        cs(&mut st).program = prog(127, Some(0));
        assert_eq!(
            step(&st, 10, -1),
            set(&st, 10, |s| s.program = Some(ClipProgram {
                program: 127,
                bank: None
            }))
        );
        cs(&mut st).program = prog(127, Some(ClipProgram::MAX_BANK));
        assert_eq!(clip_rows(&st)[10].value, "127/127");
        ends(step(&st, 10, 1), "last, 127/127");
        // Its bank is kept as the program steps, and goes when it turns off.
        cs(&mut st).program = prog(0, Some(9));
        assert_eq!(
            step(&st, 9, 1),
            set(&st, 9, |s| s.program = Some(ClipProgram {
                program: 1,
                bank: Some(9)
            }))
        );
        assert_eq!(step(&st, 9, -1), set(&st, 9, |s| s.program = None));
        cs(&mut st).program = None;
        // A loop cell: only Follow, After and Launch.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        ends(step(&st, 1, 1), "loop cell");
        ends(step(&st, 8, 1), "loop cell");
        ends(step(&st, 9, 1), "loop cell");
        ends(step(&st, 10, 1), "loop cell");
        assert!(step(&st, 6, 1).0.is_some());
        assert!(step(&st, 7, -1).0.is_some());
        // A settings take in the Takes list.
        let row = TakeRow {
            number: 4,
            at: Some((1, 1)),
            settings: true,
            ..TakeRow::of(&Take::default())
        };
        assert_eq!(row.short(), "settings 4");
        assert_eq!(take_title(&st, &row), "Settings 4 · Bass / 2");
        assert_eq!(take_detail(&st, &row), "clip settings");
        // No clip on the armed track: no rows, and it says so.
        st.tracks[1].slots = vec![None; SLOTS];
        assert_eq!(st.focus(), None);
        assert!(clip_rows(&st).is_empty());
        assert_eq!(panel_tabs(&st)[1], "Clip");
        ends(step(&st, 0, 1), "No clip to set");
    }

    /// SHIFT + the card's arrows: Length doubles with its notes repeated or
    /// halves, Speed doubles or halves; the other rows step as without it.
    /// The Direction row (S6) steps Forward, Reverse, Ping-pong and
    /// Random, but moves only a clip of two whole 1/16 steps or more.
    #[test]
    fn the_clip_cards_direction_row_steps_the_play_direction() {
        let mut st = state();
        st.armed = Some(1);
        let step = |st: &TimeState, delta| match card_step(st, CLIP_DIRECTION, delta) {
            Ok(TimeAction::ClipSettings { row, settings, .. }) => {
                assert_eq!(usize::from(row), CLIP_DIRECTION);
                Ok(settings.direction)
            }
            other => Err(format!("{other:?}")),
        };
        let set = |st: &mut TimeState, d| {
            st.tracks[1].slots[1].as_mut().unwrap().settings.direction = d;
        };
        assert_eq!(step(&st, 1), Ok(Direction::Reverse));
        assert!(step(&st, -1).unwrap_err().contains("its first, Forward"));
        set(&mut st, Direction::PingPong);
        assert_eq!(step(&st, 1), Ok(Direction::Random));
        assert_eq!(step(&st, -1), Ok(Direction::Reverse));
        set(&mut st, Direction::Random);
        assert!(step(&st, 1).unwrap_err().contains("its last, Random"));
        let shown = |st: &TimeState| clip_rows(st)[CLIP_DIRECTION].value.clone();
        assert_eq!(shown(&st), "Random");
        // A clip of one step plays forward: the row says "—", a step
        // puts it back to Forward, and then says why it can't move.
        st.tracks[1].slots[1].as_mut().unwrap().length = CARD_STEP;
        assert_eq!(shown(&st), "—");
        assert_eq!(step(&st, 1), Ok(Direction::Forward));
        set(&mut st, Direction::Forward);
        assert_eq!(shown(&st), "Forward");
        assert_eq!(
            step(&st, 1),
            Err(format!("{:?}", Err::<(), _>("Direction moves the clip's 1/16 steps: it needs a clip of 2 whole steps or more, so this one plays forward")))
        );
        // A loop cell plays its loop as recorded.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert!(step(&st, 1).unwrap_err().contains("loop cell"));
    }

    /// The card's Mutate row (S6): › a variation (SHIFT + › a bolder one),
    /// the pitch free to move on a melodic track; ‹ the original back once
    /// there is one. It reads "original", then "variant"; "—" with no
    /// notes or on a loop's cell.
    #[test]
    fn the_clip_cards_mutate_row_mutates_and_restores() {
        let mut st = state();
        st.armed = Some(1);
        st.tracks[1].slots[1].as_mut().unwrap().notes = 4;
        let shown = |st: &TimeState| clip_rows(st)[CLIP_MUTATE].value.clone();
        assert_eq!(clip_rows(&st).len(), CLIP_ROWS.len());
        assert_eq!(clip_rows(&st)[CLIP_MUTATE].name, "Mutate");
        assert_eq!(shown(&st), "original");
        let mutate = |more, melodic| {
            Ok(TimeAction::Mutate {
                track: 1,
                slot: 1,
                more,
                melodic,
            })
        };
        assert_eq!(
            card_action(&st, CLIP_MUTATE, 1, false),
            mutate(false, false)
        );
        assert_eq!(card_action(&st, CLIP_MUTATE, 1, true), mutate(true, false));
        st.tracks[1].melodic = true;
        assert_eq!(card_action(&st, CLIP_MUTATE, 1, true), mutate(true, true));
        assert!(card_action(&st, CLIP_MUTATE, -1, false)
            .unwrap_err()
            .contains("plays its original"));
        let c = st.tracks[1].slots[1].as_mut().unwrap();
        c.variation = Some(3);
        assert_eq!(shown(&st), "variant");
        assert_eq!(
            card_action(&st, CLIP_MUTATE, -1, true),
            Ok(TimeAction::Restore { track: 1, slot: 1 })
        );
        assert_eq!(card_action(&st, CLIP_MUTATE, 1, false), mutate(false, true));
        // No notes: nothing to vary, unless a variation is there to undo.
        st.tracks[1].slots[1].as_mut().unwrap().notes = 0;
        assert_eq!(shown(&st), "variant");
        st.tracks[1].slots[1].as_mut().unwrap().variation = None;
        assert_eq!(shown(&st), "—");
        assert!(card_action(&st, CLIP_MUTATE, 1, false)
            .unwrap_err()
            .contains("none yet"));
        // A loop cell plays its loop as recorded.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert_eq!(shown(&st), "—");
        assert!(card_action(&st, CLIP_MUTATE, 1, false)
            .unwrap_err()
            .contains("loop cell"));
    }

    #[test]
    fn shift_doubles_or_halves_the_cards_length_and_speed() {
        let mut st = state();
        st.armed = Some(1);
        // Bass's clip B: 7 sixteenths, at x1.
        let act = |st: &TimeState, index, delta| card_action(st, index, delta, true);
        assert_eq!(
            act(&st, 0, 1),
            Ok(TimeAction::ClipLength {
                track: 1,
                slot: 1,
                steps: 14,
                copy: true
            })
        );
        assert_eq!(
            act(&st, 0, -1),
            Ok(TimeAction::ClipLength {
                track: 1,
                slot: 1,
                steps: 3,
                copy: false
            })
        );
        let speed = |st: &TimeState, delta| match act(st, 1, delta) {
            Ok(TimeAction::ClipSettings {
                row: 1, settings, ..
            }) => Ok(speed_name(settings.speed)),
            other => Err(format!("{other:?}")),
        };
        assert_eq!(speed(&st, 1).as_deref(), Ok("×2"));
        assert_eq!(speed(&st, -1).as_deref(), Ok("×1/2"));
        let set = |st: &mut TimeState, n, d| {
            st.tracks[1].slots[1].as_mut().unwrap().settings.speed = Speed::new(n, d).unwrap();
        };
        set(&mut st, 3, 4);
        assert_eq!(speed(&st, 1).as_deref(), Ok("×3/2"));
        set(&mut st, 3, 1);
        assert!(speed(&st, 1).unwrap_err().contains("fastest"));
        set(&mut st, 1, 8);
        assert!(speed(&st, -1).unwrap_err().contains("slowest"));
        set(&mut st, 1, 3);
        assert!(speed(&st, -1).unwrap_err().contains("not a speed"));
        // The other rows as without SHIFT.
        assert_eq!(act(&st, 3, 1), card_step(&st, 3, 1));
        // A clip a sixteenth long can't halve.
        st.tracks[1].slots[1].as_mut().unwrap().length = CARD_STEP;
        assert!(act(&st, 0, -1).unwrap_err().contains("can't halve"));
        // A loop cell: neither.
        st.tracks[1].slots[1].as_mut().unwrap().looped = Some(0);
        assert!(act(&st, 1, 1).unwrap_err().contains("loop cell"));
    }

    /// COPY held: the hint says what a tap copies or pastes, and a paste
    /// is named in the takes list.
    #[test]
    fn copy_says_what_a_tap_copies_or_pastes() {
        let mut st = state();
        assert!(copy_hint(&st).starts_with("COPY held: tap a clip to copy it"));
        st.clipboard = Some("Verse B".into());
        assert!(copy_hint(&st).contains("a copy of Verse B goes there"));
        let row = TakeRow {
            number: 5,
            at: Some((1, 2)),
            made: true,
            pasted: true,
            ..TakeRow::of(&Take::default())
        };
        assert_eq!(row.short(), "paste 5");
        assert_eq!(take_title(&st, &row), "Paste 5 · Bass / 3");
        assert_eq!(take_detail(&st, &row), "a copy");
    }

    /// The Record panel's Clip tab swaps the clip settings card in for the
    /// record settings, on the grid and the song alike, and the wheel
    /// steps it.
    #[test]
    fn the_record_panels_tabs_swap_in_the_clip_card() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        page.update(&mut ui, &st);
        assert_eq!(page.wheel_list(&ui), RECORD);
        let record = ui.get::<OptionList>(RECORD).unwrap();
        assert_eq!((record.tab, record.tabs.clone()), (0, panel_tabs(&st)));
        // The Clip tab.
        assert!(page.page_tracks(&ev(RECORD, EventKind::Tab(1)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_CARD_LAYOUT));
        assert_eq!(page.wheel_list(&ui), CLIP_CARD);
        let card = ui.get::<OptionList>(CLIP_CARD).unwrap();
        assert_eq!(card.tab, 1);
        assert_eq!(card.tabs[1], "Clip · A");
        assert_eq!(card.rows, clip_rows(&st));
        // The Song view keeps it, and its wheel still scrolls the song.
        st.song_view = true;
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(SONG_CARD_LAYOUT));
        assert_eq!(page.scroll(&ui, 2), None);
        assert_eq!(page.first_bar, 2);
        // Another clip focused: the card follows.
        st.armed = Some(1);
        page.update(&mut ui, &st);
        assert_eq!(ui.get::<OptionList>(CLIP_CARD).unwrap().tabs[1], "Clip · B");
        // The Record tab again.
        assert!(page.page_tracks(&ev(CLIP_CARD, EventKind::Tab(0)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(SONG_LAYOUT));
        assert_eq!(page.wheel_list(&ui), RECORD);
    }

    /// The scene editor (S4, part 7): the Record panel's third tab, its
    /// rows for the scene in focus, and what each row's – / + asks for.
    #[test]
    fn the_scene_tab_steps_the_scene_in_focus() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        st.scene_focus = 1;
        page.update(&mut ui, &st);
        assert!(page.page_tracks(&ev(RECORD, EventKind::Tab(2)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_SCENE_LAYOUT));
        assert_eq!(page.wheel_list(&ui), SCENE_CARD);
        let card = ui.get::<OptionList>(SCENE_CARD).unwrap();
        assert_eq!((card.tab, card.tabs[2].as_str()), (2, "Scene"));
        let shown = |st: &TimeState| -> Vec<String> {
            scene_rows(st)
                .iter()
                .map(|r| format!("{} {}", r.name, r.value))
                .collect()
        };
        assert_eq!(
            shown(&st),
            [
                "Scene 2",
                "Follow off",
                "After —",
                "Chance —",
                "Or —",
                "Mutes as they are",
                "Program off",
                "Bank —",
                "Sent to none"
            ]
        );
        let step = |st: &TimeState, row: usize, delta: i32| {
            action(&ev(SCENE_CARD, EventKind::Step { index: row, delta }), st)
        };
        let set = |a: Option<TimeAction>| match a {
            Some(TimeAction::SceneSettings {
                row: 1, settings, ..
            }) => settings,
            other => panic!("{other:?}"),
        };
        // ‹ › on the Scene row: the scene in focus.
        assert_eq!(step(&st, 0, -1), Some(TimeAction::SceneFocus(0)));
        st.scene_focus = 0;
        assert_eq!(step(&st, 0, -1), None);
        assert_eq!(
            why_not(
                &ev(
                    SCENE_CARD,
                    EventKind::Step {
                        index: 0,
                        delta: -1
                    }
                ),
                &st
            )
            .as_deref(),
            Some("Scene 1 is the first")
        );
        st.scene_focus = 1;
        // Follow on: Next, after the scene's longest clip (B, under a
        // bar: 1 bar).
        st.scene = set(step(&st, 1, 1));
        assert_eq!(st.scene.follow, Some(Follow::after(1, FollowAction::Next)));
        assert_eq!(
            shown(&st)[1..5],
            ["Follow Next", "After 1 bar", "Chance 100 %", "Or —"]
        );
        // Or waits for a chance under 100 %.
        assert_eq!(step(&st, 4, 1), None);
        st.scene = set(step(&st, 3, -1));
        assert_eq!(st.scene.follow.map(|f| f.chance), Some(90));
        st.scene = set(step(&st, 4, 1));
        assert_eq!(st.scene.follow.map(|f| f.other), Some(OTHERS[1]));
        // Mutes: none to begin with, then – takes them off again.
        st.scene = set(step(&st, 5, 1));
        assert_eq!(st.scene.mutes, Some(0));
        assert_eq!(shown(&st)[5], "Mutes none");
        st.tracks[0].mute = true;
        st.tracks[2].mute = true;
        assert_eq!(
            action(&ev(SCENE_CARD, EventKind::Toggle(SCENE_MUTES_NOW)), &st)
                .map(|a| set(Some(a)).mutes),
            Some(Some(0b101))
        );
        st.scene.mutes = Some(0b101);
        assert_eq!(shown(&st)[5], "Mutes Drums, Stab");
        st.scene = set(step(&st, 5, -1));
        assert_eq!(st.scene.mutes, None);
        // Follow off again.
        st.scene = set(step(&st, 1, -1));
        assert_eq!(st.scene.follow, None);
        // A name shows by the number.
        st.scene.name = "Drop".into();
        assert_eq!(shown(&st)[0], "Scene 2 · Drop");
        // The Record tab again.
        assert!(page.page_tracks(&ev(SCENE_CARD, EventKind::Tab(0)), &st));
        page.update(&mut ui, &st);
        assert_eq!(ui.layout_of(CLIPS_PAGE), Some(CLIPS_LAYOUT));
    }

    /// The scene editor's Program and Bank (S5): the program change the
    /// scene sends the armed track, off then 1 to 128, its bank with it
    /// (SHIFT steps a whole CC 0); Sent to lists every track's.
    #[test]
    fn the_scene_tab_sets_the_armed_tracks_program_change() {
        let mut st = state();
        st.scene_focus = 1;
        let shown = |st: &TimeState| -> Vec<String> {
            scene_rows(st)[SCENE_PROGRAM..]
                .iter()
                .map(|r| format!("{} {}", r.name, r.value))
                .collect()
        };
        let step = |st: &TimeState, row: usize, delta: i32| {
            action(&ev(SCENE_CARD, EventKind::Step { index: row, delta }), st)
        };
        let why = |st: &TimeState, row: usize, delta: i32| {
            why_not(&ev(SCENE_CARD, EventKind::Step { index: row, delta }), st)
        };
        let set = |a: Option<TimeAction>| match a {
            Some(TimeAction::SceneSettings {
                row: 1,
                card_row,
                settings,
            }) => {
                assert!(
                    usize::from(card_row) == SCENE_PROGRAM || usize::from(card_row) == SCENE_BANK
                );
                settings
            }
            other => panic!("{other:?}"),
        };
        // Off, and no bank before a program.
        assert_eq!(step(&st, SCENE_PROGRAM, -1), None);
        assert!(why(&st, SCENE_PROGRAM, -1).unwrap().contains("off already"));
        assert_eq!(step(&st, SCENE_BANK, 1), None);
        assert!(why(&st, SCENE_BANK, 1)
            .unwrap()
            .starts_with("Set Program first"));
        // + : program 1, then 2.
        st.scene = set(step(&st, SCENE_PROGRAM, 1));
        st.scene = set(step(&st, SCENE_PROGRAM, 1));
        assert_eq!(
            st.scene.program(2),
            Some(ClipProgram {
                program: 1,
                bank: None
            })
        );
        assert_eq!(shown(&st), ["Program 2", "Bank off", "Sent to Stab"]);
        // Bank 0/0, SHIFT a whole CC 0 on, then back to off.
        st.scene = set(step(&st, SCENE_BANK, 1));
        assert_eq!(shown(&st)[1], "Bank 0/0");
        st.scene = set(scene_action(&st, SCENE_BANK, 1, true).ok());
        assert_eq!(shown(&st)[1], "Bank 1/0");
        st.scene = set(scene_action(&st, SCENE_BANK, -1, true).ok());
        st.scene = set(step(&st, SCENE_BANK, -1));
        assert_eq!(shown(&st)[1], "Bank off");
        // Another track armed: its own, and Sent to lists them all.
        st.armed = Some(0);
        assert_eq!(shown(&st)[..2], ["Program off", "Bank —"]);
        st.scene = set(step(&st, SCENE_PROGRAM, 1));
        assert_eq!(
            shown(&st)[..3],
            ["Program 1", "Bank off", "Sent to Drums, Stab"]
        );
        st.scene.set_program(
            1,
            Some(ClipProgram {
                program: 127,
                bank: None,
            }),
        );
        assert_eq!(shown(&st)[2], "Sent to Drums +2");
        st.armed = Some(1);
        assert_eq!(step(&st, SCENE_PROGRAM, 1), None);
        assert_eq!(
            why(&st, SCENE_PROGRAM, 1).as_deref(),
            Some("Program is at its last, 128")
        );
        // Off again: – from 1.
        st.armed = Some(0);
        st.scene = set(step(&st, SCENE_PROGRAM, -1));
        assert_eq!(st.scene.program(0), None);
        // Nothing armed: nothing to set, and it says so.
        st.armed = None;
        assert_eq!(shown(&st)[..2], ["Program —", "Bank —"]);
        assert!(why(&st, SCENE_PROGRAM, 1)
            .unwrap()
            .starts_with("Arm a track first"));
        assert!(why(&st, 8, 1)
            .unwrap()
            .starts_with("Arm a track and set Program"));
    }

    /// Save state and Reload on the clip card, Reload clip on the morph
    /// bar (S4): lit once the focused clip has a saved state, outlined
    /// while it waits, and NEXT names it in Perform.
    #[test]
    fn save_state_and_reload_show_on_the_card_and_the_morph_bar() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        let (track, slot) = st.focus().unwrap();
        let focus = (track, slot);
        page.update(&mut ui, &st);
        let card = ui.get::<OptionList>(CLIP_CARD).unwrap();
        assert_eq!(
            card.buttons,
            [
                OptionButton::new("Save state", true),
                OptionButton::new("Reload", false)
            ]
        );
        let morph = ui.get::<MorphBar>(MORPH).unwrap();
        assert!(!morph.reload && !morph.reload_waiting);
        let save = ev(CLIP_CARD, EventKind::Toggle(0));
        let card_reload = ev(CLIP_CARD, EventKind::Toggle(1));
        let morph_reload = ev(MORPH, EventKind::Tab(3));
        assert_eq!(
            action(&save, &st),
            Some(TimeAction::SaveState { track, slot })
        );
        assert_eq!(action(&card_reload, &st), None);
        assert_eq!(action(&morph_reload, &st), None);
        assert!(why_not(&morph_reload, &st)
            .unwrap()
            .starts_with("Save state on the Clip card first"));
        // Saved: both reloads are lit.
        st.saved = vec![focus];
        page.update(&mut ui, &st);
        assert!(ui.get::<OptionList>(CLIP_CARD).unwrap().buttons[1].enabled);
        assert!(ui.get::<MorphBar>(MORPH).unwrap().reload);
        let reload = Some(TimeAction::Reload { track, slot });
        assert_eq!(action(&card_reload, &st), reload);
        assert_eq!(action(&morph_reload, &st), reload);
        // Waiting for bar 3: outlined, and NEXT says so in Perform.
        st.reloads = vec![(track, slot, 2 * BAR)];
        st.transport.state = State::Playing;
        st.transport.position = 2 * BAR - 480;
        st.cues.clear();
        st.scene_cued = None;
        page.update(&mut ui, &st);
        assert!(ui.get::<OptionList>(CLIP_CARD).unwrap().buttons[1].waiting);
        assert!(ui.get::<MorphBar>(MORPH).unwrap().reload_waiting);
        assert_eq!(st.next_reading(), None, "Make");
        st.perform = true;
        assert_eq!(st.next_reading().as_deref(), Some("A reloads in 1 beat"));
        // No clip in focus: no buttons.
        st.armed = Some(9);
        assert!(card_buttons(&st).is_empty());
    }

    fn project() -> Project {
        let mut p = Project::new();
        for (i, name) in ["Drums", "Bass", "Stab"].iter().enumerate() {
            let mut t = Track::new(name, i as u16);
            t.slots[0] = Some(Clip::new("A", 2 * BAR));
            t.slots[1] = Some(Clip::new("B", 1680));
            p.tracks.push(t);
        }
        p
    }

    #[test]
    fn while_playing_launches_and_stops_wait_as_cues() {
        let st = state();
        let (p, t) = (project(), st.transport);
        let act = |l: &[Launch], c: &[Cue], a: TimeAction| clips_after(&p, l, c, &a, &t);
        let (l, c) = act(&st.launches, &[], TimeAction::Launch { track: 0, slot: 1 });
        assert_eq!(
            (l.as_slice(), c.as_slice()),
            (
                st.launches.as_slice(),
                &[Cue {
                    track: 0,
                    slot: Some(1)
                }][..]
            ),
            "the launches stay until the bar line"
        );
        // A stop on the same track takes the launch's place; another track
        // gets its own cue.
        let (l, c) = act(&l, &c, TimeAction::StopClip(0));
        assert_eq!(
            c,
            vec![Cue {
                track: 0,
                slot: None
            }]
        );
        let (l, c) = act(&l, &c, TimeAction::Launch { track: 2, slot: 0 });
        assert_eq!(
            c,
            vec![
                Cue {
                    track: 0,
                    slot: None
                },
                Cue {
                    track: 2,
                    slot: Some(0)
                }
            ]
        );
        let (l, c) = act(&l, &c, TimeAction::Uncue(0));
        assert_eq!(
            c,
            vec![Cue {
                track: 2,
                slot: Some(0)
            }]
        );
        // A new clip launches at once, and its track's cue goes.
        let (l, c) = act(&l, &c, TimeAction::NewClip { track: 2, slot: 4 });
        assert_eq!(
            (l.last(), c.len()),
            (
                Some(&Launch {
                    track: 2,
                    slot: 4,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                }),
                0
            )
        );
        assert_eq!(act(&l, &c, TimeAction::Undo), (l.clone(), c.clone()));
    }

    #[test]
    fn while_stopped_a_launch_happens_at_once_and_plays_wherever_the_song_goes() {
        let st = state();
        let p = project();
        let t = TransportInfo {
            state: State::Stopped,
            position: 3 * BAR + 100,
            ..st.transport
        };
        let cues = [Cue {
            track: 0,
            slot: Some(1),
        }];
        let (l, c) = clips_after(
            &p,
            &st.launches,
            &cues,
            &TimeAction::Launch { track: 0, slot: 1 },
            &t,
        );
        assert!(c.is_empty());
        let origin = anchored(3 * BAR, 1680, Speed::NORMAL);
        assert_eq!(
            l,
            vec![
                Launch {
                    track: 1,
                    slot: 1,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                },
                Launch {
                    track: 0,
                    slot: 1,
                    origin,
                    start: 3 * BAR,
                }
            ]
        );
        let (l, _) = clips_after(&p, &l, &[], &TimeAction::StopClip(1), &t);
        assert_eq!(
            l,
            vec![Launch {
                track: 0,
                slot: 1,
                origin,
                start: 3 * BAR,
            }]
        );
    }

    #[test]
    fn cued_cells_show_where_they_start_and_taps_take_cues_back() {
        let mut st = state();
        st.cues = vec![
            Cue {
                track: 0,
                slot: Some(1),
            },
            Cue {
                track: 2,
                slot: None,
            },
        ];
        st.launches.push(Launch {
            track: 2,
            slot: 0,
            origin: 0,
            start: 0,
        });
        let q = cell_of(&st, 0, 1);
        assert_eq!((q.state, q.foot.as_str()), (CellState::Queued, "from 6.1"));
        let a = cell_of(&st, 0, 0);
        assert_eq!(
            (a.state, a.detail.as_str()),
            (CellState::Playing, "until 6.1")
        );
        let s = cell_of(&st, 2, 0);
        assert_eq!(
            (s.state, s.detail.as_str()),
            (CellState::Playing, "stops 6.1")
        );
        // In a loop of bars 4 and 5, the next bar line played is bar 4's.
        st.transport.loop_range = Some((3 * BAR, 5 * BAR));
        assert_eq!(cell_of(&st, 0, 1).foot, "from 4.1");
        assert_eq!(cue_tick(&st.transport, LaunchGrid::Bars(1)), 3 * BAR);
        st.transport.loop_range = None;
        // The cued clip's own launch quantise moves its label; the time
        // engine's line for a cue (one kept when Launch changed) wins.
        st.tracks[0].slots[1].as_mut().unwrap().settings.launch = Some(LaunchGrid::Ticks(PPQ * 2));
        assert_eq!(st.cue_line(0), LaunchGrid::Ticks(PPQ * 2));
        assert_eq!(cell_of(&st, 0, 1).foot, "from 5.3");
        st.cue_lines = vec![(0, LaunchGrid::Bars(2))];
        assert_eq!(cell_of(&st, 0, 1).foot, "from 7.1");
        assert_eq!(cell_of(&st, 0, 0).detail, "until 7.1");
        st.launch = LaunchGrid::Bars(4);
        assert_eq!(st.cue_line(2), LaunchGrid::Bars(4), "a stop: the project's");
        assert_eq!(cell_of(&st, 2, 0).detail, "stops 9.1");
        // A one-shot's stop lands where its pass ends, on no bar line.
        st.cue_lines = vec![(2, LaunchGrid::At(5 * BAR + 3 * Tick::from(PPQ)))];
        assert_eq!(cell_of(&st, 2, 0).detail, "stops 6.4");
        st.cue_lines.clear();
        st.launch = LaunchGrid::Bars(1);
        let tap = |track: usize, slot: usize| {
            action(&ev(CLIPS, EventKind::Select(track * SLOTS + slot)), &st)
        };
        assert_eq!(tap(0, 1), Some(TimeAction::Uncue(0)), "the cued clip");
        assert_eq!(
            tap(2, 0),
            Some(TimeAction::Uncue(2)),
            "the clip with a stop cued"
        );
        assert_eq!(tap(0, 0), Some(TimeAction::StopClip(0)), "the playing clip");
        assert_eq!(tap(2, 1), Some(TimeAction::Launch { track: 2, slot: 1 }));
    }

    /// The mute queue (S4): each waiting track's grid header says what
    /// it does and how soon, and the transport bar's WAITING reading
    /// names the tracks and their line.
    #[test]
    fn mutes_waiting_show_on_the_grid_and_the_transport_bar() {
        let mut st = state();
        assert_eq!((st.waiting_reading(), st.mute_waiting(1)), (None, None));
        assert_ne!(bar_of(&st).items[0].label, "WAITING");
        st.mute_wait = Some(MuteWait {
            want: vec![(1, true, false), (2, false, true)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        // At 5.2: three beats to bar 6.
        assert_eq!(st.mute_waiting(1).as_deref(), Some("mutes in 3 beats"));
        assert_eq!(st.mute_waiting(2).as_deref(), Some("solo in 3 beats"));
        assert_eq!(st.mute_waiting(0), None);
        assert_eq!(st.waiting_reading().as_deref(), Some("Bass, Stab at 6.1"));
        let item = &bar_of(&st).items[0];
        assert_eq!(
            (item.label.as_str(), item.value.as_str(), item.tone),
            ("WAITING", "Bass, Stab at 6.1", Tone::Warn)
        );
        let g = grid_tracks(&st);
        assert_eq!(g[1].waiting.as_deref(), Some("mutes in 3 beats"));
        assert_eq!(g[0].waiting, None);
        // A longer line counts bars; more than two tracks are counted.
        st.mute_wait = Some(MuteWait {
            want: vec![(0, true, false), (1, true, false), (2, true, false)],
            ..MuteWait::new(LaunchGrid::Bars(2))
        });
        assert_eq!(st.mute_waiting(0).as_deref(), Some("mutes in 2 bars"));
        assert_eq!(
            st.waiting_reading().as_deref(),
            Some("Drums, Bass +1 at 7.1")
        );
        // The kit's pads wait with them, named by number; alone, too.
        st.mute_wait = Some(MuteWait {
            want: vec![(1, true, false)],
            pads: vec![(2, true, false)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        assert_eq!(st.waiting_reading().as_deref(), Some("Bass, Pad 3 at 6.1"));
        st.mute_wait = Some(MuteWait {
            pads: vec![(0, false, true)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        assert_eq!(st.waiting_reading().as_deref(), Some("Pad 1 at 6.1"));
        assert_eq!(st.mute_waiting(0), None, "a pad is no track's");
        // Solo: the soloed track is outlined, the others are silent.
        st.mute_wait = None;
        st.tracks[2].solo = true;
        st.tracks[0].silent = true;
        let g = grid_tracks(&st);
        assert!(g[2].solo && !g[2].mute && g[0].mute);
    }

    #[test]
    fn perform_shows_big_rows_countdowns_and_what_comes_next() {
        // Make: eight compact rows; Perform: five big ones.
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut st = state();
        page.update(&mut ui, &st);
        assert_eq!(page.visible_rows(&ui), 8);
        st.perform = true;
        page.update(&mut ui, &st);
        assert_eq!(page.visible_rows(&ui), 5);
        // A queued clip says where it starts; in Perform it counts down
        // too: at 5.2, a quarter of the bar to 6.1 has gone.
        st.perform = false;
        let c = cell_of(&st, 1, 1);
        assert_eq!(
            (c.state, c.foot.as_str(), c.progress),
            (CellState::Queued, "from 6.1", 0.0)
        );
        st.perform = true;
        let c = cell_of(&st, 1, 1);
        assert_eq!((c.foot.as_str(), c.progress), ("from 6.1", 0.25));
        // NEXT: nothing coming.
        assert_eq!(st.next_reading(), None);
        // Clips cued: the first by name and how many more.
        st.cues = vec![Cue {
            track: 2,
            slot: Some(1),
        }];
        assert_eq!(st.next_reading().as_deref(), Some("B in 3 beats"));
        let c = cell_of(&st, 2, 1);
        assert_eq!((c.foot.as_str(), c.progress), ("from 6.1", 0.25));
        st.cues.push(Cue {
            track: 0,
            slot: None,
        });
        assert_eq!(st.next_reading().as_deref(), Some("B +1 in 3 beats"));
        st.cues = vec![Cue {
            track: 0,
            slot: None,
        }];
        assert_eq!(st.next_reading().as_deref(), Some("Drums stops in 3 beats"));
        // A scene launched names the scene.
        st.scenes = vec![String::new(), "Drop".into()];
        st.scene_cued = Some(1);
        assert_eq!(st.next_reading().as_deref(), Some("Drop in 3 beats"));
        let item = &bar_of(&st).items[0];
        assert_eq!(
            (item.label.as_str(), item.value.as_str(), item.tone),
            ("NEXT", "Drop in 3 beats", Tone::Accent)
        );
        // The scene playing's follow action, bars rounded up.
        st.cues.clear();
        st.scene_cued = None;
        let follow = |action, other, chance| FollowNext {
            at: 8 * BAR,
            row: 0,
            follow: Follow {
                bars: 4,
                action,
                other,
                chance,
            },
        };
        st.follow_next = Some(follow(FollowAction::Next, FollowAction::Stay, 100));
        assert_eq!(st.next_reading().as_deref(), Some("Drop in 4 bars"));
        st.follow_next = Some(follow(FollowAction::Next, FollowAction::Stay, 70));
        assert_eq!(st.next_reading().as_deref(), Some("Drop or stay in 4 bars"));
        st.follow_next = Some(follow(FollowAction::Again, FollowAction::Stay, 100));
        assert_eq!(st.next_reading().as_deref(), Some("row 1 again in 4 bars"));
        st.follow_next = Some(follow(FollowAction::Other, FollowAction::Stay, 100));
        assert_eq!(
            st.next_reading().as_deref(),
            Some("Another scene in 4 bars")
        );
        st.follow_next = Some(follow(FollowAction::Stay, FollowAction::Next, 100));
        assert_eq!(st.next_reading(), None);
        // Song mode: the next marker.
        st.follow_next = None;
        st.song_on = true;
        st.markers = vec![(2 * BAR, "Intro".into()), (6 * BAR, "Chorus".into())];
        assert_eq!(st.next_reading().as_deref(), Some("Chorus in 2 bars"));
        // WAITING goes before NEXT.
        st.mute_wait = Some(MuteWait {
            want: vec![(1, true, false)],
            ..MuteWait::new(LaunchGrid::Bars(1))
        });
        let items = bar_of(&st).items;
        assert_eq!(
            (items[0].label.as_str(), items[1].label.as_str()),
            ("WAITING", "NEXT")
        );
        // Nothing in Make, or stopped.
        st.perform = false;
        assert_eq!(st.next_reading(), None);
        st.perform = true;
        st.transport.state = State::Stopped;
        assert_eq!(st.next_reading(), None);
    }

    #[test]
    fn countdowns_round_up_to_beats_then_bars() {
        let m = Meter::default();
        let beat = m.beat_ticks();
        let bar = m.bar_ticks();
        assert_eq!(countdown(1, m), "1 beat");
        assert_eq!(countdown(beat + 1, m), "2 beats");
        assert_eq!(countdown(bar - 1, m), "4 beats");
        assert_eq!(countdown(bar, m), "1 bar");
        assert_eq!(countdown(bar + 1, m), "2 bars");
        assert_eq!(countdown(0, m), "1 beat");
    }

    #[test]
    fn a_launch_starts_on_the_next_bar_and_replaces_the_tracks_clip() {
        let st = state();
        let t = st.transport;
        let after = launches_after(&st.launches, &TimeAction::Launch { track: 0, slot: 1 }, &t);
        assert_eq!(
            after,
            vec![
                Launch {
                    track: 1,
                    slot: 1,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                },
                Launch {
                    track: 0,
                    slot: 1,
                    origin: 5 * BAR,
                    start: 5 * BAR,
                }
            ]
        );
        let stopped = launches_after(&after, &TimeAction::StopClip(1), &t);
        assert_eq!(
            stopped,
            vec![Launch {
                track: 0,
                slot: 1,
                origin: 5 * BAR,
                start: 5 * BAR,
            }]
        );
        // On a bar line: now. Stopped: from the start of the bar.
        let on_bar = TransportInfo {
            position: 3 * BAR,
            ..t
        };
        assert_eq!(launch_origin(&on_bar), 3 * BAR);
        let stopped = TransportInfo {
            state: State::Stopped,
            position: 3 * BAR + 100,
            ..t
        };
        assert_eq!(launch_origin(&stopped), 3 * BAR);
        // Counting in from bar 0: bar 1.
        let count = TransportInfo {
            state: State::CountIn,
            position: -1000,
            ..t
        };
        assert_eq!(launch_origin(&count), 0);
        assert_eq!(
            launches_after(&st.launches, &TimeAction::Undo, &t),
            st.launches
        );
    }

    #[test]
    fn takes_list_newest_first_with_the_undone_dimmed() {
        let mut st = state();
        let row = |n: u32| TakeRow {
            number: n,
            captured: false,
            edited: false,
            made: false,
            at: Some((2, 0)),
            added: n as usize,
            erased: 0,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
        };
        st.done = vec![row(1), row(2)];
        st.undone = vec![row(4), row(3)];
        let (items, current) = take_items(&st);
        let titles: Vec<(&str, bool)> =
            items.iter().map(|i| (i.title.as_str(), i.undone)).collect();
        assert_eq!(
            titles,
            vec![
                ("Take 4 · Stab / 1", true),
                ("Take 3 · Stab / 1", true),
                ("Take 2 · Stab / 1", false),
                ("Take 1 · Stab / 1", false)
            ]
        );
        assert_eq!(current, Some(2));
        assert_eq!(items[2].detail, "+2");
        // ERASE + a pad while playing: live erase's own take, counted
        // from 1 ("erase 1"), not by its top-bit number.
        let erase = row(mpfree_play::ERASE_TAKES + 1);
        assert_eq!(erase.short(), "erase 1");
        assert_eq!(take_title(&st, &erase), "Erase 1 · Stab / 1");
        st.done[1].erased = 5;
        assert_eq!(take_items(&st).0[2].detail, "+2 −5");
        // A capture that made a clip.
        st.done[1] = TakeRow {
            number: 1,
            captured: true,
            edited: false,
            made: true,
            at: Some((0, 3)),
            added: 9,
            erased: 0,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
        };
        let (items, _) = take_items(&st);
        assert_eq!(
            (items[2].title.as_str(), items[2].detail.as_str()),
            ("Capture 1 · Drums / 4", "new +9")
        );
        assert_eq!(
            (st.done[1].short(), st.done[0].short()),
            ("capture 1".to_string(), "take 1".to_string())
        );
        st.done[1] = TakeRow {
            number: 5,
            captured: false,
            edited: true,
            made: false,
            at: Some((1, 0)),
            added: 1,
            erased: 1,
            deleted: false,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            pasted: false,
            replaced: None,
            variant: None,
            scene: None,
            rows: None,
        };
        assert_eq!(take_items(&st).0[2].title, "Edit 5 · Bass / 1");
        assert_eq!(st.done[1].short(), "edit 5");
        // ERASE + a clip: a delete, named by where the clip was.
        st.done[1] = TakeRow {
            number: 6,
            deleted: true,
            new_track: None,
            dropped: None,
            song: None,
            settings: false,
            at: Some((1, 0)),
            ..st.done[1]
        };
        let (items, _) = take_items(&st);
        assert_eq!(
            (items[2].title.as_str(), items[2].detail.as_str()),
            ("Delete 6 · Bass / 1", "clip")
        );
        assert_eq!(take_title(&st, &st.done[1]), "Delete 6 · Bass / 1");
        // Song → scenes: one edit, named by the rows it filled.
        st.done[1] = TakeRow {
            number: 7,
            deleted: false,
            made: true,
            rows: Some((4, 6)),
            ..st.done[1]
        };
        let (items, _) = take_items(&st);
        assert_eq!(
            (items[2].title.as_str(), items[2].detail.as_str()),
            ("Song → scenes 7", "rows 5 to 7")
        );
        assert_eq!(st.done[1].short(), "song → scenes 7");
        st.done[1].rows = Some((2, 2));
        assert_eq!(take_items(&st).0[2].detail, "row 3");
        assert_eq!(
            (bars_label(1), bars_label(4)),
            ("1 bar".to_string(), "4 bars".to_string())
        );
        st.done.clear();
        assert_eq!(take_items(&st).1, None);
    }

    #[test]
    fn labels() {
        let m = Meter::default();
        assert_eq!(length_label(3840.0, m), "1 bar");
        assert_eq!(length_label(7680.0, m), "2 bars");
        assert_eq!(length_label(1920.0, m), "2 beats");
        assert_eq!(length_label(5760.0, m), "6 beats");
        assert_eq!(length_label(1680.0, m), "7/16");
        assert_eq!(length_label(100.0, m), "100 ticks");
        assert_eq!(loop_label(None, m), "off");
        assert_eq!(loop_label(Some((0, 3840)), m), "1");
        assert_eq!(loop_label(Some((0, 4 * 3840)), m), "1–4");
        assert_eq!(loop_label(Some((960, 3840)), m), "1.2.000–2.1.000");
    }

    #[test]
    fn update_redraws_only_what_changed() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Clips"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let mut page = TimePage::add_to(&mut ui).unwrap();
        ui.show(CLIPS_PAGE).unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        let mut st = state();
        page.update(&mut ui, &st);
        ui.paint(&mut c);
        page.update(&mut ui, &st);
        assert!(!ui.needs_paint(), "nothing changed");
        st.transport.position += 1;
        page.update(&mut ui, &st);
        let drawn = ui.paint(&mut c);
        assert_eq!(
            drawn,
            vec![ui.rect_of(TRANSPORT).unwrap()],
            "one tick on: only the bar"
        );
        st.scene_cued = Some(1);
        page.update(&mut ui, &st);
        assert_eq!(
            ui.paint(&mut c),
            vec![ui.rect_of(CLIPS).unwrap()],
            "a scene cued: the grid"
        );
        st.morph.pos = 0.4;
        page.update(&mut ui, &st);
        assert_eq!(
            ui.paint(&mut c),
            vec![ui.rect_of(MORPH).unwrap()],
            "the crossfader moved: the morph bar"
        );
    }
}
