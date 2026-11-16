//! The Plugins place (Jay, 15 Nov 23:36Z; layout spec section 3, the
//! browser): MPC plugins from the Open MPC catalog, the ones installed and
//! the release zips copied to the stick, with the selected one's picture,
//! what it is and Install. The browser draws the place's list and selected
//! column from a [`PluginsScreen`] the app gives it
//! ([`crate::Browser::set_plugins`]) and hands every touch, dial turn and
//! button back as [`PluginInput`]s ([`crate::BrowseAction::Plugins`]): the
//! app keeps the catalog, downloads and installs (`mpfree-mpcstore`).

use mpfree_ui::draw;
use mpfree_ui::{Align, Color, FontId, Image, PaintCx, Rect};
use std::sync::Arc;

/// The list's three views, at the top of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginsView {
    /// The catalog's plugins that run on this MPC.
    Catalog,
    Installed,
    /// Release zips copied to the stick's plugin folder.
    OnStick,
}

pub const VIEWS: [PluginsView; 3] = [
    PluginsView::Catalog,
    PluginsView::Installed,
    PluginsView::OnStick,
];

impl PluginsView {
    pub fn label(self) -> &'static str {
        match self {
            PluginsView::Catalog => "Catalog",
            PluginsView::Installed => "Installed",
            PluginsView::OnStick => "On the stick",
        }
    }
}

/// The chips under the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KindFilter {
    All,
    Instruments,
    Effects,
}

pub const KIND_FILTERS: [KindFilter; 3] = [
    KindFilter::All,
    KindFilter::Instruments,
    KindFilter::Effects,
];

impl KindFilter {
    pub fn label(self) -> &'static str {
        match self {
            KindFilter::All => "All",
            KindFilter::Instruments => "Instruments",
            KindFilter::Effects => "Effects",
        }
    }
}

/// How a word is coloured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Dim,
    Plain,
    Good,
    Warn,
    Accent,
    Bad,
}

fn ink(cx: &PaintCx, tone: Tone) -> Color {
    let t = cx.theme;
    match tone {
        Tone::Dim => t.text_dim,
        Tone::Plain => t.text,
        Tone::Good => t.good,
        Tone::Warn => t.warn,
        Tone::Accent => t.accent,
        Tone::Bad => t.bad,
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginRow {
    pub name: String,
    pub maker: String,
    pub instrument: bool,
    /// Its state (Installed, Update 1.6.1, Zip on the stick) or its kind.
    pub tag: String,
    pub tone: Tone,
    pub favourite: bool,
}

/// The selected column's buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginButton {
    Install,
    InstallZip,
    Update,
    Pictures,
    Favourite,
    Remove,
    /// Remove, asked again: a second tap removes.
    ConfirmRemove,
    DeleteZip,
    ConfirmDeleteZip,
    /// Stops a download.
    Cancel,
    /// Fetches the catalog again.
    Refresh,
}

impl PluginButton {
    pub fn label(self) -> &'static str {
        match self {
            PluginButton::Install => "Install",
            PluginButton::InstallZip => "Install from zip",
            PluginButton::Update => "Update",
            PluginButton::Pictures => "Pictures",
            PluginButton::Favourite => "Favourite",
            PluginButton::Remove => "Remove",
            PluginButton::ConfirmRemove => "Tap again to remove",
            PluginButton::DeleteZip => "Delete zip",
            PluginButton::ConfirmDeleteZip => "Tap again to delete",
            PluginButton::Cancel => "Cancel",
            PluginButton::Refresh => "Update list",
        }
    }

    fn primary(self) -> bool {
        matches!(
            self,
            PluginButton::Install | PluginButton::InstallZip | PluginButton::Update
        )
    }

    fn danger(self) -> bool {
        matches!(
            self,
            PluginButton::Remove
                | PluginButton::ConfirmRemove
                | PluginButton::DeleteZip
                | PluginButton::ConfirmDeleteZip
        )
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PluginDetail {
    pub title: String,
    pub maker: String,
    /// Label, value and the value's colour.
    pub lines: Vec<(String, String, Tone)>,
    pub summary: String,
    /// One small line over the buttons (what Install will do, or what
    /// went wrong).
    pub note: String,
    pub note_tone: Tone,
    /// The plugin's own screen or the catalog's picture of it.
    pub picture: Option<Arc<Image>>,
    /// Over the picture's corner: `"2 / 11"`.
    pub picture_label: String,
    /// What the picture's place says while there is none.
    pub picture_empty: String,
    pub buttons: Vec<PluginButton>,
    pub favourite: bool,
    /// A download's progress, 0 to 1, drawn as a bar over the buttons.
    pub progress: Option<f32>,
}

/// Everything the place draws.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginsScreen {
    pub view: PluginsView,
    pub rows: Vec<PluginRow>,
    pub selected: Option<usize>,
    pub top: usize,
    pub filter: KindFilter,
    pub sort: String,
    /// The text being looked for, if any.
    pub search: Option<String>,
    /// The line under the list.
    pub status: String,
    pub status_tone: Tone,
    /// Said in the list when it is empty.
    pub empty: String,
    /// The list's own button at its head (Update list), if any.
    pub list_button: Option<PluginButton>,
    pub detail: PluginDetail,
}

impl Default for PluginsScreen {
    fn default() -> Self {
        PluginsScreen {
            view: PluginsView::Catalog,
            rows: Vec::new(),
            selected: None,
            top: 0,
            filter: KindFilter::All,
            sort: "Most used".into(),
            search: None,
            status: String::new(),
            status_tone: Tone::Dim,
            empty: "Reading…".into(),
            list_button: None,
            detail: PluginDetail::default(),
        }
    }
}

/// What a touch, the dial or a button meant, for the app.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginInput {
    /// The place was opened: list what is there.
    Shown,
    View(PluginsView),
    Filter(KindFilter),
    Sort,
    /// The Search chip: the browser's keyboard opens, and gives
    /// [`PluginInput::SearchText`] when done.
    Search,
    SearchText(String),
    /// A tap on a row.
    Row(usize),
    /// The list dragged by this many rows.
    Scroll(i32),
    Button(PluginButton),
    /// The data dial, its push, - and +.
    Dial(i32),
    Push,
    Back,
    Enter,
    /// The picture tapped: the next one.
    Picture,
}

const HEAD_H: i32 = 56;
const FOOT_H: i32 = 52;
const STATUS_H: i32 = 30;
const BUTTON_H: i32 = 56;
/// A list row's height, as the browser's.
pub const ROW_H: i32 = crate::view::ROW_H;
/// The picture's height in the selected column (1280 x 628 scaled to the
/// column's width, 396 px).
pub const PICTURE_H: i32 = 194;

struct Geo {
    views: Vec<(PluginsView, Rect)>,
    list_button: Option<Rect>,
    head: Rect,
    rows: Rect,
    filters: Vec<(KindFilter, Rect)>,
    search: Rect,
    sort: Rect,
    status: Rect,
    picture: Rect,
    body: Rect,
    buttons: Vec<(PluginButton, Rect)>,
    note: Rect,
}

impl Geo {
    fn new(list: Rect, detail: Rect, s: &PluginsScreen) -> Geo {
        // Widths from the number of letters, as the browser's own buttons,
        // so a touch finds what was drawn without the fonts.
        let word_w = |label: &str| 8 * label.chars().count() as i32;
        let inner = list.inset(12);
        let (head, body) = inner.take_top(HEAD_H);
        let (status, body) = body.take_bottom(STATUS_H);
        let (foot, rows) = body.take_bottom(FOOT_H);
        // The views as a row of buttons at the head's right; the list's
        // own button, if any, left of them.
        let mut views = Vec::new();
        let mut right = head;
        for v in VIEWS.iter().rev() {
            let w = word_w(v.label()) + 20;
            let (br, r2) = right.take_right(w);
            views.push((*v, br.inset_xy(0, 6)));
            right = r2.take_right(8).1;
        }
        views.reverse();
        let list_button = s.list_button.map(|b| {
            let w = word_w(b.label()) + 20;
            right.take_right(w).0.inset_xy(0, 6)
        });
        let chip_w = |label: &str| 26 + 10 * label.len() as i32;
        let mut filters = Vec::new();
        let mut x = foot.x;
        for f in KIND_FILTERS {
            let w = chip_w(f.label());
            filters.push((f, Rect::new(x, foot.y + 8, w, foot.h - 12)));
            x += w + 8;
        }
        let (sort, rest) = foot.take_right(chip_w(&s.sort));
        let sort = Rect::new(sort.x, foot.y + 8, sort.w, foot.h - 12);
        let (_, rest) = rest.take_right(8);
        let (search, _) = rest.take_right(chip_w("Search"));
        let search = Rect::new(search.x, foot.y + 8, search.w, foot.h - 12);
        filters.retain(|(_, fr)| fr.right() <= search.x - 8);
        // The selected column.
        let db = draw::card_head(detail).1;
        let (picture, body) = db.take_top(PICTURE_H.min(db.w * 628 / 1280));
        let body = body.take_top(14).1;
        let n = s.detail.buttons.len() as i32;
        let lines = (n + 1) / 2;
        let (btns, body) = body.take_bottom(lines * (BUTTON_H + 8));
        let (note, body) = body.take_bottom(if s.detail.progress.is_some() { 30 } else { 22 });
        let bw = (btns.w - 8) / 2;
        let buttons = s
            .detail
            .buttons
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (c, l) = (i as i32 % 2, i as i32 / 2);
                (
                    *b,
                    Rect::new(
                        btns.x + c * (bw + 8),
                        btns.y + 8 + l * (BUTTON_H + 8),
                        bw,
                        BUTTON_H,
                    ),
                )
            })
            .collect();
        Geo {
            views,
            list_button,
            head,
            rows,
            filters,
            search,
            sort,
            status,
            picture,
            body,
            buttons,
            note,
        }
    }

    fn visible(&self) -> usize {
        (self.rows.h / ROW_H).max(1) as usize
    }

    fn row_rect(&self, k: usize) -> Rect {
        Rect::new(
            self.rows.x,
            self.rows.y + k as i32 * ROW_H,
            self.rows.w - 8,
            ROW_H - 4,
        )
    }
}

/// The rows that fit in the list drawn in `list`.
pub fn rows_in(list: Rect) -> usize {
    let inner = list.inset(12);
    let h = inner.h - HEAD_H - STATUS_H - FOOT_H;
    (h / ROW_H).max(1) as usize
}

/// Draws the place's list in `list` and the selected plugin in `detail`.
pub fn paint(cx: &mut PaintCx, list: Rect, detail: Rect, s: &PluginsScreen) {
    let g = Geo::new(list, detail, s);
    let t = cx.theme.clone();
    // List.
    cx.panel(list);
    let title = match &s.search {
        Some(q) => format!("Plugins: \u{201c}{q}\u{201d}"),
        None => "Plugins".to_string(),
    };
    let first = g.list_button.map_or(g.views[0].1.x, |b| b.x);
    cx.text(
        FontId::Bold,
        t.size_title,
        Rect::new(g.head.x + 4, g.head.y, first - g.head.x - 12, g.head.h),
        Align::Left,
        t.text,
        &title,
    );
    if let (Some(b), Some(br)) = (s.list_button, g.list_button) {
        cx.canvas.fill_round_rect(br, 10, t.raised);
        cx.text(
            FontId::Sans,
            t.size_small,
            br,
            Align::Center,
            t.text,
            b.label(),
        );
    }
    for (v, vr) in &g.views {
        let on = *v == s.view;
        cx.canvas
            .fill_round_rect(*vr, 10, if on { t.accent } else { t.raised });
        cx.text(
            if on { FontId::Bold } else { FontId::Sans },
            t.size_small,
            *vr,
            Align::Center,
            if on { t.on_accent } else { t.text },
            v.label(),
        );
    }
    cx.canvas.fill_rect(
        Rect::new(g.head.x, g.head.bottom() - 1, g.head.w, 1),
        t.outline,
    );
    let vis = g.visible();
    if s.rows.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_body,
            g.rows,
            Align::Center,
            t.text_faint,
            &s.empty,
        );
    }
    for (k, row) in s.rows.iter().enumerate().skip(s.top).take(vis) {
        paint_row(cx, g.row_rect(k - s.top), row, s.selected == Some(k));
    }
    if s.rows.len() > vis {
        let track = Rect::new(g.rows.right() - 4, g.rows.y, 4, vis as i32 * ROW_H);
        cx.canvas.fill_round_rect(track, 2, t.outline);
        let h = (track.h * vis as i32 / s.rows.len() as i32).max(24);
        let y = track.y
            + (track.h - h) * s.top.min(s.rows.len() - vis) as i32 / (s.rows.len() - vis) as i32;
        cx.canvas
            .fill_round_rect(Rect::new(track.x, y, 4, h), 2, t.text_dim);
    }
    for (f, fr) in &g.filters {
        chip(cx, *fr, f.label(), *f == s.filter);
    }
    chip(cx, g.search, "Search", s.search.is_some());
    chip(cx, g.sort, &s.sort, false);
    cx.text(
        FontId::Sans,
        t.size_small,
        g.status.inset_xy(4, 0),
        Align::Left,
        ink(cx, s.status_tone),
        &s.status,
    );
    // The selected plugin.
    draw::card(cx, detail, "Selected", "");
    let d = &s.detail;
    match &d.picture {
        Some(img) => {
            let x = g.picture.x + (g.picture.w - img.w).max(0) / 2;
            cx.canvas.draw_image(x, g.picture.y, img);
            if !d.picture_label.is_empty() {
                let w = cx.measure(FontId::Sans, t.size_small, &d.picture_label) + 16;
                let lr = Rect::new(g.picture.right() - w - 6, g.picture.y + 6, w, 24);
                cx.canvas.fill_round_rect(lr, 12, t.bg.with_alpha(190));
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    lr,
                    Align::Center,
                    t.text,
                    &d.picture_label,
                );
            }
        }
        None => {
            cx.canvas.fill_round_rect(g.picture, 10, t.raised);
            cx.text(
                FontId::Sans,
                t.size_small,
                g.picture,
                Align::Center,
                t.text_faint,
                &d.picture_empty,
            );
        }
    }
    let (title, rest) = g.body.take_top(34);
    let tw = cx.text(
        FontId::Bold,
        t.size_title,
        title,
        Align::Left,
        t.text,
        &d.title,
    );
    if !d.maker.is_empty() {
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(
                title.x + tw + 12,
                title.y,
                (title.w - tw - 12).max(0),
                title.h,
            ),
            Align::Left,
            t.text_dim,
            &format!("by {}", d.maker),
        );
    }
    let mut body = rest.take_top(4).1;
    for (label, value, tone) in &d.lines {
        if body.h < 28 {
            break;
        }
        let (line, rest) = body.take_top(30);
        let (l, v) = line.take_left(110);
        cx.text(
            FontId::Sans,
            t.size_small,
            l,
            Align::Left,
            t.text_dim,
            label,
        );
        let c = ink(cx, *tone);
        let c = if *tone == Tone::Dim { t.text } else { c };
        cx.text(FontId::Sans, t.size_body, v, Align::Left, c, value);
        body = rest;
    }
    let body = body.take_top(6).1;
    paragraph(cx, body, &d.summary, t.text_dim);
    let mut note = g.note;
    if let Some(p) = d.progress {
        let (bar, rest) = note.take_bottom(6);
        note = rest;
        cx.canvas.fill_round_rect(bar, 3, t.outline);
        let w = (bar.w as f32 * p.clamp(0.0, 1.0)) as i32;
        if w > 0 {
            cx.canvas
                .fill_round_rect(Rect::new(bar.x, bar.y, w.max(6), bar.h), 3, t.accent);
        }
    }
    cx.text(
        FontId::Sans,
        t.size_small * 0.86,
        note,
        Align::Left,
        ink(cx, d.note_tone),
        &d.note,
    );
    for (b, br) in &g.buttons {
        let lit = b.primary() || (*b == PluginButton::Favourite && d.favourite);
        button(cx, *br, b.label(), lit, b.danger());
    }
}

/// What a touch at (x, y) meant, or None.
pub fn hit(list: Rect, detail: Rect, s: &PluginsScreen, x: i32, y: i32) -> Option<PluginInput> {
    let g = Geo::new(list, detail, s);
    if let Some((v, _)) = g.views.iter().find(|(_, r)| r.contains(x, y)) {
        return Some(PluginInput::View(*v));
    }
    if let (Some(b), Some(br)) = (s.list_button, g.list_button) {
        if br.contains(x, y) {
            return Some(PluginInput::Button(b));
        }
    }
    if let Some((f, _)) = g.filters.iter().find(|(_, r)| r.contains(x, y)) {
        return Some(PluginInput::Filter(*f));
    }
    if g.search.contains(x, y) {
        return Some(PluginInput::Search);
    }
    if g.sort.contains(x, y) {
        return Some(PluginInput::Sort);
    }
    if let Some((b, _)) = g.buttons.iter().find(|(_, r)| r.contains(x, y)) {
        return Some(PluginInput::Button(*b));
    }
    if g.picture.contains(x, y) {
        return Some(PluginInput::Picture);
    }
    if g.rows.contains(x, y) {
        let k = ((y - g.rows.y) / ROW_H) as usize;
        let i = s.top + k;
        if k < g.visible() && i < s.rows.len() {
            return Some(PluginInput::Row(i));
        }
    }
    None
}

/// The list's rows' area (where a drag scrolls).
pub fn rows_rect(list: Rect, detail: Rect, s: &PluginsScreen) -> Rect {
    Geo::new(list, detail, s).rows
}

/// The place's controls, for the control audit and the smoke tests.
pub fn targets(list: Rect, detail: Rect, s: &PluginsScreen) -> Vec<mpfree_ui::Target> {
    use mpfree_ui::Target;
    let g = Geo::new(list, detail, s);
    let mut out = Vec::new();
    for (i, (v, r)) in g.views.iter().enumerate() {
        out.push(Target::tap("plugins-view", i, v.label(), *r));
    }
    if let (Some(b), Some(r)) = (s.list_button, g.list_button) {
        out.push(Target::tap("plugins-button", b as usize, b.label(), r));
    }
    for (i, (f, r)) in g.filters.iter().enumerate() {
        out.push(Target::tap("plugins-filter", i, f.label(), *r));
    }
    out.push(Target::tap("plugins-search", 0, "Search", g.search));
    out.push(Target::tap("plugins-sort", 0, "Sort", g.sort));
    out.push(Target::tap("plugins-picture", 0, "Picture", g.picture));
    for (b, r) in &g.buttons {
        out.push(Target::tap("plugins-button", *b as usize, b.label(), *r));
    }
    let shown = g.visible().min(s.rows.len().saturating_sub(s.top));
    for k in 0..shown {
        out.push(Target::tap(
            "plugins-row",
            s.top + k,
            s.rows[s.top + k].name.clone(),
            g.row_rect(k),
        ));
    }
    out
}

fn paint_row(cx: &mut PaintCx, rr: Rect, row: &PluginRow, selected: bool) {
    let t = cx.theme.clone();
    if selected {
        cx.canvas.fill_round_rect(rr, 10, t.raised);
        cx.canvas
            .fill_round_rect(Rect::new(rr.x, rr.y + 12, 4, rr.h - 24), 2, t.accent);
    }
    let inner = rr.inset_xy(14, 0);
    let (icon, rest) = inner.take_left(30);
    let swatch = Rect::new(icon.x + 5, icon.y + icon.h / 2 - 10, 20, 20);
    cx.canvas
        .fill_round_rect(swatch, 5, if row.instrument { t.accent2 } else { t.good });
    let tag_w = if row.tag.is_empty() {
        0
    } else {
        cx.measure(FontId::Bold, t.size_small, &row.tag)
            .min(rest.w / 2)
            + 16
    };
    let (tag, rest) = rest.take_right(tag_w);
    let (star, rest) = rest.take_right(if row.favourite { 26 } else { 0 });
    if row.favourite {
        let (x, y) = (star.x + star.w / 2, star.y + star.h / 2);
        crate::view::star_shape(cx, x, y, 8, t.warn);
    }
    let name = rest.inset_xy(10, 0);
    let nw = cx.text(
        FontId::Bold,
        t.size_body,
        name,
        Align::Left,
        t.text,
        &row.name,
    );
    if !row.maker.is_empty() && nw + 40 < name.w {
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(name.x + nw + 10, name.y + 1, name.w - nw - 10, name.h),
            Align::Left,
            t.text_dim,
            &row.maker,
        );
    }
    let c = ink(cx, row.tone);
    cx.text(
        if row.tone == Tone::Dim {
            FontId::Sans
        } else {
            FontId::Bold
        },
        t.size_small,
        tag,
        Align::Right,
        c,
        &row.tag,
    );
}

fn button(cx: &mut PaintCx, r: Rect, label: &str, lit: bool, danger: bool) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(r, 10, if lit { t.accent } else { t.raised });
    let ink = if lit {
        t.on_accent
    } else if danger {
        t.bad
    } else {
        t.text
    };
    cx.text(
        if lit { FontId::Bold } else { FontId::Sans },
        t.size_body,
        r,
        Align::Center,
        ink,
        label,
    );
}

fn chip(cx: &mut PaintCx, r: Rect, label: &str, on: bool) {
    let t = cx.theme.clone();
    if on {
        cx.canvas.fill_round_rect(r, r.h / 2, t.accent);
    } else {
        cx.canvas.stroke_round_rect(r, r.h / 2, 1, t.outline);
    }
    cx.text(
        FontId::Sans,
        t.size_small,
        r,
        Align::Center,
        if on { t.on_accent } else { t.text_dim },
        label,
    );
}

/// Words wrapped to `r`'s width, as many lines as fit.
fn paragraph(cx: &mut PaintCx, r: Rect, text: &str, c: Color) {
    let px = cx.theme.size_small;
    let lh = 21;
    let mut y = r.y;
    let mut line = String::new();
    let mut words = text.split_whitespace().peekable();
    while let Some(w) = words.next() {
        let try_line = if line.is_empty() {
            w.to_string()
        } else {
            format!("{line} {w}")
        };
        if cx.measure(FontId::Sans, px, &try_line) <= r.w || line.is_empty() {
            line = try_line;
        } else {
            if y + lh > r.bottom() {
                return;
            }
            let last = y + 2 * lh > r.bottom();
            let shown = if last {
                format!("{line}…")
            } else {
                line.clone()
            };
            cx.text(
                FontId::Sans,
                px,
                Rect::new(r.x, y, r.w, lh),
                Align::Left,
                c,
                &shown,
            );
            if last {
                return;
            }
            y += lh;
            line = w.to_string();
        }
        if words.peek().is_none() && y + lh <= r.bottom() {
            cx.text(
                FontId::Sans,
                px,
                Rect::new(r.x, y, r.w, lh),
                Align::Left,
                c,
                &line,
            );
        }
    }
}
