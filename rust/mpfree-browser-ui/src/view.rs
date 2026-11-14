//! The browser's widget: places on the left, the list in the middle, the
//! selected item on the right, and the keyboard or a question over them
//! when one is open. It draws a [`Screen`] and turns touches into
//! [`Input`]s; it never touches the library itself.

use crate::export::{
    ExportBits, ExportInput, ExportPanel, ExportPhase, ExportRange, StemsBy, EXPORT_RATES,
};
use crate::state::{Button, Filter, Input, Key, Modal, Row, Screen, Sort, Source, SOURCES};
use mpfree_library::{AudioFormat, Kind, Place};
use mpfree_ui::draw;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Touch, Widget};

/// The places column's width.
pub const PLACES_W: i32 = 220;
/// The selected item's column's width.
pub const DETAIL_W: i32 = 420;
/// Narrower than this, the browser takes its narrow form (beside the Pads
/// tab's pad bank): slimmer places and selected item, so the list keeps
/// room for names.
pub const NARROW_BELOW: i32 = 1000;
/// The narrow form's places column.
pub const NARROW_PLACES_W: i32 = 176;
/// The narrow form's selected item column.
pub const NARROW_DETAIL_W: i32 = 290;
/// A list row's height (finger sized).
pub const ROW_H: i32 = 52;
const HEAD_H: i32 = 56;
const FOOT_H: i32 = 52;
const STATUS_H: i32 = 30;
const PLACE_H: i32 = 50;
const BUTTON_H: i32 = 56;
/// Movement before a touch on the list counts as a drag, not a tap.
const DRAG: i32 = 14;

/// Where everything is, for drawing and for touches alike.
struct Geo {
    places: Rect,
    list: Rect,
    detail: Rect,
    head: Rect,
    back: Rect,
    path: Rect,
    list_buttons: Vec<(Button, Rect)>,
    rows: Rect,
    filters: Vec<(Filter, Rect)>,
    /// The Search and order chips, at the foot's right.
    search: Rect,
    sort: Rect,
    status: Rect,
    place_rows: Vec<Rect>,
    detail_buttons: Vec<(Button, Rect)>,
    detail_body: Rect,
}

fn card_body(r: Rect) -> Rect {
    draw::card_head(r).1
}

impl Geo {
    fn new(r: Rect, s: &Screen) -> Geo {
        let gap = 12;
        let (places_w, detail_w) = if r.w < NARROW_BELOW {
            (NARROW_PLACES_W, NARROW_DETAIL_W)
        } else {
            (PLACES_W, DETAIL_W)
        };
        let (places, rest) = r.take_left(places_w);
        let (_, rest) = rest.take_left(gap);
        let (detail, rest) = rest.take_right(detail_w);
        let (_, list) = rest.take_right(gap);
        // Places.
        let pb = card_body(places);
        let mut place_rows = Vec::new();
        let mut y = pb.y;
        for (k, &i) in s.sources.iter().enumerate() {
            let src = &SOURCES[i];
            if k > 0 && matches!(src, Source::Recent | Source::Place(Place::Trash)) {
                y += 14;
            }
            place_rows.push(Rect::new(pb.x, y, pb.w, PLACE_H - 4));
            y += PLACE_H;
        }
        // List.
        let inner = list.inset(12);
        let (head, body) = inner.take_top(HEAD_H);
        let (status, body) = body.take_bottom(STATUS_H);
        let (foot, rows) = body.take_bottom(FOOT_H);
        let (back, head_rest) = head.take_left(if s.can_go_back { 56 } else { 0 });
        let mut list_buttons = Vec::new();
        let mut right = head_rest;
        for b in s.list_buttons.iter().rev() {
            let w = 26 + 12 * b.label().len() as i32;
            let (br, r2) = right.take_right(w);
            list_buttons.push((*b, br.inset_xy(0, 6)));
            right = r2.take_right(8).1;
        }
        list_buttons.reverse();
        let path = right.inset_xy(if s.can_go_back { 10 } else { 4 }, 0);
        // Chips sized to their words, so the filters and the Search and
        // order chips fit the list's foot on the MPC's screen.
        let chip_w = |label: &str| 26 + 10 * label.len() as i32;
        let mut filters = Vec::new();
        let mut x = foot.x;
        for &f in &s.filters {
            let w = chip_w(f.label());
            filters.push((f, Rect::new(x, foot.y + 8, w, foot.h - 12)));
            x += w + 8;
        }
        let (sort, rest) = foot.take_right(chip_w(s.sort.label()));
        let sort = Rect::new(sort.x, foot.y + 8, sort.w, foot.h - 12);
        let (_, rest) = rest.take_right(8);
        let (search, _) = rest.take_right(chip_w("Search"));
        let search = Rect::new(search.x, foot.y + 8, search.w, foot.h - 12);
        // A filter chip that would run under Search is left out (the
        // narrow form has room for fewer).
        filters.retain(|(_, fr)| fr.right() <= search.x - 8);
        // Detail.
        let db = card_body(detail);
        let n = s.detail.buttons.len() as i32;
        let cols = 2;
        let lines = (n + cols - 1) / cols;
        let (btns, body) = db.take_bottom(lines * (BUTTON_H + 8));
        let bw = (btns.w - 8) / cols;
        let detail_buttons = s
            .detail
            .buttons
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let (c, l) = (i as i32 % cols, i as i32 / cols);
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
            places,
            list,
            detail,
            head,
            back,
            path,
            list_buttons,
            rows,
            filters,
            search,
            sort,
            status,
            place_rows,
            detail_buttons,
            detail_body: body,
        }
    }

    fn visible(&self) -> usize {
        (self.rows.h / ROW_H).max(1) as usize
    }

    fn row_rect(&self, k: usize) -> Rect {
        Rect::new(
            self.rows.x,
            self.rows.y + k as i32 * ROW_H,
            self.rows.w - 10,
            ROW_H - 4,
        )
    }
}

/// The keyboard's keys, row by row (lower case; Shift gives capitals).
const KEY_ROWS: [&str; 4] = ["1234567890-", "qwertyuiop()", "asdfghjkl&_", "zxcvbnm,.'"];

struct Keys {
    card: Rect,
    field: Rect,
    title: Rect,
    keys: Vec<(Key, Rect)>,
}

fn keyboard_geo(r: Rect) -> Keys {
    let card = Rect::new(r.x + 40, r.y + 30, r.w - 80, r.h - 60);
    let inner = card.inset(20);
    let (title, rest) = inner.take_top(30);
    let (field, rest) = rest.take_top(64);
    let (_, kb) = rest.take_top(16);
    let rows = 5;
    let kh = (kb.h - (rows - 1) * 8) / rows;
    let kw = (kb.w - 11 * 8) / 12;
    let mut keys = Vec::new();
    for (ri, row) in KEY_ROWS.iter().enumerate() {
        let y = kb.y + ri as i32 * (kh + 8);
        let n = row.chars().count() as i32 + if ri == 3 { 2 } else { 0 };
        let mut x = kb.x + (kb.w - (n * kw + (n - 1) * 8)) / 2;
        if ri == 3 {
            keys.push((Key::Shift, Rect::new(x, y, kw * 2 + 8, kh)));
            x += 2 * (kw + 8);
        }
        for c in row.chars() {
            keys.push((Key::Char(c), Rect::new(x, y, kw, kh)));
            x += kw + 8;
        }
        if ri == 0 {
            keys.push((Key::Backspace, Rect::new(x, y, kb.right() - x, kh)));
        }
    }
    let y = kb.y + 4 * (kh + 8);
    let w3 = kw * 3 + 16;
    keys.push((Key::Cancel, Rect::new(kb.x, y, w3, kh)));
    keys.push((
        Key::Space,
        Rect::new(kb.x + w3 + 8, y, kb.w - 2 * (w3 + 8), kh),
    ));
    keys.push((Key::Done, Rect::new(kb.right() - w3, y, w3, kh)));
    Keys {
        card,
        field,
        title,
        keys,
    }
}

fn key_label(k: Key, shift: bool) -> String {
    match k {
        Key::Char(c) => {
            if shift {
                c.to_ascii_uppercase().to_string()
            } else {
                c.to_string()
            }
        }
        Key::Space => "space".into(),
        Key::Backspace => "Del".into(),
        Key::Shift => "Shift".into(),
        Key::Cancel => "Cancel".into(),
        Key::Done => "Done".into(),
    }
}

struct Ask {
    card: Rect,
    text: Rect,
    no: Rect,
    yes: Rect,
}

fn confirm_geo(r: Rect) -> Ask {
    let w = 620.min(r.w - 40);
    let card = Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - 230) / 2, w, 230);
    let inner = card.inset(24);
    let (btns, text) = inner.take_bottom(64);
    let bw = (btns.w - 12) / 2;
    Ask {
        card,
        text,
        no: Rect::new(btns.x, btns.y, bw, btns.h),
        yes: Rect::new(btns.x + bw + 12, btns.y, bw, btns.h),
    }
}

/// The Export panel's card and its touch areas (each used in the phases
/// that show it).
struct ExportGeo {
    card: Rect,
    title: Rect,
    make_label: Rect,
    mixdown: Rect,
    stems: Rect,
    midi: Rect,
    by_label: Rect,
    by_track: Rect,
    by_channel: Rect,
    by_words: Rect,
    range_label: Rect,
    arrangement: Rect,
    bars: Rect,
    fewer: Rect,
    count: Rect,
    more: Rect,
    range_words: Rect,
    file_label: Rect,
    /// 24-bit, 16-bit, 32-bit float, then 44.1 and 48 kHz.
    bits: [Rect; 3],
    rates: [Rect; 2],
    plays_label: Rect,
    plays: Rect,
    into_label: Rect,
    into: Rect,
    /// Running: the time done, and the bar under it.
    words: Rect,
    bar: Rect,
    /// Over: the outcome's lines.
    outcome: Rect,
    /// The bottom row: Close / Cancel on the left, Start / Show on the right.
    left: Rect,
    right: Rect,
}

fn export_geo(r: Rect) -> ExportGeo {
    let w = 780.min(r.w - 40);
    let h = 550.min(r.h - 24);
    let card = Rect::new(r.x + (r.w - w) / 2, r.y + (r.h - h) / 2, w, h);
    let inner = card.inset(24);
    let (title, rest) = inner.take_top(40);
    let (_, rest) = rest.take_top(10);
    let (bottom, body) = rest.take_bottom(BUTTON_H + 4);
    let bottom = bottom.take_bottom(BUTTON_H).0;
    let label_w = 120;
    let (make, rest) = body.take_top(BUTTON_H);
    let (make_label, make) = make.take_left(label_w);
    let tw = (make.w - 16) / 3;
    let mixdown = Rect::new(make.x, make.y, tw, make.h);
    let stems = Rect::new(make.x + tw + 8, make.y, tw, make.h);
    let midi = Rect::new(make.x + 2 * (tw + 8), make.y, make.w - 2 * (tw + 8), make.h);
    let (_, rest) = rest.take_top(8);
    let (by, rest) = rest.take_top(BUTTON_H);
    let (by_label, by) = by.take_left(label_w);
    let (by_track, by) = by.take_left(140);
    let (_, by) = by.take_left(8);
    let (by_channel, by) = by.take_left(140);
    let (_, by_words) = by.take_left(16);
    let (_, rest) = rest.take_top(14);
    let (range, rest) = rest.take_top(BUTTON_H);
    let (range_label, range) = range.take_left(label_w);
    let (arrangement, range) = range.take_left(200);
    let (_, range) = range.take_left(8);
    let (bars, range) = range.take_left(110);
    let (_, range) = range.take_left(16);
    let (fewer, range) = range.take_left(BUTTON_H + 8);
    let (count, range) = range.take_left(80);
    let (more, _) = range.take_left(BUTTON_H + 8);
    let (range_words, rest) = rest.take_top(34);
    let range_words = range_words.take_right(range_words.w - label_w).0;
    let (file, rest) = rest.take_top(BUTTON_H);
    let (file_label, file) = file.take_left(label_w);
    let (b24, file) = file.take_left(110);
    let (_, file) = file.take_left(8);
    let (b16, file) = file.take_left(110);
    let (_, file) = file.take_left(8);
    let (f32r, file) = file.take_left(150);
    let (_, file) = file.take_left(24);
    let (r44, file) = file.take_left(104);
    let (_, file) = file.take_left(8);
    let (r48, _) = file.take_left(104);
    let (_, rest) = rest.take_top(14);
    let (plays, rest) = rest.take_top(52);
    let (plays_label, plays) = plays.take_left(label_w);
    // The label beside the first line.
    let plays_label = plays_label.take_top(24).0;
    let (_, rest) = rest.take_top(8);
    let (into, _) = rest.take_top(30);
    let (into_label, into) = into.take_left(label_w);
    let (words, rest) = body.inset_xy(0, 20).take_top(56);
    let (_, rest) = rest.take_top(12);
    let (bar, _) = rest.take_top(18);
    let outcome = body.inset_xy(0, 12);
    let (left, rest) = bottom.take_left(220);
    let (right, _) = rest.take_right(260);
    ExportGeo {
        card,
        title,
        make_label,
        mixdown,
        stems,
        midi,
        by_label,
        by_track,
        by_channel,
        by_words,
        range_label,
        arrangement,
        bars,
        fewer,
        count,
        more,
        range_words,
        file_label,
        bits: [b24, b16, f32r],
        rates: [r44, r48],
        plays_label,
        plays,
        into_label,
        into,
        words,
        bar,
        outcome,
        left,
        right,
    }
}

/// What a touch on the Export panel at (x, y) means.
/// The panel's controls as they stand: where, what a tap means, and the
/// label (for the hit test and the page's targets).
fn export_controls(p: &ExportPanel, r: Rect) -> Vec<(Rect, ExportInput, &'static str)> {
    let g = export_geo(r);
    match p.phase {
        ExportPhase::Ready => vec![
            (g.mixdown, ExportInput::Mixdown, "Mixdown"),
            (g.stems, ExportInput::Stems, "Stems"),
            (g.midi, ExportInput::Midi, "MIDI file"),
            (g.by_track, ExportInput::ByTrack, "Track"),
            (g.by_channel, ExportInput::ByChannel, "Channel"),
            (g.arrangement, ExportInput::Arrangement, "Arrangement"),
            (g.bars, ExportInput::Bars, "Bars"),
            (g.fewer, ExportInput::Fewer, "-"),
            (g.more, ExportInput::More, "+"),
            (g.bits[0], ExportInput::Bits24, "24-bit"),
            (g.bits[1], ExportInput::Bits16, "16-bit"),
            (g.bits[2], ExportInput::Float32, "32-bit float"),
            (g.rates[0], ExportInput::Rate44, "44.1 kHz"),
            (g.rates[1], ExportInput::Rate48, "48 kHz"),
            (g.left, ExportInput::Close, "Close"),
            (g.right, ExportInput::Start, "Start"),
        ],
        ExportPhase::Running { .. } => vec![(g.right, ExportInput::Cancel, "Cancel")],
        ExportPhase::Done {
            folder: Some(_), ..
        } => vec![
            (g.left, ExportInput::Close, "Close"),
            (g.right, ExportInput::Show, "Show in BROWSE"),
        ],
        ExportPhase::Done { .. } | ExportPhase::Stopped(_) => {
            vec![(g.left, ExportInput::Close, "Close")]
        }
    }
}

fn export_hit(p: &ExportPanel, r: Rect, x: i32, y: i32) -> Option<ExportInput> {
    export_controls(p, r)
        .into_iter()
        .find(|(rr, _, _)| rr.contains(x, y))
        .map(|(_, i, _)| i)
}

/// `text` in lines no wider than `w` (a word wider than `w` gets a line of
/// its own).
fn wrap(cx: &mut PaintCx, font: FontId, size: f32, text: &str, w: i32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split(' ') {
        match lines.last_mut() {
            Some(l) if cx.measure(font, size, &format!("{l} {word}")) <= w => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    lines
}

/// Draws `text` wrapped into `r`, from its top, `line` pixels a line;
/// lines past its bottom are left out.
fn paint_lines(
    cx: &mut PaintCx,
    r: Rect,
    font: FontId,
    size: f32,
    line: i32,
    ink: Color,
    text: &str,
) {
    let lines = wrap(cx, font, size, text, r.w);
    for (k, l) in lines.iter().enumerate() {
        let lr = Rect::new(r.x, r.y + k as i32 * line, r.w, line);
        if lr.bottom() > r.bottom() + 1 {
            break;
        }
        cx.text(font, size, lr, Align::Left, ink, l);
    }
}

fn paint_export(cx: &mut PaintCx, r: Rect, p: &ExportPanel) {
    let t = cx.theme.clone();
    cx.canvas.fill_rect(r, t.bg.with_alpha(200));
    let g = export_geo(r);
    cx.canvas.fill_round_rect(g.card, t.radius, t.surface);
    cx.canvas.stroke_round_rect(g.card, t.radius, 1, t.outline);
    let title = match &p.phase {
        ExportPhase::Ready => "Export",
        ExportPhase::Running { .. } => "Exporting",
        ExportPhase::Done { .. } => "Exported",
        ExportPhase::Stopped(_) => "Export stopped",
    };
    cx.text(
        FontId::Bold,
        t.size_title + 3.0,
        g.title,
        Align::Left,
        t.text,
        title,
    );
    let label = |cx: &mut PaintCx, rr: Rect, s: &str| {
        cx.text(FontId::Bold, t.size_body, rr, Align::Left, t.text_dim, s);
    };
    match &p.phase {
        ExportPhase::Ready => {
            label(cx, g.make_label, "Make");
            button(cx, g.mixdown, "Mixdown", p.mixdown, false);
            let stems = format!("Stems ({})", p.stem_count().0);
            button(cx, g.stems, &stems, p.stems, false);
            button(cx, g.midi, "MIDI file", p.midi, false);
            label(cx, g.by_label, "Stems by");
            button(cx, g.by_track, "Track", p.by == StemsBy::Track, false);
            button(cx, g.by_channel, "Channel", p.by == StemsBy::Channel, false);
            paint_lines(
                cx,
                g.by_words.inset_xy(0, 4),
                FontId::Sans,
                t.size_small,
                24,
                if p.stems { t.text_dim } else { t.text_faint },
                &p.by_words(),
            );
            label(cx, g.range_label, "Range");
            if p.setup.arrangement.is_some() {
                button(
                    cx,
                    g.arrangement,
                    "Arrangement",
                    p.range == ExportRange::Arrangement,
                    false,
                );
            } else {
                cx.canvas.stroke_round_rect(g.arrangement, 10, 1, t.outline);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    g.arrangement,
                    Align::Center,
                    t.text_faint,
                    "No arrangement",
                );
            }
            let on_bars = matches!(p.range, ExportRange::Bars(_));
            button(cx, g.bars, "Bars", on_bars, false);
            button(cx, g.fewer, "-", false, false);
            cx.text(
                FontId::Bold,
                t.size_title,
                g.count,
                Align::Center,
                if on_bars { t.text } else { t.text_dim },
                &p.bars.to_string(),
            );
            button(cx, g.more, "+", false, false);
            cx.text(
                FontId::Sans,
                t.size_small,
                g.range_words,
                Align::Left,
                t.text_dim,
                &p.range_words(),
            );
            label(cx, g.file_label, "File");
            let bits = [
                (ExportBits::I24, "24-bit"),
                (ExportBits::I16, "16-bit"),
                (ExportBits::F32, "32-bit float"),
            ];
            for (rr, (b, name)) in g.bits.iter().zip(bits) {
                button(cx, *rr, name, p.format.bits == b, false);
            }
            let rates = EXPORT_RATES.iter().zip(["44.1 kHz", "48 kHz"]);
            for (rr, (rate, name)) in g.rates.iter().zip(rates) {
                button(cx, *rr, name, p.format.rate == *rate, false);
            }
            label(cx, g.plays_label, "Plays");
            paint_lines(
                cx,
                g.plays,
                FontId::Sans,
                t.size_small,
                24,
                t.text,
                &p.setup.mode,
            );
            label(cx, g.into_label, "Into");
            cx.text(
                FontId::Sans,
                t.size_small,
                g.into,
                Align::Left,
                t.text,
                &p.setup.folder,
            );
            button(cx, g.left, "Close", false, false);
            if p.can_start() {
                button(cx, g.right, "Start", true, false);
            } else {
                cx.canvas.stroke_round_rect(g.right, 10, 1, t.outline);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    g.right,
                    Align::Center,
                    t.text_faint,
                    "Tick something to make",
                );
            }
        }
        ExportPhase::Running { words, part } => {
            cx.text(
                FontId::Bold,
                t.size_title + 9.0,
                g.words,
                Align::Center,
                t.text,
                words,
            );
            cx.canvas.fill_round_rect(g.bar, g.bar.h / 2, t.raised);
            if let Some(f) = part {
                let w = ((g.bar.w as f32) * f).round() as i32;
                if w > 0 {
                    cx.canvas.fill_round_rect(
                        Rect::new(g.bar.x, g.bar.y, w.max(g.bar.h), g.bar.h),
                        g.bar.h / 2,
                        t.accent,
                    );
                }
            }
            label(cx, g.plays_label, "Plays");
            paint_lines(
                cx,
                g.plays,
                FontId::Sans,
                t.size_small,
                24,
                t.text,
                &p.setup.mode,
            );
            label(cx, g.into_label, "Into");
            cx.text(
                FontId::Sans,
                t.size_small,
                g.into,
                Align::Left,
                t.text,
                &p.setup.folder,
            );
            button(cx, g.right, "Cancel", false, true);
        }
        ExportPhase::Done { text, folder } => {
            paint_lines(cx, g.outcome, FontId::Sans, t.size_body, 30, t.text, text);
            button(cx, g.left, "Close", false, false);
            if folder.is_some() {
                button(cx, g.right, "Show in BROWSE", true, false);
            }
        }
        ExportPhase::Stopped(text) => {
            paint_lines(cx, g.outcome, FontId::Sans, t.size_body, 30, t.bad, text);
            button(cx, g.left, "Close", false, false);
        }
    }
}

/// The widget.
pub struct BrowserView {
    screen: Screen,
    inputs: Vec<Input>,
    /// A finger on the list: (finger, start y, rows scrolled so far, dragging).
    drag: Option<(u64, i32, i32, bool)>,
    /// The last rect drawn in, for the rows that fit.
    visible: usize,
}

impl BrowserView {
    pub fn new(screen: Screen) -> BrowserView {
        BrowserView {
            screen,
            inputs: Vec::new(),
            drag: None,
            visible: 11,
        }
    }

    pub fn show(&mut self, screen: Screen) {
        self.screen = screen;
    }

    pub fn screen(&self) -> &Screen {
        &self.screen
    }

    pub fn take_inputs(&mut self) -> Vec<Input> {
        std::mem::take(&mut self.inputs)
    }

    /// How many rows the list had room for when last drawn or touched.
    pub fn visible_rows(&self) -> usize {
        self.visible
    }

    /// The rows that fit in a browser drawn in `r`.
    pub fn rows_in(r: Rect) -> usize {
        Geo::new(r, &Screen::empty()).visible()
    }

    fn hit(&self, r: Rect, x: i32, y: i32) -> Option<Input> {
        let s = &self.screen;
        if let Some(p) = &s.export {
            return export_hit(p, r, x, y).map(Input::Export);
        }
        match &s.modal {
            Modal::Keyboard { .. } => {
                let k = keyboard_geo(r);
                return k
                    .keys
                    .iter()
                    .find(|(_, kr)| kr.contains(x, y))
                    .map(|(key, _)| Input::Key(*key));
            }
            Modal::Confirm { .. } => {
                let a = confirm_geo(r);
                if a.yes.contains(x, y) {
                    return Some(Input::ConfirmYes);
                }
                if a.no.contains(x, y) || !a.card.contains(x, y) {
                    return Some(Input::ConfirmNo);
                }
                return None;
            }
            Modal::None => {}
        }
        let g = Geo::new(r, s);
        if let Some(k) = g.place_rows.iter().position(|pr| pr.contains(x, y)) {
            return Some(Input::Source(s.sources[k]));
        }
        if s.can_go_back && g.back.contains(x, y) {
            return Some(Input::Back);
        }
        if let Some((b, _)) = g.list_buttons.iter().find(|(_, br)| br.contains(x, y)) {
            return Some(Input::Button(*b));
        }
        if let Some((f, _)) = g.filters.iter().find(|(_, fr)| fr.contains(x, y)) {
            return Some(Input::Filter(*f));
        }
        if g.search.contains(x, y) {
            return Some(Input::Search);
        }
        if g.sort.contains(x, y) {
            return Some(Input::Sort);
        }
        if let Some((b, _)) = g.detail_buttons.iter().find(|(_, br)| br.contains(x, y)) {
            return Some(Input::Button(*b));
        }
        if g.rows.contains(x, y) {
            let k = ((y - g.rows.y) / ROW_H) as usize;
            let i = s.top + k;
            if k < g.visible() && i < s.rows.len() {
                return Some(Input::Row(i));
            }
        }
        None
    }
}

impl Widget for BrowserView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let s = &self.screen;
        let g = Geo::new(r, s);
        let t = cx.theme.clone();
        // Places.
        draw::card(cx, g.places, &s.title, "");
        for (&i, pr) in s.sources.iter().zip(&g.place_rows) {
            let src = &SOURCES[i];
            let on = i == s.source;
            if on {
                cx.canvas.fill_round_rect(*pr, 10, t.raised);
                cx.canvas
                    .fill_round_rect(Rect::new(pr.x, pr.y + 12, 4, pr.h - 24), 2, t.accent);
            }
            let (icon, text) = pr.inset_xy(12, 0).take_left(30);
            source_icon(cx, icon, *src, if on { t.accent } else { t.text_dim });
            cx.text(
                if on { FontId::Bold } else { FontId::Sans },
                t.size_body,
                text.inset_xy(8, 0),
                Align::Left,
                if on { t.text } else { t.text_dim },
                src.label(),
            );
        }
        // List.
        cx.panel(g.list);
        if s.can_go_back {
            let b = g.back.inset_xy(0, 6);
            cx.canvas.fill_round_rect(b, 10, t.raised);
            cx.text(
                FontId::Bold,
                t.size_title,
                b,
                Align::Center,
                t.text,
                "\u{2039}",
            );
        }
        cx.text(
            FontId::Bold,
            t.size_title,
            g.path,
            Align::Left,
            t.text,
            &s.path,
        );
        for (b, br) in &g.list_buttons {
            button(cx, *br, b.label(), false, matches!(b, Button::EmptyTrash));
        }
        cx.canvas.fill_rect(
            Rect::new(g.head.x, g.head.bottom() - 1, g.head.w, 1),
            t.outline,
        );
        let vis = g.visible();
        if s.rows.is_empty() {
            let msg = if s.busy {
                "Reading…"
            } else {
                "Nothing here yet"
            };
            cx.text(
                FontId::Sans,
                t.size_body,
                g.rows,
                Align::Center,
                t.text_faint,
                msg,
            );
        }
        for (k, row) in s.rows.iter().enumerate().skip(s.top).take(vis) {
            let rr = g.row_rect(k - s.top);
            paint_row(cx, rr, row, s.selected == Some(k));
        }
        if s.rows.len() > vis {
            // A thin scroll bar.
            let track = Rect::new(g.rows.right() - 4, g.rows.y, 4, vis as i32 * ROW_H);
            cx.canvas.fill_round_rect(track, 2, t.outline);
            let h = (track.h * vis as i32 / s.rows.len() as i32).max(24);
            let y = track.y + (track.h - h) * s.top as i32 / (s.rows.len() - vis) as i32;
            cx.canvas
                .fill_round_rect(Rect::new(track.x, y, 4, h), 2, t.text_dim);
        }
        for (f, fr) in &g.filters {
            chip(cx, *fr, f.label(), *f == s.filter);
        }
        chip(cx, g.search, "Search", s.path.starts_with("Search: "));
        chip(cx, g.sort, s.sort.label(), s.sort == Sort::Newest);
        let clip = s.clipboard.as_deref().unwrap_or("");
        if !clip.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small,
                g.status.inset_xy(4, 0),
                Align::Right,
                t.text_dim,
                clip,
            );
        }
        if let Some((msg, bad)) = &s.status {
            cx.text(
                FontId::Sans,
                t.size_small,
                g.status.inset_xy(4, 0),
                Align::Left,
                if *bad { t.bad } else { t.text_dim },
                msg,
            );
        }
        // Detail.
        let head = if s.busy { "working…" } else { "" };
        draw::card(cx, g.detail, "Selected", head);
        let d = &s.detail;
        let mut body = g.detail_body;
        let (title, rest) = body.take_top(38);
        cx.text(
            FontId::Bold,
            t.size_title,
            title,
            Align::Left,
            t.text,
            &d.title,
        );
        let (kind, rest) = rest.take_top(28);
        cx.text(
            FontId::Sans,
            t.size_small,
            kind,
            Align::Left,
            t.text_dim,
            &d.kind,
        );
        body = rest;
        if let Some(o) = &d.overview {
            let (wave, rest) = body.take_top(130);
            paint_overview(cx, wave.inset_xy(0, 8), o, t.accent);
            body = rest;
        }
        for (label, value) in &d.lines {
            if body.h < 30 {
                break;
            }
            let (line, rest) = body.take_top(32);
            let (l, v) = line.take_left(110);
            cx.text(
                FontId::Sans,
                t.size_small,
                l,
                Align::Left,
                t.text_dim,
                label,
            );
            cx.text(FontId::Sans, t.size_body, v, Align::Left, t.text, value);
            body = rest;
        }
        for (b, br) in &g.detail_buttons {
            let lit = match b {
                Button::Audition => s.audition,
                Button::Favourite => s
                    .selected
                    .and_then(|i| s.rows.get(i))
                    .is_some_and(|r| r.favourite),
                _ => false,
            };
            let label = match b {
                Button::Audition => {
                    if s.audition {
                        "Audition on"
                    } else {
                        "Audition off"
                    }
                }
                _ => b.label(),
            };
            button(cx, *br, label, lit, *b == Button::Delete);
        }
        // Over everything: the keyboard or a question.
        match &s.modal {
            Modal::None => {}
            Modal::Keyboard {
                title, text, shift, ..
            } => {
                cx.canvas.fill_rect(r, t.bg.with_alpha(200));
                let k = keyboard_geo(r);
                cx.canvas.fill_round_rect(k.card, t.radius, t.surface);
                cx.canvas.stroke_round_rect(k.card, t.radius, 1, t.outline);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    k.title,
                    Align::Left,
                    t.text_dim,
                    title,
                );
                cx.canvas.fill_round_rect(k.field, 10, t.raised);
                let tw = cx.text(
                    FontId::Sans,
                    t.size_title,
                    k.field.inset_xy(18, 0),
                    Align::Left,
                    t.text,
                    text,
                );
                let cx0 = k.field.x + 18 + tw + 2;
                cx.canvas.fill_rect(
                    Rect::new(
                        cx0.min(k.field.right() - 12),
                        k.field.y + 16,
                        2,
                        k.field.h - 32,
                    ),
                    t.accent,
                );
                for (key, kr) in &k.keys {
                    let lit = *key == Key::Shift && *shift || *key == Key::Done;
                    button(cx, *kr, &key_label(*key, *shift), lit, false);
                }
            }
            Modal::Confirm { text, what } => {
                cx.canvas.fill_rect(r, t.bg.with_alpha(200));
                let a = confirm_geo(r);
                cx.canvas.fill_round_rect(a.card, t.radius, t.surface);
                cx.canvas.stroke_round_rect(a.card, t.radius, 1, t.outline);
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    a.text,
                    Align::Center,
                    t.text,
                    text,
                );
                button(cx, a.no, "Cancel", false, false);
                let yes = match what {
                    crate::state::Confirm::Delete(_) => "Move to Trash",
                    crate::state::Confirm::EmptyTrash => "Delete for good",
                    crate::state::Confirm::Open(_) | crate::state::Confirm::OpenVersion(..) => {
                        "Open"
                    }
                    crate::state::Confirm::NewProject => "New project",
                };
                button(cx, a.yes, yes, false, true);
            }
        }
        // The Export panel, over everything.
        if let Some(p) = &s.export {
            paint_export(cx, r, p);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let s = &self.screen;
        if let Some(p) = &s.export {
            // The panel takes the screen: only its controls.
            return export_controls(p, r)
                .into_iter()
                .map(|(rr, i, label)| Target::tap("export", i as usize, label, rr))
                .collect();
        }
        match &s.modal {
            Modal::Keyboard { .. } => {
                return keyboard_geo(r)
                    .keys
                    .iter()
                    .enumerate()
                    .map(|(i, (key, kr))| Target::tap("key", i, format!("{key:?}"), *kr))
                    .collect();
            }
            Modal::Confirm { .. } => {
                let a = confirm_geo(r);
                return vec![
                    Target::tap("confirm-yes", 0, "Yes", a.yes),
                    Target::tap("confirm-no", 0, "No", a.no),
                ];
            }
            Modal::None => {}
        }
        let g = Geo::new(r, s);
        let mut out = Vec::new();
        for (k, pr) in g.place_rows.iter().enumerate() {
            let label = s.sources.get(k).map_or("", |&i| SOURCES[i].label());
            out.push(Target::tap("place", k, label, *pr));
        }
        if s.can_go_back {
            out.push(Target::tap("back", 0, "‹ Back", g.back));
        }
        for (b, br) in g.list_buttons.iter().chain(g.detail_buttons.iter()) {
            out.push(Target::tap("button", *b as usize, b.label(), *br));
        }
        for (f, fr) in &g.filters {
            out.push(Target::tap("filter", *f as usize, f.label(), *fr));
        }
        out.push(Target::tap("search", 0, "Search", g.search));
        out.push(Target::tap("sort", 0, "Sort", g.sort));
        let shown = g.visible().min(s.rows.len().saturating_sub(s.top));
        for k in 0..shown {
            let rr = Rect::new(g.rows.x, g.rows.y + k as i32 * ROW_H, g.rows.w, ROW_H);
            out.push(Target::tap(
                "row",
                s.top + k,
                s.rows[s.top + k].name.clone(),
                rr,
            ));
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = Geo::new(r, &self.screen);
        self.visible = g.visible();
        let on_list = matches!(self.screen.modal, Modal::None)
            && self.screen.export.is_none()
            && g.rows.contains(x, y);
        match phase {
            Touch::Down => {
                self.drag = Some((finger, y, 0, false));
                if !on_list {
                    // Buttons act on lift; nothing yet.
                }
                None
            }
            Touch::Move => {
                let (f, y0, done, dragging) = self.drag?;
                if f != finger {
                    return None;
                }
                let dy = y0 - y;
                if !dragging && dy.abs() < DRAG {
                    return None;
                }
                let want = dy / ROW_H;
                self.drag = Some((f, y0, want, true));
                if want != done {
                    self.inputs.push(Input::Scroll(want - done));
                    return Some(EventKind::Value(0.0));
                }
                None
            }
            Touch::Up => {
                let (f, _, _, dragging) = self.drag.take()?;
                if f != finger || dragging {
                    return None;
                }
                let i = self.hit(r, x, y)?;
                self.inputs.push(i);
                Some(EventKind::Value(0.0))
            }
            Touch::Cancel => {
                self.drag = None;
                None
            }
        }
    }
}

fn button(cx: &mut PaintCx, r: Rect, label: &str, lit: bool, danger: bool) {
    let t = cx.theme.clone();
    let fill = if lit { t.accent } else { t.raised };
    cx.canvas.fill_round_rect(r, 10, fill);
    let ink = if lit {
        t.on_accent
    } else if danger {
        t.bad
    } else {
        t.text
    };
    cx.text(FontId::Sans, t.size_body, r, Align::Center, ink, label);
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

fn paint_row(cx: &mut PaintCx, rr: Rect, row: &Row, selected: bool) {
    let t = cx.theme.clone();
    if selected {
        cx.canvas.fill_round_rect(rr, 10, t.raised);
        cx.canvas
            .fill_round_rect(Rect::new(rr.x, rr.y + 12, 4, rr.h - 24), 2, t.accent);
    }
    let inner = rr.inset_xy(14, 0);
    let (icon, rest) = inner.take_left(30);
    let colour = match row.kind {
        Kind::Audio(_) => draw::sound_color(&row.name).map_or(t.accent, |c| t.content(c)),
        Kind::Project => t.accent2,
        Kind::Folder => t.text_dim,
        Kind::Kit => t.good,
        Kind::Preset => t.accent2,
        _ => t.text_faint,
    };
    kind_icon(cx, icon, row.kind, colour);
    let (star, rest) = rest.take_right(if row.favourite { 30 } else { 0 });
    if row.favourite {
        star_shape(cx, star.x + star.w / 2, star.y + star.h / 2, 9, t.warn);
    }
    let dw = if row.detail.is_empty() {
        0
    } else {
        cx.measure(FontId::Sans, t.size_small, &row.detail)
            .min(rest.w / 2)
            + 16
    };
    let (detail, name) = rest.take_right(dw);
    let name = name.inset_xy(10, 0);
    let fitted = cx
        .fonts
        .face(if selected { FontId::Bold } else { FontId::Sans })
        .fit(t.size_body, &row.name, name.w);
    cx.text(
        if selected { FontId::Bold } else { FontId::Sans },
        t.size_body,
        name,
        Align::Left,
        t.text,
        &fitted,
    );
    if dw > 0 {
        let fitted = cx
            .fonts
            .face(FontId::Sans)
            .fit(t.size_small, &row.detail, detail.w);
        cx.text(
            FontId::Sans,
            t.size_small,
            detail,
            Align::Right,
            t.text_dim,
            &fitted,
        );
    }
}

fn paint_overview(cx: &mut PaintCx, r: Rect, o: &[(f32, f32)], c: Color) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, 8, t.bg.with_alpha(160));
    if o.is_empty() || r.w <= 4 {
        return;
    }
    let mid = r.y + r.h / 2;
    let half = (r.h / 2 - 6) as f32;
    cx.canvas
        .fill_rect(Rect::new(r.x + 4, mid, r.w - 8, 1), t.outline);
    let w = r.w - 8;
    for x in 0..w {
        let i = (x as usize * o.len()) / w as usize;
        let (lo, hi) = o[i.min(o.len() - 1)];
        let y0 = mid - (hi * half) as i32;
        let y1 = mid - (lo * half) as i32;
        cx.canvas
            .fill_rect(Rect::new(r.x + 4 + x, y0, 1, (y1 - y0).max(1)), c);
    }
}

/// Small line icons, drawn so no picture files are needed.
fn kind_icon(cx: &mut PaintCx, r: Rect, k: Kind, c: Color) {
    let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
    match k {
        Kind::Folder => {
            cx.canvas
                .fill_round_rect(Rect::new(x - 12, y - 9, 10, 5), 2, c);
            cx.canvas
                .fill_round_rect(Rect::new(x - 12, y - 6, 24, 16), 3, c);
        }
        Kind::Project => {
            cx.canvas
                .stroke_round_rect(Rect::new(x - 8, y - 11, 20, 18), 3, 2, c.with_alpha(140));
            cx.canvas
                .fill_round_rect(Rect::new(x - 12, y - 7, 20, 18), 3, c);
        }
        Kind::Audio(f) => {
            let hs = [4, 9, 14, 7, 11, 5];
            for (i, h) in hs.iter().enumerate() {
                cx.canvas.fill_round_rect(
                    Rect::new(x - 12 + i as i32 * 4, y - h / 2 - 1, 3, *h + 2),
                    1,
                    c,
                );
            }
            if f == AudioFormat::Flac {
                cx.canvas.fill_rect(Rect::new(x + 12, y - 9, 2, 2), c);
            }
        }
        Kind::Kit => {
            for (dx, dy) in [(-11, -10), (1, -10), (-11, 1), (1, 1)] {
                cx.canvas
                    .fill_round_rect(Rect::new(x + dx, y + dy, 10, 9), 2, c);
            }
        }
        Kind::Preset => {
            // A knob: a ring with its pointer.
            cx.canvas
                .stroke_round_rect(Rect::new(x - 10, y - 10, 20, 20), 10, 2, c);
            cx.canvas.fill_rect(Rect::new(x - 1, y - 9, 2, 9), c);
        }
        Kind::Pattern => {
            for i in 0..4 {
                let h = if i % 2 == 0 { 8 } else { 14 };
                cx.canvas
                    .fill_rect(Rect::new(x - 11 + i * 6, y + 7 - h, 4, h), c);
            }
        }
        Kind::Other => {
            cx.canvas
                .stroke_round_rect(Rect::new(x - 8, y - 11, 16, 22), 2, 2, c);
        }
    }
}

fn source_icon(cx: &mut PaintCx, r: Rect, s: Source, c: Color) {
    match s {
        Source::Place(Place::Projects) => kind_icon(cx, r, Kind::Project, c),
        Source::Place(Place::Samples) => kind_icon(cx, r, Kind::Audio(AudioFormat::Wav), c),
        Source::Place(Place::Kits) | Source::Factory => kind_icon(cx, r, Kind::Kit, c),
        Source::Place(Place::Instruments) => kind_icon(cx, r, Kind::Preset, c),
        Source::Place(Place::Patterns) => kind_icon(cx, r, Kind::Pattern, c),
        Source::Place(Place::Recordings) => {
            draw::dot(cx, r.x + r.w / 2, r.y + r.h / 2, 8, c);
        }
        Source::Place(Place::Trash) => {
            let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
            cx.canvas.fill_rect(Rect::new(x - 10, y - 9, 20, 2), c);
            cx.canvas.fill_rect(Rect::new(x - 3, y - 12, 6, 3), c);
            cx.canvas
                .stroke_round_rect(Rect::new(x - 8, y - 6, 16, 18), 2, 2, c);
        }
        Source::Recent => {
            let (x, y) = (r.x + r.w / 2, r.y + r.h / 2);
            cx.canvas
                .stroke_round_rect(Rect::new(x - 10, y - 10, 20, 20), 10, 2, c);
            cx.canvas.fill_rect(Rect::new(x - 1, y - 6, 2, 7), c);
            cx.canvas.fill_rect(Rect::new(x - 1, y - 1, 6, 2), c);
        }
        Source::Favourites => star_shape(cx, r.x + r.w / 2, r.y + r.h / 2, 11, c),
    }
}

/// A five-pointed star of outer radius `rad` centred on (x, y), drawn
/// through a coverage mask (4 x 4 samples a pixel), so no font needs the
/// star character.
fn star_shape(cx: &mut PaintCx, x: i32, y: i32, rad: i32, c: Color) {
    let n = (2 * rad + 1) as usize;
    let pts: Vec<(f32, f32)> = (0..10)
        .map(|i| {
            let a = std::f32::consts::PI * (i as f32 / 5.0) - std::f32::consts::FRAC_PI_2;
            let r = if i % 2 == 0 {
                rad as f32
            } else {
                rad as f32 * 0.45
            };
            (r * a.cos(), r * a.sin())
        })
        .collect();
    let inside = |px: f32, py: f32| {
        let mut inn = false;
        let mut j = pts.len() - 1;
        for i in 0..pts.len() {
            let (xi, yi) = pts[i];
            let (xj, yj) = pts[j];
            if (yi > py) != (yj > py) && px < (xj - xi) * (py - yi) / (yj - yi) + xi {
                inn = !inn;
            }
            j = i;
        }
        inn
    };
    let mut mask = vec![0u8; n * n];
    for my in 0..n {
        for mx in 0..n {
            let mut hits = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = mx as f32 - rad as f32 + (sx as f32 + 0.5) / 4.0 - 0.5;
                    let py = my as f32 - rad as f32 + (sy as f32 + 0.5) / 4.0 - 0.5;
                    if inside(px, py) {
                        hits += 1;
                    }
                }
            }
            mask[my * n + mx] = (hits * 255 / 16) as u8;
        }
    }
    cx.canvas.blend_mask(x - rad, y - rad, n, n, &mask, c);
}
