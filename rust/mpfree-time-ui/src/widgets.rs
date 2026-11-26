//! The widgets the time engine adds to mpfree-ui: the transport bar
//! ([`TransportBar`]), the Clips launch grid ([`ClipGrid`]), the scene
//! morph bar under it ([`MorphBar`]) and the takes list ([`TakeList`]). Like mpfree-ui's own widgets, each one only shows
//! what the app put in its fields and reports touches; none of them knows
//! about the transport, the clips or the audio thread.

use crate::chord_strip::{
    chip_rects, paint_picker, paint_strip, picker_geo, picker_hit, strip_hit, ChordStrip,
    PickerGeo, PickerView, StripHit, PICKS, PICK_DISMISS,
};
use mpfree_ui::widgets::Chip;
use mpfree_ui::{
    Align, Canvas, Class, Color, Commit, Consequence, Context, EventKind, Fingers, FontId,
    Movement, PaintCx, Rect, Target, Timing, Tone, Touch, Widget,
};
use std::cell::RefCell;
use std::f32::consts::TAU;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- drawing

/// The loop mark (the ⟳ the screens' fonts don't have) in `r`: a circle
/// open just before twelve o'clock, its arrowhead pointing on clockwise.
pub fn loop_mark(canvas: &mut dyn Canvas, r: Rect, color: Color) {
    let s = r.w.min(r.h) as f32;
    if s < 6.0 {
        return;
    }
    let (cx, cy) = (r.x as f32 + s / 2.0, r.y as f32 + s / 2.0);
    let rad = s / 2.0 - 1.5;
    let up = -std::f32::consts::FRAC_PI_2;
    let (a0, a1) = (up + 0.55, up + TAU - 0.45);
    let mut m = mpfree_ui::Shape::new(r.inset_xy(-2, -2));
    m.arc(cx, cy, rad, a0, a1, 1.6);
    // The arrowhead at the arc's end, along its way round.
    let (p, d, n) = (
        (cx + rad * a1.cos(), cy + rad * a1.sin()),
        (-a1.sin(), a1.cos()),
        (a1.cos(), a1.sin()),
    );
    let tip = (p.0 + d.0 * 2.2, p.1 + d.1 * 2.2);
    let wing = s / 5.0;
    for side in [1.0f32, -1.0] {
        let w = (
            p.0 - d.0 * 1.2 + n.0 * wing * side,
            p.1 - d.1 * 1.2 + n.1 * wing * side,
        );
        m.segment(tip, w, 1.6);
    }
    m.draw(canvas, color);
}

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
    /// A change waits for its line on this playing cell (another clip,
    /// a stop, the song coming back): the detail says what and when
    /// ("until 7.1", "to song 7.1"), in amber, apart from what plays now
    /// (the Arrange refactor's 5.1, D3).
    pub cued: bool,
    /// For a playing cell with a stop or Back to song waiting, in
    /// Perform: how near its line is, an amber bar along its foot as a
    /// queued cell's (0 draws none).
    pub countdown: f32,
    /// A part being tried here, or a kept clip off its row's chords
    /// (composing tools S2.6).
    pub mark: CellMark,
    /// A loop cell on an audio track (phase 4, layout spec 4f): the loop
    /// mark before its name, and the deck's ring at its right in the
    /// track's colour, stopped or not (`progress`), never a wave.
    pub looped: bool,
    /// A layer of its loop not yet on the stick: an amber dot at the
    /// name's right, until its file is written.
    pub writing: bool,
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

/// A mark on a cell (composing tools S2.6, layout spec 4r).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CellMark {
    #[default]
    None,
    /// A part being tried in this empty cell: dashed in the track's
    /// colour, its name in it.
    Try,
    /// A kept clip with notes off its row's chords: a dashed outline in
    /// the warning colour, and its detail ("doesn't fit") in it.
    Unfit,
}

/// Add a part on the Clips title line (composing tools S2.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PartButton {
    /// Ready to open the panel.
    #[default]
    Ready,
    /// The panel is open: outlined in the accent colour.
    Open,
    /// Rec song is on (a try would be recorded into the song): faint, and
    /// a tap says why.
    Faint,
}

/// Columns of a loop cell's small wave ([`GridCell::wave`]).
pub const CELL_WAVE: usize = 24;

/// A track's SONG tag on its column's header (the Arrange refactor's
/// 5.1, D3): what it plays now, the song or a clip over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SongMark {
    /// No tag: song mode is off, or the song has nothing on the track.
    #[default]
    None,
    /// It plays the song now: a filled green tag.
    Plays,
    /// A clip launched on it plays over the song: an outlined tag. A
    /// stop, or Back to song, gives it the song again.
    Under,
}

/// The SONG tag's words.
pub const SONG_TAG: &str = "SONG";

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
    /// An audio track (layout spec 4f): a wave mark before its input,
    /// under its name, and its cells empty and dashed until audio clips
    /// come.
    pub audio: bool,
    /// Its SONG tag, on the name line's right.
    pub song: SongMark,
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
///
/// Add a part (composing tools S2.6, layout spec 4r) sits left of "+ Track"
/// and gives `Toggle(slots + `[`ADD_PART`]`)`; with the pager and "Back to
/// song" both showing there is no room and it steps aside. The row's
/// chords ([`ClipGrid::strip`]) take one row's place under their row: a
/// tap on a chip gives `Toggle(slots + `[`STRIP_CHIP`]` + k)`, on a bar of
/// a strip with no chords `Toggle(slots + `[`STRIP_BAR`]` + bar)`. The chord
/// picker ([`ClipGrid::picker`]) opens under a chip; while it is open
/// every touch is its own, `Toggle(slots + `[`PICKER`]` + answer)` with
/// the answers of [`crate::chord_strip`].
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
    /// A set is open (live shows B1): **Show** (72 px) goes back to the
    /// Show view, left of the title line's other buttons, and gives
    /// `Toggle(slots + `[`SHOW_VIEW`]`)`.
    pub show_button: bool,
    /// Perform's big rows (sequencer decision 5, mockup C): five rows of
    /// [`ROW_MIN_H`] with rings and countdowns. Off (Make), the rows are
    /// compact, [`ROW_MIN_H_MAKE`], eight in view with names and lengths.
    /// On, nothing on the grid drags: a clip or a row's number acts on the
    /// lift only while the finger is still on it.
    pub big: bool,
    /// The scene card, open over the grid (the Arrange refactor's 5.2):
    /// it takes the grid's touches while it is.
    pub scene_card: Option<crate::scene_card::SceneCard>,
    /// Add a part on the title line.
    pub add_part: PartButton,
    /// The row outlined in the accent colour (the scene pick while Add a
    /// part or the Chords tab is open), with its chords under it.
    pub outlined: Option<usize>,
    /// The row's chords, one row high under their row (Make's height in
    /// Perform): the rows under it move down one. Taps, drops and the gaps
    /// between rows skip it, and it isn't drawn while a clip or a row is
    /// dragged.
    pub strip: Option<ChordStrip>,
    /// The chord picker, open under one of the strip's chips.
    pub picker: Option<PickerView>,
    /// Scene banks (the Arrange refactor's 5.6): a bank of
    /// [`mpfree_seq::SLOTS`] rows each, true where one of its rows has a
    /// clip or a named scene. With more than one, the bank chip shows
    /// them right of the view chip; empty, it isn't there.
    pub banks: Vec<bool>,
    /// The Perform pads' clip window (the Arrange refactor's 5.5), while
    /// one is on the pads: its first track and row. Four tracks by four
    /// rows from there are outlined in the accent, where they are in view.
    pub pad_window: Option<(usize, usize)>,
    /// Capture scene can keep something (the Arrange refactor's 5.3): a
    /// track plays a clip, or the song plays. Perform's title line has
    /// the button, left of "Back to song", the pager or "+ Track". It
    /// gives `Toggle(slots + `[`CAPTURE`]`)`, and is faint while this is
    /// off.
    pub can_capture: bool,
    /// An audio track's input list is open over the grid (phase 4, board
    /// 4; [`crate::inputs`]): the grid is veiled but for this track's
    /// column head, outlined, and a touch anywhere on it gives
    /// `Toggle(slots + `[`VEIL_TAP`]`)`, which closes the list and does
    /// nothing else.
    pub veil: Option<usize>,
    /// What was drawn last, so a change redraws only the cells that changed.
    drawn: RefCell<Option<Drawn>>,
    /// Each finger is its own (the touch usability review, 25 Nov 2026:
    /// with one of each for every finger, a second finger took the first
    /// one's place, so two clips held at once launched the wrong one, or
    /// a wobble moved it). These hold the fingers down, by finger.
    ///
    /// Fingers down on a track's name: where (x) and which track. Let go
    /// near where it went down it arms the track; swiped along the names
    /// it pages.
    press: Vec<(u64, (i32, usize))>,
    /// Fingers on a clip or a row's number, and where each has dragged it.
    grip: Fingers<Grip>,
    /// Fingers on an empty cell: (track, slot) and where it went down.
    /// It makes a clip on the lift; one that moves off as a drag or a
    /// scroll does nothing (the Arrange refactor's 5.4).
    empty: Vec<(u64, EmptyCell)>,
}

/// A finger on an empty cell: (track, slot) and where it went down.
type EmptyCell = (usize, usize, (i32, i32));

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

/// A finger moves this far before what it holds lifts (Make only).
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
    show_button: bool,
    add_part: PartButton,
    outlined: Option<usize>,
    strip: Option<ChordStrip>,
    picker: Option<PickerView>,
    veil: Option<usize>,
    /// Where a dragged clip or row was drawn.
    dragged: Option<(Held, (i32, i32))>,
    /// The scene card was drawn over it.
    card: bool,
    banks: Vec<bool>,
    pad_window: Option<(usize, usize)>,
    can_capture: bool,
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
/// (`song::ADD_KINDS`: Note track, Audio track).
pub const ADD_KIND: usize = 10;
/// `Toggle(slots + ADD_DISMISS)`: a touch off the open chooser, which
/// only closes it.
pub const ADD_DISMISS: usize = 13;
/// `Toggle(slots + SHOW_VIEW)`: Show, back to the Show view while a set is
/// open (live shows B1). After the view chip's List ([`LIST_VIEW`]).
pub const SHOW_VIEW: usize = 15;
/// `Toggle(slots + VEIL_TAP)`: a touch on the grid while an input list is
/// open over it ([`ClipGrid::veil`]).
pub const VEIL_TAP: usize = 16;
/// The Show button's width, on the grid, the Song view and the song list
/// (layout spec 4p).
pub const SHOW_W: i32 = 72;
/// `Toggle(slots + ADD_PART)`: Add a part. Past the grid's other toggles
/// (Arrange's take 16 to 31), under the strip's.
pub const ADD_PART: usize = 32;
/// `Toggle(slots + STRIP_CHIP + k)`: chip `k` of the row's chords.
pub const STRIP_CHIP: usize = 64;
/// `Toggle(slots + STRIP_BAR + bar)`: a bar of a strip with no chords yet.
pub const STRIP_BAR: usize = 128;
/// `Toggle(slots + PICKER + answer)`: the open chord picker
/// (`chord_strip::PICK_*`).
pub const PICKER: usize = 256;
/// Add a part's width.
const PART_W: i32 = 116;
/// "Back to song"'s width.
const BACK_W: i32 = 124;
/// "Back to song"'s and the pager's label's widths where Show shares the
/// title line with both and the view chip leaves no room for all three at
/// their usual widths (the Clips view beside the Record panel, more than
/// eight tracks): the 36 pixels Show needs to clear the chip.
const BACK_TIGHT_W: i32 = 100;
const PAGER_LABEL_TIGHT_W: i32 = 92;
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

/// `Toggle(slots + LIST_VIEW)`: the view chip's List.
pub const LIST_VIEW: usize = 14;
/// `Toggle(slots + BANK + k)`: the bank chip's bank `k`, A to D (scene
/// banks, the Arrange refactor's 5.6): the grid scrolls to its first row.
/// After audio tracks' [`VEIL_TAP`].
pub const BANK: usize = 17;
/// The bank chip's letters, a part each.
pub const BANK_LETTERS: [&str; 4] = ["A", "B", "C", "D"];
/// A part of the bank chip.
pub const BANK_PART_W: i32 = 28;
/// `Toggle(slots + CAPTURE)`: Perform's Capture scene (the Arrange
/// refactor's 5.3), after the bank chip's four.
pub const CAPTURE: usize = 21;
/// Capture scene's width.
const CAPTURE_W: i32 = 128;

/// Arrange's three views of one song, in the order the view chip shows
/// them (the Arrange refactor's 5.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrangeView {
    Clips,
    List,
    Song,
}

impl ArrangeView {
    pub const ALL: [ArrangeView; 3] = [ArrangeView::Clips, ArrangeView::List, ArrangeView::Song];

    /// Its word on the chip.
    pub fn word(self) -> &'static str {
        match self {
            ArrangeView::Clips => "Clips",
            ArrangeView::List => "List",
            ArrangeView::Song => "Song",
        }
    }
}

/// The view chip's parts' width, and the whole chip's.
pub const VIEW_PART_W: i32 = 58;
pub const VIEW_CHIP_W: i32 = 3 * VIEW_PART_W;

/// Where the view chip is in an Arrange view's panel `r`: the left end of
/// its title line, where the view's name was.
pub fn view_chip_rect(r: Rect) -> Rect {
    let head = r.inset(14).take_top(26).0;
    Rect::new(head.x, head.y - 4, VIEW_CHIP_W, 30)
}

/// Where the title's words start, right of the view chip.
pub fn view_title_x(r: Rect) -> i32 {
    view_chip_rect(r).right() + 12
}

/// Part `v` of the view chip `chip`.
pub fn view_part_rect(chip: Rect, v: ArrangeView) -> Rect {
    let k = ArrangeView::ALL.iter().position(|a| *a == v).unwrap_or(0) as i32;
    Rect::new(chip.x + k * VIEW_PART_W, chip.y, VIEW_PART_W, chip.h)
}

/// The view a touch at (x, y) on the chip picks, if it is on it.
pub fn view_part_at(chip: Rect, x: i32, y: i32) -> Option<ArrangeView> {
    ArrangeView::ALL
        .into_iter()
        .find(|v| view_part_rect(chip, *v).contains(x, y))
}

/// The view chip: Clips, List and Song side by side, the view showing
/// filled in the accent colour.
pub fn paint_view_chip(cx: &mut PaintCx, chip: Rect, on: ArrangeView) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(chip, t.round(10), t.raised);
    for v in ArrangeView::ALL {
        let b = view_part_rect(chip, v);
        let ink = if v == on {
            let lit = b.inset_xy(2, 2);
            cx.canvas.fill_round_rect(lit, t.round(8), t.accent);
            t.ink_on(t.accent)
        } else {
            t.text
        };
        cx.text(FontId::Bold, t.size_small, b, Align::Center, ink, v.word());
    }
}

/// An Arrange view's title line: the panel, the view chip and the
/// title's words after it (in capitals, as a panel's title is), up to
/// `end` (the buttons on its right).
pub fn view_title(cx: &mut PaintCx, r: Rect, on: ArrangeView, title: &str, end: i32) {
    view_title_at(cx, r, on, title, view_title_x(r), end);
}

/// An Arrange view's title line as [`view_title`] draws it, with the
/// title's words from `x` (right of a chip of the view's own).
pub fn view_title_at(cx: &mut PaintCx, r: Rect, on: ArrangeView, title: &str, x: i32, end: i32) {
    let t = cx.theme.clone();
    cx.titled_panel(r, "", "");
    paint_view_chip(cx, view_chip_rect(r), on);
    let head = r.inset(14).take_top(26).0;
    cx.text(
        FontId::Bold,
        t.size_small,
        Rect::new(x, head.y, (end - x).max(0), head.h),
        Align::Left,
        t.text_dim,
        &title.to_uppercase(),
    );
}

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
    /// The row's chords in view: under row `k` in view (0 is the first
    /// shown), and how tall.
    strip: Option<(usize, i32)>,
}

impl Geo {
    /// The top of row `k` in view: rows under the strip move down one.
    fn row_top(&self, k: usize) -> i32 {
        let mut y = self.cells_y + k as i32 * (self.row_h + CELL_GAP);
        if let Some((under, h)) = self.strip {
            if k > under {
                y += h + CELL_GAP;
            }
        }
        y
    }

    /// Where the strip is, across the whole body (its "chords" label in
    /// the row numbers' column), if it is in view.
    fn strip_rect(&self) -> Option<Rect> {
        let (under, h) = self.strip?;
        let y = self.row_top(under) + self.row_h + CELL_GAP;
        Some(Rect::new(self.body.x, y, self.body.w, h))
    }

    /// The row in view at height `y` (with the gap under it), if any: none
    /// over the strip.
    fn row_at(&self, y: i32) -> Option<usize> {
        (0..self.rows).find(|&k| {
            let top = self.row_top(k);
            y >= top && y < top + self.row_h + CELL_GAP
        })
    }

    /// The middle of the gap before row `k` in view (`k` up to `rows`):
    /// the gap under a row with chords is the one above them.
    fn boundary(&self, k: usize) -> i32 {
        match self.strip {
            Some((under, _)) if k == under + 1 => self.row_top(under) + self.row_h + CELL_GAP / 2,
            _ => self.row_top(k) - CELL_GAP / 2,
        }
    }
}

impl ClipGrid {
    /// The row whose number a finger holds still (not dragged), for the
    /// app's hold that opens its scene card.
    pub fn held_row(&self) -> Option<usize> {
        self.grip.iter().find_map(|(_, g)| match *g {
            Grip {
                held: Held::Row(row),
                lifted: false,
                ..
            } => Some(row),
            _ => None,
        })
    }

    /// The hold on a row's number opened its scene card: the finger's lift
    /// launches nothing and drops nothing.
    pub fn let_go_of_row(&mut self) {
        let still = |g: &Grip| matches!(g.held, Held::Row(_)) && !g.lifted;
        let held = self.grip.iter().find(|(_, g)| still(g)).map(|(f, _)| f);
        if let Some(f) = held {
            self.grip.take(f);
        }
    }

    /// Whether (x, y) is still on what a finger holds: its clip's cell or
    /// its row's number.
    fn still_on(&self, r: Rect, held: Held, x: i32, y: i32) -> bool {
        match held {
            Held::Clip(t, s) => self.cell_rect(r, t, s).is_some_and(|c| c.contains(x, y)),
            Held::Row(row) => self.scene_rect(r, row).is_some_and(|b| b.contains(x, y)),
        }
    }

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
            show_button: false,
            scene_card: None,
            add_part: PartButton::Ready,
            outlined: None,
            strip: None,
            picker: None,
            banks: Vec::new(),
            pad_window: None,
            can_capture: false,
            veil: None,
            drawn: RefCell::new(None),
            press: Vec::new(),
            grip: Fingers::new(),
            empty: Vec::new(),
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
        let row = g.row_at(y)?;
        let (track, slot) = (self.first_track + c, self.first_row + row);
        (c < GRID_COLS && track < self.tracks.len() && row < g.rows && slot < self.slots)
            .then_some((track, slot))
    }

    /// The gap between rows a row dragged to height `y` goes in: before
    /// row `gap`, from the first row in view to just after the last.
    fn gap_at(&self, r: Rect, y: i32) -> usize {
        let g = self.geo(r);
        let last = (self.first_row + g.rows).min(self.slots);
        let k = (0..=last - self.first_row)
            .min_by_key(|&k| (g.boundary(k) - y).abs())
            .unwrap_or(0);
        self.first_row + k
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
        self.grip.any(|g| g.lifted)
    }

    fn dragged_now(&self) -> Option<(Held, (i32, i32))> {
        self.grip
            .iter()
            .find(|(_, g)| g.lifted)
            .map(|(_, g)| (g.held, g.at))
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
                    let y = g.boundary(gap - self.first_row) - 2;
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
        Some(Rect::new(
            g.body.x,
            g.row_top(slot - self.first_row),
            LABEL_W,
            g.row_h,
        ))
    }

    /// Where the view chip is: the left end of the title line (5.7).
    pub fn view_rect(&self, r: Rect) -> Rect {
        view_chip_rect(r)
    }

    /// Where the Rec song button is: the right end of the title line.
    pub fn rec_rect(&self, r: Rect) -> Rect {
        let head = r.inset(14).take_top(26).0;
        Rect::new(head.right() - REC_SONG_W, head.y - 4, REC_SONG_W, 30)
    }

    /// Where the title line's left side ends: before Capture scene,
    /// "Back to song", the pager, Add a part or "+ Track".
    fn left_end(&self, r: Rect) -> i32 {
        self.show_rect(r)
            .or(self.capture_rect(r))
            .or(self.back_rect(r))
            .or(self.pager_rects(r).map(|(b, _)| b))
            .map_or(self.adds_left(r).x, |b| b.x)
            - 12
    }

    /// Where Capture scene is (the Arrange refactor's 5.3): in Perform,
    /// left of "Back to song", the pager, Add a part or "+ Track", with Show (while a
    /// set is open) left of it. Where that leaves it no room right of the
    /// view chip ("Back to song" beside the pager, or Show beside the
    /// pager) it isn't there, and the scene card's Capture is the way.
    pub fn capture_rect(&self, r: Rect) -> Option<Rect> {
        if !self.big {
            return None;
        }
        let right = self
            .back_rect(r)
            .or(self.pager_rects(r).map(|(b, _)| b))
            .unwrap_or(self.adds_left(r));
        let x = right.x - 8 - CAPTURE_W;
        let show = if self.show_button { SHOW_W + 8 } else { 0 };
        (x >= self.view_rect(r).right() + 10 + show)
            .then(|| Rect::new(x, right.y, CAPTURE_W, right.h))
    }

    /// Where the bank chip is (scene banks, the Arrange refactor's 5.6):
    /// right of the view chip, a part for each bank, while there is more
    /// than one. "Back to song" beside the pager (more than 8 tracks)
    /// leaves it no room: then it isn't there, and the row numbers' bank
    /// letters and SHIFT + wheel are the way between banks.
    pub fn bank_chip_rect(&self, r: Rect) -> Option<Rect> {
        if self.banks.len() < 2 {
            return None;
        }
        let v = self.view_rect(r);
        let x = v.right() + 10;
        let w = self.banks.len() as i32 * BANK_PART_W;
        (x + w <= self.left_end(r)).then(|| Rect::new(x, v.y, w, v.h))
    }

    /// The bank chip's part for bank `k`.
    pub fn bank_part_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let chip = self.bank_chip_rect(r)?;
        (k < self.banks.len())
            .then(|| Rect::new(chip.x + k as i32 * BANK_PART_W, chip.y, BANK_PART_W, chip.h))
    }

    /// The bank the grid shows, lit on the bank chip: the bank of the
    /// middle row in view (rows `rows` in view; of two, the lower).
    pub fn bank_shown(&self, rows: usize) -> usize {
        let middle = self.first_row + rows / 2;
        (middle / mpfree_seq::SLOTS).min(self.banks.len().saturating_sub(1))
    }

    /// Where the title's words start: right of the bank chip, else of
    /// the view chip.
    fn title_x(&self, r: Rect) -> i32 {
        self.bank_chip_rect(r)
            .map_or(view_title_x(r), |c| c.right() + 12)
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
    pub fn chooser_rects(&self, r: Rect) -> Option<[Rect; crate::song::KINDS]> {
        if !self.adding {
            return None;
        }
        let add = self.add_rect(r);
        let x = (add.right() - crate::song::CHOOSER_W).max(r.x);
        Some(crate::song::chooser_rows(x, add.bottom() + 6))
    }

    /// Where Add a part is: left of "+ Track", unless two of the pager,
    /// "Back to song" and Show (live shows) show, which leave it no room.
    /// Arrange's buttons keep their places: where Perform's Capture scene
    /// or the bank chip would have to give way to it, it steps aside too.
    pub fn add_part_rect(&self, r: Rect) -> Option<Rect> {
        let others = [self.paged(), self.back_to_song, self.show_button];
        if others.iter().filter(|&&on| on).count() >= 2 {
            return None;
        }
        let add = self.add_rect(r);
        let part = Rect::new(add.x - 8 - PART_W, add.y, PART_W, add.h);
        // The title line's buttons left of it, at their usual widths (the
        // narrow ones are only for all three of the above).
        let mut left = part.x;
        if self.paged() {
            left -= 8 + 2 * ARROW_W + PAGER_LABEL_W;
        }
        if self.back_to_song {
            left -= 8 + BACK_W;
        }
        if self.big {
            left -= 8 + CAPTURE_W;
        }
        if self.show_button {
            left -= 8 + SHOW_W;
        }
        let mut need = self.view_rect(r).right() + 10;
        if self.banks.len() >= 2 {
            need += self.banks.len() as i32 * BANK_PART_W + 12;
        }
        (left >= need).then_some(part)
    }

    /// The leftmost of Add a part and "+ Track".
    fn adds_left(&self, r: Rect) -> Rect {
        self.add_part_rect(r).unwrap_or(self.add_rect(r))
    }

    /// Where the pager's ‹ and › are, left of Add a part (or "+ Track")
    /// with its label between them, while it shows.
    pub fn pager_rects(&self, r: Rect) -> Option<(Rect, Rect)> {
        if !self.paged() {
            return None;
        }
        let add = self.adds_left(r);
        let label = if self.tight(r) {
            PAGER_LABEL_TIGHT_W
        } else {
            PAGER_LABEL_W
        };
        let on = Rect::new(add.x - 8 - ARROW_W, add.y, ARROW_W, add.h);
        let back = Rect::new(on.x - label - ARROW_W, add.y, ARROW_W, add.h);
        Some((back, on))
    }

    /// Whether Show, "Back to song" and the pager all share the title line
    /// and would reach the view chip at their usual widths, so "Back to
    /// song" and the pager's label narrow.
    fn tight(&self, r: Rect) -> bool {
        if !(self.show_button && self.back_to_song && self.paged()) {
            return false;
        }
        let wide = 8 + 2 * ARROW_W + PAGER_LABEL_W + 8 + BACK_W + 8 + SHOW_W;
        self.add_rect(r).x - wide < self.view_rect(r).right() + 8
    }

    /// Where "Back to song" is, while it shows: left of the pager, or of
    /// Add a part.
    pub fn back_rect(&self, r: Rect) -> Option<Rect> {
        if !self.back_to_song {
            return None;
        }
        let right = self.pager_rects(r).map_or(self.adds_left(r), |(b, _)| b);
        let w = if self.tight(r) { BACK_TIGHT_W } else { BACK_W };
        Some(Rect::new(right.x - 8 - w, right.y, w, right.h))
    }

    /// Where Show is, while a set is open: left of the title line's other
    /// buttons (Perform's Capture scene, "Back to song", the pager, Add a
    /// part or "+ Track").
    pub fn show_rect(&self, r: Rect) -> Option<Rect> {
        if !self.show_button {
            return None;
        }
        let right = self
            .capture_rect(r)
            .or(self.back_rect(r))
            .or(self.pager_rects(r).map(|(b, _)| b))
            .unwrap_or(self.adds_left(r));
        Some(Rect::new(right.x - 8 - SHOW_W, right.y, SHOW_W, right.h))
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
        let fit = |avail: i32| ((avail + CELL_GAP) / (min_h + CELL_GAP)).max(1) as usize;
        let height = |avail: i32, n: usize| ((avail - CELL_GAP * (n as i32 - 1)) / n as i32).max(1);
        let slots = self.slots.max(1);
        let rows = fit(avail).min(slots);
        let mut g = Geo {
            body,
            x0,
            col_w,
            cells_y,
            row_h: height(avail, rows),
            rows,
            strip: None,
        };
        // The row's chords take one row's place under their row, Make's
        // height in Perform, while their row stays in view.
        let Some(s) = &self.strip else {
            return g;
        };
        let (rows, row_h, h) = if self.big {
            let left = avail - ROW_MIN_H_MAKE - CELL_GAP;
            let rows = fit(left).min(slots);
            (rows, height(left, rows), ROW_MIN_H_MAKE)
        } else {
            let n = fit(avail).min(slots + 1);
            let row_h = height(avail, n);
            (n.saturating_sub(1), row_h, row_h)
        };
        if rows > 0 && s.row >= self.first_row && s.row < self.first_row + rows {
            g.rows = rows;
            g.row_h = row_h;
            g.strip = Some((s.row - self.first_row, h));
        }
        g
    }

    /// Where the row's chords are: the "chords" label in the row numbers'
    /// column and the bars across the cells, if they are in view.
    pub fn strip_rects(&self, r: Rect) -> Option<(Rect, Rect)> {
        let g = self.geo(r);
        let s = g.strip_rect()?;
        let label = Rect::new(s.x, s.y, LABEL_W, s.h);
        let bars = Rect::new(g.x0, s.y, g.body.right() - g.x0, s.h);
        Some((label, bars))
    }

    /// Where each chip of the row's chords is, while they are in view.
    pub fn chip_rects(&self, r: Rect) -> Vec<Rect> {
        match (&self.strip, self.strip_rects(r)) {
            (Some(s), Some((_, bars))) => chip_rects(bars, s),
            _ => Vec::new(),
        }
    }

    /// Where the open chord picker's parts are: under its chip, inside
    /// the grid.
    pub fn picker_geo(&self, r: Rect) -> Option<PickerGeo> {
        let p = self.picker.as_ref()?;
        let chip = *self.chip_rects(r).get(p.chip)?;
        Some(picker_geo(chip, r))
    }

    /// The outline round the outlined row (and its chords under it), if
    /// in view.
    fn outline_rect(&self, r: Rect) -> Option<Rect> {
        let row = self.outlined?;
        let g = self.geo(r);
        let first = self.cell_rect(r, self.first_track, row)?;
        let mut bottom = first.bottom();
        if let (Some(s), Some(band)) = (&self.strip, g.strip_rect()) {
            if s.row == row {
                bottom = band.bottom();
            }
        }
        Some(Rect::new(
            g.x0 - 4,
            first.y - 4,
            g.body.right() - g.x0 + 8,
            bottom - first.y + 8,
        ))
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

    /// The outline of the Perform pads' clip window ([`ClipGrid::pad_window`]):
    /// round the cells of it in view, in the gaps between cells, so a cell
    /// drawn again leaves it be. None with no window, or none of it in
    /// view.
    pub fn pad_window_rect(&self, r: Rect) -> Option<Rect> {
        let (track, row) = self.pad_window?;
        let cells: Vec<Rect> = (track..track + 4)
            .flat_map(|t| (row..row + 4).map(move |s| (t, s)))
            .filter(|&(t, _)| t < self.tracks.len())
            .filter_map(|(t, s)| self.cell_rect(r, t, s))
            .collect();
        let first = cells.first()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.right(), first.bottom());
        for c in &cells {
            x0 = x0.min(c.x);
            y0 = y0.min(c.y);
            x1 = x1.max(c.right());
            y1 = y1.max(c.bottom());
        }
        let pad = CELL_GAP / 2;
        Some(Rect::new(
            x0 - pad,
            y0 - pad,
            x1 - x0 + 2 * pad,
            y1 - y0 + 2 * pad,
        ))
    }

    /// Where slot `slot` of track `track` is, if it is in view.
    pub fn cell_rect(&self, r: Rect, track: usize, slot: usize) -> Option<Rect> {
        let g = self.geo(r);
        let c = self.col(track)?;
        if slot < self.first_row || slot >= self.first_row + g.rows || slot >= self.slots {
            return None;
        }
        Some(Rect::new(
            g.x0 + c * (g.col_w + CELL_GAP),
            g.row_top(slot - self.first_row),
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
            || d.show_button != self.show_button
            || d.add_part != self.add_part
            || d.outlined != self.outlined
            || d.strip != self.strip
            || d.picker != self.picker
            || d.veil != self.veil
            || self.veil.is_some()
            || d.dragged.is_some()
            || self.dragged_now().is_some()
            || d.card
            || self.scene_card.is_some()
            || d.banks != self.banks
            || d.pad_window != self.pad_window
            || d.can_capture != self.can_capture
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
            show_button: self.show_button,
            add_part: self.add_part,
            outlined: self.outlined,
            strip: self.strip.clone(),
            picker: self.picker.clone(),
            veil: self.veil,
            dragged: self.dragged_now(),
            card: self.scene_card.is_some(),
            banks: self.banks.clone(),
            pad_window: self.pad_window,
            can_capture: self.can_capture,
        });
        // The view chip, then the title, shortened to what comes before
        // " · " where the buttons on its line leave it no room.
        let head = r.inset(14).take_top(26).0;
        let title_x = self.title_x(r);
        let back = self.back_rect(r);
        let show = self.show_rect(r);
        let capture = self.capture_rect(r);
        let mut title = self.title.as_str();
        // What the title has before it: Show, Capture scene, "Back to
        // song", the pager or Add a part.
        if show.is_some()
            || capture.is_some()
            || back.is_some()
            || self.paged()
            || self.add_part_rect(r).is_some()
        {
            let x = self.left_end(r);
            let mut wide = |s: &str| cx.measure(FontId::Bold, t.size_small, &s.to_uppercase());
            if title_x + wide(title) > x {
                title = title.split(" · ").next().unwrap_or(title);
            }
            if title_x + wide(title) > x {
                // The bank chip and the row numbers say which rows show.
                title = "";
            }
        }
        let end = show
            .or(capture)
            .or(back)
            .map_or(self.adds_left(r).x, |b| b.x)
            - 12;
        view_title_at(cx, r, ArrangeView::Clips, title, title_x, end);
        if let Some(chip) = self.bank_chip_rect(r) {
            // The bank chip: A to D, the bank shown filled in the accent,
            // a bank with nothing in it dim.
            let shown = self.bank_shown(self.geo(r).rows);
            cx.canvas.fill_round_rect(chip, t.round(10), t.raised);
            for (k, used) in self.banks.iter().enumerate() {
                let Some(b) = self.bank_part_rect(r, k) else {
                    continue;
                };
                let ink = if k == shown {
                    cx.canvas
                        .fill_round_rect(b.inset_xy(2, 2), t.round(8), t.accent);
                    t.ink_on(t.accent)
                } else if *used {
                    t.text
                } else {
                    t.text_dim
                };
                let letter = BANK_LETTERS.get(k).copied().unwrap_or("?");
                cx.text(FontId::Bold, t.size_small, b, Align::Center, ink, letter);
            }
        }
        // The legend (or the pager), "+ Track", Edit and Rec song.
        let rec = self.rec_rect(r);
        let edit = self.edit_rect(r);
        let add = self.add_rect(r);
        if let Some(b) = show {
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            cx.text(FontId::Bold, t.size_small, b, Align::Center, t.text, "Show");
        }
        if let Some(b) = capture {
            cx.canvas.fill_round_rect(b, t.round(10), t.raised);
            let ink = if self.can_capture {
                t.text
            } else {
                t.text_faint
            };
            cx.text(
                FontId::Bold,
                t.size_small,
                b,
                Align::Center,
                ink,
                "Capture scene",
            );
        }
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
            let end = show
                .or(capture)
                .or(back)
                .map_or(self.adds_left(r).x, |b| b.x);
            let room = end - 12 - (title_x + title_w + 16);
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
        if let Some(part) = self.add_part_rect(r) {
            cx.canvas.fill_round_rect(part, t.round(10), t.raised);
            let ink = match self.add_part {
                PartButton::Ready => t.text,
                PartButton::Open => {
                    cx.canvas.stroke_round_rect(part, t.round(10), 2, t.accent);
                    t.readable(t.accent, t.raised)
                }
                PartButton::Faint => t.text_faint,
            };
            cx.text(
                FontId::Bold,
                t.size_small,
                part,
                Align::Center,
                ink,
                "Add a part",
            );
        }
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
            if !self.banks.is_empty() && slot.is_multiple_of(mpfree_seq::SLOTS) {
                // A bank's first row: its letter in the button's corner,
                // and under the rows before it a line across the grid.
                let k = slot / mpfree_seq::SLOTS;
                let letter = BANK_LETTERS.get(k).copied().unwrap_or("?");
                let ink = if playing { ink } else { t.accent };
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    Rect::new(b.x + 5, b.y + 2, 16, 18),
                    Align::Left,
                    ink,
                    letter,
                );
                if row > 0 {
                    let y = b.y - CELL_GAP / 2 - 1;
                    cx.canvas.fill_rect(
                        Rect::new(b.x, y, g.body.right() - b.x, 2),
                        t.accent.with_alpha(140),
                    );
                }
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
            // What it plays now (D3): the SONG tag, filled while it
            // plays the song and outlined while a clip plays over it, at
            // the right of the sound line (of the name line when there is
            // none, or while a mute waits there), muted or not.
            let mut sound_w = head.w - 16;
            if track.song != SongMark::None {
                let px = t.size_small - 5.0;
                let w = cx.measure(FontId::Bold, px, SONG_TAG) + 8;
                let on_sound = two && track.waiting.is_none();
                let line = if on_sound {
                    Rect::new(head.x + 10, head.y + 22, sound_w, 18)
                } else {
                    name
                };
                if line.w > w + 24 {
                    let tag = Rect::new(line.right() - w, line.y + line.h / 2 - 7, w, 14);
                    let green = if track.mute {
                        t.good.mix(head_fill, 0.45)
                    } else {
                        t.good
                    };
                    let ink = if track.song == SongMark::Plays {
                        cx.canvas.fill_round_rect(tag, 7, green);
                        t.ink_on(green)
                    } else {
                        cx.canvas.stroke_round_rect(tag, 7, 1, green);
                        t.readable(green, head_fill)
                    };
                    cx.text(FontId::Bold, px, tag, Align::Center, ink, SONG_TAG);
                    if on_sound {
                        sound_w -= w + 4;
                    } else {
                        name = name.take_right(w + 4).1;
                    }
                }
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
                let mut sound = Rect::new(head.x + 10, head.y + 22, sound_w, 18);
                let dim = if track.mute { t.text_faint } else { t.text_dim };
                if track.audio {
                    // An audio track's mark: a little wave before its
                    // input.
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
        // The row's chords and the picked row's outline, but not while a
        // clip or a row is dragged.
        if !self.dragging() {
            if let (Some(s), Some((label, bars))) = (&self.strip, self.strip_rects(r)) {
                paint_strip(cx, label, bars, s);
            }
            if let Some(o) = self.outline_rect(r) {
                cx.canvas.stroke_round_rect(o, t.round(12), 2, t.accent);
            }
        }
        if let Some(w) = self.pad_window_rect(r) {
            cx.canvas.stroke_round_rect(w, t.round(8), 2, t.accent);
        }
        for (_, gr) in self.grip.iter().filter(|(_, g)| g.lifted) {
            self.paint_drag(cx, r, gr);
        }
        if let Some(rows) = self.chooser_rects(r) {
            crate::song::paint_chooser(cx, &rows);
        }
        if let (Some(p), Some(pg)) = (&self.picker, self.picker_geo(r)) {
            paint_picker(cx, &pg, p);
        }
        // An input list open over the grid: all but the track's column
        // head behind a veil, the head outlined as the list is.
        if let Some(track) = self.veil {
            let head = self.head_rect(r, track).unwrap_or_default();
            let veil = t.bg.with_alpha(180);
            for part in r.uncovered(&[head]) {
                cx.canvas.fill_rect(part, veil);
            }
            if !head.is_empty() {
                cx.canvas.stroke_round_rect(head, t.round(8), 2, t.good);
            }
        }
        if let Some(card) = &self.scene_card {
            card.paint(cx, r);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        if let Some(card) = &self.scene_card {
            return card.targets(r);
        }
        // The chord picker open: its parts, and anywhere else closes it.
        if let (Some(p), Some(g)) = (&self.picker, self.picker_geo(r)) {
            use crate::chord_strip::{
                picker_buttons, KINDS, PICK_CLOSE, PICK_KIND, PICK_SPLIT, ROOTS,
            };
            let mut out = Vec::new();
            for (i, b) in g.roots.iter().enumerate() {
                out.push(Target::tap("chord-root", i, ROOTS[i], *b));
            }
            for (i, b) in g.kinds.iter().enumerate() {
                out.push(Target::tap("chord-kind", PICK_KIND + i, KINDS[i], *b));
            }
            for (i, ((name, live), b)) in picker_buttons(p).into_iter().zip(g.buttons).enumerate() {
                out.push(Target::tap("chord-button", PICK_SPLIT + i, name, b).disabled_if(!live));
            }
            out.push(Target::tap("chord-close", PICK_CLOSE, "Close", g.close));
            out.push(Target::tap("chord-dismiss", PICK_DISMISS, "Close", r));
            return out;
        }
        // An input list open over the grid: a touch anywhere closes it.
        if self.veil.is_some() {
            return vec![Target::tap("veil", 0, "Close the input list", r)];
        }
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
        let chip = self.view_rect(r);
        let mut out = vec![
            Target::tap("view", 1, "List", view_part_rect(chip, ArrangeView::List)),
            Target::tap("view", 2, "Song", view_part_rect(chip, ArrangeView::Song)),
            Target::tap("rec-song", 0, "Rec song", self.rec_rect(r)),
            Target::tap("edit", 0, "Edit", self.edit_rect(r)),
            Target::tap("add-track", 0, "+ Track", self.add_rect(r)).disabled_if(!self.can_add),
        ];
        if let Some(b) = self.back_rect(r) {
            out.push(Target::tap("back-to-song", 0, "Back to song", b));
        }
        if let Some(b) = self.show_rect(r) {
            out.push(Target::tap("show-view", 0, "Show", b));
        }
        if let Some(b) = self.add_part_rect(r) {
            out.push(
                Target::tap("add-part", 0, "Add a part", b)
                    .disabled_if(self.add_part == PartButton::Faint),
            );
        }
        if let Some(b) = self.capture_rect(r) {
            out.push(
                Target::tap("capture-scene", 0, "Capture scene", b).disabled_if(!self.can_capture),
            );
        }
        // The bank shown is lit and faint as a target: a tap there takes
        // the grid where it is (as the zoom chip's lit part).
        let shown = self.bank_shown(self.geo(r).rows);
        for k in 0..self.banks.len() {
            if let Some(b) = self.bank_part_rect(r, k) {
                let letter = BANK_LETTERS.get(k).copied().unwrap_or("?");
                out.push(
                    Target::tap("bank", k, format!("Bank {letter}"), b).disabled_if(k == shown),
                );
            }
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
        // What the grid's controls say about touch (layout spec 1a), as
        // they act today: Stop all on the press; a clip, a row's number
        // and a track's name on the lift. In Perform (`big`) the clips and
        // rows are played and never move; in Make they are rows of one
        // kind that drag.
        let (class, context, movement) = if self.big {
            (Class::Performance, Context::Perform, Movement::None)
        } else {
            (Class::Dense, Context::Make, Movement::Structural)
        };
        let launch = |t: Target| {
            t.class(class)
                .plays(context, Consequence::Disruptive, Timing::Quantized)
                .movement(movement)
        };
        out.push(
            Target::tap("stop-all", 0, "Stop all", self.stop_all_rect(r))
                .class(Class::Critical)
                .plays(Context::Both, Consequence::Disruptive, Timing::Immediate)
                .commit(Commit::Press)
                .object("stop all"),
        );
        if let Some(s) = self.strip.as_ref().filter(|_| !self.dragging()) {
            for (k, c) in self.chip_rects(r).into_iter().enumerate() {
                let name = format!("Chord {}", s.chips[k].name);
                out.push(Target::tap("chord", k, name, c));
            }
            if s.chips.is_empty() {
                if let Some((_, bars)) = self.strip_rects(r) {
                    out.push(Target::tap("chord-bar", 0, "Set a chord", bars));
                }
            }
        }
        for slot in self.first_row..self.slots {
            if let Some(b) = self.scene_rect(r, slot) {
                let t = Target::tap("scene", slot, format!("Scene {}", slot + 1), b);
                out.push(launch(t).object(format!("row {}", slot + 1)));
            }
        }
        let tracks = self.first_track..(self.first_track + GRID_COLS).min(self.tracks.len());
        for track in tracks {
            let name = &self.tracks[track].name;
            if let Some(h) = self.head_rect(r, track) {
                let t = Target::tap("track", track, name.clone(), h)
                    .plays(Context::Make, Consequence::Disruptive, Timing::None)
                    .object(format!("track {}", track + 1));
                out.push(t);
            }
            for slot in self.first_row..self.slots {
                if let Some(c) = self.cell_rect(r, track, slot) {
                    let label = format!("{name} slot {}", slot + 1);
                    let t = Target::tap("slot", track * self.slots + slot, label, c);
                    let at = format!("track {} row {}", track + 1, slot + 1);
                    out.push(launch(t).object(at));
                }
            }
        }
        out
    }

    fn touch(&mut self, f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        // The scene card open: its touches, and one off it closes it.
        if let Some(card) = self.scene_card.as_mut() {
            self.grip.clear();
            self.empty.clear();
            self.press.clear();
            return card.touch(phase, x, y, r);
        }
        // The chord picker open: its parts, and a touch off it closes it
        // with nothing changed.
        if self.picker.is_some() {
            if phase != Touch::Down {
                return None;
            }
            let hit = self
                .picker_geo(r)
                .map_or(PICK_DISMISS, |g| picker_hit(&g, x, y));
            return (hit < PICKS).then_some(EventKind::Toggle(self.slots + PICKER + hit));
        }
        // An input list open over the grid: a touch only closes it.
        if self.veil.is_some() {
            return (phase == Touch::Down).then_some(EventKind::Toggle(self.slots + VEIL_TAP));
        }
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
        // A finger on an empty cell: a tap on the lift; moved off as a
        // drag or a scroll, nothing.
        if let Some(i) = self.empty.iter().position(|(g, _)| *g == f) {
            let (_, (track, slot, from)) = self.empty[i];
            match phase {
                Touch::Move => {
                    let (dx, dy) = (x - from.0, y - from.1);
                    if dx * dx + dy * dy >= LIFT_PX * LIFT_PX {
                        self.empty.remove(i);
                    }
                    return None;
                }
                Touch::Up => {
                    self.empty.remove(i);
                    return Some(EventKind::Select(track * self.slots + slot));
                }
                Touch::Cancel => {
                    self.empty.remove(i);
                    return None;
                }
                Touch::Down => {
                    self.empty.remove(i);
                }
            }
        }
        // A finger on a clip or a row's number: a tap on the lift, or
        // dragged, a drop where it lifts. Perform launches and never
        // moves anything: nothing lifts there, and a finger slid off what
        // it held lets go of it.
        if let Some(mut g) = self.grip.get(f).copied() {
            match phase {
                Touch::Move => {
                    g.at = (x, y);
                    let (dx, dy) = (x - g.from.0, y - g.from.1);
                    g.lifted |= !self.big && dx * dx + dy * dy >= LIFT_PX * LIFT_PX;
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
                    self.grip.put(f, g);
                    return page;
                }
                Touch::Up => {
                    self.grip.take(f);
                    g.at = (x, y);
                    if !g.lifted {
                        if self.big && !self.still_on(r, g.held, x, y) {
                            return None;
                        }
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
                    self.grip.take(f);
                    return None;
                }
                Touch::Down => {
                    self.grip.take(f);
                }
            }
        }
        // A finger on a track's name: let go where it went down it arms
        // the track; swiped along the names it pages.
        match phase {
            Touch::Down => self.press.retain(|(g, _)| *g != f),
            Touch::Move => return None,
            Touch::Cancel => {
                self.press.retain(|(g, _)| *g != f);
                return None;
            }
            Touch::Up => {
                let i = self.press.iter().position(|(g, _)| *g == f)?;
                let (_, (x0, track)) = self.press.remove(i);
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
        // the view chip's Song Toggle(slots + 1) and List Toggle(slots +
        // LIST_VIEW) (Clips is this view); Rec song Toggle(slots + 2);
        // Edit Toggle(slots + 3); "+ Track" and the pager as above.
        match view_part_at(self.view_rect(r), x, y) {
            Some(ArrangeView::Song) => return Some(EventKind::Toggle(self.slots + 1)),
            Some(ArrangeView::List) => return Some(EventKind::Toggle(self.slots + LIST_VIEW)),
            Some(ArrangeView::Clips) => return None,
            None => {}
        }
        for k in 0..self.banks.len() {
            if self.bank_part_rect(r, k).is_some_and(|b| b.contains(x, y)) {
                return Some(EventKind::Toggle(self.slots + BANK + k));
            }
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
        if self.show_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Toggle(self.slots + SHOW_VIEW));
        }
        if self.add_part_rect(r).is_some_and(|b| b.contains(x, y)) {
            // Faint too: the page says why.
            return Some(EventKind::Toggle(self.slots + ADD_PART));
        }
        // Faint, it still gives its toggle, for SHIFT + tap's Learn; the
        // page acts on it only while it is lit.
        if self.capture_rect(r).is_some_and(|b| b.contains(x, y)) {
            return Some(EventKind::Toggle(self.slots + CAPTURE));
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
        // The row's chords: a chip, or a bar while there are none; the
        // rest of the strip takes the touch and does nothing.
        if let (Some(s), Some((label, bars))) = (&self.strip, self.strip_rects(r)) {
            if bars.contains(x, y) || label.contains(x, y) {
                return match strip_hit(bars, s, x, y) {
                    Some(StripHit::Chip(k)) => Some(EventKind::Toggle(self.slots + STRIP_CHIP + k)),
                    Some(StripHit::Bar(b)) => {
                        Some(EventKind::Toggle(self.slots + STRIP_BAR + b as usize))
                    }
                    None => None,
                };
            }
        }
        for slot in self.first_row..self.slots {
            if self.scene_rect(r, slot).is_some_and(|b| b.contains(x, y)) {
                // Launched on the lift, unless it is dragged.
                self.grip.put(
                    f,
                    Grip {
                        held: Held::Row(slot),
                        from: (x, y),
                        at: (x, y),
                        lifted: false,
                        edge: None,
                    },
                );
                return None;
            }
        }
        let tracks = self.first_track..(self.first_track + GRID_COLS).min(self.tracks.len());
        for track in tracks {
            if self.sound_rect(r, track).is_some_and(|s| s.contains(x, y)) {
                return Some(EventKind::Tab(SOUND_TAP + track));
            }
            if self.head_rect(r, track).is_some_and(|h| h.contains(x, y)) {
                self.press.push((f, (x, track)));
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
                        // On the lift, unless it moves off (5.4).
                        self.empty.push((f, (track, slot, (x, y))));
                        return None;
                    }
                    // A clip launches on the lift, unless it is dragged.
                    self.grip.put(
                        f,
                        Grip {
                            held: Held::Clip(track, slot),
                            from: (x, y),
                            at: (x, y),
                            lifted: false,
                            edge: None,
                        },
                    );
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

/// A cell, with its mark (composing tools S2.6): a part being tried is
/// dashed in the track's colour, a clip off its row's chords in the
/// warning colour.
fn paint_cell(cx: &mut PaintCx, cr: Rect, cell: &GridCell, base: Color) {
    paint_cell_body(cx, cr, cell, base);
    let t = cx.theme.clone();
    match cell.mark {
        CellMark::None => {}
        CellMark::Try => crate::chord_strip::dashed(cx, cr, base),
        CellMark::Unfit => crate::chord_strip::dashed(cx, cr, t.warn),
    }
}

/// What a mark makes of a cell's fill, its name's ink and its detail's.
fn mark_inks(t: &mpfree_ui::Theme, cell: &GridCell, base: Color, fill: Color) -> (Color, Color) {
    match cell.mark {
        CellMark::None => (t.text, t.readable(t.text_dim, fill)),
        CellMark::Try => (
            t.readable(base.mix(t.text, 0.25), fill),
            t.readable(t.text_dim, fill),
        ),
        CellMark::Unfit => (t.text, t.readable(t.warn, fill)),
    }
}

fn paint_cell_body(cx: &mut PaintCx, cr: Rect, cell: &GridCell, base: Color) {
    let t = cx.theme.clone();
    if cell.state == CellState::Empty && cell.mark != CellMark::Try {
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
        _ if cell.mark == CellMark::Try => t.surface.mix(base, 0.14),
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
        _ if cell.mark == CellMark::Try => {}
        CellState::Queued => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.warn),
        CellState::Recording => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.bad),
        _ => {}
    }
    let (name_ink, marked_ink) = mark_inks(&t, cell, base, fill);
    let inner = cr.inset_xy(10, 8);
    let (name, rest) = inner.take_top(20);
    let name = loop_marks(cx, name, cell, base);
    cx.text(
        FontId::Bold,
        t.size_small + 1.0,
        name,
        Align::Left,
        name_ink,
        &cell.name,
    );
    let (detail, _) = rest.take_top(20);
    let detail_ink = match cell.state {
        _ if cell.mark != CellMark::None => marked_ink,
        CellState::Recording => t.readable(t.bad.mix(t.text, 0.2), fill),
        _ if cell.cued => t.readable(t.warn, fill),
        _ => t.readable(t.text_dim, fill),
    };
    let rolling = matches!(cell.state, CellState::Playing | CellState::Recording);
    // A cell with a wave shows its pass on it, not with a ring; a loop
    // cell on an audio track has its ring, stopped or not.
    let ringed = (rolling && cell.wave.is_none()) || cell.looped;
    let ring_room = if ringed { RING + 4 } else { 0 };
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
            let line = if cell.foot.is_empty() {
                &cell.detail
            } else {
                &cell.foot
            };
            cx.text(
                foot_face,
                t.size_small - 1.0,
                foot,
                Align::Left,
                foot_ink,
                line,
            );
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
    if ringed && inner.h >= RING + 20 {
        let color = match cell.state {
            CellState::Recording => t.bad,
            _ if cell.looped => base,
            _ => t.good,
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
        if cell.countdown > 0.0 {
            // A stop or Back to song nears: the countdown left of the
            // ring.
            let track = Rect::new(cr.x + 10, cr.bottom() - 7, cr.w - 24 - RING, 3);
            cx.canvas
                .fill_round_rect(track, 1, t.surface.mix(t.text_faint, 0.35));
            let w = (track.w as f32 * cell.countdown.min(1.0)) as i32;
            cx.canvas
                .fill_round_rect(Rect::new(track.x, track.y, w.max(2), 3), 1, t.warn);
        }
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

/// A loop cell's marks on its name line (layout spec 4f): the loop mark
/// before the name in the track's colour, and while a layer is being
/// written an amber dot at its right. The room left for the name.
fn loop_marks(cx: &mut PaintCx, name: Rect, cell: &GridCell, base: Color) -> Rect {
    if !cell.looped {
        return name;
    }
    let t = cx.theme.clone();
    let mut name = name;
    if cell.writing {
        let (dot, rest) = name.take_right(12);
        cx.canvas.fill_round_rect(
            Rect::new(dot.right() - 8, dot.y + dot.h / 2 - 4, 8, 8),
            4,
            t.warn,
        );
        name = rest;
    }
    let size = 12.min(name.h);
    let mark = Rect::new(name.x, name.y + (name.h - size) / 2, size, size);
    loop_mark(cx.canvas, mark, base.mix(t.text, 0.25));
    name.take_left(size + 4).1
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
        _ if cell.mark == CellMark::Try => {}
        CellState::Queued => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.warn),
        CellState::Recording => cx.canvas.stroke_round_rect(cr, t.round(10), 2, t.bad),
        CellState::Armed => {
            cx.canvas
                .fill_round_rect(Rect::new(cr.x, cr.y + 4, 4, cr.h - 8), 2, t.bad)
        }
        _ => {}
    }
    let rolling = matches!(cell.state, CellState::Playing | CellState::Recording);
    let ringed = (rolling && cell.wave.is_none()) || cell.looped;
    let inner = cr.inset_xy(9, 4);
    let inner = if ringed {
        inner.take_right(SMALL_RING + 4).1
    } else {
        inner
    };
    let (mut name, line) = inner.take_top(inner.h / 2);
    let (mut name_ink, marked_ink) = mark_inks(&t, cell, base, fill);
    if cell.state == CellState::Armed {
        cx.canvas
            .fill_round_rect(Rect::new(name.x, name.y + name.h / 2 - 4, 8, 8), 4, t.bad);
        name = name.take_left(13).1;
        name_ink = t.readable(t.bad, fill);
    }
    let name = loop_marks(cx, name, cell, base);
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
                _ if cell.mark != CellMark::None => (&cell.detail, marked_ink),
                CellState::Queued => (&cell.foot, t.warn),
                CellState::Recording => (&cell.detail, t.bad.mix(t.text, 0.2)),
                CellState::Armed => (&cell.detail, t.text),
                _ if cell.cued => (&cell.detail, t.warn),
                _ => (&cell.detail, t.text_dim),
            };
            let ink = t.readable(ink, fill);
            let face = if said { FontId::Bold } else { FontId::Sans };
            cx.text(face, t.size_small - 2.0, line, Align::Left, ink, words);
        }
    }
    if ringed {
        let color = match cell.state {
            CellState::Recording => t.bad,
            _ if cell.looped => base,
            _ => t.good,
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
/// nothing. The note editor's Continue shows its tries in it the same way
/// ([`TakeList::words`], [`TakeList::empty`]).
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
    /// The three buttons' words, Undo's, Redo's and Capture's places: a
    /// button with no words isn't there.
    pub words: [&'static str; 3],
    /// What it says with no rows: a line, and help under it (None: the
    /// takes' own).
    pub empty: Option<(&'static str, &'static str)>,
}

/// The takes' buttons' words ([`TakeList::words`]).
pub const TAKE_WORDS: [&str; 3] = ["Undo", "Redo", "Capture"];
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
            words: TAKE_WORDS,
            empty: None,
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
            const HELP: &str = "REC records one; Capture keeps what you just played";
            let (line, help) = self.empty.unwrap_or(("No takes yet", HELP));
            cx.text(
                FontId::Sans,
                t.size_body,
                a,
                Align::Left,
                t.text_faint,
                line,
            );
            // One line, or two in a narrow panel (Arrange beside the
            // Q-Link column).
            let lines: Vec<&str> = if cx.measure(FontId::Sans, t.size_small, help) <= body.w {
                vec![help]
            } else if help == HELP {
                vec!["REC records one;", "Capture keeps what you played"]
            } else {
                help.split_inclusive("; ").map(str::trim).collect()
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
            (self.words[0], &self.undo),
            (self.words[1], &self.redo),
            (self.words[2], &self.capture),
        ]
        .into_iter()
        .enumerate()
        {
            if word.is_empty() {
                continue;
            }
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
            .filter(|&i| !self.words[i].is_empty())
            .map(|i| {
                Target::tap("button", i, self.words[i], TakeList::button_rect(r, i))
                    .disabled_if(live[i].is_none())
            })
            .collect()
    }

    fn touch(&mut self, _f: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        if phase != Touch::Down {
            return None;
        }
        (0..3)
            .filter(|&i| !self.words[i].is_empty())
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
/// A dashed outline round `r`: 6 px on, 4 px off, 2 px thick.
pub(crate) fn dashed_outline(cx: &mut PaintCx, r: Rect, c: Color) {
    let mut x = r.x;
    while x < r.right() {
        let w = 6.min(r.right() - x);
        cx.canvas.fill_rect(Rect::new(x, r.y, w, 2), c);
        cx.canvas.fill_rect(Rect::new(x, r.bottom() - 2, w, 2), c);
        x += 10;
    }
    let mut y = r.y;
    while y < r.bottom() {
        let h = 6.min(r.bottom() - y);
        cx.canvas.fill_rect(Rect::new(r.x, y, 2, h), c);
        cx.canvas.fill_rect(Rect::new(r.right() - 2, y, 2, h), c);
        y += 10;
    }
}

pub(crate) fn hatch(cx: &mut PaintCx, r: Rect, c: Color) {
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
                audio: false,
                song: SongMark::None,
                cells,
            });
        }
        g
    }

    fn chords_under(row: usize, chips: usize) -> ChordStrip {
        use crate::chord_strip::{ChipLook, StripChip};
        ChordStrip {
            row,
            bars: 8,
            chips: (0..chips)
                .map(|k| StripChip {
                    from: k as f32 / chips as f32,
                    to: (k + 1) as f32 / chips as f32,
                    name: "Cm7".into(),
                    look: ChipLook::Heard,
                    word: String::new(),
                })
                .collect(),
            play: Some(0.4),
            open: None,
        }
    }

    /// Composing tools S2.6 (layout spec 4r): Add a part on the title
    /// line, left of "+ Track"; with the pager and "Back to song" both
    /// showing it steps aside, and where Capture scene or the bank chip
    /// would lose their room. Faint, it still answers, so the page can
    /// say why.
    #[test]
    fn add_a_part_sits_left_of_plus_track_and_steps_aside() {
        let r = Rect::new(12, 128, 854, 610);
        let mut g = grid(8);
        let part = g.add_part_rect(r).unwrap();
        assert!(part.right() < g.add_rect(r).x);
        g.back_to_song = true;
        let back = g.back_rect(r).unwrap();
        assert!(back.right() < part.x, "{back:?} {part:?}");
        assert_eq!(
            g.touch(1, Touch::Down, part.x + 4, part.y + 4, r),
            Some(EventKind::Toggle(16 + ADD_PART))
        );
        g.add_part = PartButton::Faint;
        assert_eq!(
            g.touch(1, Touch::Down, part.x + 4, part.y + 4, r),
            Some(EventKind::Toggle(16 + ADD_PART))
        );
        assert!(g
            .targets(r)
            .iter()
            .any(|t| t.id == "add-part" && !t.enabled));
        // Pager and Back to song: no room.
        let mut g = grid(24);
        assert!(g.add_part_rect(r).is_some());
        g.back_to_song = true;
        assert!(g.add_part_rect(r).is_none());
        assert!(g.targets(r).iter().all(|t| t.id != "add-part"));
        let (_, on) = g.pager_rects(r).unwrap();
        assert!(on.right() < g.add_rect(r).x);
        // Arrange's buttons keep their places: Perform's Capture scene
        // with 8 tracks fits beside it; with the bank chip too, or with
        // the pager, Add a part steps aside.
        let r = Rect::new(12, 128, 814, 610);
        let mut g = ClipGrid {
            big: true,
            ..grid(8)
        };
        let part = g.add_part_rect(r).unwrap();
        assert!(g.capture_rect(r).unwrap().right() + 8 <= part.x);
        g.banks = vec![true, true, false, false];
        assert!(g.add_part_rect(r).is_none());
        assert!(g.capture_rect(r).is_some() && g.bank_chip_rect(r).is_some());
        let g = ClipGrid {
            big: true,
            ..grid(24)
        };
        assert!(g.add_part_rect(r).is_none());
        // Make with the pager and four banks: the bank chip keeps its room.
        let g = ClipGrid {
            slots: 64,
            banks: vec![true, true, false, false],
            ..grid(24)
        };
        assert!(g.add_part_rect(r).is_none());
        assert!(g.bank_chip_rect(r).is_some());
    }

    /// The row's chords take one row's place under their row: the rows
    /// under it move down one, a chip's tap is its own, a drop over the
    /// strip lands nowhere and a dragged row's gap skips it. Perform's big
    /// rows give it Make's height.
    #[test]
    fn the_chords_strip_takes_a_rows_place_under_its_row() {
        let r = Rect::new(12, 128, 814, 610);
        let mut g = grid(10);
        let rows = g.visible_rows(r);
        let three = g.cell_rect(r, 0, 3).unwrap();
        g.strip = Some(chords_under(2, 4));
        g.outlined = Some(2);
        assert_eq!(g.visible_rows(r), rows - 1);
        let (label, bars) = g.strip_rects(r).unwrap();
        let two = g.cell_rect(r, 0, 2).unwrap();
        assert_eq!(two.y, g.cell_rect(r, 0, 2).unwrap().y);
        assert!(
            label.y > two.bottom() && label.h == two.h,
            "{label:?} {two:?}"
        );
        assert_eq!(bars.x, two.x);
        let moved = g.cell_rect(r, 0, 3).unwrap();
        assert_eq!(moved.y, three.y + two.h + CELL_GAP, "down one");
        assert!(moved.y > bars.bottom());
        // A chip's tap; the label's does nothing; a bar with no chords
        // sets one.
        let chips = g.chip_rects(r);
        assert_eq!(chips.len(), 4);
        let c = chips[2];
        assert_eq!(
            g.touch(1, Touch::Down, c.x + c.w / 2, c.y + c.h / 2, r),
            Some(EventKind::Toggle(16 + STRIP_CHIP + 2))
        );
        assert_eq!(g.touch(1, Touch::Down, label.x + 4, label.y + 4, r), None);
        g.strip = Some(chords_under(2, 0));
        assert_eq!(
            g.touch(1, Touch::Down, bars.x + bars.w - 4, bars.y + 20, r),
            Some(EventKind::Toggle(16 + STRIP_BAR + 7))
        );
        g.strip = Some(chords_under(2, 4));
        // A clip dragged over the strip lands nowhere; a row dragged
        // there goes in the gap above it (under row 3).
        let beat = g.cell_rect(r, 1, 0).unwrap();
        g.touch(1, Touch::Down, beat.x + 4, beat.y + 4, r);
        g.touch(1, Touch::Move, bars.x + 40, bars.y + bars.h / 2, r);
        assert!(g.dragging());
        let (theme, mut fonts) = (Theme::dark(), Fonts::embedded());
        let mut canvas = SoftCanvas::new(1280, 800);
        g.paint(
            &mut PaintCx {
                canvas: &mut canvas,
                theme: &theme,
                fonts: &mut fonts,
            },
            r,
        );
        assert_eq!(
            g.touch(1, Touch::Up, bars.x + 40, bars.y + bars.h / 2, r),
            None
        );
        let five = g.scene_rect(r, 5).unwrap();
        g.touch(1, Touch::Down, five.x + 4, five.y + 4, r);
        g.touch(1, Touch::Move, five.x + 4, bars.y + bars.h / 2, r);
        assert_eq!(
            g.touch(1, Touch::Up, five.x + 4, bars.y + bars.h / 2, r),
            Some(EventKind::Drop {
                from: ROW_DROP + 5,
                to: 3
            })
        );
        // Scrolled past its row: no strip, all the rows.
        g.first_row = 3;
        assert!(g.strip_rects(r).is_none());
        assert_eq!(g.visible_rows(r), rows);
        // Perform: Make's height.
        g.first_row = 0;
        g.big = true;
        let (label, _) = g.strip_rects(r).unwrap();
        assert_eq!(label.h, ROW_MIN_H_MAKE);
        assert!(g.cell_rect(r, 0, 2).unwrap().h >= ROW_MIN_H);
    }

    /// The grid's controls say how they act (layout spec 1a): in Make a
    /// clip and a row's number are Dense rows that drag; in Perform they
    /// are played, need Performance size and never move; Stop all is
    /// Critical, on the press. Each names what it acts on.
    #[test]
    fn the_grid_says_how_its_controls_act() {
        use mpfree_ui::{Class, Commit, Context, Movement};
        let r = Rect::new(12, 128, 814, 610);
        let mut g = grid(4);
        let slot = |g: &ClipGrid| {
            g.targets(r)
                .into_iter()
                .find(|t| t.id == "slot" && t.index == g.slots + 1)
                .unwrap()
        };
        let make = slot(&g);
        assert_eq!(
            (make.class, make.needed_class()),
            (Class::Dense, Class::Dense)
        );
        assert_eq!(make.movement, Movement::Structural);
        assert_eq!(make.commit, Commit::LiftInside);
        assert_eq!(make.object.as_deref(), Some("track 2 row 2"));
        g.big = true;
        let perform = slot(&g);
        assert_eq!(perform.context, Context::Perform);
        assert_eq!(perform.needed_class(), Class::Performance);
        assert_eq!(perform.movement, Movement::None);
        let stop = g
            .targets(r)
            .into_iter()
            .find(|t| t.id == "stop-all")
            .unwrap();
        assert_eq!(
            (stop.needed_class(), stop.commit),
            (Class::Critical, Commit::Press)
        );
    }

    /// The chord picker under a chip: every touch is its own while it is
    /// open, and a touch off it closes it.
    #[test]
    fn the_chord_picker_takes_every_touch_while_open() {
        use crate::chord_strip::{PICK_KIND, PICK_ROOT};
        let r = Rect::new(12, 128, 814, 610);
        let mut g = grid(10);
        g.strip = Some(chords_under(1, 4));
        g.picker = Some(PickerView {
            chip: 1,
            bar: 3,
            heard: "G7".into(),
            ..PickerView::default()
        });
        let pg = g.picker_geo(r).unwrap();
        let chip = g.chip_rects(r)[1];
        assert!(pg.panel.y > chip.bottom() && r.intersect(pg.panel) == pg.panel);
        let mid = |b: Rect| (b.x + b.w / 2, b.y + b.h / 2);
        let (x, y) = mid(pg.roots[8]);
        assert_eq!(
            g.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(16 + PICKER + PICK_ROOT + 8))
        );
        let (x, y) = mid(pg.kinds[4]);
        assert_eq!(
            g.touch(1, Touch::Down, x, y, r),
            Some(EventKind::Toggle(16 + PICKER + PICK_KIND + 4))
        );
        assert_eq!(g.touch(1, Touch::Up, x, y, r), None);
        let corner = g.scene_rect(r, 0).unwrap();
        assert_eq!(
            g.touch(1, Touch::Down, corner.x + 2, corner.y + 2, r),
            Some(EventKind::Toggle(16 + PICKER + PICK_DISMISS))
        );
        assert!(g.targets(r).iter().any(|t| t.id == "chord-root"));
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
        // An empty cell acts on the lift (the Arrange refactor's 5.4).
        assert_eq!(g.touch(1, Touch::Down, c.x + 2, c.y + 2, r), None);
        assert_eq!(
            g.touch(1, Touch::Up, c.x + 2, c.y + 2, r),
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
        let part = g.add_part_rect(r).unwrap();
        assert!(back.right() < on.x && on.right() < part.x && part.right() < add.x);
        assert!(
            back.x > r.x + 100,
            "clear of the title's first word: {back:?}"
        );
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

    /// Scene banks (the Arrange refactor's 5.6): with 64 rows the bank
    /// chip sits right of the view chip, clear of the pager and "Back to
    /// song", lights the bank in view and taps for a bank's rows; the
    /// title's words start after it. Without banks there is no chip.
    #[test]
    fn the_bank_chip_lights_the_bank_in_view_and_taps_for_another() {
        let r = Rect::new(12, 128, 814, 610);
        let mut g = grid(8);
        assert_eq!(g.bank_chip_rect(r), None);
        assert!(g.targets(r).iter().all(|t| t.id != "bank"));
        assert_eq!(g.title_x(r), view_title_x(r));
        g = ClipGrid {
            slots: 64,
            banks: vec![true, true, false, false],
            ..grid(24)
        };
        for t in &mut g.tracks {
            t.cells.resize(64, GridCell::default());
        }
        // Clear of the pager, and of "Back to song" with 8 tracks; with
        // both, no room.
        let chip = g.bank_chip_rect(r).unwrap();
        assert!(chip.x > g.view_rect(r).right(), "{chip:?}");
        let (back, _) = g.pager_rects(r).unwrap();
        assert!(chip.right() + 12 <= back.x, "{chip:?} {back:?}");
        assert_eq!(g.title_x(r), chip.right() + 12);
        g.back_to_song = true;
        assert_eq!(g.bank_chip_rect(r), None);
        g.tracks.truncate(8);
        let b = g.back_rect(r).unwrap();
        assert!(g.bank_chip_rect(r).unwrap().right() + 12 <= b.x);
        let c = g.bank_part_rect(r, 2).unwrap();
        assert_eq!(
            g.touch(1, Touch::Down, c.x + 4, c.y + 4, r),
            Some(EventKind::Toggle(64 + BANK + 2))
        );
        let banks: Vec<String> = g
            .targets(r)
            .into_iter()
            .filter(|t| t.id == "bank")
            .map(|t| t.label)
            .collect();
        assert_eq!(banks, ["Bank A", "Bank B", "Bank C", "Bank D"]);
        // The bank shown is faint as a target: a tap there changes nothing.
        let lit: Vec<usize> = g
            .targets(r)
            .into_iter()
            .filter(|t| t.id == "bank" && !t.enabled)
            .map(|t| t.index)
            .collect();
        assert_eq!(lit, [g.bank_shown(g.geo(r).rows)]);
        // The bank of the middle row in view; of an even split, the lower.
        assert_eq!(g.bank_shown(8), 0);
        g.first_row = 12;
        assert_eq!(g.bank_shown(8), 1, "rows 13 to 20");
        g.first_row = 56;
        assert_eq!(g.bank_shown(8), 3);
        // Row 17's number is a row like any other: it launches its scene.
        g.first_row = 14;
        let b = g.scene_rect(r, 16).unwrap();
        g.touch(1, Touch::Down, b.x + 4, b.y + 4, r);
        assert_eq!(
            g.touch(1, Touch::Up, b.x + 4, b.y + 4, r),
            Some(EventKind::Toggle(16))
        );
        // A cell of row 18 is track * 64 + 17.
        let cell = g.cell_rect(r, 5, 17).unwrap();
        g.touch(1, Touch::Down, cell.x + 4, cell.y + 4, r);
        assert_eq!(
            g.touch(1, Touch::Up, cell.x + 4, cell.y + 4, r),
            Some(EventKind::Select(5 * 64 + 17))
        );
    }

    /// Capture scene (the Arrange refactor's 5.3) is Perform's: left of
    /// "Back to song", the pager or "+ Track", clear of the view chip,
    /// faint while there is nothing to keep but still giving its toggle
    /// (SHIFT + tap learns it). Beside both the pager and "Back to song"
    /// it has no room and isn't there; it moves the bank chip out where
    /// the chip no longer fits.
    #[test]
    fn capture_scene_is_on_performs_title_line_where_it_fits() {
        // Perform's grid, beside the Record panel.
        let r = Rect::new(12, 140, 854, 534);
        let mut g = grid(8);
        assert_eq!(g.capture_rect(r), None, "Make has none");
        g.big = true;
        let c = g.capture_rect(r).unwrap();
        assert!(c.right() + 8 <= g.add_rect(r).x, "{c:?}");
        assert!(c.x > g.view_rect(r).right(), "{c:?}");
        let target = g
            .targets(r)
            .into_iter()
            .find(|t| t.id == "capture-scene")
            .unwrap();
        assert!(!target.enabled, "faint: nothing plays");
        assert_eq!(
            g.touch(1, Touch::Down, c.x + 4, c.y + 4, r),
            Some(EventKind::Toggle(16 + CAPTURE)),
            "faint, it still gives its toggle for Learn"
        );
        g.can_capture = true;
        assert!(g
            .targets(r)
            .iter()
            .any(|t| t.id == "capture-scene" && t.enabled));
        // "Back to song" shows: Capture scene moves left of it.
        g.back_to_song = true;
        let b = g.back_rect(r).unwrap();
        let c = g.capture_rect(r).unwrap();
        assert!(c.right() + 8 <= b.x, "{c:?} {b:?}");
        assert_eq!(
            g.touch(1, Touch::Down, b.x + 4, b.y + 4, r),
            Some(EventKind::Toggle(16 + BACK_TO_SONG))
        );
        // With the pager and "Back to song", no room.
        let mut g = ClipGrid {
            big: true,
            back_to_song: true,
            ..grid(24)
        };
        assert_eq!(g.capture_rect(r), None);
        assert!(g.targets(r).iter().all(|t| t.id != "capture-scene"));
        g.back_to_song = false;
        let (pager, _) = g.pager_rects(r).unwrap();
        assert!(g.capture_rect(r).unwrap().right() + 8 <= pager.x);
        // Four banks: the chip fits beside Capture scene alone, not beside
        // it and "Back to song" too.
        let mut g = ClipGrid {
            slots: 64,
            banks: vec![true, true, false, false],
            big: true,
            ..grid(8)
        };
        let chip = g.bank_chip_rect(r).unwrap();
        assert!(chip.right() + 12 <= g.capture_rect(r).unwrap().x);
        g.back_to_song = true;
        assert_eq!(g.bank_chip_rect(r), None);
        // A set open: Show left of Capture scene, both clear of the view
        // chip; beside the pager, Capture scene gives Show its room.
        for (tracks, back) in [(8, false), (8, true), (24, false), (24, true)] {
            let mut g = ClipGrid {
                big: true,
                show_button: true,
                back_to_song: back,
                ..grid(tracks)
            };
            let s = g.show_rect(r).unwrap();
            assert!(s.x > g.view_rect(r).right(), "{tracks} {back}: {s:?}");
            match g.capture_rect(r) {
                Some(c) => assert!(s.right() + 8 <= c.x, "{s:?} {c:?}"),
                None => assert!(tracks > GRID_COLS, "{tracks} {back}"),
            }
            g.show_button = false;
            assert!(g.capture_rect(r).is_some() || (tracks > GRID_COLS && back));
        }
    }

    /// While a set is open, Show sits left of the title line's other
    /// buttons, whatever else shows, and taps.
    #[test]
    fn show_sits_left_of_the_other_buttons_while_a_set_is_open() {
        let r = Rect::new(12, 128, 854, 610);
        for (tracks, back) in [(8, false), (8, true), (24, false), (24, true)] {
            let mut g = grid(tracks);
            g.back_to_song = back;
            assert_eq!(g.show_rect(r), None);
            assert!(g.targets(r).iter().all(|t| t.id != "show-view"));
            g.show_button = true;
            let s = g.show_rect(r).unwrap();
            let right = g
                .back_rect(r)
                .or(g.pager_rects(r).map(|(b, _)| b))
                .unwrap_or(g.add_rect(r));
            assert!(s.right() < right.x, "{s:?} {right:?}");
            assert_eq!(s.w, SHOW_W);
            assert!(
                s.x > g.view_rect(r).right(),
                "clear of the view chip: {s:?}"
            );
            // "Back to song" and the pager's label narrow only where all
            // three would not fit, and their words still do.
            let tight = back && tracks > GRID_COLS;
            if let Some(b) = g.back_rect(r) {
                assert_eq!(b.w, if tight { BACK_TIGHT_W } else { BACK_W });
                assert!(b.w >= 87 + 12, "Back to song's words fit: {b:?}");
            }
            if let Some((pb, po)) = g.pager_rects(r) {
                let label = po.x - pb.right();
                assert_eq!(
                    label,
                    if tight {
                        PAGER_LABEL_TIGHT_W
                    } else {
                        PAGER_LABEL_W
                    }
                );
                assert!(label - 8 >= 77, "17–24 of 24 fits: {label}");
            }
            g.show_button = false;
            assert_eq!(g.back_rect(r).map(|b| b.w), back.then_some(BACK_W));
            g.show_button = true;
            assert!(g.targets(r).iter().any(|t| t.id == "show-view"));
            assert_eq!(
                g.touch(1, Touch::Down, s.x + 3, s.y + 3, r),
                Some(EventKind::Toggle(16 + SHOW_VIEW))
            );
        }
    }

    /// The title line's and the chooser's taps each have their own id
    /// after the slots (Song 1, Rec song 2, Edit 3, then the constants).
    #[test]
    fn the_grid_s_ids_after_the_slots_are_all_different() {
        let mut ids = vec![1, 2, 3, ADD_TRACK, PAGE_BACK, PAGE_ON, ROWS_BACK, ROWS_ON];
        ids.extend([
            BACK_TO_SONG,
            ADD_DISMISS,
            LIST_VIEW,
            SHOW_VIEW,
            VEIL_TAP,
            CAPTURE,
            ADD_PART,
        ]);
        ids.extend((0..BANK_LETTERS.len()).map(|k| BANK + k));
        // The row's chords (a chip or a bar each) and the chord picker.
        ids.extend((0..STRIP_BAR - STRIP_CHIP).map(|k| STRIP_CHIP + k));
        ids.extend((0..PICKER - STRIP_BAR).map(|b| STRIP_BAR + b));
        ids.extend((0..PICKS).map(|k| PICKER + k));
        ids.extend((0..crate::song::ADD_KINDS.len()).map(|k| ADD_KIND + k));
        let mut seen = ids.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len(), "{ids:?}");
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

    /// Each finger is its own (the touch usability review, 25 Nov 2026).
    /// Before, a second finger took the first one's grip: lifting the
    /// first launched the second's clip, the first never played, and a
    /// 3 px wobble of the first finger moved the second's clip onto the
    /// first's cell. Now two clips held at once each launch on their own
    /// lift, a wobble is still a tap, and a drag with one finger moves
    /// only its own clip.
    #[test]
    fn two_fingers_on_two_clips_launch_both_and_move_neither() {
        let r = Rect::new(12, 128, 814, 610);
        let mid = |c: Rect| (c.x + c.w / 2, c.y + c.h / 2);
        let mut g = grid(10);
        let a = mid(g.cell_rect(r, 2, 0).unwrap());
        let b = mid(g.cell_rect(r, 4, 0).unwrap());
        assert_eq!(g.touch(1, Touch::Down, a.0, a.1, r), None);
        assert_eq!(g.touch(2, Touch::Down, b.0, b.1, r), None);
        assert_eq!(g.touch(1, Touch::Move, a.0 + 3, a.1 + 3, r), None);
        assert!(!g.dragging(), "a wobble lifts nothing");
        assert_eq!(
            g.touch(1, Touch::Up, a.0 + 3, a.1 + 3, r),
            Some(EventKind::Select(2 * 16)),
            "the first finger launches its own clip"
        );
        assert_eq!(
            g.touch(2, Touch::Up, b.0, b.1, r),
            Some(EventKind::Select(4 * 16)),
            "and the second its own"
        );
        // Lifted in the other order, the same.
        g.touch(1, Touch::Down, a.0, a.1, r);
        g.touch(2, Touch::Down, b.0, b.1, r);
        assert_eq!(
            g.touch(2, Touch::Up, b.0, b.1, r),
            Some(EventKind::Select(4 * 16))
        );
        assert_eq!(
            g.touch(1, Touch::Up, a.0, a.1, r),
            Some(EventKind::Select(2 * 16))
        );
        // One finger drags its clip to an empty cell while the other
        // rests on another clip: only the dragged one moves, and the
        // resting finger's lift still launches its clip.
        let there = mid(g.cell_rect(r, 6, 3).unwrap());
        g.touch(1, Touch::Down, a.0, a.1, r);
        g.touch(2, Touch::Down, b.0, b.1, r);
        g.touch(1, Touch::Move, a.0 + 30, a.1 + 40, r);
        g.touch(1, Touch::Move, there.0, there.1, r);
        assert!(g.dragging());
        assert_eq!(
            g.touch(1, Touch::Up, there.0, there.1, r),
            Some(EventKind::Drop {
                from: 2 * 16,
                to: 6 * 16 + 3
            })
        );
        assert!(!g.dragging());
        assert_eq!(
            g.touch(2, Touch::Up, b.0, b.1, r),
            Some(EventKind::Select(4 * 16))
        );
        // A finger the grid never saw go down does nothing.
        assert_eq!(g.touch(5, Touch::Up, b.0, b.1, r), None);
    }

    /// Perform launches and never moves (the touch usability review, 25
    /// Nov 2026): a finger on a clip or a row's number doesn't lift
    /// however far it moves, its lift launches only while it is still
    /// on what it held, and slid off it does nothing.
    #[test]
    fn in_perform_nothing_drags_and_a_finger_slid_off_launches_nothing() {
        let r = Rect::new(12, 128, 814, 610);
        let mid = |c: Rect| (c.x + c.w / 2, c.y + c.h / 2);
        let mut g = grid(10);
        g.big = true;
        let beat = mid(g.cell_rect(r, 2, 0).unwrap());
        let there = mid(g.cell_rect(r, 4, 3).unwrap());
        g.touch(1, Touch::Down, beat.0, beat.1, r);
        g.touch(1, Touch::Move, beat.0 + 30, beat.1 + 40, r);
        g.touch(1, Touch::Move, there.0, there.1, r);
        assert!(!g.dragging(), "nothing lifts in Perform");
        assert_eq!(g.touch(1, Touch::Up, there.0, there.1, r), None);
        // Moved within its cell, it is a tap.
        g.touch(1, Touch::Down, beat.0, beat.1, r);
        g.touch(1, Touch::Move, beat.0 + 20, beat.1 + 10, r);
        assert_eq!(
            g.touch(1, Touch::Up, beat.0 + 20, beat.1 + 10, r),
            Some(EventKind::Select(2 * 16))
        );
        // A row's number: launched on the lift on it, never dragged.
        let four = mid(g.scene_rect(r, 3).unwrap());
        let one = mid(g.scene_rect(r, 0).unwrap());
        g.touch(1, Touch::Down, four.0, four.1, r);
        assert_eq!(g.held_row(), Some(3));
        g.touch(1, Touch::Move, one.0, one.1, r);
        assert!(!g.dragging());
        assert_eq!(g.touch(1, Touch::Up, one.0, one.1, r), None);
        g.touch(1, Touch::Down, four.0, four.1, r);
        assert_eq!(
            g.touch(1, Touch::Up, four.0, four.1, r),
            Some(EventKind::Toggle(3))
        );
        // Two clips at once launch both, as in Make.
        let b = mid(g.cell_rect(r, 4, 0).unwrap());
        g.touch(1, Touch::Down, beat.0, beat.1, r);
        g.touch(2, Touch::Down, b.0, b.1, r);
        assert_eq!(
            g.touch(1, Touch::Up, beat.0, beat.1, r),
            Some(EventKind::Select(2 * 16))
        );
        assert_eq!(
            g.touch(2, Touch::Up, b.0, b.1, r),
            Some(EventKind::Select(4 * 16))
        );
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
        // An empty slot is not dragged: a tap makes a clip there, on the
        // lift (the Arrange refactor's 5.4), and a finger that slides off
        // as a drag or a scroll makes nothing.
        assert_eq!(g.touch(1, Touch::Down, there.0, there.1, r), None);
        assert_eq!(
            g.touch(1, Touch::Up, there.0 + 4, there.1 + 4, r),
            Some(EventKind::Select(4 * 16 + 3))
        );
        assert_eq!(g.touch(1, Touch::Down, there.0, there.1, r), None);
        assert_eq!(g.touch(1, Touch::Move, there.0, there.1 + 60, r), None);
        assert_eq!(g.touch(1, Touch::Up, there.0, there.1, r), None);
        assert_eq!(g.touch(1, Touch::Down, there.0, there.1, r), None);
        assert_eq!(g.touch(1, Touch::Cancel, there.0, there.1, r), None);
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
        // The view chip at the title line's left: Song and List switch,
        // Clips is this view (5.7).
        let chip = g.view_rect(r);
        assert!(chip.x >= r.x + 14 && chip.y >= r.y, "{chip:?}");
        let song = view_part_rect(chip, ArrangeView::Song);
        assert_eq!(
            g.touch(1, Touch::Down, song.x + 4, song.y + 4, r),
            Some(EventKind::Toggle(17))
        );
        let list = view_part_rect(chip, ArrangeView::List);
        assert_eq!(
            g.touch(1, Touch::Down, list.x + 4, list.y + 4, r),
            Some(EventKind::Toggle(16 + LIST_VIEW))
        );
        let clips = view_part_rect(chip, ArrangeView::Clips);
        assert_eq!(g.touch(1, Touch::Down, clips.x + 4, clips.y + 4, r), None);
        let rec = g.rec_rect(r);
        assert!(rec.right() <= r.right() - 14, "{rec:?}");
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
