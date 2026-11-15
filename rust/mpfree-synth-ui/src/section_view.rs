//! The section widget: a page's controls in labelled sections
//! ([`crate::sections`]), each in its job's colour, its controls sized by
//! how much they matter and drawn as what they are.
//!
//! - A continuous value is a knob (a hero, main, standard or trim size).
//!   An envelope's stages and a mix's levels are faders; an organ's
//!   drawbars are drawbars.
//! - A choice of a few short names is a row of buttons (two rows when one
//!   is too narrow); a longer one is a chip with arrows.
//! - A small whole number (an octave, a voice count) is a stepper.
//! - A switch is a pill, lit when on.
//! - In a section of routing rows (a pressure slot's target and depth),
//!   two controls share a line and a knob becomes a slider.
//!
//! A finger down on a control selects it: `Select(i)`, `i` the parameter's
//! index in the node's list. Dragging a knob, fader, drawbar or slider
//! gives `Value(pos)` (0 to 1) for it. Lifting on a button, an arrow, a
//! stepper's - or + or a pill gives `Step { index: i, delta }`. A tap on a
//! section's name when it links elsewhere, or on a box of its drawing (an
//! FM operator, a drum pad), gives `Tab(n)`, `n` the link's number the
//! panel gave it.
//!
//! The same placement ([`SectionView::geometry`]) serves painting,
//! touches and the test targets, so they always agree.

use crate::draw::Mask;
use crate::sections::{Job, Tier};
use crate::widgets::{knob, node_rect, paint_plot, themed, Card, QLINK_COLORS};
use mpfree_ui::layout;
use mpfree_ui::text::FontId;
use mpfree_ui::{Align, Color, EventKind, PaintCx, Rect, Target, Theme, Touch, Widget};

/// What a control is drawn as.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// A knob at `pos` (0 to 1); a bipolar one fills from the top.
    Knob { pos: f32, bipolar: bool },
    /// Named choices, `index` chosen: buttons when they fit, else a chip
    /// with arrows.
    Choice { names: Vec<String>, index: usize },
    /// A small whole number with - and +.
    Stepper,
    /// On or off: a pill.
    Toggle(bool),
    /// A vertical fader; a bipolar one fills from the middle.
    Fader { pos: f32, bipolar: bool },
    /// An organ drawbar pulled down to `pos`.
    Drawbar { pos: f32 },
    /// A horizontal slider (in routing rows).
    Slider { pos: f32, bipolar: bool },
    /// A live value the node reports.
    Meter,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// The parameter's index in the node's list: what events carry.
    pub index: usize,
    pub label: String,
    pub value: String,
    pub unit: String,
    pub kind: Kind,
    pub tier: Tier,
    /// Q-Link 1 to 4 turns it (0 to 3): a badge in that Q-Link's colour.
    pub qlink: Option<usize>,
    /// It does nothing as the sound is set (the section says why).
    pub dim: bool,
    /// A fill of its own over the job's (a drawbar's family), written as
    /// the default theme's colour.
    pub tint: Option<Color>,
}

/// One section as the panel fills it in.
#[derive(Clone, Debug)]
pub struct SectionBox {
    /// Its name in [`SectionView::spec`].
    pub key: String,
    pub title: String,
    pub job: Job,
    /// Beside the name: a note, the drawing's caption, or why a control
    /// is dim.
    pub note: String,
    /// `Tab(n)` for a tap on the name ("Open ›" shows).
    pub link: Option<usize>,
    /// The drawings, side by side in the section's well.
    pub cards: Vec<Card>,
    /// `Tab(n)` for a tap on box `k` of the drawings (counted across them).
    pub node_links: Vec<usize>,
    pub controls: Vec<Control>,
    /// Routing rows: two controls to a line.
    pub rows: bool,
    /// A toggle in the heading that turns the whole section on and off (a
    /// stage of a channel strip): while it is off every control is dim.
    pub switch: Option<Control>,
}

impl SectionBox {
    pub fn new(key: &str, title: &str, job: Job) -> SectionBox {
        SectionBox {
            key: key.to_string(),
            title: title.to_string(),
            job,
            note: String::new(),
            link: None,
            cards: Vec::new(),
            node_links: Vec::new(),
            controls: Vec::new(),
            rows: false,
            switch: None,
        }
    }

    /// The section's switch is there and off.
    pub fn switched_off(&self) -> bool {
        matches!(
            self.switch,
            Some(Control {
                kind: Kind::Toggle(false),
                ..
            })
        )
    }
}

/// Where a control's parts lie.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CtlGeom {
    /// The whole control: what a finger picks and the selection outlines.
    pub cell: Rect,
    /// A knob's centre and diameter.
    pub dial: Option<(f32, f32, f32)>,
    /// A fader's, drawbar's or slider's track.
    pub track: Rect,
    /// A choice's buttons, or its or a stepper's two arrows (less, more).
    pub parts: Vec<Rect>,
    /// The choice shows as a chip with arrows.
    pub chip: bool,
    /// The dial's name and value sit beside it, left-aligned (a short
    /// section).
    pub inline: bool,
    pub label: Rect,
    pub value: Rect,
}

/// Where a section's parts lie.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Geom {
    pub rect: Rect,
    /// The name, the note and "Open ›" (what a tap on a link takes).
    pub head: Rect,
    /// The switch's pill at the right of the heading.
    pub switch: Option<Rect>,
    /// The drawings' well, all of them ([`wells`] splits it).
    pub drawing: Option<Rect>,
    pub controls: Vec<CtlGeom>,
}

/// `n` drawings' wells side by side in `d`.
pub fn wells(d: Rect, n: usize) -> Vec<Rect> {
    let n = n.max(1) as i32;
    let w = (d.w - GAP * (n - 1)) / n;
    (0..n)
        .map(|k| Rect::new(d.x + k * (w + GAP), d.y, w, d.h))
        .collect()
}

/// Width of a section switch's pill.
const SWITCH_W: i32 = 52;

struct Drag {
    finger: u64,
    index: usize,
    x0: i32,
    y0: i32,
    pos0: f32,
    /// Pixels for the whole range; along x when `across`, else up the
    /// screen (down for a drawbar, `down`).
    px: f32,
    across: bool,
    down: bool,
}

pub struct SectionView {
    /// The sections' layout, over their keys.
    pub spec: String,
    pub sections: Vec<SectionBox>,
    /// The selected parameter (index in the node's list).
    pub selected: Option<usize>,
    /// The wheel's push opened the selected control.
    pub editing: bool,
    drag: Option<Drag>,
}

/// Space between sections, and inside one.
pub const SECTION_GAP: i32 = 12;
const PAD: i32 = 14;
const HEAD_H: i32 = 24;
const GAP: i32 = 10;
const BAR_H: i32 = 44;
const LABEL_H: i32 = 16;
const LABEL_PX: f32 = 13.0;
/// Pixels of drag for a knob's whole range.
pub const DRAG_PX: f32 = 320.0;

/// A knob's diameter by tier: the most and the least it shrinks to.
fn dial_size(tier: Tier) -> (f32, f32) {
    match tier {
        Tier::Hero => (112.0, 88.0),
        Tier::Main => (80.0, 62.0),
        Tier::Std => (60.0, 48.0),
        Tier::Trim => (48.0, 40.0),
    }
}

/// A value's text size by tier.
fn value_px(tier: Tier) -> f32 {
    match tier {
        Tier::Hero => 26.0,
        Tier::Main => 21.0,
        Tier::Std => 17.0,
        Tier::Trim => 15.0,
    }
}

/// A rough width for text, without the fonts (touches have none): about
/// 0.55 em a character.
fn est(s: &str, px: f32) -> i32 {
    (s.chars().count() as f32 * px * 0.56).ceil() as i32
}

/// The width a choice's buttons need each.
fn button_w(names: &[String]) -> i32 {
    names
        .iter()
        .map(|n| est(n, 14.0) + 20)
        .max()
        .unwrap_or(44)
        .max(44)
}

/// How a choice lies in width `w`: rows of buttons (1 or 2), or 0 for a
/// chip.
fn choice_rows(names: &[String], w: i32) -> i32 {
    let n = names.len() as i32;
    let bw = button_w(names);
    if n == 0 || n > 8 {
        0
    } else if n * bw + 6 * (n - 1) <= w {
        1
    } else if n <= 6 && (n + 1) / 2 * bw + 6 * ((n + 1) / 2 - 1) <= w {
        2
    } else {
        0
    }
}

impl SectionView {
    pub fn new() -> SectionView {
        SectionView {
            spec: String::new(),
            sections: Vec::new(),
            selected: None,
            editing: false,
            drag: None,
        }
    }

    /// Each section's rectangle in `r`, by the layout spec (the whole of
    /// `r` for a spec that won't parse, or names no section).
    pub fn section_rects(&self, r: Rect) -> Vec<Rect> {
        let slots = layout::parse(&self.spec)
            .map(|n| layout::solve(&n, r, SECTION_GAP))
            .unwrap_or_default();
        self.sections
            .iter()
            .map(|s| {
                slots
                    .iter()
                    .find(|(k, _)| *k == s.key)
                    .map_or(Rect::new(r.x, r.y, 0, 0), |(_, rr)| *rr)
            })
            .collect()
    }

    /// Where everything lies, section by section.
    pub fn geometry(&self, r: Rect) -> Vec<Geom> {
        self.sections
            .iter()
            .zip(self.section_rects(r))
            .map(|(s, sr)| place(s, sr))
            .collect()
    }

    /// Where parameter `index`'s control lies in `r`, with its control
    /// (the first, if a page shows it twice).
    pub fn control_geom(&self, r: Rect, index: usize) -> Option<(&Control, CtlGeom)> {
        self.sections
            .iter()
            .zip(self.geometry(r))
            .find_map(|(s, g)| {
                let k = s.controls.iter().position(|c| c.index == index)?;
                Some((&s.controls[k], g.controls[k].clone()))
            })
    }

    /// The section with key `key`, and where it lies in `r`.
    pub fn section(&self, r: Rect, key: &str) -> Option<(&SectionBox, Geom)> {
        self.sections
            .iter()
            .zip(self.geometry(r))
            .find(|(s, _)| s.key == key)
    }

    /// The parameter of the section switch under (x, y), counting a finger
    /// a little above or below the pill.
    fn switch_at(&self, geo: &[Geom], x: i32, y: i32) -> Option<usize> {
        self.sections.iter().zip(geo).find_map(|(s, g)| {
            let r = g.switch?;
            let hit = Rect::new(r.x - 4, r.y - 10, r.w + 8, r.h + 20);
            hit.contains(x, y)
                .then(|| s.switch.as_ref().map(|c| c.index))?
        })
    }

    /// The control under (x, y): its section and its place there.
    fn hit(&self, geo: &[Geom], x: i32, y: i32) -> Option<(usize, usize)> {
        for (si, g) in geo.iter().enumerate() {
            if !g.rect.contains(x, y) {
                continue;
            }
            for (ci, c) in g.controls.iter().enumerate() {
                if c.cell.contains(x, y) {
                    return Some((si, ci));
                }
            }
        }
        None
    }
}

impl Default for SectionView {
    fn default() -> SectionView {
        SectionView::new()
    }
}

/// A line of controls: their places in the section's list and widths.
struct Line {
    items: Vec<(usize, i32, i32)>,
    h: i32,
}

/// Lays one section out in `r`.
pub fn place(s: &SectionBox, r: Rect) -> Geom {
    let mut g = Geom {
        rect: r,
        controls: vec![CtlGeom::default(); s.controls.len()],
        ..Geom::default()
    };
    if r.w <= 2 * PAD || r.h <= 2 * PAD {
        return g;
    }
    let inner = r.inset(PAD);
    let (mut head, body) = inner.take_top(HEAD_H);
    if s.switch.is_some() {
        g.switch = Some(Rect::new(head.right() - SWITCH_W, head.y, SWITCH_W, head.h));
        head.w -= SWITCH_W + 12;
    }
    g.head = head;
    let body = Rect::new(body.x, body.y + GAP, body.w, (body.h - GAP).max(0));
    if s.controls.is_empty() {
        g.drawing = (!s.cards.is_empty()).then_some(body);
        return g;
    }
    if s.rows {
        let mut y = body.y;
        if !s.cards.is_empty() && body.h > 260 {
            let dh = (body.h / 3).min(150);
            g.drawing = Some(Rect::new(body.x, y, body.w, dh));
            y += dh + GAP;
        }
        place_rows(s, &mut g, Rect::new(body.x, y, body.w, body.bottom() - y));
        return g;
    }
    let is_bar = |c: &Control| matches!(c.kind, Kind::Choice { .. } | Kind::Toggle(_));
    let is_fader = |c: &Control| matches!(c.kind, Kind::Fader { .. } | Kind::Drawbar { .. });
    let bars: Vec<usize> = (0..s.controls.len())
        .filter(|&k| is_bar(&s.controls[k]))
        .collect();
    let faders: Vec<usize> = (0..s.controls.len())
        .filter(|&k| is_fader(&s.controls[k]))
        .collect();
    let mut dials: Vec<usize> = (0..s.controls.len())
        .filter(|&k| !is_bar(&s.controls[k]) && !is_fader(&s.controls[k]))
        .collect();
    // Heroes first.
    dials.sort_by_key(|&k| std::cmp::Reverse(s.controls[k].tier == Tier::Hero));
    let bar_lines = bar_lines(s, &bars, body.w);
    let bars_h = lines_h(&bar_lines);
    let fader_min = if faders.is_empty() { 0 } else { 150 };
    let (_, dial_min) = dial_lines(s, &dials, body.w, Fit::Inline);
    let (_, dial_pref) = dial_lines(s, &dials, body.w, Fit::Stacked(1.0));
    let regions = [bars_h > 0, !faders.is_empty(), !dials.is_empty()]
        .iter()
        .filter(|&&b| b)
        .count() as i32;
    let fixed = bars_h + GAP * (regions - 1).max(0);
    let mut y = body.y;
    if !s.cards.is_empty() {
        let need = fixed + fader_min + dial_min + GAP;
        let want = ((body.h as f32 * 0.38) as i32).clamp(72, 210);
        let dh = want.min(body.h - need);
        if dh >= 56 {
            g.drawing = Some(Rect::new(body.x, y, body.w, dh));
            y += dh + GAP;
        }
    }
    let left = body.bottom() - y;
    // Bars, then faders, then dials.
    let mut at = y;
    for line in &bar_lines {
        let mut x = body.x;
        for &(k, w, h) in &line.items {
            let cell = Rect::new(x, at, w, h);
            g.controls[k] = bar_geom(&s.controls[k], cell, &s.title);
            x += w + 12;
        }
        at += line.h + GAP;
    }
    if bars_h > 0 {
        at = y + bars_h + GAP;
    }
    let room = body.bottom() - at;
    let fader_h = if faders.is_empty() {
        0
    } else if dials.is_empty() {
        room
    } else {
        (room - GAP - dial_pref.min(room - GAP - fader_min)).max(fader_min.min(room))
    };
    if !faders.is_empty() {
        place_faders(s, &mut g, &faders, Rect::new(body.x, at, body.w, fader_h));
        at += fader_h + GAP;
    }
    if !dials.is_empty() {
        let area = Rect::new(body.x, at, body.w, (body.bottom() - at).max(0));
        place_dials(s, &mut g, &dials, area);
    }
    let _ = left;
    g
}

fn lines_h(lines: &[Line]) -> i32 {
    if lines.is_empty() {
        0
    } else {
        lines.iter().map(|l| l.h).sum::<i32>() + GAP * (lines.len() as i32 - 1)
    }
}

/// Whether a control shows its label over it (not when it is the
/// section's only choice and says what the section's name says).
fn shows_label(c: &Control, section: &str) -> bool {
    !c.label.is_empty() && !c.label.eq_ignore_ascii_case(section)
}

/// The choices and switches as lines: a choice takes a line (two share one
/// when both fit), switches flow along one.
fn bar_lines(s: &SectionBox, bars: &[usize], w: i32) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut pills: Option<Line> = None;
    let flush = |pills: &mut Option<Line>, lines: &mut Vec<Line>| {
        if let Some(l) = pills.take() {
            lines.push(l);
        }
    };
    for &k in bars {
        let c = &s.controls[k];
        match &c.kind {
            Kind::Toggle(_) => {
                let pw = (est(&c.label, 14.0) + 44).clamp(88, w);
                let fits = pills
                    .as_ref()
                    .is_some_and(|l| l.items.iter().map(|i| i.1 + 8).sum::<i32>() + pw <= w);
                if !fits {
                    flush(&mut pills, &mut lines);
                    pills = Some(Line {
                        items: Vec::new(),
                        h: BAR_H,
                    });
                }
                pills.as_mut().unwrap().items.push((k, pw, BAR_H));
            }
            Kind::Choice { names, .. } => {
                flush(&mut pills, &mut lines);
                let label = if shows_label(c, &s.title) {
                    LABEL_H + 4
                } else {
                    0
                };
                // Two choices share a line when each gets half and still
                // fits on one row of buttons (or as a chip).
                let half = (w - 12) / 2;
                let rows_half = choice_rows(names, half);
                let can_share = half >= 170 && rows_half <= 1;
                let prev_shares = lines.last().is_some_and(|l| {
                    l.items.len() == 1
                        && l.items[0].1 == half
                        && matches!(s.controls[l.items[0].0].kind, Kind::Choice { .. })
                });
                if can_share && prev_shares {
                    let l = lines.last_mut().unwrap();
                    let h = label + BAR_H;
                    l.h = l.h.max(h);
                    l.items.push((k, half, h));
                    continue;
                }
                let rows = choice_rows(names, w).max(1);
                let h = label + rows * BAR_H + (rows - 1) * 6;
                // Room for a second choice beside it later.
                let width = if can_share && next_choice_shares(s, bars, k, half) {
                    half
                } else {
                    w
                };
                let h = if width == half { label + BAR_H } else { h };
                lines.push(Line {
                    items: vec![(k, width, h)],
                    h,
                });
            }
            _ => {}
        }
    }
    flush(&mut pills, &mut lines);
    lines
}

/// Whether the choice after `k` among `bars` can share its line.
fn next_choice_shares(s: &SectionBox, bars: &[usize], k: usize, half: i32) -> bool {
    let at = bars.iter().position(|&b| b == k).unwrap_or(0);
    match bars.get(at + 1).map(|&b| &s.controls[b].kind) {
        Some(Kind::Choice { names, .. }) => {
            let mine = match &s.controls[k].kind {
                Kind::Choice { names, .. } => names,
                _ => return false,
            };
            choice_rows(names, half) <= 1 && choice_rows(mine, half) <= 1
        }
        _ => false,
    }
}

fn bar_geom(c: &Control, cell: Rect, section: &str) -> CtlGeom {
    let mut g = CtlGeom {
        cell,
        ..CtlGeom::default()
    };
    match &c.kind {
        Kind::Toggle(_) => {
            g.label = cell;
        }
        Kind::Choice { names, .. } => {
            let mut area = cell;
            if shows_label(c, section) {
                let (l, rest) = cell.take_top(LABEL_H);
                g.label = l;
                area = Rect::new(rest.x, rest.y + 4, rest.w, rest.h - 4);
            }
            let rows = choice_rows(names, area.w);
            if rows == 0 {
                g.chip = true;
                let aw = 52.min(area.w / 4);
                let h = BAR_H.min(area.h);
                g.parts = vec![
                    Rect::new(area.x, area.y, aw, h),
                    Rect::new(area.right() - aw, area.y, aw, h),
                ];
                g.value = Rect::new(area.x + aw + 6, area.y, area.w - 2 * aw - 12, h);
            } else {
                let n = names.len() as i32;
                let cols = if rows == 1 { n } else { (n + 1) / 2 };
                let rows = if area.h >= 2 * BAR_H + 6 { rows } else { 1 };
                let cols = if rows == 1 { n } else { cols };
                let bw = (area.w - 6 * (cols - 1)) / cols.max(1);
                let bh = (area.h - 6 * (rows - 1)) / rows.max(1);
                g.parts = (0..n)
                    .map(|k| {
                        Rect::new(
                            area.x + (k % cols) * (bw + 6),
                            area.y + (k / cols) * (bh + 6),
                            bw,
                            bh,
                        )
                    })
                    .collect();
            }
        }
        _ => {}
    }
    g
}

/// How dials lie: stacked (the knob over its name and value) or inline
/// (the knob beside them, for a short section).
#[derive(Clone, Copy, PartialEq)]
enum Fit {
    Stacked(f32),
    Inline,
}

/// The dials (knobs, steppers, meters) as lines, and their height.
/// Stacked at `scale` 1 is the tiers' largest, 0 their least, below 0
/// smaller still.
fn dial_lines(s: &SectionBox, dials: &[usize], w: i32, fit: Fit) -> (Vec<Line>, i32) {
    let mut lines: Vec<Line> = Vec::new();
    let mut hero_line = true;
    for &k in dials {
        let c = &s.controls[k];
        let (cw, ch) = dial_cell(c, fit);
        let cw = cw.min(w);
        let hero = c.tier == Tier::Hero && fit != Fit::Inline;
        let new_line = match lines.last() {
            None => true,
            Some(l) => {
                (hero_line && !hero) || l.items.iter().map(|i| i.1 + 8).sum::<i32>() + cw > w
            }
        };
        if new_line {
            hero_line = hero;
            lines.push(Line {
                items: Vec::new(),
                h: 0,
            });
        }
        let l = lines.last_mut().unwrap();
        l.items.push((k, cw, ch));
        l.h = l.h.max(ch);
    }
    let h = lines_h(&lines);
    (lines, h)
}

/// A stepper's cell is wide enough for a Q-Link badge beside its name.
const STEPPER_W: i32 = 132;

/// Room for a meter's "live" tag (12 px Sans) at the right of its name,
/// when the name sits beside the value rather than over it.
const LIVE_W: i32 = 28;

/// A knob's diameter beside its name, by tier.
fn inline_d(tier: Tier) -> f32 {
    match tier {
        Tier::Hero => 64.0,
        Tier::Main => 54.0,
        Tier::Std => 46.0,
        Tier::Trim => 40.0,
    }
}

/// A dial's cell: its width and height.
fn dial_cell(c: &Control, fit: Fit) -> (i32, i32) {
    let vh = value_px(c.tier) as i32;
    let label_w = est(&c.label, LABEL_PX) + 8;
    let value_w = est(&format!("{} {}", c.value, c.unit), vh as f32) + 8;
    let text_w = label_w.max(value_w);
    match (c.kind.clone(), fit) {
        (Kind::Stepper, Fit::Inline) => (text_w.max(64) + 8 + 86, (LABEL_H + 2 + vh).max(40)),
        (Kind::Stepper, _) => (
            (label_w + 64).max(value_w).clamp(STEPPER_W, 180),
            LABEL_H + 4 + vh + 6 + 40,
        ),
        (Kind::Meter, Fit::Inline) => (
            (label_w + 6 + LIVE_W).max(value_w).clamp(64, 200),
            LABEL_H + 2 + vh,
        ),
        (Kind::Meter, _) => (text_w.clamp(96, 160), LABEL_H + 4 + vh + 18),
        (_, Fit::Inline) => {
            let d = inline_d(c.tier);
            let w = d as i32 + 10 + text_w.max(56);
            (w.min(220), (d as i32).max(LABEL_H + 2 + vh))
        }
        (_, Fit::Stacked(scale)) => {
            let d = knob_d(c.tier, scale);
            let w = (d as i32 + 16).max(label_w).max(value_w).clamp(84, 168);
            (w, d as i32 + 6 + LABEL_H + 2 + vh + 2)
        }
    }
}

fn knob_d(tier: Tier, scale: f32) -> f32 {
    let (big, small) = dial_size(tier);
    (small + (big - small) * scale).max(30.0)
}

fn place_dials(s: &SectionBox, g: &mut Geom, dials: &[usize], area: Rect) {
    // Stacked as large as fits; a section too short for stacked dials at
    // their least puts each knob beside its name; failing that, stacked and
    // smaller still.
    let mut tries: Vec<Fit> = [1.0, 0.75, 0.5, 0.25, 0.0]
        .into_iter()
        .map(Fit::Stacked)
        .collect();
    tries.push(Fit::Inline);
    tries.extend([-0.25, -0.5, -0.75, -1.0].into_iter().map(Fit::Stacked));
    let mut chosen = None;
    for fit in tries {
        let (lines, h) = dial_lines(s, dials, area.w, fit);
        let ok = h <= area.h;
        chosen = Some((lines, h, fit));
        if ok {
            break;
        }
    }
    let Some((lines, h, fit)) = chosen else {
        return;
    };
    let n = lines.len() as i32;
    let extra = (area.h - h).max(0);
    let inter = if n > 1 { (extra / (n + 1)).min(28) } else { 0 };
    let mut y = area.y + (extra - inter * (n - 1)) / 2;
    for line in &lines {
        let used: i32 = line.items.iter().map(|i| i.1).sum();
        let m = line.items.len() as i32;
        let spare = (area.w - used).max(0);
        let gap = if m > 1 {
            (spare / (m + 1)).clamp(8, 56)
        } else {
            0
        };
        let mut x = area.x + (area.w - used - gap * (m - 1)) / 2;
        for &(k, cw, ch) in &line.items {
            let c = &s.controls[k];
            let cell = Rect::new(x, y + (line.h - ch) / 2, cw, ch);
            g.controls[k] = dial_geom(c, cell, fit);
            x += cw + gap;
        }
        y += line.h + GAP + inter;
    }
}

fn dial_geom(c: &Control, cell: Rect, fit: Fit) -> CtlGeom {
    let vh = value_px(c.tier) as i32;
    let mut g = CtlGeom {
        cell,
        inline: fit == Fit::Inline,
        ..CtlGeom::default()
    };
    let mid = cell.y + cell.h / 2;
    match (&c.kind, fit) {
        (Kind::Stepper, Fit::Inline) => {
            let bx = cell.right() - 86;
            g.label = Rect::new(cell.x, mid - LABEL_H - 1, bx - cell.x - 8, LABEL_H);
            g.value = Rect::new(cell.x, mid + 1, bx - cell.x - 8, vh);
            g.parts = vec![
                Rect::new(bx, mid - 20, 40, 40),
                Rect::new(bx + 46, mid - 20, 40, 40),
            ];
        }
        (Kind::Stepper, _) => {
            g.label = Rect::new(cell.x, cell.y, cell.w, LABEL_H);
            g.value = Rect::new(cell.x, cell.y + LABEL_H + 4, cell.w, vh);
            let by = cell.bottom() - 40;
            let bw = (cell.w - 8) / 2;
            g.parts = vec![
                Rect::new(cell.x, by, bw, 40),
                Rect::new(cell.right() - bw, by, bw, 40),
            ];
        }
        (Kind::Meter, Fit::Inline) => {
            g.label = Rect::new(cell.x, mid - LABEL_H - 1, cell.w, LABEL_H);
            g.value = Rect::new(cell.x, mid + 1, cell.w, vh);
        }
        (Kind::Meter, _) => {
            g.label = Rect::new(cell.x, cell.y, cell.w, LABEL_H);
            g.value = Rect::new(cell.x, cell.y + LABEL_H + 4, cell.w, vh);
        }
        (_, Fit::Inline) => {
            let d = inline_d(c.tier);
            g.dial = Some((cell.x as f32 + d / 2.0, mid as f32, d));
            let tx = cell.x + d as i32 + 10;
            g.label = Rect::new(tx, mid - LABEL_H - 1, cell.right() - tx, LABEL_H);
            g.value = Rect::new(tx, mid + 1, cell.right() - tx, vh);
        }
        (_, Fit::Stacked(scale)) => {
            let d = knob_d(c.tier, scale);
            let cx = cell.x as f32 + cell.w as f32 / 2.0;
            g.dial = Some((cx, cell.y as f32 + d / 2.0, d));
            let ly = cell.y + d as i32 + 6;
            g.label = Rect::new(cell.x - 4, ly, cell.w + 8, LABEL_H);
            g.value = Rect::new(cell.x - 4, ly + LABEL_H + 2, cell.w + 8, vh);
        }
    }
    g
}

fn place_faders(s: &SectionBox, g: &mut Geom, faders: &[usize], area: Rect) {
    let n = faders.len() as i32;
    let col = (area.w / n.max(1)).min(if n <= 4 { 96 } else { 84 });
    let x0 = area.x + (area.w - col * n) / 2;
    for (j, &k) in faders.iter().enumerate() {
        let c = &s.controls[k];
        let cell = Rect::new(x0 + j as i32 * col, area.y, col, area.h);
        let drawbar = matches!(c.kind, Kind::Drawbar { .. });
        let tw = if drawbar {
            (col - 22).clamp(16, 30)
        } else {
            10
        };
        let top = cell.y + LABEL_H + 8;
        let bottom = cell.bottom() - 22 - 6;
        g.controls[k] = CtlGeom {
            cell,
            track: Rect::new(cell.x + (col - tw) / 2, top, tw, (bottom - top).max(8)),
            label: Rect::new(cell.x, cell.y, cell.w, LABEL_H),
            value: Rect::new(cell.x, cell.bottom() - 22, cell.w, 22),
            ..CtlGeom::default()
        };
    }
}

/// Routing rows: two controls to a line, each its label over it.
fn place_rows(s: &SectionBox, g: &mut Geom, area: Rect) {
    let per = if area.w >= 360 { 2 } else { 1 };
    let lines = (s.controls.len() as i32 + per - 1) / per;
    let lh = (LABEL_H + 4 + BAR_H).min(area.h / lines.max(1));
    let extra = (area.h - lh * lines).max(0);
    let inter = (extra / (lines + 1)).min(18);
    let cw = (area.w - 12 * (per - 1)) / per;
    for (k, c) in s.controls.iter().enumerate() {
        let (row, colm) = (k as i32 / per, k as i32 % per);
        let cell = Rect::new(
            area.x + colm * (cw + 12),
            area.y + inter + row * (lh + inter),
            cw,
            lh,
        );
        let (l, rest) = cell.take_top(LABEL_H);
        let body = Rect::new(rest.x, rest.y + 4, rest.w, (rest.h - 4).max(0));
        let mut cg = CtlGeom {
            cell,
            label: l,
            ..CtlGeom::default()
        };
        match &c.kind {
            Kind::Choice { .. } => {
                let aw = 48.min(body.w / 4);
                cg.chip = true;
                cg.parts = vec![
                    Rect::new(body.x, body.y, aw, body.h),
                    Rect::new(body.right() - aw, body.y, aw, body.h),
                ];
                cg.value = Rect::new(body.x + aw + 6, body.y, body.w - 2 * aw - 12, body.h);
            }
            Kind::Slider { .. } => {
                let vw = 96.min(body.w / 3);
                cg.value = Rect::new(body.right() - vw, body.y, vw, body.h);
                let tw = body.w - vw - 12;
                cg.track = Rect::new(body.x + 10, body.y + body.h / 2 - 3, (tw - 20).max(8), 6);
            }
            Kind::Toggle(_) => {
                cg.label = body;
            }
            _ => {
                cg.value = body;
            }
        }
        g.controls[k] = cg;
    }
}

/// Theme colours for a section: its job's, a fill, the tint behind it.
struct Palette {
    job: Color,
    fill: Color,
    tint: Color,
    well: Color,
}

fn palette(t: &Theme, job: Job) -> Palette {
    let c = job.color(t);
    Palette {
        job: c,
        fill: job.fill(t),
        tint: t.surface.mix(c, 0.06),
        well: t.bg.mix(c, 0.05),
    }
}

/// A colour faded for a control that does nothing as set.
fn fade(c: Color, dim: bool) -> Color {
    if dim {
        c.with_alpha(c.a / 2)
    } else {
        c
    }
}

/// A drawing's colours in a section: the Q-Link colours become shades of
/// the section's own, so a page shows Q-Link colours only on its badges.
fn shade(t: &Theme, job: Color, c: Color) -> Color {
    let same = |a: Color| (a.r, a.g, a.b) == (c.r, c.g, c.b);
    match QLINK_COLORS.iter().position(|&q| same(q)) {
        Some(0) => job.with_alpha(c.a),
        Some(1) => job.mix(t.text, 0.55).with_alpha(c.a),
        Some(2) => job.mix(t.text, 0.3).with_alpha(c.a),
        Some(_) => job.mix(t.bg, 0.35).with_alpha(c.a),
        None => themed(t, c),
    }
}

impl Widget for SectionView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        if self.sections.is_empty() {
            cx.panel(r);
            cx.text(
                FontId::Sans,
                t.size_body,
                r.inset(PAD).take_top(40).0,
                Align::Left,
                t.text_faint,
                "no parameters",
            );
            return;
        }
        let geo = self.geometry(r);
        for (s, g) in self.sections.iter().zip(&geo) {
            if g.rect.w <= 0 || g.rect.h <= 0 {
                continue;
            }
            paint_section(cx, &t, s, g, self.selected, self.editing);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = Vec::new();
        let geo = self.geometry(r);
        for (s, g) in self.sections.iter().zip(&geo) {
            if let Some(l) = s.link {
                out.push(Target::tap("section-link", l, s.title.clone(), g.head));
            }
            if let (Some(c), Some(sr)) = (&s.switch, g.switch) {
                out.push(Target::tap("section-switch", c.index, s.title.clone(), sr));
            }
            if let Some(d) = g.drawing {
                for (k, b, label) in node_boxes(s, d) {
                    if let Some(&l) = s.node_links.get(k) {
                        out.push(Target::tap("node", l, label, b));
                    }
                }
            }
            for (c, cg) in s.controls.iter().zip(&g.controls) {
                let i = c.index;
                match &c.kind {
                    Kind::Knob { .. } => {
                        out.push(Target::drag("knob", i, c.label.clone(), cg.cell).towards(0, -40))
                    }
                    Kind::Fader { .. } => {
                        out.push(Target::drag("fader", i, c.label.clone(), cg.cell).towards(0, -40))
                    }
                    Kind::Drawbar { .. } => out
                        .push(Target::drag("drawbar", i, c.label.clone(), cg.cell).towards(0, 40)),
                    Kind::Slider { .. } => {
                        out.push(Target::drag("slider", i, c.label.clone(), cg.cell).towards(40, 0))
                    }
                    Kind::Choice { names, index } => {
                        if cg.chip {
                            for (k, pr) in cg.parts.iter().enumerate().take(2) {
                                let id = if k == 0 { "choice-less" } else { "choice-more" };
                                out.push(Target::tap(
                                    id,
                                    i,
                                    format!("{} {}", c.label, ["‹", "›"][k]),
                                    *pr,
                                ));
                            }
                        } else {
                            for (k, pr) in cg.parts.iter().enumerate() {
                                if k != *index {
                                    let name = names.get(k).cloned().unwrap_or_default();
                                    out.push(Target::tap(
                                        "choice",
                                        i * 16 + k,
                                        format!("{}: {name}", c.label),
                                        *pr,
                                    ));
                                }
                            }
                        }
                    }
                    Kind::Stepper => {
                        for (k, pr) in cg.parts.iter().enumerate().take(2) {
                            let id = if k == 0 { "step-less" } else { "step-more" };
                            out.push(Target::tap(
                                id,
                                i,
                                format!("{} {}", c.label, ["-", "+"][k]),
                                *pr,
                            ));
                        }
                    }
                    Kind::Toggle(_) => out.push(Target::tap("toggle", i, c.label.clone(), cg.cell)),
                    Kind::Meter => {}
                }
            }
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let geo = self.geometry(r);
                if let Some(i) = self.switch_at(&geo, x, y) {
                    self.selected = Some(i);
                    self.drag = None;
                    return Some(EventKind::Select(i));
                }
                if let Some((si, ci)) = self.hit(&geo, x, y) {
                    let c = &self.sections[si].controls[ci];
                    let cg = &geo[si].controls[ci];
                    self.selected = Some(c.index);
                    self.drag = match c.kind {
                        Kind::Knob { pos, .. } => Some(Drag {
                            finger,
                            index: c.index,
                            x0: x,
                            y0: y,
                            pos0: pos,
                            px: DRAG_PX,
                            across: false,
                            down: false,
                        }),
                        Kind::Fader { pos, .. } => Some(Drag {
                            finger,
                            index: c.index,
                            x0: x,
                            y0: y,
                            pos0: pos,
                            px: cg.track.h.max(40) as f32,
                            across: false,
                            down: false,
                        }),
                        Kind::Drawbar { pos } => Some(Drag {
                            finger,
                            index: c.index,
                            x0: x,
                            y0: y,
                            pos0: pos,
                            px: cg.track.h.max(40) as f32,
                            across: false,
                            down: true,
                        }),
                        Kind::Slider { pos, .. } => Some(Drag {
                            finger,
                            index: c.index,
                            x0: x,
                            y0: y,
                            pos0: pos,
                            px: cg.track.w.max(40) as f32,
                            across: true,
                            down: false,
                        }),
                        _ => None,
                    };
                    return Some(EventKind::Select(c.index));
                }
                // A box of a drawing, else a section's name or drawing
                // that links elsewhere.
                for (s, g) in self.sections.iter().zip(&geo) {
                    if !g.rect.contains(x, y) {
                        continue;
                    }
                    if let Some(d) = g.drawing {
                        let near = node_boxes(s, d)
                            .into_iter()
                            .filter(|(k, _, _)| s.node_links.get(*k).is_some())
                            .map(|(k, b, _)| {
                                let (nx, ny) = (b.x + b.w / 2, b.y + b.h / 2);
                                (k, ((nx - x) as f32).hypot((ny - y) as f32))
                            })
                            .filter(|&(_, dist)| dist <= 22.0)
                            .min_by(|a, b| a.1.total_cmp(&b.1));
                        if let Some((k, _)) = near {
                            return Some(EventKind::Tab(s.node_links[k]));
                        }
                    }
                    let on_link =
                        g.head.contains(x, y) || g.drawing.is_some_and(|d| d.contains(x, y));
                    if let (true, Some(l)) = (on_link, s.link) {
                        return Some(EventKind::Tab(l));
                    }
                }
                None
            }
            Touch::Move => {
                let d = self.drag.as_ref().filter(|d| d.finger == finger)?;
                let moved = if d.across {
                    (x - d.x0) as f32
                } else if d.down {
                    (y - d.y0) as f32
                } else {
                    (d.y0 - y) as f32
                };
                let pos = (d.pos0 + moved / d.px).clamp(0.0, 1.0);
                let index = d.index;
                let mut changed = false;
                for s in &mut self.sections {
                    for c in &mut s.controls {
                        if c.index != index {
                            continue;
                        }
                        if let Kind::Knob { pos: p, .. }
                        | Kind::Fader { pos: p, .. }
                        | Kind::Drawbar { pos: p }
                        | Kind::Slider { pos: p, .. } = &mut c.kind
                        {
                            if (*p - pos).abs() >= 1e-6 {
                                *p = pos;
                                changed = true;
                            }
                        }
                    }
                }
                if !changed {
                    return None;
                }
                self.selected = Some(index);
                Some(EventKind::Value(pos))
            }
            Touch::Up => {
                if self.drag.as_ref().is_some_and(|d| d.finger == finger) {
                    self.drag = None;
                    return None;
                }
                let geo = self.geometry(r);
                if let Some(i) = self.switch_at(&geo, x, y) {
                    if self.selected != Some(i) {
                        return None;
                    }
                    let s = self
                        .sections
                        .iter()
                        .find_map(|s| s.switch.as_ref().filter(|c| c.index == i))?;
                    let on = matches!(s.kind, Kind::Toggle(true));
                    return Some(EventKind::Step {
                        index: i,
                        delta: if on { -1 } else { 1 },
                    });
                }
                let (si, ci) = self.hit(&geo, x, y)?;
                let c = &self.sections[si].controls[ci];
                if self.selected != Some(c.index) {
                    return None;
                }
                let cg = &geo[si].controls[ci];
                let index = c.index;
                match &c.kind {
                    Kind::Choice { index: at, .. } => {
                        let k = cg.parts.iter().position(|p| p.contains(x, y))?;
                        let delta = if cg.chip {
                            if k == 0 {
                                -1
                            } else {
                                1
                            }
                        } else {
                            k as i32 - *at as i32
                        };
                        (delta != 0).then_some(EventKind::Step { index, delta })
                    }
                    Kind::Stepper => {
                        let k = cg.parts.iter().position(|p| p.contains(x, y))?;
                        Some(EventKind::Step {
                            index,
                            delta: if k == 0 { -1 } else { 1 },
                        })
                    }
                    Kind::Toggle(on) => Some(EventKind::Step {
                        index,
                        delta: if *on { -1 } else { 1 },
                    }),
                    _ => None,
                }
            }
            Touch::Cancel => {
                self.drag = None;
                None
            }
        }
    }
}

/// Every box of a section's drawings in well area `d`, counted across the
/// drawings, with where it lies and its label.
fn node_boxes(s: &SectionBox, d: Rect) -> Vec<(usize, Rect, String)> {
    let mut out = Vec::new();
    for (card, w) in s.cards.iter().zip(wells(d, s.cards.len())) {
        let p = plot_rect(w);
        for n in &card.nodes {
            out.push((out.len(), node_rect(p, n), n.label.clone()));
        }
    }
    out
}

/// Where a drawing's curves go inside its well.
pub fn plot_rect(d: Rect) -> Rect {
    d.inset_xy(10, 8)
}

fn paint_section(
    cx: &mut PaintCx,
    t: &Theme,
    s: &SectionBox,
    g: &Geom,
    selected: Option<usize>,
    editing: bool,
) {
    let pal = palette(t, s.job);
    cx.canvas.fill_round_rect(g.rect, t.radius, pal.tint);
    cx.canvas
        .stroke_round_rect(g.rect, t.radius, 1, pal.job.with_alpha(82));
    // The name in the job's colour (Voice's grey a little lighter, to read
    // on the tint), the link or the note on the right.
    let name_color = match s.job {
        Job::Voice => t.text_dim.mix(t.text, 0.4),
        _ => pal.job,
    };
    let title = s.title.to_uppercase();
    let tw = cx.text(
        FontId::Bold,
        LABEL_PX,
        g.head,
        Align::Left,
        name_color,
        &title,
    );
    let mut right = Rect::new(g.head.x + tw + 16, g.head.y, g.head.w - tw - 16, g.head.h);
    if s.link.is_some() {
        let lw = cx.text(
            FontId::Bold,
            LABEL_PX,
            right,
            Align::Right,
            name_color,
            "Open ›",
        );
        right.w -= lw + 12;
    }
    if right.w > 24 {
        cx.text(
            FontId::Sans,
            LABEL_PX,
            right,
            Align::Right,
            t.text_dim,
            &s.note,
        );
    }
    if let (Some(c), Some(sr)) = (&s.switch, g.switch) {
        let on = matches!(c.kind, Kind::Toggle(true));
        let pill = Rect::new(sr.x, sr.y + 1, sr.w, sr.h - 2);
        let r = pill.h / 2;
        if on {
            cx.canvas.fill_round_rect(pill, r, pal.job);
        } else {
            cx.canvas.fill_round_rect(pill, r, t.raised);
            cx.canvas.stroke_round_rect(pill, r, 1, t.outline);
        }
        let d = pill.h - 6;
        let kx = if on { pill.right() - 3 - d } else { pill.x + 3 };
        let knob_c = if on { t.on_accent } else { t.text_dim };
        cx.canvas
            .fill_round_rect(Rect::new(kx, pill.y + 3, d, d), d / 2, knob_c);
    }
    if let Some(d) = g.drawing {
        let job = pal.job;
        let tt = t.clone();
        for (card, w) in s.cards.iter().zip(wells(d, s.cards.len())) {
            cx.canvas.fill_round_rect(w, 10, pal.well);
            paint_plot(cx, plot_rect(w), card, &|c| shade(&tt, job, c));
        }
    }
    let off = s.switched_off();
    for (c, cg) in s.controls.iter().zip(&g.controls) {
        if cg.cell.w <= 0 {
            continue;
        }
        if off && !c.dim {
            let mut c = c.clone();
            c.dim = true;
            paint_control(cx, t, &pal, s.job, &c, cg, &s.title);
        } else {
            paint_control(cx, t, &pal, s.job, c, cg, &s.title);
        }
        if selected == Some(c.index) {
            cx.canvas.stroke_round_rect(
                cg.cell.inset(-4),
                10,
                if editing { 3 } else { 2 },
                t.accent,
            );
        }
    }
}

/// A Q-Link badge ("Q1" in Q-Link 1's colour) at (x, y), its right edge.
/// The knobs' colours are the app's, under every theme, so a badge matches
/// its card in the Q-Link column.
fn badge(cx: &mut PaintCx, t: &Theme, q: usize, right: i32, y: i32) {
    let b = Rect::new(right - 28, y, 28, 18);
    cx.canvas
        .fill_round_rect(b, 6, mpfree_ui::draw::KNOB[q % 4]);
    cx.text(
        FontId::Bold,
        12.0,
        b,
        Align::Center,
        t.on_accent,
        &format!("Q{}", q + 1),
    );
}

#[allow(clippy::too_many_arguments)]
fn paint_value(
    cx: &mut PaintCx,
    t: &Theme,
    r: Rect,
    px: f32,
    value: &str,
    unit: &str,
    dim: bool,
    left: bool,
) {
    let vw = cx.measure(FontId::Bold, px, value);
    let uw = if unit.is_empty() {
        0
    } else {
        cx.measure(FontId::Sans, LABEL_PX, unit) + 5
    };
    let x = if left {
        r.x
    } else {
        r.x + (r.w - vw - uw).max(0) / 2
    };
    cx.text(
        FontId::Bold,
        px,
        Rect::new(x, r.y, vw.min(r.w), r.h),
        Align::Left,
        fade(t.text, dim),
        value,
    );
    if uw > 0 {
        cx.text(
            FontId::Sans,
            LABEL_PX,
            Rect::new(x + vw + 5, r.y + 2, uw, r.h),
            Align::Left,
            fade(t.text_dim, dim),
            unit,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_control(
    cx: &mut PaintCx,
    t: &Theme,
    pal: &Palette,
    job: Job,
    c: &Control,
    g: &CtlGeom,
    section: &str,
) {
    let dim = c.dim;
    let fill = fade(c.tint.map_or(pal.fill, |k| themed(t, k)), dim);
    let label_c = fade(t.text_dim.mix(t.text, 0.3), dim);
    let raised = t.raised.mix(pal.job, 0.08);
    let align = if g.inline { Align::Left } else { Align::Center };
    match &c.kind {
        Kind::Knob { pos, bipolar } => {
            let Some((x, y, d)) = g.dial else {
                return;
            };
            let track = fade(pal.job.with_alpha(52), dim);
            knob(cx, x, y, d, *pos, *bipolar, track, fill);
            cx.text(FontId::Sans, LABEL_PX, g.label, align, label_c, &c.label);
            paint_value(
                cx,
                t,
                g.value,
                value_px(c.tier),
                &c.value,
                &c.unit,
                dim,
                g.inline,
            );
            if let Some(q) = c.qlink {
                if g.inline {
                    badge(cx, t, q, (x + d / 2.0) as i32 + 8, g.cell.y - 10);
                } else {
                    badge(
                        cx,
                        t,
                        q,
                        (x + d / 2.0) as i32 + 14,
                        (y - d / 2.0) as i32 - 2,
                    );
                }
            }
        }
        Kind::Stepper => {
            cx.text(FontId::Sans, LABEL_PX, g.label, align, label_c, &c.label);
            paint_value(
                cx,
                t,
                g.value,
                value_px(c.tier),
                &c.value,
                &c.unit,
                dim,
                g.inline,
            );
            for (k, p) in g.parts.iter().enumerate().take(2) {
                cx.canvas.fill_round_rect(*p, 8, fade(raised, dim));
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    *p,
                    Align::Center,
                    fade(t.text, dim),
                    ["−", "+"][k],
                );
            }
            if let Some(q) = c.qlink {
                badge(cx, t, q, g.cell.right(), g.cell.y - 2);
            }
        }
        Kind::Meter => {
            // Inline, "live" sits at the right of the name's line: the
            // name stops short of it.
            let label = if g.inline {
                Rect::new(
                    g.label.x,
                    g.label.y,
                    (g.label.w - LIVE_W - 6).max(0),
                    g.label.h,
                )
            } else {
                g.label
            };
            cx.text(FontId::Sans, LABEL_PX, label, align, label_c, &c.label);
            paint_value(
                cx,
                t,
                g.value,
                value_px(c.tier),
                &c.value,
                &c.unit,
                false,
                g.inline,
            );
            if g.inline {
                let tag = Rect::new(g.label.x, g.label.y, g.label.w, g.label.h);
                cx.text(FontId::Sans, 12.0, tag, Align::Right, t.text_faint, "live");
            } else {
                let tag = Rect::new(g.cell.x, g.value.bottom() + 2, g.cell.w, 14);
                cx.text(FontId::Sans, 12.0, tag, Align::Center, t.text_faint, "live");
            }
        }
        Kind::Toggle(on) => {
            let r = g.cell;
            let bg = if *on { fill } else { fade(raised, dim) };
            cx.canvas.fill_round_rect(r, r.h / 2, bg);
            let text_c = if *on { t.on_accent } else { fade(t.text, dim) };
            let dot = Rect::new(r.x + 14, r.y + r.h / 2 - 5, 10, 10);
            cx.canvas.fill_round_rect(
                dot,
                5,
                if *on {
                    t.on_accent
                } else {
                    fade(t.text_faint, dim)
                },
            );
            cx.text(
                FontId::Bold,
                14.0,
                Rect::new(r.x + 32, r.y, r.w - 40, r.h),
                Align::Left,
                text_c,
                &c.label,
            );
            if let Some(q) = c.qlink {
                badge(cx, t, q, r.right(), r.y - 8);
            }
        }
        Kind::Choice { names, index } => {
            if g.label.w > 0 && (shows_label(c, section) || g.chip) {
                cx.text(
                    FontId::Sans,
                    LABEL_PX,
                    g.label,
                    Align::Left,
                    label_c,
                    &c.label,
                );
            }
            if g.chip {
                for (k, p) in g.parts.iter().enumerate().take(2) {
                    cx.canvas.fill_round_rect(*p, 8, fade(raised, dim));
                    cx.text(
                        FontId::Bold,
                        t.size_title,
                        *p,
                        Align::Center,
                        fade(t.text, dim),
                        ["‹", "›"][k],
                    );
                }
                let mid = g.value;
                cx.canvas
                    .fill_round_rect(mid, 8, fade(t.bg.mix(pal.job, 0.08), dim));
                cx.text(
                    FontId::Bold,
                    15.0,
                    mid,
                    Align::Center,
                    fade(t.text, dim),
                    names.get(*index).map_or("", |s| s.as_str()),
                );
            } else {
                for (k, (p, name)) in g.parts.iter().zip(names).enumerate() {
                    let on = k == *index;
                    cx.canvas
                        .fill_round_rect(*p, 8, if on { fill } else { fade(raised, dim) });
                    cx.text(
                        if on { FontId::Bold } else { FontId::Sans },
                        14.0,
                        *p,
                        Align::Center,
                        if on { t.on_accent } else { fade(t.text, dim) },
                        name,
                    );
                }
            }
            if let Some(q) = c.qlink {
                badge(cx, t, q, g.cell.right(), g.cell.y - 2);
            }
        }
        Kind::Fader { pos, bipolar } => {
            let tr = g.track;
            cx.text(
                FontId::Sans,
                LABEL_PX,
                g.label,
                Align::Center,
                label_c,
                &c.label,
            );
            cx.canvas
                .fill_round_rect(tr, tr.w / 2, fade(pal.job.with_alpha(46), dim));
            let at = tr.bottom() - (pos.clamp(0.0, 1.0) * tr.h as f32).round() as i32;
            let from = if *bipolar {
                tr.y + tr.h / 2
            } else {
                tr.bottom()
            };
            let (a, b) = if at < from { (at, from) } else { (from, at) };
            if b > a {
                cx.canvas
                    .fill_round_rect(Rect::new(tr.x, a, tr.w, b - a), tr.w / 2, fill);
            }
            let hw = (g.cell.w - 16).clamp(24, 40);
            let handle = Rect::new(tr.x + tr.w / 2 - hw / 2, at - 8, hw, 16);
            cx.canvas.fill_round_rect(handle, 6, fade(t.text, dim));
            paint_value(cx, t, g.value, 15.0, &c.value, &c.unit, dim, false);
            if let Some(q) = c.qlink {
                badge(cx, t, q, g.cell.right() - 2, tr.y - 4);
            }
            let _ = job;
        }
        Kind::Drawbar { pos } => {
            let tr = g.track;
            cx.text(
                FontId::Bold,
                LABEL_PX,
                g.label,
                Align::Center,
                label_c,
                &c.label,
            );
            cx.canvas.fill_round_rect(tr, 6, fade(t.bg, dim));
            let len = (pos.clamp(0.0, 1.0) * tr.h as f32).round() as i32;
            let bar = Rect::new(tr.x + 3, tr.y, tr.w - 6, len.max(10));
            cx.canvas.fill_round_rect(bar, 5, fill);
            let knob_r = Rect::new(tr.x - 4, tr.y + len.max(10) - 4, tr.w + 8, 22);
            cx.canvas.fill_round_rect(knob_r, 6, fill);
            cx.text(
                FontId::Bold,
                13.0,
                knob_r,
                Align::Center,
                t.on_accent,
                &c.value,
            );
            if let Some(q) = c.qlink {
                badge(cx, t, q, g.cell.right() - 2, g.value.y);
            }
        }
        Kind::Slider { pos, bipolar } => {
            cx.text(
                FontId::Sans,
                LABEL_PX,
                g.label,
                Align::Left,
                label_c,
                &c.label,
            );
            let tr = g.track;
            cx.canvas
                .fill_round_rect(tr, 3, fade(pal.job.with_alpha(52), dim));
            let at = tr.x + (pos.clamp(0.0, 1.0) * tr.w as f32).round() as i32;
            let from = if *bipolar { tr.x + tr.w / 2 } else { tr.x };
            let (a, b) = if at < from { (at, from) } else { (from, at) };
            if b > a {
                cx.canvas
                    .fill_round_rect(Rect::new(a, tr.y, b - a, tr.h), 3, fill);
            }
            let mut m = Mask::new(Rect::new(at - 12, tr.y - 12, 25, tr.h + 25));
            m.disc(at as f32, tr.y as f32 + tr.h as f32 / 2.0, 9.0);
            m.draw(cx.canvas, fade(t.text, dim));
            paint_value(cx, t, g.value, 15.0, &c.value, &c.unit, dim, false);
            if let Some(q) = c.qlink {
                badge(cx, t, q, g.cell.right(), g.cell.y - 2);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn knob_ctl(index: usize, tier: Tier) -> Control {
        Control {
            index,
            label: format!("Knob {index}"),
            value: "50".into(),
            unit: "%".into(),
            kind: Kind::Knob {
                pos: 0.5,
                bipolar: false,
            },
            tier,
            qlink: None,
            dim: false,
            tint: None,
        }
    }

    fn view() -> SectionView {
        let mut a = SectionBox::new("a", "Filter", Job::Tone);
        a.controls = vec![
            Control {
                kind: Kind::Choice {
                    names: vec!["LP 24".into(), "LP 12".into(), "BP 12".into()],
                    index: 0,
                },
                ..knob_ctl(7, Tier::Std)
            },
            knob_ctl(1, Tier::Hero),
            knob_ctl(2, Tier::Std),
        ];
        let mut b = SectionBox::new("b", "Env", Job::Shape);
        b.controls = (3..7)
            .map(|i| Control {
                kind: Kind::Fader {
                    pos: 0.3,
                    bipolar: false,
                },
                ..knob_ctl(i, Tier::Std)
            })
            .collect();
        let mut v = SectionView::new();
        v.spec = "row(a:*, b:*)".into();
        v.sections = vec![a, b];
        v
    }

    #[test]
    fn controls_stay_inside_their_sections_and_apart() {
        let v = view();
        let r = Rect::new(0, 0, 844, 598);
        for g in v.geometry(r) {
            for (k, c) in g.controls.iter().enumerate() {
                assert!(c.cell.w > 0 && c.cell.h > 0);
                assert_eq!(c.cell.intersect(g.rect), c.cell, "control {k} spills");
                for o in &g.controls[k + 1..] {
                    assert!(c.cell.intersect(o.cell).is_empty(), "{c:?} overlaps {o:?}");
                }
            }
        }
    }

    #[test]
    fn a_drag_turns_a_knob_and_a_lift_steps_a_choice() {
        let mut v = view();
        let r = Rect::new(0, 0, 844, 598);
        let geo = v.geometry(r);
        let hero = &geo[0].controls[1];
        let (x, y) = (hero.cell.x + hero.cell.w / 2, hero.cell.y + 10);
        assert_eq!(v.touch(1, Touch::Down, x, y, r), Some(EventKind::Select(1)));
        assert_eq!(
            v.touch(1, Touch::Move, x, y - 32, r),
            Some(EventKind::Value(0.6))
        );
        assert_eq!(v.touch(1, Touch::Up, x, y - 32, r), None);
        let lp12 = geo[0].controls[0].parts[1];
        let (x, y) = (lp12.x + 5, lp12.y + 5);
        assert_eq!(v.touch(2, Touch::Down, x, y, r), Some(EventKind::Select(7)));
        assert_eq!(
            v.touch(2, Touch::Up, x, y, r),
            Some(EventKind::Step { index: 7, delta: 1 })
        );
        // A fader drags over its own track's height.
        let f = &geo[1].controls[0];
        let (x, y) = (f.track.x + 2, f.track.y + f.track.h / 2);
        assert_eq!(v.touch(3, Touch::Down, x, y, r), Some(EventKind::Select(3)));
        match v.touch(3, Touch::Move, x, y - f.track.h / 10, r) {
            Some(EventKind::Value(p)) => assert!((p - 0.4).abs() < 0.02, "{p}"),
            e => panic!("{e:?}"),
        }
    }

    #[test]
    fn two_drawings_share_the_well_and_count_their_boxes_across() {
        use crate::widgets::CardNode;
        let node = |label: &str| CardNode {
            x: 0.5,
            y: 0.5,
            label: label.into(),
            fill: Color::rgb(0x202020),
            text: Color::rgb(0xFFFFFF),
            ring: None,
            dim: false,
            feedback: None,
        };
        let mut s = SectionBox::new("d", "Drive", Job::Source);
        s.cards = vec![
            Card {
                title: "Curve".into(),
                nodes: vec![node("a")],
                ..Card::default()
            },
            Card {
                title: "Tone".into(),
                nodes: vec![node("b")],
                ..Card::default()
            },
        ];
        s.node_links = vec![4, 9];
        let mut v = SectionView::new();
        v.spec = "row(d:*)".into();
        v.sections = vec![s];
        let r = Rect::new(0, 0, 600, 300);
        let d = v.geometry(r)[0].drawing.unwrap();
        let w = wells(d, 2);
        assert_eq!(w[0].w, w[1].w);
        assert_eq!(w[1].x - w[0].right(), GAP);
        // The second drawing's box is box 1 of the section.
        let (x, y) = centre_of(node_rect(plot_rect(w[1]), &v.sections[0].cards[1].nodes[0]));
        assert_eq!(v.touch(1, Touch::Down, x, y, r), Some(EventKind::Tab(9)));
    }

    fn centre_of(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    #[test]
    fn an_inline_meter_has_room_for_its_name_and_the_live_tag() {
        // The meters' names an effect page shows, and longer.
        let mut fonts = mpfree_ui::Fonts::embedded();
        let live = fonts.measure(FontId::Sans, 12.0, "live");
        for label in [
            "Reduction",
            "Gain reduction",
            "Level change",
            "Correction",
            "Gate gain",
        ] {
            for tier in [Tier::Trim, Tier::Std, Tier::Main] {
                let c = Control {
                    label: label.into(),
                    value: "-12.5".into(),
                    unit: "dB".into(),
                    kind: Kind::Meter,
                    ..knob_ctl(0, tier)
                };
                let (w, _) = dial_cell(&c, Fit::Inline);
                let name = fonts.measure(FontId::Sans, LABEL_PX, label);
                assert!(name + 6 + live <= w, "{label}: {name} + 6 + {live} > {w}");
                assert!(name <= w - LIVE_W - 6, "{label}: the name runs under live");
            }
        }
    }

    #[test]
    fn a_switch_in_the_heading_toggles_and_dims_its_section() {
        let mut v = view();
        v.sections[0].switch = Some(Control {
            kind: Kind::Toggle(true),
            ..knob_ctl(20, Tier::Std)
        });
        assert!(!v.sections[0].switched_off());
        let r = Rect::new(0, 0, 844, 598);
        let g = v.geometry(r);
        let sw = g[0].switch.unwrap();
        assert!(g[0].head.right() < sw.x, "the name and note stay clear");
        let (x, y) = centre_of(sw);
        assert_eq!(
            v.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Select(20))
        );
        assert_eq!(
            v.touch(1, Touch::Up, x, y, r),
            Some(EventKind::Step {
                index: 20,
                delta: -1
            })
        );
        v.sections[0].switch.as_mut().unwrap().kind = Kind::Toggle(false);
        assert!(v.sections[0].switched_off());
        let ids: Vec<&str> = v.targets(r).iter().map(|t| t.id).collect();
        assert!(ids.contains(&"section-switch"));
    }
}
