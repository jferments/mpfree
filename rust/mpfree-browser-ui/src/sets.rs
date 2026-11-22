//! The Sets place and the Set editor (live shows B1; layout spec 4n): a
//! set is a list of songs in order, each a project in Projects, kept as
//! `Sets/<name>.set`. The place lists the sets with their last check, a
//! set picked shows its card (Play set, Edit set, Check set, Rename, Copy,
//! Delete, Check every file), and Edit set lists its songs, each set in
//! the Selected column: Starts, Ends, Fade, Needs and Notes.
//!
//! This module draws the place from a [`SetsScreen`] and says what a touch
//! meant as a [`SetInput`]; the browser's logic (`state::set_place`) keeps
//! the sets, reads and saves them on the library's worker and builds the
//! screen.

use mpfree_ui::draw;
use mpfree_ui::{Align, FontId, PaintCx, Rect};

pub use crate::plugins::Tone;

/// What the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetsView {
    /// Every set.
    List,
    /// One set's songs.
    Edit,
    /// Projects to add to the set being edited.
    Add,
}

/// The buttons of the list's title line and the Selected column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetButton {
    NewSet,
    PlaySet,
    EditSet,
    CheckSet,
    /// The check, reading every file in full.
    CheckEveryFile,
    Rename,
    Copy,
    Delete,
    AddSongs,
    /// Adding songs: back to the set.
    Done,
    MoveUp,
    MoveDown,
    OpenInMake,
    Remove,
    /// Remove, asked again: a second tap removes.
    ConfirmRemove,
    /// The Needs list: back to the song.
    NeedsDone,
}

impl SetButton {
    pub fn label(self) -> &'static str {
        match self {
            SetButton::NewSet => "New set",
            SetButton::PlaySet => "Play set",
            SetButton::EditSet => "Edit set",
            SetButton::CheckSet => "Check set",
            SetButton::CheckEveryFile => "Check every file",
            SetButton::Rename => "Rename",
            SetButton::Copy => "Copy",
            SetButton::Delete => "Delete",
            SetButton::AddSongs => "Add songs",
            SetButton::Done => "Done",
            SetButton::MoveUp => "Move up",
            SetButton::MoveDown => "Move down",
            SetButton::OpenInMake => "Open in Make",
            SetButton::Remove => "Remove from set",
            SetButton::ConfirmRemove => "Tap again to remove",
            SetButton::NeedsDone => "Done",
        }
    }

    fn primary(self) -> bool {
        matches!(self, SetButton::PlaySet)
    }

    fn danger(self) -> bool {
        matches!(
            self,
            SetButton::Delete | SetButton::Remove | SetButton::ConfirmRemove
        )
    }
}

/// A song's values in the Selected column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetField {
    Starts,
    Ends,
    Fade,
    /// A tap lists the rig's keyboards and controls.
    Needs,
}

impl SetField {
    pub fn label(self) -> &'static str {
        match self {
            SetField::Starts => "Starts",
            SetField::Ends => "Ends",
            SetField::Fade => "Fade",
            SetField::Needs => "Needs",
        }
    }
}

/// One row of the list: a set, a song of the set, or a project to add.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetRow {
    /// The song's place in the set.
    pub number: Option<usize>,
    pub name: String,
    /// "12 songs", "Stopped · Let it finish", "changed on stage".
    pub sub: String,
    pub sub_tone: Tone,
    /// The last check (READY, LIMITS, NOT READY, NOT CHECKED).
    pub chip: Option<(String, Tone)>,
    /// A + at the row's right (adding songs).
    pub add: bool,
}

/// A value row of a song's card: ‹ value ›.
#[derive(Clone, Debug, PartialEq)]
pub struct Stepper {
    pub field: SetField,
    pub value: String,
    /// It doesn't apply (Fade unless Ends is Fade): drawn faint, and a tap
    /// does nothing.
    pub faint: bool,
    /// The dial turns it.
    pub focused: bool,
}

/// One of the rig's keyboards or controls in a song's Needs list.
#[derive(Clone, Debug, PartialEq)]
pub struct NeedRow {
    pub name: String,
    /// "Needed", "Nice to have" or "—".
    pub state: String,
    pub tone: Tone,
    /// "connected", "not connected", "not in your rig".
    pub connected: String,
    pub connected_tone: Tone,
}

/// The Selected column.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SetDetail {
    /// The card's heading: "Selected", "Song 4 of 12", "Needs".
    pub eyebrow: String,
    pub title: String,
    /// Under the title: "Set", "Projects · 92 BPM · Ready, checked ...".
    pub line: String,
    /// One more line under it (why a song isn't ready).
    pub reason: Option<(String, Tone)>,
    /// Label, value and the value's colour.
    pub facts: Vec<(String, String, Tone)>,
    /// Boxed lists: a heading, its colour and its lines (Not ready, With
    /// limits).
    pub lists: Vec<(String, Tone, Vec<String>)>,
    pub steppers: Vec<Stepper>,
    /// A song's notes and the line under their box.
    pub notes: Option<(String, String)>,
    /// The Needs list, while it shows.
    pub needs: Option<Vec<NeedRow>>,
    pub buttons: Vec<SetButton>,
    /// Faint buttons (a check is running).
    pub faint: Vec<SetButton>,
}

/// Everything the place draws.
#[derive(Clone, Debug, PartialEq)]
pub struct SetsScreen {
    pub view: SetsView,
    /// "Sets", a set's name, "Add songs to ...".
    pub title: String,
    /// The title starts with ‹ and a tap on it goes back.
    pub back: bool,
    pub head_buttons: Vec<SetButton>,
    pub rows: Vec<SetRow>,
    pub selected: Option<usize>,
    pub top: usize,
    pub status: String,
    pub status_tone: Tone,
    /// Said in the list when it is empty.
    pub empty: String,
    pub detail: SetDetail,
}

impl Default for SetsScreen {
    fn default() -> Self {
        SetsScreen {
            view: SetsView::List,
            title: "Sets".into(),
            back: false,
            head_buttons: vec![SetButton::NewSet],
            rows: Vec::new(),
            selected: None,
            top: 0,
            status: String::new(),
            status_tone: Tone::Dim,
            empty: "Reading\u{2026}".into(),
            detail: SetDetail::default(),
        }
    }
}

/// What a touch meant.
#[derive(Clone, Debug, PartialEq)]
pub enum SetInput {
    /// A tap on a row.
    Row(usize),
    /// The list dragged by this many rows.
    Scroll(i32),
    Button(SetButton),
    /// A value's ‹ (-1) or › (+1).
    Step(SetField, i32),
    /// The Notes box: the text keyboard.
    Notes,
    /// A tap on one of the Needs list's rows.
    Need(usize),
    /// A tap on a project's + (adding songs).
    Add(usize),
    /// The ‹ on the title line.
    Back,
}

const HEAD_H: i32 = 56;
const STATUS_H: i32 = 30;
const BUTTON_H: i32 = 56;
const BUTTON_GAP: i32 = 8;
const STEP_H: i32 = 40;
const STEP_PITCH: i32 = 50;
const STEP_LABEL_W: i32 = 150;
const NOTES_H: i32 = 100;
const NEED_H: i32 = 52;

/// A row's pitch: songs in a set are 50 px rows, sets and projects 48.
pub fn row_h(view: SetsView) -> i32 {
    match view {
        SetsView::Edit => 54,
        SetsView::List | SetsView::Add => crate::view::ROW_H,
    }
}

struct Geo {
    head: Rect,
    back: Option<Rect>,
    head_buttons: Vec<(SetButton, Rect)>,
    rows: Rect,
    row_h: i32,
    status: Rect,
    body: Rect,
    steppers: Vec<(SetField, Rect)>,
    notes: Option<Rect>,
    needs: Vec<Rect>,
    buttons: Vec<(SetButton, Rect)>,
}

impl Geo {
    fn new(list: Rect, detail: Rect, s: &SetsScreen) -> Geo {
        let inner = list.inset(12);
        let (head, body) = inner.take_top(HEAD_H);
        let (status, rows) = body.take_bottom(STATUS_H);
        let rows = rows.take_top(6).1;
        // The title line's buttons at its right, sized to their words.
        let mut head_buttons = Vec::new();
        let mut right = head;
        for b in s.head_buttons.iter().rev() {
            let w = 26 + 12 * b.label().len() as i32;
            let (br, r2) = right.take_right(w);
            head_buttons.push((*b, br.inset_xy(0, 6)));
            right = r2.take_right(8).1;
        }
        head_buttons.reverse();
        let back = s.back.then_some(right);
        // The Selected column: buttons two across at its foot.
        let db = draw::card_head(detail).1;
        let n = s.detail.buttons.len() as i32;
        let lines = (n + 1) / 2;
        let (btns, body) = db.take_bottom(lines * (BUTTON_H + BUTTON_GAP));
        let bw = (btns.w - BUTTON_GAP) / 2;
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
                        btns.x + c * (bw + BUTTON_GAP),
                        btns.y + BUTTON_GAP + l * (BUTTON_H + BUTTON_GAP),
                        bw,
                        BUTTON_H,
                    ),
                )
            })
            .collect();
        // Under the title and its line (and the reason, if any): the
        // values, then Notes.
        let top = title_h(&s.detail);
        let mut y = body.y + top + 8;
        let mut steppers = Vec::new();
        let mut needs = Vec::new();
        let mut notes = None;
        if let Some(list) = &s.detail.needs {
            for _ in list {
                if y + NEED_H > body.bottom() {
                    break;
                }
                needs.push(Rect::new(body.x, y, body.w, NEED_H - 4));
                y += NEED_H;
            }
        } else {
            for st in &s.detail.steppers {
                steppers.push((st.field, Rect::new(body.x, y, body.w, STEP_H)));
                y += STEP_PITCH;
            }
            if s.detail.notes.is_some() {
                let r = Rect::new(body.x, y + 22, body.w, NOTES_H);
                if r.bottom() + 24 <= body.bottom() {
                    notes = Some(r);
                }
            }
        }
        Geo {
            head,
            back,
            head_buttons,
            rows,
            row_h: row_h(s.view),
            status,
            body,
            steppers,
            notes,
            needs,
            buttons,
        }
    }

    fn visible(&self) -> usize {
        (self.rows.h / self.row_h).max(1) as usize
    }

    fn row_rect(&self, k: usize) -> Rect {
        Rect::new(
            self.rows.x,
            self.rows.y + k as i32 * self.row_h,
            self.rows.w - 10,
            self.row_h - 4,
        )
    }

    fn add_rect(&self, k: usize) -> Rect {
        let rr = self.row_rect(k);
        Rect::new(rr.right() - 56, rr.y + 4, 48, rr.h - 8)
    }
}

/// The height of the title, its line and the reason.
fn title_h(d: &SetDetail) -> i32 {
    34 + 26 + if d.reason.is_some() { 24 } else { 0 }
}

/// The rows that fit in the list drawn in `list`, in `view`.
pub fn rows_in(list: Rect, view: SetsView) -> usize {
    let h = list.inset(12).h - HEAD_H - STATUS_H - 6;
    (h / row_h(view)).max(1) as usize
}

/// The list's rows' area (where a drag scrolls).
pub fn rows_rect(list: Rect, detail: Rect, s: &SetsScreen) -> Rect {
    Geo::new(list, detail, s).rows
}

/// What a touch at (x, y) meant, or None.
pub fn hit(list: Rect, detail: Rect, s: &SetsScreen, x: i32, y: i32) -> Option<SetInput> {
    let g = Geo::new(list, detail, s);
    if let Some((b, _)) = g.head_buttons.iter().find(|(_, r)| r.contains(x, y)) {
        return Some(SetInput::Button(*b));
    }
    if g.back.is_some_and(|r| r.contains(x, y)) {
        return Some(SetInput::Back);
    }
    if let Some((b, _)) = g.buttons.iter().find(|(_, r)| r.contains(x, y)) {
        return Some(SetInput::Button(*b));
    }
    for (i, (field, r)) in g.steppers.iter().enumerate() {
        if r.contains(x, y) {
            if s.detail.steppers[i].faint {
                return None;
            }
            let value = Rect::new(r.x + STEP_LABEL_W, r.y, r.w - STEP_LABEL_W, r.h);
            let d = if x < value.x + value.w / 2 { -1 } else { 1 };
            return Some(SetInput::Step(*field, d));
        }
    }
    if g.notes.is_some_and(|r| r.contains(x, y)) {
        return Some(SetInput::Notes);
    }
    if let Some(k) = g.needs.iter().position(|r| r.contains(x, y)) {
        return Some(SetInput::Need(k));
    }
    if g.rows.contains(x, y) {
        let k = ((y - g.rows.y) / g.row_h) as usize;
        let i = s.top + k;
        if k < g.visible() && i < s.rows.len() {
            if s.rows[i].add && g.add_rect(k).contains(x, y) {
                return Some(SetInput::Add(i));
            }
            return Some(SetInput::Row(i));
        }
    }
    None
}

/// The place's controls, for the control audit and the smoke tests.
pub fn targets(list: Rect, detail: Rect, s: &SetsScreen) -> Vec<mpfree_ui::Target> {
    use mpfree_ui::Target;
    let g = Geo::new(list, detail, s);
    let mut out = Vec::new();
    if let Some(r) = g.back {
        out.push(Target::tap(
            "sets-back",
            0,
            format!("\u{2039} {}", s.title),
            r,
        ));
    }
    for (b, r) in &g.head_buttons {
        out.push(Target::tap("sets-button", *b as usize, b.label(), *r));
    }
    for (b, r) in &g.buttons {
        out.push(
            Target::tap("sets-button", *b as usize, b.label(), *r)
                .disabled_if(s.detail.faint.contains(b)),
        );
    }
    // Each value's ‹ and › halves (Needs: either opens its list).
    for (i, (f, r)) in g.steppers.iter().enumerate() {
        let value = Rect::new(r.x + STEP_LABEL_W, r.y, r.w - STEP_LABEL_W, r.h);
        let (back, on) = value.take_left(value.w / 2);
        let faint = s.detail.steppers[i].faint;
        out.push(
            Target::tap(
                "sets-value",
                *f as usize * 2,
                format!("{} \u{2039}", f.label()),
                back,
            )
            .disabled_if(faint),
        );
        out.push(
            Target::tap(
                "sets-value",
                *f as usize * 2 + 1,
                format!("{} \u{203a}", f.label()),
                on,
            )
            .disabled_if(faint),
        );
    }
    if let Some(r) = g.notes {
        out.push(Target::tap("sets-notes", 0, "Notes", r));
    }
    if let Some(list) = &s.detail.needs {
        for (k, r) in g.needs.iter().enumerate() {
            out.push(Target::tap("sets-need", k, list[k].name.clone(), *r));
        }
    }
    let shown = g.visible().min(s.rows.len().saturating_sub(s.top));
    for k in 0..shown {
        let row = &s.rows[s.top + k];
        if row.add {
            out.push(Target::tap(
                "sets-add",
                s.top + k,
                format!("Add {}", row.name),
                g.add_rect(k),
            ));
        }
        out.push(Target::tap(
            "sets-row",
            s.top + k,
            row.name.clone(),
            g.row_rect(k),
        ));
    }
    out
}

fn ink(cx: &PaintCx, tone: Tone) -> mpfree_ui::Color {
    let t = cx.theme;
    match tone {
        Tone::Dim => t.text_dim,
        Tone::Plain => t.text,
        Tone::Good => t.good,
        Tone::Warn => t.warn,
        Tone::Accent => t.accent2,
        Tone::Bad => t.bad,
    }
}

/// Draws the place's list in `list` and the Selected column in `detail`.
pub fn paint(cx: &mut PaintCx, list: Rect, detail: Rect, s: &SetsScreen) {
    let g = Geo::new(list, detail, s);
    let t = cx.theme.clone();
    cx.panel(list);
    let title_w = g.head_buttons.first().map_or(g.head.right(), |(_, r)| r.x) - g.head.x - 12;
    let title = if s.back {
        format!("\u{2039}  {}", s.title)
    } else {
        s.title.clone()
    };
    cx.text(
        FontId::Bold,
        t.size_title,
        Rect::new(g.head.x + 4, g.head.y, title_w, g.head.h),
        Align::Left,
        t.text,
        &title,
    );
    for (b, br) in &g.head_buttons {
        cx.canvas.fill_round_rect(*br, t.round(10), t.raised);
        cx.text(
            FontId::Sans,
            t.size_body,
            *br,
            Align::Center,
            t.text,
            b.label(),
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
        let rr = g.row_rect(k - s.top);
        paint_row(cx, rr, s.view, row, s.selected == Some(k));
        if row.add {
            let ar = g.add_rect(k - s.top);
            cx.canvas.fill_round_rect(ar, t.round(10), t.accent);
            cx.text(
                FontId::Bold,
                t.size_title,
                ar,
                Align::Center,
                t.on_accent,
                "+",
            );
        }
    }
    if s.rows.len() > vis {
        let track = Rect::new(g.rows.right() - 4, g.rows.y, 4, vis as i32 * g.row_h);
        cx.canvas.fill_round_rect(track, 2, t.outline);
        let h = (track.h * vis as i32 / s.rows.len() as i32).max(24);
        let y = track.y
            + (track.h - h) * s.top.min(s.rows.len() - vis) as i32 / (s.rows.len() - vis) as i32;
        cx.canvas
            .fill_round_rect(Rect::new(track.x, y, 4, h), 2, t.text_dim);
    }
    cx.text(
        FontId::Sans,
        t.size_small,
        g.status.inset_xy(4, 0),
        Align::Left,
        ink(cx, s.status_tone),
        &s.status,
    );
    paint_detail(cx, detail, &g, &s.detail);
}

fn paint_row(cx: &mut PaintCx, rr: Rect, view: SetsView, row: &SetRow, selected: bool) {
    let t = cx.theme.clone();
    if selected {
        cx.canvas.fill_round_rect(rr, t.round(10), t.raised);
        cx.canvas
            .fill_round_rect(Rect::new(rr.x, rr.y + 12, 4, rr.h - 24), 2, t.accent);
    } else if view == SetsView::Edit {
        cx.canvas
            .fill_rect(Rect::new(rr.x, rr.bottom() - 1, rr.w, 1), t.raised);
    }
    let inner = rr.inset_xy(14, 0);
    let rest = match (view, row.number) {
        (SetsView::Edit, Some(n)) => {
            let (num, rest) = inner.take_left(32);
            cx.text(
                FontId::Mono,
                t.size_small,
                num,
                Align::Left,
                t.text_dim,
                &n.to_string(),
            );
            rest
        }
        _ => {
            let (icon, rest) = inner.take_left(36);
            let c = if selected { t.accent } else { t.text_dim };
            if view == SetsView::Add {
                project_icon(cx, icon, c);
            } else {
                set_icon(cx, icon, c);
            }
            rest
        }
    };
    let rest = if row.add { rest.take_right(64).1 } else { rest };
    let chip_w = match &row.chip {
        Some((word, _)) => cx.measure(FontId::Bold, t.size_small * 0.8, word) + 16,
        None => 0,
    };
    let (chip_r, rest) = rest.take_right(chip_w);
    if let Some((word, tone)) = &row.chip {
        let c = ink(cx, *tone);
        let r = Rect::new(chip_r.x, chip_r.y + chip_r.h / 2 - 12, chip_r.w, 24);
        cx.canvas.stroke_round_rect(r, t.round(6), 1, c);
        cx.text(FontId::Bold, t.size_small * 0.8, r, Align::Center, c, word);
    }
    let name = rest.take_right(10).1.inset_xy(4, 0);
    let nw = cx.text(
        if selected { FontId::Bold } else { FontId::Sans },
        t.size_body,
        name,
        Align::Left,
        t.text,
        &row.name,
    );
    if !row.sub.is_empty() && nw + 40 < name.w {
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(name.x + nw + 16, name.y + 1, name.w - nw - 16, name.h),
            Align::Left,
            ink(cx, row.sub_tone),
            &row.sub,
        );
    }
}

/// A project: a page with another behind it, as the browser draws one.
fn project_icon(cx: &mut PaintCx, r: Rect, c: mpfree_ui::Color) {
    let (x, y) = (r.x + 14, r.y + r.h / 2);
    cx.canvas
        .stroke_round_rect(Rect::new(x - 8, y - 11, 20, 18), 3, 2, c.with_alpha(140));
    cx.canvas
        .fill_round_rect(Rect::new(x - 12, y - 7, 20, 18), 3, c);
}

/// Three lines, each with a dot before it: a list in order.
fn set_icon(cx: &mut PaintCx, r: Rect, c: mpfree_ui::Color) {
    let (x, y) = (r.x + 4, r.y + r.h / 2 - 9);
    for k in 0..3 {
        cx.canvas
            .fill_round_rect(Rect::new(x, y + k * 8, 4, 3), 1, c);
        cx.canvas
            .fill_round_rect(Rect::new(x + 8, y + k * 8, 14, 3), 1, c);
    }
}

fn paint_detail(cx: &mut PaintCx, detail: Rect, g: &Geo, d: &SetDetail) {
    let t = cx.theme.clone();
    draw::card(cx, detail, &d.eyebrow, "");
    let body = g.body;
    let (title, rest) = body.take_top(34);
    cx.text(
        FontId::Bold,
        t.size_title,
        title,
        Align::Left,
        t.text,
        &d.title,
    );
    let (line, rest) = rest.take_top(26);
    cx.text(
        FontId::Sans,
        t.size_small,
        line,
        Align::Left,
        t.text_dim,
        &d.line,
    );
    let mut rest = rest;
    if let Some((why, tone)) = &d.reason {
        let (r, after) = rest.take_top(24);
        cx.text(
            FontId::Sans,
            t.size_small,
            r,
            Align::Left,
            ink(cx, *tone),
            why,
        );
        rest = after;
    }
    let mut rest = rest.take_top(8).1;
    for (label, value, tone) in &d.facts {
        if rest.h < 30 {
            break;
        }
        let (line, after) = rest.take_top(32);
        let (l, v) = line.take_left(110);
        cx.text(
            FontId::Sans,
            t.size_small,
            l,
            Align::Left,
            t.text_dim,
            label,
        );
        let c = if *tone == Tone::Dim {
            t.text
        } else {
            ink(cx, *tone)
        };
        cx.text(FontId::Sans, t.size_body, v, Align::Left, c, value);
        rest = after;
    }
    // The boxed lists, in what is left above the buttons.
    let lists_bottom = g.buttons.first().map_or(body.bottom(), |(_, r)| r.y - 8);
    if !d.lists.is_empty() && rest.y + 60 < lists_bottom {
        let boxr = Rect::new(rest.x, rest.y + 4, rest.w, lists_bottom - rest.y - 8);
        cx.canvas.fill_round_rect(boxr, t.round(10), t.raised);
        let mut y = boxr.y + 10;
        let inner_w = boxr.w - 28;
        'lists: for (head, tone, lines) in &d.lists {
            if y + 20 > boxr.bottom() {
                break;
            }
            cx.text(
                FontId::Bold,
                t.size_small * 0.8,
                Rect::new(boxr.x + 14, y, inner_w, 18),
                Align::Left,
                ink(cx, *tone),
                &head.to_uppercase(),
            );
            y += 22;
            for line in lines {
                for l in wrap(cx, t.size_small, line, inner_w) {
                    if y + 20 > boxr.bottom() - 6 {
                        break 'lists;
                    }
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        Rect::new(boxr.x + 14, y, inner_w, 20),
                        Align::Left,
                        t.text,
                        &l,
                    );
                    y += 20;
                }
            }
            y += 8;
        }
    }
    for (st, (_, r)) in d.steppers.iter().zip(&g.steppers) {
        let ink_c = if st.faint { t.text_faint } else { t.text };
        let (l, v) = r.take_left(STEP_LABEL_W);
        cx.text(
            FontId::Sans,
            t.size_body,
            l,
            Align::Left,
            ink_c,
            st.field.label(),
        );
        // A row that doesn't apply (Fade unless Ends is Fade) is an
        // outline on the card, so its faint words stay readable on it.
        if st.faint {
            cx.canvas.stroke_round_rect(v, t.round(8), 1, t.outline);
        } else {
            cx.canvas.fill_round_rect(v, t.round(8), t.raised);
        }
        if st.focused {
            cx.canvas.stroke_round_rect(v, t.round(8), 2, t.accent);
        }
        let inner = v.inset_xy(14, 0);
        let arrows = if st.faint { t.text_faint } else { t.text_dim };
        cx.text(
            FontId::Sans,
            t.size_body,
            inner,
            Align::Left,
            arrows,
            "\u{2039}",
        );
        cx.text(
            FontId::Sans,
            t.size_body,
            inner,
            Align::Right,
            arrows,
            "\u{203a}",
        );
        cx.text(
            FontId::Bold,
            t.size_body,
            inner.inset_xy(18, 0),
            Align::Center,
            ink_c,
            &st.value,
        );
    }
    if let (Some((text, foot)), Some(nr)) = (&d.notes, g.notes) {
        cx.text(
            FontId::Bold,
            t.size_small * 0.8,
            Rect::new(nr.x, nr.y - 22, nr.w, 18),
            Align::Left,
            t.text_dim,
            "NOTES",
        );
        cx.canvas.fill_round_rect(nr, t.round(8), t.raised);
        cx.canvas.stroke_round_rect(nr, t.round(8), 1, t.outline);
        let inner = nr.inset_xy(12, 10);
        let lines = wrap(cx, t.size_body, text, inner.w);
        let lh = 26;
        let fit = (inner.h / lh).max(1) as usize;
        for (k, l) in lines.iter().take(fit).enumerate() {
            let shown = if k + 1 == fit && lines.len() > fit {
                format!("{l}\u{2026}")
            } else {
                l.clone()
            };
            cx.text(
                FontId::Sans,
                t.size_body,
                Rect::new(inner.x, inner.y + k as i32 * lh, inner.w, lh),
                Align::Left,
                t.text,
                &shown,
            );
        }
        cx.text(
            FontId::Sans,
            t.size_small * 0.86,
            Rect::new(nr.x, nr.bottom() + 4, nr.w, 20),
            Align::Left,
            t.text_dim,
            foot,
        );
    }
    if let Some(list) = &d.needs {
        for (n, r) in list.iter().zip(&g.needs) {
            cx.canvas.fill_round_rect(*r, t.round(8), t.raised);
            let inner = r.inset_xy(14, 0);
            let (state, name) = inner.take_right(130);
            let c = ink(cx, n.tone);
            cx.text(FontId::Bold, t.size_small, state, Align::Right, c, &n.state);
            let (top, bottom) = name.take_top(name.h / 2 + 4);
            cx.text(FontId::Sans, t.size_body, top, Align::Left, t.text, &n.name);
            cx.text(
                FontId::Sans,
                t.size_small * 0.86,
                bottom,
                Align::Left,
                ink(cx, n.connected_tone),
                &n.connected,
            );
        }
    }
    for (b, br) in &g.buttons {
        let faint = d.faint.contains(b);
        button(cx, *br, b.label(), b.primary() && !faint, b.danger(), faint);
    }
}

fn button(cx: &mut PaintCx, r: Rect, label: &str, lit: bool, danger: bool, faint: bool) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(r, t.round(10), if lit { t.accent } else { t.raised });
    let ink = if faint {
        t.text_faint
    } else if lit {
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

/// `text` wrapped to `w` at `px`: its own line breaks kept, a word too
/// long for a line cut where it must be.
fn wrap(cx: &mut PaintCx, px: f32, text: &str, w: i32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let try_line = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if line.is_empty() || cx.measure(FontId::Sans, px, &try_line) <= w {
                line = try_line;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        out.push(line);
    }
    out
}

/// One of the rig's keyboards or learned controls, as the app gives it
/// ([`crate::Browser::set_rig`]): what a song's Needs offers, and what the
/// check asks to be connected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigThing {
    pub name: String,
    pub connected: bool,
}
