//! The Song view of the Arrange page (roadmap T13): the arrangement's
//! placements on a timeline, a lane a track, with the markers on a ruler
//! and the playhead. Like the other widgets it only shows what the app put
//! in its fields and reports touches:
//!
//! - a placement: `Select(index)` (its index in the arrangement); empty
//!   space: `Select(usize::MAX)`, nothing selected;
//! - the buttons along the bottom: `Tab(k)` for [`SONG_BUTTONS`]`[k]`.

use mpfree_transport::time::Tick;
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
/// What Select reports for a tap on empty space.
pub const NONE: usize = usize::MAX;

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

/// The Song view: see the module notes.
#[derive(Clone, Debug, PartialEq)]
pub struct SongLanes {
    pub title: String,
    pub legend: String,
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
}

const NAME_W: i32 = 96;
const RULER_H: i32 = 26;
const TOOLS_H: i32 = 48;
const LANE_MAX: i32 = 56;
const GAP: i32 = 6;

impl Default for SongLanes {
    fn default() -> SongLanes {
        SongLanes {
            title: "Song".to_string(),
            legend: String::new(),
            tracks: Vec::new(),
            blocks: Vec::new(),
            markers: Vec::new(),
            bar: 3840,
            first_bar: 0,
            bars: 16,
            playhead: None,
            selected: None,
            on: false,
        }
    }
}

struct Geo {
    ruler: Rect,
    lanes: Rect,
    lane_h: i32,
    tools: Rect,
}

impl SongLanes {
    pub fn new() -> SongLanes {
        SongLanes::default()
    }

    fn geo(&self, r: Rect) -> Geo {
        // Matches PaintCx::titled_panel with the default theme's padding.
        let body = r.inset(14).take_top(34).1;
        let (rest, tools) = (
            Rect::new(body.x, body.y, body.w, body.h - TOOLS_H - GAP),
            Rect::new(body.x, body.bottom() - TOOLS_H, body.w, TOOLS_H),
        );
        let (ruler, lanes) = rest.take_top(RULER_H);
        let n = self.tracks.len().max(1) as i32;
        let lane_h = ((lanes.h - GAP) / n).clamp(8, LANE_MAX);
        Geo {
            ruler,
            lanes: Rect::new(lanes.x, lanes.y + GAP, lanes.w, lanes.h - GAP),
            lane_h,
            tools,
        }
    }

    /// The x of song tick `t` in `area` (the lanes past the names).
    fn x_of(&self, area: Rect, t: Tick) -> i32 {
        let span = (self.bars.max(1) * self.bar.max(1)) as f64;
        let from = (self.first_bar * self.bar) as f64;
        area.x + NAME_W + ((t as f64 - from) / span * f64::from(area.w - NAME_W)).round() as i32
    }

    /// Where placement `b` is drawn, clipped to the view, if it shows.
    pub fn block_rect(&self, r: Rect, b: &SongBlock) -> Option<Rect> {
        let g = self.geo(r);
        let row = i32::try_from(b.track).ok()?;
        if b.track >= self.tracks.len() {
            return None;
        }
        let x0 = self.x_of(g.lanes, b.start).max(g.lanes.x + NAME_W);
        let x1 = self.x_of(g.lanes, b.start + b.length).min(g.lanes.right());
        let y = g.lanes.y + row * g.lane_h;
        (x1 - x0 >= 2 && y + g.lane_h <= g.lanes.bottom() + 1)
            .then(|| Rect::new(x0, y + 2, x1 - x0 - 2, g.lane_h - 4))
    }

    /// The buttons along the bottom.
    pub fn button_rects(&self, r: Rect) -> [Rect; 6] {
        let g = self.geo(r);
        let widths = [84, 124, 140, 140, 124, 96];
        let mut out = [Rect::new(0, 0, 0, 0); 6];
        let mut x = g.tools.x;
        for (k, w) in widths.iter().enumerate() {
            out[k] = Rect::new(x, g.tools.y, *w, g.tools.h);
            x += w + 8;
        }
        out
    }
}

impl Widget for SongLanes {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.legend);
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
        for (k, (name, color)) in self.tracks.iter().enumerate() {
            let y = g.lanes.y + k as i32 * g.lane_h;
            if y + g.lane_h > g.lanes.bottom() + 1 {
                break;
            }
            cx.text(
                FontId::Bold,
                t.size_small,
                Rect::new(g.lanes.x, y, NAME_W - 8, g.lane_h),
                Align::Left,
                color.mix(t.text, 0.25),
                name,
            );
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
        if let Some(p) = self.playhead {
            let x = self.x_of(g.lanes, p);
            if x >= g.lanes.x + NAME_W && x < g.lanes.right() {
                let c = if self.on { t.good } else { t.text_faint };
                cx.canvas
                    .fill_rect(Rect::new(x, g.ruler.y, 2, g.lanes.bottom() - g.ruler.y), c);
            }
        }
        let selected = self.selected.is_some();
        for (k, br) in self.button_rects(r).iter().enumerate() {
            let lit = k == 1 && self.on;
            let live = !(k >= 4 && !selected);
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
            let word = if k == 1 {
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
        if phase != Touch::Down {
            return None;
        }
        if let Some(k) = self.button_rects(r).iter().position(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(k));
        }
        if let Some(b) = self
            .blocks
            .iter()
            .find(|b| self.block_rect(r, b).is_some_and(|br| br.contains(x, y)))
        {
            return Some(EventKind::Select(b.index));
        }
        let g = self.geo(r);
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
    }
}
