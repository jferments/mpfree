//! The widgets the time engine adds to mpfree-ui: the transport bar
//! ([`TransportBar`]), the Clips launch grid ([`ClipGrid`]), the scene
//! morph bar under it ([`MorphBar`]) and the takes list ([`TakeList`]). Like mpfree-ui's own widgets, each one only shows
//! what the app put in its fields and reports touches; none of them knows
//! about the transport, the clips or the audio thread.

use mpfree_ui::widgets::Chip;
use mpfree_ui::{
    Align, Canvas, Color, EventKind, FontId, PaintCx, Rect, Target, Tone, Touch, Widget,
};
use std::cell::RefCell;
use std::f32::consts::TAU;

// ---------------------------------------------------------------- drawing

/// A progress ring: a circle `size` pixels across with its top left corner
/// at (x, y), `width` pixels thick. The part from twelve o'clock clockwise
/// to `frac` (0.0 to 1.0) is drawn in `color`, the rest in `rest`.
#[allow(clippy::too_many_arguments)]
pub fn ring(
    canvas: &mut dyn Canvas,
    x: i32,
    y: i32,
    size: i32,
    width: f32,
    frac: f32,
    rest: Color,
    color: Color,
) {
    if size <= 0 {
        return;
    }
    let n = size as usize;
    let outer = size as f32 / 2.0;
    let inner = outer - width;
    let end = frac.clamp(0.0, 1.0) * TAU;
    let mut done = vec![0u8; n * n];
    let mut left = vec![0u8; n * n];
    for py in 0..n {
        for px in 0..n {
            let dx = px as f32 + 0.5 - outer;
            let dy = py as f32 + 0.5 - outer;
            let d = (dx * dx + dy * dy).sqrt();
            let cov = (outer - d + 0.5).clamp(0.0, 1.0) * (d - inner + 0.5).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            // Clockwise from twelve o'clock.
            let mut a = dx.atan2(-dy);
            if a < 0.0 {
                a += TAU;
            }
            let v = (cov * 255.0).round() as u8;
            if a < end {
                done[py * n + px] = v;
            } else {
                left[py * n + px] = v;
            }
        }
    }
    canvas.blend_mask(x, y, n, n, &left, rest);
    canvas.blend_mask(x, y, n, n, &done, color);
}

/// A triangle pointing right that fills `r` (the play symbol), smoothed by
/// sampling each pixel 4 x 4 times.
pub fn play_symbol(canvas: &mut dyn Canvas, r: Rect, color: Color) {
    if r.is_empty() {
        return;
    }
    let (w, h) = (r.w as usize, r.h as usize);
    let (fw, half) = (r.w as f32, r.h as f32 / 2.0);
    let mut mask = vec![0u8; w * h];
    for py in 0..h {
        for px in 0..w {
            let mut hits = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let fx = px as f32 + (sx as f32 + 0.5) / 4.0;
                    let fy = py as f32 + (sy as f32 + 0.5) / 4.0;
                    if fx <= fw * (1.0 - (fy - half).abs() / half) {
                        hits += 1;
                    }
                }
            }
            mask[py * w + px] = (hits * 255 / 16) as u8;
        }
    }
    canvas.blend_mask(r.x, r.y, w, h, &mask, color);
}

// ---------------------------------------------------------------- transport bar

/// What the transport is doing, as the bar shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Run {
    #[default]
    Stopped,
    CountIn,
    Playing,
    Recording,
}

/// The bar's buttons. A tap on one gives `Tab(index)` in this order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarButton {
    Stop,
    Play,
    Rec,
    /// Replace or overdub.
    Mode,
    Undo,
}

impl BarButton {
    pub const ALL: [BarButton; 5] = [
        BarButton::Stop,
        BarButton::Play,
        BarButton::Rec,
        BarButton::Mode,
        BarButton::Undo,
    ];

    pub fn index(self) -> usize {
        BarButton::ALL.iter().position(|b| *b == self).unwrap_or(0)
    }
}

/// One reading on the bar, such as "BPM 92.0": the label is drawn dim, the
/// value bold in its tone.
#[derive(Clone, Debug, PartialEq)]
pub struct BarItem {
    pub label: String,
    pub value: String,
    pub tone: Tone,
}

impl BarItem {
    pub fn new(label: &str, value: impl Into<String>, tone: Tone) -> BarItem {
        BarItem {
            label: label.to_string(),
            value: value.into(),
            tone,
        }
    }
}

/// The transport bar under the header: stop, play and record, replace or
/// overdub, the song position, readings (tempo, meter, swing, loop ...),
/// Undo, and status chips on the right (the arp's chip goes there). A tap on
/// a button gives `Tab(BarButton index)`; a tap on chip `i` gives
/// `Select(i)`. Readings that do not fit are left out from the right.
pub struct TransportBar {
    pub run: Run,
    /// Bar.beat.tick.
    pub position: String,
    pub items: Vec<BarItem>,
    /// Overdub (true) or replace (false).
    pub overdub: bool,
    /// What Undo would take back, such as "take 3"; None when there is
    /// nothing to undo.
    pub undo: Option<String>,
    pub chips: Vec<Chip>,
    /// Where the buttons and chips were drawn last, for touches.
    hits: RefCell<Vec<(Rect, EventKind)>>,
}

pub const BAR_BUTTON_W: i32 = 46;
const MODE_W: i32 = 118;
const UNDO_W: i32 = 140;
const POSITION_PX: f32 = 22.0;

impl Default for TransportBar {
    fn default() -> Self {
        TransportBar::new()
    }
}

impl TransportBar {
    pub fn new() -> TransportBar {
        TransportBar {
            run: Run::Stopped,
            position: "1.1.000".to_string(),
            items: Vec::new(),
            overdub: false,
            undo: None,
            chips: Vec::new(),
            hits: RefCell::new(Vec::new()),
        }
    }

    /// The rectangle of a stop, play or record button inside `r` (the
    /// others depend on text widths and are known after drawing).
    pub fn button_rect(r: Rect, b: BarButton) -> Option<Rect> {
        let inner = r.inset_xy(4, 4);
        let i = match b {
            BarButton::Stop => 0,
            BarButton::Play => 1,
            BarButton::Rec => 2,
            _ => return None,
        };
        Some(Rect::new(
            inner.x + i * (BAR_BUTTON_W + 4),
            inner.y,
            BAR_BUTTON_W,
            inner.h,
        ))
    }
}

impl Widget for TransportBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let mut hits = Vec::new();
        cx.canvas.fill_round_rect(r, t.radius, t.surface);
        let inner = r.inset_xy(4, 4);
        // Stop, play, record.
        let (lit, ink) = match self.run {
            Run::Stopped => (BarButton::Stop, t.text),
            Run::Playing => (BarButton::Play, t.bg),
            Run::Recording | Run::CountIn => (BarButton::Rec, t.bg),
        };
        let fill = match self.run {
            Run::Stopped => t.raised,
            Run::Playing => t.good,
            Run::Recording => t.bad,
            Run::CountIn => t.warn,
        };
        for b in [BarButton::Stop, BarButton::Play, BarButton::Rec] {
            let Some(br) = TransportBar::button_rect(r, b) else {
                continue;
            };
            let on = b == lit;
            if on {
                cx.canvas.fill_round_rect(br, 10, fill);
            }
            let c = if on { ink } else { t.text_dim };
            let (mx, my) = (br.x + br.w / 2, br.y + br.h / 2);
            match b {
                BarButton::Stop => {
                    cx.canvas
                        .fill_round_rect(Rect::new(mx - 6, my - 6, 12, 12), 2, c)
                }
                BarButton::Play => play_symbol(cx.canvas, Rect::new(mx - 5, my - 7, 12, 14), c),
                _ => {
                    let c = if on { ink } else { t.bad.mix(t.text_dim, 0.35) };
                    cx.canvas
                        .fill_round_rect(Rect::new(mx - 7, my - 7, 14, 14), 7, c)
                }
            }
            hits.push((br, EventKind::Tab(b.index())));
        }
        let mut x = TransportBar::button_rect(r, BarButton::Rec).map_or(inner.x, |b| b.right()) + 8;
        // Replace or overdub.
        let mode = Rect::new(x, inner.y, MODE_W, inner.h);
        cx.canvas.fill_round_rect(mode, 10, t.raised);
        let dot = if self.overdub { t.accent } else { t.warn };
        cx.canvas.fill_round_rect(
            Rect::new(mode.x + 14, mode.y + mode.h / 2 - 4, 8, 8),
            4,
            dot,
        );
        let word = if self.overdub { "Overdub" } else { "Replace" };
        cx.text(
            FontId::Bold,
            t.size_small + 1.0,
            Rect::new(mode.x + 30, mode.y, mode.w - 36, mode.h),
            Align::Left,
            t.text,
            word,
        );
        hits.push((mode, EventKind::Tab(BarButton::Mode.index())));
        x = mode.right() + 16;
        // Chips from the right edge leftwards, then Undo.
        let mut right = inner.right();
        for (i, chip) in self.chips.iter().enumerate() {
            let tw = cx.measure(FontId::Sans, t.size_small, &chip.text);
            let w = tw + 40;
            if right - w < x + UNDO_W + 200 {
                break;
            }
            let cr = Rect::new(right - w, inner.y, w, inner.h);
            cx.canvas.fill_round_rect(cr, cr.h / 2, t.raised);
            cx.canvas.fill_round_rect(
                Rect::new(cr.x + 14, cr.y + cr.h / 2 - 4, 8, 8),
                4,
                t.tone(chip.tone),
            );
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(cr.x + 28, cr.y, tw + 4, cr.h),
                Align::Left,
                t.text,
                &chip.text,
            );
            hits.push((cr, EventKind::Select(i)));
            right -= w + 8;
        }
        let undo = Rect::new(right - UNDO_W, inner.y, UNDO_W, inner.h);
        match &self.undo {
            Some(what) => {
                cx.canvas.fill_round_rect(undo, 10, t.raised);
                cx.text(
                    FontId::Bold,
                    t.size_small + 1.0,
                    undo,
                    Align::Center,
                    t.text,
                    &format!("Undo {what}"),
                );
                hits.push((undo, EventKind::Tab(BarButton::Undo.index())));
            }
            None => {
                cx.canvas.stroke_round_rect(undo, 10, 1, t.outline);
                cx.text(
                    FontId::Sans,
                    t.size_small + 1.0,
                    undo,
                    Align::Center,
                    t.text_faint,
                    "Undo",
                );
            }
        }
        let limit = undo.x - 12;
        // The song position, coloured by what the transport is doing.
        let pc = match self.run {
            Run::Stopped => t.text,
            Run::Playing => t.good,
            Run::Recording => t.bad,
            Run::CountIn => t.warn,
        };
        let pw = cx
            .measure(FontId::Mono, POSITION_PX, &self.position)
            .max(cx.measure(FontId::Mono, POSITION_PX, "88.8.888"));
        if x + pw <= limit {
            cx.text(
                FontId::Mono,
                POSITION_PX,
                Rect::new(x, inner.y, pw + 4, inner.h),
                Align::Left,
                pc,
                &self.position,
            );
            x += pw + 4;
        }
        // Readings, each after a thin rule, while they fit.
        for item in &self.items {
            let lw = if item.label.is_empty() {
                0
            } else {
                cx.measure(FontId::Sans, t.size_small, &item.label) + 6
            };
            let vw = cx.measure(FontId::Bold, t.size_body, &item.value);
            let w = 32 + lw + vw;
            if x + w > limit {
                break;
            }
            cx.canvas
                .fill_rect(Rect::new(x + 16, inner.y + 9, 1, inner.h - 18), t.outline);
            let mut tx = x + 32;
            if lw > 0 {
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    Rect::new(tx, inner.y, lw, inner.h),
                    Align::Left,
                    t.text_dim,
                    &item.label,
                );
                tx += lw;
            }
            cx.text(
                FontId::Bold,
                t.size_body,
                Rect::new(tx, inner.y, vw + 2, inner.h),
                Align::Left,
                t.tone(item.tone),
                &item.value,
            );
            x += w;
        }
        *self.hits.borrow_mut() = hits;
    }

    fn targets(&self, _r: Rect) -> Vec<Target> {
        // What the last paint drew to be touched: the transport buttons,
        // the mode, the chips and Undo.
        let names = ["Stop", "Play", "Rec", "Mode", "Undo"];
        self.hits
            .borrow()
            .iter()
            .map(|(h, k)| match k {
                EventKind::Tab(i) => {
                    let id = ["stop", "play", "rec", "mode", "undo"][(*i).min(4)];
                    Target::tap(id, 0, names[(*i).min(4)], *h)
                }
                EventKind::Select(i) => {
                    let label = self.chips.get(*i).map_or(String::new(), |c| c.text.clone());
                    Target::tap("chip", *i, label, *h)
                }
                _ => Target::tap("other", 0, "", *h),
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, _r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        self.hits
            .borrow()
            .iter()
            .find(|(h, _)| h.contains(x, y))
            .map(|(_, k)| k.clone())
    }
}

// ---------------------------------------------------------------- clip grid

/// What a cell of the grid shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellState {
    /// No clip in the slot.
    #[default]
    Empty,
    /// A clip, not playing.
    Stopped,
    /// Launched; starts later (on the next bar).
    Queued,
    Playing,
    /// Playing, and the transport records into it.
    Recording,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridCell {
    pub state: CellState,
    pub name: String,
    /// Under the name, such as "2 bars".
    pub detail: String,
    /// At the bottom, such as "24 notes" or "from 7.1".
    pub foot: String,
    /// How far through its pass the clip is, 0.0 to 1.0 (the ring).
    pub progress: f32,
}

/// One column of the grid: a track and its slots.
#[derive(Clone, Debug, PartialEq)]
pub struct GridTrack {
    pub name: String,
    pub color: Color,
    /// Recording goes into this track.
    pub armed: bool,
    pub mute: bool,
    pub cells: Vec<GridCell>,
}

/// The launch grid: tracks across, their clip slots down, as on the MPC's
/// clip launch and in Ableton's session view. A playing clip shows a ring
/// that goes round once per pass; a queued one an amber outline; the one
/// being recorded a red outline. A tap on a track's name gives
/// `Tab(track)`; a tap on a cell gives `Select(track * slots + slot)`.
/// The app scrolls it with `first_track` and `first_row`.
pub struct ClipGrid {
    pub title: String,
    /// On the right of the title line.
    pub legend: String,
    pub tracks: Vec<GridTrack>,
    /// Slots per track.
    pub slots: usize,
    pub first_track: usize,
    pub first_row: usize,
    /// The scene (row) playing, lit on its row button.
    pub scene_playing: Option<usize>,
    /// The scene launched and waiting for the next bar line.
    pub scene_cued: Option<usize>,
    /// Live to song armed: what the grid plays is recorded into the song.
    pub song_rec: bool,
    /// What was drawn last, so a change redraws only the cells that changed.
    drawn: RefCell<Option<Drawn>>,
}

struct Drawn {
    at: Rect,
    first: (usize, usize),
    title: String,
    legend: String,
    tracks: Vec<GridTrack>,
    scenes: (Option<usize>, Option<usize>),
    song_rec: bool,
}

/// Tracks shown at once.
pub const GRID_COLS: usize = 8;
/// The Rec song button's width (on the grid and the Song view).
pub const REC_SONG_W: i32 = 112;

/// The Rec song button: live to song, red while armed.
pub fn rec_song_pill(cx: &mut PaintCx, r: Rect, on: bool) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(r, 10, if on { t.bad } else { t.raised });
    let ink = if on { t.text } else { t.text_dim };
    cx.canvas.fill_round_rect(
        Rect::new(r.x + 12, r.y + r.h / 2 - 5, 10, 10),
        5,
        if on { t.text } else { t.bad },
    );
    cx.text(
        FontId::Bold,
        t.size_small,
        Rect::new(r.x + 28, r.y, r.w - 34, r.h),
        Align::Left,
        ink,
        "Rec song",
    );
}
/// The scene buttons' column: each row's number launches the row.
const LABEL_W: i32 = 48;
const HEAD_H: i32 = 28;
const CELL_GAP: i32 = 6;
const ROW_MIN_H: i32 = 76;
const RING: i32 = 26;

struct Geo {
    body: Rect,
    x0: i32,
    col_w: i32,
    cells_y: i32,
    row_h: i32,
    rows: usize,
}

impl ClipGrid {
    pub fn new(title: &str, slots: usize) -> ClipGrid {
        ClipGrid {
            title: title.to_string(),
            legend: String::new(),
            tracks: Vec::new(),
            slots,
            first_track: 0,
            first_row: 0,
            scene_playing: None,
            scene_cued: None,
            song_rec: false,
            drawn: RefCell::new(None),
        }
    }

    /// Where row `slot`'s scene button is, if it is in view.
    pub fn scene_rect(&self, r: Rect, slot: usize) -> Option<Rect> {
        let g = self.geo(r);
        if slot < self.first_row || slot >= self.first_row + g.rows || slot >= self.slots {
            return None;
        }
        let row = (slot - self.first_row) as i32;
        Some(Rect::new(
            g.body.x,
            g.cells_y + row * (g.row_h + CELL_GAP),
            LABEL_W,
            g.row_h,
        ))
    }

    /// Where the Song button is: the right end of the title line.
    pub fn song_rect(&self, r: Rect) -> Rect {
        let head = r.inset(14).take_top(26).0;
        Rect::new(head.right() - 72, head.y - 4, 72, 30)
    }

    /// Where the Rec song button is: left of the Song button.
    pub fn rec_rect(&self, r: Rect) -> Rect {
        let song = self.song_rect(r);
        Rect::new(song.x - 8 - REC_SONG_W, song.y, REC_SONG_W, song.h)
    }

    /// Where the Edit button is (the note editor): left of Rec song.
    pub fn edit_rect(&self, r: Rect) -> Rect {
        let rec = self.rec_rect(r);
        Rect::new(rec.x - 8 - 72, rec.y, 72, rec.h)
    }

    /// Where the Stop all button is: over the scene buttons.
    pub fn stop_all_rect(&self, r: Rect) -> Rect {
        let g = self.geo(r);
        Rect::new(g.body.x, g.body.y, LABEL_W, HEAD_H)
    }

    fn geo(&self, r: Rect) -> Geo {
        // Matches PaintCx::titled_panel with the default theme's padding.
        let body = r.inset(14).take_top(34).1;
        let x0 = body.x + LABEL_W + CELL_GAP;
        let cols = GRID_COLS as i32;
        let col_w = ((body.right() - x0 - CELL_GAP * (cols - 1)) / cols).max(1);
        let cells_y = body.y + HEAD_H + CELL_GAP;
        let avail = body.bottom() - cells_y;
        let rows =
            (((avail + CELL_GAP) / (ROW_MIN_H + CELL_GAP)).max(1) as usize).min(self.slots.max(1));
        let row_h = ((avail - CELL_GAP * (rows as i32 - 1)) / rows as i32).max(1);
        Geo {
            body,
            x0,
            col_w,
            cells_y,
            row_h,
            rows,
        }
    }

    /// Rows (slots) shown at once in `r`.
    pub fn visible_rows(&self, r: Rect) -> usize {
        self.geo(r).rows
    }

    fn col(&self, track: usize) -> Option<i32> {
        (track >= self.first_track
            && track < self.first_track + GRID_COLS
            && track < self.tracks.len())
        .then(|| (track - self.first_track) as i32)
    }

    /// Where track `track`'s name is, if it is in view.
    pub fn head_rect(&self, r: Rect, track: usize) -> Option<Rect> {
        let g = self.geo(r);
        let c = self.col(track)?;
        Some(Rect::new(
            g.x0 + c * (g.col_w + CELL_GAP),
            g.body.y,
            g.col_w,
            HEAD_H,
        ))
    }

    /// Where slot `slot` of track `track` is, if it is in view.
    pub fn cell_rect(&self, r: Rect, track: usize, slot: usize) -> Option<Rect> {
        let g = self.geo(r);
        let c = self.col(track)?;
        if slot < self.first_row || slot >= self.first_row + g.rows || slot >= self.slots {
            return None;
        }
        let row = (slot - self.first_row) as i32;
        Some(Rect::new(
            g.x0 + c * (g.col_w + CELL_GAP),
            g.cells_y + row * (g.row_h + CELL_GAP),
            g.col_w,
            g.row_h,
        ))
    }
}

impl Widget for ClipGrid {
    fn changed_parts(&self, r: Rect) -> Option<Vec<Rect>> {
        let drawn = self.drawn.borrow();
        let d = drawn.as_ref()?;
        if d.at != r
            || d.first != (self.first_track, self.first_row)
            || d.title != self.title
            || d.legend != self.legend
            || d.tracks.len() != self.tracks.len()
            || d.scenes != (self.scene_playing, self.scene_cued)
            || d.song_rec != self.song_rec
        {
            return None;
        }
        let mut parts = Vec::new();
        for (i, (old, new)) in d.tracks.iter().zip(&self.tracks).enumerate() {
            if old.name != new.name
                || old.color != new.color
                || old.armed != new.armed
                || old.mute != new.mute
                || old.cells.len() != new.cells.len()
            {
                return None;
            }
            let changed = (0..new.cells.len()).filter(|&s| old.cells[s] != new.cells[s]);
            parts.extend(changed.filter_map(|s| self.cell_rect(r, i, s)));
        }
        Some(parts)
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        *self.drawn.borrow_mut() = Some(Drawn {
            at: r,
            first: (self.first_track, self.first_row),
            title: self.title.clone(),
            legend: self.legend.clone(),
            tracks: self.tracks.clone(),
            scenes: (self.scene_playing, self.scene_cued),
            song_rec: self.song_rec,
        });
        cx.titled_panel(r, &self.title, "");
        // The legend, then Rec song and the Song button (the Song view of
        // the page).
        let song = self.song_rect(r);
        let rec = self.rec_rect(r);
        let edit = self.edit_rect(r);
        let head = r.inset(14).take_top(26).0;
        // The legend's parts that fit between the title and the buttons,
        // dropping the first ones when it is too wide.
        let title_w = cx.measure(FontId::Bold, t.size_small, &self.title.to_uppercase());
        let room = edit.x - 12 - (head.x + title_w + 16);
        let mut legend = self.legend.as_str();
        while !legend.is_empty() && cx.measure(FontId::Sans, t.size_small, legend) > room {
            legend = legend.split_once(" · ").map_or("", |(_, rest)| rest);
        }
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(head.x, head.y, edit.x - head.x - 12, head.h),
            Align::Right,
            t.text_dim,
            legend,
        );
        rec_song_pill(cx, rec, self.song_rec);
        cx.canvas.fill_round_rect(edit, 10, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            edit,
            Align::Center,
            t.text,
            "Edit ›",
        );
        cx.canvas.fill_round_rect(song, 10, t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            song,
            Align::Center,
            t.text,
            "Song ›",
        );
        let g = self.geo(r);
        if self.tracks.is_empty() {
            let line = Rect::new(g.body.x, g.body.y, g.body.w, 30);
            cx.text(
                FontId::Sans,
                t.size_body,
                line,
                Align::Left,
                t.text_faint,
                "No tracks yet",
            );
            return;
        }
        for row in 0..g.rows {
            let slot = self.first_row + row;
            if slot >= self.slots {
                break;
            }
            let Some(b) = self.scene_rect(r, slot) else {
                break;
            };
            // A scene button: a play mark over the row's number, lit while
            // the scene plays, ringed while it waits for the bar line.
            let playing = self.scene_playing == Some(slot);
            let fill = if playing { t.good } else { t.raised };
            let ink = if playing { t.bg } else { t.text_dim };
            cx.canvas.fill_round_rect(b, 10, fill);
            if self.scene_cued == Some(slot) {
                cx.canvas.stroke_round_rect(b, 10, 2, t.warn);
            }
            let mx = b.x + b.w / 2;
            let top = b.y + (b.h / 2 - 18).max(4);
            play_symbol(cx.canvas, Rect::new(mx - 5, top, 10, 12), ink);
            cx.text(
                FontId::Bold,
                t.size_small,
                Rect::new(b.x, top + 14, b.w, 22),
                Align::Center,
                ink,
                &format!("{}", slot + 1),
            );
        }
        let stop = self.stop_all_rect(r);
        cx.canvas.fill_round_rect(stop, 8, t.raised);
        let (mx, my) = (stop.x + stop.w / 2, stop.y + stop.h / 2);
        cx.canvas
            .fill_round_rect(Rect::new(mx - 5, my - 5, 10, 10), 2, t.text_dim);
        for (ti, track) in self.tracks.iter().enumerate() {
            let Some(head) = self.head_rect(r, ti) else {
                continue;
            };
            let base = if track.mute {
                track.color.mix(t.surface, 0.6)
            } else {
                track.color
            };
            cx.canvas
                .fill_round_rect(head, 8, base.mix(t.surface, 0.72));
            let mut name = head.inset_xy(10, 0);
            if track.armed {
                cx.canvas.stroke_round_rect(head, 8, 2, t.bad);
                cx.canvas.fill_round_rect(
                    Rect::new(name.x, head.y + head.h / 2 - 4, 8, 8),
                    4,
                    t.bad,
                );
                name = name.take_left(14).1;
            }
            let ink = if track.mute {
                t.text_faint
            } else {
                base.mix(t.text, 0.25)
            };
            cx.text(
                FontId::Bold,
                t.size_small,
                name,
                Align::Left,
                ink,
                &track.name,
            );
            for (slot, cell) in track.cells.iter().enumerate() {
                let Some(cr) = self.cell_rect(r, ti, slot) else {
                    continue;
                };
                paint_cell(cx, cr, cell, base);
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let mut out = vec![
            Target::tap("song", 0, "Song", self.song_rect(r)),
            Target::tap("rec-song", 0, "Rec song", self.rec_rect(r)),
            Target::tap("edit", 0, "Edit", self.edit_rect(r)),
        ];
        if self.tracks.is_empty() {
            return out;
        }
        out.push(Target::tap(
            "stop-all",
            0,
            "Stop all",
            self.stop_all_rect(r),
        ));
        for slot in self.first_row..self.slots {
            if let Some(b) = self.scene_rect(r, slot) {
                out.push(Target::tap("scene", slot, format!("Scene {}", slot + 1), b));
            }
        }
        let tracks = self.first_track..(self.first_track + GRID_COLS).min(self.tracks.len());
        for track in tracks {
            let name = &self.tracks[track].name;
            if let Some(h) = self.head_rect(r, track) {
                out.push(Target::tap("track", track, name.clone(), h));
            }
            for slot in self.first_row..self.slots {
                if let Some(c) = self.cell_rect(r, track, slot) {
                    let label = format!("{name} slot {}", slot + 1);
                    out.push(Target::tap("slot", track * self.slots + slot, label, c));
                }
            }
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        // A row's scene button is Toggle(row); Stop all is Toggle(slots);
        // the Song button Toggle(slots + 1); Rec song Toggle(slots + 2);
        // Edit Toggle(slots + 3).
        if self.song_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots + 1));
        }
        if self.rec_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots + 2));
        }
        if self.edit_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots + 3));
        }
        if self.tracks.is_empty() {
            return None;
        }
        if self.stop_all_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots));
        }
        for slot in self.first_row..self.slots {
            if self.scene_rect(r, slot).is_some_and(|b| b.contains(x, y)) {
                return Some(EventKind::Toggle(slot));
            }
        }
        let tracks = self.first_track..(self.first_track + GRID_COLS).min(self.tracks.len());
        for track in tracks {
            if self.head_rect(r, track).is_some_and(|h| h.contains(x, y)) {
                return Some(EventKind::Tab(track));
            }
            for slot in self.first_row..self.slots {
                if self
                    .cell_rect(r, track, slot)
                    .is_some_and(|c| c.contains(x, y))
                {
                    return Some(EventKind::Select(track * self.slots + slot));
                }
            }
        }
        None
    }
}

/// The scene morph under the grid: the rows on A and B (a tap on a chip's
/// left half steps it down, on its right half up: `Step { index: 0 or 1,
/// delta }`), the crossfader between them (`Value`, while the finger is
/// down), and Snap back, Keep and Save (`Tab(0)`, `Tab(1)`, `Tab(2)`).
#[derive(Clone, Debug, PartialEq)]
pub struct MorphBar {
    /// The rows on A and B, shown 1-based.
    pub a: usize,
    pub b: usize,
    /// The crossfader, 0 (A) to 1 (B).
    pub pos: f32,
    /// A morph is loaded: the crossfader moves the parameters.
    pub on: bool,
    /// Parameters the morph moves, for the label.
    pub params: usize,
}

const CHIP_W: i32 = 96;
const MORPH_BUTTONS: [(&str, i32); 3] = [("Snap back", 104), ("Keep", 72), ("Save", 84)];

impl Default for MorphBar {
    fn default() -> MorphBar {
        MorphBar {
            a: 0,
            b: 1,
            pos: 0.0,
            on: false,
            params: 0,
        }
    }
}

impl MorphBar {
    pub fn new() -> MorphBar {
        MorphBar::default()
    }

    /// The A chip, the crossfader, the B chip and the three buttons.
    pub fn parts(r: Rect) -> (Rect, Rect, Rect, [Rect; 3]) {
        let inner = r.inset_xy(14, 8);
        let gap = 8;
        let buttons_w: i32 = MORPH_BUTTONS.iter().map(|b| b.1 + gap).sum();
        let (buttons, rest) = inner.take_right(buttons_w);
        let mut x = buttons.x + gap;
        let mut bs = [Rect::new(0, 0, 0, 0); 3];
        for (i, (_, w)) in MORPH_BUTTONS.iter().enumerate() {
            bs[i] = Rect::new(x, inner.y, *w, inner.h);
            x += w + gap;
        }
        let (a, rest) = rest.take_left(CHIP_W);
        let (b, rest) = rest.take_right(CHIP_W);
        let fader = rest.inset_xy(16, 0);
        (a, fader, b, bs)
    }

    /// The label of the Save button: the row the crossfader is nearer.
    pub fn save_row(&self) -> usize {
        if self.pos < 0.5 {
            self.a
        } else {
            self.b
        }
    }
}

impl Widget for MorphBar {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(r.inset_xy(4, 2), 12, t.surface);
        let (a, fader, b, buttons) = MorphBar::parts(r);
        for (chip, side, row, near) in [
            (a, "A", self.a, self.pos < 0.5),
            (b, "B", self.b, self.pos >= 0.5),
        ] {
            cx.canvas.fill_round_rect(chip, 10, t.raised);
            if self.on && near {
                cx.canvas.stroke_round_rect(chip, 10, 2, t.accent);
            }
            cx.text(
                FontId::Bold,
                t.size_small,
                chip,
                Align::Center,
                t.text,
                &format!("‹  {side} {}  ›", row + 1),
            );
        }
        // The crossfader: a track, the part from A filled while on, a knob.
        let my = fader.y + fader.h / 2;
        let track = Rect::new(fader.x, my - 3, fader.w, 6);
        cx.canvas.fill_round_rect(track, 3, t.raised);
        let x = fader.x + (self.pos.clamp(0.0, 1.0) * fader.w as f32).round() as i32;
        let ink = if self.on { t.accent } else { t.text_faint };
        if self.on {
            cx.canvas
                .fill_round_rect(Rect::new(fader.x, my - 3, x - fader.x, 6), 3, ink);
        }
        cx.canvas
            .fill_round_rect(Rect::new(x - 9, fader.y + 4, 18, fader.h - 8), 6, ink);
        let label = if self.on {
            format!("Morph · {} parameters", self.params)
        } else {
            "Morph".to_string()
        };
        cx.text(
            FontId::Sans,
            t.size_small - 2.0,
            Rect::new(fader.x, fader.y - 2, fader.w, 16),
            Align::Center,
            t.text_dim,
            &label,
        );
        for (i, br) in buttons.iter().enumerate() {
            let live = self.on || i == 2;
            cx.canvas
                .fill_round_rect(*br, 10, if live { t.raised } else { t.surface });
            let word = if i == 2 {
                format!("Save {}", if self.pos < 0.5 { "A" } else { "B" })
            } else {
                MORPH_BUTTONS[i].0.to_string()
            };
            cx.text(
                FontId::Bold,
                t.size_small,
                *br,
                Align::Center,
                if live { t.text } else { t.text_faint },
                &word,
            );
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let (a, fader, b, buttons) = MorphBar::parts(r);
        let mut out = Vec::new();
        for (index, chip) in [a, b].iter().enumerate() {
            let name = ["A", "B"][index];
            let (down, up) = chip.take_left(chip.w / 2);
            out.push(Target::tap(
                "row-down",
                index,
                format!("{name} (left half)"),
                down,
            ));
            out.push(Target::tap(
                "row-up",
                index,
                format!("{name} (right half)"),
                up,
            ));
        }
        out.push(Target::drag("crossfader", 0, "crossfader", fader).towards(fader.w / 3, 0));
        for (i, br) in buttons.iter().enumerate() {
            out.push(Target::tap(
                "button",
                i,
                ["Snap back", "Keep", "Save"][i],
                *br,
            ));
        }
        out
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let (a, fader, b, buttons) = MorphBar::parts(r);
        let slide = fader.inset_xy(-12, 0);
        match phase {
            Touch::Down => {
                for (index, chip) in [a, b].iter().enumerate() {
                    if chip.contains(x, y) {
                        let delta = if x < chip.x + chip.w / 2 { -1 } else { 1 };
                        return Some(EventKind::Step { index, delta });
                    }
                }
                if let Some(i) = buttons.iter().position(|br| br.contains(x, y)) {
                    return Some(EventKind::Tab(i));
                }
                slide.contains(x, y).then(|| {
                    EventKind::Value(((x - fader.x) as f32 / fader.w.max(1) as f32).clamp(0.0, 1.0))
                })
            }
            Touch::Move if slide.inset_xy(0, -40).contains(x, y) => Some(EventKind::Value(
                ((x - fader.x) as f32 / fader.w.max(1) as f32).clamp(0.0, 1.0),
            )),
            _ => None,
        }
    }
}

fn paint_cell(cx: &mut PaintCx, cr: Rect, cell: &GridCell, base: Color) {
    let t = cx.theme.clone();
    if cell.state == CellState::Empty {
        cx.canvas.stroke_round_rect(cr, 10, 1, t.outline);
        cx.text(
            FontId::Sans,
            t.size_title + 3.0,
            cr,
            Align::Center,
            t.text_faint,
            "+",
        );
        return;
    }
    let fill = match cell.state {
        CellState::Playing => t.raised.mix(base, 0.36),
        CellState::Recording => t.raised.mix(t.bad, 0.24),
        _ => t.raised.mix(base, 0.16),
    };
    cx.canvas.fill_round_rect(cr, 10, fill);
    match cell.state {
        CellState::Queued => cx.canvas.stroke_round_rect(cr, 10, 2, t.warn),
        CellState::Recording => cx.canvas.stroke_round_rect(cr, 10, 2, t.bad),
        _ => {}
    }
    let inner = cr.inset_xy(10, 8);
    let (name, rest) = inner.take_top(20);
    cx.text(
        FontId::Bold,
        t.size_small + 1.0,
        name,
        Align::Left,
        t.text,
        &cell.name,
    );
    let (detail, _) = rest.take_top(20);
    let detail_ink = match cell.state {
        CellState::Recording => t.bad.mix(t.text, 0.2),
        _ => t.text_dim,
    };
    cx.text(
        FontId::Sans,
        t.size_small - 1.0,
        detail,
        Align::Left,
        detail_ink,
        &cell.detail,
    );
    let rolling = matches!(cell.state, CellState::Playing | CellState::Recording);
    let ring_room = if rolling { RING + 4 } else { 0 };
    let foot = Rect::new(
        inner.x,
        inner.bottom() - 18,
        (inner.w - ring_room).max(0),
        18,
    );
    let foot_ink = match cell.state {
        CellState::Queued => t.warn,
        CellState::Recording => t.bad.mix(t.text, 0.2),
        _ => t.text_faint,
    };
    if foot.y >= detail.bottom() - 2 {
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            foot,
            Align::Left,
            foot_ink,
            &cell.foot,
        );
    }
    if rolling && inner.h >= RING + 20 {
        let color = if cell.state == CellState::Recording {
            t.bad
        } else {
            t.good
        };
        ring(
            cx.canvas,
            cr.right() - RING - 7,
            cr.bottom() - RING - 7,
            RING,
            3.5,
            cell.progress,
            t.surface.mix(t.text_faint, 0.35),
            color,
        );
    }
}

// ---------------------------------------------------------------- takes

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TakeItem {
    pub title: String,
    /// On the right, in the mono face, such as "+9 −5".
    pub detail: String,
    /// The track's colour, for the dot.
    pub color: Option<Color>,
    /// Undone: Redo would bring it back.
    pub undone: bool,
}

/// The takes, newest first, with Undo and Redo buttons along the bottom.
/// The row `current` (the take Undo would take back) is highlighted;
/// undone takes are dimmed. A tap on Undo gives `Tab(0)`, on Redo `Tab(1)`,
/// each only while it has something to do.
pub struct TakeList {
    pub title: String,
    pub right: String,
    pub items: Vec<TakeItem>,
    pub current: Option<usize>,
    /// What Undo and Redo would do, such as "take 3"; None when nothing.
    pub undo: Option<String>,
    pub redo: Option<String>,
    /// What Capture would keep, such as "2 bars"; None when nothing.
    pub capture: Option<String>,
}

const TAKE_ROW_H: i32 = 38;
const TAKE_BUTTON_H: i32 = 48;

impl TakeList {
    pub fn new(title: &str) -> TakeList {
        TakeList {
            title: title.to_string(),
            right: String::new(),
            items: Vec::new(),
            current: None,
            undo: None,
            redo: None,
            capture: None,
        }
    }

    fn body(r: Rect) -> Rect {
        // Matches PaintCx::titled_panel with the default theme's padding.
        r.inset(14).take_top(34).1
    }

    /// Undo (0), Redo (1) or Capture (2): the event's index. Left to right
    /// they are Capture, Undo, Redo.
    pub fn button_rect(r: Rect, i: usize) -> Rect {
        let body = TakeList::body(r);
        let w = (body.w - 16) / 3;
        let at = [1, 2, 0][i.min(2)];
        Rect::new(
            body.x + at * (w + 8),
            body.bottom() - TAKE_BUTTON_H,
            w,
            TAKE_BUTTON_H,
        )
    }
}

impl Widget for TakeList {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        let body = cx.titled_panel(r, &self.title, &self.right);
        let limit = TakeList::button_rect(r, 0).y - 8;
        if self.items.is_empty() {
            let (a, rest) = body.take_top(26);
            cx.text(
                FontId::Sans,
                t.size_body,
                a,
                Align::Left,
                t.text_faint,
                "No takes yet",
            );
            // One line, or two in a narrow panel (Arrange beside the
            // Q-Link column).
            const HELP: &str = "REC records one; Capture keeps what you just played";
            let lines: Vec<&str> = if cx.measure(FontId::Sans, t.size_small, HELP) <= body.w {
                vec![HELP]
            } else {
                vec!["REC records one;", "Capture keeps what you played"]
            };
            let mut rest = rest;
            for line in lines {
                let (b, after) = rest.take_top(24);
                if b.bottom() > limit {
                    break;
                }
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    b,
                    Align::Left,
                    t.text_faint,
                    line,
                );
                rest = after;
            }
        }
        let mut y = body.y;
        for (i, item) in self.items.iter().enumerate() {
            if y + TAKE_ROW_H > limit {
                break;
            }
            let row = Rect::new(body.x, y, body.w, TAKE_ROW_H - 4);
            if self.current == Some(i) {
                cx.canvas.fill_round_rect(row, 8, t.raised);
                cx.canvas
                    .fill_round_rect(Rect::new(row.x, row.y + 9, 3, row.h - 18), 1, t.accent);
            }
            let dot = match (item.color, item.undone) {
                (Some(c), false) => c,
                (Some(c), true) => c.mix(t.surface, 0.65),
                (None, _) => t.text_faint,
            };
            cx.canvas
                .fill_round_rect(Rect::new(row.x + 12, row.y + row.h / 2 - 4, 8, 8), 4, dot);
            let (ink, sub) = if item.undone {
                (t.text_faint, t.text_faint)
            } else {
                (t.text, t.text_dim)
            };
            let dw = cx.measure(FontId::Mono, t.size_mono, &item.detail);
            let detail = Rect::new(row.right() - dw - 10, row.y, dw + 2, row.h);
            cx.text(
                FontId::Mono,
                t.size_mono,
                detail,
                Align::Right,
                sub,
                &item.detail,
            );
            let title = Rect::new(row.x + 28, row.y, (detail.x - row.x - 36).max(0), row.h);
            cx.text(
                FontId::Sans,
                t.size_small + 1.0,
                title,
                Align::Left,
                ink,
                &item.title,
            );
            y += TAKE_ROW_H;
        }
        for (i, (word, what)) in [
            ("Undo", &self.undo),
            ("Redo", &self.redo),
            ("Capture", &self.capture),
        ]
        .into_iter()
        .enumerate()
        {
            let b = TakeList::button_rect(r, i);
            match what {
                Some(w) => {
                    cx.canvas.fill_round_rect(b, 12, t.raised);
                    // Capture stands out: there is something to keep.
                    let ink = if i == 2 {
                        cx.canvas.stroke_round_rect(b, 12, 2, t.accent);
                        t.accent
                    } else {
                        t.text
                    };
                    let full = format!("{word} {w}");
                    // A narrow panel shows the word alone.
                    let fits = cx.measure(FontId::Bold, t.size_small + 1.0, &full) <= b.w - 12;
                    cx.text(
                        FontId::Bold,
                        t.size_small + 1.0,
                        b,
                        Align::Center,
                        ink,
                        if fits { &full } else { word },
                    );
                }
                None => {
                    cx.canvas.stroke_round_rect(b, 12, 1, t.outline);
                    cx.text(
                        FontId::Sans,
                        t.size_small + 1.0,
                        b,
                        Align::Center,
                        t.text_faint,
                        word,
                    );
                }
            }
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        (0..3)
            .map(|i| {
                Target::tap(
                    "button",
                    i,
                    ["Undo", "Redo", "Capture"][i],
                    TakeList::button_rect(r, i),
                )
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        let i = (0..3).find(|&i| TakeList::button_rect(r, i).contains(x, y))?;
        let live = [&self.undo, &self.redo, &self.capture][i].is_some();
        live.then_some(EventKind::Tab(i))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};

    fn bar_ui(bar: TransportBar) -> Ui {
        let mut ui = Ui::new(Theme::dark(), Fonts::embedded(), 1280, 68);
        ui.add("bar", bar);
        ui.add_page("p", "bar").unwrap();
        ui
    }

    #[test]
    fn the_bar_reports_its_buttons_and_chips() {
        let mut bar = TransportBar::new();
        bar.undo = Some("take 2".into());
        bar.chips = vec![Chip::new("Arp · Up 1/16", Tone::Accent)];
        let mut ui = bar_ui(bar);
        // Before the first paint the bar knows only the fixed buttons' places.
        let r = ui.rect_of("bar").unwrap();
        let mut c = SoftCanvas::new(1280, 68);
        ui.paint(&mut c);
        for b in [BarButton::Stop, BarButton::Play, BarButton::Rec] {
            let br = TransportBar::button_rect(r, b).unwrap();
            let ev = ui.touch(1, Touch::Down, br.x + 5, br.y + 5).unwrap();
            assert_eq!(ev.kind, EventKind::Tab(b.index()));
            ui.touch(1, Touch::Up, br.x + 5, br.y + 5);
        }
        let mode_x = TransportBar::button_rect(r, BarButton::Rec)
            .unwrap()
            .right()
            + 20;
        assert_eq!(
            ui.touch(2, Touch::Down, mode_x, r.y + 20).unwrap().kind,
            EventKind::Tab(BarButton::Mode.index())
        );
        ui.touch(2, Touch::Up, mode_x, r.y + 20);
        // The chip is at the right edge, Undo just left of it.
        assert_eq!(
            ui.touch(3, Touch::Down, r.right() - 20, r.y + 20)
                .unwrap()
                .kind,
            EventKind::Select(0)
        );
        ui.touch(3, Touch::Up, r.right() - 20, r.y + 20);
        let chip_w = ui.fonts.measure(FontId::Sans, 14.0, "Arp · Up 1/16") + 40;
        let undo_x = r.right() - 4 - chip_w - 8 - 20;
        assert_eq!(
            ui.touch(4, Touch::Down, undo_x, r.y + 20).unwrap().kind,
            EventKind::Tab(BarButton::Undo.index())
        );
    }

    #[test]
    fn undo_with_nothing_to_undo_is_not_a_button() {
        let mut ui = bar_ui(TransportBar::new());
        let r = ui.rect_of("bar").unwrap();
        ui.paint(&mut SoftCanvas::new(1280, 68));
        assert_eq!(ui.touch(1, Touch::Down, r.right() - 30, r.y + 20), None);
    }

    fn grid(tracks: usize) -> ClipGrid {
        let mut g = ClipGrid::new("Clips", 16);
        for i in 0..tracks {
            let mut cells = vec![GridCell::default(); 16];
            cells[0] = GridCell {
                state: CellState::Playing,
                name: "Beat".into(),
                progress: 0.25,
                ..GridCell::default()
            };
            g.tracks.push(GridTrack {
                name: format!("T{i}"),
                color: Color::rgb(0x3DDC97),
                armed: i == 1,
                mute: false,
                cells,
            });
        }
        g
    }

    #[test]
    fn grid_cells_and_names_report_where_they_are() {
        let mut g = grid(10);
        let r = Rect::new(12, 128, 814, 610);
        let rows = g.visible_rows(r);
        assert!((6..=7).contains(&rows), "{rows}");
        // Eight tracks across; the ninth is off to the right.
        assert!(g.cell_rect(r, 7, 0).is_some() && g.cell_rect(r, 8, 0).is_none());
        let c = g.cell_rect(r, 2, 3).unwrap();
        assert!(c.w > 80 && c.h >= ROW_MIN_H, "{c:?}");
        assert!(r.intersect(c) == c);
        assert_eq!(
            g.touch(1, Touch::Down, c.x + 2, c.y + 2, r),
            Some(EventKind::Select(2 * 16 + 3))
        );
        let h = g.head_rect(r, 5).unwrap();
        assert_eq!(
            g.touch(1, Touch::Down, h.x + 2, h.y + 2, r),
            Some(EventKind::Tab(5))
        );
        // Scrolled: slot 3 is the first row, track 2 the first column.
        let corner = g.cell_rect(r, 0, 0).unwrap();
        g.first_row = 3;
        g.first_track = 2;
        assert_eq!(g.cell_rect(r, 2, 3), Some(corner));
        assert!(g.cell_rect(r, 1, 3).is_none() && g.cell_rect(r, 2, 2).is_none());
        // The last slot is in reach when scrolled to the bottom.
        g.first_row = 16 - rows;
        assert!(g
            .cell_rect(r, 2, 15)
            .is_some_and(|c| c.bottom() <= r.bottom()));
    }

    #[test]
    fn the_morph_bar_steps_rows_slides_and_has_three_buttons() {
        let mut m = MorphBar::new();
        let r = Rect::new(12, 680, 830, 64);
        let (a, fader, b, buttons) = MorphBar::parts(r);
        assert!(a.right() < fader.x && fader.right() < b.x && b.right() < buttons[0].x);
        assert!(fader.w > 200, "{fader:?}");
        assert_eq!(
            m.touch(1, Touch::Down, a.x + 4, a.y + 4, r),
            Some(EventKind::Step {
                index: 0,
                delta: -1
            })
        );
        assert_eq!(
            m.touch(1, Touch::Down, b.right() - 4, b.y + 4, r),
            Some(EventKind::Step { index: 1, delta: 1 })
        );
        let mid = fader.x + fader.w / 2;
        assert_eq!(
            m.touch(1, Touch::Down, mid, fader.y + 10, r),
            Some(EventKind::Value(0.5))
        );
        assert_eq!(
            m.touch(1, Touch::Move, fader.right() + 5, fader.y + 10, r),
            Some(EventKind::Value(1.0)),
            "a drag past the end holds at B"
        );
        for (i, br) in buttons.iter().enumerate() {
            assert_eq!(
                m.touch(1, Touch::Down, br.x + 4, br.y + 4, r),
                Some(EventKind::Tab(i))
            );
        }
        m.pos = 0.7;
        assert_eq!(m.save_row(), 1);
    }

    #[test]
    fn row_numbers_launch_scenes_and_the_corner_stops_all() {
        let mut g = grid(3);
        let r = Rect::new(12, 128, 814, 610);
        let b = g.scene_rect(r, 2).unwrap();
        let c = g.cell_rect(r, 0, 2).unwrap();
        assert_eq!((b.y, b.h), (c.y, c.h), "beside its row");
        assert!(b.right() < c.x && b.w >= 44, "{b:?} {c:?}");
        assert_eq!(
            g.touch(1, Touch::Down, b.x + 4, b.y + 4, r),
            Some(EventKind::Toggle(2))
        );
        let s = g.stop_all_rect(r);
        assert_eq!(
            g.touch(1, Touch::Down, s.x + 4, s.y + 4, r),
            Some(EventKind::Toggle(16))
        );
        let song = g.song_rect(r);
        assert!(song.right() <= r.right() - 14 && song.y >= r.y, "{song:?}");
        assert_eq!(
            g.touch(1, Touch::Down, song.x + 4, song.y + 4, r),
            Some(EventKind::Toggle(17))
        );
        let rec = g.rec_rect(r);
        assert!(rec.right() < song.x, "{rec:?}");
        assert_eq!(
            g.touch(1, Touch::Down, rec.x + 4, rec.y + 4, r),
            Some(EventKind::Toggle(18))
        );
        let edit = g.edit_rect(r);
        assert!(edit.right() < rec.x, "{edit:?}");
        assert_eq!(
            g.touch(1, Touch::Down, edit.x + 4, edit.y + 4, r),
            Some(EventKind::Toggle(19))
        );
        g.first_row = 2;
        assert_eq!(
            g.scene_rect(r, 2).map(|x| x.y),
            g.cell_rect(r, 0, 2).map(|x| x.y)
        );
        assert!(g.scene_rect(r, 1).is_none());
    }

    #[test]
    fn take_buttons_answer_only_with_something_to_do() {
        let mut l = TakeList::new("Takes");
        let r = Rect::new(0, 0, 400, 260);
        let undo = TakeList::button_rect(r, 0);
        let redo = TakeList::button_rect(r, 1);
        assert!(undo.right() < redo.x && redo.right() <= r.right() - 14);
        assert_eq!(l.touch(1, Touch::Down, undo.x + 2, undo.y + 2, r), None);
        l.undo = Some("take 1".into());
        assert_eq!(
            l.touch(1, Touch::Down, undo.x + 2, undo.y + 2, r),
            Some(EventKind::Tab(0))
        );
        assert_eq!(l.touch(1, Touch::Down, redo.x + 2, redo.y + 2, r), None);
        l.redo = Some("take 2".into());
        assert_eq!(
            l.touch(1, Touch::Down, redo.x + 2, redo.y + 2, r),
            Some(EventKind::Tab(1))
        );
    }

    #[test]
    fn a_ring_fills_clockwise_from_the_top() {
        let mut c = SoftCanvas::new(40, 40);
        let (rest, done) = (Color::rgb(0x0000FF), Color::rgb(0xFF0000));
        ring(&mut c, 0, 0, 40, 6.0, 0.25, rest, done);
        // A quarter: the right side of the top is done, the left side not.
        assert_eq!(c.pixel(23, 2), done);
        assert_eq!(c.pixel(16, 2), rest);
        assert_eq!(c.pixel(37, 16), done);
        assert_eq!(c.pixel(37, 24), rest);
        // The middle stays empty.
        assert_eq!(c.pixel(20, 20), Color::rgb(0));
    }
}
