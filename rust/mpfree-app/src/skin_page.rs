//! MPC plugin skins on the Synth tab ([skins design](../../../docs/dev/design/mpc-plugin-skins.md)).
//!
//! An MPC plugin (the Open MPC catalog's format) brings its own screens:
//! `Plugin Skins/TUI.json` and its pictures. When the armed track plays
//! one that has a skin, the Synth tab shows it the way its makers drew it
//! (Jay, 15 Nov: full width at MPC size): the skin fills the screen under
//! the top bar at 1280 x 628, pixel for pixel as on an MPC, with its tabs
//! in a row below. The side list and the Q-Link column step aside while it
//! shows. "MPFree page" in the tabs row goes to MPFree's own page for the
//! plugin, and "Plugin skin" on that page comes back.
//!
//! The skin is drawn by `mpfree-mpcskin` into a picture of its own
//! ([`SkinWidget`]), again only when something changed. Values and the
//! plugin's own texts for them come from the plugin process
//! (`Sandbox::values`), asked for a few times a second while the skin
//! shows; the skin's edits go out as the Synth page's own
//! `SynthAction::Set`, so undo, saving and the audio thread see nothing
//! new.

use std::path::PathBuf;
use std::time::Instant;

use mpfree_mpcskin::handle::Params;
use mpfree_mpcskin::view::{
    Edit, Touch as SkinTouch, View, MPC_ONE, SCREEN_H, TAB_H, TAB_TOP, TAB_W,
};
use mpfree_node::ParamInfo;
use mpfree_ui::canvas::Image;
use mpfree_ui::qlink::QLinkCard;
use mpfree_ui::text::{Align, FontId};
use mpfree_ui::ui::{EventKind, PaintCx, Target, Touch, Widget};
use mpfree_ui::{Color, Rect, SoftCanvas};

/// The skin itself, 1280 x 628.
pub const SKIN: &str = "mpc-skin";
/// The skin's tabs, and "MPFree page".
pub const TABS: &str = "mpc-skin-tabs";
/// "Plugin skin" on MPFree's own Synth page.
pub const CHIP: &str = "mpc-skin-chip";
/// The page the Synth tab shows while the skin is on.
pub const PAGE: &str = "Synth skin";
/// The page, for a `w` x `h` screen and the theme's gap: the top bar
/// where every page has it, the skin full width right under it at MPC
/// size, its tabs, and the hint line where every page has it.
pub fn slots(w: i32, h: i32, gap: i32) -> [(&'static str, Rect); 4] {
    let header = Rect::new(gap, gap, w - gap * 2, 60);
    let skin = Rect::new(0, header.bottom(), TAB_W as i32, TAB_H as i32);
    let hint = Rect::new(gap, h - gap - 26, w - gap * 2, 26);
    let top = skin.bottom() + gap / 2;
    let tabs = Rect::new(gap, top, w - gap * 2, (hint.y - gap / 2 - top).max(0));
    [
        ("header", header),
        (SKIN, skin),
        (TABS, tabs),
        ("hint", hint),
    ]
}
/// How often the plugin's values are asked for while the skin shows, in
/// ms, and sooner after an edit.
pub const ASK_EVERY_MS: u64 = 250;
/// An ask with no answer this long is given up (the plugin restarted).
pub const ASK_GIVE_UP_MS: u64 = 2000;
/// After an edit, the values are asked for again this much later, once
/// the edit has reached the plugin.
pub const EDIT_SETTLE_MS: u64 = 60;

/// MPC plugins' node ids and their folders, from the rack (the Synth tab
/// looks a plugin's skin up here).
pub fn folders(rack: &mpfree_plughost::rack::Rack) -> Vec<(String, PathBuf)> {
    rack.entries()
        .iter()
        .filter(|e| e.node_id.starts_with("mpc:"))
        .map(|e| (e.node_id.to_string(), e.file.clone()))
        .collect()
}

/// The plugin's parameters as the skin reads them: names, values (0 to
/// 1) and the plugin's text for each value.
#[derive(Clone, Debug, Default)]
pub struct Values {
    pub ids: Vec<u16>,
    pub names: Vec<String>,
    pub values: Vec<f32>,
    pub texts: Vec<String>,
}

impl Values {
    pub fn new(params: &[ParamInfo], values: &[f32]) -> Values {
        Values {
            ids: params.iter().map(|p| p.id).collect(),
            names: params.iter().map(|p| p.name.to_string()).collect(),
            values: values.to_vec(),
            texts: Vec::new(),
        }
    }
}

impl Params for Values {
    fn count(&self) -> usize {
        self.names.len()
    }
    fn value(&self, i: usize) -> f32 {
        self.values.get(i).copied().unwrap_or(0.0)
    }
    fn name(&self, i: usize) -> String {
        self.names.get(i).cloned().unwrap_or_default()
    }
    fn text(&self, i: usize) -> String {
        self.texts.get(i).cloned().unwrap_or_default()
    }
}

/// The skin on screen: a [`View`] and the picture it last drew.
pub struct SkinWidget {
    pub view: View,
    pub vals: Values,
    image: Image,
    edits: Vec<Edit>,
    start: Instant,
}

impl SkinWidget {
    pub fn new(view: View, vals: Values) -> SkinWidget {
        let mut w = SkinWidget {
            view,
            vals,
            image: Image::new(TAB_W as i32, TAB_H as i32),
            edits: Vec::new(),
            start: Instant::now(),
        };
        w.render();
        w
    }

    /// Draws the skin again into its picture (and the open overlay over
    /// it, as the MPC draws it on the whole screen).
    pub fn render(&mut self) {
        let (w, h) = (TAB_W as i32, TAB_H as i32);
        let mut c = SoftCanvas::new(w, h);
        self.view.draw(&self.vals, &mut c, Rect::new(0, 0, w, h));
        if self.view.overlay_open() {
            let top = TAB_TOP as i32;
            let mut screen = SoftCanvas::new(w, SCREEN_H as i32);
            let at = (top * w) as usize;
            screen.px[at..at + c.px.len()].copy_from_slice(&c.px);
            self.view.draw_overlay(&self.vals, &mut screen);
            c.px.copy_from_slice(&screen.px[at..at + (w * h) as usize]);
        }
        self.image.px = c.px;
    }

    /// The skin's edits since the last call, as the Synth page's own:
    /// parameter id and value.
    pub fn take_edits(&mut self) -> Vec<(u16, f32)> {
        let ids = &self.vals.ids;
        self.edits
            .drain(..)
            .filter_map(|e| match e {
                Edit::Set(i, v) => ids.get(i).map(|&id| (id, v)),
                _ => None,
            })
            .collect()
    }

    /// Edits from the skin's own controls, the wheel or a Q-Link. The
    /// values are kept here at once, so the skin moves with the finger
    /// before the plugin answers.
    pub fn push(&mut self, edits: Vec<Edit>) {
        for e in &edits {
            if let Edit::Set(i, v) = *e {
                if let Some(slot) = self.vals.values.get_mut(i) {
                    *slot = v;
                }
            }
        }
        self.edits.extend(edits);
    }

    /// The plugin's values and texts now.
    pub fn set_values(&mut self, values: Vec<f32>, texts: Vec<String>) {
        if values.len() == self.vals.names.len() {
            self.vals.values = values;
            self.vals.texts = texts;
        }
    }

    /// The Q-Link column's four cards: the knobs the skin's Q-Links file
    /// gives the tab's bank now, top first.
    pub fn qlink_cards(&self) -> [QLinkCard; 4] {
        let mut cards: [QLinkCard; 4] = Default::default();
        for (card, p) in cards.iter_mut().zip(self.view.knobs()) {
            if let Some(p) = p.filter(|&p| p < self.vals.count()) {
                *card = QLinkCard::new(
                    &self.vals.name(p),
                    "Skin",
                    &self.vals.text(p),
                    self.vals.value(p),
                );
            }
        }
        cards
    }
}

impl Widget for SkinWidget {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        cx.canvas.fill_rect(r, Color::rgb(0));
        // Pixel for pixel, from the top left; a smaller area shows the
        // top left of it.
        let clip = cx.canvas.clip();
        cx.canvas.set_clip(clip.intersect(r));
        cx.canvas.draw_image(r.x, r.y, &self.image);
        cx.canvas.set_clip(clip);
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let (x, y) = ((x - r.x) as f32, (y - r.y) as f32);
        let t = match phase {
            Touch::Down => SkinTouch::Down {
                x,
                y,
                ms: self.start.elapsed().as_millis() as u64,
            },
            Touch::Move => SkinTouch::Move { x, y },
            Touch::Up | Touch::Cancel => SkinTouch::Up { x, y },
        };
        let edits = self.view.touch(&self.vals, t);
        self.push(edits);
        self.render();
        Some(EventKind::Value(0.0))
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        self.view
            .controls(&self.vals)
            .into_iter()
            .enumerate()
            .map(|(i, c)| {
                let rect = Rect::new(r.x + c.x as i32, r.y + c.y as i32, c.w as i32, c.h as i32);
                Target::drag(
                    SKIN,
                    i,
                    format!("skin control {}", i + 1),
                    rect.intersect(r),
                )
            })
            .filter(|t| !t.rect.is_empty())
            .collect()
    }
}

/// Where button `i` of `n` (and "MPFree page", `i == n`) sits in `r`, in
/// one row.
fn tab_rects(n: usize, r: Rect) -> Vec<Rect> {
    let back_w = 200;
    let gap = 6;
    let room = (r.w - back_w - gap * 2).max(1);
    let w = (room / n.max(1) as i32).min(220);
    let mut out: Vec<Rect> = (0..n)
        .map(|i| Rect::new(r.x + gap + i as i32 * w, r.y + gap, w - gap, r.h - gap * 2))
        .collect();
    out.push(Rect::new(
        r.right() - back_w - gap,
        r.y + gap,
        back_w,
        r.h - gap * 2,
    ));
    out
}

/// The skin's tabs as the MPC lays them out: one button per function key
/// (a skin can put several sub-tabs on one key; a tap on the key's button
/// steps through them, as a second press of the key does on an MPC), and
/// "MPFree page" on the right.
#[derive(Clone, Debug, Default)]
pub struct SkinTabs {
    /// Each key's tabs (index, name), first sub-tab first.
    pub keys: Vec<(usize, Vec<(usize, String)>)>,
    /// The tab showing (its index in the skin).
    pub current: usize,
    pressed: Option<usize>,
}

impl SkinTabs {
    /// From the skin's tabs: name, function key and sub-tab each.
    pub fn new(tabs: &[(String, usize, usize)]) -> SkinTabs {
        // Each key's (sub-tab, tab index, name).
        type Subs = Vec<(usize, usize, String)>;
        let mut keys: Vec<(usize, Subs)> = Vec::new();
        for (i, (name, key, sub)) in tabs.iter().enumerate() {
            match keys.iter_mut().find(|k| k.0 == *key) {
                Some(k) => k.1.push((*sub, i, name.clone())),
                None => keys.push((*key, vec![(*sub, i, name.clone())])),
            }
        }
        keys.sort_by_key(|k| k.0);
        SkinTabs {
            keys: keys
                .into_iter()
                .map(|(key, mut subs)| {
                    subs.sort_by_key(|s| (s.0, s.1));
                    (key, subs.into_iter().map(|(_, i, n)| (i, n)).collect())
                })
                .collect(),
            current: 0,
            pressed: None,
        }
    }

    /// Button `k`'s label: the sub-tab showing (or the key's first), and
    /// which of how many when the key has several.
    pub fn label(&self, k: usize) -> String {
        let Some((_, subs)) = self.keys.get(k) else {
            return "MPFree page".to_string();
        };
        let at = subs.iter().position(|s| s.0 == self.current);
        let (i, name) = (at.unwrap_or(0), &subs[at.unwrap_or(0)].1);
        if subs.len() > 1 {
            format!("{name}  {}/{}", i + 1, subs.len())
        } else {
            name.clone()
        }
    }

    fn at(&self, x: i32, y: i32, r: Rect) -> Option<usize> {
        tab_rects(self.keys.len(), r)
            .iter()
            .position(|b| b.contains(x, y))
    }
}

impl Widget for SkinTabs {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let n = self.keys.len();
        for (k, b) in tab_rects(n, r).iter().enumerate() {
            let on = self
                .keys
                .get(k)
                .is_some_and(|key| key.1.iter().any(|s| s.0 == self.current));
            let fill = if on || self.pressed == Some(k) {
                t.raised
            } else {
                t.surface
            };
            cx.canvas.fill_round_rect(*b, t.radius.min(10), fill);
            if on {
                cx.canvas
                    .stroke_round_rect(*b, t.radius.min(10), 2, t.accent);
            }
            let colour = if on { t.text } else { t.text_dim };
            let label = self.label(k);
            let room = b.inset(8);
            // The smaller size when the name doesn't fit (many keys).
            let px = if cx.measure(FontId::Bold, t.size_body, &label) > room.w {
                t.size_small
            } else {
                t.size_body
            };
            cx.text(FontId::Bold, px, room, Align::Center, colour, &label);
        }
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {
                self.pressed = self.at(x, y, r);
                None
            }
            Touch::Move => None,
            Touch::Cancel => {
                self.pressed = None;
                None
            }
            Touch::Up => {
                let hit = self.pressed.take().filter(|&k| self.at(x, y, r) == Some(k));
                hit.map(EventKind::Tab)
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        tab_rects(self.keys.len(), r)
            .into_iter()
            .enumerate()
            .map(|(k, b)| Target::tap(TABS, k, self.label(k), b))
            .collect()
    }
}

/// "Plugin skin" on MPFree's own Synth page: back to the plugin's skin.
#[derive(Clone, Debug, Default)]
pub struct SkinChip;

impl Widget for SkinChip {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let b = r.inset(4);
        cx.canvas.fill_round_rect(b, t.radius.min(10), t.surface);
        cx.canvas
            .stroke_round_rect(b, t.radius.min(10), 2, t.accent);
        cx.text(
            FontId::Bold,
            t.size_body,
            b.inset(8),
            Align::Center,
            t.text,
            "Plugin skin",
        );
    }

    fn touch(&mut self, _finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        (phase == Touch::Up && r.contains(x, y)).then_some(EventKind::Tab(0))
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        vec![Target::tap(CHIP, 0, "Plugin skin", r.inset(4))]
    }
}

/// MPFree's own Synth page with "Plugin skin" under the instrument card,
/// for a plugin that has a skin (on top of the pad player's own change).
pub fn synth_with_chip(spec: &str) -> String {
    crate::play_page::synth_with_pads(spec).replacen(
        "synth-inst:72,",
        &format!("synth-inst:72, {CHIP}:52,"),
        1,
    )
}

/// What the Synth tab knows about the skin of the plugin the armed track
/// plays.
pub struct SkinState {
    pub id: String,
    /// The skin shows (else MPFree's own page, with the chip).
    pub on: bool,
    /// Asked the plugin for its values, at this time; no answer yet.
    pub asked: Option<Instant>,
    pub last_ask: Option<Instant>,
    /// An edit since the last answer: ask again soon.
    pub stale: bool,
    /// The last edit: an answer to an ask made before it is out of date.
    pub edited: Option<Instant>,
}

impl SkinState {
    pub fn new(id: &str) -> SkinState {
        SkinState {
            id: id.to_string(),
            on: true,
            asked: None,
            last_ask: None,
            stale: true,
            edited: None,
        }
    }

    /// An edit went out: ask for the values again once it has reached the
    /// plugin.
    pub fn edited_now(&mut self, now: Instant) {
        self.edited = Some(now);
        self.stale = true;
    }

    /// An answer came: true to take its values (it was asked after the
    /// last edit).
    pub fn answer(&mut self) -> bool {
        let fresh = match (self.asked, self.edited) {
            (Some(a), Some(e)) => a > e,
            _ => true,
        };
        self.asked = None;
        fresh
    }

    /// True when it's time to ask the plugin for its values (and marks
    /// the ask).
    pub fn ask_now(&mut self, now: Instant) -> bool {
        if let Some(a) = self.asked {
            if now.duration_since(a).as_millis() as u64 >= ASK_GIVE_UP_MS {
                self.asked = None;
            } else {
                return false;
            }
        }
        // An edit takes a block or two to reach the plugin.
        let settling = self
            .edited
            .is_some_and(|e| (now.duration_since(e).as_millis() as u64) < EDIT_SETTLE_MS);
        let due = self
            .last_ask
            .is_none_or(|l| now.duration_since(l).as_millis() as u64 >= ASK_EVERY_MS);
        if !settling && (self.stale || due) {
            self.asked = Some(now);
            self.last_ask = Some(now);
            self.stale = false;
            true
        } else {
            false
        }
    }
}

/// Reads plugin `id`'s skin from its folder, for the MPC One. None when it
/// has none (MPFree's own page shows, as the MPC shows its default pages)
/// or it doesn't read (said in the log).
pub fn load(
    id: &str,
    folder: &std::path::Path,
    params: &[ParamInfo],
    values: &[f32],
) -> Result<Option<SkinWidget>, String> {
    let Some((skin, qlinks)) = mpfree_mpcskin::view::load(folder, &MPC_ONE, params.len())
        .map_err(|e| format!("{id}: skin doesn't read: {e}"))?
    else {
        return Ok(None);
    };
    if skin.tabs.is_empty() {
        return Err(format!("{id}: skin has no tabs"));
    }
    Ok(Some(SkinWidget::new(
        View::new(skin, qlinks),
        Values::new(params, values),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_puts_the_skin_right_under_the_top_bar() {
        let s = slots(1280, 800, 12);
        assert_eq!(s[0], ("header", Rect::new(12, 12, 1256, 60)));
        assert_eq!(s[1], (SKIN, Rect::new(0, 72, 1280, 628)));
        assert_eq!(s[3], ("hint", Rect::new(12, 762, 1256, 26)));
        let tabs = s[2].1;
        assert!(
            tabs.y >= 700 && tabs.bottom() <= 762 && tabs.h >= 44,
            "{tabs:?}"
        );
    }

    #[test]
    fn one_button_per_function_key_steps_its_sub_tabs() {
        let tabs: Vec<(String, usize, usize)> = [
            ("Osc", 0, 0),
            ("Osc 2", 0, 1),
            ("Filter", 1, 0),
            ("Env", 2, 0),
        ]
        .iter()
        .map(|&(n, k, s)| (n.to_string(), k, s))
        .collect();
        let mut t = SkinTabs::new(&tabs);
        assert_eq!(t.keys.len(), 3);
        assert_eq!(t.label(0), "Osc  1/2");
        t.current = 1;
        assert_eq!(t.label(0), "Osc 2  2/2");
        assert_eq!(t.label(1), "Filter");
        assert_eq!(t.label(3), "MPFree page");
        let r = Rect::new(12, 706, 1256, 50);
        let b = tab_rects(3, r);
        assert_eq!(b.len(), 4);
        for x in &b[..3] {
            assert!(x.right() <= b[3].x && x.w >= 100);
        }
    }

    #[test]
    fn the_chip_goes_under_the_instrument_card() {
        let spec = synth_with_chip(mpfree_synth_ui::view::LAYOUT);
        assert!(spec.contains("synth-inst:72, mpc-skin-chip:52,"), "{spec}");
    }

    #[test]
    fn values_are_asked_for_after_an_edit_and_now_and_then() {
        let t0 = Instant::now();
        let mut s = SkinState::new("mpc:AwXn.density2");
        assert!(s.ask_now(t0));
        assert!(!s.ask_now(t0), "one ask at a time");
        s.asked = None;
        assert!(!s.ask_now(t0 + std::time::Duration::from_millis(10)));
        s.stale = true;
        assert!(s.ask_now(t0 + std::time::Duration::from_millis(20)));
        s.asked = None;
        assert!(s.ask_now(t0 + std::time::Duration::from_millis(300)));
        // An ask nobody answered is given up.
        assert!(s.ask_now(t0 + std::time::Duration::from_millis(2400)));
        // An answer to an ask from before an edit is out of date, and the
        // next ask waits for the edit to reach the plugin.
        s.edited_now(t0 + std::time::Duration::from_millis(2410));
        assert!(!s.answer());
        assert!(!s.ask_now(t0 + std::time::Duration::from_millis(2420)));
        assert!(s.ask_now(t0 + std::time::Duration::from_millis(2480)));
        assert!(s.answer());
    }
}
