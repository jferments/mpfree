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
use std::time::{Duration, Instant};

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
/// `Select(i)`. Readings that do not fit are left out from the right; the first is cut
/// to fit (WAITING or NEXT).
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
        let fill = match self.run {
            Run::Stopped => t.raised,
            Run::Playing => t.good,
            Run::Recording => t.bad,
            Run::CountIn => t.warn,
        };
        let (lit, ink) = match self.run {
            Run::Stopped => (BarButton::Stop, t.text),
            Run::Playing => (BarButton::Play, t.ink_on(fill)),
            Run::Recording | Run::CountIn => (BarButton::Rec, t.ink_on(fill)),
        };
        for b in [BarButton::Stop, BarButton::Play, BarButton::Rec] {
            let Some(br) = TransportBar::button_rect(r, b) else {
                continue;
            };
            let on = b == lit;
            if on {
                cx.canvas.fill_round_rect(br, t.round(10), fill);
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
        cx.canvas.fill_round_rect(mode, t.round(10), t.raised);
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
                cx.canvas.fill_round_rect(undo, t.round(10), t.raised);
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
                cx.canvas.stroke_round_rect(undo, t.round(10), 1, t.outline);
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
            let mut vw = cx.measure(FontId::Bold, t.size_body, &item.value);
            let mut w = 32 + lw + vw;
            if x + w > limit {
                // The first reading (WAITING or NEXT, when there is one)
                // is cut to fit rather than left out.
                let room = limit - x - 32 - lw;
                if room >= 80 && std::ptr::eq(item, &self.items[0]) {
                    vw = room;
                    w = limit - x;
                } else {
                    break;
                }
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
    /// An empty Loop cell armed: the next loop recorded lands here.
    Armed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridCell {
    pub state: CellState,
    pub name: String,
    /// Under the name, such as "2 bars".
    pub detail: String,
    /// At the bottom, such as "24 notes" or "from 7.1".
    pub foot: String,
    /// How far through its pass the clip is, 0.0 to 1.0 (the ring); for
    /// a queued clip in Perform, how near its line is (the countdown, an
    /// amber bar along its foot; 0 draws none).
    pub progress: f32,
    /// A loop cell's small wave, drawn under its name in the detail's
    /// place (the detail goes to the foot when it is empty): each column's
    /// peak, 0 to 255 (all 0 until the looper has made it). An audio
    /// clip's cell has one too (layout spec 4f).
    pub wave: Option<[u8; CELL_WAVE]>,
    /// The foot is a word that matters (an audio clip's "made at 120",
    /// "not ready", "missing"): in Make it takes the wave's place while
    /// the clip is stopped or queued, and it is drawn in its colour.
    pub alert: CellAlert,
    /// A once audio clip still sounding past its stop: its wave lit up to
    /// `progress` with the playhead dimmed.
    pub ringing: bool,
    /// An audio clip whose audio is missing: a flat dashed line in its
    /// wave's place, on a darker cell with a red edge.
    pub missing: bool,
}

/// The colour of a cell's word ([`GridCell::alert`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellAlert {
    #[default]
    None,
    /// Amber: it plays, but not as made ("made at 120", "waits 18.1").
    Warn,
    /// Red: it can't play ("not ready", "missing").
    Bad,
}

/// Columns of a loop cell's small wave ([`GridCell::wave`]).
pub const CELL_WAVE: usize = 24;

/// One column of the grid: a track and its slots.
#[derive(Clone, Debug, PartialEq)]
pub struct GridTrack {
    pub name: String,
    /// What it plays, under its name ("Kit", "FM · Glass Keys"); empty
    /// shows only the name, in a shorter header.
    pub sound: String,
    pub color: Color,
    /// Recording goes into this track.
    pub armed: bool,
    /// Silent: muted, or not soloed while another track is.
    pub mute: bool,
    /// Soloed (sequencer S4): outlined in the accent colour.
    pub solo: bool,
    /// A mute or solo change waiting for its line: the header is hatched
    /// and this says what and when ("mutes in 2 beats") under the name.
    pub waiting: Option<String>,
    /// A Loop track: a wave mark before what it records, under its name.
    pub loops: bool,
    /// An audio track (layout spec 4f): a wave mark before its input,
    /// under its name, and its cells empty and dashed until audio clips
    /// come.
    pub audio: bool,
    pub cells: Vec<GridCell>,
}

/// The launch grid: tracks across, their clip slots down, as on the MPC's
/// clip launch and in Ableton's session view. A playing clip shows a ring
/// that goes round once per pass; a queued one an amber outline; the one
/// being recorded a red outline. A tap on a track's name gives
/// `Tab(track)` (on the lift), on the sound line under it
/// `Tab(SOUND_TAP + track)`; a tap on a cell gives `Select(track * slots
/// + slot)`. The app scrolls it with `first_track` and `first_row`.
///
/// With more than [`GRID_COLS`] tracks a pager takes the legend's place,
/// "‹ 9–16 of 24 ›": its arrows, or a swipe along the track names, give
/// `Toggle(slots + `[`PAGE_BACK`]`)` and `Toggle(slots + `[`PAGE_ON`]`)`.
/// "+ Track", left of Edit, gives `Toggle(slots + `[`ADD_TRACK`]`)` while
/// [`ClipGrid::can_add`] holds.
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
    /// The project has room for another track: "+ Track" is lit.
    pub can_add: bool,
    /// The "+ Track" chooser is open under it (the app's, from a tap on
    /// "+ Track"): its rows give `Toggle(slots + `[`ADD_KIND`]` + k)` for
    /// `song::ADD_KINDS[k]`, and a touch anywhere else
    /// `Toggle(slots + `[`ADD_DISMISS`]`)`.
    pub adding: bool,
    /// Clips launched over the song (song mode on): "Back to song" shows
    /// on the title line, left of the pager or "+ Track" (sequencer S4,
    /// mockup C), and gives `Toggle(slots + `[`BACK_TO_SONG`]`)`.
    pub back_to_song: bool,
    /// Perform's big rows (sequencer decision 5, mockup C): five rows of
    /// [`ROW_MIN_H`] with rings and countdowns. Off (Make), the rows are
    /// compact, [`ROW_MIN_H_MAKE`], eight in view with names and lengths.
    pub big: bool,
    /// What was drawn last, so a change redraws only the cells that changed.
    drawn: RefCell<Option<Drawn>>,
    /// A finger down on a track's name: where (x) and which track. Let go
    /// near where it went down it arms the track; swiped along the names
    /// it pages.
    press: Option<(i32, usize)>,
    /// A finger on a clip or a row's number, and where it has dragged it.
    grip: Option<Grip>,
}

/// What a finger holds on the grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    /// The clip in (track, slot).
    Clip(usize, usize),
    /// A row, by its number.
    Row(usize),
}

/// A finger on a clip or a row's number (Jay, 14 Nov: drag and drop).
/// Lifted once it has moved more than a fingertip ([`LIFT_PX`]): it
/// follows the finger and drops where the finger lifts. Not lifted, the
/// lift is a tap.
#[derive(Clone, Copy, Debug)]
struct Grip {
    held: Held,
    from: (i32, i32),
    at: (i32, i32),
    lifted: bool,
    /// The edge the finger is held at, and when it last paged there.
    edge: Option<(usize, Instant)>,
}

/// A finger moves this far before what it holds lifts.
const LIFT_PX: i32 = 12;
/// Held this close to the grid's edge, a dragged clip pages the tracks or
/// scrolls the rows, again every [`EDGE_REPEAT`] while it stays there.
const EDGE_PX: i32 = 28;
const EDGE_REPEAT: Duration = Duration::from_millis(600);
/// `Drop { from: ROW_DROP + row, to: gap }`: row `row` dragged to the
/// gap before row `gap` (0 to the slots). A clip's drop is `Drop { from:
/// track * slots + slot, to: track * slots + slot }`.
pub const ROW_DROP: usize = 1 << 20;

struct Drawn {
    at: Rect,
    first: (usize, usize),
    title: String,
    legend: String,
    tracks: Vec<GridTrack>,
    scenes: (Option<usize>, Option<usize>),
    song_rec: bool,
    can_add: bool,
    adding: bool,
    big: bool,
    back_to_song: bool,
    /// Where a dragged clip or row was drawn.
    dragged: Option<(Held, (i32, i32))>,
}

/// `Toggle(slots + ADD_TRACK)`: "+ Track".
pub const ADD_TRACK: usize = 4;
/// `Toggle(slots + PAGE_BACK)`: the tracks before (the pager's ‹, or a
/// swipe to the right along the names).
pub const PAGE_BACK: usize = 5;
/// `Toggle(slots + PAGE_ON)`: the tracks after (›, or a swipe to the left).
pub const PAGE_ON: usize = 6;
/// `Toggle(slots + ROWS_BACK)` and `Toggle(slots + ROWS_ON)`: a dragged
/// clip or row held at the top or bottom edge scrolls the rows one.
pub const ROWS_BACK: usize = 7;
pub const ROWS_ON: usize = 8;
/// `Toggle(slots + BACK_TO_SONG)`: "Back to song", every track back to the
/// arrangement on the launch line.
pub const BACK_TO_SONG: usize = 9;
/// `Toggle(slots + ADD_KIND + k)`: the "+ Track" chooser's row `k`
/// (`song::ADD_KINDS`: Note track, Audio track, Loop track).
pub const ADD_KIND: usize = 10;
/// `Toggle(slots + ADD_DISMISS)`: a touch off the open chooser, which
/// only closes it.
pub const ADD_DISMISS: usize = 13;
/// "Back to song"'s width.
const BACK_W: i32 = 124;
/// Pixels a finger travels along the track names to page.
const SWIPE_PX: i32 = 48;
/// "+ Track"'s width, the pager's arrows' and its label's.
const ADD_W: i32 = 88;
const ARROW_W: i32 = 34;
const PAGER_LABEL_W: i32 = 104;

/// Tracks shown at once.
pub const GRID_COLS: usize = 8;
/// The Rec song button's width (on the grid and the Song view).
pub const REC_SONG_W: i32 = 112;

/// The Rec song button: live to song, red while armed.
pub fn rec_song_pill(cx: &mut PaintCx, r: Rect, on: bool) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(r, t.round(10), if on { t.bad } else { t.raised });
    let ink = if on { t.ink_on(t.bad) } else { t.text_dim };
    cx.canvas.fill_round_rect(
        Rect::new(r.x + 12, r.y + r.h / 2 - 5, 10, 10),
        5,
        if on { ink } else { t.bad },
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
const HEAD_SOUND_H: i32 = 44;
/// A tap on track `t`'s sound line, under its name, gives
/// `Tab(SOUND_TAP + t)`.
pub const SOUND_TAP: usize = 1 << 16;
const CELL_GAP: i32 = 6;
/// Perform's big rows: five in view on the MPC's screen.
pub const ROW_MIN_H: i32 = 76;
/// Make's compact rows: eight in view, a name and a length each.
pub const ROW_MIN_H_MAKE: i32 = 44;
const RING: i32 = 26;
/// A compact cell's ring.
const SMALL_RING: i32 = 18;
/// Under this height a cell is drawn compact: its name, and its length
/// (or when it starts) under it.
const COMPACT_H: i32 = 64;

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
            can_add: false,
            adding: false,
            big: false,
            back_to_song: false,
            drawn: RefCell::new(None),
            press: None,
            grip: None,
        }
    }

    /// The place a clip dragged to (`x`, `y`) lands in, if it is over a
    /// cell in view.
    fn cell_at(&self, r: Rect, x: i32, y: i32) -> Option<(usize, usize)> {
        let g = self.geo(r);
        if x < g.x0 || y < g.cells_y || x >= g.body.right() || y >= g.body.bottom() {
            return None;
        }
        let c = ((x - g.x0) / (g.col_w + CELL_GAP)) as usize;
        let row = ((y - g.cells_y) / (g.row_h + CELL_GAP)) as usize;
        let (track, slot) = (self.first_track + c, self.first_row + row);
        (c < GRID_COLS && track < self.tracks.len() && row < g.rows && slot < self.slots)
            .then_some((track, slot))
    }

    /// The gap between rows a row dragged to height `y` goes in: before
    /// row `gap`, from the first row in view to just after the last.
    fn gap_at(&self, r: Rect, y: i32) -> usize {
        let g = self.geo(r);
        let pitch = g.row_h + CELL_GAP;
        let k = ((y - g.cells_y + pitch / 2).max(0) / pitch) as usize;
        let last = (self.first_row + g.rows).min(self.slots);
        (self.first_row + k).min(last)
    }

    /// Where a dragged clip lands now, if it would change anything.
    fn clip_target(&self, r: Rect, g: &Grip) -> Option<(usize, usize)> {
        match g.held {
            Held::Clip(t, s) => self
                .cell_at(r, g.at.0, g.at.1)
                .filter(|c| *c != (t, s) && g.lifted),
            Held::Row(_) => None,
        }
    }

    /// The gap a dragged row goes in now, if it would move.
    fn row_target(&self, r: Rect, g: &Grip) -> Option<usize> {
        match g.held {
            Held::Row(row) if g.lifted => {
                Some(self.gap_at(r, g.at.1)).filter(|&k| k != row && k != row + 1)
            }
            _ => None,
        }
    }

    /// A dragged clip or row held at an edge of the grid: which way to
    /// page or scroll (`PAGE_BACK`, `PAGE_ON`, `ROWS_BACK`, `ROWS_ON`).
    fn edge_at(&self, r: Rect, held: Held, x: i32, y: i32) -> Option<usize> {
        let g = self.geo(r);
        let clip = matches!(held, Held::Clip(..));
        let n = self.tracks.len();
        if clip && x < g.x0 + EDGE_PX && self.first_track > 0 {
            Some(PAGE_BACK)
        } else if clip && x > g.body.right() - EDGE_PX && self.first_track + GRID_COLS < n {
            Some(PAGE_ON)
        } else if y < g.cells_y + EDGE_PX && y >= g.cells_y - 40 && self.first_row > 0 {
            Some(ROWS_BACK)
        } else if y > g.body.bottom() - EDGE_PX && self.first_row + g.rows < self.slots {
            Some(ROWS_ON)
        } else {
            None
        }
    }

    /// Whether a drag is under way (something lifted).
    pub fn dragging(&self) -> bool {
        self.grip.is_some_and(|g| g.lifted)
    }

    fn dragged_now(&self) -> Option<(Held, (i32, i32))> {
        self.grip.filter(|g| g.lifted).map(|g| (g.held, g.at))
    }

    /// A track's colour as its column draws it (faded while muted).
    fn base(&self, cx: &PaintCx, track: usize) -> Color {
        let tr = &self.tracks[track];
        if tr.mute {
            tr.color.mix(cx.theme.surface, 0.6)
        } else {
            tr.color
        }
    }

    /// A clip or a row being dragged: its own place empty but outlined,
    /// where it lands outlined (a clip) or a line (a row), and the clip
    /// or the row under the finger.
    fn paint_drag(&self, cx: &mut PaintCx, r: Rect, gr: &Grip) {
        let t = cx.theme.clone();
        let g = self.geo(r);
        let (dx, dy) = (gr.at.0 - gr.from.0, gr.at.1 - gr.from.1);
        match gr.held {
            Held::Clip(track, slot) => {
                let home = self.cell_rect(r, track, slot);
                if let Some(h) = home {
                    cx.canvas.fill_round_rect(h, t.round(8), t.surface);
                    cx.canvas.stroke_round_rect(h, t.round(8), 2, t.outline);
                }
                let to = self.clip_target(r, gr);
                if let Some(c) = to.and_then(|(t2, s2)| self.cell_rect(r, t2, s2)) {
                    cx.canvas
                        .stroke_round_rect(c.inset(-3), t.round(10), 3, t.accent);
                }
                let card = match home {
                    Some(h) => Rect::new(h.x + dx, h.y + dy, h.w, h.h),
                    None => Rect::new(
                        gr.at.0 - g.col_w / 2,
                        gr.at.1 - g.row_h / 2,
                        g.col_w,
                        g.row_h,
                    ),
                };
                if let Some(cell) = self.tracks.get(track).and_then(|tr| tr.cells.get(slot)) {
                    paint_cell(cx, card, cell, self.base(cx, track));
                }
                cx.canvas.stroke_round_rect(card, t.round(8), 2, t.accent);
            }
            Held::Row(row) => {
                let strip = |b: Rect| Rect::new(b.x, b.y, g.body.right() - b.x, b.h);
                let home = self.scene_rect(r, row);
                if let Some(b) = home {
                    cx.canvas.fill_round_rect(strip(b), t.round(8), t.surface);
                    cx.canvas
                        .stroke_round_rect(strip(b), t.round(8), 2, t.outline);
                }
                // The row under the finger follows it, but stays clear of
                // the line: below it on the way up, above it on the way
                // down.
                let (mut lo, mut hi) = (g.cells_y - g.row_h / 2, g.body.bottom() - g.row_h / 2);
                if let Some(gap) = self.row_target(r, gr) {
                    let pitch = g.row_h + CELL_GAP;
                    let y = g.cells_y + (gap - self.first_row) as i32 * pitch - CELL_GAP / 2 - 2;
                    let w = g.body.right() - g.body.x;
                    cx.canvas
                        .fill_round_rect(Rect::new(g.body.x, y, w, 4), 2, t.accent);
                    for x in [g.body.x - 4, g.body.right() - 8] {
                        cx.canvas
                            .fill_round_rect(Rect::new(x, y - 4, 12, 12), 6, t.accent);
                    }
                    if gap <= row {
                        lo = y + 14;
                    } else {
                        hi = y - 10 - g.row_h;
                    }
                }
                let y0 = match home {
                    Some(b) => b.y + dy,
                    None => gr.at.1 - g.row_h / 2,
                };
                let y0 = y0.clamp(lo, hi.max(lo));
                let b = Rect::new(g.body.x + 8, y0, LABEL_W, g.row_h);
                cx.canvas.fill_round_rect(strip(b), t.round(8), t.raised);
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    b,
                    Align::Center,
                    t.text,
                    &format!("{}", row + 1),
                );
                let tracks =
                    self.first_track..(self.first_track + GRID_COLS).min(self.tracks.len());
                for track in tracks {
                    let Some(c) = self.head_rect(r, track) else {
                        continue;
                    };
                    if let Some(cell) = self.tracks[track].cells.get(row) {
                        let cr = Rect::new(c.x + 8, y0, c.w, g.row_h);
                        paint_cell(cx, cr, cell, self.base(cx, track));
                    }
                }
                cx.canvas
                    .stroke_round_rect(strip(b), t.round(8), 2, t.accent);
            }
        }
    }

    /// More tracks than fit across: the pager shows.
    pub fn paged(&self) -> bool {
        self.tracks.len() > GRID_COLS
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

    /// Where "+ Track" is: left of Edit.
    pub fn add_rect(&self, r: Rect) -> Rect {
        let edit = self.edit_rect(r);
        Rect::new(edit.x - 8 - ADD_W, edit.y, ADD_W, edit.h)
    }

    /// The "+ Track" chooser's rows while it is open: under "+ Track",
    /// its right edge with the button's.
    pub fn chooser_rects(&self, r: Rect) -> Option<[Rect; 3]> {
        if !self.adding {
            return None;
        }
        let add = self.add_rect(r);
        let x = (add.right() - crate::song::CHOOSER_W).max(r.x);
        Some(crate::song::chooser_rows(x, add.bottom() + 6))
    }

    /// Where the pager's ‹ and › are, left of "+ Track" with its label
    /// between them, while it shows.
    pub fn pager_rects(&self, r: Rect) -> Option<(Rect, Rect)> {
        if !self.paged() {
            return None;
        }
        let add = self.add_rect(r);
        let on = Rect::new(add.x - 8 - ARROW_W, add.y, ARROW_W, add.h);
        let back = Rect::new(on.x - PAGER_LABEL_W - ARROW_W, add.y, ARROW_W, add.h);
        Some((back, on))
    }

    /// Where "Back to song" is, while it shows: left of the pager, or of
    /// "+ Track".
    pub fn back_rect(&self, r: Rect) -> Option<Rect> {
        if !self.back_to_song {
            return None;
        }
        let right = self.pager_rects(r).map_or(self.add_rect(r), |(b, _)| b);
        Some(Rect::new(right.x - 8 - BACK_W, right.y, BACK_W, right.h))
    }

    /// Where the Stop all button is: over the scene buttons.
    pub fn stop_all_rect(&self, r: Rect) -> Rect {
        let g = self.geo(r);
        Rect::new(g.body.x, g.body.y, LABEL_W, self.head_h())
    }

    /// The track names' height: taller with a sound line under them.
    fn head_h(&self) -> i32 {
        if self.tracks.iter().any(|t| !t.sound.is_empty()) {
            HEAD_SOUND_H
        } else {
            HEAD_H
        }
    }

    fn geo(&self, r: Rect) -> Geo {
        // Matches PaintCx::titled_panel with the default theme's padding.
        let body = r.inset(14).take_top(34).1;
        let x0 = body.x + LABEL_W + CELL_GAP;
        let cols = GRID_COLS as i32;
        let col_w = ((body.right() - x0 - CELL_GAP * (cols - 1)) / cols).max(1);
        let cells_y = body.y + self.head_h() + CELL_GAP;
        let avail = body.bottom() - cells_y;
        let min_h = if self.big { ROW_MIN_H } else { ROW_MIN_H_MAKE };
        let rows =
            (((avail + CELL_GAP) / (min_h + CELL_GAP)).max(1) as usize).min(self.slots.max(1));
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
            self.head_h(),
        ))
    }

    /// Where track `track`'s sound line is (the lower half of its name),
    /// if it has one and is in view.
    pub fn sound_rect(&self, r: Rect, track: usize) -> Option<Rect> {
        if self.tracks.get(track)?.sound.is_empty() {
            return None;
        }
        let h = self.head_rect(r, track)?;
        Some(Rect::new(h.x, h.y + h.h / 2, h.w, h.h - h.h / 2))
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
            || d.can_add != self.can_add
            || d.adding != self.adding
            || d.big != self.big
            || d.back_to_song != self.back_to_song
            || d.dragged.is_some()
            || self.dragged_now().is_some()
        {
            return None;
        }
        let mut parts = Vec::new();
        for (i, (old, new)) in d.tracks.iter().zip(&self.tracks).enumerate() {
            if old.name != new.name
                || old.color != new.color
                || old.armed != new.armed
                || old.mute != new.mute
                || old.solo != new.solo
                || old.waiting != new.waiting
                || old.sound != new.sound
                || old.loops != new.loops
                || old.audio != new.audio
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
            can_add: self.can_add,
            adding: self.adding,
            big: self.big,
            back_to_song: self.back_to_song,
            dragged: self.dragged_now(),
        });
        // The title, shortened to what comes before " · " where "Back to
        // song" leaves it no room.
        let head = r.inset(14).take_top(26).0;
        let back = self.back_rect(r);
        let mut title = self.title.as_str();
        if let Some(b) = back {
            let wide = cx.measure(FontId::Bold, t.size_small, &title.to_uppercase());
            if head.x + wide + 12 > b.x {
                title = title.split(" · ").next().unwrap_or(title);
            }
        }
        cx.titled_panel(r, title, "");
        // The legend (or the pager), "+ Track", Edit, Rec song and the
        // Song button (the Song view of the page).
        let song = self.song_rect(r);
        let rec = self.rec_rect(r);
        let edit = self.edit_rect(r);
        let add = self.add_rect(r);
        if let Some(b) = back {
            cx.canvas
                .fill_round_rect(b, t.round(10), t.raised.mix(t.good, 0.35));
            cx.text(
                FontId::Bold,
                t.size_small,
                b,
                Align::Center,
                t.text,
                "Back to song",
            );
        }
        if let Some((back, on)) = self.pager_rects(r) {
            // The pager: the tracks shown, with a step either way, dim at
            // either end.
            let n = self.tracks.len();
            let last = (self.first_track + GRID_COLS).min(n);
            for (b, glyph, live) in [(back, "‹", self.first_track > 0), (on, "›", last < n)] {
                cx.canvas.fill_round_rect(b, t.round(10), t.raised);
                let ink = if live { t.text } else { t.text_faint };
                cx.text(FontId::Bold, t.size_body, b, Align::Center, ink, glyph);
            }
            let label = Rect::new(back.right(), back.y, on.x - back.right(), back.h);
            let mut words = format!("tracks {}–{last} of {n}", self.first_track + 1);
            if cx.measure(FontId::Sans, t.size_small, &words) > label.w - 8 {
                words = format!("{}–{last} of {n}", self.first_track + 1);
            }
            cx.text(
                FontId::Sans,
                t.size_small,
                label,
                Align::Center,
                t.text_dim,
                &words,
            );
        } else {
            // The legend's parts that fit between the title and the
            // buttons, dropping the first ones when it is too wide.
            let title_w = cx.measure(FontId::Bold, t.size_small, &title.to_uppercase());
            let end = back.map_or(add.x, |b| b.x);
            let room = end - 12 - (head.x + title_w + 16);
            let mut legend = self.legend.as_str();
            while !legend.is_empty() && cx.measure(FontId::Sans, t.size_small, legend) > room {
                legend = legend.split_once(" · ").map_or("", |(_, rest)| rest);
            }
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(head.x, head.y, end - head.x - 12, head.h),
                Align::Right,
                t.text_dim,
                legend,
            );
        }
        cx.canvas.fill_round_rect(add, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            add,
            Align::Center,
            if self.can_add { t.text } else { t.text_faint },
            "+ Track",
        );
        rec_song_pill(cx, rec, self.song_rec);
        cx.canvas.fill_round_rect(edit, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            t.size_small,
            edit,
            Align::Center,
            t.text,
            "Edit ›",
        );
        cx.canvas.fill_round_rect(song, t.round(10), t.raised);
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
            let ink = if playing { t.ink_on(fill) } else { t.text_dim };
            cx.canvas.fill_round_rect(b, t.round(10), fill);
            if self.scene_cued == Some(slot) {
                cx.canvas.stroke_round_rect(b, t.round(10), 2, t.warn);
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
        cx.canvas.fill_round_rect(stop, t.round(8), t.raised);
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
            let head_fill = base.mix(t.surface, 0.72);
            cx.canvas.fill_round_rect(head, t.round(8), head_fill);
            let two = !track.sound.is_empty();
            let mut name = if two {
                Rect::new(head.x + 10, head.y + 3, head.w - 20, 20)
            } else {
                head.inset_xy(10, 0)
            };
            if track.armed {
                cx.canvas.stroke_round_rect(head, t.round(8), 2, t.bad);
                cx.canvas.fill_round_rect(
                    Rect::new(name.x, name.y + name.h / 2 - 4, 8, 8),
                    4,
                    t.bad,
                );
                name = name.take_left(14).1;
            }
            if track.solo && !track.armed {
                cx.canvas.stroke_round_rect(head, t.round(8), 2, t.accent);
            }
            if let Some(w) = &track.waiting {
                // Hatched while a change waits for its line, with what and
                // when under the name (the second line, or the only one).
                hatch(cx, head, t.warn.mix(t.surface, 0.55));
                cx.canvas.stroke_round_rect(head, t.round(8), 1, t.warn);
                let line = if two {
                    Rect::new(head.x + 10, head.y + 22, head.w - 14, 18)
                } else {
                    Rect::new(head.x + 10, head.y + head.h - 16, head.w - 14, 14)
                };
                let px = t.size_small - 3.0;
                let w = waiting_fit(w, |s| cx.measure(FontId::Bold, px, s) <= line.w);
                let ink = t.readable(t.warn, head_fill);
                cx.text(FontId::Bold, px, line, Align::Left, ink, &w);
            } else if two {
                let mut sound = Rect::new(head.x + 10, head.y + 22, head.w - 16, 18);
                let dim = if track.mute { t.text_faint } else { t.text_dim };
                if track.loops || track.audio {
                    // A Loop or audio track's mark: a little wave before
                    // its input.
                    let cy = sound.y + sound.h / 2;
                    for (k, h) in [6, 10, 5].into_iter().enumerate() {
                        let x = sound.x + 3 * k as i32;
                        cx.canvas.fill_rect(Rect::new(x, cy - h / 2, 2, h), dim);
                    }
                    sound = sound.take_left(12).1;
                }
                cx.text(
                    FontId::Sans,
                    t.size_small - 2.0,
                    sound,
                    Align::Left,
                    dim,
                    &track.sound,
                );
            }
            let ink = if track.mute {
                t.text_faint
            } else {
                t.readable(base.mix(t.text, 0.25), head_fill)
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
                if track.audio && cell.state == CellState::Empty {
                    // An empty audio cell (layout spec 4f): dashed, with
                    // no +, as nothing is recorded into it here.
                    dashed_rect(cx, cr, t.outline);
                    continue;
                }
                paint_cell(cx, cr, cell, base);
            }
        }
        if let Some(gr) = self.grip.filter(|g| g.lifted) {
            self.paint_drag(cx, r, &gr);
        }
        if let Some(rows) = self.chooser_rects(r) {
            crate::song::paint_chooser(cx, &rows);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        // The chooser open: its rows, and anywhere else closes it.
        if let Some(rows) = self.chooser_rects(r) {
            let mut out: Vec<Target> = rows
                .iter()
                .enumerate()
                .map(|(k, b)| Target::tap("add-kind", k, crate::song::ADD_KINDS[k].0, *b))
                .collect();
            out.push(Target::tap("add-close", 0, "Close", r));
            return out;
        }
        let mut out = vec![
            Target::tap("song", 0, "Song", self.song_rect(r)),
            Target::tap("rec-song", 0, "Rec song", self.rec_rect(r)),
            Target::tap("edit", 0, "Edit", self.edit_rect(r)),
            Target::tap("add-track", 0, "+ Track", self.add_rect(r)).disabled_if(!self.can_add),
        ];
        if let Some(b) = self.back_rect(r) {
            out.push(Target::tap("back-to-song", 0, "Back to song", b));
        }
        if let Some((back, on)) = self.pager_rects(r) {
            let last = self.first_track + GRID_COLS >= self.tracks.len();
            out.push(
                Target::tap("pager", 0, "Tracks before", back).disabled_if(self.first_track == 0),
            );
            out.push(Target::tap("pager", 1, "Tracks after", on).disabled_if(last));
        }
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
        // The "+ Track" chooser open: a row adds that kind of track, and a
        // touch anywhere else only closes it.
        if self.adding {
            if phase != Touch::Down {
                return None;
            }
            let kind = self
                .chooser_rects(r)
                .and_then(|rows| rows.iter().position(|b| b.contains(x, y)));
            return Some(EventKind::Toggle(
                self.slots + kind.map_or(ADD_DISMISS, |k| ADD_KIND + k),
            ));
        }
        // A finger on a clip or a row's number: a tap on the lift, or
        // dragged, a drop where it lifts.
        if let Some(mut g) = self.grip {
            match phase {
                Touch::Move => {
                    g.at = (x, y);
                    let (dx, dy) = (x - g.from.0, y - g.from.1);
                    g.lifted |= dx * dx + dy * dy >= LIFT_PX * LIFT_PX;
                    let mut page = None;
                    let edge = g.lifted.then(|| self.edge_at(r, g.held, x, y)).flatten();
                    g.edge = match (edge, g.edge) {
                        (Some(e), Some((was, at))) if e == was && at.elapsed() < EDGE_REPEAT => {
                            Some((was, at))
                        }
                        (Some(e), _) => {
                            page = Some(EventKind::Toggle(self.slots + e));
                            Some((e, Instant::now()))
                        }
                        (None, _) => None,
                    };
                    self.grip = Some(g);
                    return page;
                }
                Touch::Up => {
                    self.grip = None;
                    g.at = (x, y);
                    if !g.lifted {
                        return Some(match g.held {
                            Held::Clip(t, s) => EventKind::Select(t * self.slots + s),
                            Held::Row(row) => EventKind::Toggle(row),
                        });
                    }
                    if let Some((t, s)) = self.clip_target(r, &g) {
                        let Held::Clip(t0, s0) = g.held else {
                            return None;
                        };
                        return Some(EventKind::Drop {
                            from: t0 * self.slots + s0,
                            to: t * self.slots + s,
                        });
                    }
                    if let (Some(gap), Held::Row(row)) = (self.row_target(r, &g), g.held) {
                        return Some(EventKind::Drop {
                            from: ROW_DROP + row,
                            to: gap,
                        });
                    }
                    // Lifted where it started, or off the grid: nothing.
                    return None;
                }
                Touch::Cancel => {
                    self.grip = None;
                    return None;
                }
                Touch::Down => self.grip = None,
            }
        }
        // A finger on a track's name: let go where it went down it arms
        // the track; swiped along the names it pages.
        match phase {
            Touch::Down => self.press = None,
            Touch::Move => return None,
            Touch::Cancel => {
                self.press = None;
                return None;
            }
            Touch::Up => {
                let (x0, track) = self.press.take()?;
                let dx = x - x0;
                return Some(if dx.abs() < SWIPE_PX {
                    EventKind::Tab(track)
                } else if dx < 0 {
                    EventKind::Toggle(self.slots + PAGE_ON)
                } else {
                    EventKind::Toggle(self.slots + PAGE_BACK)
                });
            }
        }
        // A row's scene button is Toggle(row); Stop all is Toggle(slots);
        // the Song button Toggle(slots + 1); Rec song Toggle(slots + 2);
        // Edit Toggle(slots + 3); "+ Track" and the pager as above.
        if self.song_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots + 1));
        }
        if self.rec_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots + 2));
        }
        if self.edit_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots + 3));
        }
        if self.back_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Toggle(self.slots + BACK_TO_SONG));
        }
        if self.add_rect(r).contains(x, y) {
            return self
                .can_add
                .then_some(EventKind::Toggle(self.slots + ADD_TRACK));
        }
        if let Some((back, on)) = self.pager_rects(r) {
            if back.contains(x, y) {
                return Some(EventKind::Toggle(self.slots + PAGE_BACK));
            }
            if on.contains(x, y) {
                return Some(EventKind::Toggle(self.slots + PAGE_ON));
            }
        }
        if self.tracks.is_empty() {
            return None;
        }
        if self.stop_all_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(self.slots));
        }
        for slot in self.first_row..self.slots {
            if self.scene_rect(r, slot).is_some_and(|b| b.contains(x, y)) {
                // Launched on the lift, unless it is dragged.
                self.grip = Some(Grip {
                    held: Held::Row(slot),
                    from: (x, y),
                    at: (x, y),
                    lifted: false,
                    edge: None,
                });
                return None;
            }
        }
        let tracks = self.first_track..(self.first_track + GRID_COLS).min(self.tracks.len());
        for track in tracks {
            if self.sound_rect(r, track).is_some_and(|s| s.contains(x, y)) {
                return Some(EventKind::Tab(SOUND_TAP + track));
            }
            if self.head_rect(r, track).is_some_and(|h| h.contains(x, y)) {
                self.press = Some((x, track));
                return None;
            }
            for slot in self.first_row..self.slots {
                if self
                    .cell_rect(r, track, slot)
                    .is_some_and(|c| c.contains(x, y))
                {
                    let filled = self.tracks[track]
                        .cells
                        .get(slot)
                        .is_some_and(|c| !matches!(c.state, CellState::Empty | CellState::Armed));
                    if !filled {
                        return Some(EventKind::Select(track * self.slots + slot));
                    }
                    // A clip launches on the lift, unless it is dragged.
                    self.grip = Some(Grip {
                        held: Held::Clip(track, slot),
                        from: (x, y),
                        at: (x, y),
                        lifted: false,
                        edge: None,
                    });
                    return None;
                }
            }
        }
        None
    }
}

/// The scene morph under the grid: the rows on A and B (a tap on a chip's
/// left half steps it down, on its right half up: `Step { index: 0 or 1,
/// delta }`), the crossfader between them (`Value`, while the finger is
/// down), Snap back, Keep and Save (`Tab(0)`, `Tab(1)`, `Tab(2)`), and
/// Reload clip (`Tab(3)`, sequencer S4, mockup C).
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
    /// The focused clip has a saved state: Reload clip is lit.
    pub reload: bool,
    /// Its reload waits for the launch line: Reload clip is outlined
    /// amber.
    pub reload_waiting: bool,
}

const CHIP_W: i32 = 96;
const MORPH_BUTTONS: [(&str, i32); 4] = [
    ("Snap back", 96),
    ("Keep", 64),
    ("Save", 76),
    ("Reload clip", 104),
];

impl Default for MorphBar {
    fn default() -> MorphBar {
        MorphBar {
            a: 0,
            b: 1,
            pos: 0.0,
            on: false,
            params: 0,
            reload: false,
            reload_waiting: false,
        }
    }
}

impl MorphBar {
    pub fn new() -> MorphBar {
        MorphBar::default()
    }

    /// The A chip, the crossfader, the B chip and the four buttons.
    pub fn parts(r: Rect) -> (Rect, Rect, Rect, [Rect; 4]) {
        let inner = r.inset_xy(14, 8);
        let gap = 8;
        let buttons_w: i32 = MORPH_BUTTONS.iter().map(|b| b.1 + gap).sum();
        let (buttons, rest) = inner.take_right(buttons_w);
        let mut x = buttons.x + gap;
        let mut bs = [Rect::new(0, 0, 0, 0); 4];
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
        cx.canvas
            .fill_round_rect(r.inset_xy(4, 2), t.round(12), t.surface);
        let (a, fader, b, buttons) = MorphBar::parts(r);
        for (chip, side, row, near) in [
            (a, "A", self.a, self.pos < 0.5),
            (b, "B", self.b, self.pos >= 0.5),
        ] {
            cx.canvas.fill_round_rect(chip, t.round(10), t.raised);
            if self.on && near {
                cx.canvas.stroke_round_rect(chip, t.round(10), 2, t.accent);
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
        cx.canvas.fill_round_rect(
            Rect::new(x - 9, fader.y + 4, 18, fader.h - 8),
            t.round(6),
            ink,
        );
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
            let live = match i {
                2 => true,
                3 => self.reload,
                _ => self.on,
            };
            let waiting = i == 3 && self.reload_waiting;
            cx.canvas
                .fill_round_rect(*br, t.round(10), if live { t.raised } else { t.surface });
            if waiting {
                cx.canvas.stroke_round_rect(*br, t.round(10), 2, t.warn);
            }
            let word = if i == 2 {
                format!("Save {}", if self.pos < 0.5 { "A" } else { "B" })
            } else {
                MORPH_BUTTONS[i].0.to_string()
            };
            let ink = if waiting {
                t.warn
            } else if live {
                t.text
            } else {
                t.text_faint
            };
            cx.text(FontId::Bold, t.size_small, *br, Align::Center, ink, &word);
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
            // Snap back and Keep are faint until the crossfader morphs,
            // Reload clip without a saved state.
            out.push(
                Target::tap("button", i, MORPH_BUTTONS[i].0, *br)
                    .disabled_if((i < 2 && !self.on) || (i == 3 && !self.reload)),
            );
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

/// A dashed outline round `r`: 6 px on, 5 px off along each side, clear
/// of the corners.
fn dashed_rect(cx: &mut PaintCx, r: Rect, color: Color) {
    let (on, off, inset) = (6, 5, 8);
    let mut x = r.x + inset;
    while x < r.right() - inset {
        let w = on.min(r.right() - inset - x);
        cx.canvas.fill_rect(Rect::new(x, r.y, w, 1), color);
        cx.canvas
            .fill_rect(Rect::new(x, r.bottom() - 1, w, 1), color);
        x += on + off;
    }
    let mut y = r.y + inset;
    while y < r.bottom() - inset {
        let h = on.min(r.bottom() - inset - y);
        cx.canvas.fill_rect(Rect::new(r.x, y, 1, h), color);
        cx.canvas
            .fill_rect(Rect::new(r.right() - 1, y, 1, h), color);
        y += on + off;
    }
}

fn paint_cell(cx: &mut PaintCx, cr: Rect, cell: &GridCell, base: Color) {
    let t = cx.theme.clone();
    if cell.state == CellState::Empty {
        cx.canvas.stroke_round_rect(cr, t.round(10), 1, t.outline);
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
        _ if cell.missing => t.surface.mix(t.raised, 0.5),
        CellState::Playing => t.raised.mix(base, 0.36),
        CellState::Recording => t.raised.mix(t.bad, 0.24),
        CellState::Armed => t.raised.mix(t.bad, 0.12),
        _ => t.raised.mix(base, 0.16),
    };
    cx.canvas.fill_round_rect(cr, t.round(10), fill);
    if cell.missing && cell.state != CellState::Queued {
        cx.canvas
            .stroke_round_rect(cr, t.round(10), 1, t.surface.mix(t.bad, 0.4));
    }
    if cr.h < COMPACT_H {
        paint_compact(cx, cr, cell, base, fill);
        return;
    }
    if cell.state == CellState::Armed {
        // The armed Loop cell: a red edge and a red dot before "Armed".
        cx.canvas
            .fill_round_rect(Rect::new(cr.x, cr.y + 6, 4, cr.h - 12), 2, t.bad);
        let inner = cr.inset_xy(10, 8);
        let (name, rest) = inner.take_top(20);
        cx.canvas
            .fill_round_rect(Rect::new(name.x, name.y + name.h / 2 - 4, 8, 8), 4, t.bad);
        let name = name.take_left(13).1;
        cx.text(
            FontId::Bold,
            t.size_small + 1.0,
            name,
            Align::Left,
            t.readable(t.bad, fill),
            &cell.name,
        );
        let (detail, rest) = rest.take_top(20);
        for (r, line) in [(detail, &cell.detail), (rest.take_top(20).0, &cell.foot)] {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                r,
                Align::Left,
                t.text,
                line,
            );
        }
        return;
    }
    match cell.state {
        CellState::Queued => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.warn),
        CellState::Recording => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.bad),
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
        CellState::Recording => t.readable(t.bad.mix(t.text, 0.2), fill),
        _ => t.readable(t.text_dim, fill),
    };
    let rolling = matches!(cell.state, CellState::Playing | CellState::Recording);
    // A loop shows its pass on its wave, not with a ring.
    let ring_room = if rolling && cell.wave.is_none() {
        RING + 4
    } else {
        0
    };
    let foot = Rect::new(
        inner.x,
        inner.bottom() - 18,
        (inner.w - ring_room).max(0),
        18,
    );
    let foot_ink = match cell.state {
        _ if cell.alert == CellAlert::Bad => t.readable(t.bad, fill),
        _ if cell.alert == CellAlert::Warn => t.readable(t.warn, fill),
        CellState::Queued => t.readable(t.warn, fill),
        CellState::Recording => t.readable(t.bad.mix(t.text, 0.2), fill),
        _ => t.text_faint,
    };
    let foot_face = if cell.alert == CellAlert::None {
        FontId::Sans
    } else {
        FontId::Bold
    };
    if cell.missing && cell.wave.is_none() {
        // Missing audio: a flat dashed line where its wave would be.
        let band = detail.inset_xy(0, 2);
        dashed_line(cx, band, t.surface.mix(t.bad, 0.45));
        if foot.y >= detail.bottom() - 2 {
            let line = if cell.foot.is_empty() { &cell.detail } else { &cell.foot };
            cx.text(foot_face, t.size_small - 1.0, foot, Align::Left, foot_ink, line);
        }
    } else if let Some(wave) = &cell.wave {
        // A loop or an audio clip: its wave in the detail's place, and at
        // the foot the foot, or the detail when there is none ("2 bars",
        // "overdub"). Playing, the wave is lit up to the playhead, as on
        // Play's strip.
        paint_wave(cx, detail.inset_xy(0, 2), wave, cell, base, fill);
        let (line, ink, face) = if cell.foot.is_empty() {
            (&cell.detail, detail_ink, FontId::Sans)
        } else {
            (&cell.foot, foot_ink, foot_face)
        };
        if foot.y >= detail.bottom() - 2 {
            cx.text(face, t.size_small - 1.0, foot, Align::Left, ink, line);
        }
    } else {
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            detail,
            Align::Left,
            detail_ink,
            &cell.detail,
        );
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
    }
    if rolling && cell.wave.is_none() && inner.h >= RING + 20 {
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
    } else if cell.state == CellState::Queued && cell.progress > 0.0 {
        // Perform's countdown: an amber bar along the foot that fills as
        // the line nears.
        let track = Rect::new(cr.x + 10, cr.bottom() - 7, cr.w - 20, 3);
        cx.canvas
            .fill_round_rect(track, 1, t.surface.mix(t.text_faint, 0.35));
        let w = (track.w as f32 * cell.progress.min(1.0)) as i32;
        cx.canvas
            .fill_round_rect(Rect::new(track.x, track.y, w.max(2), 3), 1, t.warn);
    }
}

/// A loop cell's small wave in `band`, lit up to the playhead while it
/// plays, as on Play's strip.
fn paint_wave(
    cx: &mut PaintCx,
    band: Rect,
    wave: &[u8; CELL_WAVE],
    cell: &GridCell,
    base: Color,
    fill: Color,
) {
    let t = cx.theme.clone();
    let rolling = matches!(cell.state, CellState::Playing | CellState::Recording);
    let lit = match cell.state {
        CellState::Recording => fill.mix(t.bad.mix(t.text, 0.2), 0.85),
        _ => fill.mix(base.mix(t.text, 0.45), 0.9),
    };
    let ink = fill.mix(base.mix(t.text, 0.3), 0.7);
    let dim = fill.mix(base.mix(t.text, 0.3), 0.4);
    // A once audio clip ringing past its stop shows how far it has got
    // too, its playhead dimmed.
    let head = (rolling || cell.ringing).then(|| band.x + (band.w as f32 * cell.progress) as i32);
    let cy = band.y + band.h / 2;
    for (c, &p) in wave.iter().enumerate() {
        let x0 = band.x + band.w * c as i32 / CELL_WAVE as i32;
        let x1 = band.x + band.w * (c as i32 + 1) / CELL_WAVE as i32;
        let h = (i32::from(p) * band.h / 255).max(1);
        let ink = match head {
            Some(hx) if x0 < hx => lit,
            Some(_) => dim,
            None => ink,
        };
        cx.canvas
            .fill_rect(Rect::new(x0, cy - h / 2, (x1 - x0 - 1).max(1), h), ink);
    }
    if let Some(hx) = head {
        let ink = if rolling { t.text } else { t.text_faint };
        cx.canvas
            .fill_rect(Rect::new(hx, band.y - 1, 1, band.h + 2), ink);
    }
}

/// A flat dashed line across the middle of `band`: a missing audio
/// clip's wave.
fn dashed_line(cx: &mut PaintCx, band: Rect, color: Color) {
    let y = band.y + band.h / 2;
    let mut x = band.x;
    while x < band.right() {
        let w = 3.min(band.right() - x);
        cx.canvas.fill_rect(Rect::new(x, y, w, 1), color);
        x += 6;
    }
}

/// Make's compact cell (sequencer decision 5: eight rows with names and
/// lengths): the name, and under it the length, or when a queued clip
/// starts ("from 7.1"), or a loop's wave; a small ring while it plays.
fn paint_compact(cx: &mut PaintCx, cr: Rect, cell: &GridCell, base: Color, fill: Color) {
    let t = cx.theme.clone();
    match cell.state {
        CellState::Queued => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.warn),
        CellState::Recording => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.bad),
        CellState::Armed => {
            cx.canvas
                .fill_round_rect(Rect::new(cr.x, cr.y + 4, 4, cr.h - 8), 2, t.bad)
        }
        _ => {}
    }
    let rolling = matches!(cell.state, CellState::Playing | CellState::Recording);
    let ringed = rolling && cell.wave.is_none();
    let inner = cr.inset_xy(9, 4);
    let inner = if ringed {
        inner.take_right(SMALL_RING + 4).1
    } else {
        inner
    };
    let (mut name, line) = inner.take_top(inner.h / 2);
    let mut name_ink = t.text;
    if cell.state == CellState::Armed {
        cx.canvas
            .fill_round_rect(Rect::new(name.x, name.y + name.h / 2 - 4, 8, 8), 4, t.bad);
        name = name.take_left(13).1;
        name_ink = t.readable(t.bad, fill);
    }
    cx.text(
        FontId::Bold,
        t.size_small,
        name,
        Align::Left,
        name_ink,
        &cell.name,
    );
    let queued = cell.state == CellState::Queued;
    // A word that matters takes the wave's place while the clip is
    // stopped or queued (an audio clip's "made at 120", "missing").
    let said = cell.alert != CellAlert::None && !rolling && !cell.ringing;
    match &cell.wave {
        Some(wave) if !queued && !said => {
            paint_wave(cx, line.inset_xy(0, 3), wave, cell, base, fill)
        }
        _ => {
            let (words, ink) = match cell.state {
                _ if said && cell.alert == CellAlert::Bad => (&cell.foot, t.bad),
                _ if said => (&cell.foot, t.warn),
                CellState::Queued => (&cell.foot, t.warn),
                CellState::Recording => (&cell.detail, t.bad.mix(t.text, 0.2)),
                CellState::Armed => (&cell.detail, t.text),
                _ => (&cell.detail, t.text_dim),
            };
            let ink = t.readable(ink, fill);
            let face = if said { FontId::Bold } else { FontId::Sans };
            cx.text(face, t.size_small - 2.0, line, Align::Left, ink, words);
        }
    }
    if ringed {
        let color = if cell.state == CellState::Recording {
            t.bad
        } else {
            t.good
        };
        ring(
            cx.canvas,
            cr.right() - SMALL_RING - 6,
            cr.y + (cr.h - SMALL_RING) / 2,
            SMALL_RING,
            3.0,
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
/// on Capture `Tab(2)`, faint or not: the page says why a faint one does
/// nothing.
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
                cx.canvas.fill_round_rect(row, t.round(8), t.raised);
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
                    cx.canvas.fill_round_rect(b, t.round(12), t.raised);
                    // Capture stands out: there is something to keep.
                    let ink = if i == 2 {
                        cx.canvas.stroke_round_rect(b, t.round(12), 2, t.accent);
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
                    cx.canvas.stroke_round_rect(b, t.round(12), 1, t.outline);
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
        let live = [&self.undo, &self.redo, &self.capture];
        (0..3)
            .map(|i| {
                Target::tap(
                    "button",
                    i,
                    ["Undo", "Redo", "Capture"][i],
                    TakeList::button_rect(r, i),
                )
                .disabled_if(live[i].is_none())
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        (0..3)
            .find(|&i| TakeList::button_rect(r, i).contains(x, y))
            .map(EventKind::Tab)
    }
}

/// A grid header's waiting line made to fit (`fits`): "mutes in 2
/// beats", else "mutes 2 beats", else "mutes" (the transport bar's
/// WAITING reading still says where it lands).
fn waiting_fit(full: &str, mut fits: impl FnMut(&str) -> bool) -> String {
    if fits(full) {
        return full.to_string();
    }
    let short = full.replacen(" in ", " ", 1);
    if fits(&short) {
        return short;
    }
    full.split(" in ").next().unwrap_or(full).to_string()
}

/// Diagonal stripes over `r` in `c`, 3 px wide every 9 px, inside the
/// canvas's clip (a mute or solo change waiting, the grid's header).
fn hatch(cx: &mut PaintCx, r: Rect, c: Color) {
    let was = cx.canvas.clip();
    cx.canvas.set_clip(was.intersect(r));
    for y in r.y..r.y + r.h {
        let shift = (y - r.y) % 9;
        let mut x = r.x - 9 + shift;
        while x < r.x + r.w {
            cx.canvas.fill_rect(Rect::new(x, y, 3, 1), c);
            x += 9;
        }
    }
    cx.canvas.set_clip(was);
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas, Theme, Ui};

    #[test]
    fn a_waiting_line_drops_words_until_it_fits() {
        let fits = |n: usize| move |s: &str| s.len() <= n;
        assert_eq!(
            waiting_fit("mutes in 2 beats", fits(20)),
            "mutes in 2 beats"
        );
        assert_eq!(waiting_fit("mutes in 2 beats", fits(13)), "mutes 2 beats");
        assert_eq!(waiting_fit("solo off in 2 beats", fits(13)), "solo off");
    }

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
                sound: String::new(),
                color: Color::rgb(0x3DDC97),
                armed: i == 1,
                mute: false,
                solo: false,
                waiting: None,
                loops: false,
                audio: false,
                cells,
            });
        }
        g
    }

    #[test]
    fn grid_cells_and_names_report_where_they_are() {
        let mut g = grid(10);
        let r = Rect::new(12, 128, 814, 610);
        // Make's compact rows; Perform's big ones (sequencer S4).
        let rows = g.visible_rows(r);
        assert!((9..=10).contains(&rows), "{rows}");
        let c = g.cell_rect(r, 2, 3).unwrap();
        assert!(c.h >= ROW_MIN_H_MAKE && c.h < COMPACT_H, "{c:?}");
        g.big = true;
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
        // A track's name arms it on the lift.
        let h = g.head_rect(r, 5).unwrap();
        assert_eq!(g.touch(1, Touch::Down, h.x + 2, h.y + 2, r), None);
        assert_eq!(
            g.touch(1, Touch::Up, h.x + 6, h.y + 2, r),
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

    /// With what each track plays under its name, the names are taller
    /// (the cells move down), and a tap on the sound line is its own:
    /// `Tab(SOUND_TAP + track)`, while the name above still arms.
    #[test]
    fn a_tracks_sound_line_sits_under_its_name_and_is_tapped_on_its_own() {
        let mut g = grid(10);
        let r = Rect::new(12, 128, 814, 610);
        let (one_line, first_cell) = (g.head_rect(r, 3).unwrap(), g.cell_rect(r, 3, 0).unwrap());
        assert_eq!(g.sound_rect(r, 3), None, "no sound line");
        for (i, t) in g.tracks.iter_mut().enumerate() {
            t.sound = if i == 3 { "FM".into() } else { "Kit".into() };
        }
        let h = g.head_rect(r, 3).unwrap();
        assert_eq!(h.h, HEAD_SOUND_H);
        assert!(h.h > one_line.h);
        assert_eq!(
            g.cell_rect(r, 3, 0).unwrap().y,
            first_cell.y + HEAD_SOUND_H - HEAD_H
        );
        let s = g.sound_rect(r, 3).unwrap();
        assert!(h.intersect(s) == s && s.y > h.y);
        assert_eq!(
            g.touch(1, Touch::Down, s.x + 4, s.bottom() - 2, r),
            Some(EventKind::Tab(SOUND_TAP + 3))
        );
        // The name arms on the lift (a swipe along the names pages).
        assert_eq!(g.touch(1, Touch::Down, h.x + 4, h.y + 2, r), None);
        assert_eq!(
            g.touch(1, Touch::Up, h.x + 4, h.y + 2, r),
            Some(EventKind::Tab(3))
        );
        // Off to the right: no sound line in view.
        assert_eq!(g.sound_rect(r, 9), None);
    }

    /// Past 8 tracks the pager takes the legend's place: ‹ and ›, or a
    /// swipe along the names, page; "+ Track" is lit while there is room.
    #[test]
    fn the_pager_and_plus_track_and_a_swipe_on_the_names() {
        let mut g = grid(24);
        let r = Rect::new(12, 128, 814, 610);
        let add = g.add_rect(r);
        let edit = g.edit_rect(r);
        assert!(add.right() < edit.x, "{add:?} {edit:?}");
        let (back, on) = g.pager_rects(r).unwrap();
        assert!(back.right() < on.x && on.right() < add.x);
        assert!(back.x > r.x + 240, "clear of the title: {back:?}");
        let tap = |g: &mut ClipGrid, b: Rect| g.touch(1, Touch::Down, b.x + 3, b.y + 3, r);
        assert_eq!(tap(&mut g, add), None, "no room: dim");
        g.can_add = true;
        assert_eq!(tap(&mut g, add), Some(EventKind::Toggle(16 + ADD_TRACK)));
        assert_eq!(tap(&mut g, back), Some(EventKind::Toggle(16 + PAGE_BACK)));
        assert_eq!(tap(&mut g, on), Some(EventKind::Toggle(16 + PAGE_ON)));
        // A swipe to the left along the names: the next 8; to the right:
        // the 8 before. Neither arms a track.
        let h = g.head_rect(r, 3).unwrap();
        g.touch(1, Touch::Down, h.x + 60, h.y + 4, r);
        g.touch(1, Touch::Move, h.x + 20, h.y + 4, r);
        assert_eq!(
            g.touch(1, Touch::Up, h.x - 40, h.y + 4, r),
            Some(EventKind::Toggle(16 + PAGE_ON))
        );
        g.touch(1, Touch::Down, h.x + 2, h.y + 4, r);
        assert_eq!(
            g.touch(1, Touch::Up, h.x + 90, h.y + 4, r),
            Some(EventKind::Toggle(16 + PAGE_BACK))
        );
        // A lost touch does nothing.
        g.touch(1, Touch::Down, h.x + 2, h.y + 4, r);
        assert_eq!(g.touch(1, Touch::Cancel, h.x + 2, h.y + 4, r), None);
        assert_eq!(g.touch(1, Touch::Up, h.x + 2, h.y + 4, r), None);
        // Eight tracks: no pager.
        assert!(grid(8).pager_rects(r).is_none());
    }

    /// "Back to song" shows while clips play over the song, left of the
    /// pager or "+ Track", and the title gives way to it.
    #[test]
    fn back_to_song_shows_left_of_the_pager_and_taps() {
        let r = Rect::new(12, 128, 814, 610);
        for tracks in [8, 24] {
            let mut g = grid(tracks);
            assert_eq!(g.back_rect(r), None);
            assert!(g.targets(r).iter().all(|t| t.id != "back-to-song"));
            g.back_to_song = true;
            let b = g.back_rect(r).unwrap();
            let right = g.pager_rects(r).map_or(g.add_rect(r), |(back, _)| back);
            assert!(b.right() < right.x, "{b:?} {right:?}");
            assert!(b.x > r.x + 100, "clear of the title's start: {b:?}");
            assert!(g.targets(r).iter().any(|t| t.id == "back-to-song"));
            assert_eq!(
                g.touch(1, Touch::Down, b.x + 3, b.y + 3, r),
                Some(EventKind::Toggle(16 + BACK_TO_SONG))
            );
        }
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

    /// Jay, 14 Nov: drag and drop on the Clips grid. A clip lifts once
    /// the finger moves a fingertip and drops on the cell under the lift;
    /// not lifted, it is a tap (on the lift); a row's number drags the
    /// row to a gap; at an edge the tracks page.
    #[test]
    fn clips_and_rows_drag_and_drop_and_a_still_finger_taps() {
        let mut g = grid(10);
        let r = Rect::new(12, 128, 814, 610);
        let mid = |c: Rect| (c.x + c.w / 2, c.y + c.h / 2);
        let beat = mid(g.cell_rect(r, 2, 0).unwrap());
        let there = mid(g.cell_rect(r, 4, 3).unwrap());
        assert_eq!(g.touch(1, Touch::Down, beat.0, beat.1, r), None);
        assert_eq!(g.touch(1, Touch::Move, beat.0 + 30, beat.1 + 40, r), None);
        assert!(g.dragging());
        assert_eq!(g.touch(1, Touch::Move, there.0, there.1, r), None);
        let (theme, mut fonts) = (Theme::dark(), Fonts::embedded());
        let mut c = SoftCanvas::new(1280, 800);
        let mut paint = |g: &ClipGrid| {
            g.paint(
                &mut PaintCx {
                    canvas: &mut c,
                    theme: &theme,
                    fonts: &mut fonts,
                },
                r,
            )
        };
        paint(&g);
        assert_eq!(
            g.touch(1, Touch::Up, there.0, there.1, r),
            Some(EventKind::Drop {
                from: 2 * 16,
                to: 4 * 16 + 3
            })
        );
        assert!(!g.dragging());
        // A wobble under a fingertip: a tap, which launches it.
        g.touch(1, Touch::Down, beat.0, beat.1, r);
        g.touch(1, Touch::Move, beat.0 + 5, beat.1 + 5, r);
        assert_eq!(
            g.touch(1, Touch::Up, beat.0 + 5, beat.1 + 5, r),
            Some(EventKind::Select(2 * 16))
        );
        // Lifted where it started, or cancelled: nothing.
        g.touch(1, Touch::Down, beat.0, beat.1, r);
        g.touch(1, Touch::Move, beat.0, beat.1 + 60, r);
        assert_eq!(g.touch(1, Touch::Up, beat.0, beat.1, r), None);
        g.touch(1, Touch::Down, beat.0, beat.1, r);
        g.touch(1, Touch::Move, there.0, there.1, r);
        assert_eq!(g.touch(1, Touch::Cancel, there.0, there.1, r), None);
        // An empty slot is not dragged: a touch makes a clip there, as
        // before.
        assert_eq!(
            g.touch(1, Touch::Down, there.0, there.1, r),
            Some(EventKind::Select(4 * 16 + 3))
        );
        g.touch(1, Touch::Up, there.0, there.1, r);
        // At the right edge the tracks after show.
        g.touch(1, Touch::Down, beat.0, beat.1, r);
        g.touch(1, Touch::Move, beat.0, beat.1 + 40, r);
        let edge = (r.right() - 18, beat.1 + 40);
        assert_eq!(
            g.touch(1, Touch::Move, edge.0, edge.1, r),
            Some(EventKind::Toggle(16 + PAGE_ON))
        );
        assert_eq!(g.touch(1, Touch::Move, edge.0, edge.1 + 1, r), None, "once");
        g.touch(1, Touch::Cancel, edge.0, edge.1, r);
        // Row 4's number, dragged up between rows 1 and 2.
        let four = mid(g.scene_rect(r, 3).unwrap());
        let gap = g.scene_rect(r, 1).unwrap().y - 2;
        g.touch(1, Touch::Down, four.0, four.1, r);
        g.touch(1, Touch::Move, four.0, gap, r);
        paint(&g);
        assert_eq!(
            g.touch(1, Touch::Up, four.0, gap, r),
            Some(EventKind::Drop {
                from: ROW_DROP + 3,
                to: 1
            })
        );
        // Lifted next to where it was: it would land where it is.
        g.touch(1, Touch::Down, four.0, four.1, r);
        g.touch(1, Touch::Move, four.0, four.1 + 20, r);
        assert_eq!(g.touch(1, Touch::Up, four.0, four.1 + 20, r), None);
    }

    #[test]
    fn row_numbers_launch_scenes_and_the_corner_stops_all() {
        let mut g = grid(3);
        let r = Rect::new(12, 128, 814, 610);
        let b = g.scene_rect(r, 2).unwrap();
        let c = g.cell_rect(r, 0, 2).unwrap();
        assert_eq!((b.y, b.h), (c.y, c.h), "beside its row");
        assert!(b.right() < c.x && b.w >= 44, "{b:?} {c:?}");
        // On the lift, as a row's number can be dragged.
        assert_eq!(g.touch(1, Touch::Down, b.x + 4, b.y + 4, r), None);
        assert_eq!(
            g.touch(1, Touch::Up, b.x + 4, b.y + 4, r),
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

    /// Faint or not, a take button answers (the page says why a faint one
    /// does nothing); its target is off while it is faint.
    #[test]
    fn take_buttons_answer_and_are_off_while_faint() {
        let mut l = TakeList::new("Takes");
        let r = Rect::new(0, 0, 400, 260);
        let undo = TakeList::button_rect(r, 0);
        let redo = TakeList::button_rect(r, 1);
        assert!(undo.right() < redo.x && redo.right() <= r.right() - 14);
        let on = |l: &TakeList| l.targets(r).iter().map(|t| t.enabled).collect::<Vec<_>>();
        assert_eq!(on(&l), [false, false, false]);
        assert_eq!(
            l.touch(1, Touch::Down, undo.x + 2, undo.y + 2, r),
            Some(EventKind::Tab(0))
        );
        l.undo = Some("take 1".into());
        assert_eq!(on(&l), [true, false, false]);
        l.redo = Some("take 2".into());
        assert_eq!(
            l.touch(1, Touch::Down, redo.x + 2, redo.y + 2, r),
            Some(EventKind::Tab(1))
        );
        assert_eq!(on(&l), [true, true, false]);
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
