//! The macros and morph panel's widget. It opens over a page, under the
//! top bar (as the mixer does), with Back in its corner.
//!
//! Left: four macro cards (macros 1-4 or 5-8, a tap on the bank chips
//! switches). Each card has its knob (drag up or down), its name in its
//! colour, and up to eight targets, each its name over a bar with the part
//! of the parameter's travel the macro covers, a mark where it is now, and
//! the range as text. A tap on a target row asks the app to edit its range
//! and curve.
//!
//! Right: the morph between scene A and scene B: a slider (drag), what
//! changes from A to B with where each parameter is now, Snap back to A,
//! and Save as A or B (the values as they are now).

use mpfree_param::ParamAddr;
use mpfree_synth_ui::widgets::knob;
use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Touch, Widget};

use crate::state::{macro_color, on_accent, MacrosState};

/// What a touch on the panel meant, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
pub enum MacroAction {
    /// Set macro `m` (0 to 1): send `ModEvent::Set` for `ParamAddr::macro_value(m)`.
    Macro { m: usize, value: f32 },
    /// Show macros 1-4 (0) or 5-8 (1).
    Bank(usize),
    /// Open the range and curve editor for one of macro `m`'s targets.
    EditTarget { m: usize, target: ParamAddr },
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

pub struct MacrosLayout {
    pub back: Rect,
    pub banks: [Rect; 2],
    pub cards: [Rect; 4],
    pub knobs: [(f32, f32, f32); 4],
    pub morph: Rect,
    pub slider: Rect,
    pub snap: Rect,
    pub save: [Rect; 2],
}

pub fn layout(r: Rect) -> MacrosLayout {
    let (morph, left) = r.take_right(MORPH_W);
    let left = Rect::new(left.x, left.y, left.w - GAP, left.h);
    let (head, grid) = left.take_top(44);
    let back = Rect::new(head.x, head.y, 96, 34);
    let banks = [Rect::new(head.right() - 2 * 112 - 6, head.y + 1, 112, 32), Rect::new(head.right() - 112, head.y + 1, 112, 32)];
    let (cw, ch) = ((grid.w - GAP) / 2, (grid.h - GAP) / 2);
    let cards = std::array::from_fn(|i| Rect::new(grid.x + (i as i32 % 2) * (cw + GAP), grid.y + (i as i32 / 2) * (ch + GAP), cw, ch));
    let knobs = cards.map(|c| (c.x as f32 + 14.0 + KNOB_D * 0.5, c.y as f32 + 10.0 + KNOB_D * 0.5, KNOB_D));
    let inner = morph.inset(16);
    let slider = Rect::new(inner.x, inner.y + 70, inner.w, 44);
    let (foot, _) = inner.take_bottom(48);
    let snap = Rect::new(foot.x, foot.y, (foot.w - 8) / 2, foot.h);
    let half = ((foot.w - 8) / 2 - 6) / 2;
    let save = [Rect::new(snap.right() + 8, foot.y, half, foot.h), Rect::new(snap.right() + 8 + half + 6, foot.y, half, foot.h)];
    MacrosLayout { back, banks, cards, knobs, morph, slider, snap, save }
}

/// A card's target rows, full width under its knob and name.
fn target_rows(card: Rect) -> Rect {
    Rect::new(card.x + 14, card.y + CARD_HEAD, card.w - 28, card.h - CARD_HEAD - 8)
}

enum Drag {
    Macro { m: usize, y0: i32, v0: f32 },
    Morph,
}

/// See the module docs.
pub struct MacrosView {
    state: MacrosState,
    actions: Vec<MacroAction>,
    drag: Option<(u64, Drag)>,
}

impl Default for MacrosView {
    fn default() -> Self { Self::new() }
}

impl MacrosView {
    pub fn new() -> MacrosView { MacrosView { state: MacrosState::default(), actions: Vec::new(), drag: None } }
    pub fn state(&self) -> &MacrosState { &self.state }
    pub fn update(&mut self, st: MacrosState) { self.state = st; }
    pub fn take_actions(&mut self) -> Vec<MacroAction> { std::mem::take(&mut self.actions) }

    fn push(&mut self, a: MacroAction) {
        match a {
            MacroAction::Macro { m, value } => {
                if let Some(v) = self.state.macros.get_mut(m) { v.value = value; }
            }
            MacroAction::Bank(b) => self.state.bank = b,
            MacroAction::Morph(p) => self.state.morph = p,
            _ => {}
        }
        self.actions.push(a);
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
        cx.text(FontId::Bold, t.size_small, l.back, Align::Center, t.text, "‹  Back");
        let head = Rect::new(l.back.right() + 14, l.back.y, l.banks[0].x - l.back.right() - 14, l.back.h);
        cx.text(FontId::Bold, t.size_small, head, Align::Left, t.text_dim, "MACROS");
        cx.text(FontId::Sans, t.size_small, head.inset_xy(72, 0), Align::Left, t.text_faint, "each moves up to 8 parameters");
        for (b, (br, label)) in l.banks.iter().zip(["Macros 1-4", "Macros 5-8"]).enumerate() {
            let on = b == st.bank;
            cx.canvas.fill_round_rect(*br, 9, if on { t.raised } else { t.surface });
            cx.text(FontId::Bold, t.size_small, *br, Align::Center, if on { t.text } else { t.text_dim }, label);
        }
        for (i, card) in l.cards.iter().enumerate() {
            let m = st.bank * 4 + i;
            let Some(mv) = st.macros.get(m) else { continue };
            let c = macro_color(&t, m);
            cx.canvas.fill_round_rect(*card, t.radius, t.surface);
            let (kx, ky, d) = l.knobs[i];
            knob(cx, kx, ky, d, mv.value, false, t.outline, c);
            cx.text(FontId::Bold, t.size_small - 1.0, Rect::new((kx - d * 0.5) as i32, (ky - 10.0) as i32, d as i32, 20), Align::Center, t.text, &format!("{:.0} %", mv.value * 100.0));
            let text_x = (kx + d * 0.5) as i32 + 16;
            let title = Rect::new(text_x, card.y + 14, card.right() - text_x - 14, 28);
            cx.text(FontId::Bold, t.size_title, title, Align::Left, c, &format!("Macro {} · {}", m + 1, mv.name));
            let note = if mv.targets.is_empty() { "no targets yet".to_string() } else { format!("{} of 8 targets", mv.targets.len()) };
            cx.text(FontId::Sans, t.size_small - 1.0, Rect::new(text_x, title.bottom() + 2, title.w, 20), Align::Left, t.text_faint, &note);
            let rows = target_rows(*card);
            let fit = (rows.h / TARGET_H).max(1) as usize;
            for (k, tv) in mv.targets.iter().enumerate() {
                let row = Rect::new(rows.x, rows.y + k as i32 * TARGET_H, rows.w, TARGET_H);
                if k + 1 == fit && mv.targets.len() > fit {
                    cx.text(FontId::Sans, t.size_small - 1.0, row.take_top(18).0, Align::Left, t.text_faint, &format!("+ {} more", mv.targets.len() - k));
                    break;
                }
                let (name, line) = row.take_top(17);
                cx.text(FontId::Sans, t.size_small, name, Align::Left, t.text, &tv.label);
                let (range, bar) = line.take_right(RANGE_W);
                cx.text(FontId::Mono, t.size_small - 3.0, range.take_top(12).0, Align::Right, t.text_dim, &tv.range_text);
                let track = Rect::new(bar.x, bar.y + 4, bar.w - 10, 5);
                cx.canvas.fill_round_rect(track, 3, t.raised);
                let (a, b) = (tv.from.min(tv.to), tv.from.max(tv.to));
                let xa = track.x + (a * track.w as f32) as i32;
                let xb = track.x + (b * track.w as f32) as i32;
                cx.canvas.fill_round_rect(Rect::new(xa, track.y, (xb - xa).max(3), track.h), 3, c.mix(t.surface, 0.45));
                let xn = track.x + (tv.now.clamp(0.0, 1.0) * track.w as f32) as i32;
                cx.canvas.fill_round_rect(Rect::new(xn - 2, track.y - 4, 4, track.h + 8), 2, c);
            }
            if mv.targets.is_empty() {
                let empty = Rect::new(rows.x, rows.y, rows.w, 44);
                cx.text(FontId::Sans, t.size_small, empty, Align::Left, t.text_dim, "Touch a knob in an effect, then this macro's number");
            }
        }
        // Morph.
        cx.canvas.fill_round_rect(l.morph, t.radius, t.surface);
        let inner = l.morph.inset(16);
        cx.text(FontId::Bold, t.size_small, inner.take_top(24).0, Align::Left, t.text_dim, "MORPH");
        cx.text(FontId::Sans, t.size_small, inner.take_top(24).0, Align::Right, t.text_faint, "every parameter in the scenes");
        let names = Rect::new(inner.x, inner.y + 32, inner.w, 28);
        cx.text(FontId::Bold, t.size_body, names, Align::Left, t.accent, &format!("A · {}", st.scene_names[0]));
        cx.text(FontId::Bold, t.size_body, names, Align::Right, t.warn, &format!("B · {}", st.scene_names[1]));
        let track = l.slider.inset_xy(18, 16);
        let steps = 24;
        for k in 0..steps {
            let x0 = track.x + track.w * k / steps;
            let x1 = track.x + track.w * (k + 1) / steps;
            let c = t.accent.mix(t.warn, k as f32 / (steps - 1) as f32);
            cx.canvas.fill_rect(Rect::new(x0, track.y, x1 - x0, track.h), c);
        }
        let hx = track.x + (st.morph.clamp(0.0, 1.0) * track.w as f32) as i32;
        let handle = Rect::new(hx - 16, l.slider.y + 4, 32, l.slider.h - 8);
        cx.canvas.fill_round_rect(handle, 8, t.text);
        cx.canvas.stroke_round_rect(handle, 8, 1, t.outline);
        let what = Rect::new(inner.x, l.slider.bottom() + 18, inner.w, 22);
        cx.text(FontId::Bold, t.size_small - 1.0, what, Align::Left, t.text_faint, &format!("WHAT CHANGES · {:.0} % TO B", st.morph * 100.0));
        let mut y = what.bottom() + 8;
        if st.changes.is_empty() {
            cx.text(FontId::Sans, t.size_small, Rect::new(inner.x, y, inner.w, 44), Align::Left, t.text_dim, "Save A, change some knobs, save B");
        }
        for (k, ch) in st.changes.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.w, CHANGE_H - 2);
            if row.bottom() + CHANGE_H > l.snap.y - 8 && k + 1 < st.changes.len() {
                cx.text(FontId::Sans, t.size_small - 1.0, row.take_top(20).0, Align::Left, t.text_faint, &format!("+ {} more", st.changes.len() - k));
                break;
            }
            cx.canvas.fill_rect(Rect::new(row.x, row.bottom(), row.w, 1), t.outline);
            let (top, bottom) = row.take_top(20);
            let (now, label) = top.take_right(90);
            cx.text(FontId::Sans, t.size_small, label, Align::Left, t.text, &ch.label);
            cx.text(FontId::Bold, t.size_small, now, Align::Right, t.text, &ch.now);
            cx.text(FontId::Mono, t.size_small - 3.0, bottom.take_top(16).0, Align::Left, t.text_dim, &format!("A {}  →  B {}", ch.a, ch.b));
            y += CHANGE_H;
        }
        cx.canvas.fill_round_rect(l.snap, 10, t.raised);
        cx.text(FontId::Bold, t.size_small, l.snap, Align::Center, t.text, "Snap back to A");
        for (k, b) in l.save.iter().enumerate() {
            cx.canvas.fill_round_rect(*b, 10, if k == 0 { t.accent } else { t.warn });
            cx.text(FontId::Bold, t.size_small, *b, Align::Center, on_accent(&t), if k == 0 { "Save A" } else { "Save B" });
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
                        if let Some(tv) = self.state.macros.get(m).and_then(|v| v.targets.get(k)) {
                            let target = tv.addr;
                            self.push(MacroAction::EditTarget { m, target });
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
                let Some((f, d)) = &self.drag else { return None };
                if *f != finger { return None; }
                match *d {
                    Drag::Macro { m, y0, v0 } => {
                        let value = ((v0 + (y0 - y) as f32 / DRAG_TRAVEL).clamp(0.0, 1.0) * 1000.0).round() / 1000.0;
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
                }
                None
            }
            Touch::Up | Touch::Cancel => {
                if self.drag.as_ref().is_some_and(|(f, _)| *f == finger) { self.drag = None; }
                None
            }
        }
    }
}
