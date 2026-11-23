//! The Song view of the Arrange page (roadmap T13): the arrangement's
//! placements on a timeline, a lane a track in track order, an audio
//! track's lane in its place (layout spec 4c), with the markers on a ruler
//! and the playhead. Like the other widgets it only shows what the app put
//! in its fields and reports touches:
//!
//! - a placement: `Select(index)` (its index in the arrangement) as the
//!   finger lifts; empty space: `Select(usize::MAX)`, nothing selected;
//! - a placement dragged (Jay, 14 Nov: drag and drop): it lifts once the
//!   finger moves [`LIFT_PX`], follows along its lane snapped to the bar
//!   or up and down to another track's, and where it lifts `Place {
//!   index, row: track, at: start }`; the picked one's end grip dragged,
//!   `Stretch { index, to: length }` (to the bar, a bar at least); let go
//!   where it started, nothing. Held at the left or right edge the bars
//!   scroll: `Tab(BARS_BACK)` or `Tab(BARS_ON)`;
//! - a section's name on the ruler: tapped `Tab(SECTION_TAP + k)` (pick
//!   it); dragged `Drop { from: k, to: gap }`, the gap nearest where its
//!   start is let go (before section `gap`, or after the last for the
//!   markers' count);
//! - the buttons along the bottom: `Tab(k)` for [`SONG_BUTTONS`]`[k]`, or
//!   for [`AUDIO_BUTTONS`]`[k]` while an audio region is picked;
//! - Rec song, at the right of the title line: `Tab(REC)`; the view chip
//!   at its left (the Arrange refactor's 5.7): its Clips `Tab(CLIPS)` and
//!   its List (the song list, Arrange's third view) `Tab(LIST)`;
//! - an audio region: `Cell { row: audio track, col: region index }` as
//!   the finger goes down; dragged sideways and let go, `Marker { index:
//!   audio track * REGION_SLOTS + region index, frame: its new start in
//!   song ticks }`;
//! - an audio lane's header: its arm dot `Toggle(audio track)`, its name
//!   or input chip `Cell { row: audio track, col: PANEL }` (on the lift,
//!   unless the finger dragged to scroll);
//! - "+ Track", the lane after the last while
//!   [`SongLanes::can_add_track`] holds: `Tab(ADD_TRACK)`; with the
//!   chooser open ([`SongLanes::adding`]) its rows `Tab(ADD_NOTE)`,
//!   `Tab(ADD_AUDIO)` and `Tab(ADD_LOOP)`, and a touch anywhere else
//!   `Tab(ADD_CLOSE)`;
//! - the track panel, while one is open ([`SongLanes::panel`]): Close
//!   `Tab(PANEL_CLOSE)`, Arm `Toggle(audio track)`, Mute, Solo and Takes
//!   `Tab(PANEL_MUTE)` and so on, an input `Tab(PANEL_INPUT + k)` for
//!   [`INPUTS`]`[k]`, a monitor mode `Tab(PANEL_MONITOR + k)` for
//!   [`MONITORS`]`[k]`, Mix `Tab(PANEL_MIX)`, and while the track records
//!   from the mix its From steps `Tab(PANEL_FROM_LESS)` and
//!   `Tab(PANEL_FROM_MORE)` and its Effects `Tab(PANEL_BEFORE)` and
//!   `Tab(PANEL_AFTER)`; the level's and pan's steps `Step { index:
//!   PANEL_LEVEL or PANEL_PAN, delta: -1 or 1 }`, and "Its strip in Mix
//!   ›" `Tab(PANEL_STRIP)`.
//!
//! A drag up or down the name column scrolls the lanes when there are more
//! than fit ([`MIN_LANE`] pixels each at least). The scroll is the
//! widget's own: [`SongLanes::keep_scroll`] carries it over when the app
//! shows new contents.
//!
//! **Audio lanes.** One for each audio track, in its track's place
//! ([`AudioLane::row`]). Regions are blocks in the track's
//! colour with the take's waveform (from [`SongLanes::waves`], the app's
//! `AudioHost::wave` reads), a "take N" tag when an older take plays, and
//! fades as slopes. While a track records, a red block grows from its
//! punch in to the playhead. The header has the name, an arm dot, the
//! input chip and a 3 px level bar along its bottom edge.
//!
//! **The track panel** (layout spec 4c) opens over the lanes, right of the
//! names, for the track in [`SongLanes::panel`]: Arm, Mute, Solo and
//! Takes, the input (L, R, L+R, ST or Mix), the monitor mode (Auto, In,
//! Off), and the level and pan with a step either way (Q1 and Q2 turn them
//! too). Mix records from inside the mix (the instrument redesign's step
//! 9, *proposed* in layout spec 4c): a row more, From (an instrument, a
//! submix or the master, a step either way) and Effects (Before, After),
//! and the monitor modes give way to a line saying the track never plays
//! it through. It sits under the track's lane if it fits, else over it;
//! touches on it go to it alone.

use crate::report::ReportCard;
use crate::view::PlacementInfo;
use crate::widgets::{
    dashed_outline, hatch, rec_song_pill, view_chip_rect, view_part_at, view_part_rect, view_title,
    ArrangeView, REC_SONG_W,
};
use mpfree_audiotracks::host::TakeWave;
use mpfree_audiotracks::{frames_from, Input, Monitor, Region, TakeInfo};
use mpfree_transport::time::{TempoMap, Tick};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};
use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The buttons along the bottom, left to right.
pub const SONG_BUTTONS: [&str; 6] = [
    "Split",
    "Play song",
    "Scenes → song",
    "Song → scenes",
    "Make unique",
    "Delete",
];
/// The buttons along the bottom while an audio region is picked (layout
/// spec 4c). To a cell puts the region in its track's first empty cell;
/// UNDO and SHIFT + UNDO undo and redo the region edits while they are
/// what changed last.
pub const AUDIO_BUTTONS: [&str; 6] = ["Split", "Erase", "Fades", "Gain", "Takes", "To a cell"];
/// The last two buttons while a tempo point is picked (tempo ramps).
pub const TEMPO_BUTTONS: [&str; 2] = ["Ramp", "Delete"];
/// The last four buttons while a section is picked (Arrange 3.3).
pub const SECTION_BUTTONS: [&str; 4] = ["+ Bar", "Rename", "Remove marker", "Delete bars"];
/// The ruler, as the row of [`EventKind::Place`]: a marker added on the
/// bar at `at` (index [`NONE`], a tap where no name is), or marker
/// `index` moved there (the picked section's grip dragged).
pub const RULER_ROW: usize = 0x501;
/// The picked section's grip on the ruler: its width, left of its marker.
pub const GRIP_W: i32 = 28;
/// The picked block's start grip, as the row of [`EventKind::Place`]:
/// block `index` trimmed from the left to start on `at` (the Arrange
/// refactor's phase 6).
pub const START_ROW: usize = 0x502;
/// A slide, as the row of [`EventKind::Place`]: what block `index` plays
/// moved `at` song ticks later under it, earlier below 0 (phase 6).
pub const SLIDE_ROW: usize = 0x503;
/// What Tab reports for Duplicate (phase 6), drawn between Song → scenes
/// and Make unique (after `SHOW_VIEW`: see
/// `tests::the_song_views_tab_ids_are_all_different`).
pub const DUPLICATE: usize = 0x313;
/// How long a finger stays still on the picked block before a drag
/// slides what the block plays instead of moving it (phase 6).
pub const SLIDE_HOLD: Duration = Duration::from_millis(400);
/// A box dragged across the lanes (6.5), as the index of
/// [`EventKind::Span`]: `BOX + top * 0x100 + bottom`, the first and last
/// tracks whose lanes it crosses ([`box_tracks`]), from and to the song
/// ticks at its left and right. Above any take's number, which a take
/// lane's swipe reports there.
pub const BOX: usize = 1 << 24;

/// The first and last tracks of a box's [`EventKind::Span`] index.
pub fn box_tracks(index: usize) -> (usize, usize) {
    let k = index.saturating_sub(BOX);
    (k / 0x100, k % 0x100)
}

/// The zoom chip's choices (6.4): how many bars the Song view shows.
pub const ZOOMS: [i64; 5] = [4, 8, 16, 32, 64];
/// What Tab reports for the zoom chip's part `k`: `ZOOM + k` for
/// [`ZOOMS`]`[k]`.
pub const ZOOM: usize = 0x320;
/// The zoom chip's "Bars" and each of its parts.
const ZOOM_LABEL_W: i32 = 44;
const ZOOM_PART_W: i32 = 40;
pub const ZOOM_W: i32 = ZOOM_LABEL_W + ZOOMS.len() as i32 * ZOOM_PART_W;

/// Duplicate's width.
const DUPLICATE_W: i32 = 104;
/// Split's width, the bottom row's first place (phase 6).
const SPLIT_W: i32 = 76;
/// The tempo lane, as the row of [`EventKind::Place`] (a point added, with
/// index [`NONE`], or point `index` moved to `at`) and of
/// [`EventKind::Cell`] (point `col` picked).
pub const TEMPO_ROW: usize = 0x500;
/// What Tab reports for a tap on section `k`'s name on the ruler: this
/// plus `k` (fewer than `mpfree_seq::arrange::MAX_MARKERS`, 256).
pub const SECTION_TAP: usize = 0x200;
/// What Tab reports while a dragged block or section is held at the left
/// or right edge: the bars scroll one back or on.
pub const BARS_BACK: usize = 0x300;
pub const BARS_ON: usize = 0x301;
/// Pixels a finger moves on a block, its end grip or a section's name
/// before it lifts (a drag, not a tap).
pub const LIFT_PX: i32 = 12;
/// How near the left or right edge a dragged thing scrolls the bars, and
/// how often while it stays there.
const EDGE_PX: i32 = 28;
const SCROLL_REPEAT: Duration = Duration::from_millis(400);
/// What Select reports for a tap on empty space.
pub const NONE: usize = usize::MAX;
/// What Tab reports for Rec song.
pub const REC: usize = SONG_BUTTONS.len();
/// What Tab reports for the "+ Track" chooser's Audio track.
pub const ADD_AUDIO: usize = REC + 1;
/// The `col` of a `Cell` for a tap on an audio lane's name or input chip.
pub const PANEL: usize = usize::MAX;
/// The track panel's Close, Mute, Solo and Takes, as Tab reports them.
pub const PANEL_CLOSE: usize = ADD_AUDIO + 1;
pub const PANEL_MUTE: usize = PANEL_CLOSE + 1;
pub const PANEL_SOLO: usize = PANEL_CLOSE + 2;
pub const PANEL_TAKES: usize = PANEL_CLOSE + 3;
/// The panel's inputs: `Tab(PANEL_INPUT + k)` for [`INPUTS`]`[k]`.
pub const PANEL_INPUT: usize = PANEL_CLOSE + 4;
/// The panel's monitor modes: `Tab(PANEL_MONITOR + k)` for
/// [`MONITORS`]`[k]`.
pub const PANEL_MONITOR: usize = PANEL_INPUT + INPUTS.len();
/// The panel's Mix (record from inside the mix), its From steps and its
/// Effects (step 9).
pub const PANEL_MIX: usize = PANEL_MONITOR + MONITORS.len();
pub const PANEL_FROM_LESS: usize = PANEL_MIX + 1;
pub const PANEL_FROM_MORE: usize = PANEL_MIX + 2;
pub const PANEL_BEFORE: usize = PANEL_MIX + 3;
pub const PANEL_AFTER: usize = PANEL_MIX + 4;
/// The panel's "Its strip in Mix ›": the Mix tab, with the track's strip
/// picked (the audio tracks rethink, phase 1b).
pub const PANEL_STRIP: usize = PANEL_MIX + 5;
/// The line under the panel's level and pan, as drawn on the approved
/// screens: they are its strip's.
pub const PANEL_NOTE: &str =
    "Level and pan move its strip in Mix  ·  Mute and Solo are the track's";
/// The `index` of the panel's level and pan steps.
pub const PANEL_LEVEL: usize = 0;
pub const PANEL_PAN: usize = 1;
/// What a track can record from, as the header's chip and the panel name
/// them.
pub const INPUTS: [(&str, Input); 4] = [
    ("L", Input::Left),
    ("R", Input::Right),
    ("L+R", Input::Sum),
    ("ST", Input::Stereo),
];
/// When a track hears its input, as the panel names it.
pub const MONITORS: [(&str, Monitor); 3] = [
    ("Auto", Monitor::Auto),
    ("In", Monitor::In),
    ("Off", Monitor::Off),
];
/// The chip for input `i`.
pub fn input_chip(i: Input) -> &'static str {
    INPUTS.iter().find(|x| x.1 == i).map_or("", |x| x.0)
}
/// What Tab reports for "+ Track" (the lane after the last): well clear
/// of the panel's, so a step past the panel's last is still nothing. The
/// app opens the chooser ([`SongLanes::adding`]), or with SHIFT adds a
/// Loop track at once (layout spec 4c).
pub const ADD_TRACK: usize = 0x100;
/// The chooser's Note track and Loop track, as Tab reports them; its
/// Audio track is [`ADD_AUDIO`]. A touch anywhere else while it is open
/// reports [`ADD_CLOSE`].
pub const ADD_NOTE: usize = 0x101;
pub const ADD_LOOP: usize = 0x102;
pub const ADD_CLOSE: usize = 0x103;
/// The chooser's rows, top to bottom, with what each reports.
pub const ADD_KINDS: [(&str, usize); 3] = [
    ("Note track", ADD_NOTE),
    ("Audio track", ADD_AUDIO),
    ("Loop track", ADD_LOOP),
];
/// The line under each row's name, as drawn on the approved screens.
pub const ADD_ABOUT: [&str; 3] = [
    "Plays a kit or an instrument; records notes",
    "Records sound from an input; its own mixer strip",
    "Loops in the Clips grid (they join audio tracks later)",
];
/// The hint line while the "+ Track" chooser is open.
pub const ADD_HINT: &str =
    "Pick the kind of track to add: Note, Audio or Loop   ·   it goes at the end of the track list   ·   UNDO takes it away   ·   tap anywhere else: nothing is added";
/// What Tab reports for the view chip's List: the song list.
pub const LIST: usize = 0x310;
/// What Tab reports for the view chip's Clips: the grid.
pub const CLIPS: usize = 0x311;
/// What Tab reports for Show, left of Rec song while a set is open: back
/// to the Show view (live shows B1).
pub const SHOW_VIEW: usize = 0x312;
/// A region drag's `Marker` index is the audio track times this, plus
/// the region's index (`mpfree_audiotracks::MAX_REGIONS`).
pub const REGION_SLOTS: usize = mpfree_audiotracks::MAX_REGIONS;
/// A lane is never thinner than this: past it the lanes scroll.
pub const MIN_LANE: i32 = 32;

/// A placement as the Song view shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct SongBlock {
    /// Its index in the arrangement.
    pub index: usize,
    pub track: usize,
    pub start: Tick,
    pub length: Tick,
    pub name: String,
    /// Its own copy, not linked to a slot.
    pub unique: bool,
}

/// The song's blocks as the Song view keeps them (Arrange refactor 2.4):
/// in the arrangement's order (it derefs to them), shared by every copy of
/// the view the screen makes to compare, and found by lane and time, so a
/// frame draws and hit-tests only the blocks in view and an update that
/// changes no block copies none, however long the song.
///
/// Equal when they are the same blocks: a copy compares in one step.
#[derive(Clone, Debug, Default)]
pub struct SongBlocks(Arc<BlockIndex>);

#[derive(Debug, Default)]
struct BlockIndex {
    all: Vec<SongBlock>,
    /// Places in `all`, by track and then start.
    order: Vec<u32>,
    /// Where track `t`'s blocks are in `order`: `lanes[t]..lanes[t + 1]`.
    lanes: Vec<usize>,
    /// For each place in `order`, the latest end of its track's blocks up
    /// to it: with the starts in order, the blocks between two ticks are
    /// two binary searches away, whether or not blocks overlap.
    reach: Vec<Tick>,
    /// The latest end of all (0 with none).
    end: Tick,
    /// The app's list they were made from ([`SongBlocks::of`]).
    from: Option<Arc<[PlacementInfo]>>,
}

impl SongBlocks {
    pub fn new(all: Vec<SongBlock>) -> SongBlocks {
        SongBlocks::index(all, None)
    }

    /// The blocks of the app's placements, kept with them: see
    /// [`SongBlocks::made_from`].
    pub fn of(placements: &Arc<[PlacementInfo]>) -> SongBlocks {
        let all = placements
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
            .collect();
        SongBlocks::index(all, Some(placements.clone()))
    }

    /// They were made from `placements`, the same list (not an equal one):
    /// one comparison, whatever the song's length.
    pub fn made_from(&self, placements: &Arc<[PlacementInfo]>) -> bool {
        self.0
            .from
            .as_ref()
            .is_some_and(|f| Arc::ptr_eq(f, placements))
    }

    fn index(all: Vec<SongBlock>, from: Option<Arc<[PlacementInfo]>>) -> SongBlocks {
        let lanes_n = all.iter().map(|b| b.track + 1).max().unwrap_or(0);
        let mut order: Vec<u32> = (0..all.len() as u32).collect();
        // The arrangement keeps its placements so: then this is one pass.
        order.sort_by_key(|&i| (all[i as usize].track, all[i as usize].start));
        let mut lanes = vec![0; lanes_n + 1];
        for b in &all {
            lanes[b.track + 1] += 1;
        }
        for t in 0..lanes_n {
            lanes[t + 1] += lanes[t];
        }
        let mut reach = Vec::with_capacity(order.len());
        for (k, &i) in order.iter().enumerate() {
            let b = &all[i as usize];
            let before = if k > 0 && all[order[k - 1] as usize].track == b.track {
                reach[k - 1]
            } else {
                Tick::MIN
            };
            reach.push(before.max(b.start + b.length));
        }
        let end = all.iter().map(|b| b.start + b.length).max().unwrap_or(0);
        SongBlocks(Arc::new(BlockIndex {
            all,
            order,
            lanes,
            reach,
            end,
            from,
        }))
    }

    /// Placement `index` (its index in the arrangement).
    pub fn get(&self, index: usize) -> Option<&SongBlock> {
        let all = &self.0.all;
        match all.get(index) {
            Some(b) if b.index == index => Some(b),
            _ => all.iter().find(|b| b.index == index),
        }
    }

    /// Track `track`'s blocks that play some of `from..to`, by start.
    pub fn between(
        &self,
        track: usize,
        from: Tick,
        to: Tick,
    ) -> impl Iterator<Item = &SongBlock> + '_ {
        let ix = &*self.0;
        let run = match (ix.lanes.get(track), ix.lanes.get(track + 1)) {
            (Some(&a), Some(&b)) => a..b,
            _ => 0..0,
        };
        let (order, reach) = (&ix.order[run.clone()], &ix.reach[run]);
        let first = reach.partition_point(|&e| e <= from);
        let last = order.partition_point(|&i| ix.all[i as usize].start < to);
        order[first..last.max(first)]
            .iter()
            .map(|&i| &ix.all[i as usize])
            .filter(move |b| b.start + b.length > from)
    }

    /// Where the last block ends (0 with none).
    pub fn end(&self) -> Tick {
        self.0.end
    }
}

impl std::ops::Deref for SongBlocks {
    type Target = [SongBlock];
    fn deref(&self) -> &[SongBlock] {
        &self.0.all
    }
}

impl From<Vec<SongBlock>> for SongBlocks {
    fn from(all: Vec<SongBlock>) -> SongBlocks {
        SongBlocks::new(all)
    }
}

impl PartialEq for SongBlocks {
    fn eq(&self, o: &SongBlocks) -> bool {
        Arc::ptr_eq(&self.0, &o.0) || self.0.all == o.0.all
    }
}

/// An audio region as the Song view shows it: see [`audio_blocks`].
#[derive(Clone, Debug, PartialEq)]
pub struct AudioBlock {
    pub start: Tick,
    pub end: Tick,
    pub take: u16,
    /// "take 2" when it plays an older take than the track's newest, else
    /// empty.
    pub tag: String,
    /// The take's frame at `start`, and the frames from `start` to `end`
    /// along the tempo map: the waveform under the block.
    pub src: i64,
    pub frames: f64,
    /// Its level, 0 to 4 (1 is as recorded).
    pub gain: f32,
    /// Its fades, in frames, measured on the region it was cut from:
    /// `into` frames of that region lie before `start` and `left` after
    /// `end` (0 for a region never cut), so a cut shows its share.
    pub fade_in: u32,
    pub fade_out: u32,
    pub into: u32,
    pub left: u32,
}

/// One of an audio track's takes, as its take lane shows it: see
/// [`take_lanes`].
#[derive(Clone, Debug, PartialEq)]
pub struct TakeLane {
    pub take: u16,
    /// "take 3": the track's takes count from 1, oldest first, as the
    /// regions' tags do.
    pub name: String,
    /// Where its audio lies on the song, and the frames it spans.
    pub start: Tick,
    pub end: Tick,
    pub frames: f64,
    pub recording: bool,
    /// Where the track plays it now (its regions of this take).
    pub used: Vec<(Tick, Tick)>,
}

/// An audio track's lane.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioLane {
    /// The audio track: its engine track, 0 to
    /// `mpfree_audiotracks::MAX_TRACKS` - 1, as the actions name it.
    pub track: usize,
    /// Its track's place in the track list (an index into
    /// [`SongLanes::tracks`]): its lane is that track's.
    pub row: usize,
    pub name: String,
    pub color: Color,
    pub armed: bool,
    /// What it records from: the input chip, "L", "R", "L+R" or "ST".
    pub input: Input,
    /// What it records from inside the mix instead (step 9; the chip
    /// reads "Mix"), None for the line input.
    pub mix: Option<MixFrom>,
    /// The level bar along the header's bottom edge, 0 to 1.
    pub level: f32,
    /// The track's level, 0 to 2 (1 is as recorded), and its balance, -1
    /// (left) to 1 (right): its mixer strip's fader and pan (phase 1b), as
    /// the panel's Level and Pan and Q1 and Q2 show and turn them.
    pub volume: f32,
    pub pan: f32,
    /// When it hears its input, and whether it is muted or soloed: for
    /// the panel.
    pub monitor: Monitor,
    pub mute: bool,
    pub solo: bool,
    /// Its regions, in order (`AudioTracks::regions`).
    pub blocks: Vec<AudioBlock>,
    /// Its takes, newest first, for its take lanes ([`take_lanes`]).
    pub takes: Vec<TakeLane>,
    /// Recording: the punch in (the take's anchor). The red block runs
    /// from it to the playhead.
    pub recording: Option<Tick>,
    /// The blocks of audio clips on its lane, by placement (phase 2,
    /// step 2d): what each one's wave draws.
    pub clips: Vec<ClipBlock>,
}

/// A block of an audio clip on an audio track's lane, as the Song view
/// draws it (the audio tracks rethink's phase 2, step 2d): a header strip
/// with its clip's name over its wave, pass after pass, with a faint line
/// where each pass after the first starts, so it never looks like a
/// region.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipBlock {
    /// The placement: its index in the arrangement.
    pub index: usize,
    /// The song tick its clip's passes count from (before the block's
    /// start when it starts inside a pass), and a pass's song ticks.
    pub origin: Tick,
    pub pass: Tick,
    /// A once clip: its header says so.
    pub once: bool,
    /// What one pass plays, in ticks from the pass's start.
    pub pieces: Vec<ClipPiece>,
}

/// One region of an audio clip as its block draws it ([`ClipBlock`]):
/// ticks `start..end` of each pass play take `take` from frame `src`, for
/// `frames` frames (along the tempo map, at the block's first pass).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPiece {
    pub start: Tick,
    pub end: Tick,
    pub take: u16,
    pub src: i64,
    pub frames: f64,
}

/// What an audio track records from inside the mix (the instrument
/// redesign's step 9), as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct MixFrom {
    /// The instrument, submix or master, by name.
    pub name: String,
    /// After its effects (else before them).
    pub after_effects: bool,
}

/// The Song view: see the module notes.
#[derive(Clone, Debug, PartialEq)]
pub struct SongLanes {
    pub title: String,
    pub legend: String,
    /// The legend counts something lost or refused: it shows in the warning
    /// colour.
    pub alert: bool,
    /// Each track's name and colour, top to bottom.
    pub tracks: Vec<(String, Color)>,
    /// What each track plays, under its name; empty shows only the name.
    pub sounds: Vec<String>,
    pub blocks: SongBlocks,
    pub markers: Vec<(Tick, String)>,
    /// Each marker's passes and their length while its part follows its
    /// scene on a linked song, with more than one ([`crate::view::TimeState::passes`]).
    pub passes: Vec<Option<(u16, Tick)>>,
    /// Ticks a bar, and a beat.
    pub bar: Tick,
    pub beat: Tick,
    /// The first bar shown (0-based) and how many: one of [`ZOOMS`] (the
    /// zoom chip lights it, 6.4).
    pub first_bar: i64,
    pub bars: i64,
    pub playhead: Option<Tick>,
    pub selected: Option<usize>,
    /// The blocks picked together (6.5): two or more, `selected` being
    /// None while they are. Each has the pick's outline, and a drag of
    /// one carries them all. The app sets it.
    pub picks: Vec<usize>,
    /// Song mode: the song plays.
    pub on: bool,
    /// Live to song armed.
    pub rec: bool,
    /// A set is open (live shows B1): **Show** (72 px), left of Rec song,
    /// gives `Tab(`[`SHOW_VIEW`]`)`.
    pub show_button: bool,
    /// The audio tracks' lanes, each in its track's place
    /// ([`AudioLane::row`]).
    pub audio: Vec<AudioLane>,
    /// The waveforms read so far, a take each.
    pub waves: Vec<TakeWave>,
    /// The picked audio region: (audio track, region index).
    pub region: Option<(usize, usize)>,
    /// The audio track whose panel is open over the lanes.
    pub panel: Option<usize>,
    /// The audio track whose take lanes show under its lane.
    pub takes_open: Option<usize>,
    /// The project has room for another track: "+ Track" follows the
    /// last lane.
    pub can_add_track: bool,
    /// The "+ Track" chooser is open under it (the app's, from a tap on
    /// "+ Track"): Note track, Audio track, Loop track.
    pub adding: bool,
    /// A finger swiping across a take lane: the take, where it went down
    /// and where it is now (x).
    pub(crate) swipe: Option<(u16, i32, i32)>,
    /// A finger dragging an audio region: its lane (an index into
    /// `audio`), its index, where it went down and where it is now (x).
    pub(crate) moving: Option<(usize, usize, i32, i32)>,
    /// The first lane shown (the tracks in order, take lanes under the
    /// open audio track, then "+ Track"). Past the last that can be first
    /// it shows the end: the app sets `usize::MAX` to bring a track just
    /// added at the end into view.
    pub first_lane: usize,
    /// A finger on the name column: where it went down, the first lane
    /// then, and whether it has dragged.
    pub(crate) drag: Option<(i32, usize, bool)>,
    /// The section picked (by marker), for the wheel. The app sets it.
    pub section: Option<usize>,
    /// A finger holding a block, its end grip or a section's name.
    pub(crate) grip: Option<Grip>,
    /// The song's tempo points, in order: tick, tempo and whether it
    /// glides to the next (tempo ramps). The tempo lane under the ruler
    /// draws them.
    pub tempo: Vec<(Tick, f64, bool)>,
    /// The project's tempo, before the first point.
    pub tempo_base: f64,
    /// The tempo point picked (an index into `tempo`), for the wheel and
    /// Ramp and Delete. The app sets it.
    pub tempo_pick: Option<usize>,
    /// The last conversion's report card, open over the lanes (the
    /// Arrange refactor's 4.3): it takes their touches and their targets.
    pub report: Option<ReportCard>,
    /// The clips launched over the song, each drawn over its track's
    /// lane (the Arrange refactor's 5.1, D3).
    pub over: Vec<LaneOver>,
    /// What the last whole paint showed: see [`Painted`].
    pub(crate) painted: Painted,
}

/// A clip launched over the song (the Arrange refactor's 5.1, D3): the
/// Song view draws it over its track's lane, from `from` to `until` (the
/// view's right edge while nothing waits), the song's blocks hatched
/// under it, silent while it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneOver {
    /// The track (an index into [`SongLanes::tracks`]).
    pub track: usize,
    pub name: String,
    /// Where it started, or the line it starts on while it waits.
    pub from: Tick,
    /// The line it ends on, while a cue waits on the track.
    pub until: Option<Tick>,
    /// The cue is a stop or Back to song: the song comes back at `until`
    /// (an amber dashed line there).
    pub back: bool,
    /// Not started yet: it waits for `from` (a dashed amber outline).
    pub waiting: bool,
}

/// What a finger holds on the Song view, and where in it (song ticks from
/// its start, or from its end for the end grip).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Held {
    /// A placement, by its index in the arrangement.
    Block { index: usize, dt: Tick },
    /// The picked placement's end grip.
    End { index: usize, dt: Tick },
    /// The picked placement's start grip (trim from the left, phase 6).
    Start { index: usize, dt: Tick },
    /// The picked placement held still, then dragged: what it plays
    /// slides under it (phase 6).
    Slide { index: usize },
    /// Section `k`'s name on the ruler.
    Section { k: usize, dt: Tick },
    /// Tempo point `k`, or the tempo lane where there is none (`None`: a
    /// tap there adds one).
    Tempo { k: Option<usize>, dt: Tick },
    /// The ruler where no name is: a tap adds a marker (Arrange 3.3).
    Ruler,
    /// The picked section's grip: marker `k` alone moves (Arrange 3.3).
    MarkerGrip { k: usize, dt: Tick },
    /// Empty space on the lanes: a tap picks nothing, a drag draws a box
    /// that picks the blocks it touches (6.5).
    Box,
}

/// A finger holding something on the Song view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Grip {
    held: Held,
    /// Where the finger went down, and where it is now.
    from: (i32, i32),
    at: (i32, i32),
    /// It moved [`LIFT_PX`]: a drag, not a tap.
    lifted: bool,
    /// When the finger went down: held [`SLIDE_HOLD`] still on the picked
    /// block, a drag slides what it plays.
    since: Instant,
    /// Held at an edge: which way (`BARS_BACK` or `BARS_ON`), and when the
    /// bars last scrolled.
    edge: Option<(usize, Instant)>,
}

impl Grip {
    fn new(held: Held, x: i32, y: i32) -> Grip {
        Grip {
            held,
            from: (x, y),
            at: (x, y),
            lifted: false,
            since: Instant::now(),
            edge: None,
        }
    }
}

pub const NAME_W: i32 = 96;
/// The least lane height that shows what a track plays under its name.
const SOUND_LANE_H: i32 = 34;
const RULER_H: i32 = 26;
/// The tempo lane's height, under the ruler.
const TEMPO_H: i32 = 24;
/// How near a point a finger picks it up, across.
const TEMPO_HIT: i32 = 14;
const TOOLS_H: i32 = 48;
const LANE_MAX: i32 = 56;
const GAP: i32 = 6;
/// Pixels a finger moves on the name column before it scrolls.
const DRAG_SLOP: i32 = 8;
/// A take lane's height.
const TAKE_H: i32 = 28;
/// The "+ Track" chooser (the Clips grid's too): the card's width, a
/// row's height, the card's padding, the gap between rows and the title
/// line's height.
pub const CHOOSER_W: i32 = 360;
pub const CHOOSER_ROW: i32 = 52;
const CHOOSER_PAD: i32 = 12;
const CHOOSER_GAP: i32 = 8;
const CHOOSER_HEAD: i32 = 16;
/// The chooser card's height.
pub const CHOOSER_H: i32 =
    CHOOSER_PAD + CHOOSER_HEAD + CHOOSER_GAP + 3 * CHOOSER_ROW + 2 * CHOOSER_GAP + CHOOSER_PAD;

/// The chooser's rows for a card whose top left corner is `(x, y)`.
pub fn chooser_rows(x: i32, y: i32) -> [Rect; 3] {
    let row = |k: i32| {
        Rect::new(
            x + CHOOSER_PAD,
            y + CHOOSER_PAD + CHOOSER_HEAD + CHOOSER_GAP + k * (CHOOSER_ROW + CHOOSER_GAP),
            CHOOSER_W - 2 * CHOOSER_PAD,
            CHOOSER_ROW,
        )
    };
    [row(0), row(1), row(2)]
}
/// The track panel's size at most, its rows' height and its labels'
/// width.
const PANEL_W: i32 = 560;
const PANEL_H: i32 = 188;
/// The panel's "Its strip in Mix ›" button's width.
const PANEL_STRIP_W: i32 = 140;
const PANEL_ROW: i32 = 36;
const PANEL_LABEL: i32 = 52;

/// Where the track panel's controls are: see [`SongLanes::panel_parts`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PanelParts {
    /// The audio track it is for.
    pub track: usize,
    pub panel: Rect,
    pub close: Rect,
    pub arm: Rect,
    pub mute: Rect,
    pub solo: Rect,
    pub takes: Rect,
    /// A button for each of [`INPUTS`] and [`MONITORS`], and Mix.
    pub inputs: [Rect; 4],
    pub mix: Rect,
    pub monitors: [Rect; 3],
    /// While the track records from the mix: From's step down, name and
    /// step up, and Effects' Before and After.
    pub from: Option<[Rect; 3]>,
    pub effects: Option<[Rect; 2]>,
    /// The level's and the pan's step down, value and step up.
    pub level: [Rect; 3],
    pub pan: [Rect; 3],
    /// "Its strip in Mix ›", at the end of the level's row, and the line
    /// under it ([`PANEL_NOTE`]).
    pub strip: Rect,
    pub note: Rect,
}

impl Default for SongLanes {
    fn default() -> SongLanes {
        SongLanes {
            title: String::new(),
            legend: String::new(),
            alert: false,
            tracks: Vec::new(),
            sounds: Vec::new(),
            blocks: SongBlocks::default(),
            markers: Vec::new(),
            passes: Vec::new(),
            bar: 3840,
            beat: 960,
            first_bar: 0,
            bars: 16,
            playhead: None,
            selected: None,
            picks: Vec::new(),
            on: false,
            rec: false,
            show_button: false,
            audio: Vec::new(),
            waves: Vec::new(),
            region: None,
            panel: None,
            takes_open: None,
            can_add_track: false,
            adding: false,
            swipe: None,
            moving: None,
            first_lane: 0,
            drag: None,
            section: None,
            grip: None,
            tempo: Vec::new(),
            tempo_base: 120.0,
            tempo_pick: None,
            report: None,
            over: Vec::new(),
            painted: Painted::default(),
        }
    }
}

/// The playhead's line on the Song view, in pixels.
const PLAYHEAD_W: i32 = 2;

/// What the Song view's last whole paint showed: its rectangle, a copy
/// of it (without the waveforms, with the playhead since drawn) and its
/// waveforms' keys. A frame that only moves the playhead, as every frame
/// does while the song plays, then draws the two strips the line left and
/// reached ([`SongLanes::changed_parts`]) rather than the whole view.
/// It is never compared (two views that show the same are equal), and a
/// copy starts with none, so a view the app puts in place paints whole.
#[derive(Default)]
pub(crate) struct Painted(RefCell<Option<Box<Shown>>>);

/// A whole paint's rectangle, its copy of the view and its waveforms' keys.
type Shown = (Rect, SongLanes, Vec<WaveKey>);

impl Clone for Painted {
    fn clone(&self) -> Painted {
        Painted::default()
    }
}

impl PartialEq for Painted {
    fn eq(&self, _: &Painted) -> bool {
        true
    }
}

impl fmt::Debug for Painted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Painted")
    }
}

/// What [`same_waves`] compares of a waveform.
type WaveKey = (u16, usize, usize, usize, usize);

fn wave_keys(waves: &[TakeWave]) -> Vec<WaveKey> {
    waves
        .iter()
        .map(|w| {
            (
                w.id,
                w.frames,
                w.channels,
                w.frames_per_column,
                w.columns.len(),
            )
        })
        .collect()
}

struct Geo {
    ruler: Rect,
    /// The tempo lane, the name column included.
    tempo: Rect,
    lanes: Rect,
    /// The height of every lane but the take lanes.
    lane_h: i32,
    tools: Rect,
}

/// What a lane is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Midi(usize),
    /// An index into `audio`.
    Audio(usize),
    /// Take lane `k` (an index into its `takes`) of audio lane `a`.
    Take(usize, usize),
    /// "+ Track", after the last lane.
    AddTrack,
}

/// The "+ Track" chooser (layout spec 4c, as drawn on the approved
/// screens): a raised card titled "ADD A TRACK" round `rows`, each row a
/// kind of track ([`ADD_KINDS`]) with a line on what it is
/// ([`ADD_ABOUT`]). The Clips grid draws the same under its "+ Track".
pub fn paint_chooser(cx: &mut PaintCx, rows: &[Rect; 3]) {
    let t = cx.theme.clone();
    let card = Rect::new(
        rows[0].x - CHOOSER_PAD,
        rows[0].y - CHOOSER_PAD - CHOOSER_HEAD - CHOOSER_GAP,
        CHOOSER_W,
        CHOOSER_H,
    );
    // A shadow under it, so it lifts off the lanes, as the panel does.
    let shadow = Rect::new(card.x + 2, card.y + 4, card.w, card.h);
    cx.canvas.fill_round_rect(shadow, t.round(12), t.bg);
    cx.canvas.fill_round_rect(card, t.round(12), t.raised);
    cx.canvas.stroke_round_rect(card, t.round(12), 1, t.outline);
    cx.text(
        FontId::Bold,
        t.size_small - 2.0,
        Rect::new(
            card.x + CHOOSER_PAD,
            card.y + CHOOSER_PAD,
            card.w - 2 * CHOOSER_PAD,
            CHOOSER_HEAD,
        ),
        Align::Left,
        t.text_dim,
        "ADD A TRACK",
    );
    let row_fill = t.raised.mix(t.text, 0.08);
    for (k, b) in rows.iter().enumerate() {
        cx.canvas.fill_round_rect(*b, t.round(8), row_fill);
        let inner = Rect::new(b.x + 12, b.y + 7, b.w - 24, b.h - 14);
        let (name, about) = inner.take_top(inner.h / 2 + 1);
        cx.text(
            FontId::Bold,
            t.size_small + 1.0,
            name,
            Align::Left,
            t.text,
            ADD_KINDS[k].0,
        );
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            about,
            Align::Left,
            t.text_dim,
            ADD_ABOUT[k],
        );
    }
}

/// A small wave mark in `r` (an audio track's, a Loop track's): four
/// bars of different heights.
pub fn wave_mark(cx: &mut PaintCx, r: Rect, color: Color) {
    for (k, h) in [5, 12, 8, 3].iter().enumerate() {
        let x = r.x + k as i32 * 4;
        cx.canvas
            .fill_round_rect(Rect::new(x, r.y + (r.h - h) / 2, 2, *h), 1, color);
    }
}

/// The two lists hold the same waveforms: the same takes, each read to the
/// same length at the same detail. (A take never changes once recorded,
/// and a recording one grows, so this is enough.)
pub fn same_waves(a: &[TakeWave], b: &[TakeWave]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            (
                x.id,
                x.frames,
                x.channels,
                x.frames_per_column,
                x.columns.len(),
            ) == (
                y.id,
                y.frames,
                y.channels,
                y.frames_per_column,
                y.columns.len(),
            )
        })
}

/// The Song view's blocks for an audio track's `regions`: each with its
/// "take N" tag (N counts the track's takes from 1, oldest first by group
/// and pass; the tag shows on all but the newest), and the frames it spans
/// along `map` at `rate`. `takes` is every take (`AudioTracks::takes`);
/// a region whose take is not among them gets no tag.
/// Track `track`'s takes as its take lanes show them, newest first:
/// each one's name (numbered as [`audio_blocks`] tags them), its stretch
/// of the song along the tempo map, and where `regions` (the track's)
/// play it.
pub fn take_lanes(
    track: usize,
    regions: &[Region],
    takes: &[TakeInfo],
    map: &TempoMap,
    rate: u32,
) -> Vec<TakeLane> {
    let mut mine: Vec<&TakeInfo> = takes.iter().filter(|t| t.track == track).collect();
    mine.sort_by_key(|t| (t.group, t.pass, t.id));
    mine.iter()
        .enumerate()
        .rev()
        .map(|(n, t)| {
            let end = mpfree_audiotracks::tick_after(map, t.anchor, t.frames as f64, rate);
            TakeLane {
                take: t.id,
                name: format!("take {}", n + 1),
                start: t.anchor.ceil() as Tick,
                end: (end.floor() as Tick).max(t.anchor.ceil() as Tick),
                frames: t.frames as f64,
                recording: t.recording,
                used: regions
                    .iter()
                    .filter(|r| r.take == t.id)
                    .map(|r| (r.start, r.end))
                    .collect(),
            }
        })
        .collect()
}

pub fn audio_blocks(
    track: usize,
    regions: &[Region],
    takes: &[TakeInfo],
    map: &TempoMap,
    rate: u32,
) -> Vec<AudioBlock> {
    let mut mine: Vec<&TakeInfo> = takes.iter().filter(|t| t.track == track).collect();
    mine.sort_by_key(|t| (t.group, t.pass, t.id));
    let newest = mine.last().map(|t| t.id);
    regions
        .iter()
        .map(|r| {
            let n = mine.iter().position(|t| t.id == r.take);
            AudioBlock {
                start: r.start,
                end: r.end,
                take: r.take,
                tag: match n {
                    Some(n) if Some(r.take) != newest => format!("take {}", n + 1),
                    _ => String::new(),
                },
                src: r.src,
                frames: frames_from(map, r.start as f64, r.end as f64, rate),
                gain: r.gain,
                fade_in: r.fade_in,
                fade_out: r.fade_out,
                into: r.into,
                left: r.left,
            }
        })
        .collect()
}

impl SongLanes {
    pub fn new() -> SongLanes {
        SongLanes::default()
    }

    /// Take the scroll (and a drag under way) from `old`, the contents
    /// shown before, keeping the first lane in range.
    pub fn keep_scroll(&mut self, old: &SongLanes) {
        self.first_lane = old.first_lane;
        self.drag = old.drag;
        self.swipe = old.swipe;
        self.moving = old.moving;
        self.grip = old.grip;
        if let (Some(card), Some(was)) = (self.report.as_mut(), old.report.as_ref()) {
            card.keep_scroll(was);
        }
    }

    /// Where the playhead's line is drawn at `p`: its left edge, while it
    /// is over the lanes (not the names).
    fn playhead_x(&self, g: &Geo, p: Option<Tick>) -> Option<i32> {
        let x = self.x_of(g.lanes, p?);
        (x >= g.lanes.x + NAME_W && x < g.lanes.right()).then_some(x)
    }

    /// Keeps what a paint of `r`, clipped to `clip`, showed: a whole
    /// paint a copy of the view, a strip the playhead it drew ([`Painted`]).
    fn keep_painted(&self, r: Rect, clip: Rect) {
        let mut p = self.painted.0.borrow_mut();
        if clip.intersect(r) == r {
            *p = Some(Box::new((
                r,
                self.clone_without_waves(),
                wave_keys(&self.waves),
            )));
        } else if let Some(p) = p.as_mut() {
            p.1.playhead = self.playhead;
        }
    }

    /// A copy with no waveforms (they can be large).
    pub fn clone_without_waves(&self) -> SongLanes {
        let waves = Vec::new();
        SongLanes {
            title: self.title.clone(),
            legend: self.legend.clone(),
            alert: self.alert,
            tracks: self.tracks.clone(),
            sounds: self.sounds.clone(),
            blocks: self.blocks.clone(),
            markers: self.markers.clone(),
            passes: self.passes.clone(),
            bar: self.bar,
            beat: self.beat,
            first_bar: self.first_bar,
            bars: self.bars,
            playhead: self.playhead,
            selected: self.selected,
            picks: self.picks.clone(),
            on: self.on,
            rec: self.rec,
            show_button: self.show_button,
            audio: self.audio.clone(),
            waves,
            region: self.region,
            panel: self.panel,
            takes_open: self.takes_open,
            can_add_track: self.can_add_track,
            adding: self.adding,
            swipe: self.swipe,
            moving: self.moving,
            first_lane: self.first_lane,
            drag: self.drag,
            section: self.section,
            grip: self.grip,
            tempo: self.tempo.clone(),
            tempo_base: self.tempo_base,
            tempo_pick: self.tempo_pick,
            report: self.report.clone(),
            over: self.over.clone(),
            painted: Painted::default(),
        }
    }

    /// The clips launched over the song ([`SongLanes::over`]), over
    /// their tracks' lanes: the song's blocks hatched under one that
    /// plays, a band in the track's colour with its name, outlined green;
    /// one waiting only a dashed amber outline. Where the song comes back
    /// (a stop or Back to song waits), an amber dashed line.
    fn paint_over(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let view_end = (self.first_bar + self.bars) * self.bar;
        for o in &self.over {
            let Some((_, color)) = self.tracks.get(o.track) else {
                continue;
            };
            let k = self.track_k(o.track);
            let end = o.until.unwrap_or(view_end).max(o.from + 1);
            let Some(span) = self.span_rect(g, k, o.from, end) else {
                continue;
            };
            let px = t.size_small - 2.0;
            if o.waiting {
                dashed_outline(cx, span, t.warn);
                if span.w > 40 {
                    cx.text(
                        FontId::Sans,
                        px,
                        span.inset_xy(8, 0),
                        Align::Left,
                        t.readable(t.warn, t.surface),
                        &o.name,
                    );
                }
                continue;
            }
            // The song under it waits, silent.
            hatch(cx, span, t.surface);
            let band = span.inset_xy(0, (span.h / 5).max(3));
            let fill = t.raised.mix(*color, 0.62);
            cx.canvas.fill_round_rect(band, t.round(6), fill);
            cx.canvas.stroke_round_rect(band, t.round(6), 2, t.good);
            if band.w > 40 {
                cx.text(
                    FontId::Bold,
                    px,
                    band.inset_xy(8, 0),
                    Align::Left,
                    t.readable(t.text, fill),
                    &o.name,
                );
            }
            if let (true, Some(at)) = (o.back, o.until) {
                let x = self.x_of(g.lanes, at);
                if x >= g.lanes.x + NAME_W && x < g.lanes.right() {
                    let row = self.lane_rect(g, k).unwrap_or(span);
                    let mut y = row.y;
                    while y < row.bottom() {
                        let h = 4.min(row.bottom() - y);
                        cx.canvas.fill_rect(Rect::new(x - 1, y, 2, h), t.warn);
                        y += 7;
                    }
                }
            }
        }
    }

    /// The audio lane whose take lanes show (an index into `audio`), and
    /// how many there are.
    fn open_takes(&self) -> Option<(usize, usize)> {
        let t = self.takes_open?;
        let a = self.audio.iter().position(|l| l.track == t)?;
        Some((a, self.audio[a].takes.len()))
    }

    fn take_count(&self) -> usize {
        self.open_takes().map_or(0, |o| o.1)
    }

    /// Lanes in all: a lane a track, in track order (an audio track's
    /// with its take lanes under it while they are open), then "+ Track"
    /// while there is room for another.
    fn lane_count(&self) -> usize {
        self.tracks.len() + self.take_count() + usize::from(self.can_add_track)
    }

    /// The audio lane of the track in place `row`, if it is an audio
    /// track.
    fn audio_at(&self, row: usize) -> Option<usize> {
        self.audio.iter().position(|l| l.row == row)
    }

    /// The open take lanes: their audio track's place in the list, its
    /// audio lane and how many.
    fn open_row(&self) -> Option<(usize, usize, usize)> {
        self.open_takes().map(|(a, n)| (self.audio[a].row, a, n))
    }

    fn lane(&self, k: usize) -> Option<Lane> {
        let mut j = k;
        if let Some((row, a, n)) = self.open_row() {
            if j > row {
                if j <= row + n {
                    return Some(Lane::Take(a, j - row - 1));
                }
                j -= n;
            }
        }
        if j < self.tracks.len() {
            return Some(match self.audio_at(j) {
                Some(a) => Lane::Audio(a),
                None => Lane::Midi(j),
            });
        }
        (j == self.tracks.len() && self.can_add_track).then_some(Lane::AddTrack)
    }

    /// The lane number of the track in place `row`.
    fn track_k(&self, row: usize) -> usize {
        let before = self.open_row().filter(|o| o.0 < row).map_or(0, |o| o.2);
        row + before
    }

    /// The lane number of audio lane `a` (an index into `audio`).
    fn audio_k(&self, a: usize) -> usize {
        self.track_k(self.audio[a].row)
    }

    /// Lane `k`'s height: take lanes are thin.
    fn height(&self, g: &Geo, k: usize) -> i32 {
        match self.lane(k) {
            Some(Lane::Take(..)) => TAKE_H,
            _ => g.lane_h,
        }
    }

    fn geo(&self, r: Rect) -> Geo {
        // Matches PaintCx::titled_panel with the default theme's padding.
        let body = r.inset(14).take_top(34).1;
        let (rest, tools) = (
            Rect::new(body.x, body.y, body.w, body.h - TOOLS_H - GAP),
            Rect::new(body.x, body.bottom() - TOOLS_H, body.w, TOOLS_H),
        );
        let (ruler, below) = rest.take_top(RULER_H);
        let tempo = Rect::new(below.x, below.y + 2, below.w, TEMPO_H);
        let lanes = Rect::new(
            below.x,
            tempo.bottom() + GAP,
            below.w,
            below.h - 2 - TEMPO_H - GAP,
        );
        // Take lanes keep their height; the others share what is left.
        let takes = self.take_count() as i32;
        let n = (self.lane_count() as i32 - takes).max(1);
        let room = lanes.h - takes * TAKE_H;
        let lane_h = (room / n).clamp(MIN_LANE.min(lanes.h.max(8)), LANE_MAX);
        Geo {
            ruler,
            tempo,
            lanes,
            lane_h,
            tools,
        }
    }

    /// The last lane that can be first: from it to the end, the lanes fit.
    fn max_first(&self, g: &Geo) -> usize {
        let n = self.lane_count();
        let mut used = 0;
        let mut k = n;
        while k > 0 {
            let h = self.height(g, k - 1);
            if used + h > g.lanes.h {
                break;
            }
            used += h;
            k -= 1;
        }
        k.min(n.saturating_sub(1))
    }

    /// The first lane shown, kept in range for `g`.
    fn first(&self, g: &Geo) -> usize {
        self.first_lane.min(self.max_first(g))
    }

    /// Lanes in view from the first.
    fn shown(&self, g: &Geo) -> usize {
        let first = self.first(g);
        (first..self.lane_count())
            .take_while(|&k| self.lane_rect(g, k).is_some())
            .count()
    }

    /// Lane `k`'s rectangle (names included), if it is in view.
    fn lane_rect(&self, g: &Geo, k: usize) -> Option<Rect> {
        let first = self.first(g);
        if k < first || k >= self.lane_count() {
            return None;
        }
        let y = g.lanes.y + (first..k).map(|j| self.height(g, j)).sum::<i32>();
        let h = self.height(g, k);
        (y + h <= g.lanes.bottom()).then(|| Rect::new(g.lanes.x, y, g.lanes.w, h))
    }

    /// The x of song tick `t` in `area` (the lanes past the names).
    fn x_of(&self, area: Rect, t: Tick) -> i32 {
        let span = (self.bars.max(1) * self.bar.max(1)) as f64;
        let from = (self.first_bar * self.bar) as f64;
        area.x + NAME_W + ((t as f64 - from) / span * f64::from(area.w - NAME_W)).round() as i32
    }

    /// The stretch `start..end` of lane `lane`, clipped to the view, as a
    /// block: None if it does not show.
    fn span_rect(&self, g: &Geo, lane: usize, start: Tick, end: Tick) -> Option<Rect> {
        let row = self.lane_rect(g, lane)?;
        let x0 = self.x_of(g.lanes, start).max(g.lanes.x + NAME_W);
        let x1 = self.x_of(g.lanes, end).min(g.lanes.right());
        (x1 - x0 >= 2).then(|| Rect::new(x0, row.y + 2, x1 - x0 - 2, row.h - 4))
    }

    /// Where placement `b` is drawn, clipped to the view, if it shows.
    pub fn block_rect(&self, r: Rect, b: &SongBlock) -> Option<Rect> {
        if b.track >= self.tracks.len() {
            return None;
        }
        self.span_rect(
            &self.geo(r),
            self.track_k(b.track),
            b.start,
            b.start + b.length,
        )
    }

    /// Where region `i` of audio lane `a` (an index into `audio`) is drawn,
    /// clipped to the view, if it shows.
    pub fn region_rect(&self, r: Rect, a: usize, i: usize) -> Option<Rect> {
        let b = self.audio.get(a)?.blocks.get(i)?;
        self.span_rect(&self.geo(r), self.audio_k(a), b.start, b.end)
    }

    /// Audio lane `a`'s header (its part of the name column), if in view.
    pub fn header_rect(&self, r: Rect, a: usize) -> Option<Rect> {
        if a >= self.audio.len() {
            return None;
        }
        let row = self.lane_rect(&self.geo(r), self.audio_k(a))?;
        Some(Rect::new(row.x, row.y, NAME_W - 8, row.h))
    }

    /// Audio lane `a`'s arm dot: the left of its header.
    pub fn arm_rect(&self, r: Rect, a: usize) -> Option<Rect> {
        let h = self.header_rect(r, a)?;
        Some(Rect::new(h.x, h.y, 22, h.h - 4))
    }

    /// The audio lane whose arm dot is under (x, y).
    fn arm_hit(&self, r: Rect, x: i32, y: i32) -> Option<usize> {
        (0..self.audio.len()).find(|&a| self.arm_rect(r, a).is_some_and(|d| d.contains(x, y)))
    }

    /// "+ Track", if it is in view.
    pub fn add_track_rect(&self, r: Rect) -> Option<Rect> {
        if !self.can_add_track {
            return None;
        }
        let g = self.geo(r);
        let row = self.lane_rect(&g, self.lane_count() - 1)?;
        let h = (row.h - 8).min(30);
        Some(Rect::new(row.x, row.y + (row.h - h) / 2, NAME_W - 4, h))
    }

    /// The "+ Track" chooser's rows ([`ADD_KINDS`]), while it is open
    /// and "+ Track" shows: the card beside it, right of the names,
    /// level with it where it fits (as drawn), kept inside the view.
    pub fn chooser_rects(&self, r: Rect) -> Option<[Rect; 3]> {
        if !self.adding {
            return None;
        }
        let b = self.add_track_rect(r)?;
        let x = b.right() + 12;
        let y = (b.y + b.h / 2 - CHOOSER_H / 2)
            .min(r.bottom() - 8 - CHOOSER_H)
            .max(r.y + 8);
        Some(chooser_rows(x, y))
    }

    /// Where Rec song is: the right end of the title line.
    pub fn rec_rect(&self, r: Rect) -> Rect {
        let head = r.inset(14).take_top(26).0;
        Rect::new(head.right() - REC_SONG_W, head.y - 4, REC_SONG_W, 30)
    }

    /// Where the view chip is: the left end of the title line (5.7).
    pub fn view_rect(&self, r: Rect) -> Rect {
        view_chip_rect(r)
    }

    /// Where Show is, while a set is open: left of Rec song.
    pub fn show_rect(&self, r: Rect) -> Option<Rect> {
        let rec = self.rec_rect(r);
        let w = crate::widgets::SHOW_W;
        self.show_button
            .then(|| Rect::new(rec.x - 8 - w, rec.y, w, rec.h))
    }

    /// The zoom chip (6.4), right of the view chip: "Bars", then a part
    /// for each of [`ZOOMS`].
    pub fn zoom_rect(&self, r: Rect) -> Rect {
        let chip = view_chip_rect(r);
        Rect::new(chip.right() + 12, chip.y, ZOOM_W, chip.h)
    }

    /// The zoom chip's part `k`, for [`ZOOMS`]`[k]`.
    pub fn zoom_part_rect(&self, r: Rect, k: usize) -> Rect {
        let z = self.zoom_rect(r);
        Rect::new(
            z.x + ZOOM_LABEL_W + k as i32 * ZOOM_PART_W,
            z.y,
            ZOOM_PART_W,
            z.h,
        )
    }

    /// What a drag snaps to (6.4): the beat while 8 bars or fewer show,
    /// else the bar.
    fn snap(&self) -> Tick {
        if self.bars <= 8 {
            self.beat.clamp(1, self.bar.max(1))
        } else {
            self.bar.max(1)
        }
    }

    /// "bar 5", or "bar 5 beat 3" off the bar line: where a dragged block
    /// starts.
    fn at_text(&self, t: Tick) -> String {
        let bar = self.bar.max(1);
        let beat = self.beat.clamp(1, bar);
        let (b, rest) = (t.div_euclid(bar), t.rem_euclid(bar));
        if rest == 0 {
            format!("bar {}", b + 1)
        } else {
            format!("bar {} beat {}", b + 1, rest / beat + 1)
        }
    }

    /// The buttons along the bottom (the audio region's while one is
    /// picked). Button 0 is the audio region's Split alone: the view chip
    /// took the place of the Clips button that was there (5.7), so
    /// otherwise it has no width and the others start at the left.
    pub fn button_rects(&self, r: Rect) -> [Rect; 6] {
        let g = self.geo(r);
        let widths = if self.region.is_some() {
            [84, 84, 84, 84, 84, 84]
        } else if self.section_buttons() {
            [SPLIT_W, 124, 90, 110, 150, 150]
        } else {
            [SPLIT_W, 104, 128, 128, 112, 80]
        };
        // Duplicate's place, after Song → scenes, while it shows.
        let dup = self.duplicate_shows();
        let mut out = [Rect::new(0, 0, 0, 0); 6];
        let mut x = g.tools.x;
        for (k, w) in widths.iter().enumerate() {
            out[k] = Rect::new(x, g.tools.y, *w, g.tools.h);
            if *w > 0 {
                x += w + 8;
            }
            if k == 3 && dup {
                x += DUPLICATE_W + 8;
            }
        }
        out
    }

    /// Whether Duplicate shows (phase 6): with no audio region, tempo
    /// point or section picked, the last buttons being the blocks'.
    fn duplicate_shows(&self) -> bool {
        self.region.is_none() && self.tempo_pick.is_none() && !self.section_buttons()
    }

    /// Duplicate (phase 6), between Song → scenes and Make unique, while
    /// it shows.
    pub fn duplicate_rect(&self, r: Rect) -> Option<Rect> {
        if !self.duplicate_shows() {
            return None;
        }
        let g = self.geo(r);
        let mut x = g.tools.x;
        for w in [SPLIT_W, 104, 128, 128] {
            x += w + 8;
        }
        Some(Rect::new(x, g.tools.y, DUPLICATE_W, g.tools.h))
    }

    /// The track panel's parts, while one is open: over the lanes, right
    /// of the names, from the top.
    pub fn panel_parts(&self, r: Rect) -> Option<PanelParts> {
        let track = self
            .panel
            .filter(|&t| self.audio.iter().any(|l| l.track == t))?;
        let g = self.geo(r);
        let mixed = self
            .audio
            .iter()
            .any(|l| l.track == track && l.mix.is_some());
        let w = (g.lanes.w - NAME_W).min(PANEL_W);
        let rows_h = if mixed {
            PANEL_H + PANEL_ROW + 5
        } else {
            PANEL_H
        };
        let h = (g.lanes.h - 4).min(rows_h);
        // Under the track's lane if it fits, else over it, so the lane
        // stays in view; else on the side with more room, and at the top
        // if the lane is scrolled away.
        let a = self.audio.iter().position(|l| l.track == track)?;
        let (top, bottom) = (g.lanes.y + 2, g.lanes.bottom() - 2);
        let y = match self.lane_rect(&g, self.audio_k(a)) {
            Some(row) if row.bottom() + 2 + h <= bottom => row.bottom() + 2,
            Some(row) if row.y - 2 - h >= top => row.y - 2 - h,
            Some(row) if bottom - row.bottom() >= row.y - top => bottom - h,
            _ => top,
        };
        let panel = Rect::new(g.lanes.x + NAME_W, y, w, h);
        let inner = panel.inset(12);
        let row = |k: i32| inner.y + 30 + k * (PANEL_ROW + 5);
        let at = |x: i32, k: i32, w: i32| Rect::new(x, row(k), w, PANEL_ROW);
        let close = Rect::new(inner.right() - 72, inner.y - 2, 72, 28);
        let x1 = inner.x + PANEL_LABEL;
        let arm = at(inner.x, 0, 104);
        let mute = at(arm.right() + 8, 0, 72);
        let solo = at(mute.right() + 8, 0, 72);
        let takes = at(inner.right() - 90, 0, 90);
        let inputs: [Rect; 4] = std::array::from_fn(|k| at(x1 + k as i32 * 48, 1, 44));
        let mix = at(x1 + 4 * 48, 1, 44);
        let mx = mix.right() + 16 + PANEL_LABEL + 12;
        let monitors = std::array::from_fn(|k| at(mx + k as i32 * 54, 1, 50));
        // Step 9: From and Effects take a row of their own, under Input.
        let (from, effects) = if mixed {
            let from = [at(x1, 2, 36), at(x1 + 40, 2, 160), at(x1 + 204, 2, 36)];
            let ex = from[2].right() + 16 + 64 + 8;
            (Some(from), Some([at(ex, 2, 72), at(ex + 76, 2, 72)]))
        } else {
            (None, None)
        };
        let lk = if mixed { 3 } else { 2 };
        let level = [at(x1, lk, 36), at(x1 + 40, lk, 72), at(x1 + 116, lk, 36)];
        let px = level[2].right() + 12 + 40;
        let pan = [at(px, lk, 36), at(px + 40, lk, 52), at(px + 96, lk, 36)];
        let strip = at(inner.right() - PANEL_STRIP_W, lk, PANEL_STRIP_W);
        let note = Rect::new(inner.x, row(lk) + PANEL_ROW + 2, inner.w, 14);
        Some(PanelParts {
            track,
            panel,
            close,
            arm,
            mute,
            solo,
            takes,
            inputs,
            mix,
            monitors,
            from,
            effects,
            level,
            pan,
            strip,
            note,
        })
    }

    /// The track panel: its name, Close, then a row a control.
    fn paint_panel(&self, cx: &mut PaintCx, p: &PanelParts) {
        let t = cx.theme.clone();
        let Some(lane) = self.audio.iter().find(|l| l.track == p.track) else {
            return;
        };
        // A shadow under it, so it lifts off the lanes.
        let shadow = Rect::new(p.panel.x + 2, p.panel.y + 4, p.panel.w, p.panel.h);
        cx.canvas.fill_round_rect(shadow, t.round(12), t.bg);
        let old = cx.canvas.clip();
        cx.canvas.set_clip(old.intersect(p.panel));
        let fill = t.surface.mix(t.raised, 0.35);
        cx.canvas.fill_round_rect(p.panel, t.round(12), fill);
        cx.canvas
            .stroke_round_rect(p.panel, t.round(12), 1, t.outline);
        let inner = p.panel.inset(12);
        cx.canvas
            .fill_round_rect(Rect::new(inner.x, inner.y + 4, 4, 18), 2, lane.color);
        let state = if lane.recording.is_some() {
            " · recording"
        } else if lane.armed {
            " · armed"
        } else {
            ""
        };
        cx.text(
            FontId::Bold,
            t.size_small,
            Rect::new(inner.x + 12, inner.y - 2, p.close.x - inner.x - 16, 28),
            Align::Left,
            t.readable(lane.color.mix(t.text, 0.25), fill),
            &format!("{}{state}", lane.name),
        );
        let button = |cx: &mut PaintCx, b: Rect, word: &str, on: bool, tone: Color| {
            cx.canvas
                .fill_round_rect(b, t.round(8), if on { tone } else { t.raised });
            let ink = if on { t.ink_on(tone) } else { t.text };
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                b,
                Align::Center,
                ink,
                word,
            );
        };
        button(cx, p.close, "Close", false, t.accent);
        button(
            cx,
            p.arm,
            if lane.armed { "● Armed" } else { "○ Arm" },
            lane.armed,
            t.bad,
        );
        button(cx, p.mute, "Mute", lane.mute, t.warn);
        button(cx, p.solo, "Solo", lane.solo, t.good);
        button(cx, p.takes, "Takes", false, t.accent);
        let label = |cx: &mut PaintCx, b: Rect, word: &str| {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                b,
                Align::Left,
                t.text_dim,
                word,
            );
        };
        let lab = |b: Rect| Rect::new(inner.x, b.y, PANEL_LABEL - 8, b.h);
        label(cx, lab(p.inputs[0]), "Input");
        let line = lane.mix.is_none();
        for (k, b) in p.inputs.iter().enumerate() {
            let on = line && lane.input == INPUTS[k].1;
            button(cx, *b, INPUTS[k].0, on, t.accent);
        }
        button(cx, p.mix, "Mix", !line, t.accent);
        let mon = p.monitors[0];
        let mon_label = Rect::new(mon.x - PANEL_LABEL - 12, mon.y, PANEL_LABEL + 12, mon.h);
        if let (Some(m), Some(from), Some(fx)) = (&lane.mix, p.from, p.effects) {
            // A track recording from the mix never plays it through, so
            // the monitor modes would change nothing: said in their place.
            let said = Rect::new(
                mon_label.x,
                mon.y,
                p.monitors[2].right() - mon_label.x,
                mon.h,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                said,
                Align::Left,
                t.text_dim,
                "Not played through",
            );
            label(cx, lab(from[0]), "From");
            button(cx, from[0], "‹", false, t.accent);
            button(cx, from[2], "›", false, t.accent);
            cx.canvas.fill_round_rect(from[1], 8, t.surface);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                from[1].inset_xy(6, 0),
                Align::Center,
                t.text,
                &m.name,
            );
            label(cx, Rect::new(fx[0].x - 72, fx[0].y, 64, fx[0].h), "Effects");
            button(cx, fx[0], "Before", !m.after_effects, t.accent);
            button(cx, fx[1], "After", m.after_effects, t.accent);
        } else {
            label(cx, mon_label, "Monitor");
            for (k, b) in p.monitors.iter().enumerate() {
                button(
                    cx,
                    *b,
                    MONITORS[k].0,
                    lane.monitor == MONITORS[k].1,
                    t.accent,
                );
            }
        }
        label(cx, lab(p.level[0]), "Level");
        let pan_label = Rect::new(p.pan[0].x - 40, p.pan[0].y, 36, p.pan[0].h);
        label(cx, pan_label, "Pan");
        let level = if lane.volume <= 0.0 {
            "off".to_string()
        } else {
            format!("{:.1} dB", 20.0 * lane.volume.log10())
        };
        let pan_n = (lane.pan.abs() * 100.0).round() as i32;
        let pan = match pan_n {
            0 => "C".to_string(),
            _ if lane.pan < 0.0 => format!("L{pan_n}"),
            _ => format!("R{pan_n}"),
        };
        for (parts, value) in [(&p.level, level), (&p.pan, pan)] {
            button(cx, parts[0], "−", false, t.accent);
            button(cx, parts[2], "+", false, t.accent);
            cx.text(
                FontId::Bold,
                t.size_small,
                parts[1],
                Align::Center,
                t.text,
                &value,
            );
        }
        // Level and pan are the track's strip's (phase 1b): the way to it,
        // and the line saying so.
        button(cx, p.strip, "Its strip in Mix ›", false, t.accent);
        cx.text(
            FontId::Sans,
            t.size_small - 3.0,
            p.note,
            Align::Left,
            t.text_dim,
            PANEL_NOTE,
        );
        cx.canvas.set_clip(old);
    }

    /// What a touch at (x, y) on the open panel asks for. Some(None) on
    /// the panel's empty space: it takes the touch and asks for nothing.
    fn panel_touch(&self, r: Rect, x: i32, y: i32) -> Option<Option<EventKind>> {
        let p = self.panel_parts(r)?;
        if !p.panel.contains(x, y) {
            return None;
        }
        let hit = |b: &Rect| b.contains(x, y);
        let tab = |k: usize| Some(EventKind::Tab(k));
        let step = |index: usize, delta: i32| Some(EventKind::Step { index, delta });
        Some(if hit(&p.close) {
            tab(PANEL_CLOSE)
        } else if hit(&p.arm) {
            Some(EventKind::Toggle(p.track))
        } else if hit(&p.mute) {
            tab(PANEL_MUTE)
        } else if hit(&p.solo) {
            tab(PANEL_SOLO)
        } else if hit(&p.takes) {
            tab(PANEL_TAKES)
        } else if let Some(k) = p.inputs.iter().position(hit) {
            tab(PANEL_INPUT + k)
        } else if hit(&p.mix) {
            tab(PANEL_MIX)
        } else if let Some(from) = p.from.filter(|f| f.iter().any(hit)) {
            match () {
                _ if hit(&from[0]) => tab(PANEL_FROM_LESS),
                _ if hit(&from[2]) => tab(PANEL_FROM_MORE),
                _ => None,
            }
        } else if let Some(fx) = p.effects.filter(|f| f.iter().any(hit)) {
            tab(if hit(&fx[0]) {
                PANEL_BEFORE
            } else {
                PANEL_AFTER
            })
        } else if let Some(k) = p.monitors.iter().position(hit).filter(|_| p.from.is_none()) {
            tab(PANEL_MONITOR + k)
        } else if hit(&p.level[0]) {
            step(PANEL_LEVEL, -1)
        } else if hit(&p.level[2]) {
            step(PANEL_LEVEL, 1)
        } else if hit(&p.pan[0]) {
            step(PANEL_PAN, -1)
        } else if hit(&p.pan[2]) {
            step(PANEL_PAN, 1)
        } else if hit(&p.strip) {
            tab(PANEL_STRIP)
        } else {
            None
        })
    }

    fn wave_of(&self, take: u16) -> Option<&TakeWave> {
        self.waves
            .iter()
            .find(|w| w.id == take && w.frames_per_column > 0)
    }

    /// Take `take`'s waveform in `wave` (its columns from `cols.x` to
    /// `cols.right()`): the column at x shows the frames from `src + (x -
    /// x0) * per_px` on. `peaks` draws each column's peak up from the
    /// bottom over all of `wave`'s height (a block of an audio clip's
    /// wave under its header strip, where a lane leaves little room),
    /// else its swing either way of the middle.
    #[allow(clippy::too_many_arguments)]
    fn paint_wave(
        &self,
        cx: &mut PaintCx,
        take: u16,
        wave: Rect,
        cols: Rect,
        x0: i32,
        src: f64,
        per_px: f64,
        ink: Color,
        peaks: bool,
    ) {
        let Some(w) = self.wave_of(take) else {
            return;
        };
        let mid = wave.y + wave.h / 2;
        let half = f64::from((wave.h - 4) / 2);
        for x in cols.x..cols.right() {
            let f0 = src + f64::from(x - x0) * per_px;
            let f1 = f0 + per_px;
            let c0 = (f0.max(0.0) / w.frames_per_column as f64) as usize;
            let c1 = ((f1.max(0.0) / w.frames_per_column as f64).ceil() as usize).max(c0 + 1);
            let (mut lo, mut hi) = (0.0f32, 0.0f32);
            if f1 > 0.0 && f0 < w.frames as f64 {
                for &(l, h) in w.columns.iter().take(c1).skip(c0) {
                    lo = lo.min(l);
                    hi = hi.max(h);
                }
            }
            let (top, bottom) = if peaks {
                let up = f64::from(hi.max(-lo).clamp(0.0, 1.0)) * f64::from(wave.h - 1);
                (wave.bottom() - up.round() as i32, wave.bottom())
            } else {
                (
                    mid - (f64::from(hi.clamp(0.0, 1.0)) * half).round() as i32,
                    mid - (f64::from(lo.clamp(-1.0, 0.0)) * half).round() as i32,
                )
            };
            cx.canvas
                .fill_rect(Rect::new(x, top, 1, (bottom - top).max(1)), ink);
        }
    }

    /// Take lane `k` (an index into its `takes`) of audio lane `a`, in
    /// `row`: the take's stretch with its waveform, brighter where the
    /// track plays it, and a swipe under way.
    fn paint_take(&self, cx: &mut PaintCx, g: &Geo, a: usize, k: usize, row: Rect) {
        let t = cx.theme.clone();
        let lane = &self.audio[a];
        let take = &lane.takes[k];
        let body = Rect::new(row.x + NAME_W, row.y, row.w - NAME_W, row.h);
        let old = cx.canvas.clip();
        cx.canvas.set_clip(old.intersect(body));
        let lane_k = self.audio_k(a) + 1 + k;
        if let Some(br) = self.span_rect(g, lane_k, take.start, take.end.max(take.start + 1)) {
            cx.canvas
                .fill_round_rect(br, 4, t.raised.mix(lane.color, 0.16));
            for &(from, to) in &take.used {
                if let Some(ur) = self.span_rect(g, lane_k, from, to) {
                    cx.canvas
                        .fill_round_rect(ur, 4, t.raised.mix(lane.color, 0.45));
                }
            }
            let x0 = self.x_of(g.lanes, take.start);
            let x1 = self.x_of(g.lanes, take.end).max(x0 + 1);
            let per_px = take.frames / f64::from(x1 - x0);
            let ink = lane.color.mix(t.text, 0.45);
            self.paint_wave(
                cx,
                take.take,
                br.inset_xy(0, 1),
                br,
                x0,
                0.0,
                per_px,
                ink,
                false,
            );
        }
        if let Some((id, x0, x1)) = self.swipe {
            if id == take.take {
                let (l, r) = (x0.min(x1), x0.max(x1));
                let sr = Rect::new(l, row.y + 1, (r - l).max(2), row.h - 2);
                cx.canvas.stroke_round_rect(sr, 4, 2, t.accent);
            }
        }
        cx.canvas.set_clip(old);
    }

    /// The song tick at x in `area` (the lanes past the names).
    fn tick_at(&self, area: Rect, x: i32) -> Tick {
        let span = (self.bars.max(1) * self.bar.max(1)) as f64;
        let from = (self.first_bar * self.bar) as f64;
        let w = f64::from((area.w - NAME_W).max(1));
        (from + f64::from(x - area.x - NAME_W) / w * span).round() as Tick
    }

    /// Placement `index` (its index in the arrangement).
    fn block(&self, index: usize) -> Option<&SongBlock> {
        self.blocks.get(index)
    }

    /// The song ticks the view shows: its first bar to past its last.
    fn ticks_shown(&self) -> (Tick, Tick) {
        let bar = self.bar.max(1);
        (
            self.first_bar * bar,
            (self.first_bar + self.bars.max(1)) * bar,
        )
    }

    /// The blocks in view, lane by lane, with where each is drawn (clipped
    /// to the view): the only blocks a frame draws or hit-tests (Arrange
    /// refactor 2.4), found by lane and time, so a long song costs no more
    /// than a short one.
    fn blocks_in_view<'a>(
        &'a self,
        g: &'a Geo,
    ) -> impl Iterator<Item = (&'a SongBlock, Rect)> + 'a {
        let (from, to) = self.ticks_shown();
        let first = self.first(g);
        let lanes = first..first + self.shown(g);
        lanes
            .filter_map(move |k| match self.lane(k) {
                Some(Lane::Midi(t)) => Some((k, t)),
                // An audio track's lane holds blocks of audio clips too
                // (phase 2, step 2d).
                Some(Lane::Audio(a)) => Some((k, self.audio[a].row)),
                _ => None,
            })
            .flat_map(move |(k, t)| {
                self.blocks.between(t, from, to).filter_map(move |b| {
                    Some((b, self.span_rect(g, k, b.start, b.start + b.length)?))
                })
            })
    }

    /// Section `k`'s name on the ruler, where a finger picks it up, if it
    /// shows.
    pub fn marker_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let g = self.geo(r);
        let at = self.markers.get(k)?.0;
        let x = self.x_of(g.ruler, at);
        if x < g.ruler.x + NAME_W || x >= g.ruler.right() {
            return None;
        }
        let next = self
            .markers
            .get(k + 1)
            .map_or(g.ruler.right(), |m| self.x_of(g.ruler, m.0))
            .min(g.ruler.right());
        Some(Rect::new(
            x - 4,
            g.ruler.y,
            (next - x).clamp(28, 120),
            RULER_H + GAP,
        ))
    }

    /// Whether the last four buttons are the picked section's
    /// ([`SECTION_BUTTONS`]): a section picked, and no audio region or
    /// tempo point.
    pub fn section_buttons(&self) -> bool {
        self.region.is_none()
            && self.tempo_pick.is_none()
            && self.section.is_some_and(|k| k < self.markers.len())
    }

    /// The picked section's grip on the ruler, left of its marker, if its
    /// marker shows: [`GRIP_W`] wide, the ruler's height.
    pub fn grip_rect(&self, r: Rect) -> Option<Rect> {
        let k = self.section?;
        let g = self.geo(r);
        let x = self.x_of(g.ruler, self.markers.get(k)?.0);
        if x < g.ruler.x + NAME_W || x >= g.ruler.right() {
            return None;
        }
        let left = (x - GRIP_W).max(g.ruler.x + NAME_W);
        (x - left >= 12).then(|| Rect::new(left, g.ruler.y, x - left, RULER_H))
    }

    /// The ruler right of the name column, where a tap adds a marker.
    pub fn ruler_rect(&self, r: Rect) -> Rect {
        let g = self.geo(r);
        Rect::new(g.ruler.x + NAME_W, g.ruler.y, g.ruler.w - NAME_W, RULER_H)
    }

    /// Where the ruler tapped or the grip held lands: the nearest bar
    /// line, never before bar 1.
    fn ruler_landing(&self, r: Rect, gr: &Grip) -> Tick {
        let dt = match gr.held {
            Held::MarkerGrip { dt, .. } => dt,
            _ => 0,
        };
        let bar = self.bar.max(1);
        let t = self.tick_at(self.geo(r).lanes, gr.at.0) - dt;
        ((t + bar / 2).div_euclid(bar) * bar).max(0)
    }

    /// The picked placement's end grip, if its end shows.
    pub fn end_rect(&self, r: Rect) -> Option<Rect> {
        let b = self.block(self.selected?)?;
        let g = self.geo(r);
        let x = self.x_of(g.lanes, b.start + b.length);
        if x <= g.lanes.x + NAME_W || x > g.lanes.right() {
            return None;
        }
        let br = self.block_rect(r, b)?;
        Some(Rect::new(br.right() - 16, br.y, 28, br.h))
    }

    /// The picked placement's start grip (trim from the left, phase 6),
    /// if its start shows: two bars inside its left end.
    pub fn start_rect(&self, r: Rect) -> Option<Rect> {
        let b = self.block(self.selected?)?;
        let g = self.geo(r);
        let x = self.x_of(g.lanes, b.start);
        if x < g.lanes.x + NAME_W || x >= g.lanes.right() {
            return None;
        }
        let br = self.block_rect(r, b)?;
        Some(Rect::new(br.x - 12, br.y, 28, br.h))
    }

    /// Where section edge `j` is: section `j`'s start, or for the markers'
    /// count the last section's end (the song's end in whole bars from
    /// its marker, as `Arrangement::section` has it).
    fn edge_tick(&self, j: usize) -> Option<Tick> {
        if let Some(m) = self.markers.get(j) {
            return Some(m.0);
        }
        let last = self.markers.last()?.0;
        let end = self.blocks.end();
        let bar = self.bar.max(1);
        Some(last + ((end - last).max(1) + bar - 1) / bar * bar)
    }

    /// The bars scroll while a dragged thing is held at this edge.
    fn edge_at(&self, r: Rect, x: i32) -> Option<usize> {
        let g = self.geo(r);
        if x < g.lanes.x + NAME_W + EDGE_PX && self.first_bar > 0 {
            Some(BARS_BACK)
        } else if x > g.lanes.right() - EDGE_PX {
            Some(BARS_ON)
        } else {
            None
        }
    }

    /// Where a held block lands: the track whose lane is under the finger
    /// (an audio track's too: the time engine says why it can't go
    /// there), and its start snapped to the nearest bar. None off the
    /// tracks' lanes.
    fn block_landing(&self, r: Rect, gr: &Grip) -> Option<(usize, Tick)> {
        let Held::Block { dt, .. } = gr.held else {
            return None;
        };
        let g = self.geo(r);
        let y = gr.at.1;
        let k = (0..self.lane_count()).find(|&k| {
            self.lane_rect(&g, k)
                .is_some_and(|l| (l.y..l.bottom()).contains(&y))
        })?;
        let track = match self.lane(k)? {
            Lane::Midi(t) => t,
            Lane::Audio(a) => self.audio[a].row,
            _ => return None,
        };
        let snap = self.snap();
        let t = self.tick_at(g.lanes, gr.at.0) - dt;
        Some((track, ((t + snap / 2).div_euclid(snap) * snap).max(0)))
    }

    /// How long the picked block becomes: its end where the grip is,
    /// snapped to the nearest bar, a bar line past its start at least.
    fn end_landing(&self, r: Rect, gr: &Grip) -> Option<Tick> {
        let Held::End { index, dt } = gr.held else {
            return None;
        };
        let b = self.block(index)?;
        let g = self.geo(r);
        let bar = self.snap();
        let t = self.tick_at(g.lanes, gr.at.0) - dt;
        let end = (t + bar / 2).div_euclid(bar) * bar;
        let least = (b.start.div_euclid(bar) + 1) * bar;
        Some(end.max(least) - b.start)
    }

    /// Where the picked block starts once trimmed from the left: its
    /// start where the grip is, snapped to the nearest bar, up to the last
    /// bar line before its end, and back no further than the end of the
    /// block before it on its lane (or 0).
    fn start_landing(&self, r: Rect, gr: &Grip) -> Option<Tick> {
        let Held::Start { index, dt } = gr.held else {
            return None;
        };
        let b = self.block(index)?;
        let g = self.geo(r);
        let bar = self.snap();
        let t = self.tick_at(g.lanes, gr.at.0) - dt;
        let start = (t + bar / 2).div_euclid(bar) * bar;
        let most = (b.start + b.length - 1).div_euclid(bar) * bar;
        let least = self
            .blocks
            .between(b.track, 0, b.start)
            .filter(|o| o.index != index && o.start < b.start)
            .map(|o| o.start + o.length)
            .max()
            .unwrap_or(0)
            .min(b.start);
        Some(start.min(most).max(least))
    }

    /// How far a slide moves what the held block plays: the finger's way
    /// along the lane in song ticks, snapped to a 1/16 (phase 6).
    fn slide_by(&self, r: Rect, gr: &Grip) -> Tick {
        let g = self.geo(r);
        let step = (self.bar / 16).max(1);
        let d = self.tick_at(g.lanes, gr.at.0) - self.tick_at(g.lanes, gr.from.0);
        (d + step / 2).div_euclid(step) * step
    }

    /// A slide's amount in words: "+1 beat", "−3/16", "+2 bars".
    fn slide_text(&self, by: Tick) -> String {
        let step = (self.bar / 16).max(1);
        let n = by / step;
        let sign = if n < 0 { "−" } else { "+" };
        let n = n.abs();
        match n {
            _ if n % 16 == 0 && n > 0 => {
                let k = n / 16;
                format!("{sign}{k} bar{}", if k == 1 { "" } else { "s" })
            }
            _ if n % 4 == 0 && n > 0 => {
                let k = n / 4;
                format!("{sign}{k} beat{}", if k == 1 { "" } else { "s" })
            }
            _ => format!("{sign}{n}/16"),
        }
    }

    /// Where a held section goes: the section edge nearest where its start
    /// is now.
    fn section_gap(&self, r: Rect, gr: &Grip) -> Option<usize> {
        let Held::Section { dt, .. } = gr.held else {
            return None;
        };
        let t = self.tick_at(self.geo(r).lanes, gr.at.0) - dt;
        (0..=self.markers.len())
            .min_by_key(|&j| self.edge_tick(j).map_or(Tick::MAX, |e| (e - t).abs()))
    }

    /// The box a finger drags across the lanes (6.5), from where it went
    /// down to where it is, right of the names and inside the lanes.
    fn box_rect(&self, r: Rect, gr: &Grip) -> Rect {
        let g = self.geo(r);
        let x0 = gr.from.0.min(gr.at.0).max(g.lanes.x + NAME_W);
        let x1 = gr.from.0.max(gr.at.0).min(g.lanes.right());
        let y0 = gr.from.1.min(gr.at.1).max(g.lanes.y);
        let y1 = gr.from.1.max(gr.at.1).min(g.lanes.bottom());
        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }

    /// The first and last tracks whose lanes the box crosses; None when
    /// it crosses none (only "+ Track" or take lanes).
    fn box_tracks(&self, r: Rect, gr: &Grip) -> Option<(usize, usize)> {
        let g = self.geo(r);
        let b = self.box_rect(r, gr);
        let tracks: Vec<usize> = (0..self.lane_count())
            .filter(|&k| {
                self.lane_rect(&g, k)
                    .is_some_and(|l| l.y < b.bottom().max(b.y + 1) && l.bottom() > b.y)
            })
            .filter_map(|k| match self.lane(k)? {
                Lane::Midi(t) => Some(t),
                Lane::Audio(a) => Some(self.audio[a].row),
                _ => None,
            })
            .collect();
        Some((*tracks.iter().min()?, *tracks.iter().max()?))
    }

    /// What letting go of `gr` reports: see the module notes.
    fn let_go(&self, r: Rect, gr: &Grip) -> Option<EventKind> {
        match gr.held {
            Held::Box if !gr.lifted => Some(EventKind::Select(NONE)),
            Held::Box => {
                let g = self.geo(r);
                // Under the last lane: it picks nothing, as a tap does.
                let Some((top, bottom)) = self.box_tracks(r, gr) else {
                    return Some(EventKind::Select(NONE));
                };
                let (x0, x1) = (gr.from.0.min(gr.at.0), gr.from.0.max(gr.at.0));
                let left = x0.max(g.lanes.x + NAME_W);
                Some(EventKind::Span {
                    index: BOX + top * 0x100 + bottom,
                    from: self.tick_at(g.lanes, left).max(0),
                    to: self.tick_at(g.lanes, x1.min(g.lanes.right())),
                })
            }
            Held::Block { index, .. } if !gr.lifted => Some(EventKind::Select(index)),
            Held::End { .. } if !gr.lifted => None,
            Held::Start { .. } if !gr.lifted => None,
            Held::Slide { index } => {
                let by = self.slide_by(r, gr);
                (by != 0).then_some(EventKind::Place {
                    index,
                    row: SLIDE_ROW,
                    at: by,
                })
            }
            Held::Start { index, .. } => {
                let b = self.block(index)?;
                let start = self.start_landing(r, gr)?;
                (start != b.start).then_some(EventKind::Place {
                    index,
                    row: START_ROW,
                    at: start,
                })
            }
            Held::Section { k, .. } if !gr.lifted => Some(EventKind::Tab(SECTION_TAP + k)),
            Held::Block { index, .. } => {
                let b = self.block(index)?;
                let (track, start) = self.block_landing(r, gr)?;
                ((track, start) != (b.track, b.start)).then_some(EventKind::Place {
                    index,
                    row: track,
                    at: start,
                })
            }
            Held::End { index, .. } => {
                let b = self.block(index)?;
                let length = self.end_landing(r, gr)?;
                (length != b.length).then_some(EventKind::Stretch { index, to: length })
            }
            Held::Section { k, .. } => self
                .section_gap(r, gr)
                .map(|gap| EventKind::Drop { from: k, to: gap }),
            Held::Tempo { k: Some(k), .. } if !gr.lifted => Some(EventKind::Cell {
                row: TEMPO_ROW,
                col: k,
            }),
            Held::Tempo { k: None, .. } if !gr.lifted => Some(EventKind::Place {
                index: NONE,
                row: TEMPO_ROW,
                at: self.tempo_landing(r, gr),
            }),
            Held::Tempo { k: Some(k), .. } => {
                let at = self.tempo_landing(r, gr);
                (self.tempo.get(k)?.0 != at).then_some(EventKind::Place {
                    index: k,
                    row: TEMPO_ROW,
                    at,
                })
            }
            // A swipe along the empty lane does nothing.
            Held::Tempo { k: None, .. } => None,
            Held::Ruler if !gr.lifted => Some(EventKind::Place {
                index: NONE,
                row: RULER_ROW,
                at: self.ruler_landing(r, gr),
            }),
            Held::Ruler => None,
            Held::MarkerGrip { .. } if !gr.lifted => None,
            Held::MarkerGrip { k, .. } => {
                let at = self.ruler_landing(r, gr);
                (self.markers.get(k)?.0 != at).then_some(EventKind::Place {
                    index: k,
                    row: RULER_ROW,
                    at,
                })
            }
        }
    }

    /// Where a held tempo point lands, or where a tap on the tempo lane
    /// adds one: the nearest bar line, never before bar 1.
    fn tempo_landing(&self, r: Rect, gr: &Grip) -> Tick {
        let Held::Tempo { dt, .. } = gr.held else {
            return 0;
        };
        let bar = self.bar.max(1);
        let t = self.tick_at(self.geo(r).lanes, gr.at.0) - dt;
        ((t + bar / 2).div_euclid(bar) * bar).max(0)
    }

    /// The tempo lane where the points go (right of the name column).
    pub fn tempo_rect(&self, r: Rect) -> Rect {
        let g = self.geo(r);
        Rect::new(g.tempo.x + NAME_W, g.tempo.y, g.tempo.w - NAME_W, g.tempo.h)
    }

    /// The lowest and highest tempo the lane shows: the song's, with room
    /// around them (10 BPM each way when they are all one).
    fn tempo_range(&self) -> (f64, f64) {
        let all = self
            .tempo
            .iter()
            .map(|p| p.1)
            .chain(std::iter::once(self.tempo_base));
        let lo = all.clone().fold(f64::INFINITY, f64::min);
        let hi = all.fold(f64::NEG_INFINITY, f64::max);
        if hi - lo < 1.0 {
            (lo - 10.0, hi + 10.0)
        } else {
            (lo, hi)
        }
    }

    /// The height in the lane of a tempo.
    fn tempo_y(&self, lane: Rect, bpm: f64) -> f32 {
        let (lo, hi) = self.tempo_range();
        let k = ((bpm - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
        (lane.bottom() - 5) as f32 - k * (lane.h - 10) as f32
    }

    /// Tempo point `k`'s dot, if it shows, and the area around it a
    /// finger picks it up by.
    pub fn tempo_point_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let lane = self.tempo_rect(r);
        let (at, bpm, _) = *self.tempo.get(k)?;
        let x = self.x_of(self.geo(r).lanes, at);
        if x < lane.x || x > lane.right() {
            return None;
        }
        let y = self.tempo_y(lane, bpm).round() as i32;
        Some(
            Rect::new(x - TEMPO_HIT, y - TEMPO_HIT, 2 * TEMPO_HIT, 2 * TEMPO_HIT).intersect(
                Rect::new(lane.x - TEMPO_HIT, lane.y, lane.w + 2 * TEMPO_HIT, lane.h),
            ),
        )
    }

    /// Where each pass after the first of a linked song's part starts
    /// (4b.4): a dashed line in a faint warning colour across the ruler
    /// and the lanes, under the blocks, and "pass 2 of 3" on the ruler
    /// where it has room before the next line.
    fn paint_passes(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let line = t.warn.mix(t.surface, 0.45);
        let ink = t.readable(t.warn.mix(t.surface, 0.25), t.surface);
        let size = t.size_small - 2.0;
        let (top, bottom) = (g.ruler.y + 14, g.lanes.bottom());
        for (k, (at, name)) in self.markers.iter().enumerate() {
            let Some((n, pass)) = self.passes.get(k).copied().flatten() else {
                continue;
            };
            let next = self.markers.get(k + 1).map(|m| m.0);
            // The part's name, "Verse ×2", ends here on the ruler.
            let named = self.x_of(g.ruler, *at)
                + 4
                + cx.measure(FontId::Bold, size, &format!("{name} ×{n}"))
                + 6;
            for j in 1..Tick::from(n) {
                let tick = at + j * pass.max(1);
                let x = self.x_of(g.ruler, tick);
                if x < g.ruler.x + NAME_W || x >= g.ruler.right() {
                    continue;
                }
                let mut y = top;
                while y < bottom {
                    let h = 5.min(bottom - y);
                    cx.canvas.fill_rect(Rect::new(x, y, 1, h), line);
                    y += 9;
                }
                let label = format!("pass {} of {n}", j + 1);
                let after = match j + 1 < Tick::from(n) {
                    true => Some(tick + pass),
                    false => next,
                };
                let room = after.map_or(g.ruler.right(), |a| self.x_of(g.ruler, a)) - x - 8;
                if x >= named && cx.measure(FontId::Sans, size, &label) <= room {
                    cx.text(
                        FontId::Sans,
                        size,
                        Rect::new(x + 4, g.ruler.y + 12, room, 14),
                        Align::Left,
                        ink,
                        &label,
                    );
                }
            }
        }
    }

    /// The tempo lane: "Tempo" in the name column, the song's tempo as a
    /// line across the bars (flat from each point, sloping where it
    /// ramps), a dot on each point with its tempo beside it, the picked
    /// one ringed.
    fn paint_tempo(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            Rect::new(g.tempo.x, g.tempo.y, NAME_W - 8, g.tempo.h),
            Align::Left,
            t.text_dim,
            "Tempo",
        );
        let lane = Rect::new(g.tempo.x + NAME_W, g.tempo.y, g.tempo.w - NAME_W, g.tempo.h);
        cx.canvas
            .fill_round_rect(lane, 4, t.raised.mix(t.surface, 0.6));
        let from = self.first_bar * self.bar;
        let to = from + self.bars.max(1) * self.bar.max(1);
        // The line: a stretch from each change to the next.
        let mut pts: Vec<(f32, f32)> = Vec::new();
        let first = self.tempo.first().map_or(Tick::MAX, |p| p.0);
        let mut stretches = Vec::with_capacity(self.tempo.len() + 1);
        if first > 0 {
            stretches.push((Tick::MIN, first, self.tempo_base, self.tempo_base));
        }
        for (i, &(at, bpm, ramp)) in self.tempo.iter().enumerate() {
            let next = self.tempo.get(i + 1);
            let end = next.map_or(Tick::MAX, |n| n.0);
            let goal = match next {
                Some(n) if ramp => n.1,
                _ => bpm,
            };
            stretches.push((at, end, bpm, goal));
        }
        for (a, b, b0, b1) in stretches {
            let (x0t, x1t) = (a.max(from), b.min(to));
            if x1t <= x0t {
                continue;
            }
            let at = |tick: Tick| {
                if b == Tick::MAX || a == Tick::MIN {
                    b0
                } else {
                    b0 + (b1 - b0) * (tick - a) as f64 / (b - a) as f64
                }
            };
            pts.push((self.x_of(g.lanes, x0t) as f32, self.tempo_y(lane, at(x0t))));
            pts.push((self.x_of(g.lanes, x1t) as f32, self.tempo_y(lane, at(x1t))));
        }
        let mut line = mpfree_ui::Shape::new(lane);
        line.polyline(&pts, 2.0);
        line.draw(cx.canvas, t.accent2);
        for (k, &(at, bpm, _)) in self.tempo.iter().enumerate() {
            let x = self.x_of(g.lanes, at);
            if x < lane.x || x > lane.right() {
                continue;
            }
            let y = self.tempo_y(lane, bpm);
            let mut dot = mpfree_ui::Shape::new(lane.inset_xy(-6, 0));
            dot.disc(x as f32, y, 4.0);
            dot.draw(cx.canvas, t.text);
            if self.tempo_pick == Some(k) {
                let mut ring = mpfree_ui::Shape::new(lane.inset_xy(-10, -2));
                ring.arc(x as f32, y, 7.0, 0.0, std::f32::consts::TAU, 2.0);
                ring.draw(cx.canvas, t.accent);
            }
            let label = format!("{}", (bpm * 10.0).round() / 10.0);
            let w = cx.measure(FontId::Bold, t.size_small - 4.0, &label) + 4;
            let above = y > (lane.y + lane.h / 2) as f32;
            let ly = if above { lane.y } else { lane.bottom() - 13 };
            cx.text(
                FontId::Bold,
                t.size_small - 4.0,
                Rect::new(x + 7, ly, w, 13),
                Align::Left,
                t.text_dim,
                &label,
            );
        }
    }

    /// "4 bars", "1 bar", "2.5 bars": song ticks in bars.
    fn bars_text(&self, ticks: Tick) -> String {
        let bar = self.bar.max(1);
        if ticks == bar {
            "1 bar".into()
        } else if ticks % bar == 0 {
            format!("{} bars", ticks / bar)
        } else {
            format!("{:.1} bars", ticks as f64 / bar as f64)
        }
    }

    /// A tag over `at`: where a dragged thing starts, or how long it is.
    fn paint_tag(&self, cx: &mut PaintCx, x: i32, y: i32, text: &str) {
        let t = cx.theme.clone();
        let w = cx.measure(FontId::Bold, t.size_small - 2.0, text) + 14;
        let tag = Rect::new(x, y, w, 20);
        cx.canvas.fill_round_rect(tag, 10, t.accent);
        cx.text(
            FontId::Bold,
            t.size_small - 2.0,
            tag,
            Align::Center,
            t.ink_on(t.accent),
            text,
        );
    }

    /// What a finger drags: its old place, where it lands and the thing
    /// itself under the finger, over everything else.
    fn paint_drag(&self, cx: &mut PaintCx, r: Rect, gr: &Grip) {
        let t = cx.theme.clone();
        let g = self.geo(r);
        let (dx, dy) = (gr.at.0 - gr.from.0, gr.at.1 - gr.from.1);
        let bar = self.bar.max(1);
        match gr.held {
            Held::Block { index, .. } => {
                let Some(b) = self.block(index) else {
                    return;
                };
                let home = self.block_rect(r, b);
                if let Some(h) = home {
                    cx.canvas.fill_round_rect(h, t.round(6), t.surface);
                    cx.canvas.stroke_round_rect(h, t.round(6), 2, t.outline);
                }
                let land = self.block_landing(r, gr);
                // One of several picked (6.5): the others go along, each
                // its old place left, an outline where it lands and its
                // card moved with the finger.
                if self.picks.contains(&index) {
                    let shift = land.map(|(k, s)| (k as i64 - b.track as i64, s - b.start));
                    let set = |o: &SongBlock| self.picks.contains(&o.index);
                    for &j in self.picks.iter().filter(|&&j| j != index) {
                        let Some(o) = self.block(j) else {
                            continue;
                        };
                        let Some(h) = self.block_rect(r, o) else {
                            continue;
                        };
                        cx.canvas.fill_round_rect(h, t.round(6), t.surface);
                        cx.canvas.stroke_round_rect(h, t.round(6), 2, t.outline);
                        if let Some((dk, ds)) = shift {
                            let track = usize::try_from(o.track as i64 + dk).ok();
                            let lane = track
                                .filter(|&k| k < self.tracks.len())
                                .map(|k| self.track_k(k));
                            let (s, e) = (o.start + ds, o.start + ds + o.length);
                            if let Some(l) = lane.and_then(|k| self.span_rect(&g, k, s, e)) {
                                let clash = track
                                    .is_some_and(|k| self.blocks.between(k, s, e).any(|q| !set(q)))
                                    || s < 0;
                                let ink = if clash { t.bad } else { t.accent };
                                cx.canvas.stroke_round_rect(l.inset(-2), t.round(8), 3, ink);
                            }
                        }
                        let color = self.tracks.get(o.track).map_or(t.accent, |c| c.1);
                        let card = Rect::new(h.x + dx, h.y + dy, h.w, h.h);
                        cx.canvas
                            .fill_round_rect(card, t.round(6), t.raised.mix(color, 0.5));
                        cx.canvas.stroke_round_rect(card, t.round(6), 2, t.accent);
                        if card.w > 30 {
                            cx.text(
                                FontId::Sans,
                                t.size_small - 1.0,
                                card.inset_xy(6, 0),
                                Align::Left,
                                t.text,
                                &o.name,
                            );
                        }
                    }
                }
                let at = land.and_then(|(k, s)| Some((self.span_rect(&g, k, s, s + b.length)?, s)));
                if let (Some((l, _)), Some((k, start))) = (at, land) {
                    // Red where it would run into another block (the app
                    // refuses it and says so).
                    // and on an audio track's lane, into a region.
                    let end = start + b.length;
                    let clash = self
                        .blocks
                        .between(k, start, end)
                        .any(|o| o.index != index && !self.picks.contains(&o.index))
                        || self.region_between(k, start, end);
                    let ink = if clash { t.bad } else { t.accent };
                    cx.canvas.stroke_round_rect(l.inset(-2), t.round(8), 3, ink);
                }
                let w =
                    (self.x_of(g.lanes, b.start + b.length) - self.x_of(g.lanes, b.start)).max(24);
                let card = match home {
                    Some(h) => Rect::new(h.x + dx, h.y + dy, h.w, h.h),
                    None => Rect::new(gr.at.0 - 12, gr.at.1 - g.lane_h / 2, w, g.lane_h - 4),
                };
                let color = self.tracks.get(b.track).map_or(t.accent, |c| c.1);
                cx.canvas
                    .fill_round_rect(card, t.round(6), t.raised.mix(color, 0.5));
                cx.canvas.stroke_round_rect(card, t.round(6), 2, t.accent);
                if card.w > 30 {
                    cx.text(
                        FontId::Sans,
                        t.size_small - 1.0,
                        card.inset_xy(6, 0),
                        Align::Left,
                        t.text,
                        &b.name,
                    );
                }
                // The tag on where it lands, says the bar it starts on.
                if let Some((l, start)) = at {
                    let text = self.at_text(start);
                    self.paint_tag(cx, l.x, l.y - 24, &text);
                }
            }
            Held::End { index, .. } => {
                let (Some(b), Some(length)) = (self.block(index), self.end_landing(r, gr)) else {
                    return;
                };
                if let Some(l) = self.span_rect(&g, b.track, b.start, b.start + length) {
                    let color = self.tracks.get(b.track).map_or(t.accent, |c| c.1);
                    cx.canvas
                        .fill_round_rect(l, t.round(6), t.raised.mix(color, 0.3));
                    // Red where it would run over a region (the app
                    // refuses it and says so).
                    let end = b.start + length;
                    let ink = if self.region_between(b.track, b.start + b.length.min(length), end) {
                        t.bad
                    } else {
                        t.accent
                    };
                    cx.canvas.stroke_round_rect(l.inset(-2), t.round(8), 3, ink);
                    if l.w > 30 {
                        cx.text(
                            FontId::Sans,
                            t.size_small - 1.0,
                            l.inset_xy(6, 0),
                            Align::Left,
                            t.text,
                            &b.name,
                        );
                    }
                    let text = self.bars_text(length);
                    self.paint_tag(cx, (l.right() - 60).max(l.x), l.y - 24, &text);
                }
            }
            Held::Start { index, .. } => {
                let (Some(b), Some(start)) = (self.block(index), self.start_landing(r, gr)) else {
                    return;
                };
                if let Some(l) = self.span_rect(&g, b.track, start, b.start + b.length) {
                    let color = self.tracks.get(b.track).map_or(t.accent, |c| c.1);
                    cx.canvas
                        .fill_round_rect(l, t.round(6), t.raised.mix(color, 0.3));
                    cx.canvas
                        .stroke_round_rect(l.inset(-2), t.round(8), 3, t.accent);
                    if l.w > 30 {
                        cx.text(
                            FontId::Sans,
                            t.size_small - 1.0,
                            l.inset_xy(16, 0),
                            Align::Left,
                            t.text,
                            &b.name,
                        );
                    }
                    let text = format!("from bar {}", start / bar + 1);
                    self.paint_tag(cx, l.x, l.y - 24, &text);
                }
            }
            Held::Slide { index } => {
                let Some(b) = self.block(index) else {
                    return;
                };
                if let Some(l) = self.block_rect(r, b) {
                    // The block stays; what it plays moves inside it.
                    cx.canvas
                        .stroke_round_rect(l.inset(-2), t.round(8), 3, t.warn);
                    let by = self.slide_by(r, gr);
                    let text = format!("slide {}", self.slide_text(by));
                    self.paint_tag(cx, l.x, l.y - 24, &text);
                }
            }
            Held::Tempo { k: None, .. } | Held::Ruler => {}
            Held::Box => {
                // The box, and the pick's outline on every block it
                // touches.
                let b = self.box_rect(r, gr);
                for (_, br) in self.blocks_in_view(&g) {
                    if br.intersect(b).w > 0 && br.intersect(b).h > 0 {
                        cx.canvas.stroke_round_rect(br, t.round(6), 2, t.accent);
                    }
                }
                cx.canvas.fill_rect(b, t.accent.with_alpha(30));
                cx.canvas.stroke_round_rect(b, 2, 1, t.accent);
            }
            Held::MarkerGrip { k, .. } => {
                // A line where the marker lands, and a tag saying where
                // its section will start; the blocks stay as they are.
                let at = self.ruler_landing(r, gr);
                let x = self.x_of(g.lanes, at);
                if x >= g.lanes.x + NAME_W && x <= g.lanes.right() {
                    let h = g.lanes.bottom() - g.ruler.y;
                    cx.canvas
                        .fill_round_rect(Rect::new(x - 1, g.ruler.y, 2, h), 1, t.accent);
                    let name = self.markers.get(k).map_or("", |m| m.1.as_str());
                    let text = format!("{name} from bar {}", at / bar + 1);
                    self.paint_tag(cx, x + 6, g.ruler.y + RULER_H + 4, &text);
                }
            }
            Held::Tempo { k: Some(k), .. } => {
                // A line where it lands, and its tempo and bar on a tag.
                let at = self.tempo_landing(r, gr);
                let lane = self.tempo_rect(r);
                let x = self.x_of(g.lanes, at);
                if x >= lane.x && x <= lane.right() {
                    let h = g.lanes.bottom() - g.ruler.y;
                    cx.canvas
                        .fill_round_rect(Rect::new(x - 1, g.ruler.y, 2, h), 1, t.accent);
                }
                let bpm = self.tempo.get(k).map_or(0.0, |p| p.1);
                let text = format!("{} BPM · bar {}", (bpm * 10.0).round() / 10.0, at / bar + 1);
                self.paint_tag(cx, x + 6, lane.bottom() + 2, &text);
            }
            Held::Section { k, .. } => {
                let (Some(a), Some(b)) = (self.edge_tick(k), self.edge_tick(k + 1)) else {
                    return;
                };
                let top = g.ruler.y + 14;
                let x0 = self.x_of(g.lanes, a).max(g.lanes.x + NAME_W);
                let x1 = self.x_of(g.lanes, b).min(g.lanes.right());
                if x1 > x0 {
                    let home = Rect::new(x0, top, x1 - x0, g.lanes.bottom() - top);
                    cx.canvas.stroke_round_rect(home, t.round(6), 2, t.outline);
                }
                let gap = self.section_gap(r, gr).and_then(|j| self.edge_tick(j));
                if let Some(e) = gap {
                    let x = self.x_of(g.lanes, e);
                    if x >= g.lanes.x + NAME_W && x <= g.lanes.right() {
                        let h = g.lanes.bottom() - g.ruler.y;
                        cx.canvas
                            .fill_round_rect(Rect::new(x - 2, g.ruler.y, 4, h), 2, t.accent);
                        for y in [g.ruler.y - 4, g.lanes.bottom() - 8] {
                            cx.canvas
                                .fill_round_rect(Rect::new(x - 6, y, 12, 12), 6, t.accent);
                        }
                    }
                }
                // The section's name under the finger, kept clear of the
                // line.
                let name = &self.markers[k].1;
                let text = format!("{name} · {}", self.bars_text(b - a));
                let w = cx.measure(FontId::Bold, t.size_small - 1.0, &text) + 20;
                let mut x = gr.at.0 - 16;
                if let Some(e) = gap {
                    let line = self.x_of(g.lanes, e);
                    if (x - 8..x + w + 8).contains(&line) {
                        x = if gr.at.0 >= line {
                            line + 12
                        } else {
                            line - w - 12
                        };
                    }
                }
                let y = (g.ruler.y + dy).clamp(g.ruler.y, g.lanes.bottom() - 28);
                let chip = Rect::new(x, y, w, 28);
                cx.canvas.fill_round_rect(chip, t.round(8), t.raised);
                cx.canvas.stroke_round_rect(chip, t.round(8), 2, t.accent);
                cx.text(
                    FontId::Bold,
                    t.size_small - 1.0,
                    chip,
                    Align::Center,
                    t.text,
                    &text,
                );
            }
        }
    }

    /// The take lane whose body (past the names) is under (x, y): its
    /// take.
    fn take_hit(&self, g: &Geo, x: i32, y: i32) -> Option<u16> {
        let (a, n) = self.open_takes()?;
        if x < g.lanes.x + NAME_W {
            return None;
        }
        let k0 = self.audio_k(a) + 1;
        (0..n)
            .find(|&k| self.lane_rect(g, k0 + k).is_some_and(|r| r.contains(x, y)))
            .map(|k| self.audio[a].takes[k].take)
    }

    /// Block `b` when it plays an audio clip on an audio track's lane:
    /// that lane and what its wave draws ([`ClipBlock`]).
    fn clip_block(&self, b: &SongBlock) -> Option<(&AudioLane, &ClipBlock)> {
        let lane = &self.audio[self.audio_at(b.track)?];
        let k = lane
            .clips
            .binary_search_by_key(&b.index, |c| c.index)
            .ok()?;
        Some((lane, &lane.clips[k]))
    }

    /// Whether a region on the lane of the track in place `row` plays some
    /// of `start..end` (a block can't go there).
    fn region_between(&self, row: usize, start: Tick, end: Tick) -> bool {
        self.audio_at(row).is_some_and(|a| {
            self.audio[a]
                .blocks
                .iter()
                .any(|r| r.start < end && r.end > start)
        })
    }

    /// A block of an audio clip (phase 2, step 2d, layout spec 4c), in
    /// `br` (the canvas clipped to it): the lane's dark fill edged in its
    /// wave's ink, a header strip in the track's colour with the clip's
    /// name (and "once" for a once clip), and under it the clip's wave
    /// pass after pass, with a faint line where each pass after the first
    /// starts. Lanes too thin for the strip (under 24 pixels) show the
    /// name over the wave.
    fn paint_clip_block(
        &self,
        cx: &mut PaintCx,
        g: &Geo,
        lane: &AudioLane,
        c: &ClipBlock,
        b: &SongBlock,
        br: Rect,
    ) {
        let t = cx.theme.clone();
        let color = lane.color;
        let ink = color.mix(t.text, 0.6);
        let body = t.raised.mix(color, 0.2);
        // The block's whole stretch, unclipped, so its corners, its strip
        // and its passes line up when it runs off an edge.
        let (start, end) = (b.start, b.start + b.length);
        let x0 = self.x_of(g.lanes, start);
        let x1 = self.x_of(g.lanes, end).max(x0 + 1);
        let full = Rect::new(x0, br.y, x1 - x0, br.h);
        let strip_h = match br.h {
            h if h >= 40 => 15,
            h if h >= 24 => 13,
            _ => 0,
        };
        let old = cx.canvas.clip();
        cx.canvas
            .set_clip(old.intersect(Rect::new(full.x, full.y, full.w, strip_h)));
        cx.canvas.fill_round_rect(full, t.round(6), color);
        cx.canvas.set_clip(old.intersect(Rect::new(
            full.x,
            full.y + strip_h,
            full.w,
            full.h - strip_h,
        )));
        cx.canvas.fill_round_rect(full, t.round(6), body);
        cx.canvas.set_clip(old);
        let wave = Rect::new(br.x, br.y + strip_h + 1, br.w, br.h - strip_h - 2);
        // The passes in view; none where a pass is too thin to see.
        let pass = c.pass.max(1);
        let (from, to) = self.ticks_shown();
        let (lo, hi) = (start.max(from), end.min(to));
        let pass_px = self.x_of(g.lanes, c.origin + pass) - self.x_of(g.lanes, c.origin);
        if pass_px >= 3 {
            let seam = t.text.with_alpha(0x38);
            let mut o = c.origin + (lo - c.origin).div_euclid(pass) * pass;
            while o < hi {
                for p in &c.pieces {
                    let (s, e) = (o + p.start, o + p.end);
                    if e <= lo || s >= hi {
                        continue;
                    }
                    let px0 = self.x_of(g.lanes, s);
                    let px1 = self.x_of(g.lanes, e).max(px0 + 1);
                    let per_px = p.frames / f64::from(px1 - px0);
                    let l = px0.max(br.x);
                    let cols = Rect::new(l, wave.y, (px1.min(br.right()) - l).max(0), wave.h);
                    self.paint_wave(cx, p.take, wave, cols, px0, p.src as f64, per_px, ink, true);
                }
                if o > start {
                    let x = self.x_of(g.lanes, o);
                    cx.canvas
                        .fill_rect(Rect::new(x, br.y + strip_h, 1, br.h - strip_h), seam);
                }
                o += pass;
            }
        }
        cx.canvas.stroke_round_rect(full, t.round(6), 1, ink);
        if br.w > 30 {
            let name = if c.once {
                format!("{} · once", b.name)
            } else {
                b.name.clone()
            };
            let room = br.w - 10 - if b.unique { 10 } else { 0 };
            let (at, text) = if strip_h > 0 {
                (Rect::new(br.x + 5, br.y, room, strip_h), t.ink_on(color))
            } else {
                (Rect::new(br.x + 5, br.y, room, br.h), t.text)
            };
            cx.text(
                FontId::Bold,
                t.size_small - 3.0,
                at,
                Align::Left,
                text,
                &name,
            );
        }
    }

    /// An audio region: its block, waveform, fades and tag.
    fn paint_region(
        &self,
        cx: &mut PaintCx,
        g: &Geo,
        lane: &AudioLane,
        b: &AudioBlock,
        br: Rect,
        picked: bool,
    ) {
        let t = cx.theme.clone();
        cx.canvas
            .fill_round_rect(br, t.round(6), t.raised.mix(lane.color, 0.42));
        // The block's full stretch on screen, unclipped, so the waveform
        // and the fades line up when it runs off an edge.
        let x0 = self.x_of(g.lanes, b.start);
        let x1 = self.x_of(g.lanes, b.end).max(x0 + 1);
        let per_px = b.frames / f64::from(x1 - x0);
        // The waveform fills the block; the name sits over its top.
        let label_h = if br.h >= 26 { 14 } else { 0 };
        let wave = br.inset_xy(0, 2);
        let ink = lane.color.mix(t.text, 0.6);
        self.paint_wave(cx, b.take, wave, br, x0, b.src as f64, per_px, ink, false);
        // Fades: the part above each slope is dimmed. A cut shows its share
        // of the fades of the region it was cut from.
        let shade = t.surface.mix(lane.color, 0.15);
        let px_of = |frames: u32| (f64::from(frames) / per_px.max(1e-9)).round() as i32;
        for (frames, cut, rising) in [(b.fade_in, b.into, true), (b.fade_out, b.left, false)] {
            let (px, off) = (px_of(frames), px_of(cut));
            if px < 2 || off >= px {
                continue;
            }
            for k in off..px {
                let x = if rising {
                    x0 - off + k
                } else {
                    x1 - 1 + off - k
                };
                if x < br.x || x >= br.right() {
                    continue;
                }
                let up = (wave.h * k / px).clamp(0, wave.h);
                cx.canvas
                    .fill_rect(Rect::new(x, wave.y, 1, wave.h - up), shade);
                cx.canvas
                    .fill_rect(Rect::new(x, wave.y + wave.h - up - 1, 1, 2), t.text_dim);
            }
        }
        if label_h > 0 && br.w > 30 {
            let name = if b.tag.is_empty() {
                lane.name.clone()
            } else {
                format!("{} · {}", lane.name, b.tag)
            };
            cx.text(
                FontId::Sans,
                t.size_small - 3.0,
                Rect::new(br.x + 6, br.y + 1, br.w - 8, label_h),
                Align::Left,
                t.text,
                &name,
            );
        }
        if picked {
            cx.canvas.stroke_round_rect(br, t.round(6), 2, t.accent);
        }
    }

    /// An audio lane's header: arm dot, name, input chip, level bar.
    fn paint_header(&self, cx: &mut PaintCx, h: Rect, lane: &AudioLane) {
        let t = cx.theme.clone();
        let dot = Rect::new(h.x + 2, h.y + h.h / 2 - 6, 12, 12);
        if lane.armed {
            cx.canvas.fill_round_rect(dot, 6, t.bad);
        } else {
            cx.canvas.stroke_round_rect(dot, 6, 2, t.text_dim);
        }
        let word = if lane.mix.is_some() {
            "Mix"
        } else {
            input_chip(lane.input)
        };
        let chip_w = cx.measure(FontId::Bold, t.size_small - 3.0, word) + 10;
        let chip = Rect::new(h.right() - chip_w, h.y + h.h / 2 - 9, chip_w, 18);
        cx.canvas.fill_round_rect(chip, 5, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small - 3.0,
            chip,
            Align::Center,
            t.text_dim,
            word,
        );
        if self.panel == Some(lane.track) {
            // Its panel is open.
            cx.canvas
                .stroke_round_rect(h.inset_xy(-2, 1), t.round(6), 1, t.accent);
        }
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            Rect::new(h.x + 20, h.y, (chip.x - h.x - 24).max(0), h.h),
            Align::Left,
            t.readable(lane.color.mix(t.text, 0.25), t.surface),
            &lane.name,
        );
        let bar = Rect::new(h.x + 2, h.bottom() - 6, h.w - 4, 3);
        cx.canvas.fill_rect(bar, t.raised);
        let lit = (bar.w as f32 * lane.level.clamp(0.0, 1.0)).round() as i32;
        if lit > 0 {
            let c = if lane.level >= 0.95 { t.bad } else { t.good };
            cx.canvas.fill_rect(Rect::new(bar.x, bar.y, lit, 3), c);
        }
    }
}

impl Widget for SongLanes {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let clip = cx.canvas.clip();
        self.keep_painted(r, clip);
        let g = self.geo(r);
        // The title line, above the ruler, only where the clip reaches it
        // (a frame that moves the playhead draws strips below it); the
        // panel under everything either way.
        if !clip
            .intersect(Rect::new(r.x, r.y, r.w, g.ruler.y - r.y))
            .is_empty()
        {
            // The view chip and the title, the legend, then Rec song.
            let rec = self.rec_rect(r);
            let show = self.show_rect(r);
            let end = show.map_or(rec.x, |s| s.x);
            view_title(cx, r, ArrangeView::Song, &self.title, end - 12);
            // The zoom chip (6.4): "Bars", then 4 to 64, the one shown lit.
            let zoom = self.zoom_rect(r);
            cx.canvas.fill_round_rect(zoom, t.round(10), t.raised);
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(zoom.x + 10, zoom.y, ZOOM_LABEL_W - 10, zoom.h),
                Align::Left,
                t.text_dim,
                "Bars",
            );
            for (k, n) in ZOOMS.iter().enumerate() {
                let b = self.zoom_part_rect(r, k);
                let ink = if *n == self.bars {
                    cx.canvas
                        .fill_round_rect(b.inset_xy(2, 2), t.round(8), t.accent);
                    t.ink_on(t.accent)
                } else {
                    t.text
                };
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    b,
                    Align::Center,
                    ink,
                    &n.to_string(),
                );
            }
            let head = r.inset(14).take_top(26).0;
            let legend_x = zoom.right() + 12;
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(legend_x, head.y, (end - legend_x - 12).max(0), head.h),
                Align::Right,
                if self.alert { t.warn } else { t.text_dim },
                &self.legend,
            );
            if let Some(b) = show {
                cx.canvas.fill_round_rect(b, t.round(10), t.raised);
                cx.text(FontId::Bold, t.size_small, b, Align::Center, t.text, "Show");
            }
            rec_song_pill(cx, rec, self.rec);
        } else {
            // The view's panel, as `view_title` starts it.
            cx.panel(r);
        }
        // The ruler: bar numbers, then the markers' names over them.
        // Every bar's number up to 16 bars, every 4th at 32, every 8th
        // at 64 (6.4); at 4 and 8 bars a faint line on each beat too.
        let step = match self.bars {
            ..=16 => 1,
            17..=32 => 4,
            _ => 8,
        };
        if self.bars <= 8 && self.beat > 0 && self.beat < self.bar {
            let (from, to) = self.ticks_shown();
            let mut at = from;
            while at < to {
                if at % self.bar.max(1) != 0 {
                    let x = self.x_of(g.ruler, at);
                    cx.canvas.fill_rect(
                        Rect::new(x, g.lanes.y, 1, g.lanes.h),
                        t.outline.mix(t.surface, 0.6),
                    );
                }
                at += self.beat;
            }
        }
        for k in 0..=self.bars {
            let bar = self.first_bar + k;
            if bar % step != 0 {
                continue;
            }
            let x = self.x_of(g.ruler, bar * self.bar);
            cx.canvas.fill_rect(
                Rect::new(x, g.ruler.y + 16, 1, g.lanes.bottom() - g.ruler.y - 16),
                t.outline,
            );
            if k < self.bars {
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    Rect::new(x + 3, g.ruler.y, 40, 16),
                    Align::Left,
                    t.text_faint,
                    &format!("{}", bar + 1),
                );
            }
        }
        self.paint_passes(cx, &g);
        for (k, (at, name)) in self.markers.iter().enumerate() {
            let x = self.x_of(g.ruler, *at);
            if x < g.ruler.x + NAME_W || x >= g.ruler.right() {
                continue;
            }
            cx.canvas.fill_rect(
                Rect::new(x, g.ruler.y + 14, 2, g.lanes.bottom() - g.ruler.y - 14),
                t.warn,
            );
            // A part of N passes: "Verse ×2" (4b.4).
            let name = &match self.passes.get(k).copied().flatten() {
                Some((n, _)) => format!("{name} ×{n}"),
                None => name.clone(),
            };
            let mut ink = t.warn;
            if self.section == Some(k) {
                // Picked, for the wheel: the name on a pill, and its grip
                // (two short bars) left of the marker, which moves the
                // marker alone (Arrange 3.3).
                let w = cx.measure(FontId::Bold, t.size_small - 2.0, name) + 10;
                cx.canvas
                    .fill_round_rect(Rect::new(x, g.ruler.y + 11, w, 16), 8, t.warn);
                ink = t.ink_on(t.warn);
                if let Some(gp) = self.grip_rect(r) {
                    let pill = Rect::new(gp.right() - 15, gp.y + 2, 13, gp.h - 4);
                    cx.canvas.fill_round_rect(pill, 4, t.raised);
                    for k in 0..2 {
                        let bx = pill.x + 4 + k * 4;
                        cx.canvas.fill_round_rect(
                            Rect::new(bx, pill.y + 5, 2, pill.h - 10),
                            1,
                            t.warn,
                        );
                    }
                }
            }
            // Cut short before the next marker, so names never run into
            // each other zoomed out (6.4).
            let room = self
                .markers
                .get(k + 1)
                .map_or(120, |m| (self.x_of(g.ruler, m.0) - x - 6).min(120))
                .max(0);
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                Rect::new(x + 4, g.ruler.y + 12, room, 14),
                Align::Left,
                ink,
                name,
            );
        }
        self.paint_tempo(cx, &g);
        // A scroll bar at the left edge when the lanes scroll.
        let n = self.lane_count();
        let shown = self.shown(&g);
        if n > shown {
            let first = self.first(&g);
            let track_h = g.lanes.h;
            let h = (track_h * shown as i32 / n as i32).max(16);
            let y = g.lanes.y + (track_h - h) * first as i32 / self.max_first(&g).max(1) as i32;
            cx.canvas
                .fill_round_rect(Rect::new(g.lanes.x - 9, y, 3, h), 1, t.text_faint);
        }
        for k in 0..n {
            let Some(row) = self.lane_rect(&g, k) else {
                continue;
            };
            match self.lane(k) {
                Some(Lane::Midi(i)) => {
                    let (name, color) = &self.tracks[i];
                    // Under the name when the lane has room for two lines
                    // (not with take lanes open, say).
                    let sound = self
                        .sounds
                        .get(i)
                        .filter(|s| !s.is_empty() && row.h >= SOUND_LANE_H);
                    let at = match sound {
                        Some(_) => Rect::new(row.x, row.y + row.h / 2 - 18, NAME_W - 8, 18),
                        None => Rect::new(row.x, row.y, NAME_W - 8, row.h),
                    };
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        at,
                        Align::Left,
                        t.readable(color.mix(t.text, 0.25), t.surface),
                        name,
                    );
                    if let Some(s) = sound {
                        cx.text(
                            FontId::Sans,
                            t.size_small - 2.0,
                            Rect::new(row.x, row.y + row.h / 2, NAME_W - 8, 16),
                            Align::Left,
                            t.text_dim,
                            s,
                        );
                    }
                }
                Some(Lane::Audio(a)) => {
                    let h = Rect::new(row.x, row.y, NAME_W - 8, row.h);
                    self.paint_header(cx, h, &self.audio[a]);
                }
                Some(Lane::Take(a, j)) => {
                    let lane = &self.audio[a];
                    let take = &lane.takes[j];
                    let used = !take.used.is_empty();
                    cx.text(
                        FontId::Sans,
                        t.size_small - 3.0,
                        Rect::new(row.x + 20, row.y, NAME_W - 28, row.h),
                        Align::Left,
                        if used {
                            t.readable(lane.color.mix(t.text, 0.4), t.surface)
                        } else {
                            t.text_faint
                        },
                        &if take.recording {
                            format!("{} · rec", take.name)
                        } else {
                            take.name.clone()
                        },
                    );
                    cx.canvas.fill_rect(
                        Rect::new(row.x + 8, row.y + 4, 2, row.h - 8),
                        lane.color.mix(t.surface, 0.5),
                    );
                    self.paint_take(cx, &g, a, j, row);
                }
                Some(Lane::AddTrack) => {
                    if let Some(b) = self.add_track_rect(r) {
                        cx.canvas.stroke_round_rect(b, t.round(8), 1, t.outline);
                        cx.text(
                            FontId::Bold,
                            t.size_small - 1.0,
                            b,
                            Align::Center,
                            t.text_dim,
                            "+ Track",
                        );
                    }
                }
                None => {}
            }
        }
        for (b, br) in self.blocks_in_view(&g) {
            // Wholly outside the clip (a glowing theme's halo included):
            // nothing of it to draw.
            if br.inset(-4).intersect(clip).is_empty() {
                continue;
            }
            let color = self.tracks.get(b.track).map_or(t.accent, |c| c.1);
            let audio = self.clip_block(b);
            match audio {
                Some((lane, c)) => {
                    let old = cx.canvas.clip();
                    cx.canvas.set_clip(old.intersect(br));
                    self.paint_clip_block(cx, &g, lane, c, b, br);
                    cx.canvas.set_clip(old);
                }
                None => cx
                    .canvas
                    .fill_round_rect(br, t.round(6), t.raised.mix(color, 0.42)),
            }
            if b.unique {
                // Its own copy: a corner mark (on an audio clip's header
                // strip, in its ink).
                let ink = if audio.is_some() {
                    t.ink_on(color)
                } else {
                    t.text
                };
                cx.canvas
                    .fill_round_rect(Rect::new(br.right() - 10, br.y + 4, 6, 6), 3, ink);
            }
            if self.picks.contains(&b.index) {
                cx.canvas.stroke_round_rect(br, t.round(6), 2, t.accent);
            }
            if self.selected == Some(b.index) {
                cx.canvas.stroke_round_rect(br, t.round(6), 2, t.accent);
                // Its end grip: two bars inside its right end.
                if self.end_rect(r).is_some() && br.w > 24 {
                    for k in 0..2 {
                        let x = br.right() - 10 + k * 4;
                        cx.canvas
                            .fill_round_rect(Rect::new(x, br.y + 6, 2, br.h - 12), 1, t.text);
                    }
                }
                // Its start grip (phase 6): two bars inside its left end.
                if self.start_rect(r).is_some() && br.w > 48 {
                    for k in 0..2 {
                        let x = br.x + 6 + k * 4;
                        cx.canvas
                            .fill_round_rect(Rect::new(x, br.y + 6, 2, br.h - 12), 1, t.text);
                    }
                }
            }
            if audio.is_none() && br.w > 30 {
                // Picked, the name makes room for the start grip.
                let left = if self.selected == Some(b.index) && br.w > 48 {
                    16
                } else {
                    6
                };
                let name = Rect::new(br.x + left, br.y, (br.w - left - 6).max(0), br.h);
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    name,
                    Align::Left,
                    t.text,
                    &b.name,
                );
            }
        }
        self.paint_over(cx, &g);
        for (a, lane) in self.audio.iter().enumerate() {
            for (i, b) in lane.blocks.iter().enumerate() {
                let Some(br) = self.region_rect(r, a, i) else {
                    continue;
                };
                let picked = self.region == Some((lane.track, i));
                let old = cx.canvas.clip();
                cx.canvas.set_clip(old.intersect(br));
                self.paint_region(cx, &g, lane, b, br, picked);
                cx.canvas.set_clip(old);
            }
            // A region being dragged: an outline where it would land.
            if let Some((ma, mi, x0, x1)) = self.moving {
                if ma == a && (x1 - x0).abs() >= DRAG_SLOP {
                    if let Some(br) = self.region_rect(r, a, mi) {
                        let ghost = Rect::new(br.x + x1 - x0, br.y, br.w, br.h);
                        cx.canvas.stroke_round_rect(ghost, t.round(6), 2, t.accent);
                    }
                }
            }
            // Recording: red from the punch in to the playhead.
            if let (Some(from), Some(now)) = (lane.recording, self.playhead) {
                let lane_k = self.audio_k(a);
                if let Some(br) = self.span_rect(&g, lane_k, from, now.max(from + 1)) {
                    cx.canvas
                        .fill_round_rect(br, t.round(6), t.bad.mix(t.surface, 0.35));
                    cx.canvas.stroke_round_rect(br, t.round(6), 1, t.bad);
                    if br.w > 34 && br.h >= 30 {
                        cx.text(
                            FontId::Bold,
                            t.size_small - 3.0,
                            Rect::new(br.x + 6, br.y + 1, br.w - 8, 14),
                            Align::Left,
                            t.text,
                            "rec",
                        );
                    }
                }
            }
        }
        if let Some(x) = self.playhead_x(&g, self.playhead) {
            let c = if self.on { t.good } else { t.text_faint };
            cx.canvas.fill_rect(
                Rect::new(x, g.ruler.y, PLAYHEAD_W, g.lanes.bottom() - g.ruler.y),
                c,
            );
        }
        if let Some(p) = self.panel_parts(r) {
            self.paint_panel(cx, &p);
        }
        let audio = self.region.is_some();
        let selected = self.selected.is_some();
        // Several picked (6.5): Delete and Duplicate take them all, Make
        // unique only one block.
        let several = !self.picks.is_empty();
        let tempo = self
            .tempo_pick
            .and_then(|i| self.tempo.get(i))
            .filter(|_| !audio);
        let section = self.section_buttons();
        for (k, br) in self.button_rects(r).iter().enumerate() {
            if br.w == 0 {
                continue;
            }
            let lit = (k == 1 && self.on && !audio) || (k == 4 && tempo.is_some_and(|p| p.2));
            let live = audio
                || tempo.is_some()
                || section
                || !(k >= 4 && !selected && !(k == 5 && several));
            let fill = if lit {
                t.good
            } else if live {
                t.raised
            } else {
                t.surface
            };
            cx.canvas.fill_round_rect(*br, t.round(10), fill);
            let ink = if lit {
                t.ink_on(fill)
            } else if live {
                t.text
            } else {
                t.text_faint
            };
            let word = if audio {
                AUDIO_BUTTONS[k]
            } else if k == 1 {
                if self.on {
                    "Song on"
                } else {
                    "Song off"
                }
            } else if k >= 4 && tempo.is_some() {
                TEMPO_BUTTONS[k - 4]
            } else if k >= 2 && section {
                SECTION_BUTTONS[k - 2]
            } else {
                SONG_BUTTONS[k]
            };
            cx.text(FontId::Bold, t.size_small, *br, Align::Center, ink, word);
        }
        if let Some(d) = self.duplicate_rect(r) {
            let (fill, ink) = if selected || several {
                (t.raised, t.text)
            } else {
                (t.surface, t.text_faint)
            };
            cx.canvas.fill_round_rect(d, t.round(10), fill);
            cx.text(
                FontId::Bold,
                t.size_small,
                d,
                Align::Center,
                ink,
                "Duplicate",
            );
        }
        if let Some(gr) = self.grip.filter(|g| g.lifted) {
            self.paint_drag(cx, r, &gr);
        }
        if let Some(rows) = self.chooser_rects(r) {
            paint_chooser(cx, &rows);
        }
        if let Some(card) = &self.report {
            card.paint(cx, r);
        }
    }

    /// While the song plays, a frame that only moves the playhead draws
    /// the strip its line left and the one it reached: the rest is as the
    /// last whole paint showed it ([`Painted`]). Anything else changed, a
    /// lane recording (its red runs to the playhead) or no whole paint
    /// yet: all of it.
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let p = self.painted.0.borrow();
        let (at, was, waves) = p.as_deref()?;
        if *at != r || self.audio.iter().any(|l| l.recording.is_some()) {
            return None;
        }
        let mut now = self.clone_without_waves();
        now.playhead = was.playhead;
        if now != *was || wave_keys(&self.waves) != *waves {
            return None;
        }
        let g = self.geo(r);
        let strip = |p: Option<Tick>| {
            let x = self.playhead_x(&g, p)?;
            Some(Rect::new(
                x - 2,
                g.ruler.y,
                PLAYHEAD_W + 4,
                g.lanes.bottom() - g.ruler.y,
            ))
        };
        let (old, new) = (strip(was.playhead), strip(self.playhead));
        Some(if old == new {
            Vec::new()
        } else {
            old.into_iter().chain(new).collect()
        })
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if let Some(card) = &self.report {
            return card.targets(r);
        }
        // The chooser open: its rows, and anywhere else closes it.
        if let Some(rows) = self.chooser_rects(r) {
            let mut out: Vec<Target> = rows
                .iter()
                .enumerate()
                .map(|(k, b)| Target::tap("add-kind", k, ADD_KINDS[k].0, *b))
                .collect();
            out.push(Target::tap("add-close", 0, "Close", r));
            return out;
        }
        let audio = self.region.is_some();
        // The track panel takes the touches over it: a block under it can
        // be tapped only where it shows.
        let panel = self.panel_parts(r);
        let shown = |br: Rect| match panel {
            Some(p) => uncovered(br, p.panel),
            None => Some(br),
        };
        let mut out: Vec<Target> = self
            .button_rects(r)
            .iter()
            .enumerate()
            .filter(|(_, b)| b.w > 0)
            .map(|(k, b)| {
                let tempo = !audio && k >= 4 && self.tempo_pick.is_some();
                let (id, word) = if audio {
                    ("region-button", AUDIO_BUTTONS[k])
                } else if tempo {
                    ("tempo-button", TEMPO_BUTTONS[k - 4])
                } else if k >= 2 && self.section_buttons() {
                    ("section-button", SECTION_BUTTONS[k - 2])
                } else {
                    ("button", SONG_BUTTONS[k])
                };
                // Make unique takes one block, not several (6.5).
                let one = id == "button" && k == 4 && !self.picks.is_empty();
                Target::tap(id, k, word, *b).disabled_if(one)
            })
            .collect();
        if let Some(d) = self.duplicate_rect(r) {
            out.push(
                Target::tap("button", DUPLICATE, "Duplicate", d)
                    .disabled_if(self.selected.is_none() && self.picks.is_empty()),
            );
        }
        // The tempo lane: each point, and the lane after the last (a tap
        // there adds one).
        let lane = self.tempo_rect(r);
        let mut free = lane;
        for k in 0..self.tempo.len() {
            if let Some(p) = self.tempo_point_rect(r, k) {
                out.push(Target::tap(
                    "tempo-point",
                    k,
                    format!("{:.1} BPM", self.tempo[k].1),
                    p,
                ));
                free = Rect::new(p.right(), lane.y, (lane.right() - p.right()).max(0), lane.h);
            }
        }
        if free.w >= 2 * TEMPO_HIT {
            out.push(Target::tap("tempo-lane", 0, "Tempo", free));
        }
        out.push(Target::tap("rec-song", 0, "Rec song", self.rec_rect(r)));
        if let Some(b) = self.show_rect(r) {
            out.push(Target::tap("show-view", 0, "Show", b));
        }
        // The zoom chip (6.4): the bars shown are lit, a tap on them
        // changes nothing.
        for (k, n) in ZOOMS.iter().enumerate() {
            out.push(
                Target::tap("zoom", k, format!("{n} bars"), self.zoom_part_rect(r, k))
                    .disabled_if(*n == self.bars),
            );
        }
        let chip = self.view_rect(r);
        out.push(Target::tap(
            "view",
            0,
            "Clips",
            view_part_rect(chip, ArrangeView::Clips),
        ));
        out.push(Target::tap(
            "view",
            1,
            "List",
            view_part_rect(chip, ArrangeView::List),
        ));
        if let Some(b) = self.add_track_rect(r) {
            out.push(Target::tap("add-track", 0, "+ Track", b));
        }
        for (a, lane) in self.audio.iter().enumerate() {
            if let (Some(d), Some(h)) = (self.arm_rect(r, a), self.header_rect(r, a)) {
                out.push(Target::tap("arm", a, format!("{} arm", lane.name), d));
                let rest = Rect::new(d.right(), h.y, h.right() - d.right(), h.h);
                out.push(Target::tap("audio-header", a, lane.name.clone(), rest));
            }
            for i in 0..lane.blocks.len() {
                if let Some(br) = self.region_rect(r, a, i).and_then(&shown) {
                    out.push(Target::tap(
                        "region",
                        a * 1000 + i,
                        format!("{} region {}", lane.name, i + 1),
                        br,
                    ));
                }
            }
        }
        let g = self.geo(r);
        for (b, br) in self.blocks_in_view(&g) {
            if let Some(br) = shown(br) {
                out.push(Target::tap("block", b.index, b.name.clone(), br));
            }
        }
        if let (Some(e), Some(b)) = (
            self.end_rect(r).and_then(&shown),
            self.selected.and_then(|i| self.block(i)),
        ) {
            out.push(
                Target::drag("block-end", b.index, format!("{} end", b.name), e).towards(-40, 0),
            );
        }
        if let (Some(s), Some(b)) = (
            self.start_rect(r).and_then(&shown),
            self.selected.and_then(|i| self.block(i)),
        ) {
            out.push(
                Target::drag("block-start", b.index, format!("{} start", b.name), s).towards(40, 0),
            );
        }
        let mut free = self.ruler_rect(r);
        for (k, (_, name)) in self.markers.iter().enumerate() {
            if let Some(m) = self.marker_rect(r, k) {
                out.push(Target::tap("section", k, name.clone(), m));
                if m.right() > free.x {
                    free = Rect::new(m.right(), free.y, (free.right() - m.right()).max(0), free.h);
                }
            }
        }
        // The ruler past the last name (a tap there adds a marker), and
        // the picked section's grip (Arrange 3.3).
        if free.w >= 40 {
            out.push(Target::tap("ruler", 0, "Ruler", free));
        }
        if let (Some(k), Some(gp)) = (self.section, self.grip_rect(r)) {
            let name = self.markers.get(k).map_or("", |m| m.1.as_str());
            out.push(
                Target::drag("section-grip", k, format!("{name} starts"), gp).towards(
                    self.x_of(self.geo(r).lanes, self.bar) - self.x_of(self.geo(r).lanes, 0),
                    0,
                ),
            );
        }
        if let (Some(p), Some(lane)) = (
            panel,
            panel.and_then(|p| self.audio.iter().find(|l| l.track == p.track)),
        ) {
            out.push(Target::tap("panel-close", 0, "Close", p.close));
            let arm = if lane.armed { "Armed" } else { "Arm" };
            out.push(Target::tap("panel-arm", 0, arm, p.arm));
            out.push(Target::tap("panel-mute", 0, "Mute", p.mute));
            out.push(Target::tap("panel-solo", 0, "Solo", p.solo));
            out.push(Target::tap("panel-takes", 0, "Takes", p.takes));
            // The input and monitor showing are lit: a tap changes nothing.
            let line = lane.mix.is_none();
            for (k, b) in p.inputs.iter().enumerate() {
                out.push(
                    Target::tap("panel-input", k, INPUTS[k].0, *b)
                        .disabled_if(line && INPUTS[k].1 == lane.input),
                );
            }
            out.push(Target::tap("panel-mix", 0, "Mix", p.mix).disabled_if(!line));
            if let (Some(m), Some(from), Some(fx)) = (&lane.mix, p.from, p.effects) {
                out.push(Target::tap("panel-from-less", 0, "From -", from[0]));
                out.push(Target::tap("panel-from-more", 0, "From +", from[2]));
                out.push(
                    Target::tap("panel-effects", 0, "Before", fx[0]).disabled_if(!m.after_effects),
                );
                out.push(
                    Target::tap("panel-effects", 1, "After", fx[1]).disabled_if(m.after_effects),
                );
            } else {
                for (k, b) in p.monitors.iter().enumerate() {
                    out.push(
                        Target::tap("panel-monitor", k, MONITORS[k].0, *b)
                            .disabled_if(MONITORS[k].1 == lane.monitor),
                    );
                }
            }
            out.push(Target::tap("panel-strip", 0, "Its strip in Mix", p.strip));
            out.push(Target::tap("panel-level-less", 0, "Level -", p.level[0]));
            out.push(Target::tap("panel-level-more", 0, "Level +", p.level[2]));
            out.push(Target::tap("panel-pan-less", 0, "Pan -", p.pan[0]));
            out.push(Target::tap("panel-pan-more", 0, "Pan +", p.pan[2]));
        }
        out
    }

    fn touch(&mut self, f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if let Some(card) = self.report.as_mut() {
            return card.touch(f, phase, x, y, r);
        }
        // The "+ Track" chooser open: a row adds that kind of track, and a
        // touch anywhere else only closes it.
        if self.adding {
            if phase != Touch::Down {
                return None;
            }
            let kind = self
                .chooser_rects(r)
                .and_then(|rows| rows.iter().position(|b| b.contains(x, y)));
            return Some(EventKind::Tab(kind.map_or(ADD_CLOSE, |k| ADD_KINDS[k].1)));
        }
        let g = self.geo(r);
        let names = Rect::new(g.lanes.x - 12, g.lanes.y, NAME_W + 12, g.lanes.h);
        // A block, its end grip or a section's name held: it lifts once
        // dragged, and lands where it is let go.
        if let Some(mut gr) = self.grip {
            match phase {
                Touch::Move => {
                    gr.at = (x, y);
                    let (dx, dy) = (x - gr.from.0, y - gr.from.1);
                    let lifts = !gr.lifted && dx * dx + dy * dy >= LIFT_PX * LIFT_PX;
                    gr.lifted |= lifts;
                    // The picked block held still, then dragged: what it
                    // plays slides under it (phase 6).
                    if let Held::Block { index, .. } = gr.held {
                        if lifts && self.selected == Some(index) && gr.since.elapsed() >= SLIDE_HOLD
                        {
                            gr.held = Held::Slide { index };
                        }
                    }
                    let slides = matches!(gr.held, Held::Slide { .. } | Held::Box);
                    let edge = if gr.lifted && !slides {
                        self.edge_at(r, x)
                    } else {
                        None
                    };
                    let mut scroll = None;
                    gr.edge = match (edge, gr.edge) {
                        (Some(e), Some((was, at))) if e == was && at.elapsed() < SCROLL_REPEAT => {
                            Some((was, at))
                        }
                        (Some(e), _) => {
                            scroll = Some(EventKind::Tab(e));
                            Some((e, Instant::now()))
                        }
                        (None, _) => None,
                    };
                    self.grip = Some(gr);
                    return scroll;
                }
                Touch::Up => {
                    self.grip = None;
                    gr.at = (x, y);
                    return self.let_go(r, &gr);
                }
                Touch::Cancel => {
                    self.grip = None;
                    return None;
                }
                Touch::Down => self.grip = None,
            }
        }
        match phase {
            Touch::Down => {
                // A new finger: whatever an earlier one began is over.
                self.swipe = None;
                self.moving = None;
            }
            Touch::Move if self.swipe.is_some() => {
                if let Some(sw) = self.swipe.as_mut() {
                    sw.2 = x.clamp(g.lanes.x + NAME_W, g.lanes.right());
                }
                return None;
            }
            Touch::Up if self.swipe.is_some() => {
                let (take, x0, _) = self.swipe.take()?;
                let x1 = x.clamp(g.lanes.x + NAME_W, g.lanes.right());
                return Some(if (x1 - x0).abs() < DRAG_SLOP {
                    // A tap: the whole take.
                    EventKind::Span {
                        index: usize::from(take),
                        from: 0,
                        to: 0,
                    }
                } else {
                    let (l, r) = (x0.min(x1), x0.max(x1));
                    EventKind::Span {
                        index: usize::from(take),
                        from: self.tick_at(g.lanes, l),
                        to: self.tick_at(g.lanes, r),
                    }
                });
            }
            Touch::Move if self.moving.is_some() => {
                if let Some(m) = self.moving.as_mut() {
                    m.3 = x;
                }
                return None;
            }
            Touch::Up if self.moving.is_some() => {
                let (a, i, x0, _) = self.moving.take()?;
                if (x - x0).abs() < DRAG_SLOP {
                    return None;
                }
                let lane = self.audio.get(a)?;
                let b = lane.blocks.get(i)?;
                let to = (b.start + self.tick_at(g.lanes, x) - self.tick_at(g.lanes, x0)).max(0);
                return Some(EventKind::Marker {
                    index: lane.track * REGION_SLOTS + i,
                    frame: to as u64,
                });
            }
            Touch::Move => {
                let (y0, first, moved) = self.drag?;
                if !moved && (y - y0).abs() < DRAG_SLOP {
                    return None;
                }
                let rows = (y0 - y) / g.lane_h.max(1);
                let last = self.max_first(&g) as i32;
                self.first_lane = (first as i32 + rows).clamp(0, last) as usize;
                self.drag = Some((y0, first, true));
                return None;
            }
            Touch::Up => {
                let (_, _, moved) = self.drag.take()?;
                if moved {
                    return None;
                }
                // A tap on a header: its name or input chip opens its
                // panel (the arm dot answered on the way down). Anywhere
                // else on the names picks nothing, as empty space does.
                if self.arm_hit(r, x, y).is_some() {
                    return None;
                }
                return Some(
                    (0..self.audio.len())
                        .find(|&a| self.header_rect(r, a).is_some_and(|h| h.contains(x, y)))
                        .map_or(EventKind::Select(NONE), |a| EventKind::Cell {
                            row: self.audio[a].track,
                            col: PANEL,
                        }),
                );
            }
            Touch::Cancel => {
                self.drag = None;
                self.swipe = None;
                self.moving = None;
                return None;
            }
        }
        if let Some(k) = self
            .button_rects(r)
            .iter()
            .position(|b| b.w > 0 && b.contains(x, y))
        {
            return Some(EventKind::Tab(k));
        }
        if self.duplicate_rect(r).is_some_and(|d| d.contains(x, y)) {
            return Some(EventKind::Tab(DUPLICATE));
        }
        if self.rec_rect(r).contains(x, y) {
            return Some(EventKind::Tab(REC));
        }
        if let Some(k) = (0..ZOOMS.len()).find(|&k| self.zoom_part_rect(r, k).contains(x, y)) {
            return Some(EventKind::Tab(ZOOM + k));
        }
        match view_part_at(self.view_rect(r), x, y) {
            Some(ArrangeView::Clips) => return Some(EventKind::Tab(CLIPS)),
            Some(ArrangeView::List) => return Some(EventKind::Tab(LIST)),
            Some(ArrangeView::Song) => return None,
            None => {}
        }
        if self.show_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(SHOW_VIEW));
        }
        if self.add_track_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(ADD_TRACK));
        }
        if let Some(asked) = self.panel_touch(r, x, y) {
            return asked;
        }
        if let Some(take) = self.take_hit(&g, x, y) {
            self.swipe = Some((take, x, x));
            return None;
        }
        if names.contains(x, y) {
            self.first_lane = self.first(&g);
            self.drag = Some((y, self.first_lane, false));
            return self
                .arm_hit(r, x, y)
                .map(|a| EventKind::Toggle(self.audio[a].track));
        }
        // The picked block's end grip, then a section's name, then a
        // block: held, to tap (on the lift) or drag.
        if let (Some(i), Some(e)) = (self.selected, self.end_rect(r)) {
            if let Some(b) = self.block(i).filter(|_| e.contains(x, y)) {
                let dt = self.tick_at(g.lanes, x) - (b.start + b.length);
                self.grip = Some(Grip::new(Held::End { index: i, dt }, x, y));
                return None;
            }
        }
        if let (Some(i), Some(s)) = (self.selected, self.start_rect(r)) {
            if let Some(b) = self.block(i).filter(|_| s.contains(x, y)) {
                let dt = self.tick_at(g.lanes, x) - b.start;
                self.grip = Some(Grip::new(Held::Start { index: i, dt }, x, y));
                return None;
            }
        }
        // The tempo lane: a point (nearest first) or the lane itself, held
        // to tap (on the lift) or drag.
        if self.tempo_rect(r).inset_xy(-TEMPO_HIT, 0).contains(x, y) {
            let k = (0..self.tempo.len())
                .filter(|&k| {
                    self.tempo_point_rect(r, k)
                        .is_some_and(|p| p.contains(x, y))
                })
                .min_by_key(|&k| (self.x_of(g.lanes, self.tempo[k].0) - x).abs());
            if k.is_some() || self.tempo_rect(r).contains(x, y) {
                let dt = k.map_or(0, |k| self.tick_at(g.lanes, x) - self.tempo[k].0);
                self.grip = Some(Grip::new(Held::Tempo { k, dt }, x, y));
                return None;
            }
        }
        // The picked section's grip: its marker alone moves.
        if let (Some(k), Some(gp)) = (self.section, self.grip_rect(r)) {
            if gp.contains(x, y) {
                let dt = self.tick_at(g.lanes, x) - self.markers[k].0;
                self.grip = Some(Grip::new(Held::MarkerGrip { k, dt }, x, y));
                return None;
            }
        }
        if let Some(k) = (0..self.markers.len())
            .find(|&k| self.marker_rect(r, k).is_some_and(|m| m.contains(x, y)))
        {
            let dt = self.tick_at(g.lanes, x) - self.markers[k].0;
            self.grip = Some(Grip::new(Held::Section { k, dt }, x, y));
            return None;
        }
        // The ruler where no name is: a tap adds a marker.
        if self.ruler_rect(r).contains(x, y) {
            self.grip = Some(Grip::new(Held::Ruler, x, y));
            return None;
        }
        let hit = self
            .blocks_in_view(&g)
            .find(|(_, br)| br.contains(x, y))
            .map(|(b, _)| (b.index, b.start));
        if let Some((index, start)) = hit {
            let dt = self.tick_at(g.lanes, x) - start;
            self.grip = Some(Grip::new(Held::Block { index, dt }, x, y));
            return None;
        }
        for (a, lane) in self.audio.iter().enumerate() {
            if let Some(i) = (0..lane.blocks.len()).find(|&i| {
                self.region_rect(r, a, i)
                    .is_some_and(|br| br.contains(x, y))
            }) {
                // Picked as the finger goes down; a drag then moves it.
                self.moving = Some((a, i, x, x));
                return Some(EventKind::Cell {
                    row: lane.track,
                    col: i,
                });
            }
        }
        // Empty space: a tap picks nothing, a drag draws a box (6.5).
        if g.lanes.contains(x, y) {
            self.grip = Some(Grip::new(Held::Box, x, y));
        }
        None
    }
}

/// The biggest part of `r` that `cover` leaves showing, beside it or above
/// or below it; None when too little shows for a fingertip.
fn uncovered(r: Rect, cover: Rect) -> Option<Rect> {
    if r.intersect(cover).w == 0 {
        return Some(r);
    }
    let parts = [
        Rect::new(r.x, r.y, cover.x - r.x, r.h),
        Rect::new(cover.right(), r.y, r.right() - cover.right(), r.h),
        Rect::new(r.x, r.y, r.w, cover.y - r.y),
        Rect::new(r.x, cover.bottom(), r.w, r.bottom() - cover.bottom()),
    ];
    parts
        .into_iter()
        .filter(|p| p.w >= 16 && p.h >= 16)
        .max_by_key(|p| p.w * p.h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lanes() -> SongLanes {
        SongLanes {
            tracks: vec![
                ("Drums".into(), Color::rgb(0xE8A33D)),
                ("Bass".into(), Color::rgb(0x6EA8FE)),
            ],
            blocks: SongBlocks::new(vec![
                SongBlock {
                    index: 0,
                    track: 0,
                    start: 0,
                    length: 4 * 3840,
                    name: "Boom".into(),
                    unique: false,
                },
                SongBlock {
                    index: 1,
                    track: 1,
                    start: 20 * 3840,
                    length: 3840,
                    name: "Late".into(),
                    unique: true,
                },
            ]),
            ..SongLanes::default()
        }
    }

    /// Each control's `Tab` says which control it is: no two share a
    /// value (Duplicate once shared Show's, and the app took its tap for
    /// Show).
    #[test]
    fn the_song_views_tab_ids_are_all_different() {
        let mut ids: Vec<usize> = (0..SONG_BUTTONS.len()).collect();
        ids.extend([
            REC,
            ADD_AUDIO,
            PANEL_CLOSE,
            PANEL_MUTE,
            PANEL_SOLO,
            PANEL_TAKES,
            ADD_TRACK,
            ADD_NOTE,
            ADD_LOOP,
            ADD_CLOSE,
            LIST,
            CLIPS,
            SHOW_VIEW,
            DUPLICATE,
            BARS_BACK,
            BARS_ON,
            TEMPO_ROW,
            RULER_ROW,
            START_ROW,
            SLIDE_ROW,
        ]);
        ids.extend((0..ZOOMS.len()).map(|k| ZOOM + k));
        let mut seen = std::collections::HashSet::new();
        for id in &ids {
            assert!(seen.insert(*id), "{id:#x} is two controls' id");
            assert!(
                !(SECTION_TAP..BARS_BACK).contains(id),
                "{id:#x} is a section's tap"
            );
        }
    }

    #[test]
    fn placements_and_buttons_report_where_they_are() {
        let mut s = lanes();
        let r = Rect::new(12, 140, 814, 534);
        let b = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        // Four bars of sixteen: a quarter of the lane past the names.
        let lane_w = r.w - 28 - NAME_W;
        assert!((b.w - lane_w / 4).abs() <= 3, "{b:?}");
        // Picked as the finger lifts (a drag would move it).
        assert_eq!(s.touch(1, Touch::Down, b.x + 3, b.y + 3, r), None);
        assert_eq!(
            s.touch(1, Touch::Up, b.x + 4, b.y + 3, r),
            Some(EventKind::Select(0))
        );
        assert!(
            s.block_rect(r, &s.blocks[1].clone()).is_none(),
            "off to the right"
        );
        s.first_bar = 16;
        assert!(s.block_rect(r, &s.blocks[1].clone()).is_some());
        assert!(s.block_rect(r, &s.blocks[0].clone()).is_none());
        let empty = s.block_rect(r, &s.blocks[1].clone()).unwrap();
        // Empty space picks nothing as the finger lifts (a drag would
        // draw a box, 6.5).
        assert_eq!(s.touch(1, Touch::Down, empty.x - 20, empty.y + 3, r), None);
        assert_eq!(
            s.touch(1, Touch::Up, empty.x - 20, empty.y + 3, r),
            Some(EventKind::Select(NONE))
        );
        let buttons = s.button_rects(r);
        // The first, Clips, went to the view chip (5.7), and Split took
        // its place (phase 6); Duplicate sits after Song → scenes.
        assert_eq!((buttons[0].x, buttons[0].w), (r.x + 14, SPLIT_W));
        for (k, br) in buttons.iter().enumerate() {
            assert!(br.right() <= r.right() - 14, "{k}: {br:?}");
            assert_eq!(
                s.touch(1, Touch::Down, br.x + 3, br.y + 3, r),
                Some(EventKind::Tab(k))
            );
        }
        let dup = s.duplicate_rect(r).unwrap();
        assert!(
            dup.x > buttons[3].right() && dup.right() < buttons[4].x,
            "{dup:?}"
        );
        assert_eq!(
            s.touch(1, Touch::Down, dup.x + 3, dup.y + 3, r),
            Some(EventKind::Tab(DUPLICATE))
        );
        let rec = s.rec_rect(r);
        assert!(rec.right() <= r.right() - 14 && rec.y >= r.y, "{rec:?}");
        assert_eq!(
            s.touch(1, Touch::Down, rec.x + 3, rec.y + 3, r),
            Some(EventKind::Tab(REC))
        );
        let chip = s.view_rect(r);
        assert!(chip.right() < rec.x, "{chip:?}");
        for (v, want) in [
            (ArrangeView::Clips, Some(EventKind::Tab(CLIPS))),
            (ArrangeView::List, Some(EventKind::Tab(LIST))),
            (ArrangeView::Song, None),
        ] {
            let b = view_part_rect(chip, v);
            assert_eq!(s.touch(1, Touch::Down, b.x + 3, b.y + 3, r), want, "{v:?}");
        }
        // While a set is open, Show sits left of Rec song (live shows B1).
        assert_eq!(s.show_rect(r), None);
        assert!(s.targets(r).iter().all(|t| t.id != "show-view"));
        s.show_button = true;
        let show = s.show_rect(r).unwrap();
        assert!(
            show.right() < rec.x && show.x > chip.right() && show.w == 72,
            "{show:?}"
        );
        assert!(s.targets(r).iter().any(|t| t.id == "show-view"));
        assert_eq!(
            s.touch(1, Touch::Down, show.x + 3, show.y + 3, r),
            Some(EventKind::Tab(SHOW_VIEW))
        );
    }

    #[test]
    fn blocks_drag_along_and_across_lanes_snapped_to_the_bar() {
        let mut s = lanes();
        let r = Rect::new(12, 140, 814, 534);
        let b = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        let bar_px = (r.w - 28 - NAME_W) / 16;
        let (x, y) = (b.x + 10, b.y + b.h / 2);
        // A wobble under LIFT_PX is still a tap.
        s.touch(1, Touch::Down, x, y, r);
        assert_eq!(s.touch(1, Touch::Move, x + 5, y + 4, r), None);
        assert!(s.grip.is_some_and(|g| !g.lifted));
        assert_eq!(
            s.touch(1, Touch::Up, x + 5, y + 4, r),
            Some(EventKind::Select(0))
        );
        // Two bars and a bit on: it lands on bar 3, still the drums'.
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 2 * bar_px + 6, y, r);
        assert!(s.grip.is_some_and(|g| g.lifted));
        assert_eq!(
            s.touch(1, Touch::Up, x + 2 * bar_px + 6, y, r),
            Some(EventKind::Place {
                index: 0,
                row: 0,
                at: 2 * 3840
            })
        );
        // Down a lane: the bass's, from where it was.
        let lane_h = s.geo(r).lane_h;
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x, y + lane_h, r);
        assert_eq!(
            s.touch(1, Touch::Up, x + 2, y + lane_h, r),
            Some(EventKind::Place {
                index: 0,
                row: 1,
                at: 0
            })
        );
        // Lifted and brought back: nothing.
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 3 * bar_px, y, r);
        assert_eq!(s.touch(1, Touch::Up, x + 3, y, r), None);
        // Off the MIDI lanes (onto the ruler): nothing.
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x, y - 3 * lane_h, r);
        assert_eq!(s.touch(1, Touch::Up, x, s.geo(r).ruler.y + 4, r), None);
        // Held at the right edge, the bars scroll on, once a while.
        s.touch(1, Touch::Down, x, y, r);
        let right = r.right() - 20;
        assert_eq!(
            s.touch(1, Touch::Move, right, y, r),
            Some(EventKind::Tab(BARS_ON))
        );
        assert_eq!(s.touch(1, Touch::Move, right - 1, y, r), None);
        s.touch(1, Touch::Cancel, right, y, r);
        assert!(s.grip.is_none());
    }

    /// Several blocks (6.5): a box dragged across empty space reports the
    /// tracks and ticks it covers; a picked one dragged reports its own
    /// landing, the app moving the rest with it; Make unique is faint.
    #[test]
    fn a_box_picks_blocks_and_one_picked_block_carries_the_others() {
        let mut s = lanes();
        let r = Rect::new(12, 140, 814, 534);
        let g = s.geo(r);
        let bar_px = (r.w - 28 - NAME_W) / 16;
        let x0 = g.lanes.x + NAME_W;
        let drums = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        let lane_h = g.lane_h;
        // From bar 6 on the drums' lane, back to bar 3 on the bass's.
        let (fx, fy) = (x0 + 5 * bar_px + 4, drums.y + 4);
        assert_eq!(s.touch(1, Touch::Down, fx, fy, r), None);
        let (tx, ty) = (x0 + 2 * bar_px + 4, drums.y + lane_h + 4);
        assert_eq!(s.touch(1, Touch::Move, tx, ty, r), None, "no edge scroll");
        let Some(EventKind::Span { index, from, to }) = s.touch(1, Touch::Up, tx, ty, r) else {
            panic!("a box");
        };
        assert_eq!(box_tracks(index), (0, 1));
        assert!(index >= BOX);
        assert!((from - (2 * 3840 + 3840 / 20)).abs() < 3840 / 4, "{from}");
        assert!((to - (5 * 3840 + 3840 / 20)).abs() < 3840 / 4, "{to}");
        // Two picked: each has the pick's outline; a drag of one lands
        // as one block's would, and the other's place shows under it.
        s.picks = vec![0, 1];
        s.first_bar = 0;
        let b = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        let (x, y) = (b.x + 10, b.y + b.h / 2);
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 2 * bar_px + 6, y + lane_h, r);
        // Painted mid-drag (the others' cards and landings) without a
        // fault.
        let mut c = mpfree_ui::SoftCanvas::new(r.right() + 20, r.bottom() + 20);
        let theme = mpfree_ui::Theme::dark();
        let mut fonts = mpfree_ui::Fonts::embedded();
        s.paint(
            &mut PaintCx {
                canvas: &mut c,
                theme: &theme,
                fonts: &mut fonts,
            },
            r,
        );
        assert_eq!(
            s.touch(1, Touch::Up, x + 2 * bar_px + 6, y + lane_h, r),
            Some(EventKind::Place {
                index: 0,
                row: 1,
                at: 2 * 3840
            })
        );
        // Make unique takes one block: faint with several picked;
        // Duplicate isn't.
        let targets = s.targets(r);
        let unique = targets.iter().find(|t| t.id == "button" && t.index == 4);
        assert!(unique.is_some_and(|t| !t.enabled), "{unique:?}");
        let dup = targets.iter().find(|t| t.index == DUPLICATE);
        assert!(dup.is_some_and(|t| t.enabled), "{dup:?}");
        // A box under the last lane picks nothing, as a tap there does.
        s.picks.clear();
        s.touch(1, Touch::Down, fx, g.lanes.bottom() - 4, r);
        s.touch(1, Touch::Move, fx + 30, g.lanes.bottom() - 30, r);
        assert_eq!(
            s.touch(1, Touch::Up, fx + 30, g.lanes.bottom() - 30, r),
            Some(EventKind::Select(NONE))
        );
    }

    /// The zoom chip (6.4): 4 to 64 bars, the shown one lit and faint as
    /// a target; up close a drag snaps to the beat and the tag says it.
    #[test]
    fn the_zoom_chip_shows_4_to_64_bars_and_snaps_to_the_beat_up_close() {
        let mut s = lanes();
        let r = Rect::new(12, 140, 814, 534);
        let chip = s.view_rect(r);
        let zoom = s.zoom_rect(r);
        assert!(
            zoom.x > chip.right() && zoom.right() < s.rec_rect(r).x,
            "{zoom:?}"
        );
        for k in 0..ZOOMS.len() {
            let p = s.zoom_part_rect(r, k);
            assert_eq!(
                s.touch(1, Touch::Down, p.x + 3, p.y + 3, r),
                Some(EventKind::Tab(ZOOM + k))
            );
        }
        let lit: Vec<_> = s
            .targets(r)
            .into_iter()
            .filter(|t| t.id == "zoom" && !t.enabled)
            .map(|t| t.index)
            .collect();
        assert_eq!(lit, [2], "16 bars shown");
        // Four bars: a bar is a quarter of the lane, and a block dragged a
        // beat and a bit lands on beat 2.
        s.bars = 4;
        let b = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        let lane_w = r.w - 28 - NAME_W;
        assert!((b.w - lane_w).abs() <= 3, "four bars fill the lane: {b:?}");
        let beat_px = lane_w / 16;
        let (x, y) = (b.x + 10, b.y + b.h / 2);
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + beat_px + 4, y, r);
        assert_eq!(s.at_text(960), "bar 1 beat 2");
        assert_eq!(s.at_text(2 * 3840), "bar 3");
        assert_eq!(
            s.touch(1, Touch::Up, x + beat_px + 4, y, r),
            Some(EventKind::Place {
                index: 0,
                row: 0,
                at: 960
            })
        );
        // Sixty-four: back to the bar.
        s.bars = 64;
        assert_eq!(s.snap(), 3840);
    }

    #[test]
    fn the_picked_blocks_end_grip_makes_it_longer_or_shorter() {
        let mut s = lanes();
        let r = Rect::new(12, 140, 814, 534);
        assert!(s.end_rect(r).is_none(), "nothing picked");
        s.selected = Some(0);
        let e = s.end_rect(r).unwrap();
        let bar_px = (r.w - 28 - NAME_W) / 16;
        let (x, y) = (e.x + e.w / 2, e.y + e.h / 2);
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 2 * bar_px, y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x + 2 * bar_px - 4, y, r),
            Some(EventKind::Stretch {
                index: 0,
                to: 6 * 3840
            })
        );
        // Dragged far back: a bar at least.
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x - 10 * bar_px, y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x - 10 * bar_px, y, r),
            Some(EventKind::Stretch { index: 0, to: 3840 })
        );
        // A tap on the grip keeps the pick.
        s.touch(1, Touch::Down, x, y, r);
        assert_eq!(s.touch(1, Touch::Up, x, y, r), None);
        let t = s.targets(r);
        assert!(t
            .iter()
            .any(|t| t.id == "block-end" && t.gesture == mpfree_ui::Gesture::Drag));
    }

    /// Phase 6: the picked block's start grip trims it from the left (to
    /// the bar, its last bar kept), and the block held still before a drag
    /// slides what it plays (to the 1/16) instead of moving it.
    #[test]
    fn the_start_grip_trims_and_a_held_drag_slides() {
        let mut s = lanes();
        let r = Rect::new(12, 140, 814, 534);
        assert!(s.start_rect(r).is_none(), "nothing picked");
        s.selected = Some(0);
        let g = s.start_rect(r).unwrap();
        let bar_px = (r.w - 28 - NAME_W) / 16;
        let (x, y) = (g.x + g.w / 2 + 4, g.y + g.h / 2);
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 2 * bar_px, y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x + 2 * bar_px + 3, y, r),
            Some(EventKind::Place {
                index: 0,
                row: START_ROW,
                at: 2 * 3840
            })
        );
        // Dragged past its end: its last bar stays.
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 10 * bar_px, y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x + 10 * bar_px, y, r),
            Some(EventKind::Place {
                index: 0,
                row: START_ROW,
                at: 3 * 3840
            })
        );
        // A tap on the grip keeps the pick.
        s.touch(1, Touch::Down, x, y, r);
        assert_eq!(s.touch(1, Touch::Up, x, y, r), None);
        assert!(s
            .targets(r)
            .iter()
            .any(|t| t.id == "block-start" && t.gesture == mpfree_ui::Gesture::Drag));
        // Back no further than the block before it on its lane.
        let mut q = lanes();
        q.blocks = SongBlocks::new(vec![
            SongBlock {
                index: 0,
                track: 0,
                start: 0,
                length: 3840 + 960,
                name: "Before".into(),
                unique: false,
            },
            SongBlock {
                index: 1,
                track: 0,
                start: 4 * 3840,
                length: 2 * 3840,
                name: "Picked".into(),
                unique: false,
            },
        ]);
        q.selected = Some(1);
        let g = q.start_rect(r).unwrap();
        let (x, y) = (g.x + g.w / 2 + 4, g.y + g.h / 2);
        q.touch(1, Touch::Down, x, y, r);
        q.touch(1, Touch::Move, x - 6 * bar_px, y, r);
        assert_eq!(
            q.touch(1, Touch::Up, x - 6 * bar_px, y, r),
            Some(EventKind::Place {
                index: 1,
                row: START_ROW,
                at: 3840 + 960
            })
        );
        // The block's middle: dragged at once it moves; held still first,
        // what it plays slides a bar later.
        let b = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        let (mx, my) = (b.x + b.w / 2, b.y + b.h / 2);
        s.touch(1, Touch::Down, mx, my, r);
        s.touch(1, Touch::Move, mx + bar_px, my, r);
        assert_eq!(
            s.touch(1, Touch::Up, mx + bar_px, my, r),
            Some(EventKind::Place {
                index: 0,
                row: 0,
                at: 3840
            })
        );
        s.touch(1, Touch::Down, mx, my, r);
        if let Some(gr) = s.grip.as_mut() {
            gr.since -= SLIDE_HOLD;
        }
        s.touch(1, Touch::Move, mx + bar_px, my, r);
        assert!(matches!(
            s.grip.map(|g| g.held),
            Some(Held::Slide { index: 0 })
        ));
        assert_eq!(
            s.touch(1, Touch::Up, mx + bar_px, my, r),
            Some(EventKind::Place {
                index: 0,
                row: SLIDE_ROW,
                at: 3840
            })
        );
        assert_eq!(s.slide_text(3840), "+1 bar");
        assert_eq!(s.slide_text(-960), "−1 beat");
        assert_eq!(s.slide_text(720), "+3/16");
        // A block not picked moves, held or not.
        s.selected = None;
        s.touch(1, Touch::Down, mx, my, r);
        if let Some(gr) = s.grip.as_mut() {
            gr.since -= SLIDE_HOLD;
        }
        s.touch(1, Touch::Move, mx + bar_px, my, r);
        assert!(matches!(s.grip.map(|g| g.held), Some(Held::Block { .. })));
        s.touch(1, Touch::Cancel, mx, my, r);
    }

    #[test]
    fn sections_are_picked_by_a_tap_and_dragged_by_their_names() {
        let mut s = lanes();
        let bar = 3840;
        s.markers = vec![
            (0, "Intro".into()),
            (4 * bar, "Verse".into()),
            (8 * bar, "Out".into()),
        ];
        let mut all = s.blocks.to_vec();
        all[1].start = 10 * bar;
        s.blocks = all.into();
        let r = Rect::new(12, 140, 814, 534);
        let bar_px = (r.w - 28 - NAME_W) / 16;
        let m = s.marker_rect(r, 1).unwrap();
        let (x, y) = (m.x + 10, m.y + m.h / 2);
        s.touch(1, Touch::Down, x, y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x, y, r),
            Some(EventKind::Tab(SECTION_TAP + 1))
        );
        // The last section runs to the song's end in whole bars.
        assert_eq!(s.edge_tick(3), Some(11 * bar));
        // Verse dragged back to the start: before Intro.
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x - 4 * bar_px + 8, y + 30, r);
        assert_eq!(
            s.touch(1, Touch::Up, x - 4 * bar_px + 8, y + 30, r),
            Some(EventKind::Drop { from: 1, to: 0 })
        );
        // On past Out's end: after the last.
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 7 * bar_px, y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x + 7 * bar_px, y, r),
            Some(EventKind::Drop { from: 1, to: 3 })
        );
        let t = s.targets(r);
        assert_eq!(t.iter().filter(|t| t.id == "section").count(), 3);
    }

    /// Arrange 3.3: a tap on the ruler where no name is adds a marker on
    /// the nearest bar; a swipe there does nothing; the picked section's
    /// grip, left of its marker, drags the marker alone to the bar; and a
    /// picked section turns the last four buttons into its own.
    #[test]
    fn a_tap_on_the_ruler_adds_a_marker_and_the_grip_moves_it() {
        let mut s = lanes();
        let bar = 3840;
        s.markers = vec![(0, "Intro".into()), (4 * bar, "Verse".into())];
        let r = Rect::new(12, 140, 814, 534);
        let ruler = s.ruler_rect(r);
        let bar_px = f64::from(ruler.w) / 16.0;
        let x_at = |b: f64| ruler.x + (b * bar_px).round() as i32;
        let y = ruler.y + ruler.h / 2;
        // Bar 11 and a bit: past Verse's name, the nearest bar line.
        let x = x_at(10.3);
        s.touch(1, Touch::Down, x, y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x, y, r),
            Some(EventKind::Place {
                index: NONE,
                row: RULER_ROW,
                at: 10 * bar
            })
        );
        s.touch(1, Touch::Down, x, y, r);
        s.touch(1, Touch::Move, x + 3 * bar_px as i32, y, r);
        assert_eq!(s.touch(1, Touch::Up, x + 3 * bar_px as i32, y, r), None);
        // No grip until a section is picked; then left of its marker.
        assert_eq!(s.grip_rect(r), None);
        assert!(!s.section_buttons());
        s.section = Some(1);
        let gp = s.grip_rect(r).unwrap();
        assert_eq!((gp.right(), gp.w), (x_at(4.0), GRIP_W));
        assert!(s.section_buttons());
        let (gx, gy) = (gp.x + gp.w / 2, gp.y + gp.h / 2);
        // A tap on it does nothing; dragged two bars on, Verse's marker
        // lands on bar 7.
        s.touch(1, Touch::Down, gx, gy, r);
        assert_eq!(s.touch(1, Touch::Up, gx, gy, r), None);
        s.touch(1, Touch::Down, gx, gy, r);
        s.touch(1, Touch::Move, gx + 2 * bar_px as i32, gy, r);
        assert_eq!(
            s.touch(1, Touch::Up, gx + 2 * bar_px as i32, gy, r),
            Some(EventKind::Place {
                index: 1,
                row: RULER_ROW,
                at: 6 * bar
            })
        );
        // Let go where it started: nothing.
        s.touch(1, Touch::Down, gx, gy, r);
        s.touch(1, Touch::Move, gx, gy + 20, r);
        assert_eq!(s.touch(1, Touch::Up, gx, gy + 20, r), None);
        let t = s.targets(r);
        let words: Vec<&str> = t
            .iter()
            .filter(|t| t.id == "section-button")
            .map(|t| t.label.as_str())
            .collect();
        assert_eq!(words, SECTION_BUTTONS);
        assert_eq!(t.iter().filter(|t| t.id == "section-grip").count(), 1);
        assert_eq!(t.iter().filter(|t| t.id == "ruler").count(), 1);
        // The buttons still fit where they were.
        let b = s.button_rects(r);
        assert!(b[5].right() <= r.right());
        // A tempo point picked takes the last buttons back.
        s.tempo = vec![(0, 120.0, false)];
        s.tempo_pick = Some(0);
        assert!(!s.section_buttons());
    }

    /// Tempo ramps: the tempo lane sits under the ruler, above the lanes;
    /// a tap on it adds a point on the nearest bar, a tap on a point picks
    /// it, a drag moves it to the bar; a picked point turns the last two
    /// buttons into Ramp (lit while it ramps) and Delete.
    #[test]
    fn the_tempo_lane_adds_picks_and_moves_points() {
        let mut s = lanes();
        let bar = 3840;
        s.tempo_base = 92.0;
        s.tempo = vec![(4 * bar, 100.0, true), (8 * bar, 140.0, false)];
        let r = Rect::new(12, 140, 814, 534);
        let g = s.geo(r);
        let lane = s.tempo_rect(r);
        assert!(lane.y >= g.ruler.bottom() && lane.bottom() < g.lanes.y);
        assert_eq!(
            (lane.x, lane.right()),
            (g.lanes.x + NAME_W, g.lanes.right())
        );
        let bar_px = f64::from(lane.w) / 16.0;
        let x_at = |b: f64| lane.x + (b * bar_px).round() as i32;
        let y = lane.y + lane.h / 2;
        // A tap a little past bar 13's line: a point on bar 13.
        s.touch(1, Touch::Down, x_at(12.2), lane.y + 3, r);
        assert_eq!(
            s.touch(1, Touch::Up, x_at(12.2), lane.y + 3, r),
            Some(EventKind::Place {
                index: NONE,
                row: TEMPO_ROW,
                at: 12 * bar
            })
        );
        // A tap on the second point picks it.
        let p = s.tempo_point_rect(r, 1).unwrap();
        assert!(p.contains(x_at(8.0), p.y + p.h / 2));
        s.touch(1, Touch::Down, x_at(8.0) + 3, p.y + p.h / 2, r);
        assert_eq!(
            s.touch(1, Touch::Up, x_at(8.0) + 3, p.y + p.h / 2, r),
            Some(EventKind::Cell {
                row: TEMPO_ROW,
                col: 1
            })
        );
        // Dragged two bars on: it lands on bar 11.
        s.touch(1, Touch::Down, x_at(8.0), p.y + p.h / 2, r);
        s.touch(1, Touch::Move, x_at(10.1), y, r);
        assert_eq!(
            s.touch(1, Touch::Up, x_at(10.1), y, r),
            Some(EventKind::Place {
                index: 1,
                row: TEMPO_ROW,
                at: 10 * bar
            })
        );
        // Dragged and brought back: nothing.
        s.touch(1, Touch::Down, x_at(8.0), p.y + p.h / 2, r);
        s.touch(1, Touch::Move, x_at(9.5), y, r);
        s.touch(1, Touch::Move, x_at(8.1), y, r);
        assert_eq!(s.touch(1, Touch::Up, x_at(8.1), y, r), None);
        // The faster point sits higher in the lane than the slower one.
        let (p0, p1) = (s.tempo_point_rect(r, 0).unwrap(), p);
        assert!(p1.y < p0.y);
        // Picked: it paints, with Ramp and Delete in the bottom row.
        s.tempo_pick = Some(0);
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        ui.add("song", s.clone());
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
        let b = s.button_rects(r);
        assert_eq!(
            s.touch(1, Touch::Down, b[4].x + 3, b[4].y + 3, r),
            Some(EventKind::Tab(4))
        );
    }

    /// The audio lanes, each as a track at the end of the track list.
    fn put_audio(s: &mut SongLanes, lanes: Vec<AudioLane>) {
        s.audio = lanes;
        for l in &mut s.audio {
            l.row = s.tracks.len();
            s.tracks.push((l.name.clone(), l.color));
        }
    }

    fn audio_lane(track: usize, name: &str, blocks: Vec<AudioBlock>) -> AudioLane {
        AudioLane {
            track,
            row: usize::MAX,
            name: name.into(),
            color: Color::rgb(0x2EC4B6),
            armed: false,
            input: Input::Left,
            mix: None,
            level: 0.5,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: false,
            solo: false,
            blocks,
            takes: Vec::new(),
            recording: None,
            clips: Vec::new(),
        }
    }

    fn block(start: Tick, end: Tick, take: u16) -> AudioBlock {
        AudioBlock {
            start,
            end,
            take,
            tag: String::new(),
            src: 0,
            frames: 1000.0,
            gain: 1.0,
            fade_in: 0,
            fade_out: 0,
            into: 0,
            left: 0,
        }
    }

    #[test]
    fn audio_lanes_sit_in_their_tracks_places() {
        let mut s = lanes();
        put_audio(
            &mut s,
            vec![
                audio_lane(0, "Vocal", vec![block(0, 4 * 3840, 0)]),
                audio_lane(3, "Guitar", vec![block(3840, 2 * 3840, 1)]),
            ],
        );
        let r = Rect::new(12, 140, 814, 534);
        let midi = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        let vocal = s.region_rect(r, 0, 0).unwrap();
        let guitar = s.region_rect(r, 1, 0).unwrap();
        // Same ruler: the vocal starts where the drums' bar 1 does, and
        // the lanes go down in order with the same height.
        assert_eq!(vocal.x, midi.x);
        assert!(vocal.y > midi.y && guitar.y > vocal.y);
        // Two note tracks (Drums, Bass), then Vocal and Guitar.
        assert_eq!(guitar.y - vocal.y, (vocal.y - midi.y) / 2);
        assert_eq!(vocal.h, midi.h);
        // Taps: a region is a Cell of its audio track and index.
        assert_eq!(
            s.touch(1, Touch::Down, vocal.x + 3, vocal.y + 3, r),
            Some(EventKind::Cell { row: 0, col: 0 })
        );
        assert_eq!(
            s.touch(1, Touch::Down, guitar.x + 3, guitar.y + 3, r),
            Some(EventKind::Cell { row: 3, col: 0 })
        );
        // The arm dot answers on the way down; the name on the lift.
        let dot = s.arm_rect(r, 1).unwrap();
        assert_eq!(
            s.touch(2, Touch::Down, dot.x + 4, dot.y + dot.h / 2, r),
            Some(EventKind::Toggle(3))
        );
        assert_eq!(s.touch(2, Touch::Up, dot.x + 4, dot.y + dot.h / 2, r), None);
        let head = s.header_rect(r, 0).unwrap();
        let (x, y) = (head.x + 40, head.y + head.h / 2);
        assert_eq!(s.touch(3, Touch::Down, x, y, r), None);
        assert_eq!(
            s.touch(3, Touch::Up, x, y, r),
            Some(EventKind::Cell { row: 0, col: PANEL })
        );
        // A MIDI lane's name picks nothing, as empty space does.
        let (x, y) = (r.x + 20, midi.y + 4);
        assert_eq!(s.touch(4, Touch::Down, x, y, r), None);
        assert_eq!(
            s.touch(4, Touch::Up, x, y, r),
            Some(EventKind::Select(NONE))
        );
        // An audio track between two note tracks has its lane there.
        let mut s = lanes();
        s.tracks.insert(1, ("Vocal".into(), Color::rgb(0x2EC4B6)));
        s.blocks = SongBlocks::new(vec![SongBlock {
            index: 0,
            track: 2,
            start: 0,
            length: 3840,
            name: "Late".into(),
            unique: false,
        }]);
        s.audio = vec![AudioLane {
            row: 1,
            ..audio_lane(5, "Vocal", vec![block(0, 3840, 0)])
        }];
        assert_eq!(
            (0..3).map(|k| s.lane(k)).collect::<Vec<_>>(),
            [
                Some(Lane::Midi(0)),
                Some(Lane::Audio(0)),
                Some(Lane::Midi(2))
            ]
        );
        let vocal = s.region_rect(r, 0, 0).unwrap();
        let bass = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        assert!(bass.y > vocal.y, "Bass's placement is under the vocal");
        assert_eq!(
            s.touch(6, Touch::Down, vocal.x + 3, vocal.y + 3, r),
            Some(EventKind::Cell { row: 5, col: 0 })
        );
    }

    /// A block of an audio clip (phase 2, step 2d, layout spec 4c): it
    /// sits on its audio track's lane, draws a header strip in the track's
    /// colour over its wave, is picked and dragged as any block, and shows
    /// red where it would land over a region.
    #[test]
    fn a_block_of_an_audio_clip_sits_on_its_lane_with_a_header_strip() {
        let bar = 3840;
        let mut s = lanes();
        put_audio(
            &mut s,
            vec![audio_lane(0, "Guitar", vec![block(0, bar, 0)])],
        );
        let row = s.audio[0].row;
        let mut all: Vec<SongBlock> = s.blocks.iter().cloned().collect();
        let index = all.len();
        all.push(SongBlock {
            index,
            track: row,
            start: 2 * bar,
            length: 2 * bar,
            name: "Riff".into(),
            unique: false,
        });
        s.blocks = SongBlocks::new(all);
        s.audio[0].clips.push(ClipBlock {
            index,
            origin: 2 * bar,
            pass: bar,
            once: false,
            pieces: vec![ClipPiece {
                start: 0,
                end: bar,
                take: 0,
                src: 0,
                frames: 1000.0,
            }],
        });
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        ui.add("song", s.clone());
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        let r = ui.rect_of("song").unwrap();
        let region = s.region_rect(r, 0, 0).unwrap();
        let br = s.block_rect(r, s.block(index).unwrap()).unwrap();
        assert_eq!((br.y, br.h), (region.y, region.h), "on Guitar's lane");
        assert!(br.x > region.right());
        // The strip in the track's colour along its top; the lane's dark
        // fill under it.
        let colour = s.audio[0].color;
        let x = br.x + br.w - 6;
        assert_eq!(c.pixel(x, br.y + 3).xrgb(), colour.xrgb(), "the strip");
        assert_ne!(
            c.pixel(x, br.bottom() - 3).xrgb(),
            colour.xrgb(),
            "the body"
        );
        // Picked by a tap, as any block.
        let (x, y) = (br.x + 8, br.y + br.h / 2);
        assert_eq!(s.touch(1, Touch::Down, x, y, r), None);
        assert_eq!(
            s.touch(1, Touch::Up, x, y, r),
            Some(EventKind::Select(index))
        );
        // Dragged back over the region on bar 1: it lands there (the app
        // refuses it and says why), and shows red on the way.
        assert!(s.region_between(row, 0, 2 * bar));
        assert!(!s.region_between(row, 2 * bar, 4 * bar));
        let to = region.x + 2;
        s.touch(2, Touch::Down, x, y, r);
        s.touch(2, Touch::Move, to, y, r);
        assert_eq!(
            s.touch(2, Touch::Up, to, y, r),
            Some(EventKind::Place { index, row, at: 0 })
        );
    }

    #[test]
    fn a_picked_region_swaps_the_bottom_row() {
        let mut s = lanes();
        put_audio(
            &mut s,
            vec![audio_lane(0, "Vocal", vec![block(0, 3840, 0)])],
        );
        let r = Rect::new(12, 140, 814, 534);
        s.region = Some((0, 0));
        for (k, br) in s.button_rects(r).iter().enumerate() {
            assert!(br.right() <= r.right() - 14, "{k}: {br:?}");
            assert_eq!(
                s.touch(1, Touch::Down, br.x + 3, br.y + 3, r),
                Some(EventKind::Tab(k))
            );
        }
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        ui.add("song", s);
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
    }

    /// Layout spec 4c: the track panel opens over the lanes, right of the
    /// names, and its controls report themselves; touches on it never
    /// reach the blocks under it.
    #[test]
    fn the_track_panel_sits_over_the_lanes_and_reports_its_controls() {
        let mut s = lanes();
        put_audio(
            &mut s,
            vec![audio_lane(1, "Vocal", vec![block(0, 3840, 0)])],
        );
        let r = Rect::new(12, 140, 814, 534);
        assert_eq!(s.panel_parts(r), None, "no panel open");
        s.panel = Some(4);
        assert_eq!(s.panel_parts(r), None, "no lane for that track");
        s.panel = Some(1);
        let p = s.panel_parts(r).unwrap();
        let g = s.geo(r);
        assert!(p.panel.x >= g.lanes.x + NAME_W && p.panel.right() <= g.lanes.right());
        assert!(p.panel.y >= g.lanes.y && p.panel.bottom() <= g.lanes.bottom());
        let every = [p.close, p.arm, p.mute, p.solo, p.takes]
            .into_iter()
            .chain(p.inputs)
            .chain(p.monitors)
            .chain(p.level)
            .chain(p.pan)
            .chain([p.strip, p.note]);
        // "Its strip in Mix ›" ends the level's row, clear of the pan, with
        // the line saying what it is under it (phase 1b, as drawn).
        assert!(
            p.pan[2].right() + 8 <= p.strip.x,
            "{:?} {:?}",
            p.pan,
            p.strip
        );
        assert!(p.strip.bottom() <= p.note.y);
        for b in every {
            assert!(
                p.panel.inset(4).contains(b.x, b.y),
                "{b:?} in {:?}",
                p.panel
            );
            assert!(
                b.right() <= p.panel.right() - 4 && b.bottom() <= p.panel.bottom() - 4,
                "{b:?} in {:?}",
                p.panel
            );
        }
        // The track's lane stays in view: the panel goes under it here,
        // and over it when the lane is low.
        let lane = s.lane_rect(&g, s.audio_k(0)).unwrap();
        assert!(lane.bottom() <= p.panel.y, "{lane:?} over {:?}", p.panel);
        let mut low = s.clone();
        low.tracks = (0..5)
            .map(|k| (format!("T{k}"), Color::rgb(0x6EA8FE)))
            .collect();
        low.tracks.push(("Vocal".into(), Color::rgb(0x2EC4B6)));
        low.audio[0].row = 5;
        let lp = low.panel_parts(r).unwrap();
        let lane = low.lane_rect(&low.geo(r), low.audio_k(0)).unwrap();
        assert!(lp.panel.bottom() <= lane.y, "{lane:?} under {:?}", lp.panel);
        let mut tap = |b: Rect| s.touch(1, Touch::Down, b.x + 3, b.y + 3, r);
        assert_eq!(tap(p.close), Some(EventKind::Tab(PANEL_CLOSE)));
        assert_eq!(tap(p.arm), Some(EventKind::Toggle(1)));
        assert_eq!(tap(p.mute), Some(EventKind::Tab(PANEL_MUTE)));
        assert_eq!(tap(p.solo), Some(EventKind::Tab(PANEL_SOLO)));
        assert_eq!(tap(p.takes), Some(EventKind::Tab(PANEL_TAKES)));
        for k in 0..INPUTS.len() {
            assert_eq!(tap(p.inputs[k]), Some(EventKind::Tab(PANEL_INPUT + k)));
        }
        for k in 0..MONITORS.len() {
            assert_eq!(tap(p.monitors[k]), Some(EventKind::Tab(PANEL_MONITOR + k)));
        }
        let step = |index, delta| Some(EventKind::Step { index, delta });
        assert_eq!(tap(p.level[0]), step(PANEL_LEVEL, -1));
        assert_eq!(tap(p.level[2]), step(PANEL_LEVEL, 1));
        assert_eq!(tap(p.pan[0]), step(PANEL_PAN, -1));
        assert_eq!(tap(p.pan[2]), step(PANEL_PAN, 1));
        assert_eq!(tap(p.strip), Some(EventKind::Tab(PANEL_STRIP)));
        // Its empty space and its value read-outs take the touch and ask
        // for nothing, even with a block under them.
        assert_eq!(tap(p.level[1]), None);
        let mut all = low.blocks.to_vec();
        all.push(SongBlock {
            index: 2,
            track: 3,
            start: 0,
            length: 4 * 3840,
            name: "Under".into(),
            unique: false,
        });
        low.blocks = all.into();
        let under = low.block_rect(r, &low.blocks[2].clone()).unwrap();
        assert!(lp.panel.contains(under.x + 3, under.y + 3));
        assert_eq!(low.touch(1, Touch::Down, under.x + 3, under.y + 3, r), None);
        low.panel = None;
        assert_eq!(low.touch(1, Touch::Down, under.x + 3, under.y + 3, r), None);
        assert_eq!(
            low.touch(1, Touch::Up, under.x + 3, under.y + 3, r),
            Some(EventKind::Select(2))
        );
        // The names stay live: another header opens its own panel.
        let h = s.header_rect(r, 0).unwrap();
        s.touch(1, Touch::Down, h.x + 40, h.y + 5, r);
        assert_eq!(
            s.touch(1, Touch::Up, h.x + 40, h.y + 5, r),
            Some(EventKind::Cell { row: 1, col: PANEL })
        );
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        ui.add("song", s);
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
    }

    /// Mix (the instrument redesign's step 9, proposed in layout spec
    /// 4c): the panel grows a row, From and Effects, inside it; the
    /// monitor modes give way to a line (a track recording from the mix
    /// never plays it through) and take no touch; the header's chip
    /// reads "Mix".
    #[test]
    fn mix_gives_the_panel_from_and_effects_and_the_monitors_give_way() {
        let mut s = lanes();
        put_audio(
            &mut s,
            vec![audio_lane(1, "Vocal", vec![block(0, 3840, 0)])],
        );
        s.panel = Some(1);
        let r = Rect::new(12, 140, 814, 534);
        let line = s.panel_parts(r).unwrap();
        assert_eq!((line.from, line.effects), (None, None));
        let tap = |s: &mut SongLanes, b: Rect| s.touch(1, Touch::Down, b.x + 3, b.y + 3, r);
        assert_eq!(tap(&mut s, line.mix), Some(EventKind::Tab(PANEL_MIX)));
        s.audio[0].mix = Some(MixFrom {
            name: "FM · Pads".into(),
            after_effects: true,
        });
        let p = s.panel_parts(r).unwrap();
        assert_eq!(p.panel.h, line.panel.h + PANEL_ROW + 5, "a row more");
        let (from, fx) = (p.from.unwrap(), p.effects.unwrap());
        let every = [p.mix]
            .into_iter()
            .chain(p.inputs)
            .chain(from)
            .chain(fx)
            .chain(p.level)
            .chain(p.pan)
            .chain([p.strip, p.note]);
        for b in every {
            assert!(
                p.panel.inset(4).contains(b.x, b.y)
                    && b.right() <= p.panel.right() - 4
                    && b.bottom() <= p.panel.bottom() - 4,
                "{b:?} in {:?}",
                p.panel
            );
        }
        assert!(from[2].right() < fx[0].x, "From, then Effects");
        assert!(p.level[0].y > from[0].y, "level and pan under them");
        assert_eq!(tap(&mut s, from[0]), Some(EventKind::Tab(PANEL_FROM_LESS)));
        assert_eq!(tap(&mut s, from[2]), Some(EventKind::Tab(PANEL_FROM_MORE)));
        assert_eq!(tap(&mut s, from[1]), None, "the name only shows");
        assert_eq!(tap(&mut s, fx[0]), Some(EventKind::Tab(PANEL_BEFORE)));
        assert_eq!(tap(&mut s, fx[1]), Some(EventKind::Tab(PANEL_AFTER)));
        assert_eq!(tap(&mut s, p.monitors[1]), None, "no monitor while mixed");
        assert_eq!(tap(&mut s, p.inputs[0]), Some(EventKind::Tab(PANEL_INPUT)));
        let t = s.targets(r);
        let has = |id: &str| t.iter().any(|t| t.id == id);
        assert!(has("panel-from-less") && has("panel-from-more") && has("panel-effects"));
        assert!(!has("panel-monitor"));
        let mix = t.iter().find(|t| t.id == "panel-mix").unwrap();
        assert!(!mix.enabled, "Mix is lit: a tap changes nothing");
        let input = t.iter().find(|t| t.id == "panel-input").unwrap();
        assert!(input.enabled, "an input chip goes back to the line input");
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        ui.add("song", s);
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
    }

    /// "+ Track" is the last lane, after every track's (up to 128
    /// tracks, Jay 14 Nov; layout spec 4c), and opens the chooser.
    #[test]
    fn plus_track_is_the_last_lane_and_opens_the_chooser() {
        let mut s = lanes();
        put_audio(&mut s, vec![audio_lane(0, "Vocal", Vec::new())]);
        let r = Rect::new(12, 140, 814, 534);
        assert_eq!(s.lane(2), Some(Lane::Audio(0)));
        assert!(s.add_track_rect(r).is_none());
        s.can_add_track = true;
        // Drums, Bass, Vocal, + Track.
        assert_eq!(s.lane_count(), 4);
        assert_eq!(s.lane(3), Some(Lane::AddTrack));
        assert_eq!(s.audio_k(0), 2);
        let b = s.add_track_rect(r).unwrap();
        assert!(
            b.y >= s.header_rect(r, 0).unwrap().bottom(),
            "under the last track"
        );
        assert_eq!(
            s.touch(1, Touch::Down, b.x + 4, b.y + 4, r),
            Some(EventKind::Tab(ADD_TRACK))
        );
        // The app opens the chooser: a card beside "+ Track", right of
        // the names, a row a kind; a touch anywhere else closes it and
        // does nothing more.
        s.adding = true;
        let rows = s.chooser_rects(r).unwrap();
        assert!(rows[0].x > b.right(), "right of the names");
        assert!(
            rows[0].y < b.bottom() && rows[2].bottom() > b.y,
            "level with it"
        );
        for (k, row) in rows.iter().enumerate() {
            assert_eq!(
                s.touch(2, Touch::Down, row.x + 8, row.y + 8, r),
                Some(EventKind::Tab(ADD_KINDS[k].1))
            );
        }
        let vocal = s.header_rect(r, 0).unwrap();
        assert_eq!(
            s.touch(3, Touch::Down, vocal.x + 4, vocal.y + 4, r),
            Some(EventKind::Tab(ADD_CLOSE))
        );
        assert_eq!(s.touch(3, Touch::Up, vocal.x + 4, vocal.y + 4, r), None);
        let targets = s.targets(r);
        assert_eq!(targets.len(), 4, "the rows and the rest");
        // With "+ Track" at the foot of the lanes, the card stays inside
        // the view.
        put_audio(
            &mut s,
            (0..10).map(|k| audio_lane(k, "A", Vec::new())).collect(),
        );
        s.first_lane = usize::MAX;
        let b = s.add_track_rect(r).unwrap();
        let rows = s.chooser_rects(r).unwrap();
        assert!(rows[0].x > b.right(), "beside it still");
        assert!(rows[2].bottom() < r.bottom() - 8, "inside the view");
        assert!(rows[0].y - 36 >= r.y, "its title too");
    }

    fn take(id: u16, name: &str, start: Tick, end: Tick, used: Vec<(Tick, Tick)>) -> TakeLane {
        TakeLane {
            take: id,
            name: name.into(),
            start,
            end,
            frames: 88_200.0,
            recording: false,
            used,
        }
    }

    /// Layout spec 4c: Takes opens the track's take lanes, thin, under
    /// its lane, newest first; the other lanes give them room. A tap on
    /// one asks for the whole take and a swipe for the stretch it crossed.
    #[test]
    fn take_lanes_open_under_their_track_and_report_taps_and_swipes() {
        let mut s = lanes();
        let mut vocal = audio_lane(0, "Vocal", vec![block(0, 3840, 1)]);
        vocal.takes = vec![
            take(1, "take 2", 0, 3840, vec![(0, 3840)]),
            take(0, "take 1", 0, 3840, Vec::new()),
        ];
        put_audio(&mut s, vec![vocal, audio_lane(1, "Guitar", Vec::new())]);
        let r = Rect::new(12, 140, 814, 534);
        s.can_add_track = true;
        assert_eq!(s.lane_count(), 5, "closed: no take lanes");
        s.takes_open = Some(0);
        let g = s.geo(r);
        // Drums, Bass, Vocal, take 2, take 1, Guitar, + Track.
        assert_eq!(s.lane_count(), 7);
        assert_eq!(s.lane(2), Some(Lane::Audio(0)));
        assert_eq!(s.lane(3), Some(Lane::Take(0, 0)));
        assert_eq!(s.lane(4), Some(Lane::Take(0, 1)));
        assert_eq!(s.lane(5), Some(Lane::Audio(1)));
        assert_eq!(s.lane(6), Some(Lane::AddTrack));
        assert_eq!(s.audio_k(1), 5);
        let lane = s.lane_rect(&g, 2).unwrap();
        let t2 = s.lane_rect(&g, 3).unwrap();
        let t1 = s.lane_rect(&g, 4).unwrap();
        assert_eq!((t2.h, t1.h), (TAKE_H, TAKE_H));
        assert_eq!((t2.y, t1.y), (lane.bottom(), t2.bottom()));
        assert_eq!(s.header_rect(r, 1).unwrap().y, t1.bottom());
        assert!(
            s.lane_rect(&g, 6).unwrap().bottom() <= g.lanes.bottom(),
            "all in view"
        );
        // A tap on take 1's lane: the whole take.
        let (x, y) = (t1.x + NAME_W + 100, t1.y + 10);
        assert_eq!(s.touch(1, Touch::Down, x, y, r), None);
        assert_eq!(
            s.touch(1, Touch::Up, x + 2, y, r),
            Some(EventKind::Span {
                index: 0,
                from: 0,
                to: 0
            })
        );
        // A swipe: the stretch it crossed, either way round, even if the
        // finger drifts off the lane.
        let want = Some(EventKind::Span {
            index: 0,
            from: s.tick_at(g.lanes, x),
            to: s.tick_at(g.lanes, x + 120),
        });
        s.touch(2, Touch::Down, x, y, r);
        assert_eq!(s.touch(2, Touch::Move, x + 120, y + 40, r), None);
        assert_eq!(s.touch(2, Touch::Up, x + 120, y + 40, r), want);
        s.touch(3, Touch::Down, x + 120, y, r);
        assert_eq!(s.touch(3, Touch::Up, x, y, r), want);
        assert!(s.tick_at(g.lanes, x) < s.tick_at(g.lanes, x + 120));
        // Take 2's lane is its own.
        let (x2, y2) = (t2.x + NAME_W + 50, t2.y + 5);
        s.touch(4, Touch::Down, x2, y2, r);
        assert_eq!(
            s.touch(4, Touch::Up, x2, y2, r),
            Some(EventKind::Span {
                index: 1,
                from: 0,
                to: 0
            })
        );
        // A cancelled swipe asks for nothing; the names still scroll.
        s.touch(5, Touch::Down, x, y, r);
        assert_eq!(s.touch(5, Touch::Cancel, x, y, r), None);
        assert_eq!(s.swipe, None);
        assert_eq!(s.touch(6, Touch::Down, t1.x + 30, y, r), None);
        assert!(s.drag.is_some(), "a finger on the names");
        s.touch(6, Touch::Cancel, t1.x + 30, y, r);
        // The audio lane's own region still picks.
        let br = s.region_rect(r, 0, 0).unwrap();
        assert_eq!(
            s.touch(7, Touch::Down, br.x + 4, br.y + 4, r),
            Some(EventKind::Cell { row: 0, col: 0 })
        );
        // Many take lanes push the rest down and the lanes scroll.
        let mut many = s.clone();
        many.audio[0].takes = (0..12)
            .rev()
            .map(|k| take(k, &format!("take {}", k + 1), 0, 3840, Vec::new()))
            .collect();
        let g = many.geo(r);
        assert!(many.max_first(&g) > 0, "they no longer fit");
        assert!(many.shown(&g) < many.lane_count());
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        s.swipe = Some((0, x, x + 60));
        ui.add("song", s);
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
    }

    /// Layout spec 4c: a region is picked as the finger goes down; dragged
    /// sideways and let go, it asks to start where it was dropped.
    #[test]
    fn a_dragged_region_asks_for_its_new_start() {
        let mut s = lanes();
        put_audio(
            &mut s,
            vec![
                audio_lane(0, "Vocal", Vec::new()),
                audio_lane(3, "Bass", vec![block(0, 3840, 0), block(7680, 11520, 1)]),
            ],
        );
        let r = Rect::new(12, 140, 814, 534);
        let g = s.geo(r);
        let br = s.region_rect(r, 1, 1).unwrap();
        let (x, y) = (br.x + 10, br.y + 10);
        let bar_px = s.x_of(g.lanes, 3840) - s.x_of(g.lanes, 0);
        assert_eq!(
            s.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Cell { row: 3, col: 1 })
        );
        assert_eq!(s.touch(1, Touch::Move, x + bar_px, y + 3, r), None);
        let Some(EventKind::Marker { index, frame }) = s.touch(1, Touch::Up, x + bar_px, y, r)
        else {
            panic!("no move");
        };
        assert_eq!(index, 3 * REGION_SLOTS + 1);
        assert!((frame as i64 - 11520).abs() < 40, "a bar later: {frame}");
        // A small wobble is a tap, not a move.
        s.touch(2, Touch::Down, x, y, r);
        assert_eq!(s.touch(2, Touch::Up, x + 3, y, r), None);
        // Never before the song's start.
        s.touch(3, Touch::Down, x, y, r);
        let Some(EventKind::Marker { frame, .. }) = s.touch(3, Touch::Up, x - 4 * bar_px, y, r)
        else {
            panic!("no move");
        };
        assert_eq!(frame, 0);
        // A drag in progress draws where it would land.
        s.touch(4, Touch::Down, x, y, r);
        s.touch(4, Touch::Move, x + 40, y, r);
        assert!(s.moving.is_some());
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        ui.add("song", s.clone());
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
        assert_eq!(s.touch(4, Touch::Cancel, x + 40, y, r), None);
        assert_eq!(s.moving, None);
    }

    #[test]
    fn take_lanes_name_takes_oldest_first_and_list_them_newest_first() {
        let map = TempoMap::constant(120.0).unwrap();
        let info = |id: u16, track: usize, group: u16, pass: u16| TakeInfo {
            id,
            rec: mpfree_audiotracks::RecordingId(u128::from(id) + 1),
            track,
            group,
            pass,
            bpm: 120.0,
            anchor: 0.0,
            frames: 44_100,
            channels: 1,
            recording: false,
            cut_short: false,
        };
        let takes = [
            info(4, 0, 1, 0),
            info(2, 0, 0, 0),
            info(3, 1, 0, 0),
            info(7, 0, 0, 1),
        ];
        let regions = [Region {
            start: 0,
            end: 960,
            take: 7,
            src: 0,
            phase: 0.0,
            gain: 1.0,
            fade_in: 0,
            fade_out: 0,
            into: 0,
            left: 0,
        }];
        let lanes = take_lanes(0, &regions, &takes, &map, 44_100);
        let shown: Vec<_> = lanes.iter().map(|l| (l.take, l.name.as_str())).collect();
        assert_eq!(shown, [(4, "take 3"), (7, "take 2"), (2, "take 1")]);
        assert_eq!(lanes[1].used, [(0, 960)]);
        // One second at 120 BPM is two beats.
        assert_eq!((lanes[0].start, lanes[0].end), (0, 2 * 960));
    }

    #[test]
    fn input_chips_name_every_input() {
        let chips: Vec<&str> = [Input::Left, Input::Right, Input::Sum, Input::Stereo]
            .into_iter()
            .map(input_chip)
            .collect();
        assert_eq!(chips, ["L", "R", "L+R", "ST"]);
    }

    /// The longest song (`MAX_PLACEMENTS`, 4,096 blocks): 16 tracks of 256
    /// four-bar blocks end to end, by track and then start, as the
    /// arrangement keeps them.
    fn longest_song() -> SongLanes {
        let bar = 3840;
        SongLanes {
            tracks: (0..16)
                .map(|k| (format!("T{k}"), Color::rgb(0xE8A33D)))
                .collect(),
            blocks: SongBlocks::new(
                (0..4096)
                    .map(|i| SongBlock {
                        index: i,
                        track: i / 256,
                        start: (i % 256) as Tick * 4 * bar,
                        length: 4 * bar,
                        name: format!("Clip {}", i % 32),
                        unique: false,
                    })
                    .collect(),
            ),
            ..SongLanes::default()
        }
    }

    #[test]
    fn only_the_blocks_in_view_are_drawn_and_hit_tested() {
        // Arrange refactor 2.4: a frame finds the blocks in view by lane
        // and time, and they are the ones every block's own rectangle
        // says show.
        let mut s = longest_song();
        let r = Rect::new(12, 140, 1256, 600);
        for (first_bar, first_lane) in [(0, 0), (37, 0), (1000, 5), (1020, 9), (5000, 0)] {
            s.first_bar = first_bar;
            s.first_lane = first_lane;
            let every: Vec<(usize, Rect)> = s
                .blocks
                .iter()
                .filter_map(|b| Some((b.index, s.block_rect(r, b)?)))
                .collect();
            let hit: Vec<(usize, Rect)> = s
                .targets(r)
                .iter()
                .filter(|t| t.id == "block")
                .map(|t| (t.index, t.rect))
                .collect();
            assert_eq!(hit, every, "from bar {first_bar}, lane {first_lane}");
            let g = s.geo(r);
            // Five four-bar blocks at most in sixteen bars of each lane shown.
            assert!(hit.len() <= 5 * s.shown(&g), "{}", hit.len());
            if first_bar < 1024 {
                assert!(!hit.is_empty());
                let (index, br) = hit[hit.len() / 2];
                assert_eq!(s.touch(1, Touch::Down, br.x + 3, br.y + 3, r), None);
                assert_eq!(
                    s.touch(1, Touch::Up, br.x + 3, br.y + 3, r),
                    Some(EventKind::Select(index))
                );
            } else {
                assert!(hit.is_empty(), "past the song's end");
            }
        }
        // The song's end, for the last section's edge, is the last block's.
        s.markers = vec![(0, "All".into())];
        assert_eq!(s.edge_tick(1), Some(1024 * 3840));
        // It paints.
        s.first_bar = 300;
        let mut ui = mpfree_ui::Ui::new(
            mpfree_ui::Theme::dark(),
            mpfree_ui::Fonts::embedded(),
            1280,
            800,
        );
        ui.add("song", s);
        ui.add_page("p", "song").unwrap();
        ui.show("p").unwrap();
        let mut c = mpfree_ui::SoftCanvas::new(1280, 800);
        assert!(!ui.paint(&mut c).is_empty());
    }

    #[test]
    fn blocks_between_two_ticks_are_found_by_lane_overlapping_or_not() {
        // In any order and overlapping (the arrangement allows neither, but
        // the index does not rely on it), against looking at every block.
        let mut seed = 7u32;
        let mut next = |n: u32| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) % n
        };
        let mut all: Vec<SongBlock> = (0..300)
            .map(|i| SongBlock {
                index: i,
                track: next(4) as usize,
                start: Tick::from(next(2000)),
                length: Tick::from(1 + next(300)),
                name: String::new(),
                unique: false,
            })
            .collect();
        // Shuffled, so a block is not where its index says.
        for i in (1..all.len()).rev() {
            all.swap(i, next(i as u32 + 1) as usize);
        }
        let blocks = SongBlocks::new(all.clone());
        for _ in 0..500 {
            let (track, from) = (next(5) as usize, Tick::from(next(2400)) - 100);
            let to = from + Tick::from(next(400));
            let mut want: Vec<usize> = all
                .iter()
                .filter(|b| b.track == track && b.start < to && from < b.start + b.length)
                .map(|b| b.index)
                .collect();
            let mut got: Vec<usize> = blocks.between(track, from, to).map(|b| b.index).collect();
            want.sort_unstable();
            got.sort_unstable();
            assert_eq!(got, want, "track {track}, {from}..{to}");
        }
        for b in &all {
            assert_eq!(blocks.get(b.index), Some(b));
        }
        assert_eq!(blocks.get(300), None);
        let end = all.iter().map(|b| b.start + b.length).max();
        assert_eq!(Some(blocks.end()), end);
        assert_eq!(SongBlocks::default().end(), 0);
        assert_eq!(SongBlocks::default().between(0, 0, 100).count(), 0);
        // Equal by content, and a copy in one step.
        assert_eq!(blocks, SongBlocks::new(all));
        assert_ne!(blocks, SongBlocks::default());
    }

    #[test]
    fn many_lanes_scroll_with_a_drag_on_the_names() {
        let mut s = lanes();
        s.tracks = (0..8)
            .map(|k| (format!("T{k}"), Color::rgb(0xE8A33D)))
            .collect();
        put_audio(
            &mut s,
            (0..6)
                .map(|k| audio_lane(k, "A", vec![block(0, 3840, 0)]))
                .collect(),
        );
        s.can_add_track = true;
        let r = Rect::new(12, 140, 814, 534);
        let g = s.geo(r);
        // 8 + 6 + "+ Track" = 15 lanes at 32 px do not fit.
        assert_eq!(g.lane_h, MIN_LANE);
        let visible = s.shown(&g);
        assert!(visible < 15, "{visible}");
        assert!(
            s.region_rect(r, 5, 0).is_none(),
            "the last audio lane is below"
        );
        // Drag the names up two lanes and a bit.
        let x = r.x + 30;
        let y = g.lanes.y + 100;
        assert_eq!(s.touch(1, Touch::Down, x, y, r), None);
        assert_eq!(
            s.touch(1, Touch::Move, x, y - 4, r),
            None,
            "inside the slop"
        );
        assert_eq!(s.first_lane, 0);
        assert_eq!(s.touch(1, Touch::Move, x, y - 2 * MIN_LANE - 5, r), None);
        assert_eq!(s.first_lane, 2);
        // The lift after a drag is not a tap.
        assert_eq!(s.touch(1, Touch::Up, x, y - 2 * MIN_LANE - 5, r), None);
        // As far as it goes, and no further.
        s.touch(2, Touch::Down, x, y, r);
        s.touch(2, Touch::Move, x, y - 2000, r);
        s.touch(2, Touch::Up, x, y - 2000, r);
        assert_eq!(s.first_lane, 15 - visible);
        assert_eq!(s.first_lane, s.max_first(&g));
        assert!(s.region_rect(r, 5, 0).is_some());
        assert!(
            s.block_rect(r, &s.blocks[0].clone()).is_none(),
            "the drums scrolled off"
        );
        // New contents keep the scroll.
        let mut new = s.clone();
        new.first_lane = 0;
        new.keep_scroll(&s);
        assert_eq!(new.first_lane, s.first_lane);
    }

    #[test]
    fn audio_blocks_tag_older_takes_and_span_their_frames() {
        use mpfree_audiotracks::{Region, TakeInfo};
        let take = |id: u16, track: usize, group: u16, pass: u16| TakeInfo {
            id,
            rec: mpfree_audiotracks::RecordingId(u128::from(id) + 1),
            track,
            group,
            pass,
            bpm: 120.0,
            anchor: 0.0,
            frames: 100_000,
            channels: 1,
            recording: false,
            cut_short: false,
        };
        let region = |start: Tick, end: Tick, take: u16| Region {
            start,
            end,
            take,
            src: 10,
            phase: 0.0,
            gain: 1.0,
            fade_in: 5,
            fade_out: 7,
            into: 3,
            left: 0,
        };
        // Track 0's takes: 4 (group 0), 2 (group 1, pass 0), 9 (group 1,
        // pass 1, the newest). Take 1 is another track's.
        let takes = [
            take(4, 0, 0, 0),
            take(1, 1, 0, 0),
            take(9, 0, 1, 1),
            take(2, 0, 1, 0),
        ];
        let map = TempoMap::constant(120.0).unwrap();
        let b = audio_blocks(
            0,
            &[
                region(0, 960, 4),
                region(960, 1920, 2),
                region(1920, 3840, 9),
            ],
            &takes,
            &map,
            48_000,
        );
        let tags: Vec<&str> = b.iter().map(|b| b.tag.as_str()).collect();
        assert_eq!(tags, ["take 1", "take 2", ""]);
        // A beat at 120 BPM and 48 kHz is 24 000 frames.
        assert_eq!(b[0].frames, 24_000.0);
        assert_eq!(b[2].frames, 48_000.0);
        assert_eq!(
            (b[1].src, b[1].fade_in, b[1].fade_out, b[1].into, b[1].left),
            (10, 5, 7, 3, 0)
        );
    }

    #[test]
    fn waves_compare_by_take_and_length() {
        let w = |id: u16, frames: usize| TakeWave {
            id,
            frames,
            channels: 1,
            frames_per_column: 100,
            columns: vec![(0.0, 0.0); frames / 100],
        };
        assert!(same_waves(&[w(1, 1000)], &[w(1, 1000)]));
        assert!(
            !same_waves(&[w(1, 1000)], &[w(1, 1200)]),
            "a recording grew"
        );
        assert!(!same_waves(&[w(1, 1000)], &[w(2, 1000)]));
        assert!(!same_waves(&[w(1, 1000)], &[]));
    }
}
