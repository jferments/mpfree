//! The retained UI: widgets by name, pages as layouts, redraw of only what
//! changed, and finger ownership.

use crate::canvas::Canvas;
use crate::geom::Rect;
use crate::layout;
use crate::style::Styled;
use crate::text::{Align, FontId, Fonts};
use crate::theme::{Backdrop, Theme};
use crate::Color;
use std::any::Any;

/// What a widget draws with.
pub struct PaintCx<'a> {
    pub canvas: &'a mut dyn Canvas,
    pub theme: &'a Theme,
    pub fonts: &'a mut Fonts,
}

impl PaintCx<'_> {
    /// One line of text in `r` (see [`Fonts::draw_in`]).
    pub fn text(
        &mut self,
        font: FontId,
        px: f32,
        r: Rect,
        align: Align,
        color: Color,
        s: &str,
    ) -> i32 {
        self.fonts
            .draw_in(self.canvas, font, px, r, align, color, s)
    }

    pub fn measure(&mut self, font: FontId, px: f32, s: &str) -> i32 {
        self.fonts.measure(font, px, s)
    }

    /// A panel: the theme's surface with its corner radius.
    pub fn panel(&mut self, r: Rect) {
        self.canvas
            .fill_round_rect(r, self.theme.radius, self.theme.surface);
    }

    /// A panel with a title line; returns the area below the title.
    pub fn titled_panel(&mut self, r: Rect, title: &str, right: &str) -> Rect {
        self.panel(r);
        let inner = r.inset(self.theme.pad);
        let (head, body) = inner.take_top(26);
        let (px, dim) = (self.theme.size_small, self.theme.text_dim);
        self.text(
            FontId::Bold,
            px,
            head,
            Align::Left,
            dim,
            &title.to_uppercase(),
        );
        if !right.is_empty() {
            let faint = self.theme.text_dim;
            self.text(FontId::Sans, px, head, Align::Right, faint, right);
        }
        body.take_top(8).1
    }
}

/// The canvas a widget paints a part of itself through: it clears the part
/// to the background, but only where the widget's own first drawing will
/// not cover it anyway.
///
/// Most widgets start with an opaque panel over their whole rectangle, so
/// clearing first wrote most pixels twice. If the first call is an opaque
/// fill (or a background fill, or a picture) that covers the part inside
/// the current clip, nothing is cleared; if it is an opaque rounded box that covers
/// the part, only the box's corners are cleared, one pixel deeper than its
/// radius (the theme's corners, round or cut, stay inside that). Anything
/// else first clears the whole part, as before. The picture is the same
/// either way.
struct Cleared<'a> {
    inner: &'a mut dyn Canvas,
    /// The part still to clear, until the first drawing call.
    pending: Option<Rect>,
    bg: Color,
    /// Whether a fill in `bg` covers what was under it: the theme's
    /// backdrop (which such a fill becomes) is opaque.
    bg_covers: bool,
}

impl Cleared<'_> {
    /// Clears what is still to clear, less what `covers` (a box and its
    /// corner radius about to be filled opaque) will draw over.
    fn settle(&mut self, covers: Option<(Rect, i32)>) {
        let Some(part) = self.pending.take() else {
            return;
        };
        let clip = self.inner.clip();
        let holds = |outer: Rect| part.intersect(outer) == part;
        let left: Vec<Rect> = match covers {
            Some((r, radius)) if holds(clip) && holds(r) => {
                // Each corner at most half the box, so no pixel is cleared
                // twice (a see-through backdrop would show it), and where
                // the corners meet they still cover the whole box.
                let k = radius.max(0) + 1;
                let (left, right) = (k.min(r.w / 2), k.min(r.w - r.w / 2));
                let (top, bottom) = (k.min(r.h / 2), k.min(r.h - r.h / 2));
                [
                    Rect::new(r.x, r.y, left, top),
                    Rect::new(r.right() - right, r.y, right, top),
                    Rect::new(r.x, r.bottom() - bottom, left, bottom),
                    Rect::new(r.right() - right, r.bottom() - bottom, right, bottom),
                ]
                .into_iter()
                .map(|c| c.intersect(part))
                .filter(|c| !c.is_empty())
                .collect()
            }
            _ => vec![part],
        };
        if left.is_empty() {
            return;
        }
        if clip != part {
            self.inner.set_clip(part);
        }
        for c in left {
            self.inner.fill_rect(c, self.bg);
        }
        if clip != part {
            self.inner.set_clip(clip);
        }
    }
}

impl Canvas for Cleared<'_> {
    fn size(&self) -> (i32, i32) {
        self.inner.size()
    }

    fn clip(&self) -> Rect {
        self.inner.clip()
    }

    fn set_clip(&mut self, r: Rect) {
        self.inner.set_clip(r)
    }

    fn fill_rect(&mut self, r: Rect, c: Color) {
        // A box with no corners (radius -1): nothing under it to clear.
        let covers = if c == self.bg {
            self.bg_covers
        } else {
            c.a == 255
        };
        self.settle(covers.then_some((r, -1)));
        self.inner.fill_rect(r, c)
    }

    fn fill_round_rect(&mut self, r: Rect, radius: i32, c: Color) {
        self.settle((c.a == 255).then_some((r, radius)));
        self.inner.fill_round_rect(r, radius, c)
    }

    fn stroke_round_rect(&mut self, r: Rect, radius: i32, width: i32, c: Color) {
        self.settle(None);
        self.inner.stroke_round_rect(r, radius, width, c)
    }

    fn blend_mask(&mut self, x: i32, y: i32, w: usize, h: usize, mask: &[u8], c: Color) {
        self.settle(None);
        self.inner.blend_mask(x, y, w, h, mask, c)
    }

    fn draw_image(&mut self, x: i32, y: i32, img: &crate::canvas::Image) {
        // A picture is copied opaque: nothing under it to clear.
        self.settle(Some((Rect::new(x, y, img.w, img.h), -1)));
        self.inner.draw_image(x, y, img)
    }

    fn blend_shape(&mut self, shape: &crate::shape::Shape, c: Color) {
        self.settle(None);
        self.inner.blend_shape(shape, c)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Touch {
    Down,
    Move,
    Up,
    /// The touch was lost (not lifted): undo whatever it was doing.
    Cancel,
}

/// What a touch on a widget meant. The app decides what each one does.
#[derive(Clone, Debug, PartialEq)]
pub enum EventKind {
    /// A tab or button, by index.
    Tab(usize),
    /// A pad pressed (`down`) or released.
    Pad { index: usize, down: bool },
    /// A list row chosen.
    Select(usize),
    /// A slider moved, 0.0 to 1.0.
    Value(f32),
    /// A value stepped down (-1) or up (+1), in row `index`.
    Step { index: usize, delta: i32 },
    /// A waveform's marker `index` dragged to `frame`.
    Marker { index: usize, frame: u64 },
    /// A waveform scrolled or zoomed to this view.
    View { start: f64, frames_per_px: f64 },
    /// A frame of a sound picked: a tap on a waveform, or on an overview
    /// (where the view should centre).
    Frame(u64),
    /// A cell of a grid tapped (a step of a pattern: `row` is the lane's
    /// sound, `col` the step).
    Cell { row: usize, col: usize },
    /// Something switched on or off by a tap (a speaker's mute), by index.
    Toggle(usize),
    /// A stretch of row `index` picked by a swipe along it, `from` before
    /// `to` (in the widget's own units, such as song ticks), or the whole
    /// row by a tap (`from == to`): a take lane's comp.
    Span { index: usize, from: i64, to: i64 },
    /// A finger on a two-way pad: 0 to 1 left to right (`x`) and bottom to
    /// top (`y`), `down` until it lifts.
    Xy { x: f32, y: f32, down: bool },
    /// A number field's value tapped, to type a number into it (row
    /// `index`; see [`crate::numpad`]).
    Type(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiEvent {
    pub widget: String,
    pub kind: EventKind,
}

/// How a finger uses a control: what the control audit's smoke tests do to
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    /// A tap: down and up in place (a button, a row, a pad, a tab).
    Tap,
    /// A drag from the middle (a slider, a knob card, an XY pad, a marker).
    Drag,
    /// A finger held still for a moment (a long press).
    Hold,
}

/// One control a widget draws: something a finger can tap, drag or hold.
/// Widgets list theirs with [`Widget::targets`]; the control audit taps
/// each one on every sample screen and checks that it does something, and
/// the controls catalogue (`docs/dev/guides/controls.md`) explains each
/// kind by its widget and `id`.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    /// What kind of control this is, stable and unique within its widget
    /// ("add-effect", "pad", "row-value"). The catalogue is keyed by it.
    pub id: &'static str,
    /// Which one of several of the same kind (pad 3, row 2); 0 when there
    /// is only one.
    pub index: usize,
    /// What the screen shows on it, for reports ("+ Add effect").
    pub label: String,
    /// Where it answers a finger, in screen pixels.
    pub rect: Rect,
    pub gesture: Gesture,
    /// For a drag: how far the finger moves from the middle, (x, y) in
    /// pixels. Up and to the right unless the control says otherwise (a
    /// grip at an end moves inwards).
    pub towards: (i32, i32),
    /// False when the control is drawn as unavailable (faint, greyed), or
    /// is the one already chosen (the lit tab of a switch), so a press has
    /// nothing to change: the audit reports what it does but does not count
    /// it dead.
    pub enabled: bool,
}

impl Target {
    pub fn tap(id: &'static str, index: usize, label: impl Into<String>, rect: Rect) -> Target {
        Target {
            id,
            index,
            label: label.into(),
            rect,
            gesture: Gesture::Tap,
            towards: (36, -60),
            enabled: true,
        }
    }

    pub fn drag(id: &'static str, index: usize, label: impl Into<String>, rect: Rect) -> Target {
        Target {
            gesture: Gesture::Drag,
            ..Target::tap(id, index, label, rect)
        }
    }

    pub fn hold(id: &'static str, index: usize, label: impl Into<String>, rect: Rect) -> Target {
        Target {
            gesture: Gesture::Hold,
            ..Target::tap(id, index, label, rect)
        }
    }

    /// The same drag, the other way: `(dx, dy)` pixels from the middle.
    pub fn towards(self, dx: i32, dy: i32) -> Target {
        Target {
            towards: (dx, dy),
            ..self
        }
    }

    /// The same control, drawn as unavailable when `off` is true.
    pub fn disabled_if(self, off: bool) -> Target {
        Target {
            enabled: self.enabled && !off,
            ..self
        }
    }

    /// The point a test touches: the middle of the control.
    pub fn centre(&self) -> (i32, i32) {
        (self.rect.x + self.rect.w / 2, self.rect.y + self.rect.h / 2)
    }
}

/// A control on the page showing, with the name of the widget it is on.
#[derive(Clone, Debug, PartialEq)]
pub struct PageTarget {
    pub widget: String,
    pub target: Target,
}

pub trait Widget: Any + Send {
    /// Draw into `r`, which the widget fills completely (background included).
    fn paint(&self, cx: &mut PaintCx, r: Rect);

    /// A finger this widget owns went down, moved or lifted at (x, y).
    /// `finger` stays the same from down to up.
    fn touch(
        &mut self,
        _finger: u64,
        _phase: Touch,
        _x: i32,
        _y: i32,
        _r: Rect,
    ) -> Option<EventKind> {
        None
    }

    /// True while the widget animates and wants to be drawn again next frame.
    fn animating(&self) -> bool {
        false
    }

    /// Every control this widget draws in `r` as it stands now: each
    /// button, row, pad, arrow, slider or card a finger can use, where a
    /// touch on it reaches the widget. The control audit taps each one and
    /// checks that it does something; a widget that only shows things
    /// lists none. Keep it in step with `touch`: the audit also probes the
    /// widget and fails on a touch that answers outside every target.
    fn targets(&self, _r: Rect) -> Vec<Target> {
        Vec::new()
    }

    /// When the widget has changed, the parts of `r` that need drawing again;
    /// None means all of it. `paint` is then called once per part, clipped.
    fn changed_parts(&self, _r: Rect) -> Option<Vec<Rect>> {
        None
    }
}

struct Page {
    name: String,
    spec: String,
    margin: bool,
    slots: Vec<(usize, Rect)>,
}

pub struct Ui {
    pub theme: Theme,
    pub fonts: Fonts,
    size: (i32, i32),
    widgets: Vec<(String, Box<dyn Widget>, bool)>,
    pages: Vec<Page>,
    current: usize,
    full: bool,
    fingers: Vec<(u64, usize)>,
    /// Layouts from a layout file, by page name or variant key: they win
    /// over the spec the app passes whenever that page is added again.
    overrides: Vec<(String, String)>,
}

impl Ui {
    pub fn new(theme: Theme, mut fonts: Fonts, width: i32, height: i32) -> Ui {
        fonts.set_roles(theme.face_body, theme.face_title);
        Ui {
            theme,
            fonts,
            size: (width, height),
            widgets: Vec::new(),
            pages: Vec::new(),
            current: 0,
            full: true,
            fingers: Vec::new(),
            overrides: Vec::new(),
        }
    }

    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// Add a widget under `name` (replacing one of the same name).
    pub fn add<W: Widget>(&mut self, name: &str, w: W) {
        match self.widgets.iter().position(|(n, _, _)| n == name) {
            Some(i) => self.widgets[i] = (name.to_string(), Box::new(w), true),
            None => self.widgets.push((name.to_string(), Box::new(w), true)),
        }
    }

    /// Add a page from a layout spec. Every slot must name a widget added already.
    /// A layout file's spec for this page wins, if it has one that fits.
    pub fn add_page(&mut self, name: &str, spec: &str) -> Result<(), String> {
        self.add_page_overridable(name, spec, true)
    }

    fn add_page_overridable(&mut self, name: &str, spec: &str, margin: bool) -> Result<(), String> {
        if let Some(o) = self.override_for(name).map(str::to_string) {
            if self.add_page_with_margin(name, &o, margin).is_ok() {
                return Ok(());
            }
        }
        self.add_page_with_margin(name, spec, margin)
    }

    fn override_for(&self, key: &str) -> Option<&str> {
        self.overrides
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The layout for a page variant (a key with a dot, such as
    /// `Arp.pattern`): the layout file's spec if it gave one, else `default`.
    /// For widgets that re-add a page with different layouts as their mode
    /// changes: `ui.add_page("Arp", &ui.layout_for("Arp.pattern", SPEC))`.
    pub fn layout_for(&self, key: &str, default: &str) -> String {
        self.override_for(key).unwrap_or(default).to_string()
    }

    /// Like `add_page`, but the page fills the screen to its edges (no outer
    /// margin), for a full-screen picture such as the loading page.
    pub fn add_full_page(&mut self, name: &str, spec: &str) -> Result<(), String> {
        self.add_page_overridable(name, spec, false)
    }

    fn add_page_with_margin(&mut self, name: &str, spec: &str, margin: bool) -> Result<(), String> {
        let node = layout::parse(spec)?;
        let g = self.theme.gap;
        let area = Rect::new(0, 0, self.size.0, self.size.1).inset(if margin { g } else { 0 });
        let mut slots = Vec::new();
        for (slot, r) in layout::solve(&node, area, g) {
            let i = self
                .widgets
                .iter()
                .position(|(n, _, _)| *n == slot)
                .ok_or_else(|| format!("page {name}: no widget called '{slot}'"))?;
            slots.push((i, r));
        }
        let page = Page {
            name: name.to_string(),
            spec: spec.to_string(),
            margin,
            slots,
        };
        match self.pages.iter().position(|p| p.name == name) {
            Some(i) => self.pages[i] = page,
            None => self.pages.push(page),
        }
        self.full = true;
        Ok(())
    }

    /// Rearrange an existing page: a new layout spec over the same widgets.
    /// On an error the page keeps its old layout.
    pub fn set_layout(&mut self, page: &str, spec: &str) -> Result<(), String> {
        let p = self
            .pages
            .iter()
            .find(|p| p.name == page)
            .ok_or_else(|| format!("no page called '{page}'"))?;
        let margin = p.margin;
        self.add_page_with_margin(page, spec, margin)
    }

    /// The layout spec a page was made from.
    pub fn layout_of(&self, page: &str) -> Option<&str> {
        self.pages
            .iter()
            .find(|p| p.name == page)
            .map(|p| p.spec.as_str())
    }

    /// Rearrange pages from a layout file: one `Page name = spec` per line
    /// (see [`layout::parse_file`]). A plain name rearranges that page now
    /// and stays in force when the app adds the page again. A name with a dot
    /// (`Arp.pattern`) is a variant that the app asks for by key
    /// ([`Ui::layout_for`]); its widget names are checked when it is used.
    /// Every page is checked before any changes, so a mistake leaves the
    /// screen as it was.
    pub fn apply_layouts(&mut self, text: &str) -> Result<(), String> {
        let entries = layout::parse_file(text)?;
        let before: Vec<(String, String)> = self
            .pages
            .iter()
            .map(|p| (p.name.clone(), p.spec.clone()))
            .collect();
        for (page, spec) in &entries {
            if page.contains('.') {
                continue;
            }
            if let Err(e) = self.set_layout(page, spec) {
                for (name, spec) in &before {
                    let _ = self.set_layout(name, spec);
                }
                return Err(e);
            }
        }
        for (key, spec) in entries {
            self.overrides.retain(|(k, _)| *k != key);
            self.overrides.push((key, spec));
        }
        Ok(())
    }

    /// Switch theme: colours, effects and font roles change at once, and
    /// pages are laid out again with the new spacing. Redraws everything.
    pub fn set_theme(&mut self, theme: Theme) {
        self.fonts.set_roles(theme.face_body, theme.face_title);
        self.theme = theme;
        let pages: Vec<(String, String)> = self
            .pages
            .iter()
            .map(|p| (p.name.clone(), p.spec.clone()))
            .collect();
        for (name, spec) in pages {
            let _ = self.set_layout(&name, &spec);
        }
        self.full = true;
    }

    pub fn page(&self) -> &str {
        self.pages
            .get(self.current)
            .map(|p| p.name.as_str())
            .unwrap_or("")
    }

    pub fn pages(&self) -> Vec<String> {
        self.pages.iter().map(|p| p.name.clone()).collect()
    }

    /// Show another page. Fingers held on the old page are cancelled.
    pub fn show(&mut self, page: &str) -> Result<(), String> {
        let i = self
            .pages
            .iter()
            .position(|p| p.name == page)
            .ok_or_else(|| format!("no page called '{page}'"))?;
        if i != self.current {
            self.current = i;
            self.full = true;
            self.fingers.clear();
        }
        Ok(())
    }

    /// Where `name` sits on the current page.
    pub fn rect_of(&self, name: &str) -> Option<Rect> {
        let page = self.pages.get(self.current)?;
        page.slots
            .iter()
            .find(|(i, _)| self.widgets[*i].0 == name)
            .map(|(_, r)| *r)
    }

    /// Where `name` sits on page `page`, shown or not (for a widget that
    /// sizes what it shows to its rectangle before its page is up).
    pub fn rect_on(&self, page: &str, name: &str) -> Option<Rect> {
        let page = self.pages.iter().find(|p| p.name == page)?;
        page.slots
            .iter()
            .find(|(i, _)| self.widgets[*i].0 == name)
            .map(|(_, r)| *r)
    }

    pub fn get<W: Widget>(&self, name: &str) -> Option<&W> {
        let (_, w, _) = self.widgets.iter().find(|(n, _, _)| n == name)?;
        let any: &dyn Any = w.as_ref();
        any.downcast_ref::<W>()
    }

    /// Change a widget; it is redrawn at the next `paint`.
    pub fn get_mut<W: Widget>(&mut self, name: &str) -> Option<&mut W> {
        let (_, w, dirty) = self.widgets.iter_mut().find(|(n, _, _)| n == name)?;
        *dirty = true;
        let any: &mut dyn Any = w.as_mut();
        any.downcast_mut::<W>()
    }

    /// Reach a widget without redrawing it: only for state it does not
    /// draw, such as taking the inputs it queued. Anything that changes
    /// what it draws goes through [`Ui::get_mut`]. (Reaching for a widget
    /// every frame through `get_mut` redraws it every frame: the Pads tab's
    /// browser did, at 30 frames a second, until 13 Nov.)
    pub fn get_mut_quiet<W: Widget>(&mut self, name: &str) -> Option<&mut W> {
        let (_, w, _) = self.widgets.iter_mut().find(|(n, _, _)| n == name)?;
        let any: &mut dyn Any = w.as_mut();
        any.downcast_mut::<W>()
    }

    /// Redraw everything at the next `paint`.
    pub fn invalidate(&mut self) {
        self.full = true;
    }

    /// True if the next `paint` would draw something.
    pub fn needs_paint(&self) -> bool {
        if self.full {
            return true;
        }
        let Some(page) = self.pages.get(self.current) else {
            return false;
        };
        page.slots
            .iter()
            .any(|(i, _)| self.widgets[*i].2 || self.widgets[*i].1.animating())
    }

    /// Every control a finger can reach on the current page: each widget's
    /// [`Widget::targets`], less any whose middle another widget covers
    /// (a touch goes to the topmost slot), in the page's slot order.
    pub fn targets(&self) -> Vec<PageTarget> {
        let Some(page) = self.pages.get(self.current) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (k, (wi, r)) in page.slots.iter().enumerate() {
            let (name, w, _) = &self.widgets[*wi];
            for t in w.targets(*r) {
                let (x, y) = t.centre();
                let covered = page.slots[k + 1..].iter().any(|(_, o)| o.contains(x, y));
                if r.contains(x, y) && !covered {
                    out.push(PageTarget {
                        widget: name.clone(),
                        target: t,
                    });
                }
            }
        }
        out
    }

    /// The widgets on the current page and where they sit, in slot order.
    pub fn slots(&self) -> Vec<(String, Rect)> {
        let Some(page) = self.pages.get(self.current) else {
            return Vec::new();
        };
        page.slots
            .iter()
            .map(|(wi, r)| (self.widgets[*wi].0.clone(), *r))
            .collect()
    }

    /// Route a touch. A finger belongs to the widget it first touched until it
    /// lifts, even if it slides off.
    pub fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32) -> Option<UiEvent> {
        let cur = self.current;
        if cur >= self.pages.len() {
            return None;
        }
        let owner = match phase {
            Touch::Down => {
                self.fingers.retain(|(f, _)| *f != finger);
                let hit = self.pages[cur]
                    .slots
                    .iter()
                    .rev()
                    .find(|(_, r)| r.contains(x, y))
                    .copied()?;
                self.fingers.push((finger, hit.0));
                hit
            }
            _ => {
                let i = self.fingers.iter().position(|(f, _)| *f == finger)?;
                let w = self.fingers[i].1;
                if matches!(phase, Touch::Up | Touch::Cancel) {
                    self.fingers.remove(i);
                }
                *self.pages[cur].slots.iter().find(|(wi, _)| *wi == w)?
            }
        };
        let (wi, r) = owner;
        let (name, w, dirty) = &mut self.widgets[wi];
        let kind = w.touch(finger, phase, x, y, r);
        *dirty = true;
        kind.map(|kind| UiEvent {
            widget: name.clone(),
            kind,
        })
    }

    /// Draw what changed on the current page; returns the rectangles drawn,
    /// for the screen to copy.
    pub fn paint(&mut self, canvas: &mut dyn Canvas) -> Vec<Rect> {
        let Some(page) = self.pages.get(self.current) else {
            return Vec::new();
        };
        let mut damage = Vec::new();
        let screen = Rect::new(0, 0, self.size.0, self.size.1);
        let mut styled = Styled::new(canvas, &self.theme);
        let canvas: &mut dyn Canvas = &mut styled;
        let theme = &self.theme;
        // Whether a fill in the background hides what was under it (the
        // theme's backdrop is opaque); a theme file can make it see-through.
        let bg_covers =
            theme.bg.a == 255 && (theme.backdrop == Backdrop::Solid || theme.backdrop_to.a == 255);
        if self.full {
            canvas.set_clip(screen);
            if bg_covers {
                // Only between the widgets: each widget clears its own
                // rectangle below, and only where its drawing leaves it.
                let slots: Vec<Rect> = page.slots.iter().map(|&(_, r)| r).collect();
                for gap in screen.uncovered(&slots) {
                    canvas.fill_rect(gap, theme.bg);
                }
            } else {
                canvas.fill_rect(screen, theme.bg);
            }
            damage.push(screen);
        }
        let (fonts, widgets, full) = (&mut self.fonts, &mut self.widgets, &mut self.full);
        for &(wi, r) in &page.slots {
            let (_, w, dirty) = &mut widgets[wi];
            if !(*full || *dirty || w.animating()) {
                continue;
            }
            let parts = if *full {
                vec![r]
            } else {
                w.changed_parts(r).unwrap_or_else(|| vec![r])
            };
            for part in parts {
                let part = part.intersect(r);
                if part.is_empty() {
                    continue;
                }
                canvas.set_clip(part);
                let mut cleared = Cleared {
                    inner: &mut *canvas,
                    pending: Some(part),
                    bg: theme.bg,
                    bg_covers,
                };
                w.paint(
                    &mut PaintCx {
                        canvas: &mut cleared,
                        theme,
                        fonts,
                    },
                    r,
                );
                cleared.settle(None);
                if !*full {
                    damage.push(part);
                }
            }
        }
        canvas.set_clip(screen);
        for (_, _, d) in widgets.iter_mut() {
            *d = false;
        }
        *full = false;
        damage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::SoftCanvas;

    struct Probe {
        paints: std::cell::Cell<u32>,
        touches: Vec<(u64, Touch)>,
    }

    impl Widget for Probe {
        fn paint(&self, cx: &mut PaintCx, r: Rect) {
            self.paints.set(self.paints.get() + 1);
            cx.canvas.fill_rect(r, Color::rgb(0xFFFFFF));
        }

        fn touch(
            &mut self,
            finger: u64,
            phase: Touch,
            _x: i32,
            _y: i32,
            _r: Rect,
        ) -> Option<EventKind> {
            self.touches.push((finger, phase));
            (phase == Touch::Down).then_some(EventKind::Tab(0))
        }
    }

    fn probe() -> Probe {
        Probe {
            paints: std::cell::Cell::new(0),
            touches: Vec::new(),
        }
    }

    fn ui() -> Ui {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 200, 100);
        ui.add("a", probe());
        ui.add("b", probe());
        ui.add_page("one", "row(a, b)").unwrap();
        ui.add_page("two", "col(b)").unwrap();
        ui
    }

    #[test]
    fn layout_files_survive_pages_being_added_again() {
        let mut ui = ui();
        ui.apply_layouts("one = row(b, a)\none.wide = row(a:150, b)\n")
            .unwrap();
        assert_eq!(ui.layout_of("one"), Some("row(b, a)"));
        // The app re-adds the page with its own spec: the file still wins.
        ui.add_page("one", "row(a, b)").unwrap();
        assert_eq!(ui.layout_of("one"), Some("row(b, a)"));
        // Variants are asked for by key.
        let spec = ui.layout_for("one.wide", "row(a, b)");
        ui.add_page("one", &spec).unwrap();
        assert_eq!(
            ui.layout_of("one"),
            Some("row(b, a)"),
            "a page's own override beats a variant"
        );
        assert_eq!(ui.layout_for("one.wide", "x"), "row(a:150, b)");
        assert_eq!(ui.layout_for("one.narrow", "row(a, b)"), "row(a, b)");
        // A bad file changes nothing.
        assert!(ui.apply_layouts("two = col(nope)\n").is_err());
        assert_eq!(ui.layout_of("two"), Some("col(b)"));
    }

    #[test]
    fn pages_need_known_widgets() {
        let mut ui = ui();
        assert!(ui.add_page("bad", "row(a, c)").unwrap_err().contains("'c'"));
        assert!(ui.show("nope").is_err());
        assert_eq!(ui.pages(), vec!["one", "two"]);
    }

    #[test]
    fn only_changed_widgets_are_redrawn() {
        let mut ui = ui();
        let mut c = SoftCanvas::new(200, 100);
        assert_eq!(ui.paint(&mut c), vec![Rect::new(0, 0, 200, 100)]);
        assert!(!ui.needs_paint());
        assert!(ui.paint(&mut c).is_empty());
        ui.get_mut::<Probe>("b").unwrap();
        let d = ui.paint(&mut c);
        assert_eq!(d, vec![ui.rect_of("b").unwrap()]);
        assert_eq!(ui.get::<Probe>("a").unwrap().paints.get(), 1);
        assert_eq!(ui.get::<Probe>("b").unwrap().paints.get(), 2);
        ui.show("two").unwrap();
        assert_eq!(ui.paint(&mut c).len(), 1);
        assert_eq!(
            ui.get::<Probe>("a").unwrap().paints.get(),
            1,
            "a is not on page two"
        );
    }

    /// One drawing call, for `clearing_less_draws_the_same_picture`.
    #[derive(Clone, Copy, Debug)]
    enum Draw {
        Clip(Rect),
        Fill(Rect, Color),
        Round(Rect, i32, Color),
        Stroke(Rect, i32, i32, Color),
        Text(i32, i32, Color),
        /// A picture of noise in the rectangle.
        Picture(Rect, u32),
    }

    fn draw(c: &mut dyn Canvas, ops: &[Draw]) {
        let glyph: Vec<u8> = (0..48).map(|i| (i * 37 % 256) as u8).collect();
        for op in ops {
            match *op {
                Draw::Clip(r) => c.set_clip(r),
                Draw::Fill(r, col) => c.fill_rect(r, col),
                Draw::Round(r, radius, col) => c.fill_round_rect(r, radius, col),
                Draw::Stroke(r, radius, w, col) => c.stroke_round_rect(r, radius, w, col),
                Draw::Text(x, y, col) => c.blend_mask(x, y, 8, 6, &glyph, col),
                Draw::Picture(r, seed) => {
                    if r.is_empty() {
                        continue;
                    }
                    let mut img = crate::canvas::Image::new(r.w, r.h);
                    for (i, p) in img.px.iter_mut().enumerate() {
                        *p = (i as u32).wrapping_mul(2_654_435_761) ^ seed;
                    }
                    c.draw_image(r.x, r.y, &img);
                }
            }
        }
    }

    #[test]
    fn clearing_less_draws_the_same_picture() {
        // Random widgets drawing a random part over a screen full of noise,
        // in every theme: clearing the whole part first, as before, and
        // clearing only what the first fill leaves give the same pixels.
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |n: i32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n.max(1) as u64) as i32
        };
        let (mut corners_only, mut none) = (0, 0);
        let mut themes: Vec<(&str, Theme)> = crate::theme::BUILTIN
            .iter()
            .map(|&n| (n, Theme::builtin(n).unwrap()))
            .collect();
        // A theme file may give the backdrop or the background some
        // transparency: a background fill then no longer hides what was
        // under it.
        let mut see_through = Theme::console();
        see_through.backdrop_to = see_through.backdrop_to.with_alpha(120);
        themes.push(("see-through backdrop", see_through));
        let mut faint = Theme::dark();
        faint.bg = faint.bg.with_alpha(200);
        themes.push(("see-through background", faint));
        for (name, theme) in themes {
            for case in 0..400 {
                let r = Rect::new(next(30), next(20), 20 + next(70), 15 + next(55));
                let part = if next(2) == 0 {
                    r
                } else {
                    let (x, y) = (r.x + next(r.w), r.y + next(r.h));
                    Rect::new(x, y, 1 + next(r.right() - x), 1 + next(r.bottom() - y))
                };
                let colour = |next: &mut dyn FnMut(i32) -> i32| match next(5) {
                    0 => theme.bg,
                    1 => theme.surface,
                    2 => Color::rgb(next(0xFFFFFF) as u32).with_alpha(next(255) as u8),
                    _ => Color::rgb(next(0xFFFFFF) as u32),
                };
                let grown = |next: &mut dyn FnMut(i32) -> i32| match next(3) {
                    0 => r,
                    1 => Rect::new(r.x - next(4), r.y - next(4), r.w + next(9), r.h + next(9)),
                    _ => Rect::new(r.x + next(4), r.y + next(4), r.w - next(9), r.h - next(9)),
                };
                let mut ops = Vec::new();
                match next(7) {
                    0 => ops.push(Draw::Fill(grown(&mut next), colour(&mut next))),
                    3 => ops.push(Draw::Picture(grown(&mut next), next(1 << 30) as u32)),
                    1 => ops.push(Draw::Clip(grown(&mut next))),
                    2 => ops.push(Draw::Text(
                        r.x + next(r.w),
                        r.y + next(r.h),
                        colour(&mut next),
                    )),
                    _ => {}
                }
                ops.push(Draw::Round(
                    grown(&mut next),
                    next(30) - 2,
                    colour(&mut next),
                ));
                for _ in 0..next(4) {
                    ops.push(match next(4) {
                        0 => Draw::Fill(grown(&mut next), colour(&mut next)),
                        1 => {
                            Draw::Stroke(grown(&mut next), next(20), 1 + next(4), colour(&mut next))
                        }
                        2 => Draw::Text(r.x + next(r.w), r.y + next(r.h), colour(&mut next)),
                        _ => Draw::Round(grown(&mut next), next(30), colour(&mut next)),
                    });
                }
                let mut noise = SoftCanvas::new(120, 90);
                for p in noise.px.iter_mut() {
                    *p = next(0xFFFFFF) as u32;
                }
                let mut before = SoftCanvas::new(120, 90);
                before.px.clone_from(&noise.px);
                {
                    let mut c = Styled::new(&mut before, &theme);
                    c.set_clip(part);
                    c.fill_rect(part, theme.bg);
                    draw(&mut c, &ops);
                }
                let mut now = noise;
                {
                    let mut styled = Styled::new(&mut now, &theme);
                    styled.set_clip(part);
                    let mut c = Cleared {
                        inner: &mut styled,
                        pending: Some(part),
                        bg: theme.bg,
                        bg_covers: theme.bg.a == 255
                            && (theme.backdrop == Backdrop::Solid || theme.backdrop_to.a == 255),
                    };
                    draw(&mut c, &ops);
                    c.settle(None);
                }
                assert!(
                    before.px == now.px,
                    "theme {name}, case {case}: part {part:?}, {ops:?}"
                );
                // How often the clear shrank, so the test is known to try it.
                let first_covers = match ops[0] {
                    Draw::Fill(f, col) => col.a == 255 && part.intersect(f) == part,
                    Draw::Round(f, _, col) => col.a == 255 && part.intersect(f) == part,
                    _ => false,
                };
                if first_covers {
                    match ops[0] {
                        Draw::Fill(..) => none += 1,
                        _ => corners_only += 1,
                    }
                }
            }
        }
        assert!(corners_only > 200 && none > 50, "{corners_only} {none}");
    }

    struct Rng(u64);

    impl Rng {
        fn n(&mut self, n: i32) -> i32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n.max(1) as u64) as i32
        }

        fn colour(&mut self, t: &Theme) -> Color {
            match self.n(5) {
                0 => t.bg,
                1 => t.surface,
                2 => Color::rgb(self.n(0xFFFFFF) as u32).with_alpha(self.n(255) as u8),
                _ => Color::rgb(self.n(0xFFFFFF) as u32),
            }
        }

        /// `r`, or a little bigger or smaller.
        fn near(&mut self, r: Rect) -> Rect {
            match self.n(3) {
                0 => r,
                1 => Rect::new(
                    r.x - self.n(4),
                    r.y - self.n(4),
                    r.w + self.n(9),
                    r.h + self.n(9),
                ),
                _ => Rect::new(
                    r.x + self.n(4),
                    r.y + self.n(4),
                    r.w - self.n(9),
                    r.h - self.n(9),
                ),
            }
        }

        /// A rectangle inside `r`.
        fn inside(&mut self, r: Rect) -> Rect {
            let (x, y) = (r.x + self.n(r.w), r.y + self.n(r.h));
            Rect::new(x, y, 1 + self.n(r.right() - x), 1 + self.n(r.bottom() - y))
        }
    }

    /// A widget that makes the calls it is given, and says which parts of
    /// it changed.
    struct Scripted {
        ops: Vec<Draw>,
        parts: Option<Vec<Rect>>,
    }

    impl Widget for Scripted {
        fn paint(&self, cx: &mut PaintCx, _r: Rect) {
            draw(cx.canvas, &self.ops);
        }

        fn changed_parts(&self, _r: Rect) -> Option<Vec<Rect>> {
            self.parts.clone()
        }
    }

    /// The page painted the way `Ui::paint` did before 14 Nov: the whole
    /// screen cleared first on a full paint, and each part before its
    /// widget paints it.
    fn paint_as_before(ui: &mut Ui, canvas: &mut dyn Canvas, full: bool) {
        let Ui {
            pages,
            widgets,
            theme,
            fonts,
            current,
            size,
            ..
        } = ui;
        let screen = Rect::new(0, 0, size.0, size.1);
        let mut c = Styled::new(canvas, theme);
        if full {
            c.set_clip(screen);
            c.fill_rect(screen, theme.bg);
        }
        for &(wi, r) in &pages[*current].slots {
            let w = &widgets[wi].1;
            let parts = match full {
                true => vec![r],
                false => w.changed_parts(r).unwrap_or_else(|| vec![r]),
            };
            for part in parts {
                let part = part.intersect(r);
                if part.is_empty() {
                    continue;
                }
                c.set_clip(part);
                c.fill_rect(part, theme.bg);
                let mut cx = PaintCx {
                    canvas: &mut c,
                    theme,
                    fonts,
                };
                w.paint(&mut cx, r);
            }
        }
        c.set_clip(screen);
    }

    #[test]
    fn pages_paint_the_same_picture_as_before() {
        // Random pages of random widgets, over a screen full of noise, in
        // every theme and two see-through ones: painting the page whole,
        // then the parts that changed, gives what clearing everything first
        // gave.
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let mut themes: Vec<Theme> = crate::theme::BUILTIN
            .iter()
            .map(|&n| Theme::builtin(n).unwrap())
            .collect();
        let mut see_through = Theme::console();
        see_through.backdrop_to = see_through.backdrop_to.with_alpha(120);
        themes.push(see_through);
        let mut faint = Theme::dark();
        faint.bg = faint.bg.with_alpha(200);
        themes.push(faint);
        for theme in themes {
            for case in 0..60 {
                let mut ui = Ui::new(theme.clone(), Fonts::embedded(), 160, 100);
                for k in 0..4 {
                    ui.add(&format!("w{k}"), probe());
                }
                if case % 2 == 0 {
                    ui.add_page("p", "col(w0:20, row(w1, col(w2, w3):*):*)")
                        .unwrap();
                } else {
                    // Slots anywhere, overlapping or not.
                    let slots = (0..4)
                        .map(|k| {
                            (
                                k,
                                Rect::new(rng.n(120), rng.n(70), 10 + rng.n(60), 10 + rng.n(40)),
                            )
                        })
                        .collect();
                    ui.pages.push(Page {
                        name: "p".into(),
                        spec: String::new(),
                        margin: true,
                        slots,
                    });
                    ui.show("p").unwrap();
                }
                let slots = ui.pages[ui.current].slots.clone();
                for (k, &(_, r)) in slots.iter().enumerate() {
                    let mut ops = Vec::new();
                    match rng.n(6) {
                        0 => ops.push(Draw::Fill(rng.near(r), rng.colour(&theme))),
                        2 => ops.push(Draw::Picture(rng.near(r), rng.n(1 << 30) as u32)),
                        1 => ops.push(Draw::Text(
                            r.x + rng.n(r.w),
                            r.y + rng.n(r.h),
                            rng.colour(&theme),
                        )),
                        _ => {}
                    }
                    ops.push(Draw::Round(rng.near(r), rng.n(20) - 2, rng.colour(&theme)));
                    for _ in 0..rng.n(4) {
                        ops.push(match rng.n(3) {
                            0 => Draw::Stroke(
                                rng.near(r),
                                rng.n(12),
                                1 + rng.n(3),
                                rng.colour(&theme),
                            ),
                            1 => Draw::Text(r.x + rng.n(r.w), r.y + rng.n(r.h), rng.colour(&theme)),
                            _ => Draw::Round(rng.inside(r), rng.n(8), rng.colour(&theme)),
                        });
                    }
                    let parts = match rng.n(3) {
                        0 => None,
                        _ => Some((0..1 + rng.n(2)).map(|_| rng.inside(r)).collect()),
                    };
                    ui.add(&format!("w{k}"), Scripted { ops, parts });
                }
                let mut before = SoftCanvas::new(160, 100);
                for p in before.px.iter_mut() {
                    *p = rng.n(0xFFFFFF) as u32;
                }
                let mut now = SoftCanvas::new(160, 100);
                now.px.clone_from(&before.px);
                ui.paint(&mut now);
                paint_as_before(&mut ui, &mut before, true);
                assert!(before.px == now.px, "whole page, case {case}");
                for w in ui.widgets.iter_mut() {
                    w.2 = true;
                }
                ui.paint(&mut now);
                paint_as_before(&mut ui, &mut before, false);
                assert!(before.px == now.px, "changed parts, case {case}");
            }
        }
    }

    #[test]
    fn full_pages_reach_the_edges() {
        let mut ui = ui();
        ui.add_full_page("full", "a").unwrap();
        ui.show("full").unwrap();
        assert_eq!(ui.rect_of("a"), Some(Rect::new(0, 0, 200, 100)));
        ui.show("two").unwrap();
        assert_eq!(ui.rect_of("b"), Some(Rect::new(12, 12, 176, 76)));
    }

    #[test]
    fn a_finger_stays_with_the_widget_it_first_touched() {
        let mut ui = ui();
        let a = ui.rect_of("a").unwrap();
        let b = ui.rect_of("b").unwrap();
        assert!(ui.touch(7, Touch::Down, a.x + 1, a.y + 1).is_some());
        assert_eq!(ui.touch(7, Touch::Move, b.x + 1, b.y + 1), None);
        ui.touch(7, Touch::Up, b.x + 1, b.y + 1);
        assert_eq!(
            ui.get::<Probe>("a").unwrap().touches,
            vec![(7, Touch::Down), (7, Touch::Move), (7, Touch::Up)]
        );
        assert!(ui.get::<Probe>("b").unwrap().touches.is_empty());
        // After lifting, a move goes nowhere.
        assert_eq!(ui.touch(7, Touch::Move, a.x + 1, a.y + 1), None);
        // Outside every widget: nothing.
        assert_eq!(ui.touch(8, Touch::Down, 0, 0), None);
    }
}
