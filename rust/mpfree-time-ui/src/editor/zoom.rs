//! The roll's view: which stretch of the clip and which rows show, and
//! how big (piano roll plan 1A.1). One small value for each clip, kept by
//! the screen ([`crate::view::TimePage`]) and never by the time engine, so
//! a pinch or a scroll costs the sound nothing.
//!
//! Time runs left to right at `ppt` pixels a tick, from tick `start` at
//! the roll's left edge. A tick's pixel is rounded to the nearest, so it
//! never runs backwards and is within half a pixel; a pixel is always
//! coarser than a tick (the closest zoom shows a 1/64 at [`FINEST_PX`],
//! under a pixel a tick), so a pixel turned into its first tick and back
//! is the same pixel. Rows are `row_h` pixels tall, note 0 at the bottom;
//! the roll's bottom edge sits `lift` pixels above note 0's bottom.

use mpfree_seq::notes::DIVISIONS;
use mpfree_transport::time::{Tick, PPQ};
use mpfree_ui::Rect;

/// The shortest a row gets (16 to 19 notes on the roll).
pub const ROW_MIN: i32 = 24;
/// Rows open at least this tall (4.7 mm on the MPC's screen).
pub const ROW_OPEN: i32 = 40;
/// Rows open at most this tall, when the clip's notes fit with room over.
pub const ROW_FILL: i32 = 64;
/// The tallest a row gets.
pub const ROW_MAX: i32 = 96;
/// The closest zoom: a 1/64 this wide.
pub const FINEST_PX: f64 = 40.0;
/// Pixels a tick at the closest zoom (under 1, so ticks are finer).
pub const MAX_PPT: f64 = FINEST_PX / (PPQ as f64 / 16.0);
/// Snap Auto picks the finest division whose cells are at least this wide
/// ...
pub const AUTO_MIN_PX: f64 = 28.0;
/// ... and moves to a finer one only once its cells are this wide, so a
/// small zoom doesn't flip it back and forth.
pub const AUTO_FINER_PX: f64 = 35.0;
/// The roll opens on the whole clip when a 1/16 is at least this wide,
/// else on two bars from the first note.
const OPEN_STEP_PX: f64 = 24.0;
/// Snap Auto's divisions, coarse to fine, by index into [`DIVISIONS`]:
/// straight (1 bar, 1/2, 1/4, 1/8, 1/16, 1/32, 1/64) ...
const STRAIGHT: [usize; 7] = [0, 1, 2, 4, 6, 8, 10];
/// ... and with Auto triplets (1 bar, 1/2, then 1/4T to 1/64T).
const TRIPLET: [usize; 7] = [0, 1, 3, 5, 7, 9, 11];

/// The roll's view (see the module notes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollView {
    /// The tick at the roll's left edge.
    pub start: Tick,
    /// Pixels a tick, from the whole clip across up to [`MAX_PPT`].
    pub ppt: f64,
    /// How far the roll's bottom edge is above note 0's bottom, in pixels.
    pub lift: i32,
    /// A row's height, [`ROW_MIN`] to [`ROW_MAX`].
    pub row_h: i32,
    /// The roll pages along with the playhead; a scroll by hand turns it
    /// off.
    pub follow: bool,
}

impl Default for RollView {
    /// Four bars across 1,000 pixels from C2, rows at [`ROW_OPEN`]: a
    /// view for before the roll has a size ([`RollView::open`] is the
    /// real one).
    fn default() -> RollView {
        RollView {
            start: 0,
            ppt: 1000.0 / (16 * PPQ) as f64,
            lift: 48 * ROW_OPEN,
            row_h: ROW_OPEN,
            follow: false,
        }
    }
}

impl RollView {
    /// The view a clip opens with on a roll of `roll`: the notes filling
    /// the rows at [`ROW_OPEN`] to [`ROW_FILL`] each (or from the lowest
    /// at [`ROW_OPEN`] when they don't fit), and the whole clip across as
    /// long as a 1/16 is at least 24 px wide, else two bars from the first
    /// note. `rows` is how many rows there are (128 notes, or 16 pads).
    pub fn open<I>(notes: I, length: Tick, bar: Tick, roll: Rect, rows: u8) -> RollView
    where
        I: IntoIterator<Item = (Tick, u8)>,
    {
        let mut first: Option<Tick> = None;
        let mut span: Option<(u8, u8)> = None;
        for (t, n) in notes {
            first = Some(first.map_or(t, |f| f.min(t)));
            span = Some(span.map_or((n, n), |(lo, hi)| (lo.min(n), hi.max(n))));
        }
        let length = length.max(1);
        let bar = bar.max(1);
        let w = f64::from(roll.w.max(1));
        let whole = w / length as f64;
        let (start, ppt) = if whole * (PPQ as f64 / 4.0) >= OPEN_STEP_PX {
            (0, whole)
        } else {
            let from = first.unwrap_or(0).clamp(0, length - 1);
            (from - from % bar, w / (2 * bar) as f64)
        };
        // An empty clip shows the octave from C2 (pads: from Pad 1).
        let (lo, hi) = span.unwrap_or(if rows <= 16 { (0, 0) } else { (48, 59) });
        let n = i32::from(hi - lo) + 1;
        let fit = roll.h / n.max(1);
        let (row_h, lift) = if fit >= ROW_OPEN {
            let rh = fit.min(ROW_FILL);
            (rh, i32::from(lo) * rh - (roll.h - n * rh) / 2)
        } else {
            (ROW_OPEN, i32::from(lo) * ROW_OPEN)
        };
        let mut v = RollView {
            start,
            ppt,
            lift,
            row_h,
            follow: false,
        };
        v.clamp(roll, length, rows);
        v
    }

    /// Kept within what the roll can show: no wider than the whole clip,
    /// no closer than [`MAX_PPT`], rows [`ROW_MIN`] to [`ROW_MAX`], and
    /// scrolled no further than the clip's end or the top row.
    pub fn clamp(&mut self, roll: Rect, length: Tick, rows: u8) {
        let length = length.max(1);
        let w = f64::from(roll.w.max(1));
        let widest = (w / length as f64).min(MAX_PPT);
        self.ppt = if self.ppt.is_finite() {
            self.ppt.clamp(widest, MAX_PPT)
        } else {
            widest
        };
        // (A hair over, so the whole clip at its own zoom doesn't leave a
        // tick to scroll by.)
        let shown = (w / self.ppt + 1e-6).floor() as Tick;
        self.start = self.start.clamp(0, (length - shown).max(0));
        self.row_h = self.row_h.clamp(ROW_MIN, ROW_MAX);
        let top = (i32::from(rows) * self.row_h - roll.h).max(0);
        self.lift = self.lift.clamp(0, top);
    }

    /// The view kept within the roll (see [`RollView::clamp`]).
    pub fn clamped(mut self, roll: Rect, length: Tick, rows: u8) -> RollView {
        self.clamp(roll, length, rows);
        self
    }

    /// The x of tick `t`: rounded to the nearest pixel.
    pub fn x_of(&self, roll: Rect, t: Tick) -> i32 {
        roll.x + ((t - self.start) as f64 * self.ppt + 0.5).floor() as i32
    }

    /// The first tick drawn at pixel `x`: [`RollView::x_of`] gives `x`
    /// back exactly.
    pub fn tick_at(&self, roll: Rect, x: i32) -> Tick {
        let u = (f64::from(x - roll.x) - 0.5) / self.ppt;
        let mut t = self.start + u.ceil() as Tick;
        // Floating point can land a tick off at a pixel's edge.
        while self.x_of(roll, t - 1) >= x {
            t -= 1;
        }
        while self.x_of(roll, t) < x {
            t += 1;
        }
        t
    }

    /// The start of the cell of `snap` ticks drawn under pixel `x`: the
    /// last cell whose line is at or left of `x`, so a touch lands in the
    /// cell it is in. With no snap, the first tick at `x`.
    pub fn cell_at(&self, roll: Rect, x: i32, snap: Tick) -> Tick {
        if snap <= 0 {
            return self.tick_at(roll, x);
        }
        let last = self.tick_at(roll, x + 1) - 1;
        last.div_euclid(snap) * snap
    }

    /// The tick in the middle of pixel `x`, not rounded.
    pub fn tick_mid(&self, roll: Rect, x: i32) -> f64 {
        self.start as f64 + f64::from(x - roll.x) / self.ppt
    }

    /// How many ticks the roll shows across.
    pub fn ticks_shown(&self, roll: Rect) -> Tick {
        (f64::from(roll.w) / self.ppt).ceil() as Tick
    }

    /// The y of the top of `note`'s row (which may be off the roll).
    pub fn row_top(&self, roll: Rect, note: u8) -> i32 {
        roll.bottom() - ((i32::from(note) + 1) * self.row_h - self.lift)
    }

    /// The note of the row at `y`, if `y` is on the roll and the row is
    /// one of `rows`.
    pub fn note_at(&self, roll: Rect, y: i32, rows: u8) -> Option<u8> {
        if y < roll.y || y >= roll.bottom() {
            return None;
        }
        let row = (self.lift + roll.bottom() - 1 - y) / self.row_h.max(1);
        (0..i32::from(rows)).contains(&row).then_some(row as u8)
    }

    /// The rows on the roll, lowest first (some may show only in part).
    pub fn notes_shown(&self, roll: Rect, rows: u8) -> std::ops::RangeInclusive<u8> {
        let rh = self.row_h.max(1);
        let top = i32::from(rows.max(1)) - 1;
        let lo = (self.lift / rh).clamp(0, top);
        let hi = ((self.lift + roll.h - 1) / rh).clamp(lo, top);
        lo as u8..=hi as u8
    }

    /// The row under `y`, with its fraction (2.5: half way up note 2's
    /// row), for a pinch to keep it under the fingers.
    pub fn row_mid(&self, roll: Rect, y: i32) -> f64 {
        f64::from(self.lift + roll.bottom() - y) / f64::from(self.row_h.max(1))
    }

    /// Zoomed in time to `ppt` with `tick` (unrounded) at pixel `x`.
    pub fn zoom_time(
        &self,
        roll: Rect,
        length: Tick,
        rows: u8,
        ppt: f64,
        tick: f64,
        x: i32,
    ) -> RollView {
        let mut v = RollView { ppt, ..*self };
        v.clamp(roll, length, rows);
        v.start = (tick - f64::from(x - roll.x) / v.ppt).round() as Tick;
        v.clamped(roll, length, rows)
    }

    /// Zoomed in pitch to rows `row_h` tall with row `row` (unrounded,
    /// see [`RollView::row_mid`]) at `y`.
    pub fn zoom_rows(
        &self,
        roll: Rect,
        length: Tick,
        rows: u8,
        row_h: i32,
        row: f64,
        y: i32,
    ) -> RollView {
        let row_h = row_h.clamp(ROW_MIN, ROW_MAX);
        let lift = (row * f64::from(row_h)).round() as i32 - (roll.bottom() - y);
        RollView {
            row_h,
            lift,
            ..*self
        }
        .clamped(roll, length, rows)
    }

    /// The ticks `from` to `to` across the roll and the notes `lo` to
    /// `hi` on it, at rows [`ROW_MIN`] to [`ROW_FILL`] (from `lo` when
    /// they don't fit): Fit.
    pub fn fit(
        &self,
        roll: Rect,
        length: Tick,
        rows: u8,
        ticks: (Tick, Tick),
        notes: (u8, u8),
    ) -> RollView {
        let (from, to) = (
            ticks.0.min(ticks.1),
            ticks.0.max(ticks.1).max(ticks.0.min(ticks.1) + 1),
        );
        let (lo, hi) = (notes.0.min(notes.1), notes.0.max(notes.1));
        let n = i32::from(hi - lo) + 1;
        let rh = (roll.h / n).clamp(ROW_MIN, ROW_FILL);
        let lift = if n * rh <= roll.h {
            i32::from(lo) * rh - (roll.h - n * rh) / 2
        } else {
            i32::from(lo) * rh
        };
        let mut v = RollView {
            start: from,
            ppt: f64::from(roll.w.max(1)) / (to - from) as f64,
            lift,
            row_h: rh,
            follow: self.follow,
        };
        v.clamp(roll, length, rows);
        // Clamped closer than the stretch: centred on it.
        let shown = v.ticks_shown(roll);
        if shown > to - from {
            v.start = from - (shown - (to - from)) / 2;
            v.clamp(roll, length, rows);
        }
        v
    }

    /// Paged on so tick `t` (the playhead) shows, by whole screens: true
    /// if it moved.
    pub fn follow_to(&mut self, roll: Rect, length: Tick, rows: u8, t: Tick) -> bool {
        let shown = self.ticks_shown(roll).max(1);
        if (self.start..self.start + shown).contains(&t) {
            return false;
        }
        let was = self.start;
        self.start = t.max(0) / shown * shown;
        self.clamp(roll, length, rows);
        self.start != was
    }
}

/// Snap Auto's division (an index into [`DIVISIONS`]) at `ppt` pixels a
/// tick, coming from division `prev`: the finest whose cells are at least
/// [`AUTO_MIN_PX`] wide, but moving finer only once the finer cells are
/// [`AUTO_FINER_PX`] wide. Straight divisions unless `triplets`.
pub fn auto_division(prev: usize, ppt: f64, triplets: bool) -> usize {
    let ladder = if triplets { &TRIPLET } else { &STRAIGHT };
    let ticks = DIVISIONS.get(prev).map_or(0, |d| d.0);
    // Where `prev` sits on the ladder: itself, or the step nearest it.
    let mut k = ladder.iter().position(|&d| d == prev).unwrap_or_else(|| {
        (0..ladder.len())
            .min_by_key(|&k| (DIVISIONS[ladder[k]].0 - ticks).abs())
            .unwrap_or(0)
    });
    let w = |k: usize| DIVISIONS[ladder[k]].0 as f64 * ppt;
    while k > 0 && w(k) < AUTO_MIN_PX {
        k -= 1;
    }
    while k + 1 < ladder.len() && w(k + 1) >= AUTO_FINER_PX {
        k += 1;
    }
    ladder[k]
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLL: Rect = Rect {
        x: 92,
        y: 180,
        w: 1060,
        h: 440,
    };

    fn view(start: Tick, ppt: f64) -> RollView {
        RollView {
            start,
            ppt,
            ..RollView::default()
        }
    }

    /// A tick turned into a pixel never runs backwards and is within half
    /// a pixel at every zoom; a pixel turned into its first tick and back
    /// is the same pixel, and that tick is the first one there.
    #[test]
    fn ticks_and_pixels_turn_into_each_other() {
        for &ppt in &[
            1060.0 / 61_440.0,
            1060.0 / 15_360.0,
            0.1,
            1.0 / 3.0,
            0.5,
            MAX_PPT,
        ] {
            for &start in &[0, 7, 960, 3_841] {
                let v = view(start, ppt);
                let mut last = i32::MIN;
                for t in start..start + 4_000 {
                    let x = v.x_of(ROLL, t);
                    assert!(x >= last, "ppt {ppt}: tick {t} ran back");
                    let exact = f64::from(ROLL.x) + (t - start) as f64 * ppt;
                    assert!(
                        (f64::from(x) - exact).abs() <= 0.5 + 1e-9,
                        "ppt {ppt}: tick {t}"
                    );
                    last = x;
                }
                for x in ROLL.x..ROLL.right() {
                    let t = v.tick_at(ROLL, x);
                    assert_eq!(v.x_of(ROLL, t), x, "ppt {ppt} start {start}");
                    assert!(v.x_of(ROLL, t - 1) < x);
                }
            }
        }
    }

    /// A touch lands in the cell it is in: every pixel of a cell (from
    /// its line to the next cell's) gives that cell's start.
    #[test]
    fn a_touch_lands_on_the_cell_under_it() {
        for &(ppt, snap) in &[
            (0.1, 240),
            (1060.0 / 15_360.0, 480),
            (MAX_PPT, 60),
            (0.37, 160),
        ] {
            let v = view(0, ppt);
            for k in 0..20 {
                let (a, b) = (v.x_of(ROLL, k * snap), v.x_of(ROLL, (k + 1) * snap));
                for x in a..b {
                    assert_eq!(v.cell_at(ROLL, x, snap), k * snap, "ppt {ppt} x {x}");
                }
            }
        }
    }

    /// Rows: note 0 at the bottom, `lift` scrolling them, and the row
    /// under a pixel the one drawn there.
    #[test]
    fn rows_and_the_note_under_a_finger() {
        let v = RollView {
            lift: 60 * 40 + 10,
            row_h: 40,
            ..RollView::default()
        };
        // Note 60's row starts 10 px under the roll's bottom edge.
        assert_eq!(v.row_top(ROLL, 60), ROLL.bottom() - 30);
        assert_eq!(v.note_at(ROLL, ROLL.bottom() - 1, 128), Some(60));
        assert_eq!(v.note_at(ROLL, ROLL.bottom() - 30, 128), Some(60));
        assert_eq!(v.note_at(ROLL, ROLL.bottom() - 31, 128), Some(61));
        assert_eq!(v.note_at(ROLL, ROLL.bottom(), 128), None);
        assert_eq!(v.notes_shown(ROLL, 128), 60..=71);
        assert_eq!(v.note_at(ROLL, ROLL.y, 16), None, "past the pads");
    }

    /// The view stays within what can show: the whole clip at most, a
    /// 1/64 at 40 px at the closest, rows 24 to 96 px, and no scrolling
    /// past the clip's end or the top row.
    #[test]
    fn the_view_stays_within_the_clip() {
        let length = 4 * 4 * PPQ as Tick;
        let mut v = RollView {
            start: 99_999,
            ppt: 0.0001,
            lift: 1 << 20,
            row_h: 300,
            follow: false,
        };
        v.clamp(ROLL, length, 128);
        assert_eq!(v.ppt, 1060.0 / length as f64);
        assert_eq!(v.start, 0);
        assert_eq!(v.row_h, ROW_MAX);
        assert_eq!(v.lift, 128 * ROW_MAX - ROLL.h);
        v.ppt = 10.0;
        v.start = 99_999;
        v.clamp(ROLL, length, 128);
        assert_eq!(v.ppt, MAX_PPT);
        assert_eq!(v.x_of(ROLL, 60) - v.x_of(ROLL, 0), 40);
        assert_eq!(v.x_of(ROLL, length), ROLL.right(), "scrolled to the end");
        v.row_h = 1;
        v.clamp(ROLL, length, 16);
        assert_eq!(v.row_h, ROW_MIN);
        assert_eq!(v.lift, 0, "16 rows of 24 px fit the roll");
    }

    /// The roll opens on the notes at 40 to 64 px rows, or from the
    /// lowest at 40 px; on the whole clip while a 1/16 is 24 px or wider,
    /// else two bars from the first note's bar.
    #[test]
    fn the_roll_opens_on_the_notes() {
        let bar = 4 * PPQ as Tick;
        // Notes a fifth apart: 8 rows fill 440 px at 55 px; two bars fit
        // across with a 1/16 at 33 px.
        let v = RollView::open([(0, 60), (960, 67)], 2 * bar, bar, ROLL, 128);
        assert_eq!(v.row_h, 55);
        assert_eq!(v.notes_shown(ROLL, 128), 60..=67);
        assert_eq!((v.start, v.x_of(ROLL, 2 * bar)), (0, ROLL.right()));
        // Four bars would make a 1/16 17 px: two bars from the first note.
        let v = RollView::open([(bar + 5, 60)], 4 * bar, bar, ROLL, 128);
        assert_eq!((v.start, v.x_of(ROLL, 3 * bar)), (bar, ROLL.right()));
        // Two notes: rows stop at 64 px, centred.
        let v = RollView::open([(0, 60), (0, 61)], bar, bar, ROLL, 128);
        assert_eq!(v.row_h, ROW_FILL);
        assert_eq!(v.row_top(ROLL, 61) - ROLL.y, (ROLL.h - 2 * 64) / 2);
        // Three octaves: from the lowest at 40 px.
        let v = RollView::open([(0, 36), (0, 72)], bar, bar, ROLL, 128);
        assert_eq!((v.row_h, *v.notes_shown(ROLL, 128).start()), (ROW_OPEN, 36));
        // Sixteen bars: a 1/16 would be 4 px, so two bars from the first
        // note's bar.
        let v = RollView::open([(5 * bar + 7, 60)], 16 * bar, bar, ROLL, 128);
        assert_eq!(v.start, 5 * bar);
        assert_eq!(v.x_of(ROLL, 7 * bar), ROLL.right());
        // Empty: the octave from C2; pads from Pad 1.
        let v = RollView::open([], bar, bar, ROLL, 128);
        assert_eq!(*v.notes_shown(ROLL, 128).start(), 48);
        let v = RollView::open([], bar, bar, ROLL, 16);
        assert_eq!(*v.notes_shown(ROLL, 16).start(), 0);
    }

    /// Zooming keeps the tick and the row under the fingers where the
    /// fingers are, and Fit shows what it is given.
    #[test]
    fn zooming_keeps_the_place_under_the_fingers() {
        let bar = 4 * PPQ as Tick;
        let v = RollView::open([(0, 60), (0, 64)], 4 * bar, bar, ROLL, 128);
        let x = ROLL.x + 300;
        let t = v.tick_mid(ROLL, x);
        let z = v.zoom_time(ROLL, 4 * bar, 128, v.ppt * 3.0, t, x);
        assert!((z.tick_mid(ROLL, x) - t).abs() <= 1.0 / z.ppt);
        let y = ROLL.y + 100;
        let row = v.row_mid(ROLL, y);
        let z = v.zoom_rows(ROLL, 4 * bar, 128, 80, row, y);
        assert_eq!(z.row_h, 80);
        assert!((z.row_mid(ROLL, y) - row).abs() <= 1.0 / 80.0);
        let f = v.fit(ROLL, 4 * bar, 128, (bar, 2 * bar), (62, 63));
        assert_eq!(
            (f.x_of(ROLL, bar), f.x_of(ROLL, 2 * bar)),
            (ROLL.x, ROLL.right())
        );
        assert_eq!(f.row_h, ROW_FILL);
        // Closer than the closest zoom: centred on what it was given.
        let f = v.fit(ROLL, 4 * bar, 128, (bar, bar + 30), (60, 60));
        assert_eq!(f.ppt, MAX_PPT);
        assert!(f.start < bar && f.start + f.ticks_shown(ROLL) > bar + 30);
    }

    /// Snap Auto: the finest straight division at least 28 px wide; it
    /// goes finer only at 35 px, so a small zoom doesn't flip it; Auto
    /// triplets keeps to triplets.
    #[test]
    fn snap_auto_follows_the_zoom_without_flipping() {
        let px = |ticks: f64, w: f64| w / ticks;
        // A 1/16 at 30 px: 1/16 from nothing finer.
        assert_eq!(
            DIVISIONS[auto_division(10, px(240.0, 30.0), false)].1,
            "1/16"
        );
        // A 1/32 at 36 px: coming from 1/16 it goes finer.
        assert_eq!(
            DIVISIONS[auto_division(6, px(120.0, 36.0), false)].1,
            "1/32"
        );
        // A 1/32 at 34 px: from 1/16 it stays, from 1/32 it stays too.
        assert_eq!(
            DIVISIONS[auto_division(6, px(120.0, 34.0), false)].1,
            "1/16"
        );
        assert_eq!(
            DIVISIONS[auto_division(8, px(120.0, 34.0), false)].1,
            "1/32"
        );
        // A 1/32 at 27 px: from 1/32 it goes coarser.
        assert_eq!(
            DIVISIONS[auto_division(8, px(120.0, 27.0), false)].1,
            "1/16"
        );
        // Whole 16-bar clip: a bar at 66 px, so 1/2 (33 px).
        assert_eq!(
            DIVISIONS[auto_division(6, 1060.0 / 61_440.0, false)].1,
            "1/2"
        );
        // Triplets: a 1/16T at 30 px.
        assert_eq!(
            DIVISIONS[auto_division(11, px(160.0, 30.0), true)].1,
            "1/16T"
        );
        // The closest zoom: 1/64 (40 px), 1/64T (26.7 px) too narrow.
        assert_eq!(DIVISIONS[auto_division(6, MAX_PPT, false)].1, "1/64");
        assert_eq!(DIVISIONS[auto_division(6, MAX_PPT, true)].1, "1/32T");
    }

    /// Follow pages by whole screens to keep the playhead on the roll.
    #[test]
    fn follow_pages_to_the_playhead() {
        let bar = 4 * PPQ as Tick;
        let mut v = RollView::open([(0, 60)], 16 * bar, bar, ROLL, 128);
        let shown = v.ticks_shown(ROLL);
        assert!(!v.follow_to(ROLL, 16 * bar, 128, 10));
        assert!(v.follow_to(ROLL, 16 * bar, 128, shown + 5));
        assert_eq!(v.start, shown);
        assert!(v.follow_to(ROLL, 16 * bar, 128, 5));
        assert_eq!(v.start, 0);
    }
}
