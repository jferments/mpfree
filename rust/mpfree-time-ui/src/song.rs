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
//! - "+ Audio", the last lane: `Tab(ADD_AUDIO)`.
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

use crate::widgets::{rec_song_pill, REC_SONG_W};
use mpfree_audiotracks::host::TakeWave;
use mpfree_audiotracks::{frames_from, Region, TakeInfo};
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

/// An audio track's lane.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioLane {
    /// The audio track, 0 to [`MAX_AUDIO`] - 1.
    pub track: usize,
    pub name: String,
    pub color: Color,
    pub armed: bool,
    /// The input chip: "L", "R", "L+R" or "ST".
    pub input: String,
    /// The level bar along the header's bottom edge, 0 to 1.
    pub level: f32,
    /// The track's level, 0 to 2 (1 is as recorded), and its balance, -1
    /// (left) to 1 (right): what Q1 and Q2 turn with a region picked.
    pub volume: f32,
    pub pan: f32,
    /// Its regions, in order (`AudioTracks::regions`).
    pub blocks: Vec<AudioBlock>,
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
            first_lane: 0,
            drag: None,
        }
    }
}

struct Geo {
    ruler: Rect,
    lanes: Rect,
    lane_h: i32,
    /// Lanes in view at once.
    visible: usize,
    tools: Rect,
}

/// What a lane is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lane {
    Midi(usize),
    /// An index into `audio`.
    Audio(usize),
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
            first_lane: self.first_lane,
            drag: self.drag,
        }
    }

    /// Lanes in all: the MIDI lanes, the audio lanes, then "+ Audio" while
    /// there is room for another audio track.
    fn lane_count(&self) -> usize {
        self.tracks.len() + self.audio.len() + usize::from(self.audio.len() < MAX_AUDIO)
    }

    fn lane(&self, k: usize) -> Option<Lane> {
        let m = self.tracks.len();
        if k < m {
            Some(Lane::Midi(k))
        } else if k < m + self.audio.len() {
            Some(Lane::Audio(k - m))
        } else if k < self.lane_count() {
            Some(Lane::Add)
        } else {
            None
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
        let n = self.lane_count().max(1) as i32;
        let lane_h = (lanes.h / n).clamp(MIN_LANE.min(lanes.h.max(8)), LANE_MAX);
        let visible = (lanes.h / lane_h.max(1)).max(1) as usize;
        Geo {
            ruler,
            lanes,
            lane_h,
            visible,
            tools,
        }
    }

    /// The first lane shown, kept in range for `g`.
    fn first(&self, g: &Geo) -> usize {
        self.first_lane
            .min(self.lane_count().saturating_sub(g.visible))
    }

    /// Lane `k`'s rectangle (names included), if it is in view.
    fn lane_rect(&self, g: &Geo, k: usize) -> Option<Rect> {
        let first = self.first(g);
        if k < first || k >= first + g.visible || k >= self.lane_count() {
            return None;
        }
        let y = g.lanes.y + (k - first) as i32 * g.lane_h;
        Some(Rect::new(g.lanes.x, y, g.lanes.w, g.lane_h))
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
        self.span_rect(&self.geo(r), self.tracks.len() + a, b.start, b.end)
    }

    /// Audio lane `a`'s header (its part of the name column), if in view.
    pub fn header_rect(&self, r: Rect, a: usize) -> Option<Rect> {
        if a >= self.audio.len() {
            return None;
        }
        let row = self.lane_rect(&self.geo(r), self.tracks.len() + a)?;
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

    fn wave_of(&self, take: u16) -> Option<&TakeWave> {
        self.waves
            .iter()
            .find(|w| w.id == take && w.frames_per_column > 0)
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
        if let Some(w) = self.wave_of(b.take) {
            let mid = wave.y + wave.h / 2;
            let half = f64::from((wave.h - 4) / 2);
            for x in br.x..br.right() {
                let f0 = b.src as f64 + f64::from(x - x0) * per_px;
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
        let chip_w = cx.measure(FontId::Bold, t.size_small - 3.0, &lane.input) + 10;
        let chip = Rect::new(h.right() - chip_w, h.y + h.h / 2 - 9, chip_w, 18);
        if !lane.input.is_empty() {
            cx.canvas.fill_round_rect(chip, 5, t.raised);
            cx.text(
                FontId::Bold,
                t.size_small - 3.0,
                chip,
                Align::Center,
                t.text_dim,
                &lane.input,
            );
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
        if n > g.visible {
            let first = self.first(&g);
            let track_h = g.lanes.h;
            let h = (track_h * g.visible as i32 / n as i32).max(16);
            let y = g.lanes.y + (track_h - h) * first as i32 / (n - g.visible) as i32;
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
                let lane_k = self.tracks.len() + a;
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
            Touch::Move => {
                let (y0, first, moved) = self.drag?;
                if !moved && (y - y0).abs() < DRAG_SLOP {
                    return None;
                }
                let rows = (y0 - y) / g.lane_h.max(1);
                let last = self.lane_count().saturating_sub(g.visible) as i32;
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
            input: "L".into(),
            level: 0.5,
            volume: 1.0,
            pan: 0.0,
            blocks,
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
        assert!(g.visible < 15, "{}", g.visible);
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
        assert_eq!(s.first_lane, 15 - g.visible);
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
