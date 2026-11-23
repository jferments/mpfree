//! A lane of bars under a grid, a value each, that a finger draws: one
//! bar per step on the Sequencer's Steps (its Velocity and Chance lanes,
//! step sequencer refactor D3) or one per note in the piano roll (its
//! lanes, the piano roll plan's step 2A.1, which extends this without
//! changing its callers).
//!
//! It knows nothing about steps, notes or locks. The caller gives each
//! bar its place across the lane in pixels (a step's cell, or a note's
//! start, so bars may be narrow and uneven), its value and how to show
//! it, and the lane's range and step (velocity 1 to 127, chance 1 to 100).
//! A finger's stroke sets the bars along its path, filling in between
//! touch samples so a fast stroke skips none; a tap is a stroke of one
//! bar. While the finger is down only the lane's preview changes. When it
//! lifts, [`Lane::touch`] returns the bars the stroke changed, once, and
//! the caller makes them one edit, so one UNDO takes the stroke back. A
//! cancelled touch, or a second finger, ends the stroke with nothing.
//!
//! Colours come from the caller (a theme's `series`, `track` or `qlink`
//! colour), and labels are drawn in the theme's own inks: no colour is
//! written here.

use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Touch};

/// How a lane draws its values: bars from the foot, or from the middle
/// for a value either side of 0 (a repeat's velocity ramp, piano roll
/// plan 2A.1). A pressure line inside a note is room left for the piano
/// roll's phase 3.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LaneStyle {
    /// A bar per value, as tall as the value, from the lane's foot.
    #[default]
    Bars,
    /// A bar per value from the height of 0 (the foot if 0 is out of the
    /// range), up for a value over 0 and down for one under it.
    Centred,
}

/// One bar: where it is across the lane and its value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bar {
    /// Its left edge and width, in pixels.
    pub x: i32,
    pub w: i32,
    pub value: i32,
    /// Outlined: a step or note that is picked.
    pub picked: bool,
    /// Faint: a step that is switched off, say.
    pub dim: bool,
    /// Drawn over the bar where it fits ("96", "50 %"); empty for none.
    pub label: String,
}

/// What a touch did to the lane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaneTouch {
    /// Not the lane's: a touch outside it, or another finger's move.
    None,
    /// The stroke's preview changed: draw the lane again. Nothing to edit
    /// yet.
    Preview,
    /// The stroke ended: each bar it changed, in bar order, with its new
    /// value, for the caller's one edit. Empty when nothing changed, or
    /// when the touch was cancelled or a second finger came down.
    Done(Vec<(usize, i32)>),
}

/// A finger drawing on the lane.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stroke {
    finger: u64,
    /// Where it was last.
    at: (i32, i32),
    /// The values it has set so far: (bar, value).
    set: Vec<(usize, i32)>,
}

/// The lane: its bars and range, and the stroke a finger is drawing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lane {
    pub bars: Vec<Bar>,
    pub min: i32,
    pub max: i32,
    /// Values go in steps of this from `min` (1 for velocity).
    pub step: i32,
    pub style: LaneStyle,
    stroke: Option<Stroke>,
}

/// Room over the bars for their labels.
const LABEL_H: i32 = 14;

impl Lane {
    /// A lane of values from `min` to `max` in steps of `step`, no bars.
    pub fn new(min: i32, max: i32, step: i32) -> Lane {
        Lane {
            min,
            max: max.max(min),
            step: step.max(1),
            ..Lane::default()
        }
    }

    /// The bars, keeping a stroke under way only when they are the same
    /// bars in the same places (a redraw from the caller mid-stroke).
    pub fn set_bars(&mut self, bars: Vec<Bar>) {
        let same = bars.len() == self.bars.len()
            && bars
                .iter()
                .zip(&self.bars)
                .all(|(a, b)| (a.x, a.w) == (b.x, b.w));
        if !same {
            self.stroke = None;
        }
        self.bars = bars;
    }

    /// A finger is drawing on the lane.
    pub fn stroking(&self) -> bool {
        self.stroke.is_some()
    }

    /// Bar `k`'s value as drawn: the stroke's, while it sets it.
    pub fn shown(&self, k: usize) -> Option<i32> {
        let drawn = self
            .stroke
            .as_ref()
            .and_then(|s| s.set.iter().find(|(b, _)| *b == k).map(|&(_, v)| v));
        drawn.or_else(|| self.bars.get(k).map(|b| b.value))
    }

    /// The bar across `x`, if any (the first, where bars overlap).
    pub fn bar_at(&self, x: i32) -> Option<usize> {
        self.bars.iter().position(|b| x >= b.x && x < b.x + b.w)
    }

    /// The value a finger at height `y` in the lane `r` sets: `max` at
    /// the top of the bars, `min` at the foot, in steps of `step`.
    pub fn value_at(&self, y: i32, r: Rect) -> i32 {
        let (top, foot) = (r.y + LABEL_H, r.bottom() - 1);
        let span = (foot - top).max(1) as f32;
        let frac = ((foot - y) as f32 / span).clamp(0.0, 1.0);
        let raw = frac * (self.max - self.min) as f32;
        let steps = (raw / self.step as f32).round() as i32;
        (self.min + steps * self.step).clamp(self.min, self.max)
    }

    /// A touch on the lane `r`: a finger down starts a stroke, its moves
    /// set the bars along its path, its lift returns what changed (see
    /// [`LaneTouch`]). One finger at a time: a second finger down ends the
    /// stroke with nothing, as a cancel does.
    pub fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> LaneTouch {
        let ours = self.stroke.as_ref().is_some_and(|s| s.finger == finger);
        match phase {
            Touch::Down if self.stroke.is_some() => {
                self.stroke = None;
                LaneTouch::Done(Vec::new())
            }
            Touch::Down if r.contains(x, y) => {
                self.stroke = Some(Stroke {
                    finger,
                    at: (x, y),
                    set: Vec::new(),
                });
                self.draw_to(x, y, r);
                LaneTouch::Preview
            }
            Touch::Move if ours => {
                self.draw_to(x, y, r);
                LaneTouch::Preview
            }
            Touch::Up if ours => {
                self.draw_to(x, y, r);
                let set = self.stroke.take().map(|s| s.set).unwrap_or_default();
                let mut changed: Vec<(usize, i32)> = set
                    .into_iter()
                    .filter(|&(k, v)| self.bars.get(k).is_some_and(|b| b.value != v))
                    .collect();
                changed.sort_unstable_by_key(|&(k, _)| k);
                LaneTouch::Done(changed)
            }
            Touch::Cancel if ours => {
                self.stroke = None;
                LaneTouch::Done(Vec::new())
            }
            _ => LaneTouch::None,
        }
    }

    /// The finger went from where it was to (`x`, `y`): every bar whose
    /// middle it passed takes the height the straight line between them
    /// has there, and the bar under it now takes its height.
    fn draw_to(&mut self, x: i32, y: i32, r: Rect) {
        let Some((x0, y0)) = self.stroke.as_ref().map(|s| s.at) else {
            return;
        };
        let (lo, hi) = (x0.min(x), x0.max(x));
        let mut set: Vec<(usize, i32)> = Vec::new();
        for (k, b) in self.bars.iter().enumerate() {
            let mid = b.x + b.w / 2;
            if mid < lo || mid > hi || x == x0 {
                continue;
            }
            let at = y0 + ((y - y0) as f32 * (mid - x0) as f32 / (x - x0) as f32).round() as i32;
            set.push((k, self.value_at(at, r)));
        }
        if let Some(k) = self.bar_at(x) {
            set.push((k, self.value_at(y, r)));
        }
        let Some(s) = self.stroke.as_mut() else {
            return;
        };
        for (k, v) in set {
            match s.set.iter_mut().find(|(b, _)| *b == k) {
                Some(e) => e.1 = v,
                None => s.set.push((k, v)),
            }
        }
        s.at = (x, y);
    }

    /// The height in the lane `r` that value `v` reaches: the foot for
    /// `min`, the top of the bars for `max`.
    pub fn y_of(&self, v: i32, r: Rect) -> i32 {
        let room = (r.h - LABEL_H).max(1);
        let span = (self.max - self.min).max(1);
        r.bottom() - (v.clamp(self.min, self.max) - self.min) * room / span
    }

    /// Draw the lane in `r`, its bars in `colour` (from the theme), each
    /// as tall as its value from the lane's foot (or from 0's height,
    /// [`LaneStyle::Centred`]), the stroke's values in place of theirs
    /// while it draws.
    pub fn paint(&self, cx: &mut PaintCx, r: Rect, colour: Color) {
        let t = cx.theme.clone();
        let base = match self.style {
            LaneStyle::Bars => r.bottom(),
            LaneStyle::Centred if self.min < 0 && self.max > 0 => self.y_of(0, r),
            LaneStyle::Centred => r.bottom(),
        };
        // The foot (or 0's line), faint, under every bar.
        cx.canvas.fill_rect(
            Rect::new(r.x, base.min(r.bottom()) - 1, r.w, 1),
            t.text_faint,
        );
        for (k, b) in self.bars.iter().enumerate() {
            let v = self.shown(k).unwrap_or(b.value).clamp(self.min, self.max);
            let top = self.y_of(v, r);
            let (y, h) = match top < base {
                true => (top, (base - top).max(2)),
                false => (base, (top - base).max(2)),
            };
            let y = y.min(r.bottom() - h);
            let w = (b.w - 4).max(2);
            let bar = Rect::new(b.x + (b.w - w) / 2, y, w, h);
            let fill = if b.dim {
                colour.mix(t.surface, 0.6)
            } else {
                colour
            };
            cx.canvas.fill_round_rect(bar, 2, fill);
            if b.picked {
                cx.canvas.stroke_round_rect(bar.inset(-2), 3, 2, t.text);
            }
            if !b.label.is_empty() && b.w >= 18 {
                let label = Rect::new(b.x, (bar.y - LABEL_H).max(r.y), b.w, LABEL_H);
                cx.text(
                    FontId::Sans,
                    11.0,
                    label,
                    Align::Center,
                    t.text_dim,
                    &b.label,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16 steps of 40 px in a lane 80 px high at (100, 200).
    fn lane() -> (Lane, Rect) {
        let mut l = Lane::new(1, 127, 1);
        l.set_bars(
            (0..16)
                .map(|k| Bar {
                    x: 100 + k * 40,
                    w: 40,
                    value: 64,
                    ..Bar::default()
                })
                .collect(),
        );
        (l, Rect::new(100, 200, 640, 80))
    }

    #[test]
    fn the_top_of_the_lane_is_its_most_and_the_foot_its_least() {
        let (mut l, r) = lane();
        assert_eq!(l.value_at(r.y + LABEL_H, r), 127);
        assert_eq!(l.value_at(r.y, r), 127, "the label room counts as the top");
        assert_eq!(l.value_at(r.bottom() - 1, r), 1);
        assert_eq!(l.value_at(r.bottom() + 30, r), 1);
        // Chance in steps of 5 from 0: never a value between.
        l.min = 0;
        l.max = 100;
        l.step = 5;
        for y in r.y..r.bottom() {
            assert_eq!(l.value_at(y, r) % 5, 0);
        }
    }

    #[test]
    fn a_tap_sets_one_bar_and_says_so_once_on_the_lift() {
        let (mut l, r) = lane();
        let top = r.y + LABEL_H;
        assert_eq!(l.touch(1, Touch::Down, 225, top, r), LaneTouch::Preview);
        assert!(l.stroking());
        assert_eq!(l.shown(3), Some(127), "the preview");
        assert_eq!(l.bars[3].value, 64, "the bar itself waits for the lift");
        assert_eq!(
            l.touch(1, Touch::Up, 225, top, r),
            LaneTouch::Done(vec![(3, 127)])
        );
        assert!(!l.stroking());
        assert_eq!(l.touch(1, Touch::Up, 225, top, r), LaneTouch::None);
        // A tap at a bar's own value changes nothing.
        let mid = r.y + r.h / 2;
        l.bars[3].value = l.value_at(mid, r);
        l.touch(2, Touch::Down, 225, mid, r);
        assert_eq!(l.touch(2, Touch::Up, 225, mid, r), LaneTouch::Done(vec![]));
        // Outside the lane: not its touch.
        assert_eq!(l.touch(3, Touch::Down, 50, 50, r), LaneTouch::None);
    }

    #[test]
    fn a_fast_stroke_sets_every_bar_it_crossed_along_its_line() {
        let (mut l, r) = lane();
        let (top, foot) = (r.y + LABEL_H, r.bottom() - 1);
        // From the foot of step 1 to the top of step 16 in one move.
        l.touch(1, Touch::Down, 120, foot, r);
        l.touch(1, Touch::Move, 720, top, r);
        let LaneTouch::Done(set) = l.touch(1, Touch::Up, 720, top, r) else {
            panic!("a lift ends the stroke");
        };
        assert_eq!(set.len(), 16, "{set:?}");
        assert_eq!(set.first(), Some(&(0, 1)));
        assert_eq!(set.last(), Some(&(15, 127)));
        assert!(set.windows(2).all(|w| w[0].1 < w[1].1), "a rising line");
        // Back over some of them: the newest height wins, once per bar.
        l.touch(1, Touch::Down, 300, foot, r);
        l.touch(1, Touch::Move, 120, foot, r);
        l.touch(1, Touch::Move, 300, top, r);
        let LaneTouch::Done(set) = l.touch(1, Touch::Up, 300, top, r) else {
            panic!();
        };
        let bars: Vec<usize> = set.iter().map(|s| s.0).collect();
        assert_eq!(bars, [0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn a_second_finger_or_a_cancel_ends_the_stroke_with_nothing() {
        let (mut l, r) = lane();
        let top = r.y + LABEL_H;
        l.touch(1, Touch::Down, 120, top, r);
        l.touch(1, Touch::Move, 400, top, r);
        assert_eq!(
            l.touch(2, Touch::Down, 500, top, r),
            LaneTouch::Done(vec![])
        );
        assert_eq!(l.shown(3), Some(64), "the preview is gone");
        assert_eq!(l.touch(2, Touch::Move, 600, top, r), LaneTouch::None);
        assert_eq!(l.touch(1, Touch::Up, 400, top, r), LaneTouch::None);
        l.touch(3, Touch::Down, 120, top, r);
        assert_eq!(
            l.touch(3, Touch::Cancel, 120, top, r),
            LaneTouch::Done(vec![])
        );
        assert!(l.bars.iter().all(|b| b.value == 64));
    }

    /// A centred lane (a ramp, -126 to 126) draws each bar from 0's
    /// height, up for a value over 0 and down for one under it; a finger
    /// on that line sets about 0.
    #[test]
    fn a_centred_lane_draws_from_its_middle() {
        use mpfree_ui::{Fonts, SoftCanvas, Theme};
        let r = Rect::new(0, 0, 200, 94);
        let mut l = Lane::new(-126, 126, 1);
        l.style = LaneStyle::Centred;
        l.set_bars(
            [(20, 100), (100, -100)]
                .into_iter()
                .map(|(x, value)| Bar {
                    x,
                    w: 20,
                    value,
                    ..Bar::default()
                })
                .collect(),
        );
        let zero = l.y_of(0, r);
        assert!(l.value_at(zero, r).abs() <= 2, "about 0 at the line");
        let theme = Theme::dark();
        let mut c = SoftCanvas::new(r.w, r.h);
        let mut fonts = Fonts::embedded();
        l.paint(
            &mut PaintCx {
                canvas: &mut c,
                theme: &theme,
                fonts: &mut fonts,
            },
            r,
            theme.accent,
        );
        let lit = |x: i32, y: i32| c.pixel(x, y) == c.pixel(30, zero - 10);
        assert!(lit(30, zero - 20) && !lit(30, zero + 20), "up from 0");
        assert!(lit(110, zero + 20) && !lit(110, zero - 20), "down from 0");
    }

    #[test]
    fn narrow_uneven_bars_are_each_set_where_the_finger_passed() {
        // A note's start each, as the piano roll draws them: 6 px wide,
        // unevenly placed, one far off the stroke.
        let r = Rect::new(0, 0, 800, 94);
        let mut l = Lane::new(1, 127, 1);
        l.set_bars(
            [10, 17, 90, 300, 700]
                .into_iter()
                .map(|x| Bar {
                    x,
                    w: 6,
                    value: 100,
                    ..Bar::default()
                })
                .collect(),
        );
        l.touch(1, Touch::Down, 0, 93, r);
        l.touch(1, Touch::Move, 320, 93, r);
        let LaneTouch::Done(set) = l.touch(1, Touch::Up, 320, 93, r) else {
            panic!();
        };
        assert_eq!(set, [(0, 1), (1, 1), (2, 1), (3, 1)]);
        // New bars in other places drop a stroke under way.
        l.touch(1, Touch::Down, 12, 93, r);
        let mut moved = l.bars.clone();
        moved[0].x = 40;
        l.set_bars(moved);
        assert!(!l.stroking());
    }
}
