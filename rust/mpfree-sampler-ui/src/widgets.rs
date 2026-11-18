//! The widgets the sampler's screens add to `mpfree-ui`'s library. Like
//! those, they only show what the page put in their fields and report
//! touches.

use crate::panel::{self, Control, Look, Panel};
use mpfree_ui::qlink;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

// ---------------------------------------------------------------- pad bar

/// The bar over the pad editor: the pad's colour and name, a line about its
/// sound, and the editor's tabs (or one button) on the right. A tap on an
/// enabled tab gives [`EventKind::Tab`].
pub struct PadBar {
    pub color: Color,
    pub title: String,
    pub subtitle: String,
    pub tabs: Vec<String>,
    pub active: usize,
    /// Tabs that can be opened; the others are drawn faint and ignore taps.
    pub enabled: Vec<bool>,
    /// The title is a name that can be typed: a dotted line under it, and
    /// a tap on it (the swatch, title and subtitle) gives
    /// [`EventKind::Type`] 0, for the text keyboard.
    pub nameable: bool,
    /// A button in the switch's place (the Pads tab's "Browse for A5"):
    /// drawn raised with a folder at its left, always on; a tap gives
    /// [`EventKind::Tab`] 0. Used when `tabs` is empty.
    pub button: Option<String>,
    /// The kit's name on a button left of `button` ("Kit Boom Bap", with
    /// a chevron: the Pads tab's Kit, which opens the list of kits); a tap
    /// gives [`EventKind::Tab`] 1. Only beside `button`.
    pub kit: Option<String>,
    /// `button` is a switch, on or off (the Pads tab's Perform view's "On
    /// the pads"): a light at its left in place of the folder, lit and
    /// the button outlined while on. A tap still gives
    /// [`EventKind::Tab`] 0.
    pub switch: Option<bool>,
}

const SEG_H: i32 = 40;
const SEG_PAD: i32 = 4;
const SEG_TAB_W: i32 = 84;
/// The width of the bar's button ([`PadBar::button`]).
const BUTTON_W: i32 = 220;
/// The width of the kit's button ([`PadBar::kit`]) and the room before
/// the bar's button.
const KIT_W: i32 = 220;
const KIT_GAP: i32 = 10;

impl PadBar {
    pub fn new(tabs: &[&str]) -> PadBar {
        PadBar {
            color: Color::rgb(0xFF8A3D),
            title: String::new(),
            subtitle: String::new(),
            tabs: tabs.iter().map(|s| s.to_string()).collect(),
            active: 0,
            enabled: vec![true; tabs.len()],
            nameable: false,
            button: None,
            kit: None,
            switch: None,
        }
    }

    /// The part a tap names the pad in: the swatch, title and subtitle.
    pub fn name_rect(&self, r: Rect) -> Rect {
        let right = self.kit_rect(r).unwrap_or_else(|| self.tabs_rect(r)).x;
        Rect::new(r.x, r.y, (right - r.x - 10).max(0), r.h)
    }

    /// The kit's button, when it has one (beside the bar's button).
    pub fn kit_rect(&self, r: Rect) -> Option<Rect> {
        self.kit.as_ref()?;
        let b = self.button_rect(r)?;
        Some(Rect::new(b.x - KIT_GAP - KIT_W, b.y, KIT_W, b.h))
    }

    /// The bar's button, when it has one (and no tabs).
    pub fn button_rect(&self, r: Rect) -> Option<Rect> {
        self.button.as_ref().filter(|_| self.tabs.is_empty())?;
        Some(self.tabs_rect(r))
    }

    /// The box holding the tabs (or the button).
    fn tabs_rect(&self, r: Rect) -> Rect {
        if self.tabs.is_empty() && self.button.is_some() {
            let h = SEG_H + 2 * SEG_PAD;
            return Rect::new(r.right() - 10 - BUTTON_W, r.y + (r.h - h) / 2, BUTTON_W, h);
        }
        let w = self.tabs.len() as i32 * SEG_TAB_W + 2 * SEG_PAD;
        let h = SEG_H + 2 * SEG_PAD;
        Rect::new(r.right() - 10 - w, r.y + (r.h - h) / 2, w, h)
    }

    /// Tab `i`'s rectangle.
    pub fn tab_rect(&self, r: Rect, i: usize) -> Rect {
        let b = self.tabs_rect(r);
        Rect::new(
            b.x + SEG_PAD + i as i32 * SEG_TAB_W,
            b.y + SEG_PAD,
            SEG_TAB_W,
            SEG_H,
        )
    }
}

impl Widget for PadBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let sq = 36.min(r.h - 16).max(8);
        let swatch = Rect::new(r.x + t.pad, r.y + (r.h - sq) / 2, sq, sq);
        cx.canvas.fill_round_rect(swatch, 8, self.color);
        let tabs = self.kit_rect(r).unwrap_or_else(|| self.tabs_rect(r));
        let text = Rect::new(
            swatch.right() + 14,
            r.y + 6,
            (tabs.x - swatch.right() - 24).max(0),
            r.h - 12,
        );
        let (top, bottom) = text.take_top(text.h / 2 + 2);
        cx.text(
            FontId::Bold,
            t.size_title + 1.0,
            top,
            Align::Left,
            t.text,
            &self.title,
        );
        if self.nameable {
            // A dotted line under the title: it can be typed.
            let w = cx
                .measure(FontId::Bold, t.size_title + 1.0, &self.title)
                .min(top.w);
            let y = top.bottom() - 3;
            let mut x = top.x;
            while x + 4 <= top.x + w {
                cx.canvas.fill_rect(Rect::new(x, y, 4, 2), t.text_dim);
                x += 8;
            }
        }
        cx.text(
            FontId::Sans,
            t.size_small,
            bottom,
            Align::Left,
            t.text_dim,
            &self.subtitle,
        );
        if let (Some(k), Some(name)) = (self.kit_rect(r), self.kit.as_ref()) {
            // The kit's button: raised, "Kit" faint before the name, a
            // chevron at its right (it opens a list).
            cx.canvas.fill_round_rect(k, 12, t.raised);
            let x = k.x + 16;
            let kw = cx.text(
                FontId::Bold,
                t.size_body,
                Rect::new(x, k.y, 60, k.h),
                Align::Left,
                t.text_dim,
                "Kit",
            );
            let chev = 14;
            cx.text(
                FontId::Bold,
                t.size_body,
                Rect::new(
                    x + kw + 8,
                    k.y,
                    k.right() - 16 - chev - 8 - (x + kw + 8),
                    k.h,
                ),
                Align::Left,
                t.text,
                name,
            );
            // The chevron: two strokes meeting at the bottom.
            let (cx0, cy0) = (k.right() - 16 - chev, k.y + k.h / 2 - 3);
            for i in 0..chev / 2 {
                let y = cy0 + i * 6 / (chev / 2);
                cx.canvas.fill_rect(Rect::new(cx0 + i, y, 2, 2), t.text_dim);
                cx.canvas
                    .fill_rect(Rect::new(cx0 + chev - 2 - i, y, 2, 2), t.text_dim);
            }
        }
        if let (Some(b), Some(label), Some(on)) =
            (self.button_rect(r), self.button.as_ref(), self.switch)
        {
            // A switch: a light at its left, lit and outlined while on.
            cx.canvas.fill_round_rect(b, 12, t.raised);
            if on {
                cx.canvas.stroke_round_rect(b, 12, 2, t.accent);
            }
            let dot = Rect::new(b.x + 18, b.y + b.h / 2 - 5, 10, 10);
            cx.canvas
                .fill_round_rect(dot, 5, if on { t.accent } else { t.text_faint });
            let text = Rect::new(dot.right() + 12, b.y, b.right() - dot.right() - 22, b.h);
            cx.text(FontId::Bold, t.size_body, text, Align::Left, t.text, label);
            return;
        }
        if let (Some(b), Some(label)) = (self.button_rect(r), self.button.as_ref()) {
            // The button: raised, a folder at its left, the label after it.
            cx.canvas.fill_round_rect(b, 12, t.raised);
            let (fw, fh) = (20, 15);
            let (x0, y0) = (b.x + 16, b.y + (b.h - fh) / 2 + 1);
            cx.canvas
                .fill_round_rect(Rect::new(x0, y0 - 3, 9, 5), 2, t.text);
            cx.canvas
                .fill_round_rect(Rect::new(x0, y0, fw, fh), 3, t.text);
            cx.canvas
                .fill_round_rect(Rect::new(x0 + 2, y0 + 4, fw - 4, fh - 6), 2, t.raised);
            let text = Rect::new(x0 + fw + 10, b.y, b.right() - x0 - fw - 20, b.h);
            cx.text(FontId::Bold, t.size_body, text, Align::Left, t.text, label);
            return;
        }
        cx.canvas.fill_round_rect(tabs, 12, t.bg);
        for (i, name) in self.tabs.iter().enumerate() {
            let tr = self.tab_rect(r, i);
            let on = i == self.active;
            let enabled = self.enabled.get(i).copied().unwrap_or(false);
            if on {
                cx.canvas.fill_round_rect(tr, 9, t.raised);
            }
            let color = if on {
                t.text
            } else if enabled {
                t.text_dim
            } else {
                t.text_faint
            };
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body,
                tr,
                Align::Center,
                color,
                name,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = (0..self.tabs.len())
            .map(|i| {
                // Faint tabs, and the tab already open, change nothing.
                let on = self.enabled.get(i).copied().unwrap_or(false) && i != self.active;
                Target::tap("tab", i, self.tabs[i].clone(), self.tab_rect(r, i)).disabled_if(!on)
            })
            .collect();
        if self.nameable {
            out.push(Target::tap(
                "name",
                0,
                self.title.clone(),
                self.name_rect(r),
            ));
        }
        if let (Some(b), Some(label)) = (self.button_rect(r), self.button.as_ref()) {
            out.push(Target::tap("button", 0, label.clone(), b));
        }
        if let (Some(k), Some(name)) = (self.kit_rect(r), self.kit.as_ref()) {
            out.push(Target::tap("kit", 0, format!("Kit {name}"), k));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if self.nameable && self.name_rect(r).contains(x, y) {
            return Some(EventKind::Type(0));
        }
        if self.button_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Tab(0));
        }
        if self.kit_rect(r).is_some_and(|k| k.contains(x, y)) {
            return Some(EventKind::Tab(1));
        }
        let i = (0..self.tabs.len()).find(|&i| self.tab_rect(r, i).contains(x, y))?;
        self.enabled
            .get(i)
            .copied()
            .unwrap_or(false)
            .then_some(EventKind::Tab(i))
    }
}

// ---------------------------------------------------------------- knob card

/// A Q-Link's card: a knob showing where its marker sits in the sound, the
/// Q-Link's job, and the marker's position in bars and beats (or seconds).
/// In the page's Q-Link column (less than twice as wide as it is high) it is
/// drawn as the shared column's cards are (`mpfree_ui::qlink`): the knob's
/// number and the job centred over the ring with the value in it; in a
/// strip, the ring beside the text.
///
/// A tap gives [`EventKind::Select`]`(0)`; the page makes it the wheel's
/// focus. A tap on a faint card with a reason in `why` gives
/// [`EventKind::Select`]`(1)`, for the page to say why. A finger dragged up
/// or down the card turns its knob, as on the shared column: an
/// [`EventKind::Step`] with the card's `knob` and the detents (up is
/// positive), a detent per [`mpfree_ui::qlink::DRAG_DETENT`] pixels.
pub struct KnobCard {
    /// "Q-Link 1 · Start": the knob, then its job.
    pub label: String,
    /// The page ("Wave"): not drawn since the column narrowed (Jay, 17
    /// Nov), kept for the page's tests.
    pub scope: String,
    /// Which Q-Link (0 to 3): the index a drag's steps carry.
    pub knob: usize,
    pub value: String,
    /// Small text after the value (ticks, or frames).
    pub sub: String,
    /// 0.0 to 1.0: how far round the knob's arc is lit.
    pub pos: f32,
    pub color: Color,
    /// The card the wheel zooms round: outlined.
    pub focused: bool,
    /// Nothing for this Q-Link to move (a loop point without a loop).
    pub enabled: bool,
    /// Why the card is faint, for the hint line; empty for no reason.
    pub why: String,
    /// A finger on the card: where it went down (or the last detent), and
    /// whether it has turned the knob yet.
    drag: Option<(i32, bool)>,
}

/// The knob's arc: from 135° (down-left) round 270° to 45° (down-right).
const ARC_START: f32 = 135.0;
const ARC_SWEEP: f32 = 270.0;

impl KnobCard {
    pub fn new(label: &str, color: Color) -> KnobCard {
        KnobCard {
            label: label.to_string(),
            scope: String::new(),
            knob: 0,
            value: String::new(),
            sub: String::new(),
            pos: 0.0,
            color,
            focused: false,
            enabled: true,
            why: String::new(),
            drag: None,
        }
    }

    /// The card for Q-Link `knob` (0 to 3) on the page `scope`.
    pub fn on(mut self, knob: usize, scope: &str) -> KnobCard {
        self.knob = knob;
        self.scope = scope.to_string();
        self
    }

    /// The job, without the knob's name before it: "Start".
    pub fn job(&self) -> &str {
        self.label
            .split_once(" · ")
            .map_or(self.label.as_str(), |(_, j)| j)
    }

    /// As the shared column's card: a coloured edge; "Q1" and the job
    /// centred; the ring with the value in it, and the small text under it.
    fn paint_tall(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let col = if self.enabled {
            self.color
        } else {
            t.text_faint
        };
        cx.canvas.fill_round_rect(r, t.radius, t.surface);
        if self.focused && self.enabled {
            cx.canvas
                .stroke_round_rect(r, t.radius, 2, t.outline.mix(self.color, 0.6));
        }
        if self.enabled {
            cx.canvas
                .fill_round_rect(Rect::new(r.x, r.y + 14, 4, (r.h - 28).max(0)), 2, col);
        }
        // As the shared column's cards: the number and the job centred
        // over the knob, where the job lives not drawn.
        let ink = if self.enabled { t.text } else { t.text_faint };
        let rest = qlink::card_head(cx, qlink::card_inner(r), self.knob, self.job(), col, ink);
        let (ring, sub) = if self.sub.is_empty() {
            (rest, None)
        } else {
            let (sub, ring) = rest.take_bottom(16);
            (ring, Some(sub))
        };
        let d = ring.h.min(ring.w - 10).max(20);
        let (ox, oy) = (ring.x + ring.w / 2, ring.y + ring.h / 2 + 2);
        let rad = d as f32 / 2.0 - 5.0;
        let lit = if self.enabled { self.pos } else { 0.0 };
        mpfree_ui::draw::ring(cx, ox, oy, rad, lit, false, col);
        let vr = Rect::new(ox - rad as i32, oy - 12, 2 * rad as i32, 24);
        let value = if self.enabled {
            self.value.as_str()
        } else {
            "—"
        };
        // A long value ("12 of 15") a size smaller, to stay inside the ring.
        let px = qlink::fitting(cx, value, vr.w - 6);
        cx.text(
            FontId::Bold,
            px,
            vr,
            Align::Center,
            if self.enabled { t.text } else { t.text_faint },
            value,
        );
        if let (Some(sub), true) = (sub, self.enabled) {
            cx.text(
                FontId::Sans,
                t.size_small,
                sub,
                Align::Center,
                t.text_dim,
                &self.sub,
            );
        }
    }
}

/// Dots along an arc of the circle at (cx, cy) with radius `rad`, from
/// `from` degrees round `sweep` degrees (clockwise on screen).
fn arc(cx: &mut PaintCx, (x, y): (f32, f32), rad: f32, from: f32, sweep: f32, dot: i32, c: Color) {
    if sweep <= 0.0 {
        return;
    }
    let steps = (sweep / 3.0).ceil().max(1.0) as i32;
    for k in 0..=steps {
        let a = (from + sweep * k as f32 / steps as f32).to_radians();
        let (px, py) = (x + rad * a.cos(), y + rad * a.sin());
        let d = Rect::new(
            (px - dot as f32 / 2.0).round() as i32,
            (py - dot as f32 / 2.0).round() as i32,
            dot,
            dot,
        );
        cx.canvas.fill_round_rect(d, dot / 2, c);
    }
}

impl Widget for KnobCard {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        // In a column (`qlink::COLUMN_WIDTH` wide) rather than a strip
        // (about three times as wide as high).
        if r.w < 2 * r.h {
            return self.paint_tall(cx, r);
        }
        let t = cx.theme.clone();
        cx.panel(r);
        if self.focused && self.enabled {
            cx.canvas
                .stroke_round_rect(r, t.radius, 2, t.outline.mix(self.color, 0.6));
        }
        let rad = (r.h as f32 / 2.0 - 22.0).clamp(10.0, 20.0);
        let centre = (
            r.x as f32 + t.pad as f32 + rad + 2.0,
            r.y as f32 + r.h as f32 / 2.0,
        );
        arc(cx, centre, rad, ARC_START, ARC_SWEEP, 5, t.raised);
        let color = if self.enabled {
            self.color
        } else {
            t.text_faint
        };
        if self.enabled {
            arc(
                cx,
                centre,
                rad,
                ARC_START,
                ARC_SWEEP * self.pos.clamp(0.0, 1.0),
                5,
                color,
            );
            let a = (ARC_START + ARC_SWEEP * self.pos.clamp(0.0, 1.0)).to_radians();
            let (px, py) = (centre.0 + rad * a.cos(), centre.1 + rad * a.sin());
            cx.canvas
                .fill_round_rect(Rect::new(px as i32 - 4, py as i32 - 4, 9, 9), 4, t.text);
        }
        let left = (centre.0 + rad + 14.0) as i32;
        let text = Rect::new(left, r.y + 12, (r.right() - t.pad - left).max(0), r.h - 24);
        let (head, body) = text.take_top(22);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.text_dim,
            &self.label.to_uppercase(),
        );
        let value = if self.enabled {
            self.value.as_str()
        } else {
            "—"
        };
        let w = cx.text(
            FontId::Bold,
            t.size_title + 5.0,
            body,
            Align::Left,
            if self.enabled { t.text } else { t.text_faint },
            value,
        );
        if self.enabled && !self.sub.is_empty() {
            let sub = Rect::new(body.x + w + 6, body.y + 3, (body.w - w - 6).max(0), body.h);
            cx.text(
                FontId::Sans,
                t.size_small,
                sub,
                Align::Left,
                t.text_dim,
                &self.sub,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        // The card the wheel is on already: a tap changes nothing.
        vec![Target::tap("card", 0, self.label.clone(), r)
            .disabled_if(!self.enabled || self.focused)]
    }

    fn touch(&mut self, _f: u64, phase: Touch, _x: i32, y: i32, _r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down if self.enabled => {
                self.drag = Some((y, false));
                Some(EventKind::Select(0))
            }
            Touch::Down => (!self.why.is_empty()).then_some(EventKind::Select(1)),
            Touch::Move => {
                let (from, _) = self.drag?;
                let detents = (from - y) / mpfree_ui::qlink::DRAG_DETENT;
                if detents == 0 {
                    return None;
                }
                self.drag = Some((from - detents * mpfree_ui::qlink::DRAG_DETENT, true));
                Some(EventKind::Step {
                    index: self.knob,
                    delta: detents,
                })
            }
            _ => {
                self.drag = None;
                None
            }
        }
    }
}

// ---------------------------------------------------------------- settings

/// One setting: its name and its value as text.
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub name: String,
    pub value: String,
}

/// A titled list of settings, each with its value between arrows, compact
/// enough to sit beside a waveform. A tap on a row selects it
/// ([`EventKind::Select`]); a tap on an arrow steps it ([`EventKind::Step`]).
/// A tap on the value of a row that takes a typed number ([`Self::typed`])
/// asks for the number pad ([`EventKind::Type`]); its value is underlined.
/// The list scrolls to keep the selected row in view.
pub struct SettingList {
    pub title: String,
    pub right: String,
    pub rows: Vec<Setting>,
    pub selected: usize,
    /// The wheel's push opened the selected row: its value box is outlined
    /// and its arrows lit, as in the menus, while the wheel changes it.
    pub editing: bool,
    /// The rows whose value can be typed (Jay after live test 3).
    pub typed: Vec<usize>,
    /// The rows whose value is a button that opens something, such as the
    /// Pad list's Effects: no arrows, and a tap on it is a
    /// [`EventKind::Tab`] with the row's index.
    pub buttons: Vec<usize>,
    /// A finger on a typed value: the number pad opens when it lifts, not
    /// when the wheel turned the value while it was held.
    typing: Option<(u64, usize)>,
    /// Drawn as a panel of knobs, buttons and switches in sections
    /// instead of a list ([`crate::panel`]): each row's control in
    /// `controls`, in the row's place. The rows, the picked row and the
    /// events are the list's own.
    pub panel: Option<Panel>,
    pub controls: Vec<Control>,
    /// Whether the panel outlines the control picked (not on a panel the
    /// wheel doesn't walk, such as the Play page's scenes).
    pub outlined: bool,
    /// A knob being dragged on the panel.
    drag: Option<panel::Drag>,
    /// A line or two said under the rows, wrapped to the list's width
    /// (the Pads tab's MIDI out pad: where its note goes, or why it
    /// would send nothing), and whether it warns. Drawn only as a list,
    /// and only where the rows leave room.
    pub footer: Option<(String, bool)>,
}

const SET_ROW_H: i32 = 44;
const SET_ARROW_W: i32 = 40;
const SET_VALUE_W: i32 = 210;

impl SettingList {
    pub fn new(title: &str) -> SettingList {
        SettingList {
            title: title.to_string(),
            right: String::new(),
            rows: Vec::new(),
            selected: 0,
            editing: false,
            typed: Vec::new(),
            buttons: Vec::new(),
            typing: None,
            panel: None,
            controls: Vec::new(),
            outlined: true,
            drag: None,
            footer: None,
        }
    }

    /// Where row `k`'s control is on the panel drawn in `r`.
    pub fn control_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        self.panel.as_ref()?.control_rect(r, &self.controls, k)
    }

    /// Why the panel's control for row `k` is faint, when it is and says.
    pub fn why_faint(&self, k: usize) -> Option<&str> {
        self.panel.as_ref()?;
        self.controls
            .get(k)
            .filter(|c| c.faint && !c.why.is_empty())
            .map(|c| c.why.as_str())
    }

    /// The rows' names and values, for the panel.
    fn texts(&self) -> (Vec<String>, Vec<String>) {
        self.rows
            .iter()
            .map(|r| (r.name.clone(), r.value.clone()))
            .unzip()
    }

    fn body(r: Rect) -> Rect {
        // Below the title, as PaintCx::titled_panel lays it out.
        r.inset(14).take_top(34).1
    }

    fn first(&self, visible: usize) -> usize {
        if self.rows.len() <= visible || visible == 0 {
            return 0;
        }
        self.selected
            .saturating_sub(visible / 2)
            .min(self.rows.len() - visible)
    }

    /// Row `k`'s rectangle and its value box, if it is in view.
    pub fn row_rects(&self, r: Rect, k: usize) -> Option<(Rect, Rect)> {
        let body = SettingList::body(r);
        let visible = (body.h / SET_ROW_H).max(0) as usize;
        let first = self.first(visible);
        if k < first || k >= first + visible || k >= self.rows.len() {
            return None;
        }
        let row = Rect::new(
            body.x,
            body.y + (k - first) as i32 * SET_ROW_H,
            body.w,
            SET_ROW_H - 4,
        );
        let vw = SET_VALUE_W.min(row.w * 3 / 5);
        Some((
            row,
            Rect::new(row.right() - vw - 4, row.y + 4, vw, row.h - 8),
        ))
    }

    /// [`SettingList::footer`] under the last row in view, word-wrapped
    /// to the body's width: as many lines as fit.
    fn paint_footer(&self, cx: &mut PaintCx, r: Rect, text: &str, warn: bool) {
        let t = cx.theme.clone();
        let body = SettingList::body(r);
        let visible = (body.h / SET_ROW_H).max(0) as usize;
        let shown = self.rows.len().min(visible) as i32;
        let px = t.size_small;
        let line_h = (px * 1.5) as i32;
        let (x, w) = (body.x + 14, (body.w - 28).max(0));
        let mut y = body.y + shown * SET_ROW_H + 8;
        let mut line = String::new();
        let mut lines = Vec::new();
        for word in text.split(' ') {
            let longer = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if !line.is_empty() && cx.measure(FontId::Sans, px, &longer) > w {
                lines.push(std::mem::replace(&mut line, word.to_string()));
            } else {
                line = longer;
            }
        }
        lines.push(line);
        for l in lines {
            if y + line_h > body.bottom() {
                break;
            }
            cx.text(
                FontId::Sans,
                px,
                Rect::new(x, y, w, line_h),
                Align::Left,
                if warn { t.warn } else { t.text_dim },
                &l,
            );
            y += line_h;
        }
    }
}

impl Widget for SettingList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        if let Some(p) = &self.panel {
            let (names, values) = self.texts();
            let sel = if self.outlined {
                self.selected
            } else {
                usize::MAX
            };
            return panel::paint(
                cx,
                r,
                p,
                &self.controls,
                &names,
                &values,
                &self.typed,
                sel,
                self.editing,
            );
        }
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        for k in 0..self.rows.len() {
            let Some((row, value)) = self.row_rects(r, k) else {
                continue;
            };
            let on = k == self.selected;
            if on {
                cx.canvas.fill_round_rect(row, 10, t.raised);
                cx.canvas
                    .fill_round_rect(Rect::new(row.x, row.y + 10, 4, row.h - 20), 2, t.accent);
            }
            let name = Rect::new(row.x + 14, row.y, (value.x - row.x - 18).max(0), row.h);
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body - 1.0,
                name,
                Align::Left,
                if on { t.text } else { t.text_dim },
                &self.rows[k].name,
            );
            if self.buttons.contains(&k) {
                // A button: raised, with a rim, and no arrows.
                cx.canvas.fill_round_rect(value, 8, t.raised);
                cx.canvas.stroke_round_rect(
                    value,
                    8,
                    if on { 2 } else { 1 },
                    if on { t.accent } else { t.text_dim },
                );
                cx.text(
                    FontId::Bold,
                    t.size_body - 1.0,
                    value.inset_xy(10, 0),
                    Align::Center,
                    if on { t.accent } else { t.text },
                    &self.rows[k].value,
                );
                continue;
            }
            cx.canvas
                .fill_round_rect(value, 8, if on { t.surface } else { t.raised });
            let open = on && self.editing;
            if open {
                cx.canvas.stroke_round_rect(value, 8, 2, t.accent);
            }
            let arrows = if open { t.accent } else { t.text_dim };
            let (left, mid) = value.take_left(SET_ARROW_W);
            let (right, mid) = mid.take_right(SET_ARROW_W);
            cx.text(FontId::Bold, t.size_title, left, Align::Center, arrows, "‹");
            cx.text(
                FontId::Bold,
                t.size_title,
                right,
                Align::Center,
                arrows,
                "›",
            );
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                mid,
                Align::Center,
                if on { t.accent } else { t.text },
                &self.rows[k].value,
            );
            if self.typed.contains(&k) {
                // A dotted line under the value: it can be typed.
                let y = mid.bottom() - 7;
                let mut x = mid.x + 18;
                while x + 4 <= mid.right() - 18 {
                    cx.canvas.fill_rect(Rect::new(x, y, 4, 2), t.text_dim);
                    x += 8;
                }
            }
        }
        if let Some((text, warn)) = &self.footer {
            self.paint_footer(cx, r, text, *warn);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if let Some(p) = &self.panel {
            let (names, _) = self.texts();
            return panel::targets(p, &self.controls, &names, &self.typed, r);
        }
        let mut out = Vec::new();
        for k in 0..self.rows.len() {
            let Some((row, value)) = self.row_rects(r, k) else {
                continue;
            };
            let name = &self.rows[k].name;
            // The row picked already: a tap picks nothing new.
            out.push(
                Target::tap(
                    "row",
                    k,
                    name.clone(),
                    Rect::new(row.x, row.y, value.x - row.x, row.h),
                )
                .disabled_if(k == self.selected),
            );
            if self.buttons.contains(&k) {
                out.push(Target::tap(
                    "row-button",
                    k,
                    self.rows[k].value.clone(),
                    value,
                ));
                continue;
            }
            out.push(Target::tap(
                "row-less",
                k,
                format!("{name} ‹"),
                value.take_left(SET_ARROW_W).0,
            ));
            out.push(Target::tap(
                "row-more",
                k,
                format!("{name} ›"),
                value.take_right(SET_ARROW_W).0,
            ));
        }
        out
    }

    /// A finger resting on a row (not a button) puts its value under the
    /// wheel, whichever part of the row it is on; on a panel, the control
    /// under it that has a value ([`panel::held`]).
    fn held(&self, x: i32, y: i32, r: Rect) -> Option<Target> {
        if let Some(p) = &self.panel {
            return panel::held(p, &self.controls, &self.texts().0, (x, y), r);
        }
        let k = (0..self.rows.len()).find(|&k| {
            self.row_rects(r, k)
                .is_some_and(|(row, _)| row.contains(x, y))
        })?;
        if self.buttons.contains(&k) {
            return None;
        }
        let (row, _) = self.row_rects(r, k)?;
        Some(Target::tap("row", k, self.rows[k].name.clone(), row))
    }

    /// The held row's value, a step a detent, as its arrows step it.
    fn held_step(&mut self, t: &Target, delta: i32, _fine: bool, _r: Rect) -> Option<EventKind> {
        if t.index >= self.rows.len() || delta == 0 {
            return None;
        }
        if let Some(Look::Slider { pos }) = self.controls.get(t.index).map(|c| &c.look) {
            let v = (pos + delta as f32 * 0.01).clamp(0.0, 1.0);
            return (v != *pos).then_some(EventKind::Value(v));
        }
        self.selected = t.index;
        Some(EventKind::Step {
            index: t.index,
            delta,
        })
    }

    fn touch(&mut self, f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if let Some(p) = &self.panel {
            let ev = panel::touch(
                p,
                &self.controls,
                &self.typed,
                &mut self.drag,
                (f, phase, x, y),
                r,
            )?;
            if let EventKind::Select(k)
            | EventKind::Step { index: k, .. }
            | EventKind::Type(k)
            | EventKind::Tab(k) = ev
            {
                // A faint control tapped only says why: the wheel stays.
                if !self.controls.get(k).is_some_and(|c| c.faint) {
                    self.selected = k;
                }
            }
            return Some(ev);
        }
        match phase {
            Touch::Down => {}
            // A typed value opens the number pad on the lift, so a value
            // turned by the wheel while held (a cancel) opens nothing.
            Touch::Up => {
                let (_, k) = self.typing.take().filter(|(g, _)| *g == f)?;
                return (k < self.rows.len()).then_some(EventKind::Type(k));
            }
            Touch::Cancel => {
                self.typing = self.typing.filter(|(g, _)| *g != f);
                return None;
            }
            Touch::Move => return None,
        }
        let k = (0..self.rows.len()).find(|&k| {
            self.row_rects(r, k)
                .is_some_and(|(row, _)| row.contains(x, y))
        })?;
        let (_, value) = self.row_rects(r, k)?;
        self.selected = k;
        if value.contains(x, y) && self.buttons.contains(&k) {
            Some(EventKind::Tab(k))
        } else if value.contains(x, y) && x < value.x + SET_ARROW_W {
            Some(EventKind::Step {
                index: k,
                delta: -1,
            })
        } else if value.contains(x, y) && x >= value.right() - SET_ARROW_W {
            Some(EventKind::Step { index: k, delta: 1 })
        } else if value.contains(x, y) && self.typed.contains(&k) {
            self.typing = Some((f, k));
            Some(EventKind::Select(k))
        } else {
            Some(EventKind::Select(k))
        }
    }
}

// ---------------------------------------------------------------- info card

/// A small card with a title and one line, such as what the wheel does now.
pub struct InfoCard {
    pub title: String,
    pub text: String,
}

impl InfoCard {
    pub fn new(title: &str) -> InfoCard {
        InfoCard {
            title: title.to_string(),
            text: String::new(),
        }
    }
}

impl Widget for InfoCard {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        // Under a list (56 px) the two lines sit closer than in a strip.
        let (pad_y, head_h) = if r.h < 70 { (7, 18) } else { (12, 22) };
        let inner = Rect::new(
            r.x + t.pad,
            r.y + pad_y,
            (r.w - 2 * t.pad).max(0),
            r.h - 2 * pad_y,
        );
        let (head, body) = inner.take_top(head_h);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.text_dim,
            &self.title.to_uppercase(),
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            body,
            Align::Left,
            t.text,
            &self.text,
        );
    }
}

// ---------------------------------------------------------------- buttons

/// A row of buttons sharing the width. A tap on an enabled button `i`
/// gives [`EventKind::Tab`]`(i)`; the `primary` one is filled with the
/// accent colour. A tap on a faint one with a reason in `why` gives
/// [`EventKind::Select`]`(i)`, for the page to say why.
pub struct ButtonRow {
    pub labels: Vec<String>,
    pub enabled: Vec<bool>,
    pub primary: Option<usize>,
    /// Why each faint button is faint, for the hint line (Jay, 13 Nov:
    /// faint controls say why); empty for none.
    pub why: Vec<String>,
}

const BUTTON_GAP: i32 = 8;

impl ButtonRow {
    pub fn new(labels: &[&str]) -> ButtonRow {
        ButtonRow {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            enabled: vec![true; labels.len()],
            primary: None,
            why: Vec::new(),
        }
    }

    /// Why button `i` is faint, if it is and there is a reason.
    pub fn why_faint(&self, i: usize) -> Option<&str> {
        let on = self.enabled.get(i).copied().unwrap_or(false);
        let why = self
            .why
            .get(i)
            .map(String::as_str)
            .filter(|w| !w.is_empty());
        if on {
            None
        } else {
            why
        }
    }

    /// Button `i`'s rectangle. The primary button takes twice the room.
    pub fn button_rect(&self, r: Rect, i: usize) -> Rect {
        let n = self.labels.len().max(1) as i32;
        let shares = n + i32::from(self.primary.is_some_and(|p| p < self.labels.len()));
        let inner = r.inset(6);
        let unit = (inner.w - BUTTON_GAP * (n - 1)).max(0) / shares.max(1);
        let width = |k: usize| {
            if self.primary == Some(k) {
                2 * unit
            } else {
                unit
            }
        };
        let x = inner.x + (0..i).map(|k| width(k) + BUTTON_GAP).sum::<i32>();
        let w = if i + 1 == self.labels.len() {
            inner.right() - x
        } else {
            width(i)
        };
        Rect::new(x, inner.y, w.max(0), inner.h)
    }
}

impl Widget for ButtonRow {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        for (i, label) in self.labels.iter().enumerate() {
            let b = self.button_rect(r, i);
            let enabled = self.enabled.get(i).copied().unwrap_or(false);
            let primary = self.primary == Some(i) && enabled;
            cx.canvas
                .fill_round_rect(b, 10, if primary { t.accent } else { t.raised });
            let color = if primary {
                t.bg
            } else if enabled {
                t.text
            } else {
                t.text_faint
            };
            cx.text(
                FontId::Bold,
                t.size_body - 1.0,
                b,
                Align::Center,
                color,
                label,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        (0..self.labels.len())
            .map(|i| {
                let on = self.enabled.get(i).copied().unwrap_or(false);
                Target::tap("button", i, self.labels[i].clone(), self.button_rect(r, i))
                    .disabled_if(!on)
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.labels.len()).find(|&i| self.button_rect(r, i).contains(x, y))?;
        if self.enabled.get(i).copied().unwrap_or(false) {
            Some(EventKind::Tab(i))
        } else {
            self.why_faint(i).map(|_| EventKind::Select(i))
        }
    }
}

// ---------------------------------------------------------------- sections

/// One chip of a [`SectionChips`]: the section's name and a short line of
/// what is in it now ("A 2 ms · D 310 ms · S 60 %").
#[derive(Clone, Debug, PartialEq)]
pub struct SectionChip {
    pub name: String,
    pub sub: String,
}

/// A page's sections as a row of chips, the one open lit with a bar under
/// it. A tap on chip `i` gives [`EventKind::Tab`]`(i)`.
pub struct SectionChips {
    pub chips: Vec<SectionChip>,
    pub selected: usize,
}

impl SectionChips {
    pub fn new(names: &[&str]) -> SectionChips {
        SectionChips {
            chips: names
                .iter()
                .map(|n| SectionChip {
                    name: n.to_string(),
                    sub: String::new(),
                })
                .collect(),
            selected: 0,
        }
    }

    /// Chip `i`'s rectangle: the chips share the width.
    pub fn chip_rect(&self, r: Rect, i: usize) -> Rect {
        let n = self.chips.len().max(1) as i32;
        let unit = (r.w - BUTTON_GAP * (n - 1)).max(0) / n;
        let x = r.x + i as i32 * (unit + BUTTON_GAP);
        let w = if i + 1 == self.chips.len() {
            r.right() - x
        } else {
            unit
        };
        Rect::new(x, r.y, w.max(0), r.h)
    }
}

impl Widget for SectionChips {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        for (i, chip) in self.chips.iter().enumerate() {
            let b = self.chip_rect(r, i);
            let open = i == self.selected;
            cx.canvas
                .fill_round_rect(b, 10, if open { t.raised } else { t.surface });
            if open {
                let bar = Rect::new(b.x + 10, b.bottom() - 4, (b.w - 20).max(0), 3);
                cx.canvas.fill_rect(bar, t.accent);
            }
            let inner = b.inset_xy(14, 6);
            let (top, bottom) = inner.take_top(inner.h / 2 + 2);
            // A name a little too long for its chip ("Instrument" on the
            // Pads tab's type switch) is drawn smaller before it is cut.
            let name_px = if cx.measure(FontId::Bold, t.size_body, &chip.name) > top.w {
                t.size_body - 3.0
            } else {
                t.size_body
            };
            cx.text(
                FontId::Bold,
                name_px,
                top,
                Align::Left,
                if open { t.text } else { t.text_dim },
                &chip.name,
            );
            // A line a little too long for its chip (eight on Play) is
            // drawn smaller before it is cut.
            let px = if cx.measure(FontId::Sans, t.size_small, &chip.sub) > bottom.w {
                t.size_small - 2.0
            } else {
                t.size_small
            };
            cx.text(
                FontId::Sans,
                px,
                bottom,
                Align::Left,
                t.text_faint,
                &chip.sub,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        // The section already open changes nothing, as the pad bar's tab.
        (0..self.chips.len())
            .map(|i| {
                Target::tap("chip", i, self.chips[i].name.clone(), self.chip_rect(r, i))
                    .disabled_if(i == self.selected)
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..self.chips.len()).find(|&i| self.chip_rect(r, i).contains(x, y))?;
        Some(EventKind::Tab(i))
    }
}

// ---------------------------------------------------------- kit tracks

/// A track on the Pads tab's "Kit of" row.
#[derive(Clone, Debug, PartialEq)]
pub struct KitTrack {
    /// The track's place (from 0).
    pub track: usize,
    /// Its name, such as "Track 3" or "Drums".
    pub name: String,
    /// What it plays: the kit's name, or why it has none ("synth, no
    /// kit").
    pub kit: String,
    pub color: Color,
    /// The armed track: the pads play its kit.
    pub armed: bool,
    /// It plays a kit, so a tap arms it; a track without one is faint.
    pub enabled: bool,
}

/// The Pads tab's "Kit of" row (the sampler review's mockup 5): the tracks
/// that play kits, each with its kit, the armed one outlined, and **+ Kit
/// track** on the right. A tap on an enabled track gives
/// [`EventKind::Select`] with its place in the project; a tap on + Kit
/// track gives [`EventKind::Tab`]`(0)`. Tracks that don't fit are left
/// out, but the armed one always shows (in the last place).
///
/// With [`KitTracks::perform`] set the row is "Pads of" (board 7): the
/// project's **Perform pads** have a chip at the far right, past a rule,
/// and a tap on it gives [`EventKind::Tab`]`(1)`. While it shows, its chip
/// is outlined in place of the armed track's, and a tap on any enabled
/// track (the armed one too) goes back to the kit's pads.
pub struct KitTracks {
    pub tracks: Vec<KitTrack>,
    /// + Kit track can add a track (faint when not).
    pub can_add: bool,
    /// The Perform chip: None for none; Some(true) while the Perform set
    /// shows.
    pub perform: Option<bool>,
}

const KT_CAPTION_W: i32 = 64;
const KT_ADD_W: i32 = 124;
const KT_GAP: i32 = 8;
const KT_CHIP_MIN: i32 = 150;
const KT_CHIP_MAX: i32 = 220;
/// The Perform chip's width, and the room for the rule before it.
const KT_PERFORM_W: i32 = 300;
const KT_RULE_W: i32 = 17;

impl KitTracks {
    pub fn new() -> KitTracks {
        KitTracks {
            tracks: Vec::new(),
            can_add: true,
            perform: None,
        }
    }

    fn inner(r: Rect) -> Rect {
        r.inset_xy(12, 6)
    }

    /// The Perform chip, when the row has one.
    pub fn perform_rect(&self, r: Rect) -> Option<Rect> {
        self.perform?;
        let b = Self::inner(r);
        Some(Rect::new(b.right() - KT_PERFORM_W, b.y, KT_PERFORM_W, b.h))
    }

    /// The room right of the tracks taken by the Perform chip and its rule.
    fn perform_room(&self) -> i32 {
        if self.perform.is_some() {
            KT_PERFORM_W + KT_RULE_W
        } else {
            0
        }
    }

    /// + Kit track's button.
    pub fn add_rect(&self, r: Rect) -> Rect {
        let b = Self::inner(r);
        let right = b.right() - self.perform_room();
        Rect::new(right - KT_ADD_W, b.y, KT_ADD_W, b.h)
    }

    /// A tap on track chip `k` arms it: an enabled track, not the armed
    /// one, unless the Perform set shows (then any enabled track goes
    /// back to the kit's pads).
    fn tappable(&self, k: &KitTrack) -> bool {
        k.enabled && (!k.armed || self.perform == Some(true))
    }

    /// The tracks shown, in order, by their place in [`KitTracks::tracks`],
    /// and each chip's width.
    pub fn shown(&self, r: Rect) -> (Vec<usize>, i32) {
        let b = Self::inner(r);
        let room = (b.w - KT_CAPTION_W - KT_ADD_W - KT_GAP - self.perform_room()).max(0);
        let fit = ((room + KT_GAP) / (KT_CHIP_MIN + KT_GAP)).max(1) as usize;
        let mut shown: Vec<usize> = (0..self.tracks.len().min(fit)).collect();
        if let Some(armed) = self.tracks.iter().position(|t| t.armed) {
            if armed >= fit {
                shown.pop();
                shown.push(armed);
            }
        }
        let n = shown.len().max(1) as i32;
        let w = ((room + KT_GAP) / n - KT_GAP).clamp(0, KT_CHIP_MAX);
        (shown, w)
    }

    /// The chip of [`KitTracks::tracks`]`[i]`, if it is shown.
    pub fn chip_rect(&self, r: Rect, i: usize) -> Option<Rect> {
        let b = Self::inner(r);
        let (shown, w) = self.shown(r);
        let k = shown.iter().position(|&s| s == i)? as i32;
        Some(Rect::new(
            b.x + KT_CAPTION_W + k * (w + KT_GAP),
            b.y,
            w,
            b.h,
        ))
    }
}

impl Default for KitTracks {
    fn default() -> KitTracks {
        KitTracks::new()
    }
}

impl Widget for KitTracks {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.panel(r);
        let b = Self::inner(r);
        let caption = Rect::new(b.x, b.y, KT_CAPTION_W, b.h);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            caption,
            Align::Left,
            t.text_dim,
            if self.perform.is_some() {
                "PADS OF"
            } else {
                "KIT OF"
            },
        );
        // While the Perform set shows, its chip is the one outlined.
        let perform_open = self.perform == Some(true);
        for (i, k) in self.tracks.iter().enumerate() {
            let Some(c) = self.chip_rect(r, i) else {
                continue;
            };
            let outlined = k.armed && !perform_open;
            cx.canvas
                .fill_round_rect(c, 10, if outlined { t.surface } else { t.raised });
            if outlined {
                cx.canvas.stroke_round_rect(c, 10, 2, t.accent);
            }
            let inner = c.inset_xy(12, 0);
            let dot = Rect::new(inner.x, c.y + c.h / 2 - 4, 8, 8);
            cx.canvas
                .fill_round_rect(dot, 4, if k.enabled { k.color } else { t.text_faint });
            let text = Rect::new(
                dot.right() + 8,
                c.y,
                (inner.right() - dot.right() - 8).max(0),
                c.h,
            );
            let name_w = cx
                .measure(FontId::Bold, t.size_small, &k.name)
                .min(text.w * 3 / 5);
            let (name, rest) = (
                Rect::new(text.x, text.y, name_w, text.h),
                Rect::new(
                    text.x + name_w + 8,
                    text.y,
                    (text.w - name_w - 8).max(0),
                    text.h,
                ),
            );
            let (ink, dim) = if k.enabled {
                (t.text, t.text_dim)
            } else {
                (t.text_faint, t.text_faint)
            };
            cx.text(FontId::Bold, t.size_small, name, Align::Left, ink, &k.name);
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                rest,
                Align::Left,
                dim,
                &k.kit,
            );
        }
        let add = self.add_rect(r);
        // A dashed outline: it makes something new.
        let c = if self.can_add {
            t.text_dim
        } else {
            t.text_faint
        };
        let (dash, gap) = (8, 6);
        let mut x = add.x + 10;
        while x + dash <= add.right() - 10 {
            cx.canvas.fill_rect(Rect::new(x, add.y, dash, 2), c);
            cx.canvas
                .fill_rect(Rect::new(x, add.bottom() - 2, dash, 2), c);
            x += dash + gap;
        }
        let mut y = add.y + 8;
        while y + dash <= add.bottom() - 8 {
            cx.canvas.fill_rect(Rect::new(add.x, y, 2, dash), c);
            cx.canvas
                .fill_rect(Rect::new(add.right() - 2, y, 2, dash), c);
            y += dash + gap;
        }
        cx.text(
            FontId::Bold,
            t.size_small,
            add,
            Align::Center,
            c,
            "+ Kit track",
        );
        if let Some(p) = self.perform_rect(r) {
            // A rule between the tracks and the set: it isn't a track.
            let x = p.x - KT_RULE_W / 2 - 1;
            cx.canvas
                .fill_rect(Rect::new(x, p.y + 6, 2, (p.h - 12).max(0)), t.outline);
            cx.canvas
                .fill_round_rect(p, 10, if perform_open { t.surface } else { t.raised });
            if perform_open {
                cx.canvas.stroke_round_rect(p, 10, 2, t.accent);
            }
            let inner = p.inset_xy(14, 0);
            let w = cx.text(
                FontId::Bold,
                t.size_small,
                inner,
                Align::Left,
                t.text,
                "Perform pads",
            );
            let rest = Rect::new(
                inner.x + w + 10,
                inner.y,
                (inner.w - w - 10).max(0),
                inner.h,
            );
            cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                rest,
                Align::Left,
                t.text_dim,
                "one set for the project",
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out: Vec<Target> = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(i, k)| {
                let c = self.chip_rect(r, i)?;
                Some(
                    Target::tap("track", k.track, k.name.clone(), c).disabled_if(!self.tappable(k)),
                )
            })
            .collect();
        out.push(Target::tap("add", 0, "+ Kit track", self.add_rect(r)).disabled_if(!self.can_add));
        if let Some(p) = self.perform_rect(r) {
            // The set showing already: the tap changes nothing.
            out.push(
                Target::tap("perform", 1, "Perform pads", p)
                    .disabled_if(self.perform == Some(true)),
            );
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        if self.add_rect(r).contains(x, y) {
            return self.can_add.then_some(EventKind::Tab(0));
        }
        if self.perform_rect(r).is_some_and(|p| p.contains(x, y)) {
            return (self.perform == Some(false)).then_some(EventKind::Tab(1));
        }
        let i = (0..self.tracks.len())
            .find(|&i| self.chip_rect(r, i).is_some_and(|c| c.contains(x, y)))?;
        let k = &self.tracks[i];
        self.tappable(k).then_some(EventKind::Select(k.track))
    }
}

// ---------------------------------------------------------------- pad map

/// One pad on the [`PadMap`].
#[derive(Clone, Debug, PartialEq)]
pub struct PadCell {
    /// The pad's name on the MPC, such as "A5".
    pub label: String,
    /// What the pad is called; empty for a pad without a sound.
    pub name: String,
    /// A short line under the name, such as "3 layers · Loop".
    pub detail: String,
    /// The clip the pad launches, if any.
    pub clip: Option<String>,
    pub color: Color,
    /// What kind of pad it is when not a plain sample ("Loop", "Synth",
    /// "MIDI"), drawn small at the top right where no clip is.
    pub badge: Option<String>,
}

/// A kit's pads, four to a row as on the MPC (pad 1 bottom left), small
/// enough to sit beside the pad's settings. A tap on a pad picks it
/// ([`EventKind::Select`]); a tap on the pad already picked plays it while
/// the finger is down ([`EventKind::Pad`]).
///
/// With [`PadMap::banks`] set, a strip of the kit's banks A to H sits over
/// the pads, each with a dot for every pad of it that has a sound, and the
/// bank shown outlined; a tap on a bank is [`EventKind::Tab`] with its
/// number (the strip answers over its whole width, no gaps).
pub struct PadMap {
    pub title: String,
    pub right: String,
    pub cells: Vec<PadCell>,
    pub selected: usize,
    /// Pads held down now (on the hardware or here).
    pub lit: Vec<bool>,
    /// Each bank's pads with a sound, pad 1 of the bank the lowest bit;
    /// empty for no strip.
    pub banks: Vec<u16>,
    /// The bank the pads are of, in [`PadMap::banks`].
    pub bank: usize,
    held: Option<(u64, usize)>,
}

const MAP_COLS: usize = 4;
const MAP_GAP: i32 = 8;
/// The bank strip's height, and the gap under it.
const BANKS_H: i32 = 52;
const BANKS_GAP: i32 = 10;
const BANKS_CELL_GAP: i32 = 4;

impl PadMap {
    pub fn new(title: &str) -> PadMap {
        PadMap {
            title: title.to_string(),
            right: String::new(),
            cells: Vec::new(),
            selected: 0,
            lit: Vec::new(),
            banks: Vec::new(),
            bank: 0,
            held: None,
        }
    }

    /// The panel's room under its title: the bank strip (if any), then
    /// the pads.
    fn body(&self, r: Rect) -> (Rect, Rect) {
        let b = r.inset(14).take_top(34).1;
        if self.banks.is_empty() {
            return (Rect::new(b.x, b.y, b.w, 0), b);
        }
        let (strip, rest) = b.take_top(BANKS_H);
        (strip, rest.take_top(BANKS_GAP).1)
    }

    /// Bank `k`'s cell on the strip, as drawn, and the slice of the strip
    /// that answers a tap for it.
    pub fn bank_rect(&self, r: Rect, k: usize) -> (Rect, Rect) {
        let strip = self.body(r).0;
        let n = self.banks.len().max(1) as i32;
        let (k, gap) = (k as i32, BANKS_CELL_GAP);
        let w = (strip.w - gap * (n - 1)) / n;
        let x = strip.x + k * (w + gap);
        let cell = Rect::new(x, strip.y, w.max(0), strip.h);
        // The answer runs to the middle of the gaps either side.
        let left = if k == 0 { strip.x } else { x - gap / 2 };
        let right = if k == n - 1 {
            strip.right()
        } else {
            x + w + gap - gap / 2
        };
        (cell, Rect::new(left, strip.y, right - left, strip.h))
    }

    /// Pad `i`'s rectangle.
    pub fn cell_rect(&self, r: Rect, i: usize) -> Rect {
        let b = self.body(r).1;
        let rows = self.cells.len().div_ceil(MAP_COLS).max(1) as i32;
        let cols = MAP_COLS as i32;
        let w = (b.w - MAP_GAP * (cols - 1)) / cols;
        let h = (b.h - MAP_GAP * (rows - 1)) / rows;
        let (col, row) = ((i % MAP_COLS) as i32, rows - 1 - (i / MAP_COLS) as i32);
        Rect::new(
            b.x + col * (w + MAP_GAP),
            b.y + row * (h + MAP_GAP),
            w.max(0),
            h.max(0),
        )
    }
}

impl Widget for PadMap {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.titled_panel(r, &self.title, &self.right);
        for (k, &sounds) in self.banks.iter().enumerate() {
            let b = self.bank_rect(r, k).0;
            let shown = k == self.bank;
            cx.canvas
                .fill_round_rect(b, 8, if shown { t.surface } else { t.raised });
            if shown {
                cx.canvas.stroke_round_rect(b, 8, 2, t.accent);
            }
            let inner = b.inset_xy(6, 5);
            let (top, rest) = inner.take_top(16);
            let letter = (b'A' + (k % 26) as u8) as char;
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                top,
                Align::Left,
                if shown { t.text } else { t.text_dim },
                &letter.to_string(),
            );
            // A 4 x 4 dot per pad, pad 1 bottom left as on the MPC.
            let (dot, step) = (5, 7);
            let x0 = rest.right() - 4 * step + (step - dot);
            let y0 = rest.bottom() - 4 * step + (step - dot);
            for p in 0..16 {
                let (col, row) = ((p % 4) as i32, 3 - (p / 4) as i32);
                let on = sounds & (1 << p) != 0;
                let d = Rect::new(x0 + col * step, y0 + row * step, dot, dot);
                let c = crate::wave_tab::pad_color(k * 16 + p);
                cx.canvas.fill_rect(d, if on { c } else { t.outline });
            }
        }
        for (i, c) in self.cells.iter().enumerate() {
            let b = self.cell_rect(r, i);
            let filled = !c.name.is_empty();
            let lit = self.lit.get(i).copied().unwrap_or(false);
            let fill = match (lit, filled) {
                (true, _) => c.color.mix(t.surface, 0.25),
                (false, true) => c.color.mix(t.surface, 0.8),
                (false, false) => t.raised,
            };
            cx.canvas.fill_round_rect(b, 10, fill);
            if filled {
                cx.canvas
                    .fill_rect(Rect::new(b.x, b.y + 8, 4, b.h - 16), c.color);
            }
            if i == self.selected {
                cx.canvas.stroke_round_rect(b, 10, 3, t.accent);
            }
            let inner = b.inset_xy(10, 6);
            let (top, rest) = inner.take_top(20);
            cx.text(
                FontId::Bold,
                t.size_small - 1.0,
                top,
                Align::Left,
                t.text_dim,
                &c.label,
            );
            if let Some(clip) = &c.clip {
                cx.text(
                    FontId::Bold,
                    t.size_small - 2.0,
                    top,
                    Align::Right,
                    t.good,
                    &format!("▶ {clip}"),
                );
            } else if let Some(badge) = c.badge.as_ref().filter(|_| filled) {
                cx.text(
                    FontId::Bold,
                    t.size_small - 3.0,
                    top,
                    Align::Right,
                    c.color,
                    badge,
                );
            }
            let (name, detail) = rest.take_top(rest.h / 2 + 4);
            if filled {
                // A name a little too long for its pad ("Delay throw") is
                // drawn smaller before it is cut.
                let px = if cx.measure(FontId::Bold, t.size_small, &c.name) > name.w {
                    t.size_small - 2.0
                } else {
                    t.size_small
                };
                cx.text(FontId::Bold, px, name, Align::Left, t.text, &c.name);
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    detail,
                    Align::Left,
                    t.text_dim,
                    &c.detail,
                );
            } else {
                cx.text(
                    FontId::Sans,
                    t.size_small - 1.0,
                    name,
                    Align::Left,
                    t.text_faint,
                    "empty",
                );
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        (0..self.cells.len())
            .map(|k| {
                let c = &self.cells[k];
                let label = if c.name.is_empty() {
                    c.label.clone()
                } else {
                    format!("{} {}", c.label, c.name)
                };
                Target::tap("pad", k, label, self.cell_rect(r, k))
            })
            .chain((0..self.banks.len()).map(|k| {
                let letter = (b'A' + (k % 26) as u8) as char;
                Target::tap("bank", k, format!("Bank {letter}"), self.bank_rect(r, k).1)
            }))
            .collect()
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                if let Some(k) =
                    (0..self.banks.len()).find(|&k| self.bank_rect(r, k).1.contains(x, y))
                {
                    return Some(EventKind::Tab(k));
                }
                let k = (0..self.cells.len()).find(|&k| self.cell_rect(r, k).contains(x, y))?;
                if k == self.selected {
                    self.held = Some((finger, k));
                    Some(EventKind::Pad {
                        index: k,
                        down: true,
                    })
                } else {
                    self.selected = k;
                    Some(EventKind::Select(k))
                }
            }
            Touch::Up | Touch::Cancel => {
                let (_, k) = self.held.take_if(|(f, _)| *f == finger)?;
                Some(EventKind::Pad {
                    index: k,
                    down: false,
                })
            }
            Touch::Move => None,
        }
    }
}
