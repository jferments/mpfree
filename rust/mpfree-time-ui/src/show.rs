//! The Show view (live shows B1, layout spec 4p): Arrange's fourth view,
//! shown while a set is open. It is drawn to the approved mockup
//! (`live-shows/show-screens.html`, board 1, approved 22 Nov 2026).
//!
//! At the left, the set column: the set's songs in order, NOW filled green,
//! the armed song (NEXT) outlined amber with its change's state, the songs
//! already played faint, and a dot for a song whose last check found
//! limits (amber) or a gap (red). A tap on a song arms it on the lift (a
//! drag up or down scrolls a long set instead); a tap on NOW or NEXT does
//! nothing, so a double tap never goes. **Back to the set order** is at the
//! column's foot.
//!
//! At the right: the title line with Clips, Song and List; the song band
//! (the song's name and its state as a pill); the parts (one tile for each
//! step of the song's song list, for showing only); the next-song card
//! with **Go** and **Cancel next**; the song's notes from the set; the
//! keyboards the rig knows; and the Click switch.
//!
//! The view only draws what it is given and says what a tap asked for: the
//! set player's rules (`mpfree-app`'s `show` module) decide what a press
//! does. The words of the next-song card and the pill are made here, by
//! [`NextCard`]'s and [`Pill`]'s constructors, so every state reads the
//! same wherever it comes from.

use std::cell::Cell;

use mpfree_ui::{Align, EventKind, FontId, PaintCx, Rect, Target, Theme, Tone, Touch, Widget};

/// `Toggle(k)` from the view's buttons. `Select(k)` arms song `k`.
pub const SHOW_CLIPS: usize = 0;
pub const SHOW_SONG: usize = 1;
pub const SHOW_LIST: usize = 2;
pub const SHOW_BACK: usize = 3;
pub const SHOW_GO: usize = 4;
pub const SHOW_CANCEL: usize = 5;
pub const SHOW_CLICK: usize = 6;
/// The card's buttons after an open failed: Try again, and Reopen the
/// song before.
pub const SHOW_RETRY: usize = 7;
pub const SHOW_REOPEN: usize = 8;

/// The set column's width, and the gap between it and the main panel.
const COL_W: i32 = 280;
const COL_GAP: i32 = 12;
/// A song's row in the set column: its height, and how far apart rows are.
const ROW_H: i32 = 48;
const ROW_PITCH: i32 = 54;
/// The line under the rows that says how many more there are.
const MORE_H: i32 = 28;
const BACK_H: i32 = 46;
/// The title line's buttons: Clips, Song and List.
const LENS_W: i32 = 72;
const LENS_H: i32 = 32;
const PILL_W: i32 = 160;
const PILL_H: i32 = 44;
/// A part's tile.
const TILE_W: i32 = 108;
const TILE_H: i32 = 76;
const TILE_GAP: i32 = 6;
/// The next-song card's width, and its buttons.
const CARD_W: i32 = 404;
const BUTTON_W: i32 = 180;
const BUTTON_H: i32 = 56;
/// The keyboards' and the click's cards.
const FOOT_H: i32 = 96;
const KEY_LINE: i32 = 28;
const SWITCH_W: i32 = 120;
const SWITCH_H: i32 = 44;
/// Text sizes the mockup draws at: the song's name, the card's song, the
/// card's state word, the notes.
const NAME_PX: f32 = 40.0;
const CARD_NAME_PX: f32 = 28.0;
const WORD_PX: f32 = 24.0;
const NOTES_PX: f32 = 20.0;
const NOTES_LINE: i32 = 29;
/// How far a finger moves before it scrolls the set instead of arming.
const DRAG_PX: i32 = 12;

/// What a song in the set column is now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SongMark {
    /// Still to come, and nothing to say about its check.
    #[default]
    Waiting,
    /// Played already (sunk, its words dim).
    Played,
    /// The song playing, or loaded and waiting for PLAY.
    Now,
    /// Armed to go next, with its change's state ("cold load",
    /// "blocked").
    Next(String),
    /// Its last check found limits (an amber dot).
    Limits,
    /// Its last check found a gap (a red dot).
    NotReady,
}

/// A song in the set column.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetRow {
    pub name: String,
    pub mark: SongMark,
}

/// Where a part stands.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum PartMark {
    #[default]
    Waiting,
    Played,
    /// Playing: "pass 3 of 4", and how far through the step (0 to 1).
    Now {
        pass: String,
        done: f32,
    },
    /// The step after it.
    Next,
}

/// One step of the song's song list, as a tile.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PartTile {
    /// Its scene's name.
    pub name: String,
    /// "× 4 · 16 bars".
    pub meta: String,
    /// Its scene's row, for its colour (None: a linked list's section
    /// that is the song's own, which has no scene colour).
    pub scene: Option<usize>,
    pub mark: PartMark,
}

/// The song's state, as the pill at the right of its name says it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pill {
    pub word: String,
    pub tone: Tone,
    /// A line under the pill ("press PLAY", or the count-in), or empty.
    pub under: String,
}

impl Default for Pill {
    fn default() -> Pill {
        Pill::playing()
    }
}

impl Pill {
    pub fn playing() -> Pill {
        Pill::new("PLAYING", Tone::Good, "")
    }

    /// Open and stopped: `under` is "press PLAY", or the count-in while it
    /// counts ("count-in 3").
    pub fn loaded(under: &str) -> Pill {
        Pill::new("LOADED", Tone::Accent, under)
    }

    /// The song is ending, at most `secs` more.
    pub fn finishing(secs: u32) -> Pill {
        Pill::new(&format!("FINISHING · {secs} s"), Tone::Warn, "")
    }

    /// The song fades out over its Fade, `secs` in all.
    pub fn fading(secs: u32) -> Pill {
        Pill::new(&format!("FADING · {secs} s"), Tone::Warn, "")
    }

    pub fn saving_takes() -> Pill {
        Pill::new("SAVING TAKES", Tone::Accent2, "")
    }

    pub fn opening() -> Pill {
        Pill::new("OPENING", Tone::Accent, "")
    }

    /// An open failed after the song before had gone.
    pub fn nothing_playing() -> Pill {
        Pill::new("NOTHING PLAYING", Tone::Bad, "")
    }

    fn new(word: &str, tone: Tone, under: &str) -> Pill {
        Pill {
            word: word.to_string(),
            tone,
            under: under.to_string(),
        }
    }
}

/// How the song playing ends when the next one takes over: its own Ends
/// in the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ending {
    /// Let it finish: no new notes, and what still rings rings out.
    Finish,
    /// A fade over this many seconds.
    Fade(u8),
    Cut,
}

/// How the next song starts: its Starts in the set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Starting {
    Stopped,
    Playing,
    CountIn,
}

/// The next-song card: see the constructors for its states.
#[derive(Clone, Debug, PartialEq)]
pub struct NextCard {
    /// "Next song", "Next in the set", "Last song", "Didn't open" (drawn
    /// in capitals).
    pub label: String,
    pub label_tone: Tone,
    /// "5 of 12", or empty.
    pub of: String,
    pub name: String,
    pub name_faint: bool,
    pub border: Option<Tone>,
    /// The state as a large word.
    pub word: Option<(String, Tone)>,
    /// How far it has got (0 to 1).
    pub progress: Option<(f32, Tone)>,
    /// What the state means.
    pub meaning: String,
    /// What Go does, or empty.
    pub go_line: String,
    /// The two buttons' words and whether each can be pressed.
    pub go: (String, bool),
    pub cancel: (String, bool),
    /// After a failed open: the buttons are Try again and Reopen.
    pub failed: bool,
}

impl Default for NextCard {
    fn default() -> NextCard {
        NextCard::not_armed("", "", None)
    }
}

impl NextCard {
    /// No song armed: the next song in the set's order, faint. `limits`
    /// is the song's check line when it plays with limits.
    pub fn not_armed(of: &str, name: &str, limits: Option<&str>) -> NextCard {
        let mut meaning = "Press Next song to arm it.".to_string();
        if let Some(l) = limits {
            meaning.push_str(&format!(" Ready with limits: {l}."));
        }
        NextCard {
            label: "Next in the set".into(),
            label_tone: Tone::Dim,
            of: of.into(),
            name: name.into(),
            name_faint: true,
            border: None,
            word: None,
            progress: None,
            meaning,
            go_line: String::new(),
            go: ("Go".into(), false),
            cancel: ("Cancel next".into(), false),
            failed: false,
        }
    }

    /// Armed, for a cold change: `pause` is the silence the last
    /// soundcheck measured, in seconds. `now` is the song playing, or
    /// `None` while nothing plays.
    pub fn cold(
        of: &str,
        name: &str,
        now: Option<&str>,
        pause: Option<u32>,
        ending: Ending,
        starting: Starting,
    ) -> NextCard {
        let meaning = match pause {
            Some(s) => format!("about {s} s of silence while it opens"),
            None => "pause not measured yet".to_string(),
        };
        let starts = match starting {
            Starting::Stopped => "opens and waits for PLAY",
            Starting::Playing => "opens and plays",
            Starting::CountIn => "opens, counts in and plays",
        };
        let go_line = match now {
            Some(now) => {
                let ends = match ending {
                    Ending::Finish => "lets its sound finish".to_string(),
                    Ending::Fade(s) => format!("fades out over {s} s"),
                    Ending::Cut => "stops".to_string(),
                };
                format!("Go: {now} {ends}, then {name} {starts}.")
            }
            None => format!("Go: {name} {starts}."),
        };
        NextCard {
            label: "Next song".into(),
            label_tone: Tone::Dim,
            border: Some(Tone::Warn),
            word: Some(("COLD LOAD".into(), Tone::Warn)),
            name_faint: false,
            meaning,
            go_line,
            go: ("Go".into(), true),
            cancel: ("Cancel next".into(), true),
            ..NextCard::not_armed(of, name, None)
        }
    }

    /// The song playing is ringing out (Let it finish): Go again goes now.
    pub fn finishing(of: &str, name: &str, now: &str, secs: u32) -> NextCard {
        NextCard {
            label: "Next song".into(),
            border: Some(Tone::Warn),
            word: Some(("FINISHING".into(), Tone::Warn)),
            name_faint: false,
            meaning: format!("{now} is ringing out: {secs} s at most"),
            go_line: "Go again: go now.".into(),
            go: ("Go now".into(), true),
            ..NextCard::not_armed(of, name, None)
        }
    }

    /// The song playing fades out over its Fade seconds (Ends: Fade), then
    /// the song opens. Presses wait.
    pub fn fading(of: &str, name: &str, now: &str, secs: u32) -> NextCard {
        NextCard {
            label: "Next song".into(),
            border: Some(Tone::Warn),
            word: Some(("FADING".into(), Tone::Warn)),
            name_faint: false,
            meaning: format!("{now} fades out over {secs} s, then {name} opens"),
            ..NextCard::not_armed(of, name, None)
        }
    }

    /// The song before's takes are being written, then the song opens.
    /// `left` says what is left ("2 takes, 3.1 MB left").
    pub fn saving_takes(of: &str, name: &str, done: f32, left: &str) -> NextCard {
        NextCard {
            label: "Next song".into(),
            word: Some(("SAVING TAKES".into(), Tone::Accent2)),
            progress: Some((done, Tone::Accent2)),
            name_faint: false,
            meaning: format!("then opening: {left}"),
            ..NextCard::not_armed(of, name, None)
        }
    }

    /// The song is opening: `line` says how far ("3 of 8 parts read").
    pub fn opening(of: &str, name: &str, done: f32, line: &str) -> NextCard {
        NextCard {
            label: "Next song".into(),
            word: Some(("OPENING".into(), Tone::Accent)),
            progress: Some((done, Tone::Accent)),
            name_faint: false,
            meaning: line.into(),
            ..NextCard::not_armed(of, name, None)
        }
    }

    /// Armed, but its check found a gap: Go does nothing.
    pub fn blocked(of: &str, name: &str, why: &str) -> NextCard {
        NextCard {
            label: "Next song".into(),
            border: Some(Tone::Bad),
            word: Some(("BLOCKED".into(), Tone::Bad)),
            name_faint: false,
            meaning: why.into(),
            go_line: "Arm another song from the set.".into(),
            cancel: ("Cancel next".into(), true),
            ..NextCard::not_armed(of, name, None)
        }
    }

    /// The set's last song plays. `changed` songs changed on stage.
    pub fn last_song(changed: usize) -> NextCard {
        let meaning = match changed {
            0 => "This is the set's last song.".to_string(),
            1 => "1 song changed on stage: keep or throw away its changes in the Set editor."
                .to_string(),
            n => format!(
                "{n} songs changed on stage: keep or throw away their changes in the Set editor."
            ),
        };
        NextCard {
            label: "Last song".into(),
            name: "End of the set".into(),
            name_faint: false,
            meaning,
            ..NextCard::not_armed("", "", None)
        }
    }

    /// An open failed after the song before had gone: `what` failed, and
    /// `before` (the song before) can be opened again.
    pub fn failed(name: &str, what: &str, before: &str) -> NextCard {
        NextCard {
            label: "Didn't open".into(),
            label_tone: Tone::Bad,
            border: Some(Tone::Bad),
            name_faint: false,
            meaning: what.into(),
            go_line: format!("{before}'s takes and changes are kept."),
            go: ("Try again".into(), true),
            cancel: (format!("Reopen {before}"), true),
            failed: true,
            ..NextCard::not_armed("", name, None)
        }
    }
}

/// A keyboard the rig knows, and what it plays.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyboardLine {
    pub name: String,
    /// "Chorus split" (the song's patch its keys reach, live shows C1),
    /// "connected", or "not connected".
    pub plays: String,
    /// Dim when it isn't connected, red when this song needs it too.
    pub tone: Tone,
    /// The patch's zones that reach this keyboard, as a strip at the
    /// line's right (empty without patches).
    pub bars: Vec<KeyBar>,
}

/// One bar of a keyboard line's zone strip (live shows C1): where its
/// keys lie across the strip, in thousandths from its left; its row of
/// the strip's `rows` (layered zones lie on rows of their own); its
/// instrument's colour; and whether that instrument is missing (red).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyBar {
    pub from: u16,
    pub to: u16,
    pub row: u8,
    pub rows: u8,
    pub color: mpfree_ui::Color,
    pub missing: bool,
}

/// A keyboard line's zone strip: as wide as this at most, and as tall.
const STRIP_W: i32 = 190;
const STRIP_H: i32 = 10;

/// A keyboard line's zone strip in `r`: a faint lane, then each bar on
/// its row.
fn paint_bars(cx: &mut PaintCx, t: &Theme, r: Rect, bars: &[KeyBar]) {
    cx.canvas.fill_round_rect(r, 3, t.raised);
    for b in bars {
        let rows = i32::from(b.rows.max(1));
        let gap = if r.h / rows >= 4 { 1 } else { 0 };
        let h = ((r.h - gap * (rows - 1)) / rows).max(1);
        let x0 = r.x + i32::from(b.from.min(1000)) * r.w / 1000;
        let x1 = r.x + i32::from(b.to.min(1000)) * r.w / 1000;
        let bar = Rect::new(x0, r.y + i32::from(b.row) * (h + gap), (x1 - x0).max(2), h);
        let ink = if b.missing {
            t.bad
        } else {
            t.swatch(b.color).fill
        };
        cx.canvas.fill_round_rect(bar, (h / 2).min(2), ink);
    }
}

/// The Show view: see the module notes.
#[derive(Debug, Default)]
pub struct ShowView {
    /// The set's name and "12 songs · 54 min".
    pub set_name: String,
    pub set_line: String,
    pub songs: Vec<SetRow>,
    /// Back to the set order would arm a different song.
    pub back_usable: bool,
    /// "Now · song 4 of 12" (drawn in capitals).
    pub title: String,
    pub song_name: String,
    /// Nothing is open (after a failed open): the name is faint.
    pub song_faint: bool,
    pub pill: Pill,
    /// "92 BPM · part 4 of 7 · 1:52 left".
    pub song_line: String,
    /// Autosave is held back while the set plays.
    pub autosave_waiting: bool,
    pub parts: Vec<PartTile>,
    pub card: NextCard,
    /// The song's notes from the set.
    pub notes: String,
    pub keyboards: Vec<KeyboardLine>,
    pub click: bool,
    /// Beside the click switch: "Count-in 1 bar, from the song."
    pub click_line: String,
    /// The first song in view, and the songs (NOW and NEXT) it last
    /// followed.
    top: Cell<usize>,
    followed: Cell<Option<(usize, Option<usize>)>>,
    grab: Option<Grab>,
}

#[derive(Clone, Copy, Debug)]
struct Grab {
    finger: u64,
    y: i32,
    top: usize,
    song: usize,
    dragging: bool,
}

/// Where the parts are.
#[derive(Clone, Copy, Debug)]
pub struct Geo {
    pub column: Rect,
    pub main: Rect,
    /// The songs' rows, and the line under them.
    pub rows: Rect,
    pub back: Rect,
    /// Clips, Song, List.
    pub lenses: [Rect; 3],
    pub title: Rect,
    pub name: Rect,
    pub pill: Rect,
    pub line: Rect,
    pub parts: Rect,
    pub card: Rect,
    pub notes: Rect,
    pub keys: Rect,
    pub click: Rect,
}

impl ShowView {
    pub fn new() -> ShowView {
        ShowView::default()
    }

    /// Everything the app fills in, from `v`, keeping where the set column
    /// is scrolled to and a finger on it.
    pub fn fill(&mut self, v: ShowView) {
        let (top, followed, grab) = (self.top.get(), self.followed.get(), self.grab.take());
        *self = v;
        self.top.set(top);
        self.followed.set(followed);
        self.grab = grab;
    }

    /// Where everything is in `r` (1116 × 610 on the MPC's screen).
    pub fn geo(r: Rect) -> Geo {
        let column = Rect::new(r.x, r.y, COL_W, r.h);
        let main = Rect::new(r.x + COL_W + COL_GAP, r.y, r.w - COL_W - COL_GAP, r.h);
        let back = Rect::new(
            column.x + 12,
            column.bottom() - 10 - BACK_H,
            column.w - 24,
            BACK_H,
        );
        let rows_y = column.y + 68;
        let rows = Rect::new(column.x + 12, rows_y, column.w - 24, back.y - 10 - rows_y);
        let inner = main.inset(16);
        let list = Rect::new(inner.right() - LENS_W, main.y + 10, LENS_W, LENS_H);
        let song = Rect::new(list.x - 8 - LENS_W, list.y, LENS_W, LENS_H);
        let clips = Rect::new(song.x - 8 - LENS_W, list.y, LENS_W, LENS_H);
        let title = Rect::new(inner.x, main.y + 14, clips.x - 16 - inner.x, 24);
        let pill = Rect::new(inner.right() - PILL_W, main.y + 58, PILL_W, PILL_H);
        let name = Rect::new(inner.x, pill.y, pill.x - 16 - inner.x, pill.h);
        let line = Rect::new(inner.x + 2, main.y + 106, inner.w - 2, 24);
        let parts = Rect::new(inner.x, main.y + 140, inner.w, TILE_H);
        let keys = Rect::new(inner.x, main.bottom() - 12 - FOOT_H, CARD_W, FOOT_H);
        let card_y = parts.bottom() + 16;
        let card = Rect::new(inner.x, card_y, CARD_W, keys.y - 12 - card_y);
        let notes = Rect::new(
            card.right() + 12,
            card.y,
            inner.right() - card.right() - 12,
            card.h,
        );
        let click = Rect::new(notes.x, keys.y, notes.w, FOOT_H);
        Geo {
            column,
            main,
            rows,
            back,
            lenses: [clips, song, list],
            title,
            name,
            pill,
            line,
            parts,
            card,
            notes,
            keys,
            click,
        }
    }

    /// The card's two buttons: Go (or Try again) and Cancel next (or
    /// Reopen).
    pub fn button_rects(r: Rect) -> [Rect; 2] {
        let c = ShowView::geo(r).card;
        let y = c.bottom() - 16 - BUTTON_H;
        [
            Rect::new(c.x + 16, y, BUTTON_W, BUTTON_H),
            Rect::new(c.x + 16 + BUTTON_W + 12, y, BUTTON_W, BUTTON_H),
        ]
    }

    /// The click switch.
    pub fn switch_rect(r: Rect) -> Rect {
        let c = ShowView::geo(r).click;
        Rect::new(c.x + 16, c.y + 38, SWITCH_W, SWITCH_H)
    }

    /// Songs that fit in the column: all of them, or as many as leave room
    /// for the line that says how many more there are.
    fn visible(&self, r: Rect) -> usize {
        let h = ShowView::geo(r).rows.h;
        let all = ((h + ROW_PITCH - ROW_H) / ROW_PITCH).max(1) as usize;
        if self.songs.len() <= all {
            all
        } else {
            ((h - MORE_H + ROW_PITCH - ROW_H) / ROW_PITCH).max(1) as usize
        }
    }

    fn now(&self) -> Option<usize> {
        self.songs.iter().position(|s| s.mark == SongMark::Now)
    }

    fn armed(&self) -> Option<usize> {
        self.songs
            .iter()
            .position(|s| matches!(s.mark, SongMark::Next(_)))
    }

    /// The first song in view: it follows NOW and NEXT when they change
    /// (both in view when they fit, else NEXT), and stays where a drag
    /// left it otherwise.
    fn first(&self, visible: usize) -> usize {
        let n = self.songs.len();
        if n <= visible {
            return 0;
        }
        let last = n - visible;
        let mut top = self.top.get().min(last);
        let now = self.now().unwrap_or(0);
        let armed = self.armed();
        if self.followed.get() != Some((now, armed)) {
            let (lo, hi) = match armed {
                Some(a) if a.abs_diff(now) < visible => (now.min(a), now.max(a)),
                Some(a) => (a, a),
                None => (now, (now + 1).min(n - 1)),
            };
            if lo < top {
                top = lo;
            } else if hi >= top + visible {
                top = (hi + 1 - visible).min(last);
            }
            self.followed.set(Some((now, armed)));
        }
        self.top.set(top);
        top
    }

    /// Song `k`'s row, if it is in view.
    pub fn row_rect(&self, r: Rect, k: usize) -> Option<Rect> {
        let rows = ShowView::geo(r).rows;
        let visible = self.visible(r);
        let first = self.first(visible);
        if k < first || k >= first + visible || k >= self.songs.len() {
            return None;
        }
        let y = rows.y + (k - first) as i32 * ROW_PITCH;
        Some(Rect::new(rows.x, y, rows.w, ROW_H))
    }

    /// The parts in view: the first, and how many fit.
    fn parts_in_view(&self, r: Rect) -> (usize, usize) {
        let w = ShowView::geo(r).parts.w;
        let fit = ((w + TILE_GAP) / (TILE_W + TILE_GAP)).max(1) as usize;
        let n = self.parts.len();
        if n <= fit {
            return (0, n);
        }
        let now = self
            .parts
            .iter()
            .position(|p| matches!(p.mark, PartMark::Now { .. }))
            .unwrap_or(0);
        // One played step before NOW stays in view.
        (now.saturating_sub(1).min(n - fit), fit)
    }

    /// The card's buttons as `Toggle` numbers, and whether each can be
    /// pressed.
    fn card_buttons(&self) -> [(usize, bool); 2] {
        if self.card.failed {
            [
                (SHOW_RETRY, self.card.go.1),
                (SHOW_REOPEN, self.card.cancel.1),
            ]
        } else {
            [(SHOW_GO, self.card.go.1), (SHOW_CANCEL, self.card.cancel.1)]
        }
    }

    /// A tap on song `k` arms it: not on NOW or NEXT.
    fn can_arm(&self, k: usize) -> bool {
        self.songs
            .get(k)
            .is_some_and(|s| !matches!(s.mark, SongMark::Now | SongMark::Next(_)))
    }

    fn paint_column(&self, cx: &mut PaintCx, r: Rect, g: &Geo) {
        let t = cx.theme.clone();
        let c = g.column;
        cx.panel(c);
        let head = Rect::new(c.x + 16, c.y + 12, c.w - 32, 22);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.text_dim,
            &self.set_name.to_uppercase(),
        );
        let sub = Rect::new(c.x + 16, c.y + 36, c.w - 32, 20);
        cx.text(
            FontId::Sans,
            t.size_small,
            sub,
            Align::Left,
            t.text_dim,
            &self.set_line,
        );
        let visible = self.visible(r);
        let first = self.first(visible);
        for k in 0..self.songs.len() {
            let Some(row) = self.row_rect(r, k) else {
                continue;
            };
            paint_song(cx, &t, row, k, &self.songs[k]);
        }
        let below = self.songs.len().saturating_sub(first + visible);
        let more = match (first, below) {
            (0, 0) => String::new(),
            (0, b) => format!("{b} more below"),
            (a, 0) => format!("{a} more above"),
            (a, b) => format!("{a} above · {b} below"),
        };
        if !more.is_empty() {
            let line = Rect::new(g.rows.x + 4, g.rows.bottom() - MORE_H, g.rows.w, MORE_H);
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                line,
                Align::Left,
                t.text_dim,
                &more,
            );
        }
        if self.songs.is_empty() {
            let msg = Rect::new(g.rows.x + 4, g.rows.y, g.rows.w - 8, ROW_H);
            cx.text(
                FontId::Sans,
                t.size_small,
                msg,
                Align::Left,
                t.text_faint,
                "This set has no songs yet",
            );
        }
        cx.canvas.fill_round_rect(g.back, t.round(8), t.raised);
        cx.text(
            FontId::Sans,
            t.size_small + 1.0,
            g.back,
            Align::Center,
            t.button_ink(self.back_usable),
            "Back to the set order",
        );
    }

    fn paint_band(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            g.title,
            Align::Left,
            t.text_dim,
            &self.title.to_uppercase(),
        );
        for (b, word) in g.lenses.iter().zip(["Clips", "Song", "List"]) {
            cx.canvas.fill_round_rect(*b, t.round(8), t.raised);
            cx.text(FontId::Sans, t.size_small, *b, Align::Center, t.text, word);
        }
        cx.text(
            FontId::Bold,
            NAME_PX,
            g.name,
            Align::Left,
            if self.song_faint {
                t.text_faint
            } else {
                t.text
            },
            &self.song_name,
        );
        let fill = t.tone(self.pill.tone);
        cx.canvas.fill_round_rect(g.pill, g.pill.h / 2, fill);
        let room = g.pill.w - 20;
        let mut px = t.size_body;
        while px > t.size_small - 2.0 && cx.measure(FontId::Bold, px, &self.pill.word) > room {
            px -= 1.0;
        }
        cx.text(
            FontId::Bold,
            px,
            g.pill,
            Align::Center,
            t.on_accent,
            &self.pill.word,
        );
        let under_w = if self.pill.under.is_empty() {
            0
        } else {
            let under = Rect::new(g.pill.x - 40, g.line.y, g.pill.w + 40, g.line.h);
            cx.text(
                FontId::Sans,
                t.size_small,
                under,
                Align::Right,
                t.text_dim,
                &self.pill.under,
            ) + 16
        };
        let line = Rect::new(g.line.x, g.line.y, g.line.w - under_w, g.line.h);
        let w = cx.text(
            FontId::Sans,
            t.size_body - 1.0,
            line,
            Align::Left,
            t.text_dim,
            &self.song_line,
        );
        if self.autosave_waiting {
            let gap = if self.song_line.is_empty() { 0 } else { 8 };
            let x = line.x + w + gap;
            let words = if self.song_line.is_empty() {
                "autosave waiting"
            } else {
                "·  autosave waiting"
            };
            let rest = Rect::new(x, line.y, (line.right() - x).max(0), line.h);
            cx.text(
                FontId::Sans,
                t.size_body - 1.0,
                rest,
                Align::Left,
                t.warn,
                words,
            );
        }
    }

    fn paint_parts(&self, cx: &mut PaintCx, r: Rect, g: &Geo) {
        let t = cx.theme.clone();
        if self.parts.is_empty() {
            let tile = g.parts;
            dashed(cx, tile, t.outline);
            cx.text(
                FontId::Sans,
                t.size_small + 1.0,
                tile.inset_xy(18, 0),
                Align::Left,
                t.text_dim,
                "No parts: a song list on Arrange gives this song its parts",
            );
            return;
        }
        let (first, n) = self.parts_in_view(r);
        for i in 0..n {
            let x = g.parts.x + i as i32 * (TILE_W + TILE_GAP);
            let tile = Rect::new(x, g.parts.y, TILE_W, TILE_H);
            paint_part(cx, &t, tile, &self.parts[first + i]);
        }
    }

    fn paint_card(&self, cx: &mut PaintCx, r: Rect, g: &Geo) {
        let t = cx.theme.clone();
        let c = g.card;
        let k = &self.card;
        card(cx, &t, c, k.border);
        let w = c.w - 32;
        let head = Rect::new(c.x + 16, c.y + 12, w, 20);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.tone(k.label_tone),
            &k.label.to_uppercase(),
        );
        if !k.of.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                head,
                Align::Right,
                t.text_dim,
                &k.of,
            );
        }
        let mut y = c.y + 38;
        if !k.name.is_empty() {
            cx.text(
                FontId::Bold,
                CARD_NAME_PX,
                Rect::new(c.x + 16, y, w, 34),
                Align::Left,
                if k.name_faint { t.text_dim } else { t.text },
                &k.name,
            );
        }
        y = c.y + 78;
        if let Some((word, tone)) = &k.word {
            cx.text(
                FontId::Bold,
                WORD_PX,
                Rect::new(c.x + 16, y, w, 30),
                Align::Left,
                t.tone(*tone),
                word,
            );
            y += 33;
        }
        if let Some((done, tone)) = k.progress {
            let track = Rect::new(c.x + 16, y + 2, w, 8);
            cx.canvas.fill_round_rect(track, 4, t.bg);
            let fill = (done.clamp(0.0, 1.0) * w as f32).round() as i32;
            cx.canvas
                .fill_round_rect(Rect::new(track.x, track.y, fill, 8), 4, t.tone(tone));
            y += 20;
        }
        let [go, _] = ShowView::button_rects(r);
        let room = go.y - 8;
        y = lines(
            cx,
            t.size_body - 1.0,
            22,
            Rect::new(c.x + 16, y, w, (room - y).max(0)),
            t.text_dim,
            &k.meaning,
            2,
        );
        if !k.go_line.is_empty() {
            lines(
                cx,
                t.size_body - 2.0,
                21,
                Rect::new(c.x + 16, y + 2, w, (room - y - 2).max(0)),
                t.text,
                &k.go_line,
                2,
            );
        }
        let words = [&k.go.0, &k.cancel.0];
        for (i, (b, (_, usable))) in ShowView::button_rects(r)
            .iter()
            .zip(self.card_buttons())
            .enumerate()
        {
            let green = i == 0 && !k.failed && usable;
            let fill = if green { t.good } else { t.raised };
            cx.canvas.fill_round_rect(*b, t.round(8), fill);
            let ink = if green {
                t.on_accent
            } else {
                t.button_ink(usable)
            };
            cx.text(
                FontId::Bold,
                t.size_body,
                b.inset_xy(8, 0),
                Align::Center,
                ink,
                words[i],
            );
        }
    }

    fn paint_notes(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let c = g.notes;
        card(cx, &t, c, None);
        let head = Rect::new(c.x + 16, c.y + 12, c.w - 32, 20);
        let title = if self.song_name.is_empty() {
            "NOTES".to_string()
        } else {
            format!("NOTES · {}", self.song_name.to_uppercase())
        };
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.text_dim,
            &title,
        );
        let body = Rect::new(c.x + 16, c.y + 40, c.w - 32, c.h - 50);
        if self.notes.trim().is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(body.x, body.y, body.w, 24),
                Align::Left,
                t.text_dim,
                "No notes: add them to the song in the Set editor",
            );
            return;
        }
        let max = (body.h / NOTES_LINE).max(1) as usize;
        lines(cx, NOTES_PX, NOTES_LINE, body, t.text, &self.notes, max);
    }

    fn paint_keys(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        let c = g.keys;
        card(cx, &t, c, None);
        let head = Rect::new(c.x + 16, c.y + 10, c.w - 32, 20);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.text_dim,
            "YOUR KEYBOARDS PLAY",
        );
        let top = c.y + 34;
        let fit = ((c.bottom() - 6 - top) / KEY_LINE).max(1) as usize;
        if self.keyboards.is_empty() {
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(c.x + 16, top, c.w - 32, KEY_LINE),
                Align::Left,
                t.text_dim,
                "No keyboards learned yet: plug one in and play it",
            );
            return;
        }
        let shown = if self.keyboards.len() > fit {
            fit - 1
        } else {
            self.keyboards.len()
        };
        for (i, k) in self.keyboards.iter().take(shown).enumerate() {
            let mut line = Rect::new(c.x + 16, top + i as i32 * KEY_LINE, c.w - 32, KEY_LINE);
            if !k.bars.is_empty() {
                let w = STRIP_W.min(line.w / 3);
                let strip = Rect::new(
                    line.right() - w,
                    line.y + (line.h - STRIP_H) / 2,
                    w,
                    STRIP_H,
                );
                paint_bars(cx, &t, strip, &k.bars);
                line.w -= w + 12;
            }
            let plays_w = cx.text(
                FontId::Sans,
                t.size_body - 1.0,
                line,
                Align::Right,
                t.tone(if k.tone == Tone::Normal {
                    Tone::Dim
                } else {
                    k.tone
                }),
                &k.plays,
            );
            let name = Rect::new(line.x, line.y, line.w - plays_w - 12, line.h);
            cx.text(
                FontId::Sans,
                t.size_body - 1.0,
                name,
                Align::Left,
                if k.tone == Tone::Dim {
                    t.text_dim
                } else {
                    t.text
                },
                &k.name,
            );
        }
        if shown < self.keyboards.len() {
            let line = Rect::new(c.x + 16, top + shown as i32 * KEY_LINE, c.w - 32, KEY_LINE);
            cx.text(
                FontId::Sans,
                t.size_small,
                line,
                Align::Left,
                t.text_dim,
                &format!("and {} more", self.keyboards.len() - shown),
            );
        }
    }

    fn paint_click(&self, cx: &mut PaintCx, r: Rect, g: &Geo) {
        let t = cx.theme.clone();
        let c = g.click;
        card(cx, &t, c, None);
        let head = Rect::new(c.x + 16, c.y + 10, c.w - 32, 20);
        cx.text(
            FontId::Bold,
            t.size_small - 1.0,
            head,
            Align::Left,
            t.text_dim,
            "CLICK",
        );
        let s = ShowView::switch_rect(r);
        cx.canvas.fill_round_rect(s, t.round(8), t.raised);
        if self.click {
            cx.canvas.stroke_round_rect(s, t.round(8), 2, t.good);
        }
        let dot = Rect::new(s.x + 18, s.y + s.h / 2 - 4, 8, 8);
        cx.canvas
            .fill_round_rect(dot, 4, if self.click { t.good } else { t.text_faint });
        cx.text(
            FontId::Bold,
            t.size_body - 1.0,
            Rect::new(dot.right() + 8, s.y, s.right() - dot.right() - 12, s.h),
            Align::Left,
            if self.click { t.text } else { t.text_dim },
            if self.click { "Click on" } else { "Click off" },
        );
        let beside = Rect::new(
            s.right() + 16,
            s.y,
            (c.right() - 14 - s.right() - 16).max(0),
            s.h + 8,
        );
        lines(
            cx,
            t.size_small,
            19,
            beside,
            t.text_dim,
            &self.click_line,
            2,
        );
    }
}

/// A song's row in the set column.
fn paint_song(cx: &mut PaintCx, t: &Theme, row: Rect, k: usize, song: &SetRow) {
    let (fill, edge) = match &song.mark {
        SongMark::Now => (t.raised.mix(t.good, 0.22), Some(t.good)),
        SongMark::Next(_) => (t.raised, Some(t.warn)),
        // Played: sunk toward the panel, as a played part's tile, its
        // words dim (never faint: they must still read).
        SongMark::Played => (t.surface.mix(t.raised, 0.35), None),
        _ => (t.raised, None),
    };
    cx.canvas.fill_round_rect(row, t.round(8), fill);
    if let Some(e) = edge {
        cx.canvas.stroke_round_rect(row, t.round(8), 2, e);
    }
    let played = song.mark == SongMark::Played;
    cx.text(
        FontId::Mono,
        t.size_body - 1.0,
        Rect::new(row.x + 14, row.y, 26, row.h),
        Align::Left,
        t.text_dim,
        &(k + 1).to_string(),
    );
    let right = row.right() - 14;
    let mark_w = match &song.mark {
        SongMark::Now => {
            let w = cx.text(
                FontId::Bold,
                t.size_small,
                Rect::new(right - 60, row.y, 60, row.h),
                Align::Right,
                t.good,
                "NOW",
            );
            w + 10
        }
        SongMark::Next(state) => {
            let top = Rect::new(right - 110, row.y + 4, 110, 20);
            let w1 = cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                top,
                Align::Right,
                t.warn,
                "NEXT",
            );
            let low = Rect::new(right - 110, row.y + 24, 110, 20);
            let w2 = cx.text(
                FontId::Sans,
                t.size_small - 2.0,
                low,
                Align::Right,
                t.text_dim,
                state,
            );
            w1.max(w2) + 10
        }
        SongMark::Played => {
            let w = cx.text(
                FontId::Sans,
                t.size_small - 1.0,
                Rect::new(right - 70, row.y, 70, row.h),
                Align::Right,
                t.text_dim,
                "played",
            );
            w + 10
        }
        SongMark::Limits | SongMark::NotReady => {
            let c = if song.mark == SongMark::Limits {
                t.warn
            } else {
                t.bad
            };
            let dot = Rect::new(right - 9, row.y + row.h / 2 - 4, 9, 9);
            cx.canvas.fill_round_rect(dot, 4, c);
            20
        }
        SongMark::Waiting => 0,
    };
    let bold = matches!(song.mark, SongMark::Now | SongMark::Next(_));
    let name = Rect::new(row.x + 42, row.y, right - mark_w - row.x - 42, row.h);
    cx.text(
        if bold { FontId::Bold } else { FontId::Sans },
        t.size_body,
        name,
        Align::Left,
        if played { t.text_dim } else { t.text },
        &song.name,
    );
}

/// A part's tile.
fn paint_part(cx: &mut PaintCx, t: &Theme, tile: Rect, part: &PartTile) {
    let played = part.mark == PartMark::Played;
    let fill = match part.mark {
        PartMark::Played => t.surface.mix(t.raised, 0.35),
        PartMark::Now { .. } => t.raised.mix(t.good, 0.22),
        _ => t.raised,
    };
    cx.canvas.fill_round_rect(tile, t.round(8), fill);
    match part.mark {
        PartMark::Now { .. } => cx.canvas.stroke_round_rect(tile, t.round(8), 2, t.good),
        PartMark::Next => cx.canvas.stroke_round_rect(tile, t.round(8), 2, t.warn),
        _ => {}
    }
    if let Some(scene) = part.scene {
        let stripe = Rect::new(tile.x + 8, tile.y + 9, 4, 22);
        let colour = t.track(scene);
        let colour = if played {
            colour.mix(t.surface, 0.55)
        } else {
            colour
        };
        cx.canvas.fill_round_rect(stripe, 2, colour);
    }
    cx.text(
        FontId::Bold,
        t.size_body - 1.0,
        Rect::new(tile.x + 18, tile.y + 9, tile.w - 24, 22),
        Align::Left,
        if played { t.text_dim } else { t.text },
        &part.name,
    );
    cx.text(
        FontId::Sans,
        t.size_small - 1.0,
        Rect::new(tile.x + 10, tile.y + 32, tile.w - 16, 18),
        Align::Left,
        t.text_dim,
        &part.meta,
    );
    match &part.mark {
        PartMark::Now { pass, done } => {
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                Rect::new(tile.x + 10, tile.y + 50, tile.w - 16, 16),
                Align::Left,
                t.good,
                pass,
            );
            let track = Rect::new(tile.x + 10, tile.y + 67, tile.w - 20, 4);
            cx.canvas.fill_round_rect(track, 2, t.bg);
            let w = (done.clamp(0.0, 1.0) * track.w as f32).round() as i32;
            cx.canvas
                .fill_round_rect(Rect::new(track.x, track.y, w, 4), 2, t.good);
        }
        PartMark::Next => {
            cx.text(
                FontId::Bold,
                t.size_small - 2.0,
                Rect::new(tile.x + 10, tile.y + 51, tile.w - 16, 16),
                Align::Left,
                t.warn,
                "NEXT",
            );
        }
        _ => {}
    }
}

/// A card: a little lighter than the panel, with an outline (or a 2 px
/// edge in `edge`'s colour).
fn card(cx: &mut PaintCx, t: &Theme, r: Rect, edge: Option<Tone>) {
    cx.canvas
        .fill_round_rect(r, t.round(10), t.surface.mix(t.raised, 0.4));
    match edge {
        Some(e) => cx.canvas.stroke_round_rect(r, t.round(10), 2, t.tone(e)),
        None => cx.canvas.stroke_round_rect(r, t.round(10), 1, t.outline),
    }
}

/// A dashed outline, 1 px: 6 px dashes, 4 px apart.
fn dashed(cx: &mut PaintCx, r: Rect, c: mpfree_ui::Color) {
    let mut x = r.x;
    while x < r.right() {
        let w = 6.min(r.right() - x);
        cx.canvas.fill_rect(Rect::new(x, r.y, w, 1), c);
        cx.canvas.fill_rect(Rect::new(x, r.bottom() - 1, w, 1), c);
        x += 10;
    }
    let mut y = r.y;
    while y < r.bottom() {
        let h = 6.min(r.bottom() - y);
        cx.canvas.fill_rect(Rect::new(r.x, y, 1, h), c);
        cx.canvas.fill_rect(Rect::new(r.right() - 1, y, 1, h), c);
        y += 10;
    }
}

/// `text` wrapped into `r` at `px`, `pitch` apart, at most `max` lines
/// (and as many as fit): anything past that ends the last line with "…".
/// Returns the y under the last line drawn.
fn lines(
    cx: &mut PaintCx,
    px: f32,
    pitch: i32,
    r: Rect,
    c: mpfree_ui::Color,
    text: &str,
    max: usize,
) -> i32 {
    if text.is_empty() || r.h <= 0 {
        return r.y;
    }
    let fit = ((r.h / pitch).max(1) as usize).min(max);
    let all = wrap(cx, px, text, r.w);
    let mut shown: Vec<String> = all.iter().take(fit).cloned().collect();
    if all.len() > fit {
        if let Some(last) = shown.last_mut() {
            // The "…" must fit: words come off the end until it does.
            let mut words: Vec<&str> = last.split(' ').collect();
            loop {
                let with = format!("{}…", words.join(" "));
                if words.len() <= 1 || cx.measure(FontId::Sans, px, &with) <= r.w {
                    *last = with;
                    break;
                }
                words.pop();
            }
        }
    }
    for (i, l) in shown.iter().enumerate() {
        let line = Rect::new(r.x, r.y + i as i32 * pitch, r.w, pitch);
        cx.text(FontId::Sans, px, line, Align::Left, c, l);
    }
    r.y + shown.len() as i32 * pitch
}

/// `text` wrapped to `w` at `px`: its own line breaks kept.
fn wrap(cx: &mut PaintCx, px: f32, text: &str, w: i32) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let longer = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if line.is_empty() || cx.measure(FontId::Sans, px, &longer) <= w {
                line = longer;
            } else {
                out.push(std::mem::take(&mut line));
                line = word.to_string();
            }
        }
        out.push(line);
    }
    out
}

/// A sample keyboard line's bar, on a strip of `row + 1` rows or two.
fn sample_bar(from: u16, to: u16, row: u8, rgb: u32) -> KeyBar {
    KeyBar {
        from,
        to,
        row,
        rows: 2.min(row + 2),
        color: mpfree_ui::Color::rgb(rgb),
        missing: false,
    }
}

/// The Show view as the approved mockup draws it: song 4 of 12 playing,
/// song 5 armed for a cold change. For the sample screens and the tests.
pub fn sample() -> ShowView {
    let names = [
        "Lanterns",
        "Copper sky",
        "Undertow",
        "Night drive",
        "Glass harbour",
        "Paper moons",
        "Slow signal",
        "Sunday chops",
        "Brass and rain",
        "Low tide",
        "Halfway house",
        "Last train",
    ];
    let songs = names
        .iter()
        .enumerate()
        .map(|(k, name)| SetRow {
            name: name.to_string(),
            mark: match k {
                0..=2 => SongMark::Played,
                3 => SongMark::Now,
                4 => SongMark::Next("cold load".into()),
                5 => SongMark::Limits,
                _ => SongMark::Waiting,
            },
        })
        .collect();
    let parts = [
        ("Intro", 2, 8, 0),
        ("Verse", 4, 16, 1),
        ("Verse B", 2, 8, 2),
        ("Drop", 4, 16, 3),
        ("Break", 1, 8, 4),
        ("Drop", 4, 16, 3),
        ("Outro", 1, 4, 5),
    ]
    .iter()
    .enumerate()
    .map(|(i, (name, times, bars, row))| PartTile {
        name: name.to_string(),
        meta: format!("× {times} · {bars} bars"),
        scene: Some(*row),
        mark: match i {
            0..=2 => PartMark::Played,
            3 => PartMark::Now {
                pass: "pass 3 of 4".into(),
                done: 0.62,
            },
            4 => PartMark::Next,
            _ => PartMark::Waiting,
        },
    })
    .collect();
    ShowView {
        set_name: "Friday at the Lantern".into(),
        set_line: "12 songs · 54 min".into(),
        songs,
        back_usable: false,
        title: "Now · song 4 of 12".into(),
        song_name: "Night drive".into(),
        pill: Pill::playing(),
        song_line: "92 BPM  ·  part 4 of 7  ·  1:52 left".into(),
        parts,
        card: NextCard::cold(
            "5 of 12",
            "Glass harbour",
            Some("Night drive"),
            Some(4),
            Ending::Finish,
            Starting::Stopped,
        ),
        notes: "F minor, count in 4 on the hats. Sam starts the Rhodes riff; drums come in \
                at bar 9. Chorus twice at the end if the room is up for it."
            .into(),
        keyboards: vec![
            // The song's patch, Chorus split: a bass under C3 on every
            // keyboard, and the Keystep's Rhodes above it with a pad.
            KeyboardLine {
                name: "Keystep 37".into(),
                plays: "Chorus split".into(),
                tone: Tone::Normal,
                bars: vec![
                    sample_bar(0, 333, 0, crate::view::TRACK_COLORS[2]),
                    sample_bar(333, 1000, 0, crate::view::TRACK_COLORS[0]),
                    sample_bar(500, 1000, 1, crate::view::TRACK_COLORS[4]),
                ],
            },
            KeyboardLine {
                name: "Nord Electro".into(),
                plays: "Chorus split".into(),
                tone: Tone::Normal,
                bars: vec![sample_bar(0, 333, 0, crate::view::TRACK_COLORS[2])],
            },
        ],
        click: true,
        click_line: "Count-in 1 bar, from the song.".into(),
        ..ShowView::default()
    }
}

impl Widget for ShowView {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let g = ShowView::geo(r);
        self.paint_column(cx, r, &g);
        cx.panel(g.main);
        self.paint_band(cx, &g);
        self.paint_parts(cx, r, &g);
        self.paint_card(cx, r, &g);
        self.paint_notes(cx, &g);
        self.paint_keys(cx, &g);
        self.paint_click(cx, r, &g);
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = ShowView::geo(r);
        let mut out = Vec::new();
        for (k, song) in self.songs.iter().enumerate() {
            let Some(row) = self.row_rect(r, k) else {
                continue;
            };
            out.push(
                Target::tap("song", k, format!("{} {}", k + 1, song.name), row)
                    .disabled_if(!self.can_arm(k)),
            );
        }
        out.push(
            Target::tap("back", 0, "Back to the set order", g.back).disabled_if(!self.back_usable),
        );
        for (k, (b, word)) in g.lenses.iter().zip(["Clips", "Song", "List"]).enumerate() {
            out.push(Target::tap("lens", k, word, *b));
        }
        let words = [&self.card.go.0, &self.card.cancel.0];
        for (i, (b, (which, usable))) in ShowView::button_rects(r)
            .iter()
            .zip(self.card_buttons())
            .enumerate()
        {
            let id = match which {
                SHOW_GO => "go",
                SHOW_CANCEL => "cancel",
                SHOW_RETRY => "retry",
                _ => "reopen",
            };
            out.push(Target::tap(id, 0, words[i].as_str(), *b).disabled_if(!usable));
        }
        out.push(Target::tap("click", 0, "Click", ShowView::switch_rect(r)));
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        match phase {
            Touch::Down => {}
            Touch::Move => {
                let visible = self.visible(r);
                let n = self.songs.len();
                let g = self.grab.as_mut().filter(|g| g.finger == finger)?;
                if !g.dragging && (y - g.y).abs() < DRAG_PX {
                    return None;
                }
                g.dragging = true;
                if n > visible {
                    let moved = (g.y - y) / ROW_PITCH;
                    let last = (n - visible) as i32;
                    let top = (g.top as i32 + moved).clamp(0, last) as usize;
                    self.top.set(top);
                    let (now, armed) = (self.now().unwrap_or(0), self.armed());
                    self.followed.set(Some((now, armed)));
                }
                return None;
            }
            Touch::Up | Touch::Cancel => {
                let g = self.grab.take().filter(|g| g.finger == finger)?;
                if g.dragging || phase == Touch::Cancel || !self.can_arm(g.song) {
                    return None;
                }
                return Some(EventKind::Select(g.song));
            }
        }
        let g = ShowView::geo(r);
        for (k, b) in g.lenses.iter().enumerate() {
            if b.contains(x, y) {
                return Some(EventKind::Toggle(SHOW_CLIPS + k));
            }
        }
        if g.back.contains(x, y) {
            return self.back_usable.then_some(EventKind::Toggle(SHOW_BACK));
        }
        for (b, (which, usable)) in ShowView::button_rects(r).iter().zip(self.card_buttons()) {
            if b.contains(x, y) {
                return usable.then_some(EventKind::Toggle(which));
            }
        }
        if ShowView::switch_rect(r).contains(x, y) {
            return Some(EventKind::Toggle(SHOW_CLICK));
        }
        let k = (0..self.songs.len())
            .find(|&k| self.row_rect(r, k).is_some_and(|row| row.contains(x, y)))?;
        // Armed on the lift, unless it drags (NOW and NEXT only scroll).
        let top = self.first(self.visible(r));
        self.grab = Some(Grab {
            finger,
            y,
            top,
            song: k,
            dragging: false,
        });
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mpfree_ui::{Fonts, SoftCanvas};

    /// The view's rectangle on the MPC's screen.
    const R: Rect = Rect {
        x: 12,
        y: 140,
        w: 1116,
        h: 610,
    };

    fn view(n: usize, now: usize, armed: Option<usize>) -> ShowView {
        let mut v = ShowView::new();
        v.set_name = "Friday at the Lantern".into();
        v.set_line = format!("{n} songs · 54 min");
        v.songs = (0..n)
            .map(|k| SetRow {
                name: format!("Song {}", k + 1),
                mark: if k < now {
                    SongMark::Played
                } else if k == now {
                    SongMark::Now
                } else if Some(k) == armed {
                    SongMark::Next("cold load".into())
                } else {
                    SongMark::Waiting
                },
            })
            .collect();
        v.back_usable = armed != Some(now + 1);
        v
    }

    /// Drawn where the approved mockup has it, at 1280 × 800.
    #[test]
    fn it_is_laid_out_as_drawn() {
        let g = ShowView::geo(R);
        assert_eq!(g.column, Rect::new(12, 140, 280, 610));
        assert_eq!(g.main, Rect::new(304, 140, 824, 610));
        assert_eq!(g.back, Rect::new(24, 694, 256, 46));
        assert_eq!(g.lenses[0], Rect::new(880, 150, 72, 32));
        assert_eq!(g.lenses[2], Rect::new(1040, 150, 72, 32));
        assert_eq!(g.pill, Rect::new(952, 198, 160, 44));
        assert_eq!(g.parts, Rect::new(320, 280, 792, 76));
        assert_eq!(g.card, Rect::new(320, 372, 404, 258));
        assert_eq!(g.notes, Rect::new(736, 372, 376, 258));
        assert_eq!(g.keys, Rect::new(320, 642, 404, 96));
        assert_eq!(g.click, Rect::new(736, 642, 376, 96));
        let [go, cancel] = ShowView::button_rects(R);
        assert_eq!(go, Rect::new(336, 558, 180, 56));
        assert_eq!(cancel, Rect::new(528, 558, 180, 56));
        assert_eq!(ShowView::switch_rect(R), Rect::new(752, 680, 120, 44));
        let v = view(12, 3, Some(4));
        assert_eq!(v.row_rect(R, 0), Some(Rect::new(24, 208, 256, 48)));
        assert_eq!(v.row_rect(R, 7), Some(Rect::new(24, 586, 256, 48)));
        assert_eq!(v.row_rect(R, 8), None, "the rest scroll");
        // Seven tiles fit across.
        let mut v = v;
        v.parts = vec![PartTile::default(); 7];
        assert_eq!(v.parts_in_view(R), (0, 7));
    }

    /// A tap on a song arms it on the lift; NOW and NEXT do nothing, so a
    /// double tap never goes; the buttons say what they are.
    #[test]
    fn a_tap_arms_a_song_and_never_goes() {
        let mut v = view(12, 3, Some(4));
        v.card = NextCard::cold(
            "5 of 12",
            "Song 5",
            Some("Song 4"),
            Some(4),
            Ending::Finish,
            Starting::Stopped,
        );
        let tap = |v: &mut ShowView, b: Rect| {
            let (x, y) = (b.x + b.w / 2, b.y + b.h / 2);
            let down = v.touch(1, Touch::Down, x, y, R);
            down.or_else(|| v.touch(1, Touch::Up, x, y, R))
        };
        let row = |v: &ShowView, k: usize| v.row_rect(R, k).unwrap();
        let r6 = row(&v, 6);
        assert_eq!(tap(&mut v, r6), Some(EventKind::Select(6)));
        let (r3, r4) = (row(&v, 3), row(&v, 4));
        assert_eq!(tap(&mut v, r3), None, "NOW");
        assert_eq!(tap(&mut v, r4), None, "NEXT");
        let [go, cancel] = ShowView::button_rects(R);
        assert_eq!(tap(&mut v, go), Some(EventKind::Toggle(SHOW_GO)));
        assert_eq!(tap(&mut v, cancel), Some(EventKind::Toggle(SHOW_CANCEL)));
        // Back to the set order is faint while the next song in order is
        // the one armed.
        let g = ShowView::geo(R);
        assert_eq!(tap(&mut v, g.back), None);
        v.back_usable = true;
        assert_eq!(tap(&mut v, g.back), Some(EventKind::Toggle(SHOW_BACK)));
        assert_eq!(tap(&mut v, g.lenses[1]), Some(EventKind::Toggle(SHOW_SONG)));
        assert_eq!(
            tap(&mut v, ShowView::switch_rect(R)),
            Some(EventKind::Toggle(SHOW_CLICK))
        );
        // Not armed: Go and Cancel next are faint and do nothing.
        v.card = NextCard::not_armed("5 of 12", "Song 5", None);
        assert_eq!(tap(&mut v, go), None);
        assert_eq!(tap(&mut v, cancel), None);
        // A failed open: Try again and Reopen.
        v.card = NextCard::failed("Song 5", "samples/strings.wav changed", "Song 4");
        assert_eq!(tap(&mut v, go), Some(EventKind::Toggle(SHOW_RETRY)));
        assert_eq!(tap(&mut v, cancel), Some(EventKind::Toggle(SHOW_REOPEN)));
        // Every control is a target, NOW and NEXT faint.
        let t = v.targets(R);
        let songs: Vec<_> = t.iter().filter(|t| t.id == "song").collect();
        assert_eq!(songs.len(), 8);
        assert!(!songs[3].enabled && !songs[4].enabled && songs[5].enabled);
        for id in ["back", "retry", "reopen", "click"] {
            assert!(t.iter().any(|t| t.id == id), "{id}");
        }
        assert_eq!(t.iter().filter(|t| t.id == "lens").count(), 3);
    }

    /// A long set keeps NOW and NEXT in view, and a drag scrolls it
    /// without arming anything.
    #[test]
    fn a_long_set_follows_now_and_next() {
        let mut v = view(20, 12, Some(13));
        assert!(v.row_rect(R, 12).is_some() && v.row_rect(R, 13).is_some());
        assert!(v.row_rect(R, 0).is_none());
        // Armed far ahead: the armed song comes into view.
        v.songs[13].mark = SongMark::Waiting;
        v.songs[19].mark = SongMark::Next("cold load".into());
        assert!(v.row_rect(R, 19).is_some());
        // A drag scrolls and arms nothing.
        let first = v.row_rect(R, 19).unwrap();
        v.touch(1, Touch::Down, first.x + 40, first.y + 10, R);
        v.touch(1, Touch::Move, first.x + 40, first.y + 200, R);
        assert_eq!(v.touch(1, Touch::Up, first.x + 40, first.y + 200, R), None);
        assert!(v.row_rect(R, 19).is_none(), "scrolled up");
        // It stays there until NOW or NEXT changes.
        assert!(v.row_rect(R, 19).is_none());
        v.songs[19].mark = SongMark::Waiting;
        v.songs[18].mark = SongMark::Next("cold load".into());
        assert!(v.row_rect(R, 18).is_some());
    }

    /// The parts strip keeps NOW in view, with the step before it.
    #[test]
    fn the_parts_keep_now_in_view() {
        let mut v = view(3, 0, None);
        v.parts = (0..12)
            .map(|i| PartTile {
                name: format!("Part {i}"),
                meta: "× 2 · 8 bars".into(),
                scene: Some(i),
                mark: if i == 9 {
                    PartMark::Now {
                        pass: "pass 1 of 2".into(),
                        done: 0.5,
                    }
                } else {
                    PartMark::Waiting
                },
            })
            .collect();
        assert_eq!(v.parts_in_view(R), (5, 7));
        v.parts[9].mark = PartMark::Waiting;
        v.parts[3].mark = PartMark::Now {
            pass: "pass 1 of 2".into(),
            done: 0.0,
        };
        assert_eq!(v.parts_in_view(R), (2, 7));
    }

    /// The card's words for each state.
    #[test]
    fn the_card_says_what_go_does() {
        let c = NextCard::cold(
            "5 of 12",
            "Glass harbour",
            Some("Night drive"),
            Some(4),
            Ending::Finish,
            Starting::Stopped,
        );
        assert_eq!(c.meaning, "about 4 s of silence while it opens");
        assert_eq!(
            c.go_line,
            "Go: Night drive lets its sound finish, then Glass harbour opens and waits for PLAY."
        );
        assert!(c.go.1 && c.cancel.1);
        let c = NextCard::cold(
            "",
            "Glass harbour",
            Some("Night drive"),
            None,
            Ending::Fade(4),
            Starting::CountIn,
        );
        assert_eq!(c.meaning, "pause not measured yet");
        assert_eq!(
            c.go_line,
            "Go: Night drive fades out over 4 s, then Glass harbour opens, counts in and plays."
        );
        let c = NextCard::finishing("5 of 12", "Glass harbour", "Night drive", 6);
        assert_eq!(c.go, ("Go now".to_string(), true));
        assert!(!c.cancel.1);
        let c = NextCard::blocked(
            "5 of 12",
            "Glass harbour",
            "the file samples/strings.wav is missing",
        );
        assert!(!c.go.1 && c.cancel.1);
        assert_eq!(c.border, Some(Tone::Bad));
        assert_eq!(
            NextCard::last_song(2).meaning,
            "2 songs changed on stage: keep or throw away their changes in the Set editor."
        );
        let c = NextCard::failed(
            "Glass harbour",
            "samples/strings.wav changed after the set check.",
            "Night drive",
        );
        assert_eq!(c.cancel.0, "Reopen Night drive");
        assert!(c.failed);
        assert_eq!(Pill::finishing(6).word, "FINISHING · 6 s");
    }

    /// The sample as drawn, written to `$MPFREE_SHOW_PNG` when it is set
    /// (to look at beside the mockup).
    #[test]
    fn the_sample_paints() {
        let theme = Theme::default();
        let mut fonts = Fonts::embedded();
        let mut canvas = SoftCanvas::new(1280, 800);
        let v = sample();
        let mut cx = PaintCx {
            canvas: &mut canvas,
            theme: &theme,
            fonts: &mut fonts,
        };
        v.paint(&mut cx, R);
        if let Ok(path) = std::env::var("MPFREE_SHOW_PNG") {
            let png = mpfree_ui::png::encode_rgb(1280, 800, &canvas.to_rgb());
            std::fs::write(path, png).expect("the picture is written");
        }
    }

    /// Every state paints without panicking, empty or full, and a long
    /// note is cut with "…".
    #[test]
    fn every_state_paints() {
        let theme = Theme::default();
        let mut fonts = Fonts::embedded();
        let mut canvas = SoftCanvas::new(1280, 800);
        let mut v = ShowView::new();
        let mut paint = |v: &ShowView| {
            let mut cx = PaintCx {
                canvas: &mut canvas,
                theme: &theme,
                fonts: &mut fonts,
            };
            v.paint(&mut cx, R);
        };
        paint(&v);
        v = view(14, 3, Some(4));
        v.notes = "F minor, count in 4 on the hats. ".repeat(20);
        v.keyboards = (0..5)
            .map(|i| KeyboardLine {
                name: format!("Keys {i}"),
                plays: "not connected".into(),
                tone: if i == 0 { Tone::Bad } else { Tone::Dim },
                bars: Vec::new(),
            })
            .collect();
        v.autosave_waiting = true;
        v.song_line = "92 BPM · part 4 of 7 · 1:52 left".into();
        for (pill, card) in [
            (
                Pill::playing(),
                NextCard::not_armed("5 of 14", "Song 5", Some("Nord isn't connected")),
            ),
            (
                Pill::finishing(6),
                NextCard::finishing("5 of 14", "Song 5", "Song 4", 6),
            ),
            (
                Pill::saving_takes(),
                NextCard::saving_takes("5 of 14", "Song 5", 0.7, "2 takes, 3.1 MB left"),
            ),
            (
                Pill::opening(),
                NextCard::opening("5 of 14", "Song 5", 0.4, "3 of 8 parts read"),
            ),
            (
                Pill::loaded("press PLAY"),
                NextCard::blocked(
                    "5 of 14",
                    "Song 5",
                    "the file samples/strings.wav is missing",
                ),
            ),
            (
                Pill::nothing_playing(),
                NextCard::failed("Song 5", "it failed", "Song 4"),
            ),
            (Pill::playing(), NextCard::last_song(0)),
        ] {
            v.pill = pill;
            v.card = card;
            paint(&v);
        }
    }
}
