//! The row's chords on the Clips grid (composing tools S2.6, layout spec
//! 4r): the strip one row high under the scene pick's row, and the chord
//! picker that opens under one of its chips. The grid
//! ([`crate::widgets::ClipGrid`]) places them and passes touches on; the
//! page fills them from `TimeState::row_chords` and its own picks
//! ([`crate::add_view`]).

use mpfree_ui::text::{Align, FontId};
use mpfree_ui::{Color, PaintCx, Rect};

/// The row's chords under it: its bars across the grid's width, a chip
/// per chord as long as it lasts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChordStrip {
    /// The row it belongs under (the scene pick).
    pub row: usize,
    /// The row's length in whole bars, numbered small along the top.
    pub bars: u32,
    pub chips: Vec<StripChip>,
    /// Where the row plays, from 0 to 1 of its length: the green line.
    pub play: Option<f32>,
    /// The chip the picker is open on: ringed in the accent colour.
    pub open: Option<usize>,
}

/// One chord on the strip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StripChip {
    /// Where it starts and ends, from 0 to 1 of the row's length.
    pub from: f32,
    pub to: f32,
    /// The chord's name ("Cm7"), or "?" for a span heard as no clear chord.
    pub name: String,
    pub look: ChipLook,
    /// A small word after the name where there is room ("heard",
    /// "yours"); empty for none.
    pub word: String,
}

/// How a chip looks: what the chord is to the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChipLook {
    /// Heard in the row's notes: solid.
    #[default]
    Heard,
    /// Suggested where nothing was heard: dashed.
    Suggested,
    /// Set by you: in the accent colour.
    Yours,
}

/// The chord picker under a chip.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerView {
    /// The chip it is open on.
    pub chip: usize,
    /// The bar the chord starts in, from 1.
    pub bar: u32,
    /// The chord the row's notes give there ("G7"), or empty when none was
    /// heard.
    pub heard: String,
    /// The chord being picked ("Ab7"): the Use button's name.
    pub picked: String,
    /// The root and kind lit: indexes into [`ROOTS`] and [`KINDS`].
    pub root: Option<usize>,
    pub kind: Option<usize>,
    pub can_split: bool,
    pub can_join: bool,
    /// The chord is not as heard: As heard gives the row's own back.
    pub can_as_heard: bool,
    /// Something picked that differs from the chord as it is.
    pub can_use: bool,
}

/// The picker's roots, as the row's chords spell them.
pub const ROOTS: [&str; 12] = mpfree_context::key::PC_NAMES;
/// The picker's kinds of chord.
pub const KINDS: [&str; 12] = [
    "major", "minor", "7", "maj7", "m7", "sus4", "6", "m6", "dim", "dim7", "m7b5", "power",
];

/// What a touch on the open picker is: `PICK_ROOT + i` for root `i`,
/// `PICK_KIND + i` for kind `i`, then Split, Join next, As heard, Use, ×
/// and a touch off the picker (which closes it with nothing changed).
pub const PICK_ROOT: usize = 0;
pub const PICK_KIND: usize = 12;
pub const PICK_SPLIT: usize = 24;
pub const PICK_JOIN: usize = 25;
pub const PICK_AS_HEARD: usize = 26;
pub const PICK_USE: usize = 27;
pub const PICK_CLOSE: usize = 28;
pub const PICK_DISMISS: usize = 29;
/// How many picker answers there are.
pub const PICKS: usize = 30;

/// The picker's size.
pub const PICKER_W: i32 = 470;
pub const PICKER_H: i32 = 192;
const PICKER_PAD: i32 = 16;

/// Where the picker's parts are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickerGeo {
    pub panel: Rect,
    /// The little arrow's middle, pointing at the chip (above the panel,
    /// or below it when the picker opens above the chip).
    pub arrow_x: i32,
    pub above: bool,
    pub close: Rect,
    pub roots: [Rect; 12],
    pub kinds: [Rect; 12],
    /// Split, Join next, As heard, Use.
    pub buttons: [Rect; 4],
}

/// The picker under `chip`, kept inside `area`; above the chip when there
/// is no room under it.
pub fn picker_geo(chip: Rect, area: Rect) -> PickerGeo {
    let mid = chip.x + chip.w / 2;
    let x = (mid - PICKER_W / 2)
        .min(area.right() - PICKER_W - 8)
        .max(area.x + 8);
    let below = chip.bottom() + 10;
    let above = below + PICKER_H > area.bottom() - 4 && chip.y - 10 - PICKER_H >= area.y;
    let y = if above { chip.y - 10 - PICKER_H } else { below };
    let panel = Rect::new(x, y, PICKER_W, PICKER_H);
    let inner_x = x + PICKER_PAD;
    let inner_w = PICKER_W - 2 * PICKER_PAD;
    let close = Rect::new(panel.right() - 44, y + 4, 40, 32);
    let mut roots = [Rect::default(); 12];
    let root_w = (inner_w - 11 * 3) / 12;
    let root_gap = (inner_w - 12 * root_w) / 11;
    for (i, r) in roots.iter_mut().enumerate() {
        *r = Rect::new(inner_x + i as i32 * (root_w + root_gap), y + 40, root_w, 32);
    }
    let mut kinds = [Rect::default(); 12];
    let kind_w = (inner_w - 5 * 6) / 6;
    for (i, r) in kinds.iter_mut().enumerate() {
        let (col, row) = ((i % 6) as i32, (i / 6) as i32);
        *r = Rect::new(inner_x + col * (kind_w + 6), y + 80 + row * 34, kind_w, 28);
    }
    let mut buttons = [Rect::default(); 4];
    let button_w = (inner_w - 3 * 8) / 4;
    for (i, r) in buttons.iter_mut().enumerate() {
        *r = Rect::new(inner_x + i as i32 * (button_w + 8), y + 150, button_w, 34);
    }
    PickerGeo {
        panel,
        arrow_x: mid.clamp(panel.x + 18, panel.right() - 18),
        above,
        close,
        roots,
        kinds,
        buttons,
    }
}

/// What a touch at (`x`, `y`) on the open picker is (`PICK_*`).
pub fn picker_hit(g: &PickerGeo, x: i32, y: i32) -> usize {
    if g.close.contains(x, y) {
        return PICK_CLOSE;
    }
    if let Some(i) = g.roots.iter().position(|r| r.contains(x, y)) {
        return PICK_ROOT + i;
    }
    if let Some(i) = g.kinds.iter().position(|r| r.contains(x, y)) {
        return PICK_KIND + i;
    }
    if let Some(i) = g.buttons.iter().position(|r| r.contains(x, y)) {
        return PICK_SPLIT + i;
    }
    if g.panel.contains(x, y) {
        // Between the parts: nothing, but it is the picker's.
        return PICKS;
    }
    PICK_DISMISS
}

/// The picker's buttons' names: Split, Join next, As heard, Use (the
/// chord's name).
pub fn picker_buttons(v: &PickerView) -> [(String, bool); 4] {
    let use_name = if v.picked.is_empty() {
        "Use".to_string()
    } else {
        format!("Use {}", v.picked)
    };
    [
        ("Split".to_string(), v.can_split),
        ("Join next".to_string(), v.can_join),
        ("As heard".to_string(), v.can_as_heard),
        (use_name, v.can_use),
    ]
}

/// Draw the picker.
pub fn paint_picker(cx: &mut PaintCx, g: &PickerGeo, v: &PickerView) {
    let t = cx.theme.clone();
    let p = g.panel;
    // A shadow's edge, then the panel and its arrow at the chip.
    cx.canvas
        .fill_round_rect(p.inset(-2), t.round(16), t.bg.mix(t.outline, 0.5));
    cx.canvas.fill_round_rect(p, t.round(14), t.raised);
    let ay = if g.above { p.bottom() } else { p.y - 8 };
    for k in 0..8 {
        let half = if g.above { 8 - k } else { k + 1 };
        cx.canvas
            .fill_rect(Rect::new(g.arrow_x - half, ay + k, 2 * half, 1), t.raised);
    }
    let head = Rect::new(p.x + PICKER_PAD, p.y + 8, p.w - 2 * PICKER_PAD - 44, 26);
    let bar = format!("BAR {}", v.bar);
    let bar_w = cx.measure(FontId::Bold, t.size_small - 1.0, &bar);
    cx.text(
        FontId::Bold,
        t.size_small - 1.0,
        head,
        Align::Left,
        t.text_dim,
        &bar,
    );
    let mut x = head.x + bar_w + 14;
    let heard = if v.heard.is_empty() {
        "nothing heard".to_string()
    } else {
        format!("heard {}", v.heard)
    };
    let px = t.size_small - 1.0;
    x += cx.text(
        FontId::Sans,
        px,
        Rect::new(x, head.y, head.right() - x, head.h),
        Align::Left,
        t.text_dim,
        &heard,
    );
    if !v.picked.is_empty() && v.picked != v.heard {
        x += 8;
        x += cx.text(
            FontId::Sans,
            px,
            Rect::new(x, head.y, (head.right() - x).max(0), head.h),
            Align::Left,
            t.text_dim,
            "→",
        );
        x += 8;
        let ink = t.readable(t.accent, t.raised);
        cx.text(
            FontId::Bold,
            px,
            Rect::new(x, head.y, (head.right() - x).max(0), head.h),
            Align::Left,
            ink,
            &v.picked,
        );
    }
    cx.text(
        FontId::Sans,
        t.size_body + 2.0,
        g.close,
        Align::Center,
        t.text_dim,
        "×",
    );
    for (i, r) in g.roots.iter().enumerate() {
        let lit = v.root == Some(i);
        let fill = if lit { t.accent } else { t.surface };
        cx.canvas.fill_round_rect(*r, t.round(8), fill);
        let ink = if lit { t.ink_on(t.accent) } else { t.text };
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            *r,
            Align::Center,
            ink,
            ROOTS[i],
        );
    }
    for (i, r) in g.kinds.iter().enumerate() {
        let lit = v.kind == Some(i);
        if lit {
            cx.canvas
                .fill_round_rect(*r, t.round(8), t.raised.mix(t.accent, 0.2));
            cx.canvas.stroke_round_rect(*r, t.round(8), 2, t.accent);
        } else {
            cx.canvas.stroke_round_rect(*r, t.round(8), 1, t.outline);
        }
        let ink = if lit {
            t.readable(t.accent, t.raised.mix(t.accent, 0.2))
        } else {
            t.text
        };
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            *r,
            Align::Center,
            ink,
            KINDS[i],
        );
    }
    for (k, ((name, live), r)) in picker_buttons(v).iter().zip(g.buttons).enumerate() {
        let use_it = k == 3;
        let fill = if use_it && *live { t.accent } else { t.surface };
        cx.canvas.fill_round_rect(r, t.round(10), fill);
        let ink = match (use_it, *live) {
            (true, true) => t.ink_on(t.accent),
            (_, true) => t.text,
            (_, false) => t.text_faint,
        };
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            r,
            Align::Center,
            ink,
            name,
        );
    }
}

/// Where each chip is on the strip's bars (`bars` is the strip's width
/// right of its "chords" label).
pub fn chip_rects(bars: Rect, s: &ChordStrip) -> Vec<Rect> {
    let w = bars.w as f32;
    s.chips
        .iter()
        .map(|c| {
            let x0 = bars.x + (c.from.clamp(0.0, 1.0) * w).round() as i32;
            let x1 = bars.x + (c.to.clamp(0.0, 1.0) * w).round() as i32;
            Rect::new(x0 + 2, bars.y + 16, (x1 - x0 - 4).max(1), bars.h - 19)
        })
        .collect()
}

/// A touch on the strip: a chip, or (with no chips, "no chords yet") a
/// bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripHit {
    Chip(usize),
    Bar(u32),
}

/// What a touch at (`x`, `y`) on the strip's bars is, if anything.
pub fn strip_hit(bars: Rect, s: &ChordStrip, x: i32, y: i32) -> Option<StripHit> {
    if !bars.contains(x, y) {
        return None;
    }
    if s.chips.is_empty() {
        let n = s.bars.max(1);
        let b = ((x - bars.x) as i64 * i64::from(n) / i64::from(bars.w.max(1))) as u32;
        return Some(StripHit::Bar(b.min(n - 1)));
    }
    // The chip under the finger; between two, the nearer.
    let rects = chip_rects(bars, s);
    rects
        .iter()
        .enumerate()
        .filter(|(_, r)| x >= r.x - 3 && x < r.right() + 3)
        .min_by_key(|(_, r)| (r.x + r.w / 2 - x).abs())
        .map(|(k, _)| StripHit::Chip(k))
}

/// Draw the strip: "chords" in `label` (the row numbers' column), and in
/// `bars` the row's bars with their chips and the play line.
pub fn paint_strip(cx: &mut PaintCx, label: Rect, bars: Rect, s: &ChordStrip) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(label, t.round(9), t.bg);
    cx.text(
        FontId::Sans,
        t.size_small - 3.0,
        label,
        Align::Center,
        t.text_dim,
        "chords",
    );
    cx.canvas.fill_round_rect(bars, t.round(9), t.bg);
    let n = s.bars.max(1);
    let px = t.size_small - 5.0;
    for b in 0..n {
        let x = bars.x + (b as i64 * bars.w as i64 / i64::from(n)) as i32;
        let next = bars.x + ((b + 1) as i64 * bars.w as i64 / i64::from(n)) as i32;
        if b > 0 {
            cx.canvas
                .fill_rect(Rect::new(x, bars.y + 4, 1, 10), t.outline);
        }
        if next - x >= 18 {
            cx.text(
                FontId::Sans,
                px,
                Rect::new(x + 6, bars.y + 1, next - x - 8, 14),
                Align::Left,
                t.text_faint,
                &format!("{}", b + 1),
            );
        }
    }
    if s.chips.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            Rect::new(bars.x + 10, bars.y + 14, bars.w - 20, bars.h - 16),
            Align::Left,
            t.text_faint,
            "no chords yet: tap a bar to set one",
        );
    }
    for (k, (c, r)) in s.chips.iter().zip(chip_rects(bars, s)).enumerate() {
        paint_chip(cx, r, c, s.open == Some(k));
    }
    if let Some(p) = s.play {
        let x = bars.x + (p.clamp(0.0, 1.0) * bars.w as f32).round() as i32;
        cx.canvas
            .fill_rect(Rect::new(x - 1, bars.y, 2, bars.h), t.good);
    }
}

fn paint_chip(cx: &mut PaintCx, r: Rect, c: &StripChip, open: bool) {
    let t = cx.theme.clone();
    let (ink, under) = match c.look {
        ChipLook::Heard => {
            cx.canvas.fill_round_rect(r, t.round(8), t.raised);
            (t.text, t.raised)
        }
        ChipLook::Suggested => {
            dashed(cx, r, t.text_faint);
            (t.text_dim, t.bg)
        }
        ChipLook::Yours => {
            let fill = t.bg.mix(t.accent, 0.18);
            cx.canvas.fill_round_rect(r, t.round(8), fill);
            cx.canvas.stroke_round_rect(r, t.round(8), 2, t.accent);
            (t.readable(t.accent, fill), fill)
        }
    };
    if open {
        cx.canvas
            .stroke_round_rect(r.inset(-4), t.round(10), 2, t.accent);
    }
    let words = Rect::new(r.x + 8, r.y, (r.w - 10).max(0), r.h);
    let w = cx.text(
        FontId::Bold,
        t.size_small - 1.0,
        words,
        Align::Left,
        t.readable(ink, under),
        &c.name,
    );
    if !c.word.is_empty() {
        let px = t.size_small - 3.0;
        let room = words.w - w - 6;
        if cx.measure(FontId::Sans, px, &c.word) <= room {
            cx.text(
                FontId::Sans,
                px,
                Rect::new(words.x + w + 6, words.y, room, words.h),
                Align::Left,
                t.text_dim,
                &c.word,
            );
        }
    }
}

/// A dashed outline round `r`, 1.5 px as the boards draw it: two 1 px
/// passes.
pub fn dashed(cx: &mut PaintCx, r: Rect, color: Color) {
    dash_once(cx, r, color);
    dash_once(cx, r.inset(1), color.mix(cx.theme.bg, 0.5));
}

fn dash_once(cx: &mut PaintCx, r: Rect, color: Color) {
    let (on, off, inset) = (6, 4, 6);
    let mut x = r.x + inset;
    while x < r.right() - inset {
        let w = on.min(r.right() - inset - x);
        cx.canvas.fill_rect(Rect::new(x, r.y, w, 1), color);
        cx.canvas
            .fill_rect(Rect::new(x, r.bottom() - 1, w, 1), color);
        x += on + off;
    }
    let mut y = r.y + inset;
    while y < r.bottom() - inset {
        let h = on.min(r.bottom() - inset - y);
        cx.canvas.fill_rect(Rect::new(r.x, y, 1, h), color);
        cx.canvas
            .fill_rect(Rect::new(r.right() - 1, y, 1, h), color);
        y += on + off;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(n: usize) -> ChordStrip {
        ChordStrip {
            row: 4,
            bars: 8,
            chips: (0..n)
                .map(|k| StripChip {
                    from: k as f32 / n as f32,
                    to: (k + 1) as f32 / n as f32,
                    name: format!("C{k}"),
                    look: ChipLook::Heard,
                    word: String::new(),
                })
                .collect(),
            play: None,
            open: None,
        }
    }

    #[test]
    fn a_tap_finds_the_chip_or_with_none_the_bar() {
        let bars = Rect::new(100, 500, 800, 47);
        let s = strip(4);
        let rects = chip_rects(bars, &s);
        assert_eq!(rects.len(), 4);
        assert!(rects.windows(2).all(|w| w[0].right() < w[1].x));
        let mid = |r: Rect| (r.x + r.w / 2, r.y + r.h / 2);
        for (k, r) in rects.iter().enumerate() {
            let (x, y) = mid(*r);
            assert_eq!(strip_hit(bars, &s, x, y), Some(StripHit::Chip(k)));
        }
        assert_eq!(strip_hit(bars, &s, 50, 520), None, "off the strip");
        let empty = strip(0);
        assert_eq!(
            strip_hit(bars, &empty, 100 + 350, 520),
            Some(StripHit::Bar(3))
        );
        assert_eq!(
            strip_hit(bars, &empty, 100 + 799, 520),
            Some(StripHit::Bar(7))
        );
    }

    #[test]
    fn the_picker_opens_under_its_chip_inside_the_grid_or_above_it() {
        let area = Rect::new(12, 128, 854, 610);
        let chip = Rect::new(380, 300, 90, 28);
        let g = picker_geo(chip, area);
        assert!(!g.above);
        assert_eq!(g.panel.y, chip.bottom() + 10);
        assert!(area.intersect(g.panel) == g.panel);
        assert!(g.arrow_x >= chip.x && g.arrow_x <= chip.right());
        // Every part apart and inside the panel.
        let mut parts: Vec<Rect> = g.roots.to_vec();
        parts.extend(g.kinds);
        parts.extend(g.buttons);
        parts.push(g.close);
        for (i, a) in parts.iter().enumerate() {
            assert!(g.panel.intersect(*a) == *a, "{a:?}");
            for b in &parts[i + 1..] {
                assert!(
                    a.intersect(*b).w <= 0 || a.intersect(*b).h <= 0,
                    "{a:?} {b:?}"
                );
            }
        }
        let mid = |r: Rect| (r.x + r.w / 2, r.y + r.h / 2);
        let (x, y) = mid(g.roots[8]);
        assert_eq!(picker_hit(&g, x, y), PICK_ROOT + 8);
        let (x, y) = mid(g.kinds[2]);
        assert_eq!(picker_hit(&g, x, y), PICK_KIND + 2);
        let (x, y) = mid(g.buttons[3]);
        assert_eq!(picker_hit(&g, x, y), PICK_USE);
        let (x, y) = mid(g.close);
        assert_eq!(picker_hit(&g, x, y), PICK_CLOSE);
        assert_eq!(picker_hit(&g, area.x + 2, area.y + 2), PICK_DISMISS);
        // A chip near the bottom: the picker opens above it.
        let low = Rect::new(700, 690, 90, 28);
        let g = picker_geo(low, area);
        assert!(g.above && g.panel.bottom() < low.y);
        assert!(g.panel.right() <= area.right());
    }

    #[test]
    fn use_is_named_for_the_chord_picked() {
        let v = PickerView {
            picked: "Ab7".into(),
            can_use: true,
            ..PickerView::default()
        };
        let b = picker_buttons(&v);
        assert_eq!(b[3], ("Use Ab7".to_string(), true));
        assert!(!b[0].1 && !b[1].1 && !b[2].1);
    }
}
