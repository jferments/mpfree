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
use crate::song::{AudioLane, SongBlock, SongLanes, ADD_AUDIO, AUDIO_BUTTONS, NONE, PANEL, REC};
use crate::widgets::{
    BarButton, BarItem, CellState, ClipGrid, GridCell, GridTrack, MorphBar, Run, TakeItem,
    TakeList, TransportBar, GRID_COLS,
};
use mpfree_audiotracks::host::TakeWave;
use mpfree_play::{anchored, Cue, Launch, Take, Takes};
use mpfree_seq::{Clip, Project, Source, Speed, SLOTS};
use mpfree_transport::time::{Meter, Swing, Tick, MAX_BPM, MIN_BPM, PPQ};
use mpfree_transport::transport::MAX_COUNT_IN;
use mpfree_transport::{Command, State, Transport};
use mpfree_ui::qlink::QLinkColumn;
use mpfree_ui::widgets::{Chip, OptionList, OptionRow};
use mpfree_ui::{Color, EventKind, Tone, Ui, UiEvent};

/// Widget names the page adds. The app's own are `header` and `hint`.
pub const TRANSPORT: &str = "transport";
pub const CLIPS: &str = "clips";
pub const RECORD: &str = "record";
pub const TAKES: &str = "takes";
pub const MORPH: &str = "morph";
pub const SONG: &str = "song";
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
/// them the record settings over the takes; Play's Q-Link column (168 px,
/// `mpfree_ui::qlink::COLUMN_WIDTH`) down the right edge, from under the
/// header to the hint line, as on Play.
pub const CLIPS_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(clips:*, morph:64):*, col(record:*, takes:240):250):*):*, qlinks:168):*, hint:26)";
/// The same page in its Song view: the timeline in place of the grid.
pub const SONG_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(col(song:*, morph:64):*, col(record:*, takes:240):250):*):*, qlinks:168):*, hint:26)";
pub const CLIPS_HINT: &str =
    "Tap a clip: it plays from the next bar, or stops   ·   a row's number plays the whole row   ·   + records into an empty slot   ·   a track's name arms it";
/// The hint line under the Song view.
pub const SONG_HINT: &str =
    "Tap a block to pick it   ·   Song on: the song plays where no clip is launched   ·   Scenes → song adds the rows at its end   ·   SHIFT + wheel: bars";
/// The same page with the note editor (a piano roll or a list) in place
/// of the grid, and the Edit options in place of the record settings.
pub const EDIT_LAYOUT: &str =
    "col(header:60, row(col(transport:44, row(notes:*, col(edit-options:*, takes:240):250):*):*, qlinks:168):*, hint:26)";
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
pub const RECORD_ROWS: [&str; 6] = ["Armed", "Mode", "Count-in", "Loop", "Tempo", "Pressure"];
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
    pub loop_range: Option<(Tick, Tick)>,
    pub overdub: bool,
    /// Bars of count-in before recording from stopped.
    pub count_in: u8,
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
        }
    }
}

impl TransportInfo {
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
    pub mute: bool,
    /// It records pad pressure (`Track::record_touch`).
    pub pressure: bool,
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
}

impl TakeRow {
    pub fn of(t: &Take) -> TakeRow {
        let made = t.made.first().map(|(track, slot, _)| (*track, *slot));
        let at = made.or_else(|| {
            t.added
                .first()
                .or(t.erased.first())
                .map(|&(track, slot, _)| (track, slot))
        });
        let added = t.added.len() + t.made.iter().map(|(_, _, c)| c.notes.len()).sum::<usize>();
        TakeRow {
            number: t.number,
            captured: t.captured,
            edited: t.edited,
            made: made.is_some(),
            at,
            added,
            erased: t.erased.len(),
        }
    }

    /// What it is: "take", "capture" or "edit".
    pub fn word(&self) -> &'static str {
        if self.captured {
            "capture"
        } else if self.edited {
            "edit"
        } else {
            "take"
        }
    }

    /// What Undo and Redo name it: "take 3", "capture 2", "edit 5".
    pub fn short(&self) -> String {
        format!("{} {}", self.word(), self.number)
    }
}

/// Everything the time engine's screens show.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimeState {
    pub transport: TransportInfo,
    pub tracks: Vec<TrackInfo>,
    pub launches: Vec<Launch>,
    /// Launches and stops waiting for the next bar line. The app sets them.
    pub cues: Vec<Cue>,
    pub armed: Option<usize>,
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
    /// The scene morph. The app sets it.
    pub morph: MorphInfo,
    /// The page shows the song (the timeline) in place of the grid. The
    /// app sets it.
    pub song_view: bool,
    /// Song mode: the song plays on tracks with no clip launched. The app
    /// sets it.
    pub song_on: bool,
    /// The arrangement's placements and markers.
    pub placements: Vec<PlacementInfo>,
    pub markers: Vec<(Tick, String)>,
    /// The placement selected on the Song view. The app sets it.
    pub selected: Option<usize>,
    /// Live to song armed: what the grid plays goes into the song. The app
    /// sets it.
    pub song_rec: bool,
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
    pub clip: Clip,
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
            .map(|t| TrackInfo {
                name: t.name.clone(),
                mute: t.mute,
                pressure: t.record_touch,
                slots: t
                    .slots
                    .iter()
                    .map(|s| {
                        s.as_ref().map(|c| ClipInfo {
                            name: c.name.clone(),
                            length: c.length,
                            speed: c.speed,
                            notes: c.notes.len(),
                        })
                    })
                    .collect(),
            })
            .collect();
        TimeState {
            transport,
            tracks,
            launches: launches.to_vec(),
            cues: Vec::new(),
            armed,
            done: takes.done().iter().map(TakeRow::of).collect(),
            undone: takes.undone().iter().map(TakeRow::of).collect(),
            chips: Vec::new(),
            capture: None,
            scene_playing: None,
            scene_cued: None,
            morph: MorphInfo::default(),
            song_view: false,
            song_on: false,
            placements: project
                .arrangement
                .placements
                .iter()
                .map(|p| PlacementInfo {
                    track: p.track,
                    start: p.start,
                    length: p.length,
                    name: p
                        .clip(&project.tracks)
                        .map_or_else(|| "(empty)".to_string(), |c| c.name.clone()),
                    unique: matches!(p.source, Source::Own(_)),
                })
                .collect(),
            markers: project
                .arrangement
                .markers
                .iter()
                .map(|m| (m.at, m.name.clone()))
                .collect(),
            selected: None,
            song_rec: false,
            edit: None,
            audio: Vec::new(),
            waves: Vec::new(),
            region: None,
            audio_free: None,
            audio_counts: Vec::new(),
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

    fn track_name(&self, track: usize) -> String {
        self.tracks
            .get(track)
            .map_or_else(|| format!("Track {}", track + 1), |t| t.name.clone())
    }
}

// ---------------------------------------------------------------- actions

/// What a touch on the time engine's screens asks for. The app applies it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TimeAction {
    /// For the audio thread's command queue.
    Transport(Command),
    Undo,
    Redo,
    /// Arm a track for recording, or none.
    Arm(Option<usize>),
    /// "Pressure" (pad pressure) for a track: whether recording keeps the pads'
    /// pressure on its notes (`Track::record_touch`).
    RecordPressure {
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
    /// Chip `i` on the bar was tapped.
    Chip(usize),
    /// Keep what was just played (CAPTURE): into the armed track's playing
    /// clip, or as a new clip (see `mpfree_capture::keep`).
    Capture,
    /// Play a whole row of the grid (a scene) from the next bar line, with
    /// its mutes (`mpfree_perform::Perform::launch_scene`).
    LaunchScene(usize),
    /// Stop every track on the next bar line.
    StopAll,
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
    /// Song mode on or off: the song plays on tracks with no clip launched.
    SongMode(bool),
    /// The rows with clips, in order, onto the end of the song
    /// (`mpfree_seq::arrange::scenes_to_song`).
    ScenesToSong,
    /// The song's sections into a run of empty rows, chained by follow
    /// actions (`mpfree_seq::arrange::song_to_scenes`).
    SongToScenes,
    /// Select a placement on the Song view, or none.
    SelectPlacement(Option<usize>),
    /// Give a placement its own copy of its clip.
    MakeUnique(usize),
    DeletePlacement(usize),
    /// Live to song on or off: while on and the transport plays, what the
    /// grid plays is recorded into the song
    /// (`mpfree_perform::SongRecorder`).
    RecSong(bool),
    /// A touch on the note editor (`crate::editor`).
    Edit(EditAction),
    /// A touch on the Song view's audio lanes, for the audio tracks
    /// (`mpfree_audiotracks`).
    Audio(AudioAction),
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
    /// Input, Monitor, level, pan, mute, solo, Takes) over the lanes.
    Panel(usize),
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
    Takes(usize),
    Undo,
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

/// Where the cues start: the next bar line the playhead plays. In a loop
/// whose end comes first, that is the loop start's bar line.
pub fn cue_tick(t: &TransportInfo) -> Tick {
    let next = t.meter.next_bar(t.position);
    match t.loop_range {
        Some((a, b)) if t.state.rolling() && t.position < b && next >= b => t.meter.next_bar(a),
        _ => next,
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
        (CLIPS, EventKind::Toggle(row)) => Some(match *row {
            r if r < SLOTS => TimeAction::LaunchScene(r),
            SLOTS => TimeAction::StopAll,
            r if r == SLOTS + 2 => TimeAction::RecSong(!st.song_rec),
            r if r == SLOTS + 3 => TimeAction::Edit(EditAction::Open),
            _ => TimeAction::SongView(true),
        }),
        (NOTES, kind) => Some(TimeAction::Edit(match *kind {
            EventKind::Tab(k) => EditAction::Button(k),
            EventKind::Select(i) => EditAction::Note(i),
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
        (SONG, EventKind::Tab(ADD_AUDIO)) => Some(TimeAction::Audio(AudioAction::Add)),
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
        (CLIPS, EventKind::Tab(track)) => Some(TimeAction::Arm(if st.armed == Some(*track) {
            None
        } else {
            Some(*track)
        })),
        (CLIPS, EventKind::Select(i)) => {
            let (track, slot) = (i / SLOTS, i % SLOTS);
            if track >= st.tracks.len() {
                None
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
                4 => {
                    let bpm = (t.bpm + f64::from(d)).clamp(MIN_BPM, MAX_BPM);
                    (bpm != t.bpm).then_some(TimeAction::Transport(Command::SetTempo(bpm)))
                }
                // Either way flips it, for the armed track.
                5 => st.armed.and_then(|a| {
                    let on = st.tracks.get(a)?.pressure;
                    Some(TimeAction::RecordPressure { track: a, on: !on })
                }),
                _ => None,
            }
        }
        (TAKES, EventKind::Tab(0)) => Some(TimeAction::Undo),
        (TAKES, EventKind::Tab(1)) => Some(TimeAction::Redo),
        (TAKES, EventKind::Tab(2)) => Some(TimeAction::Capture),
        _ => None,
    }
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

fn take_title(st: &TimeState, row: &TakeRow) -> String {
    let mut word = row.word().to_string();
    word[..1].make_ascii_uppercase();
    match row.at {
        Some((track, slot)) => format!(
            "{word} {} · {} / {}",
            row.number,
            st.track_name(track),
            slot + 1
        ),
        None => format!("{word} {}", row.number),
    }
}

fn take_detail(row: &TakeRow) -> String {
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
    bar.items = vec![
        BarItem::new("BPM", format!("{:.1}", t.bpm), Tone::Normal),
        BarItem::new(
            "",
            format!("{}/{}", t.meter.beats, t.meter.unit),
            Tone::Normal,
        ),
        BarItem::new("Swing", swing, Tone::Normal),
        BarItem::new(
            "Loop",
            loop_label(t.loop_range, t.meter),
            if t.loop_range.is_some() {
                Tone::Accent
            } else {
                Tone::Dim
            },
        ),
    ];
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

/// Cell `slot` of `track` for `st`.
pub fn cell_of(st: &TimeState, track: usize, slot: usize) -> GridCell {
    let Some(clip) = st.clip(track, slot) else {
        return GridCell::default();
    };
    let t = &st.transport;
    let len = clip.song_len().max(1.0);
    let notes = if clip.notes == 1 {
        "1 note".to_string()
    } else {
        format!("{} notes", clip.notes)
    };
    let mut cell = GridCell {
        state: CellState::Stopped,
        name: clip.name.clone(),
        detail: length_label(len, t.meter),
        foot: notes,
        progress: 0.0,
    };
    let cue = st.cue(track);
    if cue.is_some_and(|c| c.slot == Some(slot)) {
        cell.state = CellState::Queued;
        cell.foot = format!("from {}", bar_beat(cue_tick(t), t.meter));
        return cell;
    }
    let Some(l) = st.launch(track).filter(|l| l.slot == slot) else {
        return cell;
    };
    if l.origin > t.position {
        cell.state = CellState::Queued;
        cell.foot = format!("from {}", bar_beat(l.origin, t.meter));
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
    } else {
        cell.state = CellState::Playing;
    }
    if let Some(c) = cue {
        // It plays until the cue starts.
        let word = if c.slot.is_none() { "stops" } else { "until" };
        cell.detail = format!("{word} {}", bar_beat(cue_tick(t), t.meter));
    }
    cell
}

/// The grid's tracks for `st`.
pub fn grid_tracks(st: &TimeState) -> Vec<GridTrack> {
    st.tracks
        .iter()
        .enumerate()
        .map(|(i, tr)| GridTrack {
            name: tr.name.clone(),
            color: track_color(i),
            armed: st.armed == Some(i),
            mute: tr.mute,
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
        row(RECORD_ROWS[3], loop_label(t.loop_range, t.meter)),
        row(RECORD_ROWS[4], format!("{:.1}", t.bpm)),
        row(
            RECORD_ROWS[5],
            match st.armed.and_then(|a| st.tracks.get(a)) {
                None => "–".to_string(),
                Some(tr) if tr.pressure => "Recorded".to_string(),
                Some(_) => "Off".to_string(),
            },
        ),
    ]
}

/// The takes newest first: the undone ones (dimmed), then the ones Undo
/// can take back; and which row Undo takes back next.
pub fn take_items(st: &TimeState) -> (Vec<TakeItem>, Option<usize>) {
    let item = |r: &TakeRow, undone: bool| TakeItem {
        title: take_title(st, r),
        detail: take_detail(r),
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
    let t = &st.transport;
    let bar = t.meter.bar_ticks();
    let placed = st.placements.iter().map(|p| p.start + p.length);
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
        blocks: st
            .placements
            .iter()
            .enumerate()
            .map(|(index, p)| SongBlock {
                index,
                track: p.track,
                start: p.start,
                length: p.length,
                name: p.name.clone(),
                unique: p.unique,
            })
            .collect(),
        markers: st.markers.clone(),
        bar,
        first_bar,
        bars: 16,
        playhead: Some(t.position),
        selected: st.selected,
        on: st.song_on,
        rec: st.song_rec,
        audio: st.audio.clone(),
        region: st.region.map(|r| (r.track, r.index)),
        ..SongLanes::default()
    }
}

/// The note editor's roll or list for `e`, with the playhead where the
/// clip plays now (if it is launched).
pub fn note_grid(e: &EditInfo, st: &TimeState) -> NoteGrid {
    let ed = &e.editor;
    let c = &e.clip;
    let playhead = st
        .launches
        .iter()
        .find(|l| (l.track, l.slot) == (ed.track, ed.slot))
        .filter(|_| c.length > 0 && st.transport.position >= 0)
        .map(|l| {
            let at = c.speed.to_clip((st.transport.position - l.origin) as f64);
            (at.floor() as Tick).rem_euclid(c.length)
        });
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
    g.legend = format!("{} notes · {} picked · {lens}", c.notes.len(), n_sel);
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
    /// The first track shown.
    pub first_track: usize,
    /// The first bar the Song view shows (0-based).
    pub first_bar: i64,
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
        ui.add(RECORD, OptionList::new("Record"));
        ui.add(TAKES, TakeList::new("Takes"));
        ui.add(MORPH, MorphBar::new());
        ui.add(SONG, SongLanes::new());
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

    /// Scroll the grid `delta` rows down (up if negative), or on the Song
    /// view the timeline `delta` bars on. In the note editor the editor
    /// scrolls: the action for the app.
    pub fn scroll(&mut self, ui: &Ui, delta: i32) -> Option<TimeAction> {
        if ui.layout_of(CLIPS_PAGE) == Some(EDIT_LAYOUT) {
            return Some(TimeAction::Edit(EditAction::Scroll(delta)));
        }
        if ui.layout_of(CLIPS_PAGE) == Some(SONG_LAYOUT) {
            self.first_bar = (self.first_bar + i64::from(delta)).max(0);
            return None;
        }
        let rows = self.visible_rows(ui).max(1);
        let last = SLOTS.saturating_sub(rows) as i64;
        self.first_row = (self.first_row as i64 + i64::from(delta)).clamp(0, last) as usize;
        None
    }

    /// The options list the wheel steps: the Edit options in the note
    /// editor, else the record settings.
    pub fn wheel_list(&self, ui: &Ui) -> &'static str {
        if ui.layout_of(CLIPS_PAGE) == Some(EDIT_LAYOUT) {
            EDIT_OPTIONS
        } else {
            RECORD
        }
    }

    /// Show `st`. Widgets whose contents did not change are left alone, so
    /// they are not redrawn.
    pub fn update(&self, ui: &mut Ui, st: &TimeState) {
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
        let rows = self.visible_rows(ui).max(1);
        let title = format!(
            "Clips · slots {}–{} of {SLOTS}",
            self.first_row + 1,
            (self.first_row + rows).min(SLOTS)
        );
        let legend = if st.tracks.len() > GRID_COLS {
            format!(
                "tracks {}–{} of {}",
                self.first_track + 1,
                (self.first_track + GRID_COLS).min(st.tracks.len()),
                st.tracks.len()
            )
        } else {
            "ring: one pass   ·   amber: next bar   ·   red: recording".to_string()
        };
        let same_grid = ui.get::<ClipGrid>(CLIPS).is_some_and(|g| {
            g.tracks == tracks
                && g.title == title
                && g.legend == legend
                && g.first_row == self.first_row
                && g.first_track == self.first_track
                && (g.scene_playing, g.scene_cued) == (st.scene_playing, st.scene_cued)
                && g.song_rec == st.song_rec
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
            }
        }
        // The grid, the song or the note editor, as the app has it.
        let want = if st.edit.is_some() {
            EDIT_LAYOUT
        } else if st.song_view {
            SONG_LAYOUT
        } else {
            CLIPS_LAYOUT
        };
        if ui.layout_of(CLIPS_PAGE).is_some_and(|l| l != want) {
            let _ = ui.set_layout(CLIPS_PAGE, want);
        }
        let mut lanes = song_lanes(st, self.first_bar);
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
        }
        let m = st.morph;
        let bar = MorphBar {
            a: m.a,
            b: m.b,
            pos: m.pos,
            on: m.on,
            params: m.params,
        };
        if ui.get::<MorphBar>(MORPH).is_some_and(|b| *b != bar) {
            if let Some(b) = ui.get_mut::<MorphBar>(MORPH) {
                *b = bar;
            }
        }
        if let Some(e) = &st.edit {
            let grid = note_grid(e, st);
            if ui.get::<NoteGrid>(NOTES).is_some_and(|g| !g.same(&grid)) {
                if let Some(g) = ui.get_mut::<NoteGrid>(NOTES) {
                    g.show(grid);
                }
            }
            let rows = edit_rows(&e.editor, &e.clip);
            if let Some(l) = ui.get_mut::<OptionList>(EDIT_OPTIONS) {
                if l.rows != rows || l.selected != e.editor.row {
                    l.rows = rows;
                    l.selected = e.editor.row;
                    l.right = "wheel or arrows".to_string();
                }
            }
        }
        let rows = record_rows(st);
        if ui.get::<OptionList>(RECORD).is_some_and(|l| l.rows != rows) {
            if let Some(l) = ui.get_mut::<OptionList>(RECORD) {
                l.rows = rows;
                l.right = "wheel or arrows".to_string();
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
    use mpfree_ui::{Align, Fonts, Theme};

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
        // sixth, Pressure, scrolls in when a row below the middle is
        // picked (the list keeps the picked row in view).
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
            (EDIT_LAYOUT, NOTES, EDIT_OPTIONS),
        ] {
            ui.set_layout(CLIPS_PAGE, layout).unwrap();
            // Where Play has it (play_page's frame test): x 1100, y 84.
            assert_eq!(
                ui.rect_of(QLINKS),
                Some(Rect::new(1100, 84, COLUMN_WIDTH, 666)),
                "{layout}"
            );
            assert_eq!(ui.rect_of(TRANSPORT), Some(Rect::new(12, 84, 1076, 44)));
            let s = ui.rect_of(side).unwrap();
            assert_eq!((s.x, s.w), (838, SIDE_W), "{layout}");
            assert_eq!(ui.rect_of(TAKES).map(|t| (t.x, t.w)), Some((838, SIDE_W)));
            let b = ui.rect_of(body).unwrap();
            assert_eq!((b.x, b.right()), (12, 826), "{layout}");
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
        for name in RECORD_ROWS.iter().chain(crate::editor::EDIT_ROWS.iter()) {
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
            clip: c,
            track_name: "Drums".into(),
            bar: BAR,
        };
        let g = note_grid(&e, &state());
        assert_eq!(g.touch, [point(0, 90), point(40, 30)]);
        assert!(g.touch_of(0).is_empty());
        assert_eq!(g.touch_of(1), &g.touch[..]);
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
            Some(TimeAction::Arm(Some(0)))
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
        st.placements.push(PlacementInfo {
            track: 0,
            start: 3840,
            length: 7680,
            name: "Boom".into(),
            unique: false,
        });
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
            input: "L".into(),
            level: 0.3,
            blocks: vec![AudioBlock {
                start: 0,
                end: BAR,
                take: 0,
                tag: String::new(),
                src: 0,
                frames: 88_200.0,
                fade_in: 0,
                fade_out: 0,
            }],
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

    /// The waveforms reach the widget, and are copied again only when they
    /// change; the lanes' scroll survives an update.
    #[test]
    fn the_song_view_keeps_its_waves_and_scroll() {
        use mpfree_audiotracks::host::TakeWave;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Arrange"]));
        ui.add("hint", Label::new("", Tone::Dim, Align::Center));
        let page = TimePage::add_to(&mut ui).unwrap();
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
        assert_eq!(
            step(&st, 4, 1),
            Some(TimeAction::Transport(Command::SetTempo(121.0)))
        );
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
        assert_eq!(cue_tick(&st.transport), 3 * BAR);
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
        };
        assert_eq!(take_items(&st).0[2].title, "Edit 5 · Bass / 1");
        assert_eq!(st.done[1].short(), "edit 5");
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
        let page = TimePage::add_to(&mut ui).unwrap();
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
