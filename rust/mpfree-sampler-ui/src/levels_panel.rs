//! The 16 Levels panel: one sound spread over the 16 pads, on screen.
//!
//! The owner's design (Jay, 2026-11-13: "button plus a 16 Levels panel",
//! then square cells coloured as the pads are lit). 16 LEVEL turns 16
//! Levels on for the pad hit last and the panel opens where the Arp panel
//! goes, under Play's transport card:
//!
//! - **The heading** names the pad the levels play ("of pad A2 · Snare"),
//!   with **Pick pad** (the next hit picks the pad the levels play),
//!   **Close** (the panel goes, 16 Levels stays on) and **Off** (the pads
//!   play their own sounds again).
//! - **The grid**: 16 square cells laid out as the pads (pad 1 bottom
//!   left), each with its number, its value ("+3 st", "1.2 kHz") and a bar
//!   for its step. The cells take the colour the pads are lit in (the
//!   kit's), and the pad hit last shows bright, as its light flashes.
//!   Pad 9 of Tune says "as recorded".
//! - **The kinds** (Velocity, Tune, Cutoff, Decay, Start) in a column
//!   beside the grid, each with its range; a tap or the wheel picks one.
//!
//! The panel only shows a [`LevelsModel`] and gathers [`LevelsAction`]s;
//! the app changes the kit (`PadsPlay::Levels`) and hands the model back.

use mpfree_sampler::model::Levels;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The panel's widget name.
pub const PANEL: &str = "levels-panel";

/// What the panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelsModel {
    /// The pad the levels play, as the MPC names it ("A2").
    pub pad_label: String,
    /// Its sound's name; empty if it has none.
    pub pad_name: String,
    pub by: Levels,
    /// Pick pad is armed: the next hit picks the pad.
    pub picking: bool,
    /// The cell lit bright: the pad hit last (0 to 15), while held or just
    /// hit.
    pub lit: Option<usize>,
    /// The kit's pad colour, as the controller takes it (0 to 63 a
    /// channel): the cells' colour.
    pub colour: [u8; 3],
}

impl Default for LevelsModel {
    fn default() -> Self {
        LevelsModel {
            pad_label: "A1".to_string(),
            pad_name: String::new(),
            by: Levels::Velocity,
            picking: false,
            lit: None,
            colour: [0x3F, 0x22, 0x00],
        }
    }
}

/// What a touch on the panel asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelsAction {
    /// The levels step through this instead.
    By(Levels),
    /// The next hit picks the pad the levels play (again: called off).
    PickPad,
    /// The panel goes; 16 Levels stays on.
    Close,
    /// The pads play their own sounds again (and the panel goes).
    Off,
}

/// Pad `k`'s value in words for `by`: "8", "+3 st", "1.2 kHz", "50 ms",
/// "44 %".
pub fn step_text(by: Levels, k: usize) -> String {
    match by {
        Levels::Velocity => Levels::velocity(k).to_string(),
        Levels::Tune => match Levels::semitones(k) {
            0 => "0 st".to_string(),
            s => format!("{s:+} st"),
        },
        Levels::Cutoff => {
            let hz = Levels::cutoff_hz(k);
            if hz >= 1000.0 {
                format!("{:.1} kHz", hz / 1000.0)
            } else {
                format!("{hz:.0} Hz")
            }
        }
        Levels::Decay => {
            let ms = Levels::decay_ms(k);
            if ms >= 1000.0 {
                format!("{:.1} s", ms / 1000.0)
            } else {
                format!("{ms:.0} ms")
            }
        }
        Levels::Start => format!("{:.0} %", Levels::start(k) * 100.0),
    }
}

/// What a kind spans, lowest pad to highest, for its row.
pub fn range_text(by: Levels) -> String {
    format!("{} to {}", step_text(by, 0), step_text(by, 15))
}

/// The panel: heading, grid and kinds in one widget.
#[derive(Debug, Default)]
pub struct LevelsPanel {
    pub model: LevelsModel,
    actions: Vec<LevelsAction>,
}

const GAP: i32 = 10;
const HEAD_H: i32 = 60;
const BUTTON_W: i32 = 110;
const ROW_H: i32 = 54;
const KINDS_TOP: i32 = 38;

/// The heading's buttons, left to right.
const BUTTONS: [&str; 3] = ["Pick pad", "Close", "Off"];

struct Parts {
    head: Rect,
    buttons: [Rect; 3],
    cells: [Rect; 16],
    kinds_panel: Rect,
    kinds: [Rect; 5],
}

fn parts(r: Rect) -> Parts {
    let (head, body) = r.take_top(HEAD_H);
    let body = Rect::new(body.x, body.y + GAP, body.w, (body.h - GAP).max(0));
    let mut buttons = [Rect::default(); 3];
    let mut x = head.right() - 12;
    for b in buttons.iter_mut().rev() {
        x -= BUTTON_W;
        *b = Rect::new(x, head.y + 10, BUTTON_W, (head.h - 20).max(0));
        x -= GAP;
    }
    // Square cells, as many pixels as the body's height allows (and never
    // more than half its width).
    let side = ((body.h - 3 * GAP) / 4)
        .min((body.w / 2 - 3 * GAP) / 4)
        .max(0);
    let mut cells = [Rect::default(); 16];
    for (k, c) in cells.iter_mut().enumerate() {
        let (row, col) = (3 - k as i32 / 4, k as i32 % 4);
        *c = Rect::new(
            body.x + col * (side + GAP),
            body.y + row * (side + GAP),
            side,
            side,
        );
    }
    let grid_w = 4 * side + 3 * GAP;
    let kinds_panel = Rect::new(
        body.x + grid_w + GAP * 2,
        body.y,
        (body.w - grid_w - GAP * 2).max(0),
        body.h,
    );
    let mut kinds = [Rect::default(); 5];
    for (i, k) in kinds.iter_mut().enumerate() {
        *k = Rect::new(
            kinds_panel.x + 12,
            kinds_panel.y + KINDS_TOP + i as i32 * (ROW_H + 4),
            (kinds_panel.w - 24).max(0),
            ROW_H,
        );
    }
    Parts {
        head,
        buttons,
        cells,
        kinds_panel,
        kinds,
    }
}

impl LevelsPanel {
    pub fn new() -> Self {
        Self::default()
    }

    /// What the touches since the last call asked for.
    pub fn take_actions(&mut self) -> Vec<LevelsAction> {
        std::mem::take(&mut self.actions)
    }

    /// The cell for pad `k` (for tests and the app's touch checks).
    pub fn cell_rect(r: Rect, k: usize) -> Rect {
        parts(r).cells[k.min(15)]
    }

    /// Button `i` of the heading: 0 Pick pad, 1 Close, 2 Off.
    pub fn button_rect(r: Rect, i: usize) -> Rect {
        parts(r).buttons[i.min(2)]
    }

    /// The row of kind `by`.
    pub fn kind_rect(r: Rect, by: Levels) -> Rect {
        let i = Levels::ALL.iter().position(|l| *l == by).unwrap_or(0);
        parts(r).kinds[i]
    }

    /// The wheel: the next (`delta` > 0) or previous kind, without wrapping.
    pub fn wheel(by: Levels, delta: i32) -> Levels {
        let i = Levels::ALL.iter().position(|l| *l == by).unwrap_or(0) as i32;
        let n = Levels::ALL.len() as i32;
        Levels::ALL[(i + delta.signum()).clamp(0, n - 1) as usize]
    }
}

impl Widget for LevelsPanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let m = &self.model;
        cx.canvas.fill_rect(r, t.bg);
        let p = parts(r);
        // The heading.
        cx.panel(p.head);
        let inner = p.head.inset_xy(t.pad, 8);
        let (top, rest) = inner.take_top(18);
        cx.text(
            FontId::Bold,
            t.size_small,
            top,
            Align::Left,
            t.text_dim,
            "16 LEVELS",
        );
        let of = if m.picking {
            "Hit the pad the levels play".to_string()
        } else if m.pad_name.is_empty() {
            format!("of pad {}", m.pad_label)
        } else {
            format!("of pad {} · {}", m.pad_label, m.pad_name)
        };
        cx.text(
            FontId::Bold,
            t.size_body + 2.0,
            rest,
            Align::Left,
            if m.picking { t.accent } else { t.text },
            &of,
        );
        for (i, (b, label)) in p.buttons.iter().zip(BUTTONS).enumerate() {
            let armed = i == 0 && m.picking;
            cx.canvas
                .fill_round_rect(*b, 8, if armed { t.accent } else { t.raised });
            let color = match i {
                _ if armed => t.on_accent,
                2 => t.bad,
                _ => t.text,
            };
            cx.text(FontId::Bold, t.size_body, *b, Align::Center, color, label);
        }
        // The grid, in the pads' colour.
        let pad = Color::from_pad(m.colour);
        let rest_fill = t.raised.mix(pad, 0.45);
        let hit_fill = Color::from_pad([63, 63, 63]).mix(pad, 0.85);
        for (k, c) in p.cells.iter().enumerate() {
            if c.w <= 0 {
                continue;
            }
            let lit = m.lit == Some(k);
            let (fill, fg, sub) = if lit {
                (hit_fill, t.bg, t.bg.mix(hit_fill, 0.3))
            } else {
                (rest_fill, t.text, t.text.mix(rest_fill, 0.25))
            };
            cx.canvas.fill_round_rect(*c, 10, fill);
            let body = c.inset(12);
            let (num, rest) = body.take_top(16);
            cx.text(
                FontId::Bold,
                t.size_small,
                num,
                Align::Left,
                sub,
                &format!("PAD {}", k + 1),
            );
            if m.by == Levels::Tune && Levels::semitones(k) == 0 {
                let (note, _) = rest.take_top(16);
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    note,
                    Align::Left,
                    sub,
                    "as recorded",
                );
            }
            cx.text(
                FontId::Bold,
                t.size_title,
                Rect::new(c.x, c.y + c.h / 2 - 16, c.w, 32),
                Align::Center,
                fg,
                &step_text(m.by, k),
            );
            // The step's bar.
            let bar = Rect::new(c.x + 12, c.bottom() - 18, (c.w - 24).max(0), 6);
            cx.canvas.fill_round_rect(bar, 3, fill.mix(t.bg, 0.35));
            let w = (bar.w * (k as i32 + 1) / 16).max(6).min(bar.w);
            cx.canvas.fill_round_rect(
                Rect::new(bar.x, bar.y, w, bar.h),
                3,
                if lit { t.bg } else { hit_fill },
            );
        }
        // The kinds.
        if p.kinds_panel.w > 0 {
            cx.panel(p.kinds_panel);
            let head = Rect::new(
                p.kinds_panel.x + t.pad,
                p.kinds_panel.y + 10,
                (p.kinds_panel.w - 2 * t.pad).max(0),
                20,
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                head,
                Align::Left,
                t.text_dim,
                "LEVELS CHANGE · WHEEL",
            );
            for (row, by) in p.kinds.iter().zip(Levels::ALL) {
                let sel = by == m.by;
                if sel {
                    cx.canvas.fill_round_rect(*row, 8, t.raised);
                    cx.canvas.fill_rect(
                        Rect::new(row.x, row.y + 8, 3, (row.h - 16).max(0)),
                        t.accent,
                    );
                }
                let text = row.inset_xy(16, 0);
                cx.text(
                    if sel { FontId::Bold } else { FontId::Sans },
                    t.size_body + 1.0,
                    text,
                    Align::Left,
                    if sel { t.text } else { t.text_dim },
                    by.label(),
                );
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    text,
                    Align::Right,
                    t.text_dim,
                    &range_text(by),
                );
            }
        }
    }

    /// Pick pad, Close and Off, then the kinds (the one picked is lit).
    fn targets(&self, r: Rect) -> Vec<Target> {
        let p = parts(r);
        let mut v: Vec<Target> = p
            .buttons
            .iter()
            .zip(BUTTONS)
            .enumerate()
            .map(|(i, (b, label))| Target::tap("button", i, label, *b))
            .collect();
        if p.kinds_panel.w > 0 {
            for (i, (row, by)) in p.kinds.iter().zip(Levels::ALL).enumerate() {
                v.push(Target::tap("kind", i, by.label(), *row).disabled_if(by == self.model.by));
            }
        }
        v
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let p = parts(r);
        let action = match p.buttons.iter().position(|b| b.contains(x, y)) {
            Some(i) => [
                LevelsAction::PickPad,
                LevelsAction::Close,
                LevelsAction::Off,
            ][i],
            None => {
                let i = p.kinds.iter().position(|k| k.contains(x, y))?;
                LevelsAction::By(Levels::ALL[i])
            }
        };
        self.actions.push(action);
        Some(EventKind::Select(self.actions.len() - 1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: Rect = Rect::new(12, 180, 998, 570);

    #[test]
    fn values_read_as_the_engine_plays_them() {
        assert_eq!(step_text(Levels::Velocity, 0), "8");
        assert_eq!(step_text(Levels::Velocity, 15), "127");
        assert_eq!(step_text(Levels::Tune, 8), "0 st");
        assert_eq!(step_text(Levels::Tune, 11), "+3 st");
        assert_eq!(step_text(Levels::Tune, 0), "-8 st");
        assert_eq!(step_text(Levels::Cutoff, 0), "100 Hz");
        assert_eq!(step_text(Levels::Cutoff, 15), "20.0 kHz");
        assert_eq!(step_text(Levels::Decay, 0), "25 ms");
        assert_eq!(step_text(Levels::Decay, 15), "4.5 s");
        assert_eq!(step_text(Levels::Start, 8), "50 %");
        assert_eq!(range_text(Levels::Tune), "-8 st to +7 st");
    }

    #[test]
    fn cells_are_square_and_laid_out_as_the_pads() {
        let c1 = LevelsPanel::cell_rect(R, 0);
        let c4 = LevelsPanel::cell_rect(R, 3);
        let c13 = LevelsPanel::cell_rect(R, 12);
        assert_eq!(c1.w, c1.h);
        assert!(c1.w >= 100, "{c1:?}");
        // Pad 1 bottom left, pad 4 bottom right, pad 13 top left.
        assert_eq!(c1.y, c4.y);
        assert!(c4.x > c1.x);
        assert_eq!(c13.x, c1.x);
        assert!(c13.y < c1.y);
        assert!(c1.bottom() <= R.bottom());
        // The kinds sit right of the grid.
        let kind = LevelsPanel::kind_rect(R, Levels::Velocity);
        assert!(kind.x > c4.right());
        assert!(LevelsPanel::kind_rect(R, Levels::Start).bottom() <= R.bottom());
    }

    #[test]
    fn taps_ask_for_kinds_and_the_buttons() {
        let mut p = LevelsPanel::new();
        let tap =
            |p: &mut LevelsPanel, r: Rect| p.touch(1, Touch::Down, r.x + r.w / 2, r.y + r.h / 2, R);
        assert!(tap(&mut p, LevelsPanel::kind_rect(R, Levels::Decay)).is_some());
        assert!(tap(&mut p, LevelsPanel::button_rect(R, 0)).is_some());
        assert!(tap(&mut p, LevelsPanel::button_rect(R, 1)).is_some());
        assert!(tap(&mut p, LevelsPanel::button_rect(R, 2)).is_some());
        // A cell is shown, not a button.
        assert!(tap(&mut p, LevelsPanel::cell_rect(R, 5)).is_none());
        assert_eq!(
            p.take_actions(),
            vec![
                LevelsAction::By(Levels::Decay),
                LevelsAction::PickPad,
                LevelsAction::Close,
                LevelsAction::Off
            ]
        );
        assert!(p.take_actions().is_empty());
    }

    #[test]
    fn the_wheel_steps_the_kinds_without_wrapping() {
        assert_eq!(LevelsPanel::wheel(Levels::Velocity, 1), Levels::Tune);
        assert_eq!(LevelsPanel::wheel(Levels::Velocity, -1), Levels::Velocity);
        assert_eq!(LevelsPanel::wheel(Levels::Start, 3), Levels::Start);
    }
}
