//! A skin on screen: lays a tab's component tree out, draws it, and turns
//! touches, the data wheel and the Q-Link knobs into parameter edits, as the
//! MPC's Program Edit screen does with the same files.
//!
//! Coordinates are the tab's own: 1280 x 628, `(0, 0)` its top left (110 px
//! down the MPC's screen). Overlays (the knob and menu editors a double tap
//! opens) are 1280 x 800, the whole screen.

use crate::handle::{Env, Handle, Params, Target, Val};
use crate::model::{increment, Handler};
use crate::model::{
    Action, Component, Drag, Entry, Fill, HwFocus, KnobKind, KnobOverlay, LabelKind, OnAction,
    RectF, Remap, Skin, SliderKind, WhenInvalid, WhenVisible, Widget,
};
use crate::paint::{self, color, fill, outline, px, text, Fonts, Images};
use crate::qlinks::{pick, variants, QLinks, TabKnobs};
use mpfree_ui::shape::Shape;
use mpfree_ui::{Canvas, Color, Rect, SoftCanvas};
use std::collections::HashMap;
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const TAB_W: f32 = 1280.0;
pub const TAB_H: f32 = 628.0;
pub const SCREEN_H: f32 = 800.0;
/// A tab's area starts this far down the MPC's screen.
pub const TAB_TOP: f32 = 110.0;
/// Pixels of drag for a knob's whole range.
pub const DRAG_RANGE: f32 = 250.0;
/// Two taps closer than this (in ms) are a double tap.
pub const DOUBLE_TAP_MS: u64 = 350;

/// The unit a skin is drawn for: its variant files are picked by these.
#[derive(Clone, Debug)]
pub struct Unit {
    pub product: &'static str,
    pub family: &'static str,
    pub layout: &'static str,
}

/// The MPC One: four Q-Link knobs in a column.
pub const MPC_ONE: Unit = Unit {
    product: "MPC One",
    family: "MPC",
    layout: "1by4",
};

/// A plugin folder's skin, read for `unit`. `Ok(None)`: the folder has no
/// skin (no `Plugin Skins/` or no `TUI.json`), and the MPC would show its
/// default pages. `Err`: a `TUI.json` that doesn't parse (the MPC then shows
/// no tabs at all).
pub fn load(plugin: &Path, unit: &Unit, params: usize) -> Result<Option<(Skin, QLinks)>, String> {
    let dir = plugin.join("Plugin Skins");
    if !dir.is_dir() {
        return Ok(None);
    }
    let Some(tui) = pick(
        &dir,
        &variants("TUI", unit.product, unit.family, unit.layout),
    ) else {
        return Ok(None);
    };
    let mut skin = Skin::read(&tui)?;
    skin.expand_partial(params);
    let qlinks = pick(
        &dir,
        &variants("Q-Links", unit.product, unit.family, unit.layout),
    )
    .and_then(|p| QLinks::read(&p).ok())
    .unwrap_or_else(|| QLinks::sequential(params));
    Ok(Some((skin, qlinks)))
}

/// What the app does with a touch, the wheel or a knob.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// A gesture on the parameter starts (for undo grouping and automation).
    Begin(usize),
    Set(usize, f32),
    End(usize),
}

/// A touch on the skin, in tab coordinates (or screen coordinates while an
/// overlay is open), with the time in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Touch {
    Down { x: f32, y: f32, ms: u64 },
    Move { x: f32, y: f32 },
    Up { x: f32, y: f32 },
}

#[derive(Clone, Debug)]
struct ResolvedAction {
    on: OnAction,
    handler: Handler,
    handle: String,
    data: String,
}

#[derive(Clone)]
enum What<'s> {
    Back(&'s Fill, &'s Path),
    Comp {
        actions: Vec<ResolvedAction>,
        data: String,
        fine: bool,
        ignore_mouse: bool,
        hide_qlink_bounds: bool,
    },
    Widget {
        w: &'s Widget,
        handle: String,
        y_handle: String,
        dir: &'s Path,
    },
}

#[derive(Clone)]
struct Item<'s> {
    rect: RectF,
    clip: Rect,
    what: What<'s>,
    path: u64,
    focusable: bool,
    /// The enclosing component's item (its index in the list).
    parent: Option<usize>,
}

fn mix(path: u64, a: usize, b: usize) -> u64 {
    let mut h = path ^ 0x9E37_79B9_7F4A_7C15;
    for v in [a as u64, b as u64] {
        h ^= v.wrapping_add(0x9E37_79B9_7F4A_7C15);
        h = h.wrapping_mul(0x0100_0000_01B3);
        h ^= h >> 29;
    }
    h
}

/// Remaps from the innermost scope out, each with the repeat copy its
/// `{Increment}` placeholders take.
type Chain<'s> = Vec<(&'s Remap, usize)>;

fn resolve(chain: &Chain, h: &str) -> String {
    let mut s = h.to_string();
    for (remap, r) in chain.iter().rev() {
        s = increment(&remap.apply(&s), *r);
    }
    s
}

struct Layout<'s, 'e> {
    skin: &'s Skin,
    env: &'e Env<'e>,
    focus: Option<u64>,
    items: Vec<Item<'s>>,
    builtin: &'s HashMap<String, Arc<Component>>,
    /// The enclosing components' items, innermost last.
    stack: Vec<usize>,
}

impl<'s> Layout<'s, '_> {
    fn component(&self, name: &str) -> Option<&'s Arc<Component>> {
        self.skin
            .components
            .get(name)
            .or_else(|| self.builtin.get(name))
    }

    fn valid(&self, h: &str) -> Val {
        self.env.eval(&Handle::parse(h))
    }

    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        comp: &'s Component,
        rect: RectF,
        clip: Rect,
        scale: f32,
        chain: &mut Chain<'s>,
        path: u64,
        focusable: bool,
        depth: usize,
    ) {
        if depth > 40 {
            return; // a component that holds itself
        }
        let focused = self.focus == Some(path);
        let clip = clip.intersect(px(rect));
        let parent = self.items.len();
        self.items.push(Item {
            rect,
            clip,
            what: What::Comp {
                actions: comp
                    .actions
                    .iter()
                    .map(|a: &Action| {
                        let mut ch = chain.clone();
                        if !a.remap.is_empty() {
                            ch.push((&a.remap, 0));
                        }
                        ResolvedAction {
                            on: a.on,
                            handler: a.handler.clone(),
                            handle: resolve(&ch, &a.handle),
                            data: a.data.clone(),
                        }
                    })
                    .collect(),
                data: resolve(chain, "Data"),
                fine: comp.fine_wheel,
                ignore_mouse: comp.ignore_mouse,
                hide_qlink_bounds: comp.hide_qlink_bounds,
            },
            path,
            focusable,
            parent: self.stack.last().copied(),
        });
        let back = if focused {
            &comp.focussed
        } else {
            &comp.unfocussed
        };
        if back.colour >> 24 != 0 || !back.image.is_empty() {
            self.items.push(Item {
                rect,
                clip,
                what: What::Back(back, &comp.dir),
                path,
                focusable: false,
                parent: Some(parent),
            });
        }
        self.stack.push(parent);
        for r in 0..comp.repeats {
            for (ci, e) in comp.children.iter().enumerate() {
                self.entry(
                    e, comp, rect, clip, scale, chain, path, focused, r, ci, depth,
                );
            }
        }
        self.stack.pop();
    }

    #[allow(clippy::too_many_arguments)]
    fn entry(
        &mut self,
        e: &'s Entry,
        comp: &'s Component,
        rect: RectF,
        clip: Rect,
        scale: f32,
        chain: &mut Chain<'s>,
        path: u64,
        focused: bool,
        r: usize,
        ci: usize,
        depth: usize,
    ) {
        match e.visible {
            WhenVisible::WhenFocussed if !focused => return,
            WhenVisible::WhenNotFocussed if focused => return,
            _ => {}
        }
        let b = if e.proportional {
            RectF::new(
                e.bounds.x * rect.w,
                e.bounds.y * rect.h,
                e.bounds.w * rect.w,
                e.bounds.h * rect.h,
            )
        } else {
            RectF::new(
                e.bounds.x * scale,
                e.bounds.y * scale,
                e.bounds.w * scale,
                e.bounds.h * scale,
            )
        };
        let er = RectF::new(rect.x + b.x + r as f32 * b.w, rect.y + b.y, b.w, b.h);
        chain.push((&e.remap, r));
        let hidden = self.hidden(e, chain);
        if hidden {
            chain.pop();
            return;
        }
        let child_path = mix(path, ci, r);
        let focusable = e.focus == HwFocus::Yes;
        match &e.widget {
            Widget::Custom(name) => {
                if let Some(c) = self.component(name) {
                    self.walk(c, er, clip, scale, chain, child_path, focusable, depth + 1);
                }
            }
            w => {
                let (h, yh) = match w {
                    Widget::XYPad { x, y, .. } => (x.as_str(), y.as_str()),
                    _ => (widget_handle(w), ""),
                };
                let item = Item {
                    rect: er,
                    clip: clip.intersect(px(er)),
                    what: What::Widget {
                        w,
                        handle: if h.is_empty() {
                            String::new()
                        } else {
                            resolve(chain, h)
                        },
                        y_handle: if yh.is_empty() {
                            String::new()
                        } else {
                            resolve(chain, yh)
                        },
                        dir: &comp.dir,
                    },
                    path: child_path,
                    focusable,
                    parent: self.stack.last().copied(),
                };
                self.items.push(item);
            }
        }
        chain.pop();
    }

    /// The entry's visibility rules (`chain` already holds its remap).
    fn hidden(&self, e: &Entry, chain: &Chain) -> bool {
        for h in &e.extra_handles {
            if !self.valid(&resolve(chain, h)).valid {
                return true;
            }
        }
        if e.invalid == WhenInvalid::Show {
            return false;
        }
        let h = match &e.widget {
            Widget::Custom(_) => {
                let d = resolve(chain, "Data");
                if d == "Data" {
                    return false; // never bound: nothing to be invalid
                }
                d
            }
            w => {
                let h = widget_handle(w);
                if h.is_empty() {
                    return false;
                }
                resolve(chain, h)
            }
        };
        let v = self.valid(&h);
        !v.valid || (e.invalid == WhenInvalid::HideOnInvalidName && v.name.is_empty())
    }
}

/// The handle a widget shows (empty for one that shows none).
fn widget_handle(w: &Widget) -> &str {
    match w {
        Widget::Slider { handle, .. }
        | Widget::Knob { handle, .. }
        | Widget::Button { handle, .. }
        | Widget::ComboBox { handle, .. }
        | Widget::MenuOverlay { handle, .. }
        | Widget::NumericOverlay { handle }
        | Widget::Indicator { handle, .. }
        | Widget::HaloDisplay { handle, .. }
        | Widget::Meter { handle, .. } => handle,
        Widget::KnobOverlay(k) => &k.handle,
        Widget::Label { handle, kind, .. } => {
            if *kind == LabelKind::Text {
                ""
            } else {
                handle
            }
        }
        Widget::Image { handle, .. } => handle,
        Widget::XYPad { x, .. } => x,
        _ => "",
    }
}

fn is_interactive(w: &Widget) -> bool {
    matches!(
        w,
        Widget::Knob { .. }
            | Widget::Slider { .. }
            | Widget::Button { .. }
            | Widget::XYPad { .. }
            | Widget::ComboBox { .. }
            | Widget::KnobOverlay(_)
            | Widget::MenuOverlay { .. }
            | Widget::NumericOverlay { .. }
    )
}

/// The overlay components MPFree draws itself for the names skins import
/// from Akai's own files (never read or copied).
fn builtin_overlays() -> HashMap<String, Arc<Component>> {
    let mk = |name: &str, w: Widget| {
        Arc::new(Component {
            name: name.to_string(),
            actions: Vec::new(),
            focussed: Fill::default(),
            unfocussed: Fill::default(),
            ignore_mouse: true,
            fine_wheel: false,
            repeats: 1,
            hide_qlink_bounds: false,
            children: vec![Entry {
                name: "overlay".into(),
                widget: w,
                remap: Remap::default(),
                focus: HwFocus::Yes,
                invalid: WhenInvalid::Show,
                visible: WhenVisible::Always,
                bounds: RectF::new(0.0, 0.0, TAB_W, SCREEN_H),
                proportional: false,
                extra_handles: Vec::new(),
            }],
            dir: PathBuf::new(),
        })
    };
    let knob = |bipolar| {
        Widget::KnobOverlay(KnobOverlay {
            style: "Knob With Header".into(),
            bipolar,
            handle: "Data".into(),
        })
    };
    let mut m = HashMap::new();
    m.insert("knob overlay".to_string(), mk("knob overlay", knob(false)));
    m.insert(
        "knob overlay bipolar".to_string(),
        mk("knob overlay bipolar", knob(true)),
    );
    m.insert(
        "menu overlay".to_string(),
        mk(
            "menu overlay",
            Widget::MenuOverlay {
                numbers: false,
                handle: "Data".into(),
            },
        ),
    );
    m
}

#[derive(Clone, Debug)]
struct Gesture {
    path: u64,
    kind: GestureKind,
    handle: String,
    y_handle: String,
    rect: RectF,
    start: (f32, f32),
    start_value: f32,
    start_y_value: f32,
    params: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum GestureKind {
    Knob(Drag),
    SliderV,
    SliderH,
    XY,
    Button,
    /// The knob overlay: vertical drags anywhere.
    Overlay,
}

/// What a layout borrows, cloned out of the view so the items it gives
/// don't hold the view itself.
struct Frame {
    skin: Arc<Skin>,
    builtin: Arc<HashMap<String, Arc<Component>>>,
    remap: Remap,
}

#[derive(Clone, Debug, PartialEq)]
struct Overlay {
    component: String,
    handle: String,
}

/// A skin on screen.
pub struct View {
    pub skin: Arc<Skin>,
    pub qlinks: QLinks,
    tab: usize,
    locals: HashMap<String, u32>,
    focus: Option<u64>,
    bank: usize,
    overlay: Option<Overlay>,
    /// Binds the open overlay's `Data` to the handle it edits.
    overlay_remap: Remap,
    gesture: Option<Gesture>,
    last_down: Option<(u64, u64)>,
    builtin: Arc<HashMap<String, Arc<Component>>>,
    pub fonts: Fonts,
    pub images: Images,
    tick: u64,
    /// The menu overlay's first row shown.
    menu_scroll: usize,
}

impl View {
    pub fn new(skin: Skin, qlinks: QLinks) -> View {
        let root = skin.dir.parent().unwrap_or(&skin.dir).to_path_buf();
        View {
            skin: Arc::new(skin),
            qlinks,
            tab: 0,
            locals: HashMap::new(),
            focus: None,
            bank: 0,
            overlay: None,
            overlay_remap: Remap::default(),
            gesture: None,
            last_down: None,
            builtin: Arc::new(builtin_overlays()),
            fonts: Fonts::default(),
            images: Images::new(&root),
            tick: 0,
            menu_scroll: 0,
        }
    }

    /// The tabs: name, function key (0-based) and sub-tab (0-based).
    pub fn tabs(&self) -> Vec<(String, usize, usize)> {
        self.skin
            .tabs
            .iter()
            .map(|t| (t.name.clone(), t.key, t.sub))
            .collect()
    }

    pub fn tab(&self) -> usize {
        self.tab
    }

    pub fn set_tab(&mut self, i: usize) {
        if i < self.skin.tabs.len() && i != self.tab {
            self.tab = i;
            self.focus = None;
            self.bank = 0;
            self.overlay = None;
            self.gesture = None;
        }
    }

    /// Steps to the next sub-tab on function key `key` (the MPC's second
    /// press on the same key), or to the key's first tab.
    pub fn press_key(&mut self, key: usize) {
        let on_key: Vec<usize> = (0..self.skin.tabs.len())
            .filter(|&i| self.skin.tabs[i].key == key)
            .collect();
        if on_key.is_empty() {
            return;
        }
        let next = match on_key.iter().position(|&i| i == self.tab) {
            Some(p) => on_key[(p + 1) % on_key.len()],
            None => on_key[0],
        };
        self.set_tab(next);
    }

    pub fn overlay_open(&self) -> bool {
        self.overlay.is_some()
    }

    pub fn close_overlay(&mut self) {
        self.overlay = None;
        self.gesture = None;
    }

    /// True while something animates (an animated film strip): redraw often.
    pub fn animated(&self) -> bool {
        self.skin.components.values().any(|c| {
            c.children
                .iter()
                .any(|e| matches!(e.widget, Widget::Image { animated: true, .. }))
        })
    }

    fn knobs_for_tab(&self) -> Option<TabKnobs> {
        let t = self.skin.tabs.get(self.tab)?;
        self.qlinks.for_tab(t.key, t.sub)
    }

    /// The parameters the four Q-Link knobs (top first) turn now.
    pub fn knobs(&self) -> [Option<usize>; 4] {
        self.knobs_for_tab()
            .map(|k| k.bank_params(self.bank))
            .unwrap_or([None; 4])
    }

    pub fn bank(&self) -> usize {
        self.bank
    }

    pub fn set_bank(&mut self, b: usize) {
        self.bank = b.min(3);
    }

    /// The parameters the knobs turn in Program (Track) Q-Link mode.
    pub fn program_knobs(&self) -> [Option<usize>; 16] {
        self.qlinks.program
    }

    fn page_lookup(&self) -> impl Fn(isize, usize, usize) -> Option<usize> + '_ {
        move |rel: isize, sub: usize, q: usize| {
            let t = self.skin.tabs.get(self.tab)?;
            let key = (t.key as isize + rel).max(0) as usize;
            let k = self.qlinks.for_tab(key, sub.saturating_sub(1))?;
            *k.params.get(q.checked_sub(1)?)?
        }
    }

    fn with_env<R>(&self, params: &dyn Params, f: impl FnOnce(&Env) -> R) -> R {
        let page = self.page_lookup();
        let env = Env {
            params,
            replacements: &self.skin.replacements,
            locals: &self.locals,
            page: &page,
            param_offset: self.skin.tabs.get(self.tab).map_or(0, |t| t.param_offset),
        };
        f(&env)
    }

    fn frame(&self) -> Frame {
        Frame {
            skin: Arc::clone(&self.skin),
            builtin: Arc::clone(&self.builtin),
            remap: self.overlay_remap.clone(),
        }
    }

    fn layout<'s>(&self, f: &'s Frame, env: &Env, overlay: bool) -> Vec<Item<'s>> {
        let skin: &'s Skin = &f.skin;
        let builtin: &'s HashMap<String, Arc<Component>> = &f.builtin;
        let mut l = Layout {
            skin,
            env,
            focus: self.focus,
            items: Vec::new(),
            builtin,
            stack: Vec::new(),
        };
        if overlay {
            let Some(o) = &self.overlay else {
                return Vec::new();
            };
            let comp = skin
                .components
                .get(&o.component)
                .or_else(|| builtin.get(&o.component))
                .or_else(|| builtin.get("knob overlay"));
            if let Some(c) = comp {
                let mut chain: Chain = vec![(&f.remap, 0)];
                let full = RectF::new(0.0, 0.0, TAB_W, SCREEN_H);
                l.focus = Some(1);
                l.walk(c, full, px(full), 1.0, &mut chain, 1, false, 0);
            }
            return l.items;
        }
        let Some(t) = skin.tabs.get(self.tab) else {
            return Vec::new();
        };
        let Some(root) = l.component(&t.root) else {
            return Vec::new();
        };
        let full = RectF::new(0.0, 0.0, TAB_W * t.scale, TAB_H * t.scale);
        let mut chain: Chain = Vec::new();
        l.walk(root, full, px(full), t.scale, &mut chain, 0, false, 0);
        l.items
    }

    /// Draws the current tab into `c` (1280 x 628), only inside `area`.
    pub fn draw(&mut self, params: &dyn Params, c: &mut SoftCanvas, area: Rect) {
        self.tick += 1;
        self.images.frame();
        let f = self.frame();
        let skin = Arc::clone(&f.skin);
        let items: Vec<Item> = self.with_env(params, |env| self.layout(&f, env, false));
        let old = c.clip();
        c.set_clip(area);
        // The MPC fills a tab black under its root's background.
        c.fill_rect(area, Color::rgb(0));
        self.paint_items(params, c, &items);
        // The Q-Link bank's highlight.
        let hide = items.iter().any(|i| {
            matches!(
                i.what,
                What::Comp {
                    hide_qlink_bounds: true,
                    ..
                }
            )
        });
        if let (false, Some(t)) = (hide, skin.tabs.get(self.tab)) {
            if let Some(r) = t.qlink_bounds.get(self.bank) {
                if r.w > 2.0 && r.h > 2.0 {
                    let r = px(*r);
                    c.stroke_round_rect(r, 8, 3, Color::rgb(0xFFFFFF).with_alpha(170));
                }
            }
        }
        c.set_clip(old);
    }

    /// Draws the open overlay into `c` (1280 x 800, the whole screen). Does
    /// nothing without one.
    pub fn draw_overlay(&mut self, params: &dyn Params, c: &mut SoftCanvas) {
        if self.overlay.is_none() {
            return;
        }
        self.images.frame();
        let f = self.frame();
        let items: Vec<Item> = self.with_env(params, |env| self.layout(&f, env, true));
        self.paint_items(params, c, &items);
    }

    fn paint_items(&mut self, params: &dyn Params, c: &mut SoftCanvas, items: &[Item]) {
        for it in items {
            if it.clip.is_empty() {
                continue;
            }
            match &it.what {
                What::Back(f, dir) => {
                    fill_clipped(c, it.rect, f.colour, it.clip);
                    if let Some(p) = self.images.get(dir, &f.image) {
                        paint::picture(c, &p, it.rect, it.clip);
                    }
                }
                What::Comp { .. } => {}
                What::Widget {
                    w,
                    handle,
                    y_handle,
                    dir,
                } => {
                    let v = self.with_env(params, |env| env.eval(&Handle::parse(handle)));
                    let vy = if y_handle.is_empty() {
                        Val::default()
                    } else {
                        self.with_env(params, |env| env.eval(&Handle::parse(y_handle)))
                    };
                    self.paint_widget(c, w, it.rect, it.clip, &v, &vy, dir);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_widget(
        &mut self,
        c: &mut SoftCanvas,
        w: &Widget,
        r: RectF,
        clip: Rect,
        v: &Val,
        vy: &Val,
        dir: &Path,
    ) {
        match w {
            Widget::Image {
                colour,
                image,
                frames,
                handle,
                animated,
            } => {
                fill_clipped(c, r, *colour, clip);
                if let Some(p) = self.images.get(dir, image) {
                    if *frames > 1 {
                        let f = if *animated {
                            (self.tick / 2 % *frames as u64) as u32
                        } else if handle.is_empty() || !v.valid {
                            0
                        } else {
                            v.step(*frames)
                        };
                        paint::frame(c, &p, *frames, f, r, clip);
                    } else {
                        paint::picture(c, &p, r, clip);
                    }
                }
            }
            Widget::Knob { kind, .. } => match kind {
                KnobKind::FilmStrip {
                    strip,
                    frames,
                    invert,
                } => {
                    if let Some(p) = self.images.get(dir, strip) {
                        let mut f = v.step(*frames);
                        if *invert {
                            f = frames - 1 - f.min(frames - 1);
                        }
                        paint::frame(
                            c,
                            &p,
                            *frames,
                            f,
                            fit(r, p.w as f32, (p.h / frames.max(&1)) as f32),
                            clip,
                        );
                    }
                }
                KnobKind::ValueSlider(style) => {
                    text(c, &mut self.fonts, style, r, &v.text, clip);
                }
                KnobKind::Generic { label, bipolar } => {
                    generic_knob(c, &mut self.fonts, r, clip, v, *label, *bipolar);
                }
                KnobKind::Rotating {
                    foreground,
                    background,
                    scale,
                } => {
                    let s = *scale;
                    let rr = RectF::new(
                        r.x + r.w * (1.0 - s) / 2.0,
                        r.y + r.h * (1.0 - s) / 2.0,
                        r.w * s,
                        r.h * s,
                    );
                    if let Some(p) = self.images.get(dir, background) {
                        paint::picture(c, &p, rr, clip);
                    }
                    if let Some(p) = self.images.get(dir, foreground) {
                        let angle = (v.value - 0.5) * 1.5 * PI;
                        rotated(c, &p, rr, angle, clip);
                    }
                }
            },
            Widget::Label {
                style,
                kind,
                text: t,
                ..
            } => {
                let s = match kind {
                    LabelKind::Name => v.name.as_str(),
                    LabelKind::Value => v.text.as_str(),
                    LabelKind::Text => t.as_str(),
                };
                text(c, &mut self.fonts, style, r, s, clip);
            }
            Widget::Button {
                on, off, id, group, ..
            }
            | Widget::Indicator {
                on, off, id, group, ..
            } => {
                let steps = (*group).max(2);
                let lit = v.valid && v.step(steps) == *id;
                if let Some(p) = self.images.get(dir, if lit { on } else { off }) {
                    // Drawn at their own size, centred.
                    let d = RectF::new(
                        (r.x + (r.w - p.w as f32) / 2.0).round(),
                        (r.y + (r.h - p.h as f32) / 2.0).round(),
                        p.w as f32,
                        p.h as f32,
                    );
                    paint::picture(c, &p, d, clip);
                }
            }
            Widget::Focus {
                background,
                outline: o,
                inset,
                thickness,
            } => {
                let i = *inset;
                fill_clipped(
                    c,
                    RectF::new(r.x + i, r.y + i, r.w - 2.0 * i, r.h - 2.0 * i),
                    *background,
                    clip,
                );
                with_clip(c, clip, |c| outline(c, r, *thickness, *o));
            }
            Widget::Slider { kind, .. } => {
                self.slider(c, kind, r, clip, v, dir);
            }
            Widget::Separator { colour, vertical } => {
                let l = if *vertical {
                    RectF::new(r.x + r.w / 2.0 - 0.5, r.y, 1.0, r.h)
                } else {
                    RectF::new(r.x, r.y + r.h / 2.0 - 0.5, r.w, 1.0)
                };
                fill_clipped(c, l, *colour, clip);
            }
            Widget::ComboBox { style, .. } => {
                text(c, &mut self.fonts, style, r, &v.text, clip);
            }
            Widget::XYPad { image, .. } => {
                if let Some(p) = self.images.get(dir, image) {
                    let cx = r.x + v.value * r.w;
                    let cy = r.y + (1.0 - vy.value) * r.h;
                    let d = RectF::new(
                        (cx - p.w as f32 / 2.0).round(),
                        (cy - p.h as f32 / 2.0).round(),
                        p.w as f32,
                        p.h as f32,
                    );
                    paint::picture(c, &p, d, clip);
                }
            }
            Widget::HaloDisplay {
                width,
                colour,
                clip: clip_colour,
                style,
                arc,
                ..
            } => {
                let col = if v.value >= 0.999 && clip_colour >> 24 != 0 {
                    *clip_colour
                } else {
                    *colour
                };
                with_clip(c, clip, |c| match style.as_str() {
                    "Horizontal" => fill(c, RectF::new(r.x, r.y, r.w * v.value, r.h), col),
                    "Vertical" => fill(
                        c,
                        RectF::new(r.x, r.y + r.h * (1.0 - v.value), r.w, r.h * v.value),
                        col,
                    ),
                    _ => {
                        // Angles as fractions of a turn from the top.
                        let a0 = arc.0 * 2.0 * PI - PI / 2.0;
                        let a1 = (arc.0 + (arc.1 - arc.0) * v.value) * 2.0 * PI - PI / 2.0;
                        if a1 > a0 + 1e-3 {
                            let rad = r.w.min(r.h) / 2.0 - width / 2.0;
                            let mut s = Shape::new(px(r));
                            s.arc(r.x + r.w / 2.0, r.y + r.h / 2.0, rad, a0, a1, *width);
                            c.blend_shape(&s, color(col));
                        }
                    }
                });
            }
            Widget::Meter {
                vertical,
                invert,
                inactive,
                peak,
                ..
            } => {
                if let Some(p) = self.images.get(dir, inactive) {
                    paint::picture(c, &p, r, clip);
                }
                if let Some(p) = self.images.get(dir, peak) {
                    let f = if *invert { 1.0 - v.value } else { v.value }.clamp(0.0, 1.0);
                    let lit = if *vertical {
                        RectF::new(r.x, r.y + r.h * (1.0 - f), r.w, r.h * f)
                    } else {
                        RectF::new(r.x, r.y, r.w * f, r.h)
                    };
                    paint::picture(c, &p, r, clip.intersect(px(lit)));
                }
            }
            Widget::Envelope { colours, .. } => {
                fill_clipped(c, r, colours[0], clip);
            }
            Widget::KnobOverlay(k) => {
                self.knob_overlay(c, r, v, k.bipolar);
            }
            Widget::MenuOverlay { numbers, .. } => {
                self.menu_overlay(c, r, v, *numbers);
            }
            Widget::NumericOverlay { .. } => {
                self.knob_overlay(c, r, v, false);
            }
            Widget::Custom(_) | Widget::Unknown(_) => {}
        }
    }

    fn slider(
        &mut self,
        c: &mut SoftCanvas,
        kind: &SliderKind,
        r: RectF,
        clip: Rect,
        v: &Val,
        dir: &Path,
    ) {
        let val = v.value.clamp(0.0, 1.0);
        match kind {
            SliderKind::ColourBar {
                vertical,
                background,
                outline: o,
                value,
                bipolar,
            } => {
                fill_clipped(c, r, *background, clip);
                let (from, to) = if *bipolar {
                    (val.min(0.5), val.max(0.5))
                } else {
                    (0.0, val)
                };
                let bar = if *vertical {
                    RectF::new(r.x, r.y + r.h * (1.0 - to), r.w, r.h * (to - from))
                } else {
                    RectF::new(r.x + r.w * from, r.y, r.w * (to - from), r.h)
                };
                fill_clipped(c, bar, *value, clip);
                with_clip(c, clip, |c| outline(c, r, 1.0, *o));
            }
            SliderKind::TrackAndThumb {
                vertical,
                thumb,
                track,
                revealed,
                reveal,
            } => {
                if *track {
                    let t = if *vertical {
                        RectF::new(r.x + r.w / 2.0 - 2.0, r.y, 4.0, r.h)
                    } else {
                        RectF::new(r.x, r.y + r.h / 2.0 - 2.0, r.w, 4.0)
                    };
                    fill_clipped(c, t, 0xFF3A_3A3A, clip);
                }
                if let Some(p) = self.images.get(dir, revealed) {
                    let (a, b) = match reveal {
                        1 => (val, 1.0),
                        2 => (val.min(0.5), val.max(0.5)),
                        _ => (0.0, val),
                    };
                    let part = if *vertical {
                        RectF::new(r.x, r.y + r.h * (1.0 - b), r.w, r.h * (b - a))
                    } else {
                        RectF::new(r.x + r.w * a, r.y, r.w * (b - a), r.h)
                    };
                    paint::picture(c, &p, r, clip.intersect(px(part)));
                }
                if let Some(p) = self.images.get(dir, thumb) {
                    let (tw, th) = (p.w as f32, p.h as f32);
                    let d = if *vertical {
                        RectF::new(
                            r.x + (r.w - tw) / 2.0,
                            r.y + (r.h - th) * (1.0 - val),
                            tw,
                            th,
                        )
                    } else {
                        RectF::new(r.x + (r.w - tw) * val, r.y + (r.h - th) / 2.0, tw, th)
                    };
                    paint::picture(c, &p, RectF::new(d.x.round(), d.y.round(), tw, th), clip);
                }
            }
            SliderKind::Generic => {
                // The MPC's slot: the name in capitals and the value on one
                // line, a bar under them.
                let head = crate::model::TextStyle {
                    font: "Titillium Web".into(),
                    style: "SemiBold".into(),
                    height: (r.h * 0.3).clamp(14.0, 30.0),
                    colour: 0xFFFF_FFFF,
                    h_align: crate::model::HAlign::Left,
                    v_align: crate::model::VAlign::Centre,
                    case: crate::model::Case::Upper,
                };
                let line = RectF::new(r.x + 8.0, r.y, r.w - 16.0, r.h * 0.55);
                text(c, &mut self.fonts, &head, line, &v.name, clip);
                let value_style = crate::model::TextStyle {
                    h_align: crate::model::HAlign::Right,
                    case: crate::model::Case::Original,
                    colour: 0xFFB4_B4B6,
                    ..head
                };
                text(c, &mut self.fonts, &value_style, line, &v.text, clip);
                let bar = RectF::new(
                    r.x + 8.0,
                    r.y + r.h * 0.62,
                    r.w - 16.0,
                    (r.h * 0.18).max(6.0),
                );
                fill_clipped(c, bar, 0xFF2A_2A2A, clip);
                fill_clipped(
                    c,
                    RectF::new(bar.x, bar.y, bar.w * val, bar.h),
                    0xFFD3_1145,
                    clip,
                );
            }
        }
    }

    fn knob_overlay(&mut self, c: &mut SoftCanvas, r: RectF, v: &Val, bipolar: bool) {
        // MPFree's own large value editor: the screen dimmed, a panel with
        // the parameter's name, a big knob and its value.
        fill(c, r, 0xC000_0000);
        let panel = overlay_panel(r);
        c.fill_round_rect(px(panel), 18, Color::rgb(0x1E1D1E));
        c.stroke_round_rect(px(panel), 18, 2, Color::rgb(0x3A3A3A));
        let title = crate::model::TextStyle {
            font: "Titillium Web".into(),
            style: "SemiBold".into(),
            height: 40.0,
            colour: 0xFFFF_FFFF,
            h_align: crate::model::HAlign::Centre,
            v_align: crate::model::VAlign::Centre,
            case: crate::model::Case::Upper,
        };
        let full = px(r);
        text(
            c,
            &mut self.fonts,
            &title,
            RectF::new(panel.x, panel.y + 16.0, panel.w, 56.0),
            &v.name,
            full,
        );
        let k = RectF::new(
            panel.x + panel.w / 2.0 - 150.0,
            panel.y + 86.0,
            300.0,
            300.0,
        );
        generic_knob(c, &mut self.fonts, k, full, v, false, bipolar);
        let value = crate::model::TextStyle {
            height: 52.0,
            case: crate::model::Case::Original,
            style: "Bold".into(),
            ..title
        };
        text(
            c,
            &mut self.fonts,
            &value,
            RectF::new(panel.x, panel.y + panel.h - 96.0, panel.w, 72.0),
            &v.text,
            full,
        );
    }

    fn menu_overlay(&mut self, c: &mut SoftCanvas, r: RectF, v: &Val, numbers: bool) {
        fill(c, r, 0xC000_0000);
        let panel = overlay_panel(r);
        c.fill_round_rect(px(panel), 18, Color::rgb(0x1E1D1E));
        let full = px(r);
        let names = menu_names(v);
        let cur = v.steps.map(|n| v.step(n) as usize);
        let rows = menu_rows(panel, names.len(), self.menu_scroll);
        let style = crate::model::TextStyle {
            font: "Titillium Web".into(),
            style: "SemiBold".into(),
            height: 34.0,
            colour: 0xFFFF_FFFF,
            h_align: crate::model::HAlign::Left,
            v_align: crate::model::VAlign::Centre,
            case: crate::model::Case::Original,
        };
        for (i, row) in rows {
            if Some(i) == cur {
                c.fill_round_rect(px(row), 10, Color::rgb(0xD31145));
            }
            let label = if numbers {
                format!("{}  {}", i + 1, names[i])
            } else {
                names[i].clone()
            };
            text(
                c,
                &mut self.fonts,
                &style,
                RectF::new(row.x + 20.0, row.y, row.w - 40.0, row.h),
                &label,
                full,
            );
        }
    }

    // ---- input ----

    fn items_now<'s>(&self, params: &dyn Params, f: &'s Frame) -> Vec<Item<'s>> {
        self.with_env(params, |env| self.layout(f, env, self.overlay.is_some()))
    }

    fn val(&self, params: &dyn Params, h: &str) -> Val {
        self.with_env(params, |env| env.eval(&Handle::parse(h)))
    }

    /// The edit that sets `h` to `value` (0 to 1, as shown): flipped back
    /// through an Invert, stepped for a discrete handle. A Local handle is
    /// set here and gives no edit.
    fn set(&mut self, params: &dyn Params, h: &str, value: f32, out: &mut Vec<Edit>) {
        let v = self.val(params, h);
        let mut x = value.clamp(0.0, 1.0);
        if let Some(n) = v.steps {
            if n > 1 {
                x = (x * (n - 1) as f32).round() / (n - 1) as f32;
            }
        }
        if v.inverted {
            x = 1.0 - x;
        }
        match v.target {
            Some(Target::Param(i)) => {
                if (params.value(i) - x).abs() > 1e-6 {
                    out.push(Edit::Set(i, x));
                }
            }
            Some(Target::Local(id, n)) => {
                let s = if n > 1 {
                    (x * (n - 1) as f32).round() as u32
                } else {
                    0
                };
                self.locals.insert(id, s);
            }
            None => {}
        }
    }

    fn param_of(&self, params: &dyn Params, h: &str) -> Option<usize> {
        match self.val(params, h).target {
            Some(Target::Param(i)) => Some(i),
            _ => None,
        }
    }

    fn run_actions(
        &mut self,
        params: &dyn Params,
        actions: &[ResolvedAction],
        on: OnAction,
        out: &mut Vec<Edit>,
    ) -> bool {
        let mut any = false;
        for a in actions.iter().filter(|a| a.on == on) {
            any = true;
            match a.handler {
                Handler::QLink => {
                    if let (Some(p), Some(k)) =
                        (self.param_of(params, &a.handle), self.knobs_for_tab())
                    {
                        if let Some(b) = k.bank_of(p) {
                            self.bank = b;
                        }
                    }
                }
                Handler::ShowOverlay => {
                    let v = self.val(params, &a.handle);
                    if v.valid {
                        self.overlay = Some(Overlay {
                            component: a.data.clone(),
                            handle: a.handle.clone(),
                        });
                        self.overlay_remap = Remap(vec![("Data".to_string(), a.handle.clone())]);
                        self.menu_scroll = 0;
                        self.gesture = None;
                    }
                }
                Handler::ToggleSwitch => {
                    let v = self.val(params, &a.handle);
                    if v.valid {
                        let to = if v.value >= 0.5 { 0.0 } else { 1.0 };
                        if let Some(p) = self.param_of(params, &a.handle) {
                            out.push(Edit::Begin(p));
                            self.set(params, &a.handle, to, out);
                            out.push(Edit::End(p));
                        } else {
                            self.set(params, &a.handle, to, out);
                        }
                    }
                }
                Handler::FocusLost => self.close_overlay(),
                Handler::Other(_) => {}
            }
        }
        any
    }

    /// Runs `on` on the innermost enclosing component of `start` that has
    /// such an action.
    fn bubble(
        &mut self,
        params: &dyn Params,
        items: &[(Option<usize>, Vec<ResolvedAction>)],
        start: Option<usize>,
        on: OnAction,
        out: &mut Vec<Edit>,
    ) {
        let mut at = start;
        let mut guard = 0;
        while let Some(i) = at {
            if items[i].1.iter().any(|a| a.on == on) {
                let acts = items[i].1.clone();
                self.run_actions(params, &acts, on, out);
                return;
            }
            at = items[i].0;
            guard += 1;
            if guard > 64 {
                return;
            }
        }
    }

    /// Feeds a touch, in the tab's coordinates (1280 x 628) even while an
    /// overlay is open. Returns the parameter edits it makes.
    pub fn touch(&mut self, params: &dyn Params, t: Touch) -> Vec<Edit> {
        let mut out = Vec::new();
        // An overlay's items are laid out on the whole screen.
        let dy = if self.overlay.is_some() { TAB_TOP } else { 0.0 };
        let t = match t {
            Touch::Down { x, y, ms } => Touch::Down { x, y: y + dy, ms },
            Touch::Move { x, y } => Touch::Move { x, y: y + dy },
            Touch::Up { x, y } => Touch::Up { x, y: y + dy },
        };
        match t {
            Touch::Down { x, y, ms } => self.down(params, x, y, ms, &mut out),
            Touch::Move { x, y } => self.drag(params, x, y, &mut out),
            Touch::Up { x, y } => {
                self.drag(params, x, y, &mut out);
                if let Some(g) = self.gesture.take() {
                    let v = self.val(params, &g.handle);
                    if v.momentary {
                        self.set(params, &g.handle, 0.0, &mut out);
                    }
                    for p in g.params {
                        out.push(Edit::End(p));
                    }
                }
            }
        }
        out
    }

    fn down(&mut self, params: &dyn Params, x: f32, y: f32, ms: u64, out: &mut Vec<Edit>) {
        let f = self.frame();
        let items = self.items_now(params, &f);
        // What each component item does, for bubbling.
        let tree: Vec<(Option<usize>, Vec<ResolvedAction>)> = items
            .iter()
            .map(|i| {
                (
                    i.parent,
                    match &i.what {
                        What::Comp { actions, .. } => actions.clone(),
                        _ => Vec::new(),
                    },
                )
            })
            .collect();
        let inside = |i: &Item| i.clip.contains(x.floor() as i32, y.floor() as i32);
        let hit = (0..items.len()).rev().find(|&k| {
            let i = &items[k];
            inside(i)
                && match &i.what {
                    What::Widget { w, .. } => is_interactive(w),
                    What::Comp {
                        actions,
                        ignore_mouse,
                        ..
                    } => !ignore_mouse && (!actions.is_empty() || i.focusable),
                    What::Back(..) => false,
                }
        });
        if self.overlay.is_some() {
            self.overlay_down(params, &items, hit, x, y, out);
            return;
        }
        let Some(k) = hit else {
            return;
        };
        let it = &items[k];
        // Focus: the innermost focusable item from the hit outwards.
        let mut f = Some(k);
        while let Some(i) = f {
            if items[i].focusable {
                self.focus = Some(items[i].path);
                break;
            }
            f = items[i].parent;
        }
        let comp_start = match it.what {
            What::Comp { .. } => Some(k),
            _ => it.parent,
        };
        let double = matches!(self.last_down, Some((t, p)) if p == it.path && ms.saturating_sub(t) < DOUBLE_TAP_MS);
        self.last_down = Some((ms, it.path));
        self.bubble(params, &tree, comp_start, OnAction::MouseDown, out);
        if double {
            self.last_down = None;
            self.bubble(params, &tree, comp_start, OnAction::DoubleClick, out);
            if self.overlay.is_some() {
                return;
            }
        }
        if let What::Widget {
            w,
            handle,
            y_handle,
            ..
        } = &it.what
        {
            self.start_gesture(params, w, handle, y_handle, it.rect, it.path, x, y, out);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_gesture(
        &mut self,
        params: &dyn Params,
        w: &Widget,
        handle: &str,
        y_handle: &str,
        rect: RectF,
        path: u64,
        x: f32,
        y: f32,
        out: &mut Vec<Edit>,
    ) {
        let kind = match w {
            Widget::Knob { drag, .. } => GestureKind::Knob(*drag),
            Widget::Slider { kind, .. } => match kind {
                SliderKind::ColourBar { vertical: true, .. }
                | SliderKind::TrackAndThumb { vertical: true, .. } => GestureKind::SliderV,
                _ => GestureKind::SliderH,
            },
            Widget::XYPad { .. } => GestureKind::XY,
            Widget::Button { .. } => GestureKind::Button,
            Widget::ComboBox { .. } => GestureKind::Knob(Drag::Vertical),
            _ => return,
        };
        let v = self.val(params, handle);
        if !v.valid {
            return;
        }
        let vy = self.val(params, y_handle);
        let mut ps: Vec<usize> = self.param_of(params, handle).into_iter().collect();
        if kind == GestureKind::XY {
            ps.extend(self.param_of(params, y_handle));
        }
        for &p in &ps {
            out.push(Edit::Begin(p));
        }
        self.gesture = Some(Gesture {
            path,
            kind,
            handle: handle.to_string(),
            y_handle: y_handle.to_string(),
            rect,
            start: (x, y),
            start_value: v.value,
            start_y_value: vy.value,
            params: ps,
        });
        match (kind, w) {
            (GestureKind::Button, Widget::Button { id, group, .. }) => {
                let steps = (*group).max(2);
                let to = if *group <= 1 {
                    if v.step(2) == *id {
                        1.0 - *id as f32
                    } else {
                        *id as f32
                    }
                } else {
                    *id as f32 / (steps - 1) as f32
                };
                self.set(params, handle, to, out);
            }
            (GestureKind::SliderV | GestureKind::SliderH | GestureKind::XY, _) => {
                self.drag(params, x, y, out);
            }
            (GestureKind::Knob(Drag::Rotary), _) => self.drag(params, x, y, out),
            _ => {}
        }
    }

    fn drag(&mut self, params: &dyn Params, x: f32, y: f32, out: &mut Vec<Edit>) {
        let Some(g) = self.gesture.clone() else {
            return;
        };
        let r = g.rect;
        match g.kind {
            GestureKind::Knob(Drag::Vertical) | GestureKind::Overlay => {
                let v = g.start_value + (g.start.1 - y) / DRAG_RANGE;
                self.set(params, &g.handle, v, out);
            }
            GestureKind::Knob(Drag::Horizontal) => {
                let v = g.start_value + (x - g.start.0) / DRAG_RANGE;
                self.set(params, &g.handle, v, out);
            }
            GestureKind::Knob(Drag::Rotary) => {
                let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                // 0 at the bottom left (-135 degrees from the top), 1 at
                // the bottom right.
                let a = (x - cx).atan2(cy - y); // 0 at the top, clockwise
                let v = (a + 0.75 * PI) / (1.5 * PI);
                self.set(params, &g.handle, v, out);
            }
            GestureKind::SliderV => {
                let v = 1.0 - (y - r.y) / r.h.max(1.0);
                self.set(params, &g.handle, v, out);
            }
            GestureKind::SliderH => {
                let v = (x - r.x) / r.w.max(1.0);
                self.set(params, &g.handle, v, out);
            }
            GestureKind::XY => {
                let vx = (x - r.x) / r.w.max(1.0);
                let vy = 1.0 - (y - r.y) / r.h.max(1.0);
                self.set(params, &g.handle, vx, out);
                if !g.y_handle.is_empty() {
                    self.set(params, &g.y_handle, vy, out);
                }
            }
            GestureKind::Button => {}
        }
        let _ = (g.path, g.start_y_value);
    }

    fn overlay_down(
        &mut self,
        params: &dyn Params,
        items: &[Item],
        hit: Option<usize>,
        x: f32,
        y: f32,
        out: &mut Vec<Edit>,
    ) {
        let Some(o) = self.overlay.clone() else {
            return;
        };
        let full = RectF::new(0.0, 0.0, TAB_W, SCREEN_H);
        let panel = overlay_panel(full);
        let w = hit.and_then(|k| match &items[k].what {
            What::Widget { w, handle, .. } => Some(((*w).clone(), handle.clone(), items[k].rect)),
            _ => None,
        });
        match w {
            Some((Widget::MenuOverlay { .. }, h, r)) => {
                let v = self.val(params, &h);
                let names = menu_names(&v);
                let p = overlay_panel(r);
                if !p.contains(x, y) {
                    self.close_overlay();
                    return;
                }
                for (i, row) in menu_rows(p, names.len(), self.menu_scroll) {
                    if row.contains(x, y) {
                        let n = names.len().max(2) as f32;
                        if let Some(pi) = self.param_of(params, &h) {
                            out.push(Edit::Begin(pi));
                            self.set(params, &h, i as f32 / (n - 1.0), out);
                            out.push(Edit::End(pi));
                        } else {
                            self.set(params, &h, i as f32 / (n - 1.0), out);
                        }
                        self.close_overlay();
                        return;
                    }
                }
            }
            Some((Widget::KnobOverlay(_) | Widget::NumericOverlay { .. }, h, _)) => {
                if !panel.contains(x, y) {
                    self.close_overlay();
                    return;
                }
                let v = self.val(params, &h);
                let ps: Vec<usize> = self.param_of(params, &h).into_iter().collect();
                for &p in &ps {
                    out.push(Edit::Begin(p));
                }
                self.gesture = Some(Gesture {
                    path: 1,
                    kind: GestureKind::Overlay,
                    handle: h,
                    y_handle: String::new(),
                    rect: panel,
                    start: (x, y),
                    start_value: v.value,
                    start_y_value: 0.0,
                    params: ps,
                });
            }
            Some((w, h, r)) => {
                // A skin's own overlay with ordinary widgets in it.
                let p = items[hit.unwrap_or(0)].path;
                self.start_gesture(params, &w, &h, "", r, p, x, y, out);
            }
            None => {
                let _ = o;
                self.close_overlay();
            }
        }
    }

    /// Where a finger does something now, in the tab's coordinates: each
    /// control and each component with actions (for the GUI's control
    /// audit).
    pub fn controls(&self, params: &dyn Params) -> Vec<RectF> {
        let f = self.frame();
        let items = self.items_now(params, &f);
        let off = if self.overlay.is_some() { TAB_TOP } else { 0.0 };
        items
            .iter()
            .filter(|i| {
                !i.clip.is_empty()
                    && match &i.what {
                        What::Widget { w, .. } => is_interactive(w),
                        What::Comp {
                            actions,
                            ignore_mouse,
                            ..
                        } => !ignore_mouse && !actions.is_empty(),
                        What::Back(..) => false,
                    }
            })
            .map(|i| {
                let c = i.clip;
                RectF::new(c.x as f32, c.y as f32 - off, c.w as f32, c.h as f32)
            })
            .collect()
    }

    /// True when a control has the focus (the wheel turns it).
    pub fn has_focus(&self) -> bool {
        self.focus.is_some()
    }

    /// Moves the focus `delta` controls on, in reading order (top to
    /// bottom, then left to right), among the ones that take it: the
    /// wheel's pick. From no focus, the first (or last).
    pub fn focus_step(&mut self, params: &dyn Params, delta: i32) {
        if self.overlay.is_some() || delta == 0 {
            return;
        }
        let f = self.frame();
        let items = self.items_now(params, &f);
        let mut order: Vec<(i32, i32, u64)> = items
            .iter()
            .filter(|i| i.focusable && !i.clip.is_empty())
            .map(|i| {
                (
                    (i.rect.y / 24.0).round() as i32,
                    i.rect.x.round() as i32,
                    i.path,
                )
            })
            .collect();
        if order.is_empty() {
            return;
        }
        order.sort();
        order.dedup_by_key(|o| o.2);
        let n = order.len() as i32;
        let at = self
            .focus
            .and_then(|p| order.iter().position(|o| o.2 == p))
            .map(|i| i as i32);
        let next = match at {
            Some(i) => (i + delta).clamp(0, n - 1),
            None if delta > 0 => 0,
            None => n - 1,
        };
        self.focus = Some(order[next as usize].2);
    }

    /// The data wheel: `detents` clicks (negative turns down); `fine` with
    /// SHIFT. Turns the open overlay's value, else the focused control's.
    pub fn wheel(&mut self, params: &dyn Params, detents: i32, fine: bool) -> Vec<Edit> {
        let mut out = Vec::new();
        let Some((h, comp_fine)) = self.focused_handle(params) else {
            return out;
        };
        let v = self.val(params, &h);
        if !v.valid {
            return out;
        }
        let step = match v.steps {
            Some(n) if n > 1 => 1.0 / (n - 1) as f32,
            _ if fine || comp_fine => 0.001,
            _ => 0.01,
        };
        let shown = v.value + detents as f32 * step;
        if let Some(p) = self.param_of(params, &h) {
            out.push(Edit::Begin(p));
            self.set(params, &h, shown, &mut out);
            out.push(Edit::End(p));
        } else {
            self.set(params, &h, shown, &mut out);
        }
        out
    }

    /// The data wheel's push (Enter): the focused control's Enter action
    /// (usually its overlay), or closes the open overlay.
    pub fn enter(&mut self, params: &dyn Params) -> Vec<Edit> {
        let mut out = Vec::new();
        if self.overlay.is_some() {
            self.close_overlay();
            return out;
        }
        let f = self.frame();
        let items = self.items_now(params, &f);
        let Some(k) = items.iter().position(|i| Some(i.path) == self.focus) else {
            return out;
        };
        let tree: Vec<(Option<usize>, Vec<ResolvedAction>)> = items
            .iter()
            .map(|i| {
                (
                    i.parent,
                    match &i.what {
                        What::Comp { actions, .. } => actions.clone(),
                        _ => Vec::new(),
                    },
                )
            })
            .collect();
        let start = match items[k].what {
            What::Comp { .. } => Some(k),
            _ => items[k].parent,
        };
        self.bubble(params, &tree, start, OnAction::EnterPressed, &mut out);
        out
    }

    fn focused_handle(&self, params: &dyn Params) -> Option<(String, bool)> {
        if let Some(o) = &self.overlay {
            return Some((o.handle.clone(), false));
        }
        let f = self.frame();
        let items = self.items_now(params, &f);
        let it = items.iter().find(|i| Some(i.path) == self.focus)?;
        match &it.what {
            What::Comp { data, fine, .. } => Some((data.clone(), *fine)),
            What::Widget { handle, .. } => Some((handle.clone(), false)),
            What::Back(..) => None,
        }
    }

    /// Q-Link knob `knob` (0 to 3, top first) turned `detents` clicks:
    /// a hundredth of the range each (a thousandth with `fine`), or one
    /// step of a discrete parameter.
    pub fn qlink(
        &mut self,
        params: &dyn Params,
        knob: usize,
        detents: i32,
        fine: bool,
    ) -> Vec<Edit> {
        let mut out = Vec::new();
        let Some(Some(p)) = self.knobs().get(knob).copied() else {
            return out;
        };
        let h = format!("Parameter {p}");
        // The knob turns the parameter itself (the tab's offset is already
        // in the Q-Links file's index).
        let off = self.skin.tabs.get(self.tab).map_or(0, |t| t.param_offset);
        let h = if off > 0 && p >= off {
            format!("Parameter {}", p - off)
        } else {
            h
        };
        let v = self.val(params, &h);
        if v.valid {
            let step = match v.steps {
                Some(n) if n > 1 => 1.0 / (n - 1) as f32,
                _ if fine => 0.001,
                _ => 0.01,
            };
            self.set(params, &h, v.value + detents as f32 * step, &mut out);
        }
        out
    }
}

/// `r` cut to `clip`, filled.
fn fill_clipped(c: &mut SoftCanvas, r: RectF, argb: u32, clip: Rect) {
    if argb >> 24 == 0 {
        return;
    }
    let a = px(r).intersect(clip);
    if !a.is_empty() {
        c.fill_rect(a, color(argb));
    }
}

fn with_clip(c: &mut SoftCanvas, clip: Rect, f: impl FnOnce(&mut SoftCanvas)) {
    let old = c.clip();
    c.set_clip(old.intersect(clip));
    f(c);
    c.set_clip(old);
}

/// A picture of `w` x `h` fitted into `r` keeping its shape, centred; `r`
/// itself when the shapes match.
fn fit(r: RectF, w: f32, h: f32) -> RectF {
    if w <= 0.0 || h <= 0.0 {
        return r;
    }
    let k = (r.w / w).min(r.h / h);
    if ((r.w / w) - (r.h / h)).abs() < 0.02 {
        return r;
    }
    RectF::new(
        r.x + (r.w - w * k) / 2.0,
        r.y + (r.h - h * k) / 2.0,
        w * k,
        h * k,
    )
}

/// A picture rotated by `angle` (radians, clockwise) about its centre,
/// scaled into `r` (nearest pixel).
fn rotated(c: &mut SoftCanvas, p: &crate::png::Picture, r: RectF, angle: f32, clip: Rect) {
    let area = px(r).intersect(c.clip()).intersect(clip);
    let (cw, _) = c.size();
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let (s, co) = angle.sin_cos();
    let kx = p.w as f32 / r.w.max(1.0);
    let ky = p.h as f32 / r.h.max(1.0);
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            // Back-rotate into the picture.
            let ux = dx * co + dy * s;
            let uy = -dx * s + dy * co;
            let sx = (ux + r.w / 2.0) * kx;
            let sy = (uy + r.h / 2.0) * ky;
            if sx < 0.0 || sy < 0.0 || sx >= p.w as f32 || sy >= p.h as f32 {
                continue;
            }
            let src = p.px[(sy as u32 * p.w + sx as u32) as usize];
            let i = (y * cw + x) as usize;
            let a = src >> 24;
            if a == 0 {
                continue;
            }
            let inv = 255 - a;
            let d = c.px[i];
            let ch = |sh: u32| {
                ((((src >> sh) & 0xFF) + (((d >> sh) & 0xFF) * inv + 127) / 255).min(255)) << sh
            };
            c.px[i] = ch(16) | ch(8) | ch(0);
        }
    }
}

/// MPFree's knob for the generic knob types and the knob overlay: a 270
/// degree track, the value's arc, a pointer, and (with `label`) the value
/// under it.
fn generic_knob(
    c: &mut SoftCanvas,
    fonts: &mut Fonts,
    r: RectF,
    clip: Rect,
    v: &Val,
    label: bool,
    bipolar: bool,
) {
    let text_h = if label {
        (r.h * 0.22).clamp(12.0, 40.0)
    } else {
        0.0
    };
    let size = r.w.min(r.h - text_h);
    if size <= 4.0 {
        return;
    }
    let (cx, cy) = (r.x + r.w / 2.0, r.y + (r.h - text_h) / 2.0);
    let width = (size * 0.09).max(2.0);
    let rad = size / 2.0 - width;
    let a0 = 0.75 * PI;
    let a1 = 2.25 * PI;
    let val = v.value.clamp(0.0, 1.0);
    let at = a0 + (a1 - a0) * val;
    with_clip(c, clip, |c| {
        let mut track = Shape::new(px(r));
        track.arc(cx, cy, rad, a0, a1, width);
        c.blend_shape(&track, Color::rgb(0x3A3A3A));
        let (from, to) = if bipolar {
            let mid = (a0 + a1) / 2.0;
            (mid.min(at), mid.max(at))
        } else {
            (a0, at)
        };
        if to > from + 1e-3 {
            let mut lit = Shape::new(px(r));
            lit.arc(cx, cy, rad, from, to, width);
            c.blend_shape(&lit, Color::rgb(0xD31145));
        }
        let mut cap = Shape::new(px(r));
        cap.disc(cx, cy, rad - width * 1.5);
        c.blend_shape(&cap, Color::rgb(0x2A2A2A));
        let mut ptr = Shape::new(px(r));
        let (ps, pc) = at.sin_cos();
        ptr.segment(
            (cx + pc * rad * 0.25, cy + ps * rad * 0.25),
            (cx + pc * (rad - width * 2.0), cy + ps * (rad - width * 2.0)),
            width * 0.7,
        );
        c.blend_shape(&ptr, Color::rgb(0xFFFFFF));
    });
    if label {
        let style = crate::model::TextStyle {
            font: "Titillium Web".into(),
            style: "SemiBold".into(),
            height: text_h,
            colour: 0xFFFF_FFFF,
            h_align: crate::model::HAlign::Centre,
            v_align: crate::model::VAlign::Centre,
            case: crate::model::Case::Original,
        };
        text(
            c,
            fonts,
            &style,
            RectF::new(r.x, r.y + r.h - text_h, r.w, text_h),
            &v.text,
            clip,
        );
    }
}

/// The overlays' panel in the middle of the screen.
fn overlay_panel(r: RectF) -> RectF {
    let w = (r.w * 0.5).min(640.0);
    let h = (r.h * 0.75).min(600.0);
    RectF::new(r.x + (r.w - w) / 2.0, r.y + (r.h - h) / 2.0, w, h)
}

fn menu_names(v: &Val) -> Vec<String> {
    match v.steps {
        Some(n) if n > 1 => {
            // The replacement strings, or the step numbers.
            let mut names = Vec::new();
            for i in 0..n {
                names.push(if Some(i) == v.steps.map(|n| v.step(n)) {
                    v.text.clone()
                } else {
                    (i + 1).to_string()
                });
            }
            names
        }
        _ => vec![v.text.clone()],
    }
}

/// The menu's rows that fit in the panel, from `scroll`.
fn menu_rows(panel: RectF, n: usize, scroll: usize) -> Vec<(usize, RectF)> {
    let row_h = 56.0;
    let fit = ((panel.h - 32.0) / row_h).floor().max(1.0) as usize;
    (scroll.min(n.saturating_sub(1))..n)
        .take(fit)
        .enumerate()
        .map(|(k, i)| {
            (
                i,
                RectF::new(
                    panel.x + 16.0,
                    panel.y + 16.0 + k as f32 * row_h,
                    panel.w - 32.0,
                    row_h - 6.0,
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    struct Vals(Vec<f32>);

    impl Params for Vals {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn value(&self, i: usize) -> f32 {
            self.0[i]
        }
        fn name(&self, i: usize) -> String {
            format!("P{i}")
        }
        fn text(&self, i: usize) -> String {
            format!("{:.2}", self.0[i])
        }
    }

    use crate::testing::SKIN;

    fn view() -> View {
        let j = json::parse(SKIN).unwrap();
        let skin = Skin::from_json(&j, Path::new("/nonexistent/Plugin Skins")).unwrap();
        assert!(skin.warnings.is_empty(), "{:?}", skin.warnings);
        View::new(skin, QLinks::sequential(3))
    }

    fn sets(edits: &[Edit]) -> Vec<(usize, f32)> {
        edits
            .iter()
            .filter_map(|e| match e {
                Edit::Set(p, v) => Some((*p, (*v * 1000.0).round() / 1000.0)),
                _ => None,
            })
            .collect()
    }

    fn apply(p: &mut Vals, edits: &[Edit]) {
        for (i, v) in sets(edits) {
            p.0[i] = v;
        }
    }

    #[test]
    fn draws_the_tab_and_its_controls() {
        let mut v = view();
        let p = Vals(vec![0.5, 0.0, 0.0]);
        let mut c = SoftCanvas::new(TAB_W as i32, TAB_H as i32);
        v.draw(&p, &mut c, Rect::new(0, 0, TAB_W as i32, TAB_H as i32));
        // The first slot's background, and nothing outside the controls.
        assert_eq!(c.px[(110 * 1280 + 110) as usize] & 0xFF_FFFF, 0x10_1010);
        assert_eq!(c.px[(600 * 1280 + 1200) as usize] & 0xFF_FFFF, 0);
        // The generic knob drew its value arc in the slot.
        let lit = (100..300)
            .flat_map(|y| (100..300).map(move |x| (x, y)))
            .filter(|&(x, y)| c.px[(y * 1280 + x) as usize] & 0xFF_FFFF == 0xD3_1145)
            .count();
        assert!(lit > 50, "{lit}");
    }

    #[test]
    fn a_drag_turns_a_knob_and_brackets_the_gesture() {
        let mut v = view();
        let mut p = Vals(vec![0.5, 0.0, 0.0]);
        let mut all = v.touch(
            &p,
            Touch::Down {
                x: 200.0,
                y: 200.0,
                ms: 0,
            },
        );
        all.extend(v.touch(&p, Touch::Move { x: 200.0, y: 150.0 }));
        all.extend(v.touch(&p, Touch::Up { x: 200.0, y: 150.0 }));
        assert_eq!(all.first(), Some(&Edit::Begin(0)));
        assert_eq!(all.last(), Some(&Edit::End(0)));
        assert_eq!(sets(&all).last(), Some(&(0, 0.7)));
        apply(&mut p, &all);
        // The tap put the focus on the slot: the wheel turns it.
        assert!(v.has_focus());
        let w = v.wheel(&p, 10, false);
        assert_eq!(sets(&w), [(0, 0.8)]);
    }

    #[test]
    fn a_double_tap_opens_the_knob_overlay_and_a_tap_outside_closes_it() {
        let mut v = view();
        let p = Vals(vec![0.5, 0.25, 0.0]);
        for ms in [0, 100] {
            v.touch(
                &p,
                Touch::Down {
                    x: 500.0,
                    y: 200.0,
                    ms,
                },
            );
            v.touch(&p, Touch::Up { x: 500.0, y: 200.0 });
        }
        assert!(v.overlay_open());
        let mut c = SoftCanvas::new(TAB_W as i32, SCREEN_H as i32);
        v.draw_overlay(&p, &mut c);
        assert!(c.px.iter().any(|&px| px & 0xFF_FFFF != 0));
        // The overlay turns the slot's own parameter.
        let w = v.wheel(&p, 5, false);
        assert_eq!(sets(&w), [(1, 0.3)]);
        v.touch(
            &p,
            Touch::Down {
                x: 5.0,
                y: 5.0,
                ms: 1000,
            },
        );
        assert!(!v.overlay_open());
    }

    #[test]
    fn a_button_toggles_and_the_q_links_follow_the_tab() {
        let mut v = view();
        let mut p = Vals(vec![0.5, 0.0, 0.0]);
        let mut e = v.touch(
            &p,
            Touch::Down {
                x: 750.0,
                y: 130.0,
                ms: 0,
            },
        );
        e.extend(v.touch(&p, Touch::Up { x: 750.0, y: 130.0 }));
        assert_eq!(sets(&e), [(2, 1.0)]);
        apply(&mut p, &e);
        // Sequential Q-Links: parameter 0 is Q-Link 1, the bottom knob of
        // the first column.
        assert_eq!(v.knobs(), [None, None, None, Some(0)]);
        assert_eq!(sets(&v.qlink(&p, 3, -25, false)), [(0, 0.25)]);
        assert!(v.qlink(&p, 0, 10, false).is_empty());
        // Where a finger does something: the two slots, the knob in each,
        // and the button.
        assert_eq!(v.controls(&p).len(), 5);
    }

    #[test]
    fn the_wheel_picks_controls_in_reading_order() {
        let mut v = view();
        let p = Vals(vec![0.5, 0.5, 0.0]);
        v.focus_step(&p, 1);
        assert_eq!(sets(&v.wheel(&p, 1, false)), [(0, 0.51)]);
        v.focus_step(&p, 1);
        assert_eq!(sets(&v.wheel(&p, -1, false)), [(1, 0.49)]);
        // It stops at the last one.
        v.focus_step(&p, 5);
        assert_eq!(sets(&v.wheel(&p, 1, false)), [(1, 0.51)]);
    }
}
