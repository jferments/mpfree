//! The macros and morph panel's widget. It opens over a page, under the
//! top bar (as the mixer does), with Back in its corner.
//!
//! Left: four macro cards (macros 1-4 or 5-8, a tap on the bank chips
//! switches). Each card has its knob (drag up or down), its name in its
//! colour, and up to eight targets, each its name over a bar with the part
//! of the parameter's travel the macro covers, a mark where it is now, and
//! the range as text. A tap on a target row opens its editor over the
//! cards: drag the From and To handles along the parameter's travel, pick
//! a curve (a drawing shows what the macro's turn does to the target),
//! Flip swaps the ends, Remove takes the target off the macro, Done
//! closes.
//!
//! Right: the morph between scene A and scene B: a slider (drag), what
//! changes from A to B with where each parameter is now, Snap back to A,
//! and Save as A or B (the values as they are now).

use mpfree_mod::{value_text, MacroMap, MapCurve};
use mpfree_param::ParamAddr;
use mpfree_synth_ui::draw::Mask;
use mpfree_synth_ui::widgets::knob;
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Touch, Widget};

use crate::state::{macro_color, range_text, MacrosState, TargetView};

/// What a touch on the panel meant, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum MacroAction {
    /// Set macro `m` (0 to 1): send `ModEvent::hand` for
    /// `ParamAddr::macro_value(m)` (a `Set` unless it has a lane or its
    /// track is recording).
    Macro { m: usize, value: f32 },
    /// Show macros 1-4 (0) or 5-8 (1).
    Bank(usize),
    /// Change one of macro `m`'s targets (its range or curve, from the
    /// target editor): send `Change::Map { m, map }`.
    Map { m: usize, map: MacroMap },
    /// Take a target off its macro: `Change::Unmap`.
    Unmap(ParamAddr),
    /// Set the morph position (0 = A, 1 = B).
    Morph(f32),
    /// Store the values as they are now as scene A (0) or B (1).
    Capture(usize),
    /// Close the panel (back to the page it opened over).
    Back,
}

pub const MORPH_W: i32 = 360;
const GAP: i32 = 10;
/// A macro card's top: its knob and name.
const CARD_HEAD: i32 = 80;
const TARGET_H: i32 = 30;
const CHANGE_H: i32 = 42;
const KNOB_D: f32 = 60.0;
/// The range text's width on a target's bar line.
const RANGE_W: i32 = 128;
const DRAG_TRAVEL: f32 = 300.0;
/// The curves the target editor offers, in its chips' order.
pub const CURVES: [MapCurve; 4] = [MapCurve::Linear, MapCurve::Exp, MapCurve::Log, MapCurve::S];

pub struct MacrosLayout {
    pub back: Rect,
    pub banks: [Rect; 2],
    pub cards: [Rect; 4],
    pub knobs: [(f32, f32, f32); 4],
    pub morph: Rect,
    pub slider: Rect,
    pub snap: Rect,
    pub save: [Rect; 2],
    pub editor: EditorLayout,
}

/// The target editor, over the macro cards.
pub struct EditorLayout {
    pub area: Rect,
    pub done: Rect,
    /// The parameter's travel, with the From and To handles on it.
    pub track: Rect,
    /// What a turn of the macro does to the target, 0 to 100 %.
    pub graph: Rect,
    pub curves: [Rect; 4],
    pub flip: Rect,
    pub remove: Rect,
}

fn editor_layout(area: Rect) -> EditorLayout {
    let inner = area.inset(20);
    let done = Rect::new(inner.right() - 104, inner.y, 104, 38);
    let track = Rect::new(inner.x + 24, inner.y + 120, inner.w - 48, 44);
    let (foot, _) = inner.take_bottom(46);
    let curves_y = foot.y - 24 - 44;
    let cw = (inner.w - 3 * 8) / 4;
    let curves =
        std::array::from_fn(|i| Rect::new(inner.x + i as i32 * (cw + 8), curves_y, cw, 44));
    let graph_top = track.bottom() + 44;
    let graph = Rect::new(
        inner.x + 24,
        graph_top,
        inner.w - 48,
        (curves_y - 58 - graph_top).max(40),
    );
    let half = (foot.w - 8) / 2;
    EditorLayout {
        area,
        done,
        track,
        graph,
        curves,
        flip: Rect::new(foot.x, foot.y, half, foot.h),
        remove: Rect::new(foot.x + half + 8, foot.y, half, foot.h),
    }
}

pub fn layout(r: Rect) -> MacrosLayout {
    let (morph, left) = r.take_right(MORPH_W);
    let left = Rect::new(left.x, left.y, left.w - GAP, left.h);
    let (head, grid) = left.take_top(44);
    let back = Rect::new(head.x, head.y, 96, 34);
    let banks = [
        Rect::new(head.right() - 2 * 112 - 6, head.y + 1, 112, 32),
        Rect::new(head.right() - 112, head.y + 1, 112, 32),
    ];
    let (cw, ch) = ((grid.w - GAP) / 2, (grid.h - GAP) / 2);
    let cards = std::array::from_fn(|i| {
        Rect::new(
            grid.x + (i as i32 % 2) * (cw + GAP),
            grid.y + (i as i32 / 2) * (ch + GAP),
            cw,
            ch,
        )
    });
    let knobs = cards.map(|c| {
        (
            c.x as f32 + 14.0 + KNOB_D * 0.5,
            c.y as f32 + 10.0 + KNOB_D * 0.5,
            KNOB_D,
        )
    });
    let inner = morph.inset(16);
    let slider = Rect::new(inner.x, inner.y + 70, inner.w, 44);
    let (foot, _) = inner.take_bottom(48);
    let snap = Rect::new(foot.x, foot.y, (foot.w - 8) / 2, foot.h);
    let half = ((foot.w - 8) / 2 - 6) / 2;
    let save = [
        Rect::new(snap.right() + 8, foot.y, half, foot.h),
        Rect::new(snap.right() + 8 + half + 6, foot.y, half, foot.h),
    ];
    let editor = editor_layout(grid);
    MacrosLayout {
        editor,
        back,
        banks,
        cards,
        knobs,
        morph,
        slider,
        snap,
        save,
    }
}

/// A card's target rows, full width under its knob and name.
fn target_rows(card: Rect) -> Rect {
    Rect::new(
        card.x + 14,
        card.y + CARD_HEAD,
        card.w - 28,
        card.h - CARD_HEAD - 8,
    )
}

enum Drag {
    Macro {
        m: usize,
        y0: i32,
        v0: f32,
    },
    Morph,
    /// The From (`to` false) or To handle of the target being edited.
    End {
        to: bool,
    },
}

/// See the module docs.
pub struct MacrosView {
    state: MacrosState,
    actions: Vec<MacroAction>,
    drag: Option<(u64, Drag)>,
}

impl Default for MacrosView {
    fn default() -> Self {
        Self::new()
    }
}

impl MacrosView {
    pub fn new() -> MacrosView {
        MacrosView {
            state: MacrosState::default(),
            actions: Vec::new(),
            drag: None,
        }
    }
    pub fn state(&self) -> &MacrosState {
        &self.state
    }
    /// Shows a new state. Of the old one it keeps only an open target
    /// editor, while its target is still there.
    pub fn update(&mut self, st: MacrosState) {
        let editing = st.editing.or(self.state.editing);
        self.state = st;
        self.state.editing = editing;
        if self.editing().is_none() {
            self.state.editing = None;
        }
    }
    pub fn take_actions(&mut self) -> Vec<MacroAction> {
        std::mem::take(&mut self.actions)
    }

    fn push(&mut self, a: MacroAction) {
        match a {
            MacroAction::Macro { m, value } => {
                if let Some(v) = self.state.macros.get_mut(m) {
                    v.value = value;
                }
            }
            MacroAction::Bank(b) => {
                self.state.bank = b;
                self.state.editing = None;
            }
            MacroAction::Morph(p) => self.state.morph = p,
            MacroAction::Map { m, map } => {
                let tv = self
                    .state
                    .macros
                    .get_mut(m)
                    .and_then(|v| v.targets.iter_mut().find(|t| t.addr == map.target));
                if let Some(tv) = tv {
                    tv.from = map.from;
                    tv.to = map.to;
                    tv.curve = map.curve;
                    if let Some(i) = tv.info {
                        tv.range_text = range_text(&i, i.from_unit(map.from), i.from_unit(map.to));
                    }
                }
            }
            MacroAction::Unmap(target) => {
                for v in &mut self.state.macros {
                    v.targets.retain(|t| t.addr != target);
                }
                self.state.editing = None;
            }
            MacroAction::Back => self.state.editing = None,
            MacroAction::Capture(_) => {}
        }
        self.actions.push(a);
    }

    /// The target being edited, and its macro.
    fn editing(&self) -> Option<(usize, &TargetView)> {
        let (m, k) = self.state.editing?;
        Some((m, self.state.macros.get(m)?.targets.get(k)?))
    }

    /// The editing target's map with `f` applied, as an action.
    fn edit(&mut self, f: impl Fn(&mut MacroMap)) {
        let Some((m, tv)) = self.editing() else {
            return;
        };
        let mut map = MacroMap {
            target: tv.addr,
            from: tv.from,
            to: tv.to,
            curve: tv.curve,
        };
        f(&mut map);
        self.push(MacroAction::Map { m, map });
    }

    fn track_pos(l: &MacrosLayout, x: i32) -> f32 {
        let t = l.editor.track;
        (((x - t.x) as f32 / t.w.max(1) as f32).clamp(0.0, 1.0) * 100.0).round() / 100.0
    }

    fn slider_pos(l: &MacrosLayout, x: i32) -> f32 {
        let s = l.slider.inset_xy(18, 0);
        (((x - s.x) as f32 / s.w.max(1) as f32).clamp(0.0, 1.0) * 100.0).round() / 100.0
    }
}

impl Widget for MacrosView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let st = &self.state;
        let l = layout(r);
        cx.canvas.fill_rect(r, t.bg);
        cx.canvas.fill_round_rect(l.back, 9, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            l.back,
            Align::Center,
            t.text,
            "‹  Back",
        );
        let head = Rect::new(
            l.back.right() + 14,
            l.back.y,
            l.banks[0].x - l.back.right() - 14,
            l.back.h,
        );
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            "MACROS",
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            head.inset_xy(72, 0),
            Align::Left,
            t.text_faint,
            "each moves up to 8 parameters",
        );
        for (b, (br, label)) in l.banks.iter().zip(["Macros 1-4", "Macros 5-8"]).enumerate() {
            let on = b == st.bank;
            cx.canvas
                .fill_round_rect(*br, 9, if on { t.raised } else { t.surface });
            cx.text(
                FontId::Bold,
                t.size_small,
                *br,
                Align::Center,
                if on { t.text } else { t.text_dim },
                label,
            );
        }
        if let Some((m, tv)) = self.editing() {
            paint_editor(cx, &l.editor, m, &st.macros[m].name, tv);
        }
        for (i, card) in l.cards.iter().enumerate() {
            if self.editing().is_some() {
                break;
            }
            let m = st.bank * 4 + i;
            let Some(mv) = st.macros.get(m) else { continue };
            let c = macro_color(&t, m);
            cx.canvas.fill_round_rect(*card, t.radius, t.surface);
            let (kx, ky, d) = l.knobs[i];
            knob(cx, kx, ky, d, mv.value, false, t.outline, c);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                Rect::new((kx - d * 0.5) as i32, (ky - 10.0) as i32, d as i32, 20),
                Align::Center,
                t.text,
                &format!("{:.0} %", mv.value * 100.0),
            );
            let text_x = (kx + d * 0.5) as i32 + 16;
            let title = Rect::new(text_x, card.y + 14, card.right() - text_x - 14, 28);
            cx.text(
                FontId::Bold,
                t.size_title,
                title,
                Align::Left,
                c,
                &format!("Macro {} · {}", m + 1, mv.name),
            );
            let note = if mv.targets.is_empty() {
                "no targets yet".to_string()
            } else {
                format!("{} of 8 targets", mv.targets.len())
            };
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(text_x, title.bottom() + 2, title.w, 20),
                Align::Left,
                t.text_faint,
                &note,
            );
            let rows = target_rows(*card);
            let fit = (rows.h / TARGET_H).max(1) as usize;
            for (k, tv) in mv.targets.iter().enumerate() {
                let row = Rect::new(rows.x, rows.y + k as i32 * TARGET_H, rows.w, TARGET_H);
                if k + 1 == fit && mv.targets.len() > fit {
                    cx.text(
                        FontId::Sans,
                        t.size_small - 1.0,
                        row.take_top(18).0,
                        Align::Left,
                        t.text_faint,
                        &format!("+ {} more", mv.targets.len() - k),
                    );
                    break;
                }
                let (name, line) = row.take_top(17);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    name,
                    Align::Left,
                    t.text,
                    &tv.label,
                );
                let (range, bar) = line.take_right(RANGE_W);
                cx.text(
                    FontId::Mono,
                    t.size_small - 3.0,
                    range.take_top(12).0,
                    Align::Right,
                    t.text_dim,
                    &tv.range_text,
                );
                let track = Rect::new(bar.x, bar.y + 4, bar.w - 10, 5);
                cx.canvas.fill_round_rect(track, 3, t.raised);
                let (a, b) = (tv.from.min(tv.to), tv.from.max(tv.to));
                let xa = track.x + (a * track.w as f32) as i32;
                let xb = track.x + (b * track.w as f32) as i32;
                cx.canvas.fill_round_rect(
                    Rect::new(xa, track.y, (xb - xa).max(3), track.h),
                    3,
                    c.mix(t.surface, 0.45),
                );
                let xn = track.x + (tv.now.clamp(0.0, 1.0) * track.w as f32) as i32;
                cx.canvas
                    .fill_round_rect(Rect::new(xn - 2, track.y - 4, 4, track.h + 8), 2, c);
            }
            if mv.targets.is_empty() {
                let empty = Rect::new(rows.x, rows.y, rows.w, 44);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    empty,
                    Align::Left,
                    t.text_dim,
                    "Touch a knob in an effect, then this macro's number",
                );
            }
        }
        // Morph.
        cx.canvas.fill_round_rect(l.morph, t.radius, t.surface);
        let inner = l.morph.inset(16);
        cx.text(
            FontId::Bold,
            t.size_small,
            inner.take_top(24).0,
            Align::Left,
            t.text_dim,
            "MORPH",
        );
        cx.text(
            FontId::Sans,
            t.size_small,
            inner.take_top(24).0,
            Align::Right,
            t.text_faint,
            "every parameter in the scenes",
        );
        let names = Rect::new(inner.x, inner.y + 32, inner.w, 28);
        cx.text(
            FontId::Bold,
            t.size_body,
            names,
            Align::Left,
            t.accent,
            &format!("A · {}", st.scene_names[0]),
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            names,
            Align::Right,
            t.warn,
            &format!("B · {}", st.scene_names[1]),
        );
        let track = l.slider.inset_xy(18, 16);
        let steps = 24;
        for k in 0..steps {
            let x0 = track.x + track.w * k / steps;
            let x1 = track.x + track.w * (k + 1) / steps;
            let c = t.accent.mix(t.warn, k as f32 / (steps - 1) as f32);
            cx.canvas
                .fill_rect(Rect::new(x0, track.y, x1 - x0, track.h), c);
        }
        let hx = track.x + (st.morph.clamp(0.0, 1.0) * track.w as f32) as i32;
        let handle = Rect::new(hx - 16, l.slider.y + 4, 32, l.slider.h - 8);
        cx.canvas.fill_round_rect(handle, 8, t.text);
        cx.canvas.stroke_round_rect(handle, 8, 1, t.outline);
        let what = Rect::new(inner.x, l.slider.bottom() + 18, inner.w, 22);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            what,
            Align::Left,
            t.text_faint,
            &format!("WHAT CHANGES · {:.0} % TO B", st.morph * 100.0),
        );
        let mut y = what.bottom() + 8;
        if st.changes.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(inner.x, y, inner.w, 44),
                Align::Left,
                t.text_dim,
                "Save A, change some knobs, save B",
            );
        }
        for (k, ch) in st.changes.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.w, CHANGE_H - 2);
            if row.bottom() + CHANGE_H > l.snap.y - 8 && k + 1 < st.changes.len() {
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    row.take_top(20).0,
                    Align::Left,
                    t.text_faint,
                    &format!("+ {} more", st.changes.len() - k),
                );
                break;
            }
            cx.canvas
                .fill_rect(Rect::new(row.x, row.bottom(), row.w, 1), t.outline);
            let (top, bottom) = row.take_top(20);
            let (now, label) = top.take_right(90);
            cx.text(
                FontId::Sans,
                t.size_small,
                label,
                Align::Left,
                t.text,
                &ch.label,
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                now,
                Align::Right,
                t.text,
                &ch.now,
            );
            cx.text(
                FontId::Mono,
                t.size_small - 3.0,
                bottom.take_top(16).0,
                Align::Left,
                t.text_dim,
                &format!("A {}  →  B {}", ch.a, ch.b),
            );
            y += CHANGE_H;
        }
        cx.canvas.fill_round_rect(l.snap, 10, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            l.snap,
            Align::Center,
            t.text,
            "Snap back to A",
        );
        for (k, b) in l.save.iter().enumerate() {
            cx.canvas
                .fill_round_rect(*b, 10, if k == 0 { t.accent } else { t.warn });
            cx.text(
                FontId::Bold,
                t.size_small,
                *b,
                Align::Center,
                t.on_accent,
                if k == 0 { "Save A" } else { "Save B" },
            );
        }
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let l = layout(r);
        match phase {
            Touch::Down => {
                if l.back.contains(x, y) {
                    self.push(MacroAction::Back);
                    return None;
                }
                if let Some(b) = l.banks.iter().position(|b| b.contains(x, y)) {
                    self.push(MacroAction::Bank(b));
                    return None;
                }
                if self.editing().is_some() && l.editor.area.contains(x, y) {
                    let e = &l.editor;
                    if e.done.contains(x, y) {
                        self.state.editing = None;
                    } else if e.track.inset_xy(-16, -14).contains(x, y) {
                        let (_, tv) = self.editing()?;
                        let p = Self::track_pos(&l, x);
                        let to = (p - tv.to).abs() < (p - tv.from).abs()
                            || ((p - tv.to).abs() == (p - tv.from).abs() && p > tv.to);
                        self.drag = Some((finger, Drag::End { to }));
                        self.edit(|m| *if to { &mut m.to } else { &mut m.from } = p);
                    } else if let Some(c) = e.curves.iter().position(|c| c.contains(x, y)) {
                        self.edit(|m| m.curve = CURVES[c]);
                    } else if e.flip.contains(x, y) {
                        self.edit(|m| std::mem::swap(&mut m.from, &mut m.to));
                    } else if e.remove.contains(x, y) {
                        let (_, tv) = self.editing()?;
                        let target = tv.addr;
                        self.push(MacroAction::Unmap(target));
                    }
                    return None;
                }
                for (i, &(kx, ky, d)) in l.knobs.iter().enumerate() {
                    let m = self.state.bank * 4 + i;
                    if ((x as f32 - kx).powi(2) + (y as f32 - ky).powi(2)).sqrt() <= d * 0.5 + 8.0 {
                        let v0 = self.state.macros.get(m).map_or(0.0, |v| v.value);
                        self.drag = Some((finger, Drag::Macro { m, y0: y, v0 }));
                        return None;
                    }
                }
                for (i, card) in l.cards.iter().enumerate() {
                    let m = self.state.bank * 4 + i;
                    let rows = target_rows(*card);
                    if rows.contains(x, y) {
                        let k = ((y - rows.y) / TARGET_H) as usize;
                        let fit = (rows.h / TARGET_H).max(1) as usize;
                        let n = self.state.macros.get(m).map_or(0, |v| v.targets.len());
                        // The last row is "+ n more" when they don't all fit.
                        if k < n && !(k + 1 == fit && n > fit) {
                            self.state.editing = Some((m, k));
                        }
                        return None;
                    }
                }
                if l.slider.contains(x, y) {
                    self.drag = Some((finger, Drag::Morph));
                    let p = Self::slider_pos(&l, x);
                    self.push(MacroAction::Morph(p));
                    return None;
                }
                if l.snap.contains(x, y) {
                    self.push(MacroAction::Morph(0.0));
                } else if let Some(k) = l.save.iter().position(|b| b.contains(x, y)) {
                    self.push(MacroAction::Capture(k));
                }
                None
            }
            Touch::Move => {
                let Some((f, d)) = &self.drag else {
                    return None;
                };
                if *f != finger {
                    return None;
                }
                match *d {
                    Drag::Macro { m, y0, v0 } => {
                        let value = ((v0 + (y0 - y) as f32 / DRAG_TRAVEL).clamp(0.0, 1.0) * 1000.0)
                            .round()
                            / 1000.0;
                        if self.state.macros.get(m).is_some_and(|v| v.value != value) {
                            self.push(MacroAction::Macro { m, value });
                        }
                    }
                    Drag::Morph => {
                        let p = Self::slider_pos(&l, x);
                        if p != self.state.morph {
                            self.push(MacroAction::Morph(p));
                        }
                    }
                    Drag::End { to } => {
                        let p = Self::track_pos(&l, x);
                        let (_, tv) = self.editing()?;
                        if p != if to { tv.to } else { tv.from } {
                            self.edit(|m| *if to { &mut m.to } else { &mut m.from } = p);
                        }
                    }
                }
                None
            }
            Touch::Up | Touch::Cancel => {
                if self.drag.as_ref().is_some_and(|(f, _)| *f == finger) {
                    self.drag = None;
                }
                None
            }
        }
    }
}

/// The target editor: what it edits, the range on the parameter's travel,
/// the drawing of the curve, the curve chips, Flip and Remove.
fn paint_editor(cx: &mut PaintCx, e: &EditorLayout, m: usize, name: &str, tv: &TargetView) {
    let t = cx.theme.clone();
    let c = macro_color(&t, m);
    cx.canvas.fill_round_rect(e.area, t.radius, t.surface);
    let inner = e.area.inset(20);
    cx.text(
        FontId::Bold,
        t.size_small,
        Rect::new(inner.x, inner.y, inner.w - 120, 20),
        Align::Left,
        c,
        &format!("MACRO {} · {}", m + 1, name.to_uppercase()),
    );
    cx.text(
        FontId::Bold,
        t.size_title,
        Rect::new(inner.x, inner.y + 22, inner.w - 120, 34),
        Align::Left,
        t.text,
        &tv.label,
    );
    cx.canvas.fill_round_rect(e.done, 9, t.accent);
    cx.text(
        FontId::Bold,
        t.size_small,
        e.done,
        Align::Center,
        t.on_accent,
        "Done",
    );
    cx.text(
        FontId::Sans,
        t.size_body,
        Rect::new(inner.x, inner.y + 64, inner.w, 26),
        Align::Left,
        t.text_dim,
        &format!("Macro 0 → 100 % moves it {}", tv.range_text),
    );
    // The travel, the covered part, the handles and where it is now.
    let tr = e.track;
    let bar = Rect::new(tr.x, tr.y + tr.h / 2 - 4, tr.w, 8);
    cx.canvas.fill_round_rect(bar, 4, t.raised);
    let x_of = |p: f32| tr.x + (p.clamp(0.0, 1.0) * tr.w as f32) as i32;
    let (xa, xb) = (x_of(tv.from.min(tv.to)), x_of(tv.from.max(tv.to)));
    cx.canvas.fill_round_rect(
        Rect::new(xa, bar.y, (xb - xa).max(3), bar.h),
        4,
        c.mix(t.surface, 0.35),
    );
    let xn = x_of(tv.now);
    cx.canvas
        .fill_round_rect(Rect::new(xn - 2, tr.y + 4, 4, tr.h - 8), 2, t.text_dim);
    for (p, label) in [(tv.from, "From"), (tv.to, "To")] {
        let x = x_of(p);
        let h = Rect::new(x - 14, tr.y, 28, tr.h);
        cx.canvas.fill_round_rect(h, 8, c);
        cx.canvas.stroke_round_rect(h, 8, 1, t.outline);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            Rect::new(x - 60, tr.bottom() + 4, 120, 20),
            Align::Center,
            t.text_dim,
            &match tv.info {
                Some(i) => format!("{label} {}", value_text(&i, i.from_unit(p))),
                None => format!("{label} {:.0} %", p * 100.0),
            },
        );
    }
    // What a turn of the macro does: macro along, target up.
    let g = e.graph;
    cx.canvas.stroke_round_rect(g, 8, 1, t.outline);
    let pg = g.inset(10);
    let pts: Vec<(f32, f32)> = (0..=48)
        .map(|i| {
            let x = i as f32 / 48.0;
            let y = tv.from + (tv.to - tv.from) * tv.curve.apply(x);
            (
                pg.x as f32 + x * pg.w as f32,
                pg.bottom() as f32 - y * pg.h as f32,
            )
        })
        .collect();
    let mut mask = Mask::new(g);
    mask.polyline(&pts, 2.5);
    mask.draw(cx.canvas, c);
    cx.text(
        FontId::Sans,
        t.size_small - 2.0,
        Rect::new(pg.x, g.bottom() + 4, pg.w, 18),
        Align::Left,
        t.text_faint,
        "macro 0 %",
    );
    cx.text(
        FontId::Sans,
        t.size_small - 2.0,
        Rect::new(pg.x, g.bottom() + 4, pg.w, 18),
        Align::Right,
        t.text_faint,
        "macro 100 %",
    );
    cx.text(
        FontId::Sans,
        t.size_small,
        Rect::new(inner.x, e.curves[0].y - 24, inner.w, 20),
        Align::Left,
        t.text_faint,
        "CURVE",
    );
    for (k, r) in e.curves.iter().enumerate() {
        let on = CURVES[k] == tv.curve;
        cx.canvas
            .fill_round_rect(*r, 10, if on { c } else { t.raised });
        cx.text(
            FontId::Bold,
            t.size_small,
            *r,
            Align::Center,
            if on { t.on_accent } else { t.text },
            MapCurve::NAMES[k],
        );
    }
    cx.canvas.fill_round_rect(e.flip, 10, t.raised);
    cx.text(
        FontId::Bold,
        t.size_small,
        e.flip,
        Align::Center,
        t.text,
        "Flip the range",
    );
    cx.canvas.stroke_round_rect(e.remove, 10, 1, t.bad);
    cx.text(
        FontId::Bold,
        t.size_small,
        e.remove,
        Align::Center,
        t.bad,
        "Remove from the macro",
    );
}
