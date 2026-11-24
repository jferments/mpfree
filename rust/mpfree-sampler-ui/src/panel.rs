//! The Sample tab's control panel (Jay, 17 Nov 2026: the sampler should
//! feel like an instrument, not a menu). A page's settings sit in a band
//! under the full-width wave as labelled sections of knobs, buttons,
//! selectors, steppers and switches, drawn in the Synth tab's style.
//!
//! The panel is a way of drawing a [`SettingList`](crate::widgets::SettingList):
//! the page keeps its rows (name and value as text) and adds a [`Control`]
//! for each row, saying how it looks, and a [`Panel`] saying which rows go
//! in which section. Touches come back as the list's own events, so a page
//! handles them as it handled the list:
//!
//! - **A knob** dragged up or down: [`EventKind::Step`] by the detents
//!   passed, a step every [`step_px`] pixels (a knob of few steps moves
//!   further for each).
//! - **A choice's button** tapped: a step from the option on to the one
//!   tapped (`delta` is the distance, so `on + delta` is the tapped one).
//! - **A selector's or a stepper's arrows**: a step down or up; the value
//!   of a typed one, [`EventKind::Type`] when the finger lifts.
//! - **A switch**: a step up (the page flips it).
//! - **A button**: [`EventKind::Tab`] with the row.
//! - **A slider** (the Play page's crossfader) touched or dragged:
//!   [`EventKind::Value`] with the place under the finger, 0 to 1 (so a
//!   panel has one slider at most).
//!
//! A finger on any control picks its row ([`EventKind::Select`] when the
//! touch changes nothing else), so the data wheel and - / + work on the
//! control last touched, as they did on the row. The control picked is
//! outlined; when the wheel's push opened it, the outline is lit.
//!
//! The approved mockups are the design canvas "Sample tab as an
//! instrument" (`sampler/sample-tab-panel/` in the project files).

use mpfree_ui::{
    Align, Color, EventKind, FontId, Fonts, PaintCx, Rect, Role, Shape, Target, Touch,
};
use std::f32::consts::PI;

/// How one row looks in the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Look {
    /// A few named options as buttons side by side, the one `on` lit. The
    /// options in `off` can't be picked now (drawn faint).
    Choice {
        options: Vec<String>,
        on: usize,
        off: Vec<usize>,
    },
    /// The value between arrows: ‹ value ›.
    Chip,
    /// A knob lit `pos` (0 to 1) round its arc, from the middle when
    /// `bipolar`. `steps` is how many steps take it from one end to the
    /// other, which sets how far a finger drags for each.
    Knob { pos: f32, steps: u32, bipolar: bool },
    /// A switch, on or off.
    Toggle(bool),
    /// − value +.
    Stepper,
    /// A button with this text.
    Button(String),
    /// The page's main button, big and lit (Chop to pads).
    Primary(String),
    /// A slider with its handle at `pos` (0 to 1): a crossfader.
    Slider { pos: f32 },
    /// As [`Look::Choice`], the buttons in a grid `cols` wide, row by row
    /// (Put on's places, as the mockup draws them).
    Grid {
        options: Vec<String>,
        on: usize,
        off: Vec<usize>,
        cols: usize,
    },
    /// A bank's sixteen pads, pad 1 first, drawn four by four from the
    /// bottom row (pads 1 to 4) up, as the MPC's pads are: Put on's pad
    /// map. A tap on a pad is [`EventKind::Cell`] with the row and the
    /// pad's place in the bank.
    Pads(Vec<PadCell>),
}

/// One pad of a [`Look::Pads`] map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PadCell {
    /// What it says (a slice's number), drawn on its colour.
    pub text: String,
    /// The colour of the slice landing on it: the theme's series colour
    /// this many along ([`mpfree_ui::Theme::track`]); None: no slice.
    pub tint: Option<usize>,
    /// It has a sound already (drawn raised when no slice lands on it).
    pub used: bool,
    /// The slice picked lands on it (outlined).
    pub picked: bool,
}

/// One row's control: its look, and whether it does anything now (a faint
/// control is drawn grey and changes nothing). A faint control with a
/// reason in `why` answers a tap with [`EventKind::Select`] of its row,
/// without picking it, for the page to say why
/// ([`crate::widgets::SettingList::why_faint`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    pub look: Look,
    pub faint: bool,
    pub why: String,
    /// A knob lit in this colour, and its name too, in place of the
    /// accent: the Q-Link that turns it (the Pads tab's Level, Pan, Tune
    /// and Voices in Q1 to Q4's colours).
    pub tint: Option<Color>,
    /// Outlined while another row is picked: the Grain tool's route its
    /// pickers edit (sampler step 17c).
    pub outlined: bool,
}

impl Control {
    pub fn new(look: Look) -> Control {
        Control {
            look,
            faint: false,
            why: String::new(),
            tint: None,
            outlined: false,
        }
    }

    /// Outlined when `on` ([`Control::outlined`]).
    pub fn outlined(mut self, on: bool) -> Control {
        self.outlined = on;
        self
    }

    pub fn faint_if(mut self, faint: bool) -> Control {
        self.faint = faint;
        self
    }

    /// A knob lit in `color` ([`Control::tint`]).
    pub fn tinted(mut self, color: Color) -> Control {
        self.tint = Some(color);
        self
    }

    /// Faint when `faint`, saying `why` when tapped.
    pub fn faint_because(mut self, faint: bool, why: &str) -> Control {
        self.faint = faint;
        self.why = if faint {
            why.to_string()
        } else {
            String::new()
        };
        self
    }
}

/// A row placed in a section's line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Item {
    /// The row (its place in the list's rows).
    pub row: usize,
    /// Its width in pixels; 0 shares what the line's fixed items leave.
    pub width: i32,
    /// The row's name over a choice, selector, stepper or button (a knob
    /// always has it; a switch has it beside it).
    pub label: bool,
    /// A knob's name on two lines where one is too narrow for it ("Size /
    /// spray"), so every name reads whole; the knob sits a line lower.
    pub tall: bool,
    /// Under the item before it in the line, in the same slot: a column of
    /// controls side by side with the line's others (the Slice section's
    /// Method over Snap to zero, beside its knob).
    pub under: bool,
    /// The control's height in pixels in place of its look's own (a main
    /// button that leaves room for the lines over it); 0: its look's.
    pub height: i32,
}

impl Item {
    pub fn new(row: usize) -> Item {
        Item {
            row,
            width: 0,
            label: false,
            tall: false,
            under: false,
            height: 0,
        }
    }

    pub fn wide(mut self, width: i32) -> Item {
        self.width = width;
        self
    }

    pub fn labelled(mut self) -> Item {
        self.label = true;
        self
    }

    /// A knob with room for its name on two lines ([`Item::tall`]).
    pub fn two_lines(mut self) -> Item {
        self.tall = true;
        self
    }

    /// Under the item before it, in its slot ([`Item::under`]).
    pub fn under(mut self) -> Item {
        self.under = true;
        self
    }

    /// The control `h` pixels high ([`Item::height`]).
    pub fn high(mut self, h: i32) -> Item {
        self.height = h;
        self
    }
}

/// A labelled section: a title (and a note on its right), and lines of
/// controls side by side.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub note: String,
    /// A small line at the section's foot, under its controls (the Edit
    /// pad's "keys, notes and 16 Levels follow; hits don't").
    pub foot: String,
    /// Its share of the panel's width (the mockup's pixels; the panel
    /// scales them to the width it has).
    pub width: i32,
    pub lines: Vec<Vec<Item>>,
    /// The lines from the top rather than centred.
    pub from_top: bool,
    /// Outlined in the accent: the section picked (the FX page's effect
    /// on the Q-Links).
    pub lit: bool,
}

impl Section {
    pub fn new(title: &str, width: i32, lines: Vec<Vec<Item>>) -> Section {
        Section {
            title: title.to_string(),
            note: String::new(),
            foot: String::new(),
            width,
            lines,
            from_top: false,
            lit: false,
        }
    }

    pub fn note(mut self, note: &str) -> Section {
        self.note = note.to_string();
        self
    }

    /// The line at its foot ([`Section::foot`]).
    pub fn foot(mut self, foot: &str) -> Section {
        self.foot = foot.to_string();
        self
    }

    pub fn from_top(mut self) -> Section {
        self.from_top = true;
        self
    }

    pub fn lit(mut self, lit: bool) -> Section {
        self.lit = lit;
        self
    }
}

/// The panel: its sections, left to right.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel {
    pub sections: Vec<Section>,
}

/// Between sections.
const GAP: i32 = 10;
/// Inside a section.
const PAD: i32 = 12;
/// A section's title line, and the space under it.
const HEAD_H: i32 = 16;
const HEAD_GAP: i32 = 10;
const LINE_GAP: i32 = 12;
/// A section's foot line ([`Section::foot`]).
const FOOT_H: i32 = 16;
/// The least the lines close up to when a section is full.
const MIN_LINE_GAP: i32 = 4;
/// Between the items of one slot ([`Item::under`]).
const STACK_GAP: i32 = 6;
const ITEM_GAP: i32 = 12;
/// A row's name over its control.
const LABEL_H: i32 = 16;
const LABEL_GAP: i32 = 6;
/// Buttons, selectors and steppers.
const CTRL_H: i32 = 38;
const OPTION_GAP: i32 = 5;
const ARROW_W: i32 = 30;
const STEP_W: i32 = 40;
/// A knob: its name, the knob, its value.
const KNOB_D: i32 = 52;
const KNOB_VALUE_H: i32 = 20;
const KNOB_H: i32 = LABEL_H + 5 + KNOB_D + 5 + KNOB_VALUE_H;
/// The page's main button.
const PRIMARY_H: i32 = 84;
/// A slider's handle.
const HANDLE_D: i32 = 22;
/// A grid's buttons, and between them.
const GRID_H: i32 = 32;
const GRID_GAP: i32 = 6;
/// A pad map's pads, and between them.
const CELL_H: i32 = 28;
const CELL_GAP: i32 = 4;
/// A switch.
const SWITCH_W: i32 = 46;
const SWITCH_H: i32 = 27;
/// How far a finger drags a knob from one end to the other.
const KNOB_TRAVEL: i32 = 200;
/// The knob's arc, as the Synth tab's: from the bottom left round to the
/// bottom right.
const A0: f32 = 0.75 * PI;
const A1: f32 = 2.25 * PI;

/// Pixels of drag for one step of a knob with `steps` steps.
pub fn step_px(steps: u32) -> i32 {
    (KNOB_TRAVEL / steps.max(1) as i32).clamp(2, 24)
}

/// Where a placed control's parts are.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub row: usize,
    /// The whole item, with its name.
    pub rect: Rect,
    /// A knob's name has two lines ([`Item::tall`]).
    pub tall: bool,
    /// The control itself (under the name).
    pub control: Rect,
    /// A choice's buttons.
    pub options: Vec<Rect>,
}

impl Placed {
    /// A selector's or stepper's left and right arrows, and the value
    /// between them.
    fn arrows(&self, look: &Look) -> (Rect, Rect, Rect) {
        let w = if matches!(look, Look::Stepper) {
            STEP_W
        } else {
            ARROW_W
        };
        let (less, mid) = self.control.take_left(w);
        let (more, mid) = mid.take_right(w);
        (less, mid, more)
    }

    /// Where a slider's handle is at `pos`: its centre's x.
    fn handle_x(&self, pos: f32) -> i32 {
        let (x0, x1) = self.slide();
        x0 + ((x1 - x0) as f32 * pos.clamp(0.0, 1.0)).round() as i32
    }

    /// The x a slider's handle centre moves between.
    fn slide(&self) -> (i32, i32) {
        let c = self.control;
        (c.x + HANDLE_D / 2, c.right() - HANDLE_D / 2)
    }

    /// The place on a slider under `x`, 0 to 1.
    fn slide_at(&self, x: i32) -> f32 {
        let (x0, x1) = self.slide();
        ((x - x0) as f32 / (x1 - x0).max(1) as f32).clamp(0.0, 1.0)
    }

    /// A switch's track, on the item's right.
    fn switch(&self) -> Rect {
        let c = self.control;
        Rect::new(
            c.right() - SWITCH_W,
            c.y + (c.h - SWITCH_H) / 2,
            SWITCH_W,
            SWITCH_H,
        )
    }
}

fn item_h(look: &Look, it: &Item) -> i32 {
    let name = if it.label { LABEL_H + LABEL_GAP } else { 0 };
    match look {
        Look::Knob { .. } => KNOB_H + if it.tall { LABEL_H } else { 0 },
        Look::Toggle(_) => SWITCH_H,
        _ if it.height > 0 => it.height + name,
        Look::Primary(_) => PRIMARY_H + name,
        Look::Grid { options, cols, .. } => {
            let rows = options.len().div_ceil((*cols).max(1)).max(1) as i32;
            rows * GRID_H + (rows - 1) * GRID_GAP + name
        }
        Look::Pads(_) => 4 * CELL_H + 3 * CELL_GAP + name,
        _ => CTRL_H + name,
    }
}

/// A line's items in slots: each item with those [`Item::under`] it.
fn slots(line: &[Item]) -> Vec<Vec<Item>> {
    let mut out: Vec<Vec<Item>> = Vec::new();
    for it in line {
        match out.last_mut() {
            Some(slot) if it.under => slot.push(*it),
            _ => out.push(vec![*it]),
        }
    }
    out
}

/// About how wide `s` is on a choice's button, without the fonts: wide
/// enough that the paint's fitting rarely has to shrink it.
fn est_w(s: &str) -> i32 {
    (s.chars().count() as f32 * 7.6).round() as i32 + 14
}

/// The buttons of a grid of `n` options `cols` wide in `r`, row by row.
fn grid_rects(r: Rect, n: usize, cols: usize) -> Vec<Rect> {
    let cols = cols.max(1) as i32;
    let w = (r.w - GRID_GAP * (cols - 1)) / cols;
    (0..n as i32)
        .map(|i| {
            let (row, col) = (i / cols, i % cols);
            Rect::new(
                r.x + col * (w + GRID_GAP),
                r.y + row * (GRID_H + GRID_GAP),
                w,
                GRID_H,
            )
        })
        .collect()
}

/// A pad map's pads in `r`, pad 1 first: the bottom row is pads 1 to 4.
pub fn cell_rects(r: Rect) -> Vec<Rect> {
    let w = (r.w - 3 * CELL_GAP) / 4;
    (0..16)
        .map(|i| {
            let (row, col) = (3 - i / 4, i % 4);
            Rect::new(
                r.x + col * (w + CELL_GAP),
                r.y + row * (CELL_H + CELL_GAP),
                w,
                CELL_H,
            )
        })
        .collect()
}

/// The buttons of a choice of `options` across `r`: each as wide as its
/// name needs, sharing what is left; squeezed alike when they don't fit.
fn option_rects(r: Rect, options: &[String]) -> Vec<Rect> {
    let n = options.len() as i32;
    if n == 0 {
        return Vec::new();
    }
    let room = (r.w - OPTION_GAP * (n - 1)).max(n);
    let want: Vec<i32> = options.iter().map(|o| est_w(o)).collect();
    let total: i32 = want.iter().sum();
    let widths: Vec<i32> = if total <= room {
        let extra = room - total;
        want.iter()
            .enumerate()
            .map(|(i, w)| w + extra / n + i32::from((i as i32) < extra % n))
            .collect()
    } else {
        let mut ws: Vec<i32> = want.iter().map(|w| w * room / total).collect();
        let short = room - ws.iter().sum::<i32>();
        if let Some(last) = ws.last_mut() {
            *last += short;
        }
        ws
    };
    let mut x = r.x;
    widths
        .iter()
        .map(|w| {
            let b = Rect::new(x, r.y, *w, r.h);
            x += w + OPTION_GAP;
            b
        })
        .collect()
}

impl Panel {
    /// The sections' rectangles across `r`.
    pub fn section_rects(&self, r: Rect) -> Vec<Rect> {
        let n = self.sections.len() as i32;
        if n == 0 {
            return Vec::new();
        }
        let room = r.w - GAP * (n - 1);
        let total: i32 = self.sections.iter().map(|s| s.width.max(1)).sum();
        let mut x = r.x;
        let mut out = Vec::new();
        for (i, s) in self.sections.iter().enumerate() {
            let w = if i as i32 == n - 1 {
                r.right() - x
            } else {
                room * s.width.max(1) / total
            };
            out.push(Rect::new(x, r.y, w, r.h));
            x += w + GAP;
        }
        out
    }

    /// Where row `row`'s control is (under its name), if it is on the panel.
    pub fn control_rect(&self, r: Rect, controls: &[Control], row: usize) -> Option<Rect> {
        self.place(r, controls)
            .into_iter()
            .find(|p| p.row == row)
            .map(|p| p.control)
    }

    /// Where every row in the panel is drawn and answers a finger.
    pub fn place(&self, r: Rect, controls: &[Control]) -> Vec<Placed> {
        let mut out = Vec::new();
        for (s, sr) in self.sections.iter().zip(self.section_rects(r)) {
            let inner = sr.inset(PAD);
            let mut body = inner.take_top(HEAD_H + HEAD_GAP).1;
            if !s.foot.is_empty() {
                body = body.take_bottom(FOOT_H + MIN_LINE_GAP).1;
            }
            let look = |row: usize| controls.get(row).map(|c| &c.look);
            // Each line's slots, with the items that have a control.
            let lines: Vec<Vec<Vec<(Item, &Look)>>> = s
                .lines
                .iter()
                .map(|line| {
                    slots(line)
                        .into_iter()
                        .map(|slot| {
                            slot.into_iter()
                                .filter_map(|it| look(it.row).map(|l| (it, l)))
                                .collect::<Vec<_>>()
                        })
                        .filter(|slot| !slot.is_empty())
                        .collect()
                })
                .collect();
            let slot_h = |slot: &[(Item, &Look)]| {
                slot.iter().map(|(it, l)| item_h(l, it)).sum::<i32>()
                    + STACK_GAP * (slot.len() as i32 - 1).max(0)
            };
            let heights: Vec<i32> = lines
                .iter()
                .map(|line| line.iter().map(|slot| slot_h(slot)).max().unwrap_or(0))
                .collect();
            // A full section closes its lines up, down to MIN_LINE_GAP.
            let n_gaps = (heights.len() as i32 - 1).max(0);
            let sum = heights.iter().sum::<i32>();
            let gap = if n_gaps > 0 && sum + LINE_GAP * n_gaps > body.h {
                ((body.h - sum) / n_gaps).clamp(MIN_LINE_GAP, LINE_GAP)
            } else {
                LINE_GAP
            };
            let total = sum + gap * n_gaps;
            let mut y = if s.from_top {
                body.y
            } else {
                body.y + ((body.h - total) / 2).max(0)
            };
            for (line, h) in lines.iter().zip(&heights) {
                let k = line.len() as i32;
                let fixed: i32 = line.iter().map(|slot| slot[0].0.width).sum();
                let shared = line.iter().filter(|slot| slot[0].0.width == 0).count() as i32;
                let gaps = ITEM_GAP * (k - 1).max(0);
                let left = (body.w - fixed - gaps).max(0);
                // Fixed items alone spread out evenly across the line.
                let (each, mut x, spread) = if shared > 0 {
                    (left / shared, body.x, ITEM_GAP)
                } else {
                    (
                        0,
                        body.x + left / (2 * k.max(1)),
                        ITEM_GAP + left / k.max(1),
                    )
                };
                for slot in line {
                    let w = match slot[0].0.width {
                        0 => each,
                        w => w,
                    };
                    let mut iy = y + (h - slot_h(slot)) / 2;
                    for &(it, l) in slot {
                        let ih = item_h(l, &it);
                        let rect = Rect::new(x, iy, w, ih);
                        iy += ih + STACK_GAP;
                        let control = match l {
                            Look::Knob { .. } | Look::Toggle(_) => rect,
                            _ if it.label => rect.take_top(LABEL_H + LABEL_GAP).1,
                            _ => rect,
                        };
                        let options = match l {
                            Look::Choice { options, .. } => option_rects(control, options),
                            Look::Grid { options, cols, .. } => {
                                grid_rects(control, options.len(), *cols)
                            }
                            Look::Pads(_) => cell_rects(control),
                            _ => Vec::new(),
                        };
                        out.push(Placed {
                            row: it.row,
                            rect,
                            tall: it.tall && matches!(l, Look::Knob { .. }),
                            control,
                            options,
                        });
                    }
                    x += w + spread;
                }
                y += h + gap;
            }
        }
        out
    }

    /// The row whose control is under (x, y), and where it is.
    pub fn at(&self, r: Rect, controls: &[Control], x: i32, y: i32) -> Option<Placed> {
        self.place(r, controls)
            .into_iter()
            .find(|p| p.rect.contains(x, y))
    }
}

/// What a finger on a panel is doing: dragging the knob of `row` (from
/// `from`, the last step's place), or resting on a typed value, whose
/// number pad opens when it lifts (`from` unused).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub finger: u64,
    pub row: usize,
    pub from: i32,
    pub typing: bool,
}

/// The event a touch on the panel gives, and the knob drag it starts,
/// moves or ends. The list picks the row the event names.
pub fn touch(
    panel: &Panel,
    controls: &[Control],
    typed: &[usize],
    drag: &mut Option<Drag>,
    (finger, phase, x, y): (u64, Touch, i32, i32),
    r: Rect,
) -> Option<EventKind> {
    match phase {
        Touch::Down => {}
        Touch::Move => {
            let d = drag.as_mut().filter(|d| d.finger == finger && !d.typing)?;
            let steps = match controls.get(d.row).map(|c| &c.look) {
                Some(Look::Knob { steps, .. }) => *steps,
                Some(Look::Slider { .. }) => {
                    let row = d.row;
                    let p = panel
                        .place(r, controls)
                        .into_iter()
                        .find(|p| p.row == row)?;
                    return Some(EventKind::Value(p.slide_at(x)));
                }
                _ => return None,
            };
            let px = step_px(steps);
            let steps = (d.from - y) / px;
            if steps == 0 {
                return None;
            }
            d.from -= steps * px;
            return Some(EventKind::Step {
                index: d.row,
                delta: steps,
            });
        }
        Touch::Up => {
            let d = drag.take_if(|d| d.finger == finger)?;
            // A typed value opens the number pad on the lift.
            return (d.typing && d.row < controls.len()).then_some(EventKind::Type(d.row));
        }
        _ => {
            drag.take_if(|d| d.finger == finger);
            return None;
        }
    }
    let p = panel.at(r, controls, x, y)?;
    let c = controls.get(p.row)?;
    if c.faint {
        return (!c.why.is_empty()).then_some(EventKind::Select(p.row));
    }
    let k = p.row;
    match &c.look {
        Look::Knob { .. } => {
            *drag = Some(Drag {
                finger,
                row: k,
                from: y,
                typing: false,
            });
            Some(EventKind::Select(k))
        }
        Look::Choice { on, off, .. } | Look::Grid { on, off, .. } => {
            let i = p.options.iter().position(|o| o.contains(x, y));
            match i {
                Some(i) if i != *on && !off.contains(&i) => Some(EventKind::Step {
                    index: k,
                    delta: i as i32 - *on as i32,
                }),
                _ => Some(EventKind::Select(k)),
            }
        }
        Look::Pads(_) => match p.options.iter().position(|o| o.contains(x, y)) {
            Some(col) => Some(EventKind::Cell { row: k, col }),
            None => Some(EventKind::Select(k)),
        },
        Look::Chip | Look::Stepper => {
            let (less, mid, more) = p.arrows(&c.look);
            if less.contains(x, y) {
                Some(EventKind::Step {
                    index: k,
                    delta: -1,
                })
            } else if more.contains(x, y) {
                Some(EventKind::Step { index: k, delta: 1 })
            } else if mid.contains(x, y) && typed.contains(&k) {
                *drag = Some(Drag {
                    finger,
                    row: k,
                    from: y,
                    typing: true,
                });
                Some(EventKind::Select(k))
            } else {
                Some(EventKind::Select(k))
            }
        }
        Look::Toggle(_) => Some(EventKind::Step { index: k, delta: 1 }),
        Look::Slider { .. } => {
            if !p.control.contains(x, y) {
                return Some(EventKind::Select(k));
            }
            *drag = Some(Drag {
                finger,
                row: k,
                from: x,
                typing: false,
            });
            Some(EventKind::Value(p.slide_at(x)))
        }
        Look::Button(_) | Look::Primary(_) => {
            if p.control.contains(x, y) {
                Some(EventKind::Tab(k))
            } else {
                Some(EventKind::Select(k))
            }
        }
    }
}

/// The controls a finger can use, for the control audit and the smoke
/// tests. Labels are the row's name, with the option or arrow.
pub fn targets(
    panel: &Panel,
    controls: &[Control],
    names: &[String],
    typed: &[usize],
    r: Rect,
) -> Vec<Target> {
    let mut out = Vec::new();
    for p in panel.place(r, controls) {
        let (Some(c), Some(name)) = (controls.get(p.row), names.get(p.row)) else {
            continue;
        };
        let k = p.row;
        match &c.look {
            Look::Knob { pos, .. } => {
                // Dragged up; a knob at the top has nowhere to go.
                out.push(
                    Target::drag("knob", k, name.clone(), p.control)
                        .disabled_if(c.faint || *pos >= 1.0),
                );
            }
            Look::Choice { options, on, off }
            | Look::Grid {
                options, on, off, ..
            } => {
                for (i, (o, rect)) in options.iter().zip(&p.options).enumerate() {
                    out.push(
                        Target::tap("choice", k * 16 + i, format!("{name}: {o}"), *rect)
                            .disabled_if(c.faint || i == *on || off.contains(&i)),
                    );
                }
            }
            Look::Chip | Look::Stepper => {
                let (less, mid, more) = p.arrows(&c.look);
                out.push(Target::tap("less", k, format!("{name} ‹"), less).disabled_if(c.faint));
                out.push(Target::tap("more", k, format!("{name} ›"), more).disabled_if(c.faint));
                if typed.contains(&k) {
                    out.push(Target::tap("value", k, name.clone(), mid).disabled_if(c.faint));
                }
            }
            Look::Toggle(_) => {
                out.push(Target::tap("switch", k, name.clone(), p.switch()).disabled_if(c.faint));
            }
            Look::Button(text) | Look::Primary(text) => {
                out.push(Target::tap("button", k, text.clone(), p.control).disabled_if(c.faint));
            }
            Look::Pads(_) => {
                for (i, rect) in p.options.iter().enumerate() {
                    out.push(
                        Target::tap("pad", k * 16 + i, format!("{name}: pad {}", i + 1), *rect)
                            .disabled_if(c.faint),
                    );
                }
            }
            Look::Slider { pos } => {
                // Dragged right from the middle; a handle at the right
                // end has nowhere to go.
                out.push(
                    Target::drag("slider", k, name.clone(), p.control)
                        .towards(p.control.w / 3, 0)
                        .disabled_if(c.faint || *pos >= 1.0),
                );
            }
        }
    }
    out
}

/// The control a finger resting at (`x`, `y`) puts under the data wheel
/// (Jay, 17 Nov: hold a control and turn the wheel): one with a value, not
/// a button or a faint control. The wheel then steps its row as its arrows
/// do (a slider moves a hundredth of its travel a detent,
/// [`EventKind::Value`]). A knob's and a slider's are drags, so the finger
/// may move on them and keep the wheel.
pub fn held(
    panel: &Panel,
    controls: &[Control],
    names: &[String],
    (x, y): (i32, i32),
    r: Rect,
) -> Option<Target> {
    let p = panel.at(r, controls, x, y)?;
    let c = controls.get(p.row).filter(|c| !c.faint)?;
    let name = names.get(p.row).cloned().unwrap_or_default();
    match c.look {
        Look::Button(_) | Look::Primary(_) => None,
        Look::Knob { .. } => Some(Target::drag("knob", p.row, name, p.control)),
        Look::Slider { .. } => Some(Target::drag("slider", p.row, name, p.control)),
        _ => Some(Target::tap("control", p.row, name, p.control)),
    }
}

/// The largest size from `px` down to `min` at which `s` fits in `w`.
/// A knob's name as drawn over it, `w` pixels wide: whole on one line if
/// it fits; with `two` lines, split at the space that leaves the longer
/// half shortest; cut with "…" (the last line) only if it still doesn't
/// fit.
pub fn knob_name_lines(fonts: &mut Fonts, name: &str, w: i32, two: bool) -> Vec<String> {
    const PX: f32 = 13.0;
    if fonts.measure(FontId::Sans, PX, name) <= w || !two {
        return vec![fit_text(fonts, name, w)];
    }
    let split = name
        .char_indices()
        .filter(|(_, c)| *c == ' ')
        .map(|(i, _)| (&name[..i], &name[i + 1..]))
        .min_by_key(|(a, b)| {
            fonts
                .measure(FontId::Sans, PX, a)
                .max(fonts.measure(FontId::Sans, PX, b))
        });
    match split {
        Some((a, b)) => vec![fit_text(fonts, a, w), fit_text(fonts, b, w)],
        None => vec![fit_text(fonts, name, w)],
    }
}

/// `s` cut with "…" to `w` pixels at the knob names' size.
fn fit_text(fonts: &mut Fonts, s: &str, w: i32) -> String {
    fonts.face(FontId::Sans).fit(13.0, s, w)
}

fn fit(cx: &mut PaintCx, font: FontId, px: f32, min: f32, s: &str, w: i32) -> f32 {
    let mut size = px;
    while size > min && cx.measure(font, size, s) > w {
        size -= 1.0;
    }
    size
}

/// A knob in the Synth tab's style: the track, the lit arc and the
/// pointer (`colors`: track, arc, pointer).
pub(crate) fn knob(
    cx: &mut PaintCx,
    (x, y): (f32, f32),
    d: f32,
    pos: f32,
    bipolar: bool,
    colors: [Color; 3],
) {
    let [track, lit, pointer] = colors;
    let rad = d * 0.5 - 3.0;
    let width = (d / 9.0).clamp(3.0, 5.0);
    let bbox = Rect::new(
        (x - d * 0.5) as i32 - 2,
        (y - d * 0.5) as i32 - 2,
        d as i32 + 5,
        d as i32 + 5,
    );
    let mut m = Shape::new(bbox);
    m.arc(x, y, rad, A0, A1, width);
    m.draw(cx.canvas, track);
    let at = A0 + (A1 - A0) * pos.clamp(0.0, 1.0);
    let from = if bipolar { 1.5 * PI } else { A0 };
    let (a, b) = if at < from { (at, from) } else { (from, at) };
    if b - a > 0.01 {
        let mut m = Shape::new(bbox);
        m.arc(x, y, rad, a, b, width);
        m.draw(cx.canvas, lit);
    }
    let mut m = Shape::new(bbox);
    m.disc(x + rad * at.cos(), y + rad * at.sin(), width * 0.8);
    m.draw(cx.canvas, pointer);
}

/// Draws the panel in `r`: `values` are the rows' values as text, `names`
/// their names; `selected` is outlined, lit when `editing`.
#[allow(clippy::too_many_arguments)]
pub fn paint(
    cx: &mut PaintCx,
    r: Rect,
    panel: &Panel,
    controls: &[Control],
    names: &[String],
    values: &[String],
    typed: &[usize],
    selected: usize,
    editing: bool,
) {
    let t = cx.theme.clone();
    for (s, sr) in panel.sections.iter().zip(panel.section_rects(r)) {
        cx.canvas.fill_round_rect(sr, t.round(12), t.surface);
        if s.lit {
            cx.canvas.stroke_round_rect(sr, t.round(12), 2, t.accent);
        } else {
            cx.canvas.stroke_round_rect(sr, t.round(12), 1, t.outline);
        }
        let head = sr.inset(PAD).take_top(HEAD_H).0;
        cx.text(
            FontId::Bold,
            12.0,
            head,
            Align::Left,
            t.accent,
            &s.title.to_uppercase(),
        );
        if !s.note.is_empty() {
            let tw = cx.measure(FontId::Bold, 12.0, &s.title.to_uppercase());
            let note = Rect::new(head.x + tw + 8, head.y, (head.w - tw - 8).max(0), head.h);
            cx.text(FontId::Sans, 12.0, note, Align::Right, t.text_dim, &s.note);
        }
        if !s.foot.is_empty() {
            let foot = sr.inset(PAD).take_bottom(FOOT_H).0;
            cx.text(FontId::Sans, 12.0, foot, Align::Center, t.text_dim, &s.foot);
        }
    }
    let ctrl = t.raised;
    for p in panel.place(r, controls) {
        let (Some(c), Some(name), Some(value)) =
            (controls.get(p.row), names.get(p.row), values.get(p.row))
        else {
            continue;
        };
        let ink = if c.faint { t.text_faint } else { t.text };
        let dim = if c.faint { t.text_faint } else { t.text_dim };
        let lit = match (c.faint, c.tint) {
            (true, _) => t.text_faint,
            (false, Some(tint)) => tint,
            (false, None) => t.accent,
        };
        let on = p.row == selected;
        let (label, _) = p.rect.take_top(LABEL_H);
        match &c.look {
            Look::Knob { pos, bipolar, .. } => {
                let lines = knob_name_lines(cx.fonts, name, p.rect.w, p.tall);
                // One line sits on the lower of two, by the knob.
                let first = if p.tall { 2 - lines.len() as i32 } else { 0 };
                for (i, line) in lines.iter().enumerate() {
                    let at = Rect::new(
                        label.x,
                        label.y + (first + i as i32) * LABEL_H,
                        label.w,
                        LABEL_H,
                    );
                    cx.text(
                        FontId::Sans,
                        13.0,
                        at,
                        Align::Center,
                        c.tint
                            .filter(|_| !c.faint)
                            .map_or(dim, |tint| t.readable(tint, t.surface)),
                        line,
                    );
                }
                let name_h = if p.tall { 2 * LABEL_H } else { LABEL_H };
                let mid_x = p.rect.x as f32 + p.rect.w as f32 / 2.0;
                let mid_y = (p.rect.y + name_h + 5) as f32 + KNOB_D as f32 / 2.0;
                knob(
                    cx,
                    (mid_x, mid_y),
                    KNOB_D as f32,
                    *pos,
                    *bipolar,
                    [t.raised.mix(t.outline, 0.5), lit, ink],
                );
                let vr = p.rect.take_bottom(KNOB_VALUE_H).0;
                let px = fit(cx, FontId::Bold, 17.0, 12.0, value, vr.w);
                cx.text(
                    FontId::Bold,
                    px,
                    vr,
                    Align::Center,
                    if on && editing { t.accent } else { ink },
                    value,
                );
            }
            Look::Choice {
                options,
                on: k,
                off,
            }
            | Look::Grid {
                options,
                on: k,
                off,
                ..
            } => {
                if p.control != p.rect {
                    cx.text(FontId::Sans, 13.0, label, Align::Left, dim, name);
                }
                for (i, (o, b)) in options.iter().zip(&p.options).enumerate() {
                    let lit_one = i == *k && !c.faint;
                    cx.canvas.fill_round_rect(
                        *b,
                        t.round(8),
                        if lit_one { t.picked() } else { ctrl },
                    );
                    let fg = if lit_one {
                        t.on_picked()
                    } else if c.faint || off.contains(&i) {
                        t.text_faint
                    } else {
                        t.text
                    };
                    let px = fit(cx, FontId::Bold, 14.0, 11.0, o, b.w - 8);
                    cx.text(FontId::Bold, px, b.inset_xy(4, 0), Align::Center, fg, o);
                }
            }
            Look::Chip | Look::Stepper => {
                if p.control != p.rect {
                    cx.text(FontId::Sans, 13.0, label, Align::Left, dim, name);
                }
                let (less, mid, more) = p.arrows(&c.look);
                let arrows = if on && editing { t.accent } else { dim };
                if matches!(c.look, Look::Stepper) {
                    cx.canvas.fill_round_rect(less, t.round(8), ctrl);
                    cx.canvas.fill_round_rect(more, t.round(8), ctrl);
                    cx.text(FontId::Bold, 20.0, less, Align::Center, ink, "−");
                    cx.text(FontId::Bold, 20.0, more, Align::Center, ink, "+");
                } else {
                    cx.canvas.fill_round_rect(p.control, t.round(8), ctrl);
                    cx.text(FontId::Bold, 18.0, less, Align::Center, arrows, "‹");
                    cx.text(FontId::Bold, 18.0, more, Align::Center, arrows, "›");
                }
                let px = fit(cx, FontId::Bold, 15.0, 11.0, value, mid.w - 4);
                cx.text(
                    FontId::Bold,
                    px,
                    mid,
                    Align::Center,
                    if on && editing { t.accent } else { ink },
                    value,
                );
                if typed.contains(&p.row) && !c.faint {
                    // A dotted line under the value: it can be typed.
                    let y = mid.bottom() - 6;
                    let mut x = mid.x + 8;
                    while x + 4 <= mid.right() - 8 {
                        cx.canvas.fill_rect(Rect::new(x, y, 4, 2), t.text_dim);
                        x += 8;
                    }
                }
            }
            Look::Toggle(state) => {
                let sw = p.switch();
                let text = Rect::new(p.rect.x, p.rect.y, sw.x - p.rect.x - 8, p.rect.h);
                cx.text(FontId::Bold, 14.0, text, Align::Left, ink, name);
                let on_now = *state && !c.faint;
                cx.canvas
                    .fill_round_rect(sw, SWITCH_H / 2, if on_now { t.picked() } else { ctrl });
                let kx = if *state { sw.right() - 24 } else { sw.x + 3 };
                cx.canvas.fill_round_rect(
                    Rect::new(kx, sw.y + 3, 21, 21),
                    10,
                    if c.faint {
                        t.text_faint
                    } else {
                        t.role(Role::Handle)
                    },
                );
            }
            Look::Button(text) | Look::Primary(text) => {
                if p.control != p.rect {
                    cx.text(FontId::Sans, 13.0, label, Align::Left, dim, name);
                }
                let primary = matches!(c.look, Look::Primary(_));
                let (fill, fg, px) = match (primary, c.faint) {
                    (true, false) => (t.accent, t.on_accent, 17.0),
                    (true, true) => (ctrl, ink, 17.0),
                    _ => (ctrl, ink, 14.0),
                };
                cx.canvas.fill_round_rect(p.control, t.round(10), fill);
                let px = fit(cx, FontId::Bold, px, 11.0, text, p.control.w - 16);
                cx.text(
                    FontId::Bold,
                    px,
                    p.control.inset_xy(8, 0),
                    Align::Center,
                    fg,
                    text,
                );
            }
            Look::Pads(cells) => {
                if p.control != p.rect {
                    cx.text(FontId::Sans, 13.0, label, Align::Left, dim, name);
                }
                for (cell, b) in cells.iter().zip(&p.options) {
                    match cell.tint {
                        Some(k) => {
                            let tint = t.track[k % t.track.len()];
                            let tint = if c.faint {
                                tint.mix(t.surface, 0.6)
                            } else {
                                tint
                            };
                            cx.canvas.fill_round_rect(*b, 5, tint);
                            let px = fit(cx, FontId::Bold, 11.0, 9.0, &cell.text, b.w - 4);
                            let ink = t.ink_on(tint);
                            cx.text(FontId::Bold, px, *b, Align::Center, ink, &cell.text);
                        }
                        None => {
                            let fill = if cell.used {
                                ctrl
                            } else {
                                t.bg.mix(t.surface, 0.5)
                            };
                            cx.canvas.fill_round_rect(*b, 5, fill);
                            cx.canvas.stroke_round_rect(*b, 5, 1, t.outline);
                        }
                    }
                    if cell.picked && !c.faint {
                        cx.canvas.stroke_round_rect(
                            b.inset(-2),
                            t.round(6),
                            2,
                            t.role(Role::Highlight),
                        );
                    }
                }
            }
            Look::Slider { pos } => {
                if p.control != p.rect {
                    let (name_r, value_r) = (
                        label.take_left(label.w / 2).0,
                        label.take_right(label.w / 2).0,
                    );
                    cx.text(FontId::Sans, 13.0, name_r, Align::Left, dim, name);
                    cx.text(FontId::Bold, 13.0, value_r, Align::Right, ink, value);
                }
                let (x0, x1) = p.slide();
                let cy = p.control.y + p.control.h / 2;
                cx.canvas
                    .fill_round_rect(Rect::new(x0 - 3, cy - 3, x1 - x0 + 6, 6), 3, ctrl);
                let hx = p.handle_x(*pos);
                if hx > x0 {
                    cx.canvas
                        .fill_round_rect(Rect::new(x0 - 3, cy - 3, hx - x0 + 3, 6), 3, lit);
                }
                cx.canvas.fill_round_rect(
                    Rect::new(hx - HANDLE_D / 2, cy - HANDLE_D / 2, HANDLE_D, HANDLE_D),
                    HANDLE_D / 2,
                    if c.faint {
                        t.text_faint
                    } else {
                        t.role(Role::Handle)
                    },
                );
            }
        }
        if on {
            let ring = match c.look {
                Look::Knob { .. } => p.rect.inset(-5),
                _ => p.control.inset(-4),
            };
            let color = if editing {
                t.accent
            } else {
                t.outline.mix(t.accent, 0.6)
            };
            cx.canvas.stroke_round_rect(ring, t.round(12), 2, color);
        } else if c.outlined {
            let ring = match c.look {
                Look::Knob { .. } => p.rect.inset(-5),
                _ => p.control.inset(-4),
            };
            cx.canvas.stroke_round_rect(ring, t.round(12), 1, t.accent);
        }
    }
}
