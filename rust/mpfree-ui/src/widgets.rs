//! The widget library. Each widget only shows what the app put in its fields
//! and reports touches; none of them knows about MIDI, audio or files.

use crate::canvas::Image;
use crate::color::Color;
use crate::geom::Rect;
use crate::text::{Align, FontId};
use crate::theme::Tone;
use crate::ui::{EventKind, PaintCx, Target, Touch, Widget};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::Arc;

// ---------------------------------------------------------------- header

/// A status pill on the right of the header.
#[derive(Clone, Debug, PartialEq)]
pub struct Chip {
    pub text: String,
    pub tone: Tone,
    /// The widest text the chip shows ("CPU 100 %"): it is always that
    /// wide, so a reading that changes width doesn't nudge the chips
    /// beside it (Jay, 18 Nov 2026 07:28Z).
    pub room: Option<&'static str>,
    /// The text in parts, each in its own ink: a tone, or the plain text
    /// colour for None (the status chip's CPU and RAM numbers, each in
    /// its colour once past its line). Empty: all of `text`, plain. The
    /// parts make up `text`.
    pub inks: Vec<(String, Option<Tone>)>,
    /// Drawn as a ring in `tone` (an outline and a hollow dot) instead of
    /// a filled dot: something happened that hasn't been looked at
    /// (System status).
    pub ring: bool,
    /// Lit, as the folder icon is while the browser shows: the page it
    /// opens is showing.
    pub lit: bool,
    /// The same said in fewer words, for a bar with no room for every
    /// chip in full: the header shows this then, cut with "…" if it is
    /// still too long, so the chip keeps its place, a tap still finds it,
    /// and the chips after it keep theirs where they can. A chip without
    /// one is left out when it doesn't fit, with the chips after it. For
    /// a plain chip (no `inks`).
    pub short: Option<String>,
}

/// The least room, in pixels of text, a chip's short form is shown in
/// (about ten characters): with less the chip is left out.
const SHORT_MIN: i32 = 72;

impl Chip {
    pub fn new(text: impl Into<String>, tone: Tone) -> Chip {
        Chip {
            text: text.into(),
            tone,
            room: None,
            inks: Vec::new(),
            ring: false,
            lit: false,
            short: None,
        }
    }

    /// The chip saying `short` where the bar has no room for every chip
    /// in full.
    pub fn short(mut self, short: impl Into<String>) -> Chip {
        self.short = Some(short.into());
        self
    }

    /// The chip as wide as `widest` (or its text, if that is wider).
    pub fn room(mut self, widest: &'static str) -> Chip {
        self.room = Some(widest);
        self
    }

    /// A chip whose text is `parts`, each in its own ink.
    pub fn parts(parts: Vec<(String, Option<Tone>)>, tone: Tone) -> Chip {
        let text: String = parts.iter().map(|(p, _)| p.as_str()).collect();
        Chip {
            inks: parts,
            ..Chip::new(text, tone)
        }
    }

    pub fn ring(mut self, ring: bool) -> Chip {
        self.ring = ring;
        self
    }

    pub fn lit(mut self, lit: bool) -> Chip {
        self.lit = lit;
        self
    }
}

/// The pad player's tabs, in the order Jay approved (docs/dev/design/layout-spec.md).
/// Every crate's preview draws this bar ([`Header::app`]), never a list of its
/// own; adding, removing, renaming or reordering a tab needs Jay's approval.
/// Pads joined between Play and Arrange on 12 Nov 2026 (Jay, 15:46Z), Mix
/// (the mixer and the effects) last on 14 Nov (Jay's card, 07:15Z), and
/// the Sequencer after Pads on 18 Nov (the Play, Pads and Sequencer
/// redesign, Jay 00:30Z). Jay put them in this order on 18 Nov 2026
/// (07:28Z): Play, Sequencer, Looper, Sample, Synth, Arrange, Pads, Mix.
pub const APP_TABS: [&str; 8] = [
    "Play",
    "Sequencer",
    "Looper",
    "Sample",
    "Synth",
    "Arrange",
    "Pads",
    "Mix",
];
/// The tab after them while Prefs > "Show developer tools" is on.
pub const DEV_TAB: &str = "Dev";

/// The top bar: page tabs and status chips (the MPFree name is on the
/// startup and loading screens only, so the tabs get its room). A tap on a
/// tab gives `Tab(i)`; a tap on a chip gives `Select(i)`, `i` its place in
/// `chips` (chips are laid out from the right edge, so 0 is rightmost).
pub struct Header {
    pub tabs: Vec<String>,
    pub active: usize,
    pub chips: Vec<Chip>,
    /// The small mixer icon (three sliders) at the bar's right edge, after
    /// the chips (layout spec, section 2): `Some(open)`, lit while the
    /// mixer is open. A tap gives `Select(MIXER_ICON)`.
    pub mixer_icon: Option<bool>,
    /// The I/O icon (an arrow in over an arrow out) left of the folder and
    /// mixer icons: `Some(open)`, lit while the mixer's I/O view shows. A
    /// tap gives `Select(IO_ICON)`. The pad player's bar has the folder
    /// icon in its place now (the Mix tab's I/O view and the tempo chip
    /// still open the I/O view).
    pub io_icon: Option<bool>,
    /// The folder icon at the bar's right edge (layout spec, section 2;
    /// Jay's redesign of Play, Pads and the Sequencer tab, approved 18 Nov
    /// 2026): `Some(open)`, lit while the browser shows. A tap gives
    /// `Select(FOLDER_ICON)`, which opens or closes the browser as BROWSE
    /// does.
    pub folder_icon: Option<bool>,
    /// Where each chip was last drawn (chip index, rect), for taps: a chip's
    /// width depends on its text, which only painting can measure.
    drawn_chips: RefCell<Vec<(usize, Rect)>>,
}

/// What a tap on the header's mixer icon gives: `Select(MIXER_ICON)`.
pub const MIXER_ICON: usize = usize::MAX;
/// What a tap on the header's I/O icon gives: `Select(IO_ICON)`.
pub const IO_ICON: usize = usize::MAX - 1;
/// What a tap on the header's folder icon gives: `Select(FOLDER_ICON)`.
pub const FOLDER_ICON: usize = usize::MAX - 2;
/// The mixer icon's size.
const ICON_W: i32 = 44;

/// The room left of the first tab.
const LEFT_W: i32 = 8;
const TAB_W: i32 = 112;
const TAB_GAP: i32 = 6;
/// Room for six full-width tabs.
const TABS_W: i32 = 6 * (TAB_W + TAB_GAP);
/// The narrowest a tab sized to its name is ([`Header::tab_rect`]).
const NAMED_TAB_MIN_W: i32 = 72;

/// How wide a tab sized to its name wants to be: about 11 px a letter at
/// the title size, and room either side.
fn named_tab_w(name: &str) -> i32 {
    (name.chars().count() as i32 * 11 + 22).max(NAMED_TAB_MIN_W)
}

impl Header {
    pub fn new(tabs: &[&str]) -> Header {
        Header {
            tabs: tabs.iter().map(|s| s.to_string()).collect(),
            active: 0,
            chips: Vec::new(),
            mixer_icon: None,
            io_icon: None,
            folder_icon: None,
            drawn_chips: RefCell::new(Vec::new()),
        }
    }

    /// Where the mixer icon sits, when the bar has one.
    pub fn icon_rect(&self, r: Rect) -> Option<Rect> {
        self.mixer_icon?;
        let side = ICON_W.min(r.h);
        Some(Rect::new(
            r.right() - side,
            r.y + (r.h - side) / 2,
            side,
            side,
        ))
    }

    /// Where the folder icon sits, when the bar has one: left of the
    /// mixer icon, or at the right edge without one.
    pub fn folder_icon_rect(&self, r: Rect) -> Option<Rect> {
        self.folder_icon?;
        let right = self.icon_rect(r).map_or(r.right(), |b| b.x - 8);
        let side = ICON_W.min(r.h);
        Some(Rect::new(right - side, r.y + (r.h - side) / 2, side, side))
    }

    /// Where the I/O icon sits, when the bar has one: left of the folder
    /// or mixer icon, or at the right edge without either.
    pub fn io_icon_rect(&self, r: Rect) -> Option<Rect> {
        self.io_icon?;
        let right = self
            .folder_icon_rect(r)
            .or(self.icon_rect(r))
            .map_or(r.right(), |b| b.x - 8);
        let side = ICON_W.min(r.h);
        Some(Rect::new(right - side, r.y + (r.h - side) / 2, side, side))
    }

    /// The pad player's bar ([`APP_TABS`]) with the tab named `active` lit
    /// (none if it is not one of them, as on Prefs).
    pub fn app(active: &str) -> Header {
        let mut h = Header::new(&APP_TABS);
        h.active = APP_TABS
            .iter()
            .position(|t| *t == active)
            .unwrap_or(usize::MAX);
        h
    }

    /// Where chip `i` was last drawn, if it fitted.
    pub fn chip_rect(&self, i: usize) -> Option<Rect> {
        self.drawn_chips
            .borrow()
            .iter()
            .find(|(c, _)| *c == i)
            .map(|(_, r)| *r)
    }

    /// Tab `i`'s place (`i` = the number of tabs: where the tabs end).
    /// Up to six tabs are TAB_W wide. More tabs are each as wide as its
    /// name, all shrunk alike to share six tabs' room, so the chips on the
    /// right keep theirs and a long name ("Sequencer") still fits.
    pub fn tab_rect(&self, r: Rect, i: usize) -> Rect {
        let h = 44.min(r.h);
        let y = r.y + (r.h - h) / 2;
        let n = self.tabs.len();
        if n <= 6 {
            return Rect::new(r.x + LEFT_W + i as i32 * (TAB_W + TAB_GAP), y, TAB_W, h);
        }
        let pitch: Vec<i32> = self.tabs.iter().map(|t| named_tab_w(t) + TAB_GAP).collect();
        let total: i32 = pitch.iter().sum();
        let fit = |w: i32| {
            if total > TABS_W {
                w * TABS_W / total
            } else {
                w
            }
        };
        let x: i32 = pitch[..i.min(n)].iter().map(|&w| fit(w)).sum();
        let w = pitch.get(i).map_or(0, |&w| fit(w) - TAB_GAP);
        Rect::new(r.x + LEFT_W + x, y, w, h)
    }
}

impl Widget for Header {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        for (i, name) in self.tabs.iter().enumerate() {
            let tr = self.tab_rect(r, i);
            let on = i == self.active;
            if on {
                cx.canvas.fill_round_rect(tr, t.round(10), t.raised);
                let bar = Rect::new(tr.x + 20, tr.bottom() - 4, tr.w - 40, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let color = if on { t.text } else { t.text_dim };
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_title,
                tr,
                Align::Center,
                color,
                name,
            );
        }
        // The mixer icon at the right edge: three sliders.
        if let (Some(b), Some(open)) = (self.icon_rect(r), self.mixer_icon) {
            cx.canvas
                .fill_round_rect(b, t.round(10), if open { t.raised } else { t.surface });
            if open {
                let bar = Rect::new(b.x + 10, b.bottom() - 4, b.w - 20, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let ink = if open { t.text } else { t.text_dim };
            let (top, h) = (b.y + 11, b.h - 22);
            for (i, knob) in [0.35f32, 0.75, 0.5].iter().enumerate() {
                let x = b.x + 12 + i as i32 * 9;
                cx.canvas
                    .fill_rect(Rect::new(x, top, 2, h), ink.mix(t.surface, 0.4));
                let ky = top + ((h - 6) as f32 * (1.0 - knob)) as i32;
                cx.canvas
                    .fill_round_rect(Rect::new(x - 3, ky, 8, 6), 2, ink);
            }
        }
        // The I/O icon left of it: an arrow in (pointing right) over an
        // arrow out (pointing left).
        if let (Some(b), Some(open)) = (self.io_icon_rect(r), self.io_icon) {
            cx.canvas
                .fill_round_rect(b, t.round(10), if open { t.raised } else { t.surface });
            if open {
                let bar = Rect::new(b.x + 10, b.bottom() - 4, b.w - 20, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let ink = if open { t.text } else { t.text_dim };
            let (x0, x1) = (b.x + 10, b.right() - 10);
            for (y, right) in [(b.y + 15, true), (b.bottom() - 17, false)] {
                cx.canvas.fill_rect(Rect::new(x0, y, x1 - x0, 2), ink);
                for k in 0..5 {
                    let h = 2 + 2 * (4 - k);
                    let x = if right { x1 - 5 + k } else { x0 + 4 - k };
                    cx.canvas.fill_rect(Rect::new(x, y + 1 - h / 2, 1, h), ink);
                }
            }
        }
        // The folder icon: a folder's outline, its tab at the top left.
        if let (Some(b), Some(open)) = (self.folder_icon_rect(r), self.folder_icon) {
            let well = if open { t.raised } else { t.surface };
            cx.canvas.fill_round_rect(b, t.round(10), well);
            if open {
                let bar = Rect::new(b.x + 10, b.bottom() - 4, b.w - 20, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let ink = if open { t.text } else { t.text_dim };
            let (w, h) = (24, 18);
            let (x0, y0) = (b.x + (b.w - w) / 2, b.y + (b.h - h) / 2 + 1);
            cx.canvas
                .fill_round_rect(Rect::new(x0, y0 - 3, 11, 6), 2, ink);
            cx.canvas.fill_round_rect(Rect::new(x0, y0, w, h), 3, ink);
            cx.canvas
                .fill_round_rect(Rect::new(x0 + 2, y0 + 4, w - 4, h - 6), 2, well);
        }
        // Chips from the right edge (or the icons) leftwards; stop before
        // the tabs.
        let limit = self.tab_rect(r, self.tabs.len()).x;
        let mut right = self
            .io_icon_rect(r)
            .or(self.folder_icon_rect(r))
            .or(self.icon_rect(r))
            .map_or(r.right(), |b| b.x - 8);
        let h = 34.min(r.h);
        let mut drawn = self.drawn_chips.borrow_mut();
        drawn.clear();
        // Each chip's text as wide as it is in full (or as its fixed room).
        let full: Vec<i32> = self
            .chips
            .iter()
            .map(|chip| {
                let tw = cx.measure(FontId::Sans, t.size_small, &chip.text);
                chip.room
                    .map_or(tw, |r| tw.max(cx.measure(FontId::Sans, t.size_small, r)))
            })
            .collect();
        // With no room for every chip in full, the chips that have a short
        // form say that, so that as few chips as can be are left out.
        let all: i32 = full.iter().map(|tw| tw + 40 + 8).sum::<i32>() - 8;
        let crowded = right - all < limit;
        for (i, chip) in self.chips.iter().enumerate() {
            let short = chip
                .short
                .as_deref()
                .filter(|_| crowded && chip.inks.is_empty());
            let said = short.unwrap_or(chip.text.as_str());
            let mut tw = match short {
                Some(s) => cx.measure(FontId::Sans, t.size_small, s),
                None => full[i],
            };
            if right - (tw + 40) < limit {
                // Still no room: a short form is cut to the room left;
                // any other chip is left out, with the chips after it.
                let room = right - limit - 40;
                if short.is_none() || room < SHORT_MIN {
                    break;
                }
                tw = room;
            }
            let w = tw + 40;
            let cr = Rect::new(right - w, r.y + (r.h - h) / 2, w, h);
            drawn.push((i, cr));
            let well = if chip.lit { t.raised } else { t.surface };
            let tone = t.tone(chip.tone);
            if chip.ring {
                // An outline in the tone, and a hollow dot.
                cx.canvas.fill_round_rect(cr, h / 2, tone);
                let inner = Rect::new(cr.x + 2, cr.y + 2, cr.w - 4, cr.h - 4);
                cx.canvas.fill_round_rect(inner, inner.h / 2, well);
            } else {
                cx.canvas.fill_round_rect(cr, h / 2, well);
            }
            if chip.lit {
                let bar = Rect::new(cr.x + 14, cr.bottom() - 4, cr.w - 28, 3);
                cx.canvas.fill_round_rect(bar, 1, t.accent);
            }
            let dot = Rect::new(cr.x + 14, cr.y + h / 2 - 4, 8, 8);
            cx.canvas.fill_round_rect(dot, 4, tone);
            if chip.ring {
                let hole = Rect::new(dot.x + 2, dot.y + 2, 4, 4);
                cx.canvas.fill_round_rect(hole, 2, well);
            }
            if chip.inks.is_empty() {
                let text = Rect::new(cr.x + 28, cr.y, tw + 4, h);
                cx.text(FontId::Sans, t.size_small, text, Align::Left, t.text, said);
            } else {
                let mut x = cr.x + 28;
                for (part, ink) in &chip.inks {
                    let pw = cx.measure(FontId::Sans, t.size_small, part);
                    let ink = ink.map_or(t.text, |k| t.tone(k));
                    let text = Rect::new(x, cr.y, pw + 4, h);
                    cx.text(FontId::Sans, t.size_small, text, Align::Left, ink, part);
                    x += pw;
                }
            }
            right -= w + 8;
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = (0..self.tabs.len())
            .map(|i| Target::tap("tab", i, self.tabs[i].clone(), self.tab_rect(r, i)))
            .collect();
        if let Some(b) = self.icon_rect(r) {
            out.push(Target::tap("mixer-icon", 0, "mixer", b));
        }
        if let Some(b) = self.folder_icon_rect(r) {
            out.push(Target::tap("folder-icon", 0, "browser", b));
        }
        if let Some(b) = self.io_icon_rect(r) {
            out.push(Target::tap("io-icon", 0, "I/O", b));
        }
        for (i, cr) in self.drawn_chips.borrow().iter() {
            let label = self.chips.get(*i).map_or("", |c| c.text.as_str());
            out.push(Target::tap("chip", *i, label, *cr));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if let Some(i) = (0..self.tabs.len()).find(|&i| self.tab_rect(r, i).contains(x, y)) {
            self.active = i;
            return Some(EventKind::Tab(i));
        }
        if self.icon_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Select(MIXER_ICON));
        }
        if self.folder_icon_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Select(FOLDER_ICON));
        }
        if self.io_icon_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Select(IO_ICON));
        }
        let drawn = self.drawn_chips.borrow();
        drawn
            .iter()
            .find(|(_, cr)| cr.contains(x, y))
            .map(|(i, _)| EventKind::Select(*i))
    }
}

// ---------------------------------------------------------------- pads

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pad {
    /// What the pad plays; empty if nothing.
    pub label: String,
    /// The pad's colour, or None for a pad with nothing on it.
    pub color: Option<Color>,
    /// 0 at rest, 1 when just hit; the app fades it back with `PadGrid::fade`.
    pub level: f32,
    pub held: bool,
    /// Velocity of the last hit, 0-127.
    pub velocity: u8,
}

/// The 4 x 4 pads, laid out as on the MPC: pad 1 bottom left, pad 16 top right.
pub struct PadGrid {
    pub pads: Vec<Pad>,
    pub cols: usize,
    fingers: Vec<(u64, usize)>,
    /// The pads as last drawn, so a change redraws only the pads that changed.
    drawn: RefCell<Option<(Rect, Vec<Pad>)>>,
}

const PAD_GAP: i32 = 12;
/// Pads shorter than this show their name only, without the number.
const SMALL_PAD_H: i32 = 70;

impl PadGrid {
    pub fn new(count: usize, cols: usize) -> PadGrid {
        assert!(cols > 0 && count > 0 && count.is_multiple_of(cols));
        PadGrid {
            pads: vec![Pad::default(); count],
            cols,
            fingers: Vec::new(),
            drawn: RefCell::new(None),
        }
    }

    fn rows(&self) -> usize {
        self.pads.len() / self.cols
    }

    /// The rectangle of pad `i` (0-based) inside `r`.
    pub fn cell(&self, r: Rect, i: usize) -> Rect {
        let (cols, rows) = (self.cols as i32, self.rows() as i32);
        let w = (r.w - PAD_GAP * (cols - 1)) / cols;
        let h = (r.h - PAD_GAP * (rows - 1)) / rows;
        let col = (i % self.cols) as i32;
        let row_from_bottom = (i / self.cols) as i32;
        let row = rows - 1 - row_from_bottom;
        Rect::new(r.x + col * (w + PAD_GAP), r.y + row * (h + PAD_GAP), w, h)
    }

    /// True while some pad is still fading.
    pub fn fading(&self) -> bool {
        self.pads.iter().any(|p| !p.held && p.level > 0.0)
    }

    /// Fade released pads by `dt` seconds (a hit fades over about 0.35 s).
    pub fn fade(&mut self, dt: f32) {
        for p in self.pads.iter_mut().filter(|p| !p.held) {
            p.level = (p.level - dt / 0.35).max(0.0);
        }
    }

    pub fn hit(&mut self, i: usize, velocity: u8) {
        if let Some(p) = self.pads.get_mut(i) {
            p.held = true;
            p.level = 1.0;
            p.velocity = velocity;
        }
    }

    pub fn release(&mut self, i: usize) {
        if let Some(p) = self.pads.get_mut(i) {
            p.held = false;
        }
    }
}

impl Widget for PadGrid {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let (at, old) = drawn.as_ref()?;
        if *at != r || old.len() != self.pads.len() {
            return None;
        }
        Some(
            (0..self.pads.len())
                .filter(|&i| old[i] != self.pads[i])
                .map(|i| self.cell(r, i))
                .collect(),
        )
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        *self.drawn.borrow_mut() = Some((r, self.pads.clone()));
        let clip = cx.canvas.clip();
        for (i, pad) in self.pads.iter().enumerate() {
            let cell = self.cell(r, i);
            if cell.intersect(clip).is_empty() {
                continue;
            }
            let body = cell.inset(3);
            let lit = if pad.held { 1.0 } else { pad.level };
            let color = pad.color.map(|c| t.content(c));
            let (fill, ink, sub) = match color {
                Some(c) => {
                    let fill = t.surface.mix(c, 0.20 + 0.80 * lit);
                    // The name in whichever text reads on the lit pad; the
                    // line under it quieter, in the pad's colour while the
                    // pad is dim.
                    let ink = t.ink_on(fill);
                    let sub = if ink == t.text {
                        t.readable(c.mix(t.text, 0.35), fill)
                    } else {
                        ink.with_alpha(170)
                    };
                    (fill, ink, sub)
                }
                None => (t.surface, t.text_faint, t.text_faint),
            };
            if let (Some(c), true) = (color, lit > 0.0) {
                cx.canvas.stroke_round_rect(
                    cell,
                    t.radius + 3,
                    3,
                    c.with_alpha((150.0 * lit) as u8),
                );
            }
            cx.canvas.fill_round_rect(body, t.radius, fill);
            if pad.color.is_none() {
                cx.canvas.stroke_round_rect(body, t.radius, 1, t.outline);
            }
            let label = if pad.label.is_empty() {
                "—"
            } else {
                pad.label.as_str()
            };
            if body.h < SMALL_PAD_H {
                // A small pad (the Play page's pads panel): the name only.
                let inner = body.inset_xy(8, 4);
                cx.text(FontId::Sans, t.size_small, inner, Align::Center, ink, label);
            } else {
                let inner = body.inset(12);
                let (top, rest) = inner.take_top(22);
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    top,
                    Align::Left,
                    sub,
                    &format!("{}", i + 1),
                );
                let (bottom, _) = rest.take_bottom(26);
                cx.text(FontId::Sans, t.size_body, bottom, Align::Left, ink, label);
            }
            if lit > 0.0 && pad.velocity > 0 {
                let w = ((body.w - 24) as f32 * pad.velocity as f32 / 127.0) as i32;
                let bar = Rect::new(body.x + 12, body.bottom() - 9, w.max(4), 4);
                cx.canvas
                    .fill_round_rect(bar, 2, ink.with_alpha((200.0 * lit) as u8));
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        (0..self.pads.len())
            .map(|i| Target::tap("pad", i, self.pads[i].label.clone(), self.cell(r, i)))
            .collect()
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                let i = (0..self.pads.len()).find(|&i| self.cell(r, i).contains(x, y))?;
                self.fingers.push((finger, i));
                Some(EventKind::Pad {
                    index: i,
                    down: true,
                })
            }
            Touch::Move => None,
            Touch::Up | Touch::Cancel => {
                let k = self.fingers.iter().position(|(f, _)| *f == finger)?;
                let (_, i) = self.fingers.remove(k);
                Some(EventKind::Pad {
                    index: i,
                    down: false,
                })
            }
        }
    }
}

// ---------------------------------------------------------------- list

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListItem {
    pub text: String,
    pub detail: String,
    pub color: Option<Color>,
}

/// A titled list with one selected row, kept in view.
pub struct ListView {
    pub title: String,
    pub items: Vec<ListItem>,
    pub selected: usize,
    /// A tap only picks a row (the kit's sounds), so the row picked already
    /// changes nothing when tapped. Off for lists whose tap loads (kits,
    /// presets): a tap on the row playing loads it again.
    pub picks_only: bool,
}

const ROW_H: i32 = 46;

impl ListView {
    pub fn new(title: &str) -> ListView {
        ListView {
            title: title.to_string(),
            items: Vec::new(),
            selected: 0,
            picks_only: false,
        }
    }

    /// The area for rows: the panel without its title line.
    fn body(r: Rect) -> Rect {
        // Matches PaintCx::titled_panel with the default theme's padding.
        r.inset(14).take_top(34).1
    }

    fn first(&self, visible: usize) -> usize {
        if self.items.len() <= visible || visible == 0 {
            return 0;
        }
        let half = visible / 2;
        self.selected
            .saturating_sub(half)
            .min(self.items.len() - visible)
    }
}

impl Widget for ListView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let count = if self.items.is_empty() {
            String::new()
        } else {
            format!("{} / {}", self.selected + 1, self.items.len())
        };
        let body = cx.titled_panel(r, &self.title, &count);
        let visible = (body.h / ROW_H).max(0) as usize;
        let first = self.first(visible);
        for (k, item) in self.items.iter().enumerate().skip(first).take(visible) {
            let row = Rect::new(
                body.x,
                body.y + (k - first) as i32 * ROW_H,
                body.w,
                ROW_H - 4,
            );
            if k == self.selected {
                cx.canvas.fill_round_rect(row, t.round(10), t.raised);
                cx.canvas
                    .fill_round_rect(Rect::new(row.x, row.y + 10, 4, row.h - 20), 2, t.accent);
            }
            let mut text = row.inset_xy(14, 0);
            if let Some(c) = item.color.map(|c| t.content(c)) {
                cx.canvas
                    .fill_round_rect(Rect::new(text.x, row.y + row.h / 2 - 6, 12, 12), 4, c);
                text = text.take_left(22).1;
            }
            let dw = if item.detail.is_empty() {
                0
            } else {
                cx.measure(FontId::Sans, t.size_small, &item.detail) + 12
            };
            let (detail, name) = text.take_right(dw);
            let font = if k == self.selected {
                FontId::Bold
            } else {
                FontId::Sans
            };
            cx.text(font, t.size_body, name, Align::Left, t.text, &item.text);
            cx.text(
                FontId::Sans,
                t.size_small,
                detail,
                Align::Right,
                t.text_dim,
                &item.detail,
            );
        }
        if self.items.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_body,
                body.take_top(ROW_H).0,
                Align::Left,
                t.text_faint,
                "nothing here",
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let body = ListView::body(r);
        let visible = (body.h / ROW_H).max(0) as usize;
        let first = self.first(visible);
        (first..self.items.len().min(first + visible))
            .map(|k| {
                let row = Rect::new(body.x, body.y + (k - first) as i32 * ROW_H, body.w, ROW_H);
                Target::tap("row", k, self.items[k].text.clone(), row)
                    .disabled_if(self.picks_only && k == self.selected)
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let body = ListView::body(r);
        if !body.contains(x, y) {
            return None;
        }
        let visible = (body.h / ROW_H).max(0) as usize;
        let k = self.first(visible) + ((y - body.y) / ROW_H) as usize;
        if k < self.items.len() && k < self.first(visible) + visible {
            self.selected = k;
            Some(EventKind::Select(k))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------- fader

/// A horizontal slider with a label and a value text.
pub struct Fader {
    pub label: String,
    /// 0.0 to 1.0.
    pub value: f32,
    pub text: String,
}

impl Fader {
    pub fn new(label: &str) -> Fader {
        Fader {
            label: label.to_string(),
            value: 0.0,
            text: String::new(),
        }
    }

    fn track(r: Rect) -> Rect {
        let inner = r.inset(14);
        let body = inner.take_top(34).1;
        let y = body.y + (body.h - 10) / 2;
        Rect::new(body.x + 11, y.max(body.y), (body.w - 22).max(1), 10)
    }
}

impl Widget for Fader {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let inner = r.inset(t.pad);
        let (head, _) = inner.take_top(26);
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            &self.label.to_uppercase(),
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            head,
            Align::Right,
            t.text,
            &self.text,
        );
        let tr = Fader::track(r);
        cx.canvas.fill_round_rect(tr, 5, t.raised);
        let v = self.value.clamp(0.0, 1.0);
        let fill = Rect::new(tr.x, tr.y, ((tr.w as f32) * v) as i32, tr.h);
        cx.canvas.fill_round_rect(fill, 5, t.accent);
        let kx = tr.x + fill.w;
        cx.canvas
            .fill_round_rect(Rect::new(kx - 11, tr.y - 6, 22, 22), 11, t.text);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        vec![Target::drag("slider", 0, self.label.clone(), r)]
    }

    fn steps(&self, _t: &Target) -> bool {
        true
    }

    /// Held with SHIFT: a thousandth of the travel per detent.
    fn held_step(&mut self, t: &Target, delta: i32, fine: bool, r: Rect) -> Option<EventKind> {
        if !fine {
            return self.step(t, delta, r);
        }
        let v = (self.value + delta as f32 * 0.001).clamp(0.0, 1.0);
        (v != self.value).then(|| {
            self.value = v;
            EventKind::Value(v)
        })
    }

    /// One hundredth of the travel per detent.
    fn step(&mut self, _t: &Target, delta: i32, _r: Rect) -> Option<EventKind> {
        let v = (self.value + delta as f32 * 0.01).clamp(0.0, 1.0);
        (v != self.value).then(|| {
            self.value = v;
            EventKind::Value(v)
        })
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, _y: i32, r: Rect) -> Option<EventKind> {
        if !matches!(phase, Touch::Down | Touch::Move) {
            return None;
        }
        let tr = Fader::track(r);
        self.value = ((x - tr.x) as f32 / tr.w as f32).clamp(0.0, 1.0);
        Some(EventKind::Value(self.value))
    }
}

// ---------------------------------------------------------------- stats

/// A titled list of name/value rows.
pub struct StatList {
    pub title: String,
    pub rows: Vec<(String, String, Tone)>,
}

impl StatList {
    pub fn new(title: &str) -> StatList {
        StatList {
            title: title.to_string(),
            rows: Vec::new(),
        }
    }

    pub fn set(&mut self, rows: Vec<(String, String, Tone)>) {
        self.rows = rows;
    }
}

impl Widget for StatList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let mut body = cx.titled_panel(r, &self.title, "");
        for (k, v, tone) in &self.rows {
            let (line, rest) = body.take_top(26);
            if line.h < 20 {
                break;
            }
            cx.text(FontId::Sans, t.size_small, line, Align::Left, t.text_dim, k);
            let kw = cx.measure(FontId::Sans, t.size_small, k) + 16;
            cx.text(
                FontId::Bold,
                t.size_small,
                line.take_left(kw).1,
                Align::Right,
                t.tone(*tone),
                v,
            );
            body = rest;
        }
    }
}

// ---------------------------------------------------------------- log

/// A titled, scrolling list of lines, newest at the bottom.
pub struct LogPanel {
    pub title: String,
    pub right: String,
    pub mono: bool,
    lines: VecDeque<(String, Tone)>,
    max: usize,
}

impl LogPanel {
    pub fn new(title: &str, mono: bool, max: usize) -> LogPanel {
        LogPanel {
            title: title.to_string(),
            right: String::new(),
            mono,
            lines: VecDeque::new(),
            max: max.max(1),
        }
    }

    pub fn push(&mut self, line: &str, tone: Tone) {
        if self.lines.len() >= self.max {
            self.lines.pop_front();
        }
        let clean: String = line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        self.lines.push_back((clean, tone));
    }

    pub fn lines(&self) -> impl Iterator<Item = &(String, Tone)> {
        self.lines.iter()
    }
}

impl Widget for LogPanel {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let body = cx.titled_panel(r, &self.title, &self.right);
        let (font, px) = if self.mono {
            (FontId::Mono, t.size_mono)
        } else {
            (FontId::Sans, t.size_small)
        };
        let lh = (px * 1.45).round() as i32;
        let visible = (body.h / lh).max(0) as usize;
        let skip = self.lines.len().saturating_sub(visible);
        for (k, (line, tone)) in self.lines.iter().skip(skip).enumerate() {
            let row = Rect::new(body.x, body.y + k as i32 * lh, body.w, lh);
            cx.text(font, px, row, Align::Left, t.tone(*tone), line);
        }
    }
}

// ---------------------------------------------------------------- label

/// One line of text, no panel.
pub struct Label {
    pub text: String,
    pub tone: Tone,
    pub align: Align,
    /// Room left and right of the text, px.
    pub inset: i32,
    /// The label sits on the page's own background (`bg`), so a status
    /// tone (good, warn, bad) is kept readable there: a theme whose red
    /// doesn't read on its background gets it moved just far enough
    /// ([`crate::Theme::readable`]). The hint line is one.
    pub on_bg: bool,
}

impl Label {
    pub fn new(text: &str, tone: Tone, align: Align) -> Label {
        Label {
            text: text.to_string(),
            tone,
            align,
            inset: 6,
            on_bg: false,
        }
    }

    /// The same with `inset` px of room either side.
    pub fn inset(mut self, inset: i32) -> Label {
        self.inset = inset;
        self
    }

    /// The same, on the page's background ([`Label::on_bg`]).
    pub fn on_bg(mut self) -> Label {
        self.on_bg = true;
        self
    }
}

impl Widget for Label {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let (px, mut c) = (cx.theme.size_small, cx.theme.tone(self.tone));
        if self.on_bg && matches!(self.tone, Tone::Good | Tone::Warn | Tone::Bad) {
            c = cx.theme.readable(c, cx.theme.bg);
        }
        cx.text(
            FontId::Sans,
            px,
            r.inset_xy(self.inset, 0),
            self.align,
            c,
            &self.text,
        );
    }
}

// ---------------------------------------------------------------- loading

/// The full-screen loading page: a picture (the startup screen's last frame,
/// or a starry one drawn here with the title), and a card at the bottom with
/// what is happening, a progress bar and a hint. A change to the card redraws
/// only the card.
pub struct Loading {
    backdrop: Option<Arc<Image>>,
    backdrop_version: u64,
    /// Drawn over the starry backdrop only (a picture brings its own).
    pub title: String,
    pub subtitle: String,
    /// What is happening now, in plain words.
    pub message: String,
    /// A second, quieter line: the step, a count, or what went wrong.
    pub detail: String,
    /// 0.0 to 1.0.
    pub progress: f32,
    pub failed: bool,
    pub hint: String,
    drawn: RefCell<Option<(Rect, u64)>>,
}

const CARD_W: i32 = 720;
const CARD_H: i32 = 128;

impl Loading {
    pub fn new(title: &str, subtitle: &str) -> Loading {
        Loading {
            backdrop: None,
            backdrop_version: 0,
            title: title.to_string(),
            subtitle: subtitle.to_string(),
            message: String::new(),
            detail: String::new(),
            progress: 0.0,
            failed: false,
            hint: String::new(),
            drawn: RefCell::new(None),
        }
    }

    /// Show `img` (the size of the page) behind the card, or the starry
    /// backdrop with the title if None.
    pub fn set_backdrop(&mut self, img: Option<Arc<Image>>) {
        self.backdrop = img;
        self.backdrop_version += 1;
    }

    pub fn has_backdrop(&self) -> bool {
        self.backdrop.is_some()
    }

    /// The card, and the hint line under it.
    pub fn card(r: Rect) -> Rect {
        let w = CARD_W.min(r.w - 48).max(0);
        Rect::new(r.x + (r.w - w) / 2, r.bottom() - 64 - CARD_H, w, CARD_H)
    }

    fn changing_part(r: Rect) -> Rect {
        let c = Loading::card(r);
        Rect::new(c.x - 4, c.y - 4, c.w + 8, r.bottom() - c.y + 4).intersect(r)
    }

    fn starry(cx: &mut PaintCx, r: Rect, title: &str, subtitle: &str) {
        let t = cx.theme.clone();
        let clip = cx.canvas.clip();
        let top = t.bg;
        let bottom = t.bg.mix(t.backdrop_to, 0.9);
        let rows = r.intersect(clip);
        for y in rows.y..rows.bottom() {
            let k = (y - r.y) as f32 / r.h.max(1) as f32;
            cx.canvas
                .fill_rect(Rect::new(rows.x, y, rows.w, 1), top.mix(bottom, k));
        }
        // Stars at fixed places (a small hash, so every frame matches).
        let mut seed: u32 = 0x9E37_79B9;
        for _ in 0..220 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let x = r.x + (seed % r.w.max(1) as u32) as i32;
            let y = r.y + ((seed >> 11) % r.h.max(1) as u32) as i32;
            let size = 1 + (seed >> 27) as i32 % 2;
            let glow = 90 + (seed >> 20) % 150;
            cx.canvas
                .fill_rect(Rect::new(x, y, size, size), t.text.with_alpha(glow as u8));
        }
        if !title.is_empty() {
            let mid = Rect::new(r.x, r.y + r.h / 2 - 150, r.w, 120);
            let tw = cx.measure(FontId::Bold, 96.0, title);
            // "MP" in the text colour and the rest in the accent.
            let split = title
                .char_indices()
                .nth(2)
                .map(|(i, _)| i)
                .unwrap_or(title.len());
            let (a, b) = title.split_at(split);
            let aw = cx.measure(FontId::Bold, 96.0, a);
            let x0 = mid.x + (mid.w - tw) / 2;
            cx.text(
                FontId::Bold,
                96.0,
                Rect::new(x0, mid.y, aw + 4, mid.h),
                Align::Left,
                t.text,
                a,
            );
            cx.text(
                FontId::Bold,
                96.0,
                Rect::new(x0 + aw, mid.y, tw - aw + 8, mid.h),
                Align::Left,
                t.accent,
                b,
            );
            let sub = Rect::new(r.x, mid.bottom(), r.w, 40);
            cx.text(
                FontId::Sans,
                t.size_title + 3.0,
                sub,
                Align::Center,
                t.text_dim,
                subtitle,
            );
        }
    }
}

impl Widget for Loading {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        match *self.drawn.borrow() {
            Some((at, version)) if at == r && version == self.backdrop_version => {
                Some(vec![Loading::changing_part(r)])
            }
            _ => None,
        }
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let clip = cx.canvas.clip();
        if clip.intersect(r) == r {
            *self.drawn.borrow_mut() = Some((r, self.backdrop_version));
        }
        match &self.backdrop {
            Some(img) => cx.canvas.draw_image(r.x, r.y, img),
            None => Loading::starry(cx, r, &self.title, &self.subtitle),
        }
        let card = Loading::card(r);
        cx.canvas
            .fill_round_rect(card, t.radius + 4, t.surface.with_alpha(232));
        cx.canvas
            .stroke_round_rect(card, t.radius + 4, 1, t.outline);
        let inner = card.inset_xy(28, 22);
        let (head, rest) = inner.take_top(30);
        let pct = format!("{:.0} %", (self.progress.clamp(0.0, 1.0) * 100.0).floor());
        let pw = cx.measure(FontId::Sans, t.size_body, &pct) + 16;
        let (pct_r, msg_r) = head.take_right(pw);
        let ink = if self.failed { t.bad } else { t.text };
        cx.text(
            FontId::Bold,
            t.size_title + 3.0,
            msg_r,
            Align::Left,
            ink,
            &self.message,
        );
        if !self.failed {
            cx.text(
                FontId::Sans,
                t.size_body,
                pct_r,
                Align::Right,
                t.text_dim,
                &pct,
            );
        }
        let (_, rest) = rest.take_top(12);
        let (bar, rest) = rest.take_top(10);
        cx.canvas.fill_round_rect(bar, 5, t.raised);
        let fill = Rect::new(
            bar.x,
            bar.y,
            ((bar.w as f32) * self.progress.clamp(0.0, 1.0)) as i32,
            bar.h,
        );
        if fill.w > 0 {
            cx.canvas
                .fill_round_rect(fill, 5, if self.failed { t.bad } else { t.accent });
        }
        let (_, rest) = rest.take_top(12);
        let (detail, _) = rest.take_top(24);
        cx.text(
            FontId::Sans,
            t.size_small + 1.0,
            detail,
            Align::Left,
            t.text_dim,
            &self.detail,
        );
        let hint = Rect::new(r.x, card.bottom() + 14, r.w, 26);
        cx.text(
            FontId::Sans,
            t.size_small + 1.0,
            hint,
            Align::Center,
            t.text_dim,
            &self.hint,
        );
    }
}

// ---------------------------------------------------------------- card

/// A titled panel with a large heading, lines of text and buttons along the
/// bottom. A tap on button `i` gives `Tab(i)`.
pub struct Card {
    pub title: String,
    pub heading: String,
    pub lines: Vec<(String, Tone)>,
    pub buttons: Vec<String>,
}

const BUTTON_W: i32 = 200;
const BUTTON_H: i32 = 54;

impl Card {
    pub fn new(title: &str) -> Card {
        Card {
            title: title.to_string(),
            heading: String::new(),
            lines: Vec::new(),
            buttons: Vec::new(),
        }
    }

    pub fn button_rect(&self, r: Rect, i: usize) -> Rect {
        let inner = r.inset(14);
        Rect::new(
            inner.x + i as i32 * (BUTTON_W + 12),
            inner.bottom() - BUTTON_H,
            BUTTON_W,
            BUTTON_H,
        )
    }
}

impl Widget for Card {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let body = cx.titled_panel(r, &self.title, "");
        let (head, mut rest) = body.take_top(52);
        cx.text(FontId::Bold, 32.0, head, Align::Left, t.text, &self.heading);
        rest = rest.take_top(8).1;
        let limit = if self.buttons.is_empty() {
            r.bottom()
        } else {
            self.button_rect(r, 0).y - 8
        };
        for (line, tone) in &self.lines {
            let (row, after) = rest.take_top(30);
            if row.bottom() > limit || row.h < 24 {
                break;
            }
            cx.text(
                FontId::Sans,
                t.size_body,
                row,
                Align::Left,
                t.tone(*tone),
                line,
            );
            rest = after;
        }
        for (i, label) in self.buttons.iter().enumerate() {
            let b = self.button_rect(r, i);
            cx.canvas.fill_round_rect(b, t.round(12), t.raised);
            cx.text(FontId::Bold, t.size_body, b, Align::Center, t.text, label);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        (0..self.buttons.len())
            .map(|i| Target::tap("button", i, self.buttons[i].clone(), self.button_rect(r, i)))
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        (0..self.buttons.len())
            .find(|&i| self.button_rect(r, i).contains(x, y))
            .map(EventKind::Tab)
    }
}

// ---------------------------------------------------------------- options

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionRow {
    pub name: String,
    pub value: String,
}

/// A titled list of settings, each with a value and arrows to step it. A tap
/// on a row's name selects it (`Select`, when the finger lifts); a tap on an
/// arrow steps its value (`Step`, at once).
///
/// A list longer than its panel scrolls (Jay's final round, 16 Nov: Prefs
/// needed a scroll bar and continuous scrolling, not page jumps). The view
/// moves only as far as it must to keep the picked row and one more in
/// sight, so the wheel scrolls it a row at a time and a tap never jumps it;
/// a finger dragged up or down the list scrolls it, row by row with the
/// finger, and picks nothing. A thin bar at the right shows where the view
/// is.
///
/// A narrow list (rows under [`NARROW_ROW_W`] pixels, such as Arrange's
/// Record panel beside the Q-Link column) keeps every row readable: the
/// text is 15 px, the value box takes a little over half the row, its
/// arrows are drawn thin, and a tap on its left or right third steps the
/// value.
///
/// With `tabs`, the title line is a row of tabs instead (Arrange's Record
/// and Clip, sequencer S3): the tab `tab` is lit, and a tap on another
/// gives `Tab(k)` at once. The last tab takes the room the others leave,
/// its name cut with "…" if it is longer; the title and `right` are not
/// shown.
///
/// With `buttons`, a row of buttons sits under the rows, side by side
/// (the clip card's Save state and Reload, sequencer S4): a tap on button
/// `i` gives `Toggle(i)` at once, unless it is drawn off (and doesn't
/// [`OptionButton::explains`]).
///
/// With a `note`, two lines sit between the rows and the buttons (an
/// audio clip's made-at line over Fit to tempo, layout spec 4j).
pub struct OptionList {
    pub title: String,
    pub right: String,
    pub tabs: Vec<String>,
    pub tab: usize,
    /// Tab `k`'s width, but the last's; [`OPTION_TAB_W`] for those it
    /// doesn't give. Each name is cut with "…" to its tab.
    pub tab_widths: Vec<i32>,
    pub rows: Vec<OptionRow>,
    /// The buttons under the rows; none to begin with.
    pub buttons: Vec<OptionButton>,
    /// Two lines over the buttons; none to begin with.
    pub note: Option<OptionNote>,
    /// What the list says with no rows ("nothing here" to begin with).
    pub empty: String,
    /// A Back button at the left of the title line ("‹ Back"), for a
    /// finger to leave the list as MENU does (Jay's device notes, 20 Nov
    /// 2026); a tap gives `Tab(OPTION_BACK)`. Not drawn with tabs.
    pub back: Option<String>,
    pub selected: usize,
    /// The selected row is open for the wheel: its value box is outlined
    /// and its arrows lit, until the wheel's push sets it.
    pub editing: bool,
    /// The first row in view, and the selection it last followed (a drag
    /// moves the view off the selection until the selection changes).
    top: std::cell::Cell<usize>,
    followed: std::cell::Cell<Option<(usize, usize)>>,
    /// The finger on a row's name: where it went down, the view's first
    /// row then, and whether it has moved far enough to be a drag.
    grab: Option<OptionGrab>,
}

#[derive(Clone, Copy, Debug)]
struct OptionGrab {
    finger: u64,
    y: i32,
    top: usize,
    row: usize,
    dragging: bool,
}

/// A button under an [`OptionList`]'s rows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionButton {
    pub label: String,
    /// Drawn faint, and a tap does nothing, while off.
    pub enabled: bool,
    /// Outlined amber: what it asked for waits (a reload waiting for its
    /// line); a tap takes it back.
    pub waiting: bool,
    /// Drawn off, a tap still gives `Toggle(i)`, for the app to say why
    /// it does nothing (Fit to tempo on a clip at the project's tempo).
    pub explains: bool,
    /// A job it started, this far done (0 to 1): it fills from the left
    /// in a dim accent, under its label ("Fitting… 40 %").
    pub progress: Option<f32>,
}

impl OptionButton {
    pub fn new(label: &str, enabled: bool) -> OptionButton {
        OptionButton {
            label: label.to_string(),
            enabled,
            ..OptionButton::default()
        }
    }
}

/// The two lines an [`OptionList`] shows over its buttons: `lead` in bold
/// on the first, amber when it `warn`s, and `rest` under it ("Made at
/// 120 BPM:" over "at 92 a 1.2 s gap each pass").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionNote {
    pub lead: String,
    pub rest: String,
    pub warn: bool,
}

const OPTION_H: i32 = 58;
/// An [`OptionList`]'s buttons: their height, and the gap over them.
const OPTION_BUTTON_H: i32 = 44;
const OPTION_BUTTON_GAP: i32 = 8;
/// An [`OptionNote`]'s line, and the room the note takes over the buttons.
const OPTION_NOTE_LINE: i32 = 18;
const OPTION_NOTE_H: i32 = 2 * OPTION_NOTE_LINE + 12;
const VALUE_W: i32 = 300;
const ARROW_W: i32 = 64;
/// Rows narrower than this draw narrow (see [`OptionList`]): below it the
/// usual value box (half the row) has no room for its two 64 px arrows and
/// a value between them.
pub const NARROW_ROW_W: i32 = 6 * ARROW_W;
/// A narrow row's arrow, as drawn.
const NARROW_ARROW_W: i32 = 22;
/// How far a finger on a row moves before it scrolls the list instead of
/// picking the row.
const OPTION_DRAG_PX: i32 = 12;
/// A tab of the title line, but the last (which takes the rest).
pub const OPTION_TAB_W: i32 = 84;
/// The event a tap on [`OptionList::back`] gives: `Tab(OPTION_BACK)`.
pub const OPTION_BACK: usize = usize::MAX;

impl OptionList {
    pub fn new(title: &str) -> OptionList {
        OptionList {
            title: title.to_string(),
            right: String::new(),
            tabs: Vec::new(),
            tab: 0,
            tab_widths: Vec::new(),
            rows: Vec::new(),
            buttons: Vec::new(),
            note: None,
            empty: "nothing here".to_string(),
            back: None,
            selected: 0,
            editing: false,
            top: std::cell::Cell::new(0),
            followed: std::cell::Cell::new(None),
            grab: None,
        }
    }

    /// Under the title line, to the panel's padding at the foot.
    fn inside(r: Rect) -> Rect {
        // Matches PaintCx::titled_panel with the default theme's padding.
        r.inset(14).take_top(34).1
    }

    /// Where the rows are: inside, over the buttons and the note if there
    /// are any.
    fn body(&self, r: Rect) -> Rect {
        let inside = OptionList::inside(r);
        let buttons = if self.buttons.is_empty() {
            0
        } else {
            OPTION_BUTTON_H + OPTION_BUTTON_GAP
        };
        let note = if self.note.is_some() {
            OPTION_NOTE_H
        } else {
            0
        };
        inside.take_bottom(buttons + note).1
    }

    /// Where the note's two lines are: over the buttons, under the rows.
    pub fn note_rect(&self, r: Rect) -> Option<Rect> {
        self.note.as_ref()?;
        let foot = if self.buttons.is_empty() {
            0
        } else {
            OPTION_BUTTON_H + OPTION_BUTTON_GAP
        };
        let above = OptionList::inside(r).take_bottom(foot).1;
        let note = above.take_bottom(OPTION_NOTE_H).0;
        Some(Rect::new(
            note.x + 2,
            note.y + 6,
            note.w - 2,
            2 * OPTION_NOTE_LINE,
        ))
    }

    fn visible(&self, r: Rect) -> usize {
        (self.body(r).h / OPTION_H).max(0) as usize
    }

    /// Button `i`'s rectangle, if there is one: the buttons share the foot
    /// of the panel, side by side.
    pub fn button_rect(&self, r: Rect, i: usize) -> Option<Rect> {
        let n = self.buttons.len() as i32;
        if i >= self.buttons.len() {
            return None;
        }
        let foot = OptionList::inside(r).take_bottom(OPTION_BUTTON_H).0;
        let w = (foot.w - (n - 1) * OPTION_BUTTON_GAP) / n;
        let x = foot.x + i as i32 * (w + OPTION_BUTTON_GAP);
        Some(Rect::new(x, foot.y, w, foot.h))
    }

    /// The first row in view. When the selection has changed since the view
    /// last followed it, the view moves just enough to show it with one row
    /// either side (none past the ends); otherwise it stays where a drag
    /// left it.
    fn first(&self, visible: usize) -> usize {
        if self.rows.len() <= visible || visible == 0 {
            return 0;
        }
        let last = self.rows.len() - visible;
        let mut top = self.top.get().min(last);
        let now = (self.selected, self.rows.len());
        if self.followed.get() != Some(now) {
            let sel = self.selected.min(self.rows.len() - 1);
            let margin = usize::from(visible > 2);
            if sel < top + margin {
                top = sel.saturating_sub(margin);
            } else if sel + margin >= top + visible {
                top = (sel + margin + 1).saturating_sub(visible);
            }
            top = top.min(last);
            self.followed.set(Some(now));
        }
        self.top.set(top);
        top
    }

    /// Another list in the same widget (a new menu): its view starts at
    /// the top and follows its selection.
    pub fn reset_view(&mut self) {
        self.top.set(0);
        self.followed.set(None);
        self.grab = None;
    }

    /// The scroll bar's track and thumb, when the list is longer than its
    /// panel.
    pub fn scroll_bar(&self, r: Rect) -> Option<(Rect, Rect)> {
        let body = self.body(r);
        let visible = self.visible(r);
        if visible == 0 || self.rows.len() <= visible {
            return None;
        }
        let track = Rect::new(r.right() - 9, body.y, 4, visible as i32 * OPTION_H - 6);
        let h = (track.h * visible as i32 / self.rows.len() as i32).max(24);
        let span = (self.rows.len() - visible) as i32;
        let y = track.y + (track.h - h) * self.first(visible) as i32 / span;
        Some((track, Rect::new(track.x, y, track.w, h)))
    }

    /// Row `k`'s rectangle (if it is in view) and its value box.
    pub fn row_rects(&self, r: Rect, k: usize) -> Option<(Rect, Rect)> {
        let body = self.body(r);
        let visible = self.visible(r);
        let first = self.first(visible);
        if k < first || k >= first + visible || k >= self.rows.len() {
            return None;
        }
        let row = Rect::new(
            body.x,
            body.y + (k - first) as i32 * OPTION_H,
            body.w,
            OPTION_H - 6,
        );
        let w = if OptionList::narrow(row) {
            row.w * 54 / 100
        } else {
            VALUE_W.min(row.w / 2)
        };
        let value = Rect::new(row.right() - w, row.y, w, row.h);
        Some((row, value))
    }

    /// Tab `k`'s rectangle on the title line, from the panel's top edge
    /// to the first row (so a finger finds it), if there are tabs.
    pub fn tab_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        if k >= self.tabs.len() {
            return None;
        }
        let head = r.inset(14);
        let pitch = |k: usize| self.tab_widths.get(k).copied().unwrap_or(OPTION_TAB_W);
        let x = head.x + (0..k).map(pitch).sum::<i32>();
        let w = if k + 1 == self.tabs.len() {
            head.right() - x
        } else {
            pitch(k) - 6
        };
        Some(Rect::new(x, r.y, w.max(0), OptionList::inside(r).y - r.y))
    }

    /// The Back button on the title line, from near the panel's top edge
    /// to the first row (a fingertip's height), when there is one.
    pub fn back_rect(&self, r: Rect) -> Option<Rect> {
        let label = self.back.as_ref().filter(|_| self.tabs.is_empty())?;
        let w = (label.chars().count() as i32 * 11 + 44).max(96);
        let bottom = OptionList::inside(r).y - 4;
        Some(Rect::new(r.x + 8, r.y + 4, w, bottom - r.y - 4))
    }

    /// The row draws narrow (see [`OptionList`]).
    pub fn narrow(row: Rect) -> bool {
        row.w < NARROW_ROW_W
    }

    /// How far in from each end of a value box a tap steps the value.
    fn arrow_reach(row: Rect, value: Rect) -> i32 {
        if OptionList::narrow(row) {
            value.w / 3
        } else {
            ARROW_W
        }
    }
}

impl Widget for OptionList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        if let Some(b) = self.back_rect(r) {
            // ‹ Back, then the title after it.
            cx.panel(r);
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            cx.canvas.stroke_round_rect(b, t.round(10), 1, t.outline);
            let label = self.back.as_deref().unwrap_or_default();
            cx.text(
                FontId::Bold,
                t.size_small + 1.0,
                b.inset_xy(12, 0),
                Align::Left,
                t.text,
                &format!("‹  {label}"),
            );
            let head = r.inset(t.pad).take_top(26).0;
            let line = Rect::new(
                b.right() + 14,
                head.y,
                head.right() - b.right() - 14,
                head.h,
            );
            cx.text(
                FontId::Bold,
                t.size_small,
                line,
                Align::Left,
                t.text_dim,
                &self.title.to_uppercase(),
            );
            if !self.right.is_empty() {
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    line,
                    Align::Right,
                    t.text_dim,
                    &self.right,
                );
            }
        } else if self.tabs.is_empty() {
            cx.titled_panel(r, &self.title, &self.right);
        } else {
            // The title line as tabs: as the title is drawn, the lit one
            // in full ink and underlined.
            cx.panel(r);
            let head = r.inset(t.pad).take_top(26).0;
            for (k, name) in self.tabs.iter().enumerate() {
                let Some(tr) = self.tab_rect(r, k) else {
                    continue;
                };
                let line = Rect::new(tr.x, head.y, tr.w, head.h);
                let on = k == self.tab;
                let w = cx.text(
                    FontId::Bold,
                    t.size_small,
                    line,
                    Align::Left,
                    if on { t.text } else { t.text_dim },
                    &name.to_uppercase(),
                );
                if on {
                    let bar = Rect::new(line.x, line.bottom() + 2, w.min(line.w), 3);
                    cx.canvas.fill_round_rect(bar, 1, t.accent);
                }
            }
        }
        for k in 0..self.rows.len() {
            let Some((row, value)) = self.row_rects(r, k) else {
                continue;
            };
            let on = k == self.selected;
            let narrow = OptionList::narrow(row);
            if on {
                cx.canvas.fill_round_rect(row, t.round(10), t.raised);
                cx.canvas
                    .fill_round_rect(Rect::new(row.x, row.y + 12, 4, row.h - 24), 2, t.accent);
            }
            let (px, arrow, gap) = if narrow {
                (t.size_small + 1.0, NARROW_ARROW_W, 20)
            } else {
                (t.size_body, ARROW_W, 24)
            };
            let name = Rect::new(row.x + 16, row.y, (value.x - row.x - gap).max(0), row.h);
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                px,
                name,
                Align::Left,
                t.text,
                &self.rows[k].name,
            );
            let boxr = value.inset_xy(0, 7);
            let open = on && self.editing;
            cx.canvas
                .fill_round_rect(boxr, t.round(10), if on { t.surface } else { t.raised });
            if open {
                cx.canvas.stroke_round_rect(boxr, t.round(10), 2, t.accent);
            }
            let (left, mid) = boxr.take_left(arrow);
            let (right, mid) = mid.take_right(arrow);
            let arrows = if open { t.accent } else { t.text_dim };
            cx.text(
                FontId::Bold,
                t.size_title + 4.0,
                left,
                Align::Center,
                arrows,
                "‹",
            );
            cx.text(
                FontId::Bold,
                t.size_title + 4.0,
                right,
                Align::Center,
                arrows,
                "›",
            );
            cx.text(
                FontId::Bold,
                px,
                mid,
                Align::Center,
                if on { t.accent } else { t.text },
                &self.rows[k].value,
            );
        }
        if let Some((track, thumb)) = self.scroll_bar(r) {
            cx.canvas.fill_round_rect(track, 2, t.outline);
            cx.canvas.fill_round_rect(thumb, 2, t.text_dim);
        }
        if let (Some(n), Some(nr)) = (&self.note, self.note_rect(r)) {
            let (first, second) = nr.take_top(OPTION_NOTE_LINE);
            cx.text(
                FontId::Bold,
                t.size_small,
                first,
                Align::Left,
                if n.warn { t.warn } else { t.text },
                &n.lead,
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                second,
                Align::Left,
                t.text_dim,
                &n.rest,
            );
        }
        for (i, b) in self.buttons.iter().enumerate() {
            let Some(br) = self.button_rect(r, i) else {
                continue;
            };
            cx.canvas.fill_round_rect(
                br,
                t.round(10),
                if b.enabled { t.raised } else { t.surface },
            );
            if let Some(p) = b.progress {
                let w = (br.w as f32 * p.clamp(0.0, 1.0)).round() as i32;
                if w > 0 {
                    let fill = t.raised.mix(t.accent, 0.35);
                    cx.canvas
                        .fill_round_rect(Rect::new(br.x, br.y, w, br.h), t.round(10), fill);
                }
            }
            if b.waiting {
                cx.canvas.stroke_round_rect(br, t.round(10), 2, t.warn);
            }
            cx.text(
                FontId::Bold,
                t.size_small,
                br,
                Align::Center,
                if b.waiting {
                    t.warn
                } else if b.enabled {
                    t.text
                } else {
                    t.text_faint
                },
                &b.label,
            );
        }
        if self.rows.is_empty() {
            let body = self.body(r);
            cx.text(
                FontId::Sans,
                t.size_body,
                body.take_top(OPTION_H).0,
                Align::Left,
                t.text_faint,
                &self.empty,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = Vec::new();
        if let Some(b) = self.back_rect(r) {
            out.push(Target::tap(
                "back",
                OPTION_BACK,
                self.back.clone().unwrap_or_default(),
                b,
            ));
        }
        for (k, name) in self.tabs.iter().enumerate() {
            if let Some(tr) = self.tab_rect(r, k) {
                out.push(Target::tap("tab", k, name.clone(), tr).disabled_if(k == self.tab));
            }
        }
        for k in 0..self.rows.len() {
            let Some((row, value)) = self.row_rects(r, k) else {
                continue;
            };
            let reach = OptionList::arrow_reach(row, value);
            let name = &self.rows[k].name;
            let pick = Rect::new(row.x, row.y, value.x - row.x, row.h);
            // The row picked already: a tap picks nothing new.
            out.push(Target::tap("row", k, name.clone(), pick).disabled_if(k == self.selected));
            let (less, rest) = value.take_left(reach);
            let more = rest.take_right(reach).0;
            out.push(Target::tap("row-less", k, format!("{name} ‹"), less));
            out.push(Target::tap("row-more", k, format!("{name} ›"), more));
        }
        for (i, b) in self.buttons.iter().enumerate() {
            if let Some(br) = self.button_rect(r, i) {
                out.push(Target::tap("button", i, b.label.clone(), br).disabled_if(!b.enabled));
            }
        }
        out
    }

    /// A finger resting on a row puts its value under the wheel,
    /// whichever part of the row it is on (a drag that scrolls the list
    /// lets it go again).
    fn held(&self, x: i32, y: i32, r: Rect) -> Option<Target> {
        let k = (0..self.rows.len()).find(|&k| {
            self.row_rects(r, k)
                .is_some_and(|(row, _)| row.contains(x, y))
        })?;
        let (row, _) = self.row_rects(r, k)?;
        Some(Target::tap("row", k, self.rows[k].name.clone(), row))
    }

    /// The held row's value, a step a detent, as its arrows step it.
    fn held_step(&mut self, t: &Target, delta: i32, _fine: bool, _r: Rect) -> Option<EventKind> {
        if t.index >= self.rows.len() || delta == 0 {
            return None;
        }
        self.selected = t.index;
        Some(EventKind::Step {
            index: t.index,
            delta,
        })
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {}
            Touch::Move => {
                let visible = self.visible(r);
                let g = self.grab.as_mut().filter(|g| g.finger == finger)?;
                if !g.dragging && (y - g.y).abs() < OPTION_DRAG_PX {
                    return None;
                }
                g.dragging = true;
                if self.rows.len() > visible {
                    // The rows follow the finger: up a row for each row's
                    // height it moves up.
                    let moved = (g.y - y) / OPTION_H;
                    let last = (self.rows.len() - visible) as i32;
                    let top = (g.top as i32 + moved).clamp(0, last) as usize;
                    self.top.set(top);
                    self.followed.set(Some((self.selected, self.rows.len())));
                }
                return None;
            }
            Touch::Up | Touch::Cancel => {
                let g = self.grab.take().filter(|g| g.finger == finger)?;
                if g.dragging || phase == Touch::Cancel {
                    return None;
                }
                self.selected = g.row;
                return Some(EventKind::Select(g.row));
            }
        }
        if self.back_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(OPTION_BACK));
        }
        if let Some(k) =
            (0..self.tabs.len()).find(|&k| self.tab_rect(r, k).is_some_and(|tr| tr.contains(x, y)))
        {
            // Another tab: the app shows its list (the lit one does nothing).
            return (k != self.tab).then_some(EventKind::Tab(k));
        }
        if let Some(i) = (0..self.buttons.len())
            .find(|&i| self.button_rect(r, i).is_some_and(|b| b.contains(x, y)))
        {
            let b = &self.buttons[i];
            return (b.enabled || b.explains).then_some(EventKind::Toggle(i));
        }
        let k = (0..self.rows.len()).find(|&k| {
            self.row_rects(r, k)
                .is_some_and(|(row, _)| row.contains(x, y))
        })?;
        let (row, value) = self.row_rects(r, k)?;
        let reach = OptionList::arrow_reach(row, value);
        if value.contains(x, y) && x < value.x + reach {
            self.selected = k;
            Some(EventKind::Step {
                index: k,
                delta: -1,
            })
        } else if value.contains(x, y) && x >= value.right() - reach {
            self.selected = k;
            Some(EventKind::Step { index: k, delta: 1 })
        } else {
            // A row's name: picked when the finger lifts, unless it drags
            // the list.
            let top = self.first(self.visible(r));
            self.grab = Some(OptionGrab {
                finger,
                y,
                top,
                row: k,
                dragging: false,
            });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Fonts, SoftCanvas, Theme, Ui};

    #[test]
    fn pads_are_laid_out_like_the_mpc() {
        let g = PadGrid::new(16, 4);
        let r = Rect::new(0, 0, 400, 400);
        let p1 = g.cell(r, 0);
        let p16 = g.cell(r, 15);
        let p13 = g.cell(r, 12);
        assert!(p1.x == 0 && p1.bottom() > 380, "pad 1 bottom left: {p1:?}");
        assert!(p16.right() > 380 && p16.y == 0, "pad 16 top right: {p16:?}");
        assert_eq!((p13.x, p13.y), (0, 0));
    }

    #[test]
    fn touching_pads_presses_and_releases_them() {
        let mut g = PadGrid::new(16, 4);
        let r = Rect::new(0, 0, 400, 400);
        let c = g.cell(r, 5);
        assert_eq!(
            g.touch(3, Touch::Down, c.x + 5, c.y + 5, r),
            Some(EventKind::Pad {
                index: 5,
                down: true
            })
        );
        assert_eq!(g.touch(3, Touch::Move, 0, 0, r), None);
        assert_eq!(
            g.touch(3, Touch::Up, 0, 0, r),
            Some(EventKind::Pad {
                index: 5,
                down: false
            })
        );
        assert_eq!(g.touch(3, Touch::Up, 0, 0, r), None);
        // The gap between pads plays nothing.
        assert_eq!(g.touch(4, Touch::Down, c.right() + 2, c.y + 5, r), None);
    }

    #[test]
    fn only_changed_pads_are_redrawn() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 640, 400);
        ui.add("pads", PadGrid::new(16, 4));
        ui.add_page("p", "pads").unwrap();
        let mut c = SoftCanvas::new(640, 400);
        ui.paint(&mut c);
        ui.get_mut::<PadGrid>("pads").unwrap().hit(4, 90);
        let r = ui.rect_of("pads").unwrap();
        let cell = ui.get::<PadGrid>("pads").unwrap().cell(r, 4);
        assert_eq!(ui.paint(&mut c), vec![cell]);
        ui.get_mut::<PadGrid>("pads").unwrap(); // touched, nothing changed
        assert!(ui.paint(&mut c).is_empty());
    }

    #[test]
    fn pads_fade_after_release() {
        let mut g = PadGrid::new(16, 4);
        g.hit(2, 100);
        g.fade(1.0);
        assert_eq!(g.pads[2].level, 1.0, "held pads stay lit");
        g.release(2);
        assert!(g.fading());
        g.fade(0.1);
        assert!(g.pads[2].level < 1.0 && g.pads[2].level > 0.0);
        g.fade(1.0);
        assert!(!g.fading());
    }

    #[test]
    fn list_rows_select_and_scroll() {
        let mut l = ListView::new("Kits");
        l.items = (0..20)
            .map(|i| ListItem {
                text: format!("kit {i}"),
                ..Default::default()
            })
            .collect();
        let r = Rect::new(0, 0, 300, 14 * 2 + 34 + ROW_H * 5);
        let body = ListView::body(r);
        assert_eq!(
            l.touch(1, Touch::Down, body.x + 5, body.y + ROW_H + 5, r),
            Some(EventKind::Select(1))
        );
        l.selected = 15;
        assert_eq!(l.first(5), 13);
        l.selected = 19;
        assert_eq!(l.first(5), 15);
        assert_eq!(
            l.touch(1, Touch::Down, body.x + 5, body.y + 5, r),
            Some(EventKind::Select(15))
        );
    }

    #[test]
    fn faders_follow_the_finger() {
        let mut f = Fader::new("Level");
        let r = Rect::new(0, 0, 300, 90);
        let tr = Fader::track(r);
        assert_eq!(
            f.touch(1, Touch::Down, tr.x, 50, r),
            Some(EventKind::Value(0.0))
        );
        assert_eq!(
            f.touch(1, Touch::Move, tr.right() + 50, 50, r),
            Some(EventKind::Value(1.0))
        );
        assert_eq!(f.touch(1, Touch::Up, 0, 50, r), None);
    }

    #[test]
    fn the_app_bar_is_the_approved_one() {
        assert_eq!(
            APP_TABS,
            [
                "Play",
                "Sequencer",
                "Looper",
                "Sample",
                "Synth",
                "Arrange",
                "Pads",
                "Mix"
            ]
        );
        assert_eq!(Header::app("Mix").active, 7);
        assert_eq!(Header::app("Synth").active, 4);
        assert_eq!(Header::app("Prefs").active, usize::MAX);
    }

    /// Eight or nine tabs (the Dev tab) are sized to their names within
    /// six tabs' room: "Sequencer" is the widest and keeps room for its
    /// name, and the tabs touch with one gap between, in order.
    #[test]
    fn more_than_six_tabs_are_sized_to_their_names() {
        let r = Rect::new(0, 0, 1280, 60);
        for tabs in [&APP_TABS[..], &[&APP_TABS[..], &[DEV_TAB]].concat()] {
            let h = Header::new(tabs);
            let rects: Vec<Rect> = (0..tabs.len()).map(|i| h.tab_rect(r, i)).collect();
            for w in rects.windows(2) {
                assert_eq!(w[0].right() + TAB_GAP, w[1].x, "{tabs:?}");
            }
            let end = h.tab_rect(r, tabs.len()).x;
            assert!(end <= LEFT_W + TABS_W, "{end}");
            assert_eq!(end, rects[tabs.len() - 1].right() + TAB_GAP);
            let seq = rects[1];
            assert!(rects.iter().all(|t| t.w <= seq.w), "{rects:?}");
            assert!(seq.w >= 100, "Sequencer is {} px", seq.w);
            assert!(rects.iter().all(|t| t.w >= 60), "{rects:?}");
        }
        // Six or fewer keep the full width.
        let h = Header::new(&["Play", "Debug"]);
        assert_eq!(h.tab_rect(r, 1).w, TAB_W);
    }

    #[test]
    fn tabs_switch() {
        let mut h = Header::new(&["Play", "Debug"]);
        let r = Rect::new(0, 0, 1280, 60);
        let t1 = h.tab_rect(r, 1);
        assert_eq!(
            h.touch(1, Touch::Down, t1.x + 3, t1.y + 3, r),
            Some(EventKind::Tab(1))
        );
        assert_eq!(h.active, 1);
        assert_eq!(h.touch(1, Touch::Down, 5, 5, r), None);
    }

    #[test]
    fn logs_keep_the_newest_lines() {
        let mut l = LogPanel::new("Events", true, 3);
        for i in 0..5 {
            l.push(&format!("line {i}\t!"), Tone::Normal);
        }
        let got: Vec<&str> = l.lines().map(|(s, _)| s.as_str()).collect();
        assert_eq!(got, vec!["line 2 !", "line 3 !", "line 4 !"]);
    }

    #[test]
    fn option_rows_select_and_step() {
        let mut o = OptionList::new("Prefs");
        o.rows = (0..12)
            .map(|i| OptionRow {
                name: format!("option {i}"),
                value: "a".into(),
            })
            .collect();
        let r = Rect::new(0, 0, 800, 14 * 2 + 34 + OPTION_H * 6);
        let (row, value) = o.row_rects(r, 2).unwrap();
        // A row's name picks it when the finger lifts.
        assert_eq!(o.touch(1, Touch::Down, row.x + 20, row.y + 10, r), None);
        assert_eq!(
            o.touch(1, Touch::Up, row.x + 20, row.y + 10, r),
            Some(EventKind::Select(2))
        );
        assert_eq!(
            o.touch(1, Touch::Down, value.x + 5, value.y + 10, r),
            Some(EventKind::Step {
                index: 2,
                delta: -1
            })
        );
        assert_eq!(
            o.touch(1, Touch::Down, value.right() - 5, value.y + 10, r),
            Some(EventKind::Step { index: 2, delta: 1 })
        );
        assert_eq!(o.selected, 2);
        assert_eq!(
            o.touch(1, Touch::Up, value.right() - 5, value.y + 10, r),
            None
        );
        // Rows out of view cannot be hit; the selected row is kept in view.
        o.selected = 11;
        assert!(o.row_rects(r, 0).is_none() && o.row_rects(r, 11).is_some());
    }

    /// Prefs' long list (Jay's final round, 16 Nov): the view moves a row
    /// at a time with the selection, a tap doesn't jump it, a finger drag
    /// scrolls it and picks nothing, and a bar shows where it is.
    #[test]
    fn a_long_option_list_scrolls_a_row_at_a_time_and_with_a_finger() {
        let mut o = OptionList::new("Prefs");
        o.rows = (0..18)
            .map(|i| OptionRow {
                name: format!("option {i}"),
                value: "a".into(),
            })
            .collect();
        let r = Rect::new(0, 0, 800, 14 * 2 + 34 + OPTION_H * 6);
        let first = |o: &OptionList| (0..18).find(|&k| o.row_rects(r, k).is_some()).unwrap();
        assert_eq!(first(&o), 0);
        // The wheel down the list: the view starts to move only when the
        // selection nears the bottom, then a row at a time.
        let mut tops = Vec::new();
        for k in 0..18 {
            o.selected = k;
            tops.push(first(&o));
        }
        assert_eq!(
            tops,
            [0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 12]
        );
        // Back up: it stays until the selection nears the top.
        o.selected = 15;
        assert_eq!(first(&o), 12);
        o.selected = 13;
        assert_eq!(first(&o), 12);
        o.selected = 12;
        assert_eq!(first(&o), 11);
        // A tap on a row in view picks it without moving the view.
        o.selected = 3;
        let top = first(&o);
        let (row, _) = o.row_rects(r, top + 3).unwrap();
        o.touch(4, Touch::Down, row.x + 20, row.y + 10, r);
        assert_eq!(
            o.touch(4, Touch::Up, row.x + 20, row.y + 10, r),
            Some(EventKind::Select(top + 3))
        );
        assert_eq!(first(&o), top, "no jump");
        // A finger dragged up two rows scrolls two rows and picks nothing.
        let sel = o.selected;
        let (row, _) = o.row_rects(r, top + 4).unwrap();
        let y = row.y + 10;
        assert_eq!(o.touch(5, Touch::Down, row.x + 20, y, r), None);
        assert_eq!(o.touch(5, Touch::Move, row.x + 20, y - OPTION_H, r), None);
        assert_eq!(first(&o), top + 1);
        assert_eq!(
            o.touch(5, Touch::Move, row.x + 20, y - 2 * OPTION_H, r),
            None
        );
        assert_eq!(first(&o), top + 2);
        assert_eq!(o.touch(5, Touch::Up, row.x + 20, y - 2 * OPTION_H, r), None);
        assert_eq!(o.selected, sel, "a drag picks nothing");
        assert_eq!(
            first(&o),
            top + 2,
            "the view stays where the finger left it"
        );
        // The bar: its thumb moves down as the view does.
        let (track, thumb) = o.scroll_bar(r).expect("a bar on a long list");
        assert!(thumb.y > track.y && thumb.bottom() <= track.bottom());
        o.selected = 0;
        assert_eq!(first(&o), 0, "a new selection brings the view back to it");
        let (track, thumb) = o.scroll_bar(r).unwrap();
        assert_eq!(thumb.y, track.y);
        // A short list has no bar.
        o.rows.truncate(4);
        assert!(o.scroll_bar(r).is_none());
    }

    #[test]
    fn option_tabs_switch_lists_from_the_title_line() {
        let mut o = OptionList::new("Record");
        o.tabs = vec!["Record".into(), "Clip · Verse B".into()];
        o.rows = vec![OptionRow {
            name: "Armed".into(),
            value: "Bass".into(),
        }];
        let r = Rect::new(0, 0, 250, 14 * 2 + 34 + OPTION_H * 3);
        let (a, b) = (o.tab_rect(r, 0).unwrap(), o.tab_rect(r, 1).unwrap());
        // From the panel's top to the first row, the last to the right.
        let (row, _) = o.row_rects(r, 0).unwrap();
        assert_eq!((a.y, a.bottom()), (0, row.y));
        assert!(a.right() < b.x && b.right() == row.right(), "{a:?} {b:?}");
        assert!(o.tab_rect(r, 2).is_none());
        // The other tab at once; the lit one, nothing.
        assert_eq!(
            o.touch(1, Touch::Down, b.x + 4, b.y + 20, r),
            Some(EventKind::Tab(1))
        );
        assert_eq!(o.touch(1, Touch::Down, a.x + 4, a.y + 20, r), None);
        o.tab = 1;
        assert_eq!(
            o.touch(1, Touch::Down, a.x + 4, a.y + 20, r),
            Some(EventKind::Tab(0))
        );
        let t = o.targets(r);
        let tabs: Vec<_> = t.iter().filter(|t| t.id == "tab").collect();
        assert_eq!(tabs.len(), 2);
        // Three tabs with their own widths: each where the ones before
        // leave it, the last to the right.
        o.tabs.push("Scene".into());
        o.tab_widths = vec![66, 104];
        let c = o.tab_rect(r, 2).unwrap();
        let b = o.tab_rect(r, 1).unwrap();
        assert_eq!((b.x, b.w), (a.x + 66, 98));
        assert_eq!((c.x, c.right()), (b.x + 104, row.right()));
        // Without tabs, nothing on the title line answers.
        o.tabs.clear();
        assert_eq!(o.touch(1, Touch::Down, b.x + 4, b.y + 20, r), None);
        assert!(!o.targets(r).iter().any(|t| t.id == "tab"));
    }

    /// Buttons under the rows: side by side at the foot, the rows over
    /// them; a tap gives `Toggle(i)`, nothing while one is off.
    #[test]
    fn option_buttons_sit_under_the_rows_and_tap() {
        let mut o = OptionList::new("Clip");
        o.rows = (0..6)
            .map(|i| OptionRow {
                name: format!("Row {i}"),
                value: "1".into(),
            })
            .collect();
        let r = Rect::new(0, 0, 250, 14 * 2 + 34 + OPTION_H * 4);
        assert_eq!(o.visible(r), 4);
        assert!(o.button_rect(r, 0).is_none());
        o.buttons = vec![
            OptionButton::new("Save state", true),
            OptionButton::new("Reload", false),
        ];
        assert_eq!(o.visible(r), 3, "the buttons take a row's room");
        let (a, b) = (o.button_rect(r, 0).unwrap(), o.button_rect(r, 1).unwrap());
        assert!(o.button_rect(r, 2).is_none());
        assert_eq!(a.bottom(), r.bottom() - 14);
        assert!(
            a.right() < b.x && b.right() == r.right() - 14,
            "{a:?} {b:?}"
        );
        let (last, _) = o.row_rects(r, 2).unwrap();
        assert!(last.bottom() <= a.y, "{last:?} {a:?}");
        assert_eq!(
            o.touch(1, Touch::Down, a.x + 4, a.y + 4, r),
            Some(EventKind::Toggle(0))
        );
        assert_eq!(o.touch(1, Touch::Down, b.x + 4, b.y + 4, r), None, "off");
        let t = o.targets(r);
        let buttons: Vec<_> = t.iter().filter(|t| t.id == "button").collect();
        assert_eq!(buttons.len(), 2);
        assert!(buttons[0].enabled && !buttons[1].enabled);
        // Painted, waiting outlined.
        o.buttons[1].waiting = true;
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 260, r.h);
        ui.add("o", o);
        ui.add_page("p", "o").unwrap();
        let mut c = SoftCanvas::new(260, r.h);
        assert!(!ui.paint(&mut c).is_empty());
    }

    /// An audio clip's card (layout spec 4j): two lines between the rows
    /// and the button, the button filling with its job's progress, and a
    /// faint button that explains still answers a tap.
    #[test]
    fn an_option_note_sits_over_the_buttons_and_a_faint_button_can_explain() {
        let mut o = OptionList::new("Clip");
        o.rows = (0..7)
            .map(|i| OptionRow {
                name: format!("Row {i}"),
                value: "1".into(),
            })
            .collect();
        let r = Rect::new(0, 0, 250, 14 * 2 + 34 + OPTION_H * 4 + OPTION_BUTTON_H + 8);
        let mut fit = OptionButton::new("Fit to tempo", false);
        o.buttons = vec![fit.clone()];
        assert_eq!(o.visible(r), 4);
        assert!(o.note_rect(r).is_none());
        o.note = Some(OptionNote {
            lead: "Made at 120 BPM:".into(),
            rest: "at 92 a 1.2 s gap each pass".into(),
            warn: true,
        });
        assert_eq!(o.visible(r), 3, "the note takes a row's room");
        let (n, b) = (o.note_rect(r).unwrap(), o.button_rect(r, 0).unwrap());
        let (last, _) = o.row_rects(r, 2).unwrap();
        assert!(
            last.bottom() <= n.y && n.bottom() <= b.y,
            "{last:?} {n:?} {b:?}"
        );
        assert_eq!(o.touch(1, Touch::Down, b.x + 4, b.y + 4, r), None, "off");
        fit.explains = true;
        fit.progress = Some(0.4);
        o.buttons = vec![fit];
        assert_eq!(
            o.touch(1, Touch::Down, b.x + 4, b.y + 4, r),
            Some(EventKind::Toggle(0)),
            "off, but it says why"
        );
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 260, r.h);
        ui.add("o", o);
        ui.add_page("p", "o").unwrap();
        let mut c = SoftCanvas::new(260, r.h);
        assert!(!ui.paint(&mut c).is_empty());
    }

    #[test]
    fn narrow_option_rows_keep_their_arrows() {
        let mut o = OptionList::new("Record");
        o.rows = (0..3)
            .map(|i| OptionRow {
                name: format!("option {i}"),
                value: "a".into(),
            })
            .collect();
        // Arrange's Record panel beside the Q-Link column: 250 px.
        let r = Rect::new(0, 0, 250, 14 * 2 + 34 + OPTION_H * 3);
        let (row, value) = o.row_rects(r, 1).unwrap();
        assert!(OptionList::narrow(row) && row.w == 222, "{row:?}");
        // A little over half the row, and more than two wide arrows' worth
        // would leave.
        assert_eq!(value.w, 119);
        assert_eq!(value.right(), row.right());
        // The value box's left and right thirds step it; its middle and the
        // name select the row.
        let y = value.y + 10;
        let step = |delta| EventKind::Step { index: 1, delta };
        for (x, want) in [
            (value.x + 2, step(-1)),
            (value.x + value.w / 3 - 1, step(-1)),
            (value.x + value.w / 2, EventKind::Select(1)),
            (value.right() - value.w / 3, step(1)),
            (value.right() - 2, step(1)),
            (row.x + 20, EventKind::Select(1)),
        ] {
            // A step is at once; a pick when the finger lifts.
            let down = o.touch(1, Touch::Down, x, y, r);
            let got = down.or_else(|| o.touch(1, Touch::Up, x, y, r));
            assert_eq!(got, Some(want), "x {x}");
        }
        // A wide list is as before: the 64 px arrows, half the row.
        let wide = Rect::new(0, 0, 430, r.h);
        let (row, value) = o.row_rects(wide, 1).unwrap();
        assert!(!OptionList::narrow(row));
        assert_eq!(value.w, row.w / 2);
        assert_eq!(
            o.touch(1, Touch::Down, value.x + ARROW_W - 1, value.y + 10, wide),
            Some(EventKind::Step {
                index: 1,
                delta: -1
            })
        );
        assert_eq!(
            o.touch(1, Touch::Down, value.x + ARROW_W + 1, value.y + 10, wide),
            None
        );
        assert_eq!(
            o.touch(1, Touch::Up, value.x + ARROW_W + 1, value.y + 10, wide),
            Some(EventKind::Select(1))
        );
    }

    #[test]
    fn card_buttons_report_their_index() {
        let mut c = Card::new("Buttons");
        c.buttons = vec!["Skip".into(), "Skip all".into()];
        let r = Rect::new(0, 0, 700, 400);
        let b1 = c.button_rect(r, 1);
        assert_eq!(
            c.touch(1, Touch::Down, b1.x + 5, b1.y + 5, r),
            Some(EventKind::Tab(1))
        );
        assert_eq!(c.touch(1, Touch::Down, 300, 100, r), None);
    }

    #[test]
    fn loading_redraws_only_its_card_once_drawn() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 640, 400);
        ui.add("loading", Loading::new("MPFree", "Pad player"));
        ui.add_full_page("p", "loading").unwrap();
        let mut c = SoftCanvas::new(640, 400);
        assert_eq!(ui.paint(&mut c), vec![Rect::new(0, 0, 640, 400)]);
        ui.get_mut::<Loading>("loading").unwrap().progress = 0.5;
        let d = ui.paint(&mut c);
        let r = Rect::new(0, 0, 640, 400);
        assert_eq!(d, vec![Loading::changing_part(r)]);
        assert!(d[0].h < 200 && d[0].contains(320, Loading::card(r).y + 5));
        // A new backdrop redraws everything.
        let mut img = Image::new(640, 400);
        img.px.fill(0x123456);
        ui.get_mut::<Loading>("loading")
            .unwrap()
            .set_backdrop(Some(Arc::new(img)));
        assert_eq!(ui.paint(&mut c), vec![r]);
        assert_eq!(c.pixel(5, 5), Color::rgb(0x123456));
    }

    #[test]
    fn every_widget_paints_inside_its_rectangle() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 640, 400);
        let mut h = Header::new(&["Play", "Debug"]);
        h.chips.push(Chip::new("session", Tone::Good));
        ui.add("header", h);
        let mut g = PadGrid::new(16, 4);
        g.pads[0] = Pad {
            label: "Kick".into(),
            color: Some(Color::rgb(0xFF0000)),
            level: 1.0,
            held: true,
            velocity: 90,
        };
        ui.add("pads", g);
        let mut l = ListView::new("Kits");
        l.items.push(ListItem {
            text: "House".into(),
            detail: "16 pads".into(),
            color: Some(Color::rgb(0x00FF00)),
        });
        ui.add("kits", l);
        ui.add("level", Fader::new("Level"));
        let mut s = StatList::new("Timing");
        s.set(vec![("Underruns".into(), "0".into(), Tone::Good)]);
        ui.add("stats", s);
        let mut log = LogPanel::new("MIDI", true, 10);
        log.push("99 25 40 pad 1", Tone::Normal);
        ui.add("log", log);
        ui.add("hint", Label::new("wheel: kit", Tone::Dim, Align::Center));
        let mut card = Card::new("Buttons");
        card.heading = "Press MENU".into();
        card.lines.push(("a line".into(), Tone::Dim));
        card.buttons.push("Skip".into());
        ui.add("card", card);
        let mut opts = OptionList::new("Prefs");
        opts.rows.push(OptionRow {
            name: "Velocity curve".into(),
            value: "Linear".into(),
        });
        ui.add("opts", opts);
        ui.add_page("p", "col(header:60, row(pads:*, col(kits, level:90, stats, log), col(card, opts)):*, hint:24)")
            .unwrap();
        let mut c = SoftCanvas::new(640, 400);
        ui.paint(&mut c);
        // Nothing drawn in the outer margin.
        let bg = ui.theme.bg;
        for x in 0..640 {
            assert_eq!(c.pixel(x, 2), bg, "top margin at x {x}");
            assert_eq!(c.pixel(x, 397), bg, "bottom margin at x {x}");
        }
    }

    #[test]
    fn a_tap_on_a_chip_names_it() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut h = Header::new(&["Play", "Debug"]);
        h.chips = vec![
            Chip::new("Akai session", Tone::Good),
            Chip::new("Arp off", Tone::Dim),
        ];
        ui.add("header", h);
        ui.add_full_page("p", "col(header:60)").unwrap();
        let mut c = SoftCanvas::new(1280, 800);
        ui.paint(&mut c);
        // Chip 0 sits at the right edge, chip 1 to its left.
        let ev = |ui: &mut Ui, x| ui.touch(1, Touch::Down, x, 30).map(|e| e.kind);
        assert_eq!(ev(&mut ui, 1270), Some(EventKind::Select(0)));
        ui.touch(1, Touch::Up, 1270, 30);
        let first = ui.get::<Header>("header").unwrap().chip_rect(1).unwrap();
        assert_eq!(ev(&mut ui, first.x + 5), Some(EventKind::Select(1)));
        ui.touch(1, Touch::Up, first.x + 5, 30);
        // The first tab starts at the left edge: there is no name before it.
        assert_eq!(ev(&mut ui, LEFT_W + 20), Some(EventKind::Tab(0)));
        ui.touch(1, Touch::Up, LEFT_W + 20, 30);
        assert_eq!(
            ev(&mut ui, first.x - 20),
            None,
            "between the chips and the tabs"
        );
    }

    /// A chip with room for its widest reading keeps its width as the
    /// reading changes, so the chips to its left stay put.
    #[test]
    fn a_reading_that_changes_width_moves_no_chip() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        ui.add("header", Header::new(&["Play", "Debug"]));
        ui.add_full_page("p", "col(header:60)").unwrap();
        let mut at = |cpu: &str| {
            ui.get_mut::<Header>("header").unwrap().chips = vec![
                Chip::new(cpu, Tone::Good).room("CPU 100 %"),
                Chip::new("RAM 17 %", Tone::Good).room("RAM 100 %"),
            ];
            let mut c = SoftCanvas::new(1280, 800);
            ui.paint(&mut c);
            let h = ui.get::<Header>("header").unwrap();
            (h.chip_rect(0).unwrap(), h.chip_rect(1).unwrap())
        };
        let (cpu1, ram1) = at("CPU 1 %");
        let (cpu100, ram100) = at("CPU 100 %");
        assert_eq!((cpu1, ram1), (cpu100, ram100));
        // A chip with no room is as wide as its text.
        assert!(Chip::new("CPU 1 %", Tone::Good).room.is_none());
    }

    /// A chip too long for the bar is left out, with the chips after it.
    /// One with a short form says that instead, cut to the room left if
    /// that is too long as well, and stays where a tap finds it; and it
    /// says its short form before a chip after it is left out.
    #[test]
    fn a_chip_with_a_short_form_keeps_its_place() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut h = Header::app("Play");
        h.folder_icon = Some(false);
        ui.add("header", h);
        ui.add_full_page("p", "col(header:60)").unwrap();
        let bar = ui.rect_of("header").unwrap();
        let limit = ui
            .get::<Header>("header")
            .unwrap()
            .tab_rect(bar, APP_TABS.len())
            .x;
        let size = ui.theme.size_small;
        let status = || Chip::new("CPU 24 %  RAM 38 %", Tone::Good).room("CPU 100 %  RAM 100 %");
        let long = "Phrase: Six eight ostinato, latched · 120 BPM";
        let drawn = |ui: &mut Ui, chips: Vec<Chip>| -> Vec<Option<Rect>> {
            let n = chips.len();
            ui.get_mut::<Header>("header").unwrap().chips = chips;
            let mut c = SoftCanvas::new(1280, 800);
            ui.paint(&mut c);
            let h = ui.get::<Header>("header").unwrap();
            (0..n).map(|i| h.chip_rect(i)).collect()
        };
        // As it was: no room, no chip, and none after it.
        let got = drawn(
            &mut ui,
            vec![
                status(),
                Chip::new(long, Tone::Warn),
                Chip::new("MAKE", Tone::Dim),
            ],
        );
        assert!(got[0].is_some() && got[1].is_none() && got[2].is_none());
        // Its short form fits: all of it, as wide as its words.
        let short = "Six eight ostinato";
        let got = drawn(
            &mut ui,
            vec![status(), Chip::new(long, Tone::Warn).short(short)],
        );
        let r = got[1].expect("the short form shows");
        assert_eq!(r.w, ui.fonts.measure(FontId::Sans, size, short) + 40);
        assert!(r.x >= limit);
        assert_eq!(
            ui.touch(1, Touch::Down, r.x + 5, 30).map(|e| e.kind),
            Some(EventKind::Select(1)),
            "a tap finds it"
        );
        ui.touch(1, Touch::Up, r.x + 5, 30);
        // A short form too long as well is cut to the room: the chip
        // reaches from the chip before it to where the tabs end.
        let got = drawn(
            &mut ui,
            vec![status(), Chip::new(long, Tone::Warn).short(long)],
        );
        let cut = got[1].expect("cut, not left out");
        assert_eq!(cut.x, limit);
        assert_eq!(cut.right(), got[0].unwrap().x - 8);
        // A chip that fits says all of it, short form or not.
        let got = drawn(
            &mut ui,
            vec![status(), Chip::new("Arp off", Tone::Dim).short("Off")],
        );
        let fits = got[1].unwrap();
        assert_eq!(fits.w, ui.fonts.measure(FontId::Sans, size, "Arp off") + 40);
        // A chip that fits alone, but leaves no room for the chip after
        // it, gives up its extra words first: both show.
        let (all, name) = ("Phrase: Offbeat pump · 120 BPM", "Phrase: Offbeat pump");
        let make = || Chip::new("MAKE", Tone::Dim);
        let got = drawn(&mut ui, vec![status(), Chip::new(all, Tone::Warn), make()]);
        assert!(got[1].is_some() && got[2].is_none(), "as it was: {got:?}");
        let got = drawn(
            &mut ui,
            vec![status(), Chip::new(all, Tone::Warn).short(name), make()],
        );
        assert_eq!(
            got[1].unwrap().w,
            ui.fonts.measure(FontId::Sans, size, name) + 40
        );
        let last = got[2].expect("the chip after it keeps its place");
        assert!(last.x >= limit);
        assert_eq!(last.right(), got[1].unwrap().x - 8);
        // With next to no room left even the short form is left out.
        let got = drawn(
            &mut ui,
            vec![
                status(),
                Chip::new("Arp: Up 1/16, latched · 120 BPM", Tone::Warn),
                Chip::new(long, Tone::Warn).short(short),
            ],
        );
        assert!(got[1].is_some() && got[2].is_none());
    }

    #[test]
    fn the_mixer_icon_sits_at_the_right_edge_and_the_chips_move_left() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut h = Header::new(&["Play", "Clips"]);
        h.chips = vec![Chip::new("Kit 1 of 6 · Bank A", Tone::Accent)];
        h.mixer_icon = Some(false);
        ui.add("header", h);
        ui.add("rest", Label::new("", Tone::Dim, Align::Left));
        ui.add_page("p", "col(header:60, rest:*)").unwrap();
        ui.paint(&mut SoftCanvas::new(1280, 800));
        let r = ui.rect_of("header").unwrap();
        let h = ui.get::<Header>("header").unwrap();
        let icon = h.icon_rect(r).unwrap();
        assert_eq!(icon.right(), r.right());
        assert!(
            h.chip_rect(0).unwrap().right() < icon.x,
            "the chip is left of the icon"
        );
        let e = ui.touch(1, Touch::Down, icon.x + 10, icon.y + 10).unwrap();
        assert_eq!(e.kind, EventKind::Select(MIXER_ICON));
    }

    #[test]
    fn the_io_icon_sits_left_of_the_mixer_icon_and_the_chips_move_left_again() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut h = Header::new(&["Play", "Arrange"]);
        h.chips = vec![Chip::new("Kit 1 of 6 · Bank A", Tone::Accent)];
        h.mixer_icon = Some(false);
        h.io_icon = Some(true);
        ui.add("header", h);
        ui.add("rest", Label::new("", Tone::Dim, Align::Left));
        ui.add_page("p", "col(header:60, rest:*)").unwrap();
        ui.paint(&mut SoftCanvas::new(1280, 800));
        let r = ui.rect_of("header").unwrap();
        let h = ui.get::<Header>("header").unwrap();
        let (mixer, io) = (h.icon_rect(r).unwrap(), h.io_icon_rect(r).unwrap());
        assert_eq!(mixer.right(), r.right());
        assert_eq!(io.right(), mixer.x - 8, "just left of the mixer icon");
        assert!(
            h.chip_rect(0).unwrap().right() < io.x,
            "the chip is left of both"
        );
        let e = ui.touch(1, Touch::Down, io.x + 10, io.y + 10).unwrap();
        assert_eq!(e.kind, EventKind::Select(IO_ICON));
        let e = ui
            .touch(2, Touch::Down, mixer.x + 10, mixer.y + 10)
            .unwrap();
        assert_eq!(e.kind, EventKind::Select(MIXER_ICON));
    }

    #[test]
    fn the_folder_icon_sits_at_the_right_edge_and_a_tap_on_it_says_so() {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 800);
        let mut h = Header::new(&["Play", "Pads"]);
        h.chips = vec![Chip::new("MAKE", Tone::Normal)];
        h.folder_icon = Some(false);
        ui.add("header", h);
        ui.add("rest", Label::new("", Tone::Dim, Align::Left));
        ui.add_page("p", "col(header:60, rest:*)").unwrap();
        ui.paint(&mut SoftCanvas::new(1280, 800));
        let r = ui.rect_of("header").unwrap();
        let h = ui.get::<Header>("header").unwrap();
        let folder = h.folder_icon_rect(r).unwrap();
        assert_eq!(folder.right(), r.right());
        assert!(h.io_icon_rect(r).is_none() && h.icon_rect(r).is_none());
        assert!(
            h.chip_rect(0).unwrap().right() < folder.x,
            "the chip is left of the icon"
        );
        assert!(h
            .targets(r)
            .iter()
            .any(|t| t.id == "folder-icon" && t.rect == folder));
        let e = ui
            .touch(1, Touch::Down, folder.x + 10, folder.y + 10)
            .unwrap();
        assert_eq!(e.kind, EventKind::Select(FOLDER_ICON));
    }
}
