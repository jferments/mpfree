//! The Song view of the Arrange page (roadmap T13): the arrangement's
//! placements on a timeline, a lane a track, with the markers on a ruler
//! and the playhead, and under them the audio tracks' lanes (layout spec
//! 4c). Like the other widgets it only shows what the app put in its
//! fields and reports touches:
//!
//! - a placement: `Select(index)` (its index in the arrangement); empty
//!   space: `Select(usize::MAX)`, nothing selected;
//! - the buttons along the bottom: `Tab(k)` for [`SONG_BUTTONS`]`[k]`, or
//!   for [`AUDIO_BUTTONS`]`[k]` while an audio region is picked;
//! - Rec song, at the right of the title line: `Tab(REC)`;
//! - an audio region: `Cell { row: audio track, col: region index }`;
//! - an audio lane's header: its arm dot `Toggle(audio track)`, its name
//!   or input chip `Cell { row: audio track, col: PANEL }` (on the lift,
//!   unless the finger dragged to scroll);
//! - "+ Audio", the last lane: `Tab(ADD_AUDIO)`;
//! - the track panel, while one is open ([`SongLanes::panel`]): Close
//!   `Tab(PANEL_CLOSE)`, Arm `Toggle(audio track)`, Mute, Solo and Takes
//!   `Tab(PANEL_MUTE)` and so on, an input `Tab(PANEL_INPUT + k)` for
//!   [`INPUTS`]`[k]`, a monitor mode `Tab(PANEL_MONITOR + k)` for
//!   [`MONITORS`]`[k]`, and the level's and pan's steps `Step { index:
//!   PANEL_LEVEL or PANEL_PAN, delta: -1 or 1 }`.
//!
//! A drag up or down the name column scrolls the lanes when there are more
//! than fit ([`MIN_LANE`] pixels each at least). The scroll is the
//! widget's own: [`SongLanes::keep_scroll`] carries it over when the app
//! shows new contents.
//!
//! **Audio lanes.** One for each audio track the app shows (one with a
//! region or armed), in the order given. Regions are blocks in the track's
//! colour with the take's waveform (from [`SongLanes::waves`], the app's
//! `AudioHost::wave` reads), a "take N" tag when an older take plays, and
//! fades as slopes. While a track records, a red block grows from its
//! punch in to the playhead. The header has the name, an arm dot, the
//! input chip and a 3 px level bar along its bottom edge.
//!
//! **The track panel** (layout spec 4c) opens over the lanes, right of the
//! names, for the track in [`SongLanes::panel`]: Arm, Mute, Solo and
//! Takes, the input (L, R, L+R, ST), the monitor mode (Auto, In, Off), and
//! the level and pan with a step either way (Q1 and Q2 turn them too). It
//! covers the top lanes, so the audio lanes under the MIDI ones stay in
//! view; touches on it go to it alone.

use crate::widgets::{rec_song_pill, REC_SONG_W};
use mpfree_audiotracks::host::TakeWave;
use mpfree_audiotracks::{frames_from, Input, Monitor, Region, TakeInfo};
use mpfree_transport::time::{TempoMap, Tick};
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Touch, Widget};

/// The buttons along the bottom, left to right.
pub const SONG_BUTTONS: [&str; 6] = [
    "Clips",
    "Play song",
    "Scenes → song",
    "Song → scenes",
    "Make unique",
    "Delete",
];
/// The buttons along the bottom while an audio region is picked (layout
/// spec 4c). SHIFT + Undo redoes.
pub const AUDIO_BUTTONS: [&str; 6] = ["Split", "Erase", "Fades", "Gain", "Takes", "Undo"];
/// What Select reports for a tap on empty space.
pub const NONE: usize = usize::MAX;
/// What Tab reports for Rec song.
pub const REC: usize = SONG_BUTTONS.len();
/// What Tab reports for "+ Audio".
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
/// Audio tracks at most (`mpfree_audiotracks::MAX_TRACKS`).
pub const MAX_AUDIO: usize = mpfree_audiotracks::MAX_TRACKS;
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
    /// Its fades, in frames.
    pub fade_in: u32,
    pub fade_out: u32,
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
    /// The audio track, 0 to [`MAX_AUDIO`] - 1.
    pub track: usize,
    pub name: String,
    pub color: Color,
    pub armed: bool,
    /// What it records from: the input chip, "L", "R", "L+R" or "ST".
    pub input: Input,
    /// The level bar along the header's bottom edge, 0 to 1.
    pub level: f32,
    /// The track's level, 0 to 2 (1 is as recorded), and its balance, -1
    /// (left) to 1 (right): what Q1 and Q2 turn with a region picked.
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
    pub blocks: Vec<SongBlock>,
    pub markers: Vec<(Tick, String)>,
    /// Ticks a bar.
    pub bar: Tick,
    /// The first bar shown (0-based) and how many.
    pub first_bar: i64,
    pub bars: i64,
    pub playhead: Option<Tick>,
    pub selected: Option<usize>,
    /// Song mode: the song plays.
    pub on: bool,
    /// Live to song armed.
    pub rec: bool,
    /// The audio lanes, under the MIDI lanes.
    pub audio: Vec<AudioLane>,
    /// The waveforms read so far, a take each.
    pub waves: Vec<TakeWave>,
    /// The picked audio region: (audio track, region index).
    pub region: Option<(usize, usize)>,
    /// The audio track whose panel is open over the lanes.
    pub panel: Option<usize>,
    /// The audio track whose take lanes show under its lane.
    pub takes_open: Option<usize>,
    /// A finger swiping across a take lane: the take, where it went down
    /// and where it is now (x).
    pub(crate) swipe: Option<(u16, i32, i32)>,
    /// The first lane shown (MIDI lanes, then audio lanes, then "+ Audio").
    pub first_lane: usize,
    /// A finger on the name column: where it went down, the first lane
    /// then, and whether it has dragged.
    pub(crate) drag: Option<(i32, usize, bool)>,
}

const NAME_W: i32 = 96;
const RULER_H: i32 = 26;
const TOOLS_H: i32 = 48;
const LANE_MAX: i32 = 56;
const GAP: i32 = 6;
/// Pixels a finger moves on the name column before it scrolls.
const DRAG_SLOP: i32 = 8;
/// A take lane's height.
const TAKE_H: i32 = 28;
/// The track panel's size at most, its rows' height and its labels'
/// width.
const PANEL_W: i32 = 560;
const PANEL_H: i32 = 176;
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
    /// A button for each of [`INPUTS`] and [`MONITORS`].
    pub inputs: [Rect; 4],
    pub monitors: [Rect; 3],
    /// The level's and the pan's step down, value and step up.
    pub level: [Rect; 3],
    pub pan: [Rect; 3],
}

impl Default for SongLanes {
    fn default() -> SongLanes {
        SongLanes {
            title: "Song".to_string(),
            legend: String::new(),
            alert: false,
            tracks: Vec::new(),
            blocks: Vec::new(),
            markers: Vec::new(),
            bar: 3840,
            first_bar: 0,
            bars: 16,
            playhead: None,
            selected: None,
            on: false,
            rec: false,
            audio: Vec::new(),
            waves: Vec::new(),
            region: None,
            panel: None,
            takes_open: None,
            swipe: None,
            first_lane: 0,
            drag: None,
        }
    }
}

struct Geo {
    ruler: Rect,
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
    Add,
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
    }

    /// A copy with no waveforms (they can be large).
    pub fn clone_without_waves(&self) -> SongLanes {
        let waves = Vec::new();
        SongLanes {
            title: self.title.clone(),
            legend: self.legend.clone(),
            alert: self.alert,
            tracks: self.tracks.clone(),
            blocks: self.blocks.clone(),
            markers: self.markers.clone(),
            bar: self.bar,
            first_bar: self.first_bar,
            bars: self.bars,
            playhead: self.playhead,
            selected: self.selected,
            on: self.on,
            rec: self.rec,
            audio: self.audio.clone(),
            waves,
            region: self.region,
            panel: self.panel,
            takes_open: self.takes_open,
            swipe: self.swipe,
            first_lane: self.first_lane,
            drag: self.drag,
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

    /// Lanes in all: the MIDI lanes, the audio lanes (with the open
    /// track's take lanes under it), then "+ Audio" while there is room
    /// for another audio track.
    fn lane_count(&self) -> usize {
        self.tracks.len()
            + self.audio.len()
            + self.take_count()
            + usize::from(self.audio.len() < MAX_AUDIO)
    }

    fn lane(&self, k: usize) -> Option<Lane> {
        let m = self.tracks.len();
        if k < m {
            return Some(Lane::Midi(k));
        }
        let open = self.open_takes();
        let mut j = k - m;
        for a in 0..self.audio.len() {
            if j == 0 {
                return Some(Lane::Audio(a));
            }
            j -= 1;
            if let Some((oa, n)) = open.filter(|o| o.0 == a) {
                if j < n {
                    return Some(Lane::Take(oa, j));
                }
                j -= n;
            }
        }
        (j == 0 && self.audio.len() < MAX_AUDIO).then_some(Lane::Add)
    }

    /// The lane number of audio lane `a` (an index into `audio`).
    fn audio_k(&self, a: usize) -> usize {
        let before = self.open_takes().filter(|o| o.0 < a).map_or(0, |o| o.1);
        self.tracks.len() + a + before
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
        let (ruler, lanes) = rest.take_top(RULER_H);
        let lanes = Rect::new(lanes.x, lanes.y + GAP, lanes.w, lanes.h - GAP);
        // Take lanes keep their height; the others share what is left.
        let takes = self.take_count() as i32;
        let n = (self.lane_count() as i32 - takes).max(1);
        let room = lanes.h - takes * TAKE_H;
        let lane_h = (room / n).clamp(MIN_LANE.min(lanes.h.max(8)), LANE_MAX);
        Geo {
            ruler,
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
        self.span_rect(&self.geo(r), b.track, b.start, b.start + b.length)
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

    /// "+ Audio", if it is in view.
    pub fn add_rect(&self, r: Rect) -> Option<Rect> {
        if self.audio.len() >= MAX_AUDIO {
            return None;
        }
        let g = self.geo(r);
        let row = self.lane_rect(&g, self.lane_count() - 1)?;
        let h = (row.h - 8).min(30);
        Some(Rect::new(row.x, row.y + (row.h - h) / 2, NAME_W - 4, h))
    }

    /// Where Rec song is: the right end of the title line.
    pub fn rec_rect(&self, r: Rect) -> Rect {
        let head = r.inset(14).take_top(26).0;
        Rect::new(head.right() - REC_SONG_W, head.y - 4, REC_SONG_W, 30)
    }

    /// The buttons along the bottom (the audio region's while one is
    /// picked).
    pub fn button_rects(&self, r: Rect) -> [Rect; 6] {
        let g = self.geo(r);
        let widths = if self.region.is_some() {
            [84, 84, 84, 84, 84, 84]
        } else {
            [84, 124, 140, 140, 124, 96]
        };
        let mut out = [Rect::new(0, 0, 0, 0); 6];
        let mut x = g.tools.x;
        for (k, w) in widths.iter().enumerate() {
            out[k] = Rect::new(x, g.tools.y, *w, g.tools.h);
            x += w + 8;
        }
        out
    }

    /// The track panel's parts, while one is open: over the lanes, right
    /// of the names, from the top.
    pub fn panel_parts(&self, r: Rect) -> Option<PanelParts> {
        let track = self
            .panel
            .filter(|&t| self.audio.iter().any(|l| l.track == t))?;
        let g = self.geo(r);
        let w = (g.lanes.w - NAME_W).min(PANEL_W);
        let h = (g.lanes.h - 4).min(PANEL_H);
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
        let row = |k: i32| inner.y + 32 + k * (PANEL_ROW + 6);
        let at = |x: i32, k: i32, w: i32| Rect::new(x, row(k), w, PANEL_ROW);
        let close = Rect::new(inner.right() - 72, inner.y - 2, 72, 28);
        let x1 = inner.x + PANEL_LABEL;
        let arm = at(inner.x, 0, 104);
        let mute = at(arm.right() + 8, 0, 72);
        let solo = at(mute.right() + 8, 0, 72);
        let takes = at(inner.right() - 90, 0, 90);
        let inputs: [Rect; 4] = std::array::from_fn(|k| at(x1 + k as i32 * 52, 1, 48));
        let mx = inputs[3].right() + 20 + PANEL_LABEL + 14;
        let monitors = std::array::from_fn(|k| at(mx + k as i32 * 60, 1, 56));
        let level = [at(x1, 2, 36), at(x1 + 40, 2, 84), at(x1 + 128, 2, 36)];
        let px = level[2].right() + 30 + 20 + 40;
        let pan = [at(px, 2, 36), at(px + 40, 2, 64), at(px + 108, 2, 36)];
        Some(PanelParts {
            track,
            panel,
            close,
            arm,
            mute,
            solo,
            takes,
            inputs,
            monitors,
            level,
            pan,
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
        cx.canvas.fill_round_rect(shadow, 12, t.bg);
        let old = cx.canvas.clip();
        cx.canvas.set_clip(old.intersect(p.panel));
        cx.canvas
            .fill_round_rect(p.panel, 12, t.surface.mix(t.raised, 0.35));
        cx.canvas.stroke_round_rect(p.panel, 12, 1, t.outline);
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
            lane.color.mix(t.text, 0.25),
            &format!("{}{state}", lane.name),
        );
        let button = |cx: &mut PaintCx, b: Rect, word: &str, on: bool, tone: Color| {
            cx.canvas
                .fill_round_rect(b, 8, if on { tone } else { t.raised });
            let ink = if on { t.bg } else { t.text };
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
        for (k, b) in p.inputs.iter().enumerate() {
            button(cx, *b, INPUTS[k].0, lane.input == INPUTS[k].1, t.accent);
        }
        let mon = p.monitors[0];
        label(
            cx,
            Rect::new(mon.x - PANEL_LABEL - 14, mon.y, PANEL_LABEL + 12, mon.h),
            "Monitor",
        );
        for (k, b) in p.monitors.iter().enumerate() {
            button(
                cx,
                *b,
                MONITORS[k].0,
                lane.monitor == MONITORS[k].1,
                t.accent,
            );
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
        for (parts, value, q) in [(&p.level, level, "Q1"), (&p.pan, pan, "Q2")] {
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
            cx.text(
                FontId::Bold,
                t.size_small - 3.0,
                Rect::new(parts[2].right() + 6, parts[2].y, 28, parts[2].h),
                Align::Left,
                t.text_faint,
                q,
            );
        }
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
        } else if let Some(k) = p.monitors.iter().position(hit) {
            tab(PANEL_MONITOR + k)
        } else if hit(&p.level[0]) {
            step(PANEL_LEVEL, -1)
        } else if hit(&p.level[2]) {
            step(PANEL_LEVEL, 1)
        } else if hit(&p.pan[0]) {
            step(PANEL_PAN, -1)
        } else if hit(&p.pan[2]) {
            step(PANEL_PAN, 1)
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
    /// x0) * per_px` on.
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
            let top = mid - (f64::from(hi.clamp(0.0, 1.0)) * half).round() as i32;
            let bottom = mid - (f64::from(lo.clamp(-1.0, 0.0)) * half).round() as i32;
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
            self.paint_wave(cx, take.take, br.inset_xy(0, 1), br, x0, 0.0, per_px, ink);
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
            .fill_round_rect(br, 6, t.raised.mix(lane.color, 0.42));
        // The block's full stretch on screen, unclipped, so the waveform
        // and the fades line up when it runs off an edge.
        let x0 = self.x_of(g.lanes, b.start);
        let x1 = self.x_of(g.lanes, b.end).max(x0 + 1);
        let per_px = b.frames / f64::from(x1 - x0);
        // The waveform fills the block; the name sits over its top.
        let label_h = if br.h >= 26 { 14 } else { 0 };
        let wave = br.inset_xy(0, 2);
        let ink = lane.color.mix(t.text, 0.6);
        self.paint_wave(cx, b.take, wave, br, x0, b.src as f64, per_px, ink);
        // Fades: the part above each slope is dimmed.
        let shade = t.surface.mix(lane.color, 0.15);
        for (frames, rising) in [(b.fade_in, true), (b.fade_out, false)] {
            let px = (f64::from(frames) / per_px.max(1e-9)).round() as i32;
            if px < 2 {
                continue;
            }
            for k in 0..px {
                let x = if rising { x0 + k } else { x1 - 1 - k };
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
            cx.canvas.stroke_round_rect(br, 6, 2, t.accent);
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
        let word = input_chip(lane.input);
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
                .stroke_round_rect(h.inset_xy(-2, 1), 6, 1, t.accent);
        }
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            Rect::new(h.x + 20, h.y, (chip.x - h.x - 24).max(0), h.h),
            Align::Left,
            lane.color.mix(t.text, 0.25),
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
        cx.titled_panel(r, &self.title, "");
        // The legend, then Rec song.
        let rec = self.rec_rect(r);
        let head = r.inset(14).take_top(26).0;
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(head.x, head.y, rec.x - head.x - 12, head.h),
            Align::Right,
            if self.alert { t.warn } else { t.text_dim },
            &self.legend,
        );
        rec_song_pill(cx, rec, self.rec);
        let g = self.geo(r);
        // The ruler: bar numbers, then the markers' names over them.
        let step = if self.bars > 16 { 4 } else { 1 };
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
        for (at, name) in &self.markers {
            let x = self.x_of(g.ruler, *at);
            if x < g.ruler.x + NAME_W || x >= g.ruler.right() {
                continue;
            }
            cx.canvas.fill_rect(
                Rect::new(x, g.ruler.y + 14, 2, g.lanes.bottom() - g.ruler.y - 14),
                t.warn,
            );
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                Rect::new(x + 4, g.ruler.y + 12, 120, 14),
                Align::Left,
                t.warn,
                name,
            );
        }
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
        // A thin line between the MIDI lanes and the audio lanes.
        if let Some(row) = self.lane_rect(&g, self.tracks.len()) {
            if self.tracks.len() > self.first(&g) {
                cx.canvas
                    .fill_rect(Rect::new(row.x, row.y, row.w, 1), t.outline);
            }
        }
        for k in 0..n {
            let Some(row) = self.lane_rect(&g, k) else {
                continue;
            };
            match self.lane(k) {
                Some(Lane::Midi(i)) => {
                    let (name, color) = &self.tracks[i];
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        Rect::new(row.x, row.y, NAME_W - 8, row.h),
                        Align::Left,
                        color.mix(t.text, 0.25),
                        name,
                    );
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
                            lane.color.mix(t.text, 0.4)
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
                Some(Lane::Add) => {
                    if let Some(b) = self.add_rect(r) {
                        cx.canvas.stroke_round_rect(b, 8, 1, t.outline);
                        cx.text(
                            FontId::Bold,
                            t.size_small - 1.0,
                            b,
                            Align::Center,
                            t.text_dim,
                            "+ Audio",
                        );
                    }
                }
                None => {}
            }
        }
        for b in &self.blocks {
            let Some(br) = self.block_rect(r, b) else {
                continue;
            };
            let color = self.tracks.get(b.track).map_or(t.accent, |c| c.1);
            cx.canvas.fill_round_rect(br, 6, t.raised.mix(color, 0.42));
            if b.unique {
                // Its own copy: a corner mark.
                cx.canvas
                    .fill_round_rect(Rect::new(br.right() - 10, br.y + 4, 6, 6), 3, t.text);
            }
            if self.selected == Some(b.index) {
                cx.canvas.stroke_round_rect(br, 6, 2, t.accent);
            }
            if br.w > 30 {
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    br.inset_xy(6, 0),
                    Align::Left,
                    t.text,
                    &b.name,
                );
            }
        }
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
            // Recording: red from the punch in to the playhead.
            if let (Some(from), Some(now)) = (lane.recording, self.playhead) {
                let lane_k = self.audio_k(a);
                if let Some(br) = self.span_rect(&g, lane_k, from, now.max(from + 1)) {
                    cx.canvas.fill_round_rect(br, 6, t.bad.mix(t.surface, 0.35));
                    cx.canvas.stroke_round_rect(br, 6, 1, t.bad);
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
        if let Some(p) = self.playhead {
            let x = self.x_of(g.lanes, p);
            if x >= g.lanes.x + NAME_W && x < g.lanes.right() {
                let c = if self.on { t.good } else { t.text_faint };
                cx.canvas
                    .fill_rect(Rect::new(x, g.ruler.y, 2, g.lanes.bottom() - g.ruler.y), c);
            }
        }
        if let Some(p) = self.panel_parts(r) {
            self.paint_panel(cx, &p);
        }
        let audio = self.region.is_some();
        let selected = self.selected.is_some();
        for (k, br) in self.button_rects(r).iter().enumerate() {
            let lit = k == 1 && self.on && !audio;
            let live = audio || !(k >= 4 && !selected);
            let fill = if lit {
                t.good
            } else if live {
                t.raised
            } else {
                t.surface
            };
            cx.canvas.fill_round_rect(*br, 10, fill);
            let ink = if lit {
                t.bg
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
            } else {
                SONG_BUTTONS[k]
            };
            cx.text(FontId::Bold, t.size_small, *br, Align::Center, ink, word);
        }
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = self.geo(r);
        let names = Rect::new(g.lanes.x - 12, g.lanes.y, NAME_W + 12, g.lanes.h);
        match phase {
            Touch::Down => {}
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
                return None;
            }
        }
        if let Some(k) = self.button_rects(r).iter().position(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(k));
        }
        if self.rec_rect(r).contains(x, y) {
            return Some(EventKind::Tab(REC));
        }
        if self.add_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(ADD_AUDIO));
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
        if let Some(b) = self
            .blocks
            .iter()
            .find(|b| self.block_rect(r, b).is_some_and(|br| br.contains(x, y)))
        {
            return Some(EventKind::Select(b.index));
        }
        for (a, lane) in self.audio.iter().enumerate() {
            if let Some(i) = (0..lane.blocks.len()).find(|&i| {
                self.region_rect(r, a, i)
                    .is_some_and(|br| br.contains(x, y))
            }) {
                return Some(EventKind::Cell {
                    row: lane.track,
                    col: i,
                });
            }
        }
        g.lanes.contains(x, y).then_some(EventKind::Select(NONE))
    }
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
            blocks: vec![
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
            ],
            ..SongLanes::default()
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
        assert_eq!(
            s.touch(1, Touch::Down, b.x + 3, b.y + 3, r),
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
        assert_eq!(
            s.touch(1, Touch::Down, empty.x - 20, empty.y + 3, r),
            Some(EventKind::Select(NONE))
        );
        for (k, br) in s.button_rects(r).iter().enumerate() {
            assert!(br.right() <= r.right() - 14, "{k}: {br:?}");
            assert_eq!(
                s.touch(1, Touch::Down, br.x + 3, br.y + 3, r),
                Some(EventKind::Tab(k))
            );
        }
        let rec = s.rec_rect(r);
        assert!(rec.right() <= r.right() - 14 && rec.y >= r.y, "{rec:?}");
        assert_eq!(
            s.touch(1, Touch::Down, rec.x + 3, rec.y + 3, r),
            Some(EventKind::Tab(REC))
        );
    }

    fn audio_lane(track: usize, name: &str, blocks: Vec<AudioBlock>) -> AudioLane {
        AudioLane {
            track,
            name: name.into(),
            color: Color::rgb(0x2EC4B6),
            armed: false,
            input: Input::Left,
            level: 0.5,
            volume: 1.0,
            pan: 0.0,
            monitor: Monitor::Auto,
            mute: false,
            solo: false,
            blocks,
            takes: Vec::new(),
            recording: None,
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
        }
    }

    #[test]
    fn audio_lanes_sit_under_the_midi_lanes() {
        let mut s = lanes();
        s.audio = vec![
            audio_lane(0, "Vocal", vec![block(0, 4 * 3840, 0)]),
            audio_lane(3, "Guitar", vec![block(3840, 2 * 3840, 1)]),
        ];
        let r = Rect::new(12, 140, 814, 534);
        let midi = s.block_rect(r, &s.blocks[0].clone()).unwrap();
        let vocal = s.region_rect(r, 0, 0).unwrap();
        let guitar = s.region_rect(r, 1, 0).unwrap();
        // Same ruler: the vocal starts where the drums' bar 1 does, and
        // the lanes go down in order with the same height.
        assert_eq!(vocal.x, midi.x);
        assert!(vocal.y > midi.y && guitar.y > vocal.y);
        // Two MIDI lanes (Drums, Bass), then Vocal and Guitar.
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
        // "+ Audio" is the last lane.
        let add = s.add_rect(r).unwrap();
        assert!(add.y > guitar.y);
        assert_eq!(
            s.touch(5, Touch::Down, add.x + 4, add.y + 4, r),
            Some(EventKind::Tab(ADD_AUDIO))
        );
        // Eight audio tracks: no "+ Audio".
        s.audio = (0..MAX_AUDIO)
            .map(|k| audio_lane(k, "A", Vec::new()))
            .collect();
        assert!(s.add_rect(r).is_none());
    }

    #[test]
    fn a_picked_region_swaps_the_bottom_row() {
        let mut s = lanes();
        s.audio = vec![audio_lane(0, "Vocal", vec![block(0, 3840, 0)])];
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
        s.audio = vec![audio_lane(1, "Vocal", vec![block(0, 3840, 0)])];
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
            .chain(p.pan);
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
        let lane = s.lane_rect(&g, s.tracks.len()).unwrap();
        assert!(lane.bottom() <= p.panel.y, "{lane:?} over {:?}", p.panel);
        let mut low = s.clone();
        low.tracks = (0..5)
            .map(|k| (format!("T{k}"), Color::rgb(0x6EA8FE)))
            .collect();
        let lp = low.panel_parts(r).unwrap();
        let lane = low.lane_rect(&low.geo(r), low.tracks.len()).unwrap();
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
        // Its empty space and its value read-outs take the touch and ask
        // for nothing, even with a block under them.
        assert_eq!(tap(p.level[1]), None);
        low.blocks.push(SongBlock {
            index: 2,
            track: 3,
            start: 0,
            length: 4 * 3840,
            name: "Under".into(),
            unique: false,
        });
        let under = low.block_rect(r, &low.blocks[2].clone()).unwrap();
        assert!(lp.panel.contains(under.x + 3, under.y + 3));
        assert_eq!(low.touch(1, Touch::Down, under.x + 3, under.y + 3, r), None);
        low.panel = None;
        assert_eq!(
            low.touch(1, Touch::Down, under.x + 3, under.y + 3, r),
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
        s.audio = vec![vocal, audio_lane(1, "Guitar", Vec::new())];
        let r = Rect::new(12, 140, 814, 534);
        assert_eq!(s.lane_count(), 5, "closed: no take lanes");
        s.takes_open = Some(0);
        let g = s.geo(r);
        // Drums, Bass, Vocal, take 2, take 1, Guitar, + Audio.
        assert_eq!(s.lane_count(), 7);
        assert_eq!(s.lane(2), Some(Lane::Audio(0)));
        assert_eq!(s.lane(3), Some(Lane::Take(0, 0)));
        assert_eq!(s.lane(4), Some(Lane::Take(0, 1)));
        assert_eq!(s.lane(5), Some(Lane::Audio(1)));
        assert_eq!(s.lane(6), Some(Lane::Add));
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

    #[test]
    fn take_lanes_name_takes_oldest_first_and_list_them_newest_first() {
        let map = TempoMap::constant(120.0).unwrap();
        let info = |id: u16, track: usize, group: u16, pass: u16| TakeInfo {
            id,
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
            gain: 1.0,
            fade_in: 0,
            fade_out: 0,
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

    #[test]
    fn many_lanes_scroll_with_a_drag_on_the_names() {
        let mut s = lanes();
        s.tracks = (0..8)
            .map(|k| (format!("T{k}"), Color::rgb(0xE8A33D)))
            .collect();
        s.audio = (0..6)
            .map(|k| audio_lane(k, "A", vec![block(0, 3840, 0)]))
            .collect();
        let r = Rect::new(12, 140, 814, 534);
        let g = s.geo(r);
        // 8 + 6 + "+ Audio" = 15 lanes at 32 px do not fit.
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
            gain: 1.0,
            fade_in: 5,
            fade_out: 7,
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
        assert_eq!((b[1].src, b[1].fade_in, b[1].fade_out), (10, 5, 7));
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
