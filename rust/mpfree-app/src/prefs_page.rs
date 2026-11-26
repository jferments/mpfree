//! Prefs (layout spec, section 5): a sidebar of tabs on the left and the
//! tab's settings on the right, in cards that fill the page, each setting
//! drawn with the control that fits its value: buttons for two to five
//! choices, a slider for steps or a range, a knob for a fine value, a
//! switch for on and off, a button for what opens or starts something
//! (the redesign canvas, round 2, Jay's go 24 Nov 2026 17:35Z).
//!
//! The app owns what the settings are and which one the wheel is on
//! ([`crate::app::App::menu_view`] fills a [`PrefsView`]); the screen adds
//! what only it reads (System info, the status board, the theme files).

use mpfree_ui::theme::contrast;
use mpfree_ui::{Align, Color, EventKind, FontId, PaintCx, Rect, Target, Theme, Touch, Widget};

/// The widget's name on the Prefs page.
pub const WIDGET: &str = "prefs";
/// The page's layout: the header, Prefs, the hint line.
pub const LAYOUT: &str = "col(header:60, prefs:*, hint:26)";

/// `Tab(KEEP)` and `Tab(GO_BACK)`: the buttons of "Keep this theme?".
pub const KEEP: usize = usize::MAX - 20;
pub const GO_BACK: usize = usize::MAX - 21;
/// `Tab(BACK)`: ‹ Back at the top of the sidebar (what MENU does).
pub const BACK: usize = mpfree_ui::widgets::OPTION_BACK;
/// `Tab(NEW_THEME)`: the Themes gallery's New theme tile.
pub const NEW_THEME: usize = usize::MAX - 22;
/// `Tab(CUSTOMISE)`: Customise › on the theme in use's card.
pub const CUSTOMISE: usize = usize::MAX - 23;
/// `Tab(RESET)`: the sidebar's Reset button, at its foot.
pub const RESET: usize = usize::MAX - 24;
/// The words over the sidebar's Reset button.
const RESET_LINE: [&str; 2] = ["Yours, kept on the stick: the", "same in every project."];

const GAP: i32 = 12;
const SIDE_W: i32 = 220;
const TAB_H: i32 = 62;
const TAB_STEP: i32 = 66;
/// One of Appearance's views under it in the sidebar.
const SUB_H: i32 = 44;
const SUB_STEP: i32 = 48;
const SUB_INDENT: i32 = 48;
const CARD_PAD_X: i32 = 20;
const CARD_PAD_TOP: i32 = 16;
const CARD_PAD_BOTTOM: i32 = 18;
const HEAD_H: i32 = 20;
const HEAD_GAP: i32 = 12;
const ROW_GAP: i32 = 14;
const NAME_H: i32 = 22;
const DESC_H: i32 = 20;
/// A choice's buttons, with and without a line under each word: at
/// least 64 px tall, the touch audit's Normal size for a tapped control.
const SEG_H: i32 = 64;
const SEG_SUB_H: i32 = 64;
const TILE_H: i32 = 62;
const TILE_GAP: i32 = 10;
/// The most a tile grows when its card has room to spare.
const TILE_GROW: i32 = 60;
/// How much a choice's buttons and a knob may grow into a card's spare
/// room (bigger targets before more space between rows).
const SEG_GROW: i32 = 24;
const KNOB_GROW: i32 = 40;
const SWITCH_W: i32 = 56;
const SWITCH_H: i32 = 32;
const BUTTON_W: i32 = 176;
const BUTTON_H: i32 = 46;
/// The sidebar's Reset (Normal size, as a choice's buttons).
const RESET_H: i32 = 64;
const SLIDE_H: i32 = 30;
/// A meter row's bar.
const METER_H: i32 = 10;
const LABELS_H: i32 = 22;
const KNOB_D: i32 = 64;
const INFO_H: i32 = 30;
const STATUS_H: i32 = 64;
const SPLIT_H: i32 = 52;
const PICTURE_MIN: i32 = 110;
const IN_USE_H: i32 = 164;
/// A knob of a line of several (Customise), and how much it may grow.
const DIAL_D: i32 = 64;
/// Between a dial and its value.
const DIAL_TEXT_GAP: i32 = 2;
const DIAL_GROW: i32 = 16;
/// A colour to tap, and the word under it.
const SWATCH_H: i32 = 38;
const SWATCH_LABEL_H: i32 = 20;
const SWATCH_W: i32 = 56;
/// A button with a drawing of what it does over its name.
const LOOK_H: i32 = 84;
const LOOK_GROW: i32 = 24;
/// Between rows that share a line.
const LINE_GAP: i32 = 16;
const GALLERY_COLS: usize = 4;
const GALLERY_ROWS: usize = 3;

/// What a card is for, which sets its colour (the synth and effect pages'
/// rule, colour by job): the theme's track colours 6, 5, 8 and 7 (cyan,
/// coral, lime and pink in the default theme), or a light grey.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hue {
    Cyan,
    Coral,
    Lime,
    Pink,
    #[default]
    Grey,
}

impl Hue {
    /// The hue in theme `t`: a card's title and a chosen button's fill.
    pub fn color(self, t: &Theme) -> Color {
        match self {
            Hue::Cyan => t.track(5),
            Hue::Coral => t.track(4),
            Hue::Lime => t.track(7),
            Hue::Pink => t.track(6),
            Hue::Grey => t.text_dim.mix(t.text, 0.55),
        }
    }
}

/// What a [`Control::Looks`] button draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    /// A box with these corners.
    Corners(mpfree_ui::Corners),
    /// The page behind the panels, drawn this way.
    Backdrop(mpfree_ui::Backdrop),
}

/// How a setting is drawn and changed.
#[derive(Clone, Debug, PartialEq)]
pub enum Control {
    /// Two to five buttons side by side, each a name and a line under it
    /// (may be empty); `at` is the one chosen. `now`, when it differs, is
    /// the one in use until the next start (ringed).
    Choice {
        options: Vec<(String, String)>,
        at: usize,
        now: Option<usize>,
    },
    /// Larger tiles, one over the other, each a name and what it does.
    Tiles {
        options: Vec<(String, String)>,
        at: usize,
    },
    /// A slider through fixed steps, each labelled under it.
    Steps { labels: Vec<String>, at: usize },
    /// A slider through a range, 0 to 1.
    Slider { pos: f32 },
    /// A knob through a range, 0 to 1 (a fine value).
    Knob { pos: f32 },
    /// A small knob over its value and name, several to a line
    /// (Customise), 0 to 1. With `dot`, a ring of every hue round a dot of
    /// that colour, for a colour's hue.
    Dial { pos: f32, dot: Option<Color> },
    /// Colours to tap, each with its word under it (or its number inside
    /// it), the one being set ringed.
    Swatches {
        colours: Vec<(String, Color)>,
        at: Option<usize>,
        numbered: bool,
    },
    /// Two to five buttons, each a small drawing of what it does over its
    /// name; `at` is the one chosen.
    Looks {
        options: Vec<(String, Look)>,
        at: usize,
    },
    /// ‹ the words ›: steps through a list.
    Stepper(String),
    /// On or off.
    Switch(bool),
    /// A level meter across the row: what comes in now, from
    /// [`PrefsPage::meter`] (the line input's, while it is on).
    Meter,
    /// Opens or starts something; the button's words.
    Button(String),
    /// A line about the system (System status): a dot in `tone`, a title
    /// and a line, with a button that opens the page.
    Status {
        title: String,
        line: String,
        tone: mpfree_ui::Tone,
        button: String,
    },
}

/// A drawing over a setting's control, which takes the room the card has
/// spare.
#[derive(Clone, Debug, PartialEq)]
pub enum Picture {
    /// A curve from (0, 0) to (1, 1), x and y from 0 to 1, with a straight
    /// line under it for comparison; and the last hit, as it came and as
    /// it played (1 to 127), marked on it.
    Curve(Vec<(f32, f32)>, Option<(u8, u8)>),
    /// The MPC's 16 pads, pad 1 at the bottom left, each in its colour as
    /// it would light (Prefs › Screen and lights).
    Pads(Vec<Color>),
}

/// One setting: a row of the menu, in the wheel's order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowView {
    pub name: String,
    /// One line under the name: what the setting does.
    pub desc: String,
    /// The value in words at the right of the name ("5 min · 131 MB");
    /// may be empty.
    pub value: String,
    pub control: Option<Control>,
    pub picture: Option<Picture>,
    /// The name and its line are left out (a tile row named by its card).
    pub bare: bool,
    /// Drawn faded: the setting has nothing to change as things are.
    pub faded: bool,
    /// Shares a line with the rows next to it that share one too, taking
    /// this many parts of its width; 0: a line of its own.
    pub share: u8,
    /// A button drawn in the picked colour: the card's main action.
    pub strong: bool,
}

/// A bar split into parts at the foot of a card (what the two memories
/// take together): a title and total, then each part's name, size and
/// share.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Split {
    pub title: String,
    pub total: String,
    pub parts: Vec<(String, f32)>,
}

/// One card of a tab.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardView {
    pub title: String,
    /// Dim words at the right of the title.
    pub note: String,
    pub hue: Hue,
    /// 0: the left column, 1: the right, 2: across both, over them.
    pub column: u8,
    /// Takes the room the column has spare.
    pub fill: bool,
    /// Its settings, as indexes into [`PrefsView::rows`].
    pub rows: Vec<usize>,
    /// The System info list, after the first row (the screen fills it).
    pub info: bool,
    pub split: Option<Split>,
}

/// One theme in the Themes gallery.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThemeTile {
    /// What `settings.theme` holds ("default", a file's name).
    pub key: String,
    pub label: String,
    /// A file with lines it couldn't use: how many.
    pub problems: usize,
}

/// The Themes view: every theme, the one in use, and "Keep this theme?".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gallery {
    pub tiles: Vec<ThemeTile>,
    /// The tile in use (on screen now).
    pub at: usize,
    /// "Keep Console?" and its line, while it asks.
    pub trial: Option<(String, String)>,
    /// What the theme in use is, and its file's problems, a line each.
    pub about: Vec<String>,
    /// The New theme tile after the themes.
    pub new_theme: bool,
    /// Customise › on the in-use card (not while it asks to keep).
    pub customise: bool,
}

/// One entry of the sidebar: a tab, or one of Appearance's views under it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SideTab {
    pub name: String,
    /// The line under a tab's name (none under a view).
    pub line: String,
    /// One of Appearance's views, drawn under it, indented.
    pub sub: bool,
    /// The tab a tap opens (the app's number for it).
    pub tab: usize,
    /// Drawn lit: the tab open, or the tab whose view is open.
    pub lit: bool,
    /// A tab's drawing: 0 Playing, 1 Sound, 2 Appearance, 3 Projects, 4
    /// System.
    pub icon: usize,
}

/// Everything the app says about Prefs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PrefsView {
    /// The sidebar's tabs, with Appearance's views under it while one of
    /// them is open.
    pub tabs: Vec<SideTab>,
    /// The tab open (the app's number for it).
    pub tab: usize,
    pub cards: Vec<CardView>,
    pub rows: Vec<RowView>,
    /// The row the wheel is on.
    pub selected: usize,
    /// The wheel's push opened it: the wheel changes its value.
    pub editing: bool,
    /// The Themes tab draws a gallery in place of cards.
    pub gallery: Option<Gallery>,
    /// The hint line's words, where the tab has its own (Customise says
    /// how well the theme reads).
    pub hint: Option<String>,
    /// The sidebar's Reset button's words ("Reset Playing"), on a tab
    /// that has one.
    pub reset: Option<String>,
}

/// Where everything sits, worked out from the view and the widget's
/// rectangle (no text is measured, so taps and drawing agree).
#[derive(Clone, Debug, Default, PartialEq)]
struct Geo {
    side: Rect,
    back: Rect,
    /// The sidebar's Reset button, at its foot.
    reset: Option<Rect>,
    tabs: Vec<Rect>,
    content: Rect,
    cards: Vec<Rect>,
    /// Each row's whole area (the outline goes round it), and its
    /// control's parts.
    rows: Vec<RowGeo>,
    /// Each card's info list and split bar.
    info: Vec<Option<Rect>>,
    split: Vec<Option<Rect>>,
    /// Thin lines between a card's rows of buttons and switches.
    dividers: Vec<Rect>,
    /// The gallery's tiles (on screen), its New theme tile, its in-use
    /// card and its buttons.
    tiles: Vec<(usize, Rect)>,
    new_theme: Option<Rect>,
    in_use: Rect,
    keep: Option<(Rect, Rect)>,
    customise: Option<Rect>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct RowGeo {
    area: Rect,
    head: Rect,
    picture: Option<Rect>,
    /// The control: the whole of it, and each of its parts (a choice's
    /// buttons, a slider's track).
    control: Rect,
    parts: Vec<Rect>,
}

/// Prefs on screen.
#[derive(Default)]
pub struct PrefsPage {
    pub view: PrefsView,
    /// System info's rows (name, value), read when the System tab opens.
    pub info: Vec<(String, String)>,
    /// The theme files, for their tiles' colours.
    pub files: Vec<(String, Theme)>,
    /// A finger turning a knob: its finger, row, where it went down and
    /// the value then.
    drag: Option<(u64, usize, i32, f32)>,
    /// A finger moving a slider: its finger and row.
    slide: Option<(u64, usize)>,
    /// A finger down on Reset: it acts when that finger lifts on it, so
    /// a finger that slides off or is cancelled changes nothing.
    resetting: Option<u64>,
    /// The level a meter row shows, dBFS: what comes in now, None while
    /// nothing is listened to (the screen sets it as levels arrive).
    pub meter: Option<f32>,
}

impl PrefsPage {
    /// The theme a tile shows: built in, or a theme file.
    fn look(&self, key: &str) -> Option<Theme> {
        Theme::builtin(key).or_else(|| {
            self.files
                .iter()
                .find(|(f, _)| f == key)
                .map(|(_, t)| t.clone())
        })
    }

    /// The page's parts in `r`, the slot the layout gives it (already
    /// inside the screen's margins: 1256 x 666 on the MPC's panel, so the
    /// sidebar is 220 wide and the cards' page 1024, as drawn).
    fn geo(&self, r: Rect) -> Geo {
        let side = Rect::new(r.x, r.y, SIDE_W, r.h);
        let content = Rect::new(side.right() + GAP, r.y, r.right() - side.right() - GAP, r.h);
        let back = Rect::new(side.x + 10, side.y + 10, 100, 46);
        // The tabs down the sidebar, Appearance's views under it, indented.
        let mut y = side.y + 68;
        let tabs = self
            .view
            .tabs
            .iter()
            .map(|t| {
                let r = if t.sub {
                    Rect::new(side.x + 8, y, side.w - 16, SUB_H)
                } else {
                    Rect::new(side.x + 8, y, side.w - 16, TAB_H)
                };
                y += if t.sub { SUB_STEP } else { TAB_STEP };
                r
            })
            .collect();
        let reset = self.view.reset.as_ref().map(|_| {
            Rect::new(
                side.x + 10,
                side.bottom() - 10 - RESET_H,
                side.w - 20,
                RESET_H,
            )
        });
        let mut g = Geo {
            side,
            back,
            reset,
            tabs,
            content,
            rows: vec![RowGeo::default(); self.view.rows.len()],
            ..Geo::default()
        };
        if let Some(gal) = &self.view.gallery {
            self.gallery_geo(&mut g, gal);
        } else {
            self.cards_geo(&mut g);
        }
        g
    }

    fn gallery_geo(&self, g: &mut Geo, gal: &Gallery) {
        let c = g.content;
        g.in_use = Rect::new(c.x, c.bottom() - IN_USE_H, c.w, IN_USE_H);
        let area = Rect::new(c.x, c.y, c.w, c.h - IN_USE_H - GAP);
        let per = GALLERY_COLS * GALLERY_ROWS;
        let first = (gal.at / per) * per;
        let cols = GALLERY_COLS as i32;
        let tw = (area.w - GAP * (cols - 1)) / cols;
        // As many rows as the themes on this page fill (up to three), so a
        // short list draws bigger tiles rather than an empty row.
        // The New theme tile is the cell after the last theme.
        let cells = gal.tiles.len() + usize::from(gal.new_theme);
        let shown = cells.saturating_sub(first).min(per);
        let rows = shown.div_ceil(GALLERY_COLS).clamp(1, GALLERY_ROWS) as i32;
        let th = (area.h - GAP * (rows - 1)) / rows;
        let cell = |k: usize| {
            let i = (k - first) as i32;
            Rect::new(
                area.x + (i % cols) * (tw + GAP),
                area.y + (i / cols) * (th + GAP),
                tw,
                th,
            )
        };
        g.tiles = (first..gal.tiles.len().min(first + per))
            .map(|k| (k, cell(k)))
            .collect();
        let last = gal.tiles.len();
        if gal.new_theme && (first..first + per).contains(&last) {
            g.new_theme = Some(cell(last));
        }
        let inner = g.in_use.inset(18);
        let w = 180;
        if gal.trial.is_some() {
            let keep = Rect::new(inner.right() - w, inner.y + 6, w, 52);
            let back = Rect::new(inner.right() - w, keep.bottom() + 12, w, 52);
            g.keep = Some((keep, back));
        } else if gal.customise {
            g.customise = Some(Rect::new(inner.right() - w, inner.y + 6, w, 52));
        }
    }

    /// The natural height of row `i` (without spare room).
    fn row_h(&self, i: usize) -> i32 {
        let row = &self.view.rows[i];
        let head = head_h(row);
        let picture = if row.picture.is_some() {
            PICTURE_MIN + 10
        } else {
            0
        };
        let control = match &row.control {
            None => 0,
            Some(Control::Choice { options, .. }) => {
                let subs = options.iter().any(|o| !o.1.is_empty());
                8 + if subs { SEG_SUB_H } else { SEG_H }
            }
            Some(Control::Tiles { .. }) => TILE_H + if row.bare { 0 } else { 8 },
            Some(Control::Steps { .. }) => 6 + SLIDE_H + LABELS_H,
            Some(Control::Slider { .. }) => 6 + SLIDE_H,
            Some(Control::Meter) => 6 + METER_H,
            Some(Control::Knob { .. }) => return KNOB_D + 16,
            Some(Control::Dial { .. }) => return DIAL_D + DIAL_TEXT_GAP + NAME_H + DESC_H,
            Some(Control::Swatches {
                colours, numbered, ..
            }) => {
                if inline_swatches(row, colours.len()) {
                    return head.max(SWATCH_H + 4);
                }
                6 + SWATCH_H + if *numbered { 0 } else { SWATCH_LABEL_H }
            }
            Some(Control::Looks { .. }) => 8 + LOOK_H,
            Some(Control::Switch(_)) | Some(Control::Button(_)) | Some(Control::Stepper(_)) => {
                return head.max(BUTTON_H);
            }
            Some(Control::Status { .. }) => return STATUS_H,
        };
        head + picture + control
    }

    /// Card `card`'s rows in lines: rows that share a line together, the
    /// rest a line each.
    fn lines(&self, card: &CardView) -> Vec<Vec<usize>> {
        let mut out: Vec<Vec<usize>> = Vec::new();
        for &i in &card.rows {
            let shares = self.view.rows[i].share > 0;
            match out.last_mut() {
                Some(line) if shares && self.view.rows[line[0]].share > 0 => line.push(i),
                _ => out.push(vec![i]),
            }
        }
        out
    }

    /// A line's natural height: its tallest row's.
    fn line_h(&self, line: &[usize]) -> i32 {
        line.iter().map(|&i| self.row_h(i)).max().unwrap_or(0)
    }

    fn card_h(&self, card: &CardView) -> i32 {
        let lines = self.lines(card);
        let rows: i32 = lines.iter().map(|l| self.line_h(l)).sum();
        let gaps = ROW_GAP * lines.len().saturating_sub(1) as i32;
        // System info's list takes the room spare (and shows the rows that
        // fit), so a long list never pushes the cards past the foot.
        let info = if card.info { ROW_GAP + INFO_H } else { 0 };
        let split = if card.split.is_some() {
            ROW_GAP + SPLIT_H
        } else {
            0
        };
        CARD_PAD_TOP + card_head(card) + rows + gaps + info + split + CARD_PAD_BOTTOM
    }

    fn cards_geo(&self, g: &mut Geo) {
        let mut c = g.content;
        g.cards = vec![Rect::default(); self.view.cards.len()];
        g.info = vec![None; self.view.cards.len()];
        g.split = vec![None; self.view.cards.len()];
        // Cards across both columns go over them, at their own height.
        for k in 0..self.view.cards.len() {
            if self.view.cards[k].column == 2 {
                let h = self.card_h(&self.view.cards[k]);
                let rect = Rect::new(c.x, c.y, c.w, h);
                g.cards[k] = rect;
                self.card_rows_geo(g, k, rect, 0);
                c = Rect::new(c.x, c.y + h + GAP, c.w, c.h - h - GAP);
            }
        }
        let col_w = (c.w - GAP) / 2;
        for col in 0..2u8 {
            let ids: Vec<usize> = (0..self.view.cards.len())
                .filter(|&k| self.view.cards[k].column == col)
                .collect();
            if ids.is_empty() {
                continue;
            }
            let natural: Vec<i32> = ids
                .iter()
                .map(|&k| self.card_h(&self.view.cards[k]))
                .collect();
            let used: i32 = natural.iter().sum::<i32>() + GAP * (ids.len() as i32 - 1);
            let spare = (c.h - used).max(0);
            // The column's spare room goes to its filling cards, else to
            // its last card, so no column ends short of the page's foot.
            let fills: Vec<usize> = ids
                .iter()
                .copied()
                .filter(|&k| self.view.cards[k].fill)
                .collect();
            // Without one, every card takes a share by its rows, so the
            // room spreads evenly down the column.
            let takers = if fills.is_empty() { ids.clone() } else { fills };
            let weight = |k: usize| self.view.cards[k].rows.len().max(1) as i32;
            let total: i32 = takers.iter().map(|&k| weight(k)).sum();
            let x = c.x + i32::from(col) * (col_w + GAP);
            let mut y = c.y;
            let mut left = spare;
            for (n, &k) in ids.iter().enumerate() {
                let mut h = natural[n];
                if takers.contains(&k) {
                    let share = if takers.last() == Some(&k) {
                        left
                    } else {
                        spare * weight(k) / total
                    };
                    left -= share;
                    h += share;
                }
                let rect = Rect::new(x, y, col_w, h);
                g.cards[k] = rect;
                self.card_rows_geo(g, k, rect, h - natural[n]);
                y += h + GAP;
            }
        }
    }

    /// Lays out card `k`'s rows in `rect`, with `spare` pixels more than
    /// it needs: a picture takes them, else they go between the rows.
    fn card_rows_geo(&self, g: &mut Geo, k: usize, rect: Rect, spare: i32) {
        let card = &self.view.cards[k];
        let head = card_head(card);
        let inner = Rect::new(
            rect.x + CARD_PAD_X,
            rect.y + CARD_PAD_TOP + head,
            rect.w - 2 * CARD_PAD_X,
            rect.h - CARD_PAD_TOP - head - CARD_PAD_BOTTOM,
        );
        let lines = self.lines(card);
        let pictured = card
            .rows
            .iter()
            .any(|&i| self.view.rows[i].picture.is_some());
        // A picture or System info's list takes the spare room. Else the
        // controls grow first, up to a point (tiles, a choice's buttons, a
        // knob), and the rest spreads evenly around the lines.
        let room = |i: usize| match &self.view.rows[i].control {
            Some(Control::Tiles { .. }) => TILE_GROW,
            Some(Control::Choice { .. }) => SEG_GROW,
            Some(Control::Knob { .. }) => KNOB_GROW,
            Some(Control::Dial { .. }) => DIAL_GROW,
            Some(Control::Looks { .. }) => LOOK_GROW,
            _ => 0,
        };
        let line_room = |l: &[usize]| l.iter().map(|&i| room(i)).max().unwrap_or(0);
        let mut grows = vec![0; lines.len()];
        let mut spare = spare;
        if !pictured && !card.info {
            let want: i32 = lines.iter().map(|l| line_room(l)).sum();
            if want > 0 {
                let give = spare.min(want);
                for (n, l) in lines.iter().enumerate() {
                    grows[n] = give * line_room(l) / want;
                }
                spare -= grows.iter().sum::<i32>();
            }
        }
        // The rest around each line: half above it, half below.
        let around = if pictured || card.info || lines.is_empty() {
            0
        } else {
            spare / lines.len() as i32
        };
        let compact = |l: &[usize]| {
            l.len() == 1
                && matches!(
                    self.view.rows[l[0]].control,
                    Some(Control::Button(_)) | Some(Control::Switch(_))
                )
        };
        let mut y = inner.y + around / 2;
        for (n, line) in lines.iter().enumerate() {
            let pictured_line = line.iter().any(|&i| self.view.rows[i].picture.is_some());
            let grow = if pictured_line { spare } else { grows[n] };
            let h = self.line_h(line) + grow;
            // Rows sharing a line take its width by their shares.
            let shares: i32 = line
                .iter()
                .map(|&i| i32::from(self.view.rows[i].share.max(1)))
                .sum();
            let free = inner.w - LINE_GAP * (line.len() as i32 - 1);
            let mut x = inner.x;
            for (m, &i) in line.iter().enumerate() {
                let w = if m + 1 == line.len() {
                    inner.right() - x
                } else {
                    free * i32::from(self.view.rows[i].share.max(1)) / shares
                };
                let area = Rect::new(x, y, w, h);
                g.rows[i] = self.row_geo(i, area, grow);
                x += w + LINE_GAP;
            }
            y += h + ROW_GAP + around;
            if let Some(next) = lines.get(n + 1) {
                if compact(line) && compact(next) {
                    let mid = y - (ROW_GAP + around) / 2;
                    g.dividers.push(Rect::new(inner.x, mid, inner.w, 1));
                }
            }
            if n == 0 && card.info {
                // System info's list after the first line, taking the room
                // spare.
                let rest = lines.len() as i32 - 1;
                let after: i32 =
                    lines[1..].iter().map(|l| self.line_h(l)).sum::<i32>() + ROW_GAP * rest;
                let h = (inner.bottom() - y - after).max(INFO_H);
                g.info[k] = Some(Rect::new(inner.x, y, inner.w, h));
                y += h + ROW_GAP;
            }
        }
        if card.split.is_some() {
            g.split[k] = Some(Rect::new(
                inner.x,
                inner.bottom() - SPLIT_H,
                inner.w,
                SPLIT_H,
            ));
        }
    }

    fn row_geo(&self, i: usize, area: Rect, grow: i32) -> RowGeo {
        let row = &self.view.rows[i];
        let head_h = head_h(row);
        let mut geo = RowGeo {
            area,
            head: Rect::new(area.x, area.y, area.w, head_h),
            ..RowGeo::default()
        };
        let mut y = area.y + head_h;
        if row.picture.is_some() {
            let h = PICTURE_MIN + grow;
            geo.picture = Some(Rect::new(area.x, y + 6, area.w, h));
            y += h + 10;
        }
        match &row.control {
            None => {}
            Some(Control::Choice { options, .. }) => {
                let subs = options.iter().any(|o| !o.1.is_empty());
                let h = if subs { SEG_SUB_H } else { SEG_H };
                // The picture took the growth when there is one.
                let more = if row.picture.is_some() { 0 } else { grow };
                let c = Rect::new(area.x, y + 8, area.w, h + more);
                geo.control = c;
                geo.parts = split_row(c, options.len(), 8);
            }
            Some(Control::Tiles { options, .. }) => {
                let top = if row.bare { y } else { y + 8 };
                let c = Rect::new(area.x, top, area.w, area.bottom() - top);
                geo.control = c;
                geo.parts = split_row(c, options.len(), TILE_GAP);
            }
            Some(Control::Steps { labels, .. }) => {
                let c = Rect::new(area.x, y + 6, area.w, SLIDE_H + LABELS_H);
                geo.control = c;
                // Each step's place along the track, as a thin part.
                let track = Rect::new(c.x + 18, c.y, c.w - 36, SLIDE_H);
                let n = labels.len().max(2) as i32 - 1;
                geo.parts = (0..labels.len() as i32)
                    .map(|s| Rect::new(track.x + track.w * s / n, c.y, 1, c.h))
                    .collect();
            }
            Some(Control::Slider { .. }) => {
                geo.control = Rect::new(area.x, y + 6, area.w, SLIDE_H);
            }
            Some(Control::Meter) => {
                geo.control = Rect::new(area.x, y + 6, area.w, METER_H);
            }
            Some(Control::Knob { .. }) => {
                // The knob at the left; its name, value and line beside it.
                let d = KNOB_D + grow;
                geo.control = Rect::new(area.x, area.y + (area.h - d) / 2, d, d);
                let h = 2 * NAME_H + DESC_H;
                geo.head = Rect::new(
                    area.x + d + 24,
                    area.y + (area.h - h) / 2,
                    area.w - d - 24,
                    h,
                );
            }
            Some(Control::Dial { .. }) => {
                // The knob over its value and name, in the middle.
                let d = DIAL_D + grow;
                let block = d + DIAL_TEXT_GAP + NAME_H + DESC_H;
                let top = area.y + (area.h - block) / 2;
                geo.control = Rect::new(area.x + (area.w - d) / 2, top, d, d);
                geo.head = Rect::new(area.x, top + d + DIAL_TEXT_GAP, area.w, NAME_H + DESC_H);
            }
            Some(Control::Swatches { colours, .. }) => {
                let n = colours.len().max(1) as i32;
                if inline_swatches(row, colours.len()) {
                    // A few, at the right of the name.
                    let w = n * SWATCH_W + (n - 1) * 8;
                    let c = Rect::new(
                        area.right() - w,
                        area.y + (area.h - SWATCH_H) / 2,
                        w,
                        SWATCH_H,
                    );
                    geo.control = c;
                    geo.parts = split_row(c, colours.len(), 8);
                    geo.head.w = area.w - w - 16;
                } else {
                    let c = Rect::new(area.x, y + 6, area.w, SWATCH_H);
                    geo.control = c;
                    geo.parts = split_row(c, colours.len(), 6);
                }
            }
            Some(Control::Looks { options, .. }) => {
                let c = Rect::new(area.x, y + 8, area.w, area.bottom() - y - 8);
                geo.control = c;
                geo.parts = split_row(c, options.len(), 8);
            }
            Some(Control::Stepper(_)) => {
                let w = if row.bare {
                    area.w
                } else {
                    (area.w - 120).max(BUTTON_W)
                };
                let c = Rect::new(
                    area.right() - w,
                    area.y + (area.h - BUTTON_H) / 2,
                    w,
                    BUTTON_H,
                );
                geo.control = c;
                // Its two halves: back a step, on a step.
                geo.parts = split_row(c, 2, 0);
                geo.head.w = area.w - w - 16;
            }
            Some(Control::Switch(_)) => {
                geo.control = Rect::new(
                    area.right() - SWITCH_W - 4,
                    area.y + (area.h - BUTTON_H) / 2,
                    SWITCH_W + 4,
                    BUTTON_H,
                );
                geo.head.w = area.w - SWITCH_W - 16;
            }
            Some(Control::Button(_)) => {
                // A bare button fills its share of a line.
                let w = if row.bare {
                    area.w
                } else {
                    BUTTON_W.min(area.w)
                };
                geo.control = Rect::new(
                    area.right() - w,
                    area.y + (area.h - BUTTON_H) / 2,
                    w,
                    BUTTON_H,
                );
                geo.head.w = area.w - w - 16;
            }
            Some(Control::Status { .. }) => {
                geo.head = area;
                geo.control = Rect::new(
                    area.right() - BUTTON_W - 10,
                    area.y + (area.h - BUTTON_H) / 2,
                    BUTTON_W,
                    BUTTON_H,
                );
            }
        }
        geo
    }

    /// The row a touch at (x, y) changes, and how.
    fn hit(&self, g: &Geo, x: i32, y: i32) -> Option<EventKind> {
        if g.back.contains(x, y) {
            return Some(EventKind::Tab(BACK));
        }
        if g.reset.is_some_and(|r| r.contains(x, y)) {
            return Some(EventKind::Tab(RESET));
        }
        if let Some(i) = g.tabs.iter().position(|t| t.contains(x, y)) {
            let tab = self.view.tabs[i].tab;
            return (tab != self.view.tab).then_some(EventKind::Tab(tab));
        }
        if let Some(gal) = &self.view.gallery {
            if g.new_theme.is_some_and(|r| r.contains(x, y)) {
                return Some(EventKind::Tab(NEW_THEME));
            }
            if g.customise.is_some_and(|r| r.contains(x, y)) {
                return Some(EventKind::Tab(CUSTOMISE));
            }
            if let Some((keep, back)) = g.keep {
                if keep.contains(x, y) {
                    return Some(EventKind::Tab(KEEP));
                }
                if back.contains(x, y) {
                    return Some(EventKind::Tab(GO_BACK));
                }
            }
            return g
                .tiles
                .iter()
                .find(|(_, r)| r.contains(x, y))
                .filter(|(k, _)| *k != gal.at)
                .map(|(k, _)| EventKind::Cell { row: 0, col: *k });
        }
        for (i, row) in self.view.rows.iter().enumerate() {
            let geo = &g.rows[i];
            match &row.control {
                Some(Control::Choice { at, .. })
                | Some(Control::Tiles { at, .. })
                | Some(Control::Looks { at, .. }) => {
                    if let Some(k) = geo.parts.iter().position(|p| p.contains(x, y)) {
                        return (k != *at).then_some(EventKind::Cell { row: i, col: k });
                    }
                }
                Some(Control::Swatches { .. }) => {
                    if let Some(k) = geo.parts.iter().position(|p| p.contains(x, y)) {
                        return Some(EventKind::Cell { row: i, col: k });
                    }
                }
                Some(Control::Stepper(_)) => {
                    if let Some(k) = geo.parts.iter().position(|p| p.contains(x, y)) {
                        let delta = if k == 0 { -1 } else { 1 };
                        return Some(EventKind::Step { index: i, delta });
                    }
                }
                Some(Control::Steps { .. }) if geo.control.contains(x, y) => {
                    return Some(EventKind::Cell {
                        row: i,
                        col: nearest(&geo.parts, x),
                    });
                }
                Some(Control::Slider { .. }) if geo.control.contains(x, y) => {
                    return Some(slider_value(i, geo.control, x));
                }
                Some(Control::Switch(on)) if geo.control.contains(x, y) => {
                    let delta = if *on { -1 } else { 1 };
                    return Some(EventKind::Step { index: i, delta });
                }
                Some(Control::Button(_)) | Some(Control::Status { .. })
                    if geo.control.contains(x, y) =>
                {
                    return Some(EventKind::Step { index: i, delta: 1 });
                }
                _ => {}
            }
        }
        None
    }

    // ------------------------------------------------------------ paint

    fn paint_side(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        cx.panel(g.side);
        cx.canvas.fill_round_rect(g.back, t.round(10), t.raised);
        cx.text(
            FontId::Bold,
            t.size_body,
            g.back,
            Align::Center,
            t.text,
            "‹ Back",
        );
        let title = Rect::new(
            g.back.right(),
            g.back.y,
            g.side.right() - 16 - g.back.right(),
            g.back.h,
        );
        cx.text(
            FontId::Bold,
            t.size_small,
            title,
            Align::Right,
            t.text_dim,
            "PREFS",
        );
        for (i, tab) in self.view.tabs.iter().enumerate() {
            let r = g.tabs[i];
            if tab.sub {
                // One of Appearance's views: its name, indented, lit
                // while open.
                let open = tab.tab == self.view.tab;
                if open {
                    cx.canvas.fill_round_rect(r, t.round(10), t.raised);
                }
                let words = Rect::new(r.x + SUB_INDENT, r.y, r.w - SUB_INDENT - 8, r.h);
                let (font, ink) = if open {
                    (FontId::Bold, t.text)
                } else {
                    (FontId::Sans, t.text_dim)
                };
                cx.text(font, t.size_body, words, Align::Left, ink, &tab.name);
                continue;
            }
            let on = tab.lit;
            if on {
                cx.canvas.fill_round_rect(r, t.round(10), t.raised);
                let bar = Rect::new(r.x, r.y + 10, 4, r.h - 20);
                cx.canvas.fill_round_rect(bar, 2, t.accent);
            }
            let (name, sub) = (&tab.name, &tab.line);
            let icon = Rect::new(r.x + 14, r.y + (r.h - 24) / 2, 24, 24);
            paint_tab_icon(cx, tab.icon, icon, if on { t.accent } else { t.text_dim });
            let words = Rect::new(
                icon.right() + 14,
                r.y + 8,
                r.right() - icon.right() - 20,
                r.h - 16,
            );
            let (top, bottom) = words.take_top(26);
            cx.text(FontId::Sans, t.size_body, top, Align::Left, t.text, name);
            cx.text(
                FontId::Sans,
                t.size_small,
                bottom,
                Align::Left,
                t.text_dim,
                sub,
            );
        }
        // Reset, at the sidebar's foot, under what Prefs are.
        if let (Some(r), Some(words)) = (g.reset, &self.view.reset) {
            for (k, line) in RESET_LINE.iter().enumerate() {
                let at = Rect::new(r.x + 4, r.y - 52 + 20 * k as i32, r.w - 8, 20);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    at,
                    Align::Left,
                    t.text_dim,
                    line,
                );
            }
            cx.canvas.fill_round_rect(r, t.round(10), t.raised);
            cx.text(FontId::Bold, t.size_body, r, Align::Center, t.text, words);
        }
    }

    fn paint_cards(&self, cx: &mut PaintCx, g: &Geo) {
        let t = cx.theme.clone();
        for (k, card) in self.view.cards.iter().enumerate() {
            let r = g.cards[k];
            cx.panel(r);
            for d in g.dividers.iter().filter(|d| r.contains(d.x, d.y)) {
                cx.canvas.fill_rect(*d, t.outline);
            }
            let head = Rect::new(
                r.x + CARD_PAD_X,
                r.y + CARD_PAD_TOP,
                r.w - 2 * CARD_PAD_X,
                HEAD_H,
            );
            let hue = t.readable(card.hue.color(&t), t.panel());
            if card.title.is_empty() {
                for &i in &card.rows {
                    self.paint_row(cx, g, i, card.hue);
                }
                continue;
            }
            cx.text(
                FontId::Bold,
                t.size_small,
                head,
                Align::Left,
                hue,
                &card.title.to_uppercase(),
            );
            if !card.note.is_empty() {
                let w = cx.measure(FontId::Bold, t.size_small, &card.title.to_uppercase());
                let note = Rect::new(head.x + w + 16, head.y, head.w - w - 16, head.h);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    note,
                    Align::Right,
                    t.text_dim,
                    &card.note,
                );
            }
            for &i in &card.rows {
                self.paint_row(cx, g, i, card.hue);
            }
            if let Some(area) = g.info[k] {
                self.paint_info(cx, area);
            }
            if let (Some(area), Some(split)) = (g.split[k], &card.split) {
                paint_split(cx, area, split);
            }
        }
    }

    fn paint_row(&self, cx: &mut PaintCx, g: &Geo, i: usize, hue: Hue) {
        let t = cx.theme.clone();
        let row = &self.view.rows[i];
        let geo = &g.rows[i];
        let fill = hue.color(&t);
        if i == self.view.selected {
            let o = Rect::new(
                geo.area.x - 8,
                geo.area.y - 6,
                geo.area.w + 16,
                geo.area.h + 12,
            );
            if self.view.editing {
                cx.canvas
                    .fill_round_rect(o, t.round(12), t.panel().mix(t.accent, 0.10));
            }
            let width = if self.view.editing { 3 } else { 2 };
            cx.canvas.stroke_round_rect(o, t.round(12), width, t.accent);
        }
        let ink = |c: Color| if row.faded { t.faded(c, t.panel()) } else { c };
        if let Some(Control::Dial { .. }) = row.control {
            // The value over the name, under the knob.
            let (value, name) = geo.head.take_top(NAME_H);
            cx.text(
                FontId::Bold,
                t.size_body,
                value,
                Align::Center,
                ink(t.text),
                &row.value,
            );
            let name_ink = if row.faded {
                faded_small(&t, t.text_dim)
            } else {
                t.text_dim
            };
            cx.text(
                FontId::Sans,
                t.size_small,
                name,
                Align::Center,
                name_ink,
                &row.name,
            );
        } else if !row.bare && !matches!(row.control, Some(Control::Status { .. })) {
            let knob = matches!(row.control, Some(Control::Knob { .. }));
            let (name, desc) = geo.head.take_top(NAME_H);
            let (value_line, desc) = if knob {
                desc.take_top(NAME_H)
            } else {
                (Rect::default(), desc)
            };
            let value_w = if row.value.is_empty() || knob {
                0
            } else {
                cx.measure(FontId::Bold, t.size_body, &row.value) + 12
            };
            let name_r = Rect::new(name.x, name.y, name.w - value_w, name.h);
            cx.text(
                FontId::Sans,
                t.size_body,
                name_r,
                Align::Left,
                ink(t.text),
                &row.name,
            );
            if value_w > 0 {
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    name,
                    Align::Right,
                    ink(t.text),
                    &row.value,
                );
            }
            if knob {
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    value_line,
                    Align::Left,
                    ink(t.text),
                    &row.value,
                );
            }
            if !row.desc.is_empty() {
                let d = Rect::new(desc.x, desc.y, desc.w, DESC_H);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    d,
                    Align::Left,
                    ink(t.text_dim),
                    &row.desc,
                );
            }
        }
        match (geo.picture, &row.picture) {
            (Some(r), Some(Picture::Curve(pts, hit))) => paint_curve(cx, r, pts, *hit, fill),
            (Some(r), Some(Picture::Pads(colours))) => paint_pads(cx, r, colours),
            _ => {}
        }
        let Some(control) = &row.control else {
            return;
        };
        match control {
            Control::Choice { options, at, now } => {
                for (k, (label, sub)) in options.iter().enumerate() {
                    let p = geo.parts[k];
                    let chosen = k == *at;
                    let bg = if chosen { fill } else { t.raised };
                    cx.canvas.fill_round_rect(p, t.round(10), bg);
                    if *now == Some(k) && !chosen {
                        cx.canvas
                            .stroke_round_rect(p, t.round(10), 2, t.readable(fill, t.raised));
                    }
                    let text = if chosen { t.ink_on(fill) } else { ink(t.text) };
                    let font = if chosen { FontId::Bold } else { FontId::Sans };
                    if sub.is_empty() {
                        cx.text(
                            font,
                            t.size_body,
                            p.inset_xy(6, 0),
                            Align::Center,
                            text,
                            label,
                        );
                    } else {
                        // The name and its line together, in the middle.
                        let block = Rect::new(p.x + 6, p.y + (p.h - 42) / 2, p.w - 12, 42);
                        let (top, bottom) = block.take_top(24);
                        cx.text(font, t.size_body, top, Align::Center, text, label);
                        let dim = if chosen {
                            t.ink_on(fill)
                        } else {
                            ink(t.text_dim)
                        };
                        cx.text(
                            FontId::Sans,
                            t.size_small - 1.0,
                            bottom,
                            Align::Center,
                            dim,
                            sub,
                        );
                    }
                }
            }
            Control::Tiles { options, at } => {
                for (k, (title, what)) in options.iter().enumerate() {
                    let p = geo.parts[k];
                    let chosen = k == *at;
                    let bg = if chosen {
                        t.panel().mix(fill, 0.22)
                    } else {
                        t.raised
                    };
                    cx.canvas.fill_round_rect(p, t.round(12), bg);
                    if chosen {
                        cx.canvas
                            .stroke_round_rect(p, t.round(12), 2, t.readable(fill, bg));
                    }
                    // A round mark at the right, filled on the one chosen.
                    let mark = Rect::new(p.right() - 40, p.y + (p.h - 22) / 2, 22, 22);
                    cx.canvas.stroke_round_rect(
                        mark,
                        11,
                        2,
                        t.readable(if chosen { fill } else { t.text_dim }, bg),
                    );
                    if chosen {
                        cx.canvas
                            .fill_round_rect(mark.inset(5), 6, t.readable(fill, bg));
                    }
                    let inner = Rect::new(p.x + 18, p.y, mark.x - p.x - 30, p.h);
                    let lines = wrap(cx, FontId::Sans, t.size_small, what, inner.w);
                    // As many lines of its words as the tile has room for.
                    let room = ((p.h - 24 - 16) / 19).max(1) as usize;
                    let shown = lines.len().min(room) as i32;
                    let block = 24 + shown * 19;
                    let top = Rect::new(inner.x, p.y + (p.h - block) / 2, inner.w, 24);
                    cx.text(FontId::Bold, t.size_body, top, Align::Left, t.text, title);
                    for (n, line) in lines.iter().take(shown as usize).enumerate() {
                        let lr = Rect::new(inner.x, top.bottom() + n as i32 * 19, inner.w, 19);
                        cx.text(
                            FontId::Sans,
                            t.size_small,
                            lr,
                            Align::Left,
                            t.readable(t.text_dim, bg),
                            line,
                        );
                    }
                }
            }
            Control::Steps { labels, at } => {
                let c = geo.control;
                let track = Rect::new(c.x + 18, c.y, c.w - 36, SLIDE_H);
                let n = labels.len().max(2) as i32 - 1;
                let pos = *at as f32 / n as f32;
                paint_slider(cx, track, pos, fill);
                for (s, label) in labels.iter().enumerate() {
                    let x = geo.parts[s].x;
                    let lr = Rect::new(x - 40, c.y + SLIDE_H + 2, 80, LABELS_H);
                    let align = if s == 0 {
                        Align::Left
                    } else if s + 1 == labels.len() {
                        Align::Right
                    } else {
                        Align::Center
                    };
                    let lr = match align {
                        Align::Left => Rect::new(x - 18, lr.y, 80, lr.h),
                        Align::Right => Rect::new(x + 18 - 80, lr.y, 80, lr.h),
                        Align::Center => lr,
                    };
                    let (font, color) = if s == *at {
                        (FontId::Bold, ink(t.text))
                    } else {
                        (FontId::Sans, ink(t.text_dim))
                    };
                    cx.text(font, t.size_small, lr, align, color, label);
                }
            }
            Control::Slider { pos } => {
                let c = geo.control;
                let track = Rect::new(c.x + 18, c.y, c.w - 36, SLIDE_H);
                paint_slider(cx, track, *pos, fill);
            }
            Control::Meter => {
                let c = geo.control;
                let bar = Rect::new(c.x, c.y + (c.h - METER_H) / 2, c.w, METER_H);
                cx.canvas.fill_round_rect(bar, METER_H / 2, t.raised);
                if let Some(db) = self.meter {
                    // -48 dB to 0 along the bar: good, then warn past -12,
                    // bad past -3.
                    let at = ((db + 48.0) / 48.0).clamp(0.0, 1.0);
                    let w = (bar.w as f32 * at).round() as i32;
                    if w > 0 {
                        let ink = if db > -3.0 {
                            t.bad
                        } else if db > -12.0 {
                            t.warn
                        } else {
                            t.good
                        };
                        let lit = Rect::new(bar.x, bar.y, w.max(METER_H), bar.h);
                        cx.canvas.fill_round_rect(lit, METER_H / 2, ink);
                    }
                }
            }
            Control::Knob { pos } => {
                let c = geo.control;
                mpfree_synth_ui::widgets::knob(
                    cx,
                    c.x as f32 + c.w as f32 / 2.0,
                    c.y as f32 + c.h as f32 / 2.0,
                    c.w as f32,
                    *pos,
                    false,
                    t.raised,
                    fill,
                );
            }
            Control::Switch(on) => {
                let c = geo.control;
                let s = Rect::new(
                    c.right() - SWITCH_W,
                    c.y + (c.h - SWITCH_H) / 2,
                    SWITCH_W,
                    SWITCH_H,
                );
                let bg = if *on { fill } else { t.raised };
                cx.canvas.fill_round_rect(s, SWITCH_H / 2, bg);
                let d = SWITCH_H - 8;
                let knob = if *on {
                    Rect::new(s.right() - d - 4, s.y + 4, d, d)
                } else {
                    Rect::new(s.x + 4, s.y + 4, d, d)
                };
                let handle = if *on {
                    t.ink_on(fill)
                } else {
                    t.role(mpfree_ui::Role::Handle)
                };
                cx.canvas.fill_round_rect(knob, d / 2, handle);
            }
            Control::Button(label) => {
                let c = geo.control;
                let (bg, words) = if row.strong && !row.faded {
                    (t.picked(), t.on_picked())
                } else {
                    (t.raised, ink(t.text))
                };
                cx.canvas.fill_round_rect(c, t.round(10), bg);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    c.inset_xy(8, 0),
                    Align::Center,
                    words,
                    label,
                );
            }
            Control::Dial { pos, dot } => {
                let c = geo.control;
                let (x, y) = (c.x as f32 + c.w as f32 / 2.0, c.y as f32 + c.h as f32 / 2.0);
                match dot {
                    Some(colour) => paint_hue_ring(cx, c, *pos, *colour),
                    None => mpfree_synth_ui::widgets::knob(
                        cx,
                        x,
                        y,
                        c.w as f32,
                        *pos,
                        false,
                        t.raised,
                        if row.faded {
                            t.faded(fill, t.panel())
                        } else {
                            fill
                        },
                    ),
                }
            }
            Control::Swatches {
                colours,
                at,
                numbered,
            } => {
                let inline = inline_swatches(row, colours.len());
                for (k, (word, colour)) in colours.iter().enumerate() {
                    let p = geo.parts[k];
                    cx.canvas
                        .fill_round_rect(p, t.round(8), ink(colour.with_alpha(255)));
                    cx.canvas.stroke_round_rect(p, t.round(8), 1, t.outline);
                    if *at == Some(k) {
                        let ring = p.inset(-4);
                        cx.canvas.stroke_round_rect(ring, t.round(10), 3, t.accent);
                    }
                    if *numbered {
                        let nr = Rect::new(p.x + 6, p.bottom() - 20, p.w - 8, 18);
                        cx.text(
                            FontId::Bold,
                            t.size_small - 1.0,
                            nr,
                            Align::Left,
                            t.ink_on(*colour),
                            word,
                        );
                    } else if !inline {
                        let lr = Rect::new(p.x - 4, p.bottom() + 2, p.w + 8, SWATCH_LABEL_H);
                        cx.text(
                            FontId::Sans,
                            t.size_small - 1.0,
                            lr,
                            Align::Center,
                            ink(t.text_dim),
                            word,
                        );
                    }
                }
            }
            Control::Looks { options, at } => {
                for (k, (label, look)) in options.iter().enumerate() {
                    let p = geo.parts[k];
                    let chosen = k == *at;
                    let bg = if chosen { fill } else { t.raised };
                    cx.canvas.fill_round_rect(p, t.round(10), bg);
                    let words = if chosen { t.ink_on(fill) } else { ink(t.text) };
                    let (pic, name) = p.inset(8).take_top(p.h - 16 - 26);
                    paint_look(cx, pic, *look, words);
                    let font = if chosen { FontId::Bold } else { FontId::Sans };
                    cx.text(font, t.size_body, name, Align::Center, words, label);
                }
            }
            Control::Stepper(label) => {
                let c = geo.control;
                cx.canvas.fill_round_rect(c, t.round(10), t.raised);
                let arrows = c.inset_xy(18, 0);
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    arrows,
                    Align::Left,
                    ink(t.text_dim),
                    "‹",
                );
                cx.text(
                    FontId::Bold,
                    t.size_title,
                    arrows,
                    Align::Right,
                    ink(t.text_dim),
                    "›",
                );
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    c.inset_xy(40, 0),
                    Align::Center,
                    ink(t.text),
                    label,
                );
            }
            Control::Status {
                title,
                line,
                tone,
                button,
            } => {
                let a = geo.area;
                let color = t.tone(*tone);
                let strip = t.panel().mix(color, 0.12);
                cx.canvas.fill_round_rect(a, t.round(12), strip);
                cx.canvas
                    .stroke_round_rect(a, t.round(12), 1, t.panel().mix(color, 0.35));
                let dot = Rect::new(a.x + 16, a.y + 17, 12, 12);
                cx.canvas.fill_round_rect(dot, 6, color);
                let words = Rect::new(
                    dot.right() + 12,
                    a.y + 8,
                    geo.control.x - dot.right() - 20,
                    a.h - 16,
                );
                let (top, bottom) = words.take_top(26);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    top,
                    Align::Left,
                    t.readable(t.text, strip),
                    title,
                );
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    bottom,
                    Align::Left,
                    t.readable(t.text_dim, strip),
                    line,
                );
                let c = geo.control;
                cx.canvas.fill_round_rect(c, t.round(10), t.raised);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    c.inset_xy(8, 0),
                    Align::Center,
                    t.text,
                    button,
                );
            }
        }
    }

    fn paint_info(&self, cx: &mut PaintCx, area: Rect) {
        let t = cx.theme.clone();
        let lines = self.info.len().max(1) as i32;
        let h = (area.h / lines).clamp(INFO_H, INFO_H + 12);
        if self.info.is_empty() {
            let r = Rect::new(area.x, area.y, area.w, INFO_H);
            cx.text(
                FontId::Sans,
                t.size_small,
                r,
                Align::Left,
                t.text_dim,
                "Read when the tab opens",
            );
            return;
        }
        for (n, (k, v)) in self.info.iter().enumerate() {
            let y = area.y + n as i32 * h;
            if y + h > area.bottom() + 1 {
                break;
            }
            let key = Rect::new(area.x, y, 150, h);
            let val = Rect::new(area.x + 158, y, area.w - 158, h);
            cx.text(FontId::Sans, t.size_small, key, Align::Left, t.text_dim, k);
            cx.text(FontId::Sans, t.size_small, val, Align::Left, t.text, v);
            if n + 1 < self.info.len() {
                cx.canvas
                    .fill_rect(Rect::new(area.x, y + h - 1, area.w, 1), t.outline);
            }
        }
    }

    fn paint_gallery(&self, cx: &mut PaintCx, g: &Geo, gal: &Gallery) {
        let t = cx.theme.clone();
        for &(k, r) in &g.tiles {
            let tile = &gal.tiles[k];
            let on = k == gal.at;
            cx.panel(r);
            if on {
                cx.canvas.stroke_round_rect(r, t.radius, 3, t.accent);
            }
            let inner = r.inset(10);
            let (pic, foot) = inner.take_top(inner.h - 52);
            if let Some(look) = self.look(&tile.key) {
                paint_theme_preview(cx, pic, &look);
                let (name_r, sw) = foot.take_top(30);
                let label_w =
                    name_r.w - if on { 80 } else { 0 } - if tile.problems > 0 { 40 } else { 0 };
                let name_lr = Rect::new(name_r.x + 2, name_r.y + 4, label_w, name_r.h - 4);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    name_lr,
                    Align::Left,
                    t.text,
                    &tile.label,
                );
                if on {
                    let tag = Rect::new(name_r.right() - 76, name_r.y + 8, 74, 22);
                    cx.canvas.fill_round_rect(tag, 6, t.picked());
                    cx.text(
                        FontId::Bold,
                        t.size_small - 2.0,
                        tag,
                        Align::Center,
                        t.on_picked(),
                        "IN USE",
                    );
                }
                if tile.problems > 0 {
                    let x = name_r.right() - if on { 116 } else { 36 };
                    let badge = Rect::new(x, name_r.y + 8, 32, 22);
                    cx.canvas.fill_round_rect(badge, 6, t.warn);
                    cx.text(
                        FontId::Bold,
                        t.size_small - 1.0,
                        badge,
                        Align::Center,
                        t.ink_on(t.warn),
                        &tile.problems.to_string(),
                    );
                }
                let sw = Rect::new(sw.x + 2, sw.y + 8, sw.w - 4, 10);
                let w = (sw.w - 7 * 4) / 8;
                for c in 0..8 {
                    let s = Rect::new(sw.x + c * (w + 4), sw.y, w, sw.h);
                    cx.canvas.fill_round_rect(s, 3, look.track(c as usize));
                }
            }
        }
        if let Some(r) = g.new_theme {
            // A copy of the theme in use, opened in Customise.
            let mut s = mpfree_ui::Shape::new(r);
            let (x0, y0) = (r.x as f32 + 1.5, r.y as f32 + 1.5);
            let (x1, y1) = (r.right() as f32 - 1.5, r.bottom() as f32 - 1.5);
            s.dashed(
                &[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)],
                2.0,
                8.0,
            );
            cx.canvas.blend_shape(&s, t.outline.mix(t.text_dim, 0.5));
            let mid = r.y + r.h / 2;
            let plus = Rect::new(r.x, mid - 60, r.w, 40);
            cx.text(FontId::Sans, 36.0, plus, Align::Center, t.text_dim, "+");
            let name = Rect::new(r.x, mid - 14, r.w, 26);
            cx.text(
                FontId::Bold,
                t.size_body,
                name,
                Align::Center,
                t.text,
                "New theme",
            );
            for (n, line) in ["A copy of the theme in use,", "opened in Customise"]
                .iter()
                .enumerate()
            {
                let lr = Rect::new(r.x + 8, mid + 18 + n as i32 * 19, r.w - 16, 19);
                cx.text(
                    FontId::Sans,
                    t.size_small,
                    lr,
                    Align::Center,
                    t.text_dim,
                    line,
                );
            }
        }
        // The theme in use: its name, what it is, how well it reads, and
        // "Keep this theme?" while it asks.
        let r = g.in_use;
        cx.panel(r);
        let inner = r.inset_xy(22, 18);
        let words_w = if g.keep.is_some() || g.customise.is_some() {
            inner.w - 200
        } else {
            inner.w
        };
        let words = Rect::new(inner.x, inner.y, words_w, inner.h);
        let (cap, rest) = words.take_top(20);
        let (heading, rest) = rest.take_top(36);
        match &gal.trial {
            Some((ask, line)) => {
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    cap,
                    Align::Left,
                    t.warn,
                    "TRYING A THEME",
                );
                cx.text(
                    FontId::Bold,
                    t.size_title + 3.0,
                    heading,
                    Align::Left,
                    t.text,
                    ask,
                );
                let lr = Rect::new(rest.x, rest.y + 2, rest.w, 22);
                cx.text(FontId::Sans, t.size_body, lr, Align::Left, t.text_dim, line);
            }
            None => {
                let name = gal.tiles.get(gal.at).map_or("", |tl| tl.label.as_str());
                cx.text(
                    FontId::Bold,
                    t.size_small,
                    cap,
                    Align::Left,
                    t.text_dim,
                    "IN USE",
                );
                cx.text(
                    FontId::Bold,
                    t.size_title + 3.0,
                    heading,
                    Align::Left,
                    t.text,
                    name,
                );
                let mut y = rest.y + 2;
                for line in gal.about.iter().take(3) {
                    let lr = Rect::new(rest.x, y, rest.w, 22);
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        lr,
                        Align::Left,
                        t.text_dim,
                        line,
                    );
                    y += 22;
                }
            }
        }
        // How well the theme in use reads: words, dim words and the
        // accent on a panel (4.5:1 for words, 3:1 for the rest).
        let panel = t.panel();
        let marks = [
            ("Words", contrast(t.text, panel), 4.5),
            ("Dim", contrast(t.text_dim, panel), 3.0),
            ("Accent", contrast(t.accent, panel), 3.0),
        ];
        let mut x = words.x;
        let y = words.bottom() - 22;
        for (what, ratio, min) in marks {
            let tone = if ratio >= min { t.good } else { t.warn };
            let s = format!("● {what} {ratio:.1}:1");
            let w = cx.measure(FontId::Sans, t.size_small, &s);
            cx.text(
                FontId::Sans,
                t.size_small,
                Rect::new(x, y, w + 4, 22),
                Align::Left,
                t.readable(tone, panel),
                &s,
            );
            x += w + 22;
        }
        if let Some(c) = g.customise {
            cx.canvas.fill_round_rect(c, t.round(10), t.picked());
            cx.text(
                FontId::Bold,
                t.size_body,
                c,
                Align::Center,
                t.on_picked(),
                "Customise ›",
            );
        }
        if let Some((keep, back)) = g.keep {
            cx.canvas.fill_round_rect(keep, t.round(10), t.picked());
            cx.text(
                FontId::Bold,
                t.size_body,
                keep,
                Align::Center,
                t.on_picked(),
                "Keep",
            );
            cx.canvas.fill_round_rect(back, t.round(10), t.raised);
            cx.text(
                FontId::Bold,
                t.size_body,
                back,
                Align::Center,
                t.text,
                "Go back",
            );
        }
    }
}

impl Widget for PrefsPage {
    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let g = self.geo(r);
        self.paint_side(cx, &g);
        match &self.view.gallery {
            Some(gal) => self.paint_gallery(cx, &g, gal),
            None => self.paint_cards(cx, &g),
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let g = self.geo(r);
        let mut out = vec![Target::tap("back", 0, "Back", g.back)];
        if let (Some(r), Some(words)) = (g.reset, &self.view.reset) {
            out.push(Target::tap("reset", 0, words.clone(), r));
        }
        for (i, tab) in self.view.tabs.iter().enumerate() {
            out.push(
                Target::tap("tab", tab.tab, tab.name.clone(), g.tabs[i])
                    .disabled_if(tab.tab == self.view.tab),
            );
        }
        if let Some(gal) = &self.view.gallery {
            for &(k, rect) in &g.tiles {
                out.push(
                    Target::tap("theme", k, gal.tiles[k].label.clone(), rect)
                        .disabled_if(k == gal.at),
                );
            }
            if let Some(r) = g.new_theme {
                out.push(Target::tap("new", 0, "New theme", r));
            }
            if let Some(r) = g.customise {
                out.push(Target::tap("customise", 0, "Customise", r));
            }
            if let Some((keep, back)) = g.keep {
                out.push(Target::tap("keep", 0, "Keep", keep));
                out.push(Target::tap("keep", 1, "Go back", back));
            }
            return out;
        }
        for (i, row) in self.view.rows.iter().enumerate() {
            let geo = &g.rows[i];
            match &row.control {
                Some(Control::Choice { options, at, .. }) => {
                    for (k, (label, _)) in options.iter().enumerate() {
                        out.push(
                            Target::tap("choice", i * 100 + k, label.clone(), geo.parts[k])
                                .disabled_if(k == *at),
                        );
                    }
                }
                Some(Control::Tiles { options, at }) => {
                    for (k, (label, _)) in options.iter().enumerate() {
                        out.push(
                            Target::tap("tile", i * 100 + k, label.clone(), geo.parts[k])
                                .disabled_if(k == *at),
                        );
                    }
                }
                Some(Control::Steps { .. }) => {
                    out.push(
                        Target::drag("steps", i, row.name.clone(), geo.control).towards(80, 0),
                    );
                }
                Some(Control::Slider { .. }) => {
                    out.push(
                        Target::drag("slider", i, row.name.clone(), geo.control).towards(80, 0),
                    );
                }
                Some(Control::Knob { .. }) => {
                    out.push(
                        Target::drag("knob", i, row.name.clone(), geo.control).towards(0, -40),
                    );
                }
                Some(Control::Dial { .. }) => {
                    out.push(
                        Target::drag("dial", i, row.name.clone(), geo.control).towards(0, -40),
                    );
                }
                Some(Control::Swatches { colours, .. }) => {
                    for (k, (word, _)) in colours.iter().enumerate() {
                        let name = format!("{} {word}", row.name);
                        out.push(Target::tap("swatch", i * 100 + k, name, geo.parts[k]));
                    }
                }
                Some(Control::Looks { options, at }) => {
                    for (k, (label, _)) in options.iter().enumerate() {
                        out.push(
                            Target::tap("look", i * 100 + k, label.clone(), geo.parts[k])
                                .disabled_if(k == *at),
                        );
                    }
                }
                Some(Control::Stepper(_)) => {
                    for (k, word) in ["back", "on"].iter().enumerate() {
                        let name = format!("{} {word}", row.name);
                        out.push(Target::tap("stepper", i * 100 + k, name, geo.parts[k]));
                    }
                }
                Some(Control::Switch(_)) => {
                    out.push(Target::tap("switch", i, row.name.clone(), geo.control));
                }
                Some(Control::Button(label)) => {
                    // The touch checks keep their names, so a walk knows
                    // to leave them be (smoke/walk.rs).
                    let name = if label == "Start" || label == "Open" {
                        row.name.clone()
                    } else {
                        label.clone()
                    };
                    out.push(Target::tap("button", i, name, geo.control));
                }
                Some(Control::Status { button, .. }) => {
                    out.push(Target::tap("button", i, button.clone(), geo.control));
                }
                None | Some(Control::Meter) => {}
            }
        }
        out
    }

    /// A finger resting on a setting puts it under the wheel (hold and
    /// turn, Jay 17 Nov): the wheel and - / + then step it as they step
    /// the picked row. Not a row that only opens something.
    fn held(&self, x: i32, y: i32, r: Rect) -> Option<Target> {
        if self.view.gallery.is_some() {
            return None;
        }
        let g = self.geo(r);
        let i = (0..self.view.rows.len()).find(|&i| g.rows[i].area.contains(x, y))?;
        let row = &self.view.rows[i];
        let turns = matches!(
            row.control,
            Some(
                Control::Choice { .. }
                    | Control::Tiles { .. }
                    | Control::Steps { .. }
                    | Control::Slider { .. }
                    | Control::Knob { .. }
                    | Control::Dial { .. }
                    | Control::Swatches { .. }
                    | Control::Looks { .. }
                    | Control::Stepper(_)
                    | Control::Switch(_)
            )
        );
        (turns && !row.faded).then(|| Target::tap("row", i, row.name.clone(), g.rows[i].area))
    }

    fn held_step(&mut self, t: &Target, delta: i32, _fine: bool, _r: Rect) -> Option<EventKind> {
        (t.index < self.view.rows.len() && delta != 0).then_some(EventKind::Step {
            index: t.index,
            delta,
        })
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> Option<EventKind> {
        let g = self.geo(r);
        match phase {
            Touch::Down => {
                // A knob turns with the finger's move up or down.
                if self.view.gallery.is_none() {
                    for (i, row) in self.view.rows.iter().enumerate() {
                        if let Some(Control::Knob { pos } | Control::Dial { pos, .. }) = row.control
                        {
                            if g.rows[i].control.contains(x, y) {
                                self.drag = Some((finger, i, y, pos));
                                return None;
                            }
                        }
                    }
                }
                let ev = self.hit(&g, x, y);
                if ev == Some(EventKind::Tab(RESET)) {
                    self.resetting = Some(finger);
                    return None;
                }
                self.slide = match ev {
                    Some(EventKind::Values { index, .. }) => Some((finger, index)),
                    Some(EventKind::Cell { row, .. })
                        if matches!(
                            self.view.rows.get(row).and_then(|r| r.control.as_ref()),
                            Some(Control::Steps { .. })
                        ) =>
                    {
                        Some((finger, row))
                    }
                    _ => None,
                };
                ev
            }
            Touch::Move => {
                if let Some((f, i, y0, pos0)) = self.drag {
                    if f == finger {
                        let pos = (pos0 + (y0 - y) as f32 / 200.0).clamp(0.0, 1.0);
                        return Some(EventKind::Values {
                            index: i,
                            set: vec![(0, (pos * 1000.0).round() as i32)],
                        });
                    }
                }
                // A slider follows the finger that went down on it.
                let (f, i) = self.slide?;
                if f != finger || i >= g.rows.len() {
                    return None;
                }
                let c = g.rows[i].control;
                match self.view.rows[i].control {
                    Some(Control::Slider { .. }) => Some(slider_value(i, c, x)),
                    Some(Control::Steps { at, .. }) => {
                        let col = nearest(&g.rows[i].parts, x);
                        (col != at).then_some(EventKind::Cell { row: i, col })
                    }
                    _ => None,
                }
            }
            Touch::Up | Touch::Cancel => {
                if self.drag.is_some_and(|d| d.0 == finger) {
                    self.drag = None;
                }
                if self.slide.is_some_and(|d| d.0 == finger) {
                    self.slide = None;
                }
                if self.resetting == Some(finger) {
                    self.resetting = None;
                    let on = g.reset.is_some_and(|r| r.contains(x, y));
                    return (phase == Touch::Up && on).then_some(EventKind::Tab(RESET));
                }
                None
            }
        }
    }
}

// ------------------------------------------------------------ helpers

/// The height of a row's name and its line over its control.
/// Small faded text: `Theme::faded`, with room over its 3:1 for the
/// soft edges of small glyphs, which read lighter than their ink (a dial's
/// faded name measured 2.98:1 in Slate and 2.99:1 in AbleTown when faded
/// to exactly 3:1).
fn faded_small(t: &Theme, ink: Color) -> Color {
    let panel = t.panel();
    let mut c = t.faded(ink, panel);
    while c.a < ink.a && contrast(mpfree_ui::roles::over(c, panel), panel) < 3.3 {
        c = c.with_alpha(c.a.saturating_add(8).min(ink.a));
    }
    c
}

fn head_h(row: &RowView) -> i32 {
    if row.bare {
        0
    } else if row.desc.is_empty() {
        NAME_H
    } else {
        NAME_H + DESC_H
    }
}

/// A card's title and the gap under it (none for a card without one).
fn card_head(card: &CardView) -> i32 {
    if card.title.is_empty() {
        0
    } else {
        HEAD_H + HEAD_GAP
    }
}

/// A named row of a few colours draws them at the right of its name.
fn inline_swatches(row: &RowView, n: usize) -> bool {
    !row.bare && n <= 3
}

/// `r` cut into `n` equal parts side by side, `gap` apart.
fn split_row(r: Rect, n: usize, gap: i32) -> Vec<Rect> {
    let n = n.max(1) as i32;
    let w = (r.w - gap * (n - 1)) / n;
    (0..n)
        .map(|k| {
            let x = r.x + k * (w + gap);
            let w = if k == n - 1 { r.right() - x } else { w };
            Rect::new(x, r.y, w, r.h)
        })
        .collect()
}

/// The step whose place is nearest `x`.
fn nearest(parts: &[Rect], x: i32) -> usize {
    parts
        .iter()
        .enumerate()
        .min_by_key(|(_, p)| (p.x - x).abs())
        .map_or(0, |(k, _)| k)
}

/// A slider's value at `x`, as the event Prefs takes: row `i`, 0 to 1000.
fn slider_value(i: usize, c: Rect, x: i32) -> EventKind {
    let track = Rect::new(c.x + 18, c.y, c.w - 36, c.h);
    let pos = ((x - track.x) as f32 / track.w.max(1) as f32).clamp(0.0, 1.0);
    EventKind::Values {
        index: i,
        set: vec![(0, (pos * 1000.0).round() as i32)],
    }
}

/// `s` broken into lines no wider than `w`.
fn wrap(cx: &mut PaintCx, font: FontId, px: f32, s: &str, w: i32) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        let next = if line.is_empty() {
            word.to_string()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && cx.measure(font, px, &next) > w {
            out.push(std::mem::replace(&mut line, word.to_string()));
        } else {
            line = next;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// A slider: the track, filled to `pos` in `fill`, and the handle.
fn paint_slider(cx: &mut PaintCx, track: Rect, pos: f32, fill: Color) {
    let t = cx.theme.clone();
    let mid = track.y + track.h / 2;
    let bar = Rect::new(track.x, mid - 4, track.w, 8);
    cx.canvas.fill_round_rect(bar, 4, t.raised);
    let x = track.x + (track.w as f32 * pos.clamp(0.0, 1.0)).round() as i32;
    if x > bar.x {
        cx.canvas
            .fill_round_rect(Rect::new(bar.x, bar.y, x - bar.x, bar.h), 4, fill);
    }
    let handle = Rect::new(x - 18, mid - 11, 36, 22);
    cx.canvas
        .fill_round_rect(handle, 9, t.role(mpfree_ui::Role::Handle));
}

/// The velocity curve: a faint straight line, the curve over it and the
/// area under it tinted.
/// The 16 pads as a 4 by 4 grid of squares, pad 1 at the bottom left as
/// on the MPC, centred in `r` on a sunken panel, with "as they'd light".
fn paint_pads(cx: &mut PaintCx, r: Rect, colours: &[Color]) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(r, t.round(10), t.panel().mix(t.raised, 0.6));
    let words = 20;
    let inner = r.inset_xy(14, 12);
    let gap = 8;
    let side = ((inner.h - words - 3 * gap) / 4)
        .min((inner.w - 3 * gap) / 4)
        .max(8);
    let grid = 4 * side + 3 * gap;
    let x0 = inner.x + (inner.w - grid) / 2;
    let y0 = inner.y + (inner.h - words - grid) / 2;
    for (p, &c) in colours.iter().enumerate().take(16) {
        let (col, row) = ((p % 4) as i32, 3 - (p / 4) as i32);
        let cell = Rect::new(x0 + col * (side + gap), y0 + row * (side + gap), side, side);
        cx.canvas.fill_round_rect(cell, t.round(6), c);
        // An unlit pad still shows as a pad.
        cx.canvas.stroke_round_rect(cell, t.round(6), 1, t.outline);
    }
    let caption = Rect::new(inner.x, y0 + grid + 4, inner.w, words);
    cx.text(
        FontId::Sans,
        t.size_small,
        caption,
        Align::Center,
        t.text_dim,
        "as they'd light",
    );
}

fn paint_curve(cx: &mut PaintCx, r: Rect, pts: &[(f32, f32)], hit: Option<(u8, u8)>, fill: Color) {
    let t = cx.theme.clone();
    cx.canvas
        .fill_round_rect(r, t.round(10), t.panel().mix(t.raised, 0.6));
    let p = r.inset_xy(18, 14);
    let at = |(x, y): (f32, f32)| {
        (
            p.x as f32 + x * p.w as f32,
            p.bottom() as f32 - y * p.h as f32,
        )
    };
    let line = [at((0.0, 0.0)), at((1.0, 1.0))];
    let mut s = mpfree_ui::Shape::new(r);
    s.dashed(&line, 1.5, 6.0);
    cx.canvas.blend_shape(&s, t.text_faint);
    let pts: Vec<(f32, f32)> = pts.iter().map(|&q| at(q)).collect();
    let mut under = mpfree_ui::Shape::new(r);
    under.fill_under(&pts, p.bottom() as f32, 1.0);
    cx.canvas.blend_shape(&under, fill.with_alpha(46));
    let mut s = mpfree_ui::Shape::new(r);
    s.polyline(&pts, 3.0);
    cx.canvas.blend_shape(&s, fill);
    // The last hit: up from how hard it came, across to how loud it
    // played, and the two in words under "loud".
    if let Some((came, went)) = hit {
        let mark = at((f32::from(came) / 127.0, f32::from(went) / 127.0));
        let mut s = mpfree_ui::Shape::new(r);
        s.dashed(&[(mark.0, p.bottom() as f32), mark], 1.5, 4.0);
        s.dashed(&[(p.x as f32, mark.1), mark], 1.5, 4.0);
        cx.canvas.blend_shape(&s, t.warn);
        let mut dot = mpfree_ui::Shape::new(r);
        dot.disc(mark.0, mark.1, 6.0);
        cx.canvas.blend_shape(&dot, t.warn);
        cx.text(
            FontId::Sans,
            t.size_small,
            Rect::new(p.x + 8, p.y + 22, p.w, 20),
            Align::Left,
            t.text_dim,
            &format!("Last hit  {came} in → {went} out"),
        );
    }
    // Top left, where no curve's fill reaches: the top right is under
    // every curve's end.
    let words = Rect::new(p.x + 8, p.y, p.w, 20);
    cx.text(
        FontId::Sans,
        t.size_small,
        words,
        Align::Left,
        t.text_dim,
        "↑ loud",
    );
    let soft = Rect::new(p.x, p.bottom() - 20, p.w, 20);
    cx.text(
        FontId::Sans,
        t.size_small,
        soft,
        Align::Right,
        t.text_dim,
        "harder hits →",
    );
}

/// The split bar at a card's foot.
fn paint_split(cx: &mut PaintCx, area: Rect, split: &Split) {
    let t = cx.theme.clone();
    let (top, rest) = area.take_top(22);
    cx.text(
        FontId::Sans,
        t.size_small,
        top,
        Align::Left,
        t.text_dim,
        &split.title,
    );
    cx.text(
        FontId::Bold,
        t.size_body,
        top,
        Align::Right,
        t.text,
        &split.total,
    );
    let bar = Rect::new(rest.x, rest.y + 4, rest.w, 10);
    cx.canvas.fill_round_rect(bar, 5, t.raised);
    let mut x = bar.x;
    let shades = [t.text.mix(t.panel(), 0.15), t.text_dim];
    let mut legend = rest.x;
    for (n, (name, share)) in split.parts.iter().enumerate() {
        let w = (bar.w as f32 * share.clamp(0.0, 1.0)).round() as i32;
        let c = shades[n % shades.len()];
        if w > 0 {
            cx.canvas
                .fill_rect(Rect::new(x, bar.y, w.min(bar.right() - x), bar.h), c);
        }
        x += w;
        let dot = Rect::new(legend, bar.bottom() + 9, 8, 8);
        cx.canvas.fill_round_rect(dot, 4, c);
        let lw = cx.measure(FontId::Sans, t.size_small - 1.0, name) + 4;
        let lr = Rect::new(dot.right() + 6, bar.bottom() + 4, lw, 18);
        cx.text(
            FontId::Sans,
            t.size_small - 1.0,
            lr,
            Align::Left,
            t.text_dim,
            name,
        );
        legend = lr.right() + 16;
    }
}

/// A small picture of `look`: its page, a panel with words in its text,
/// dim text and accent, a picked button, in its own corners.
fn paint_theme_preview(cx: &mut PaintCx, r: Rect, look: &Theme) {
    let radius = cx.theme.round(8);
    cx.canvas.fill_round_rect(r, radius, look.bg);
    let bar = Rect::new(r.x + 10, r.y + 10, r.w - 20, 14);
    cx.canvas.fill_round_rect(bar, look.round(5), look.panel());
    cx.canvas
        .fill_round_rect(Rect::new(bar.x + 6, bar.y + 4, 30, 6), 3, look.accent);
    let card = Rect::new(
        r.x + 10,
        bar.bottom() + 8,
        r.w - 20,
        r.bottom() - bar.bottom() - 18,
    );
    cx.canvas.fill_round_rect(card, look.round(8), look.panel());
    cx.canvas
        .stroke_round_rect(card, look.round(8), 1, look.outline);
    let lines = [
        (look.text, 0.62),
        (look.text_dim, 0.44),
        (look.accent, 0.30),
    ];
    let mut y = card.y + 10;
    for (c, share) in lines {
        if y + 6 > card.bottom() - 26 {
            break;
        }
        let w = ((card.w - 24) as f32 * share) as i32;
        cx.canvas
            .fill_round_rect(Rect::new(card.x + 12, y, w, 6), 3, c);
        y += 13;
    }
    let pill = Rect::new(card.x + 12, card.bottom() - 22, 46, 14);
    if pill.y > card.y + 8 {
        cx.canvas
            .fill_round_rect(pill, look.round(6), look.picked());
        let other = Rect::new(pill.right() + 6, pill.y, 46, 14);
        cx.canvas.fill_round_rect(other, look.round(6), look.raised);
    }
}

/// A knob for a colour's hue: a ring of every hue round a dot of the
/// colour, with a handle on the ring at `pos` (0 at the top, clockwise).
fn paint_hue_ring(cx: &mut PaintCx, r: Rect, pos: f32, colour: Color) {
    let t = cx.theme.clone();
    let (x, y) = (r.x as f32 + r.w as f32 / 2.0, r.y as f32 + r.h as f32 / 2.0);
    let rad = r.w as f32 / 2.0 - 4.0;
    let width = (r.w as f32 / 9.0).clamp(4.0, 6.0);
    let steps = 36;
    let top = -std::f32::consts::FRAC_PI_2;
    for n in 0..steps {
        let a0 = top + n as f32 / steps as f32 * std::f32::consts::TAU;
        let a1 = top + (n + 1) as f32 / steps as f32 * std::f32::consts::TAU;
        let mut s = mpfree_ui::Shape::new(r.inset(-2));
        s.arc(x, y, rad, a0, a1 + 0.02, width);
        let hue = crate::theme_edit::from_hsl(
            crate::theme_edit::Hsl {
                h: n as f32 * 360.0 / steps as f32,
                s: 0.85,
                l: 0.6,
            },
            255,
        );
        cx.canvas.blend_shape(&s, hue);
    }
    let mut dot = mpfree_ui::Shape::new(r);
    dot.disc(x, y, rad - width - 5.0);
    cx.canvas.blend_shape(&dot, colour.with_alpha(255));
    let a = top + pos.clamp(0.0, 1.0) * std::f32::consts::TAU;
    let mut handle = mpfree_ui::Shape::new(r.inset(-6));
    handle.disc(x + rad * a.cos(), y + rad * a.sin(), width / 2.0 + 3.0);
    cx.canvas
        .blend_shape(&handle, t.role(mpfree_ui::Role::Handle));
}

/// The drawing on a [`Control::Looks`] button, in `ink`: a box with its
/// corners, or a little page with its backdrop.
fn paint_look(cx: &mut PaintCx, r: Rect, look: Look, ink: Color) {
    let t = cx.theme.clone();
    match look {
        Look::Corners(corners) => {
            let b = Rect::new(r.x + 6, r.y + (r.h - 30).max(0) / 2, r.w - 12, 30.min(r.h));
            let (x0, y0) = (b.x as f32 + 1.0, b.y as f32 + 1.0);
            let (x1, y1) = (b.right() as f32 - 1.0, b.bottom() as f32 - 1.0);
            let c = 9.0f32.min((y1 - y0) / 2.0);
            let mut s = mpfree_ui::Shape::new(b.inset(-2));
            match corners {
                mpfree_ui::Corners::Square => {
                    s.polyline(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)], 2.0)
                }
                mpfree_ui::Corners::Chamfer => s.polyline(
                    &[
                        (x0 + c, y0),
                        (x1 - c, y0),
                        (x1, y0 + c),
                        (x1, y1 - c),
                        (x1 - c, y1),
                        (x0 + c, y1),
                        (x0, y1 - c),
                        (x0, y0 + c),
                        (x0 + c, y0),
                    ],
                    2.0,
                ),
                mpfree_ui::Corners::Round => {
                    use std::f32::consts::{FRAC_PI_2, PI};
                    s.segment((x0 + c, y0), (x1 - c, y0), 2.0);
                    s.segment((x1, y0 + c), (x1, y1 - c), 2.0);
                    s.segment((x1 - c, y1), (x0 + c, y1), 2.0);
                    s.segment((x0, y1 - c), (x0, y0 + c), 2.0);
                    s.arc(x1 - c, y0 + c, c, -FRAC_PI_2, 0.0, 2.0);
                    s.arc(x1 - c, y1 - c, c, 0.0, FRAC_PI_2, 2.0);
                    s.arc(x0 + c, y1 - c, c, FRAC_PI_2, PI, 2.0);
                    s.arc(x0 + c, y0 + c, c, PI, PI + FRAC_PI_2, 2.0);
                }
            }
            cx.canvas.blend_shape(&s, ink);
        }
        Look::Backdrop(backdrop) => {
            use mpfree_ui::Backdrop;
            let page = r.inset_xy(4, 2);
            let (top, foot) = (t.bg, t.backdrop_to);
            let line = t.backdrop_line.mix(t.text_dim, 0.35);
            let fade = !matches!(backdrop, Backdrop::Solid);
            let bands = 6;
            for n in 0..bands {
                let y = page.y + page.h * n / bands;
                let h = page.y + page.h * (n + 1) / bands - y;
                let c = if fade {
                    top.mix(foot, n as f32 / (bands - 1) as f32)
                } else {
                    top
                };
                cx.canvas
                    .fill_rect(Rect::new(page.x, y, page.w, h), c.with_alpha(255));
            }
            let step = 8;
            match backdrop {
                Backdrop::Grid => {
                    for x in (page.x + step..page.right()).step_by(step as usize) {
                        cx.canvas.fill_rect(Rect::new(x, page.y, 1, page.h), line);
                    }
                    for y in (page.y + step..page.bottom()).step_by(step as usize) {
                        cx.canvas.fill_rect(Rect::new(page.x, y, page.w, 1), line);
                    }
                }
                Backdrop::Dots => {
                    for x in (page.x + step / 2..page.right()).step_by(step as usize) {
                        for y in (page.y + step / 2..page.bottom()).step_by(step as usize) {
                            cx.canvas.fill_rect(Rect::new(x, y, 2, 2), line);
                        }
                    }
                }
                Backdrop::Scanlines => {
                    for y in (page.y + 2..page.bottom()).step_by(4) {
                        cx.canvas.fill_rect(Rect::new(page.x, y, page.w, 1), line);
                    }
                }
                _ => {}
            }
            cx.canvas
                .stroke_round_rect(page, t.round(6), 1, ink.with_alpha(140));
        }
    }
}

/// Each tab's little drawing, in the sidebar (Playing, Sound, Appearance,
/// Projects, System).
fn paint_tab_icon(cx: &mut PaintCx, tab: usize, r: Rect, c: Color) {
    let mut s = mpfree_ui::Shape::new(r.inset(-2));
    let (x, y) = (r.x as f32, r.y as f32);
    let p = |a: f32, b: f32| (x + a, y + b);
    match tab {
        0 => {
            for (a, b) in [(3.0, 3.0), (14.0, 3.0), (3.0, 14.0), (14.0, 14.0)] {
                s.polyline(
                    &[
                        p(a, b),
                        p(a + 7.0, b),
                        p(a + 7.0, b + 7.0),
                        p(a, b + 7.0),
                        p(a, b),
                    ],
                    2.0,
                );
            }
        }
        1 => s.polyline(
            &[
                p(3.0, 12.0),
                p(5.0, 12.0),
                p(7.0, 6.0),
                p(10.0, 18.0),
                p(13.0, 9.0),
                p(15.0, 15.0),
                p(17.0, 12.0),
                p(21.0, 12.0),
            ],
            2.0,
        ),
        2 => {
            s.arc(x + 12.0, y + 12.0, 9.0, 0.0, std::f32::consts::TAU, 2.0);
            s.arc(
                x + 12.0,
                y + 12.0,
                4.5,
                -std::f32::consts::FRAC_PI_2,
                std::f32::consts::FRAC_PI_2,
                9.0,
            );
        }
        3 => s.polyline(
            &[
                p(3.0, 7.0),
                p(3.0, 19.0),
                p(21.0, 19.0),
                p(21.0, 9.0),
                p(11.0, 9.0),
                p(9.0, 5.0),
                p(3.0, 5.0),
                p(3.0, 7.0),
            ],
            2.0,
        ),
        _ => {
            for (b, knob) in [(6.0, 16.0), (12.0, 10.0), (18.0, 18.0)] {
                s.segment(p(4.0, b), p(20.0, b), 2.0);
                s.disc(x + knob, y + b, 2.6);
            }
        }
    }
    cx.canvas.blend_shape(&s, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tab with every kind of control, for the layout tests.
    pub fn sample() -> PrefsView {
        let row = |name: &str, control: Control| RowView {
            name: name.into(),
            desc: format!("What {name} does"),
            control: Some(control),
            ..RowView::default()
        };
        let pair = |a: &str, b: &str| {
            vec![
                (a.to_string(), String::new()),
                (b.to_string(), String::new()),
            ]
        };
        PrefsView {
            tabs: ["Playing", "Sound", "Appearance", "Projects", "System"]
                .iter()
                .enumerate()
                .map(|(i, n)| SideTab {
                    name: n.to_string(),
                    line: "line".to_string(),
                    tab: i,
                    lit: i == 0,
                    icon: i,
                    ..SideTab::default()
                })
                .collect(),
            tab: 0,
            rows: vec![
                RowView {
                    picture: Some(Picture::Curve(
                        vec![(0.0, 0.0), (0.5, 0.7), (1.0, 1.0)],
                        None,
                    )),
                    ..row(
                        "Curve",
                        Control::Choice {
                            options: pair("Soft", "Hard"),
                            at: 0,
                            now: None,
                        },
                    )
                },
                row("Latch", Control::Switch(false)),
                row(
                    "Memory",
                    Control::Steps {
                        labels: vec!["1".into(), "2".into(), "5".into()],
                        at: 1,
                    },
                ),
                row("Velocity", Control::Slider { pos: 0.5 }),
                row("Latency", Control::Knob { pos: 0.3 }),
                row("Guide", Control::Button("Start".into())),
                RowView {
                    bare: true,
                    ..row(
                        "At start",
                        Control::Tiles {
                            options: pair("New", "Last"),
                            at: 1,
                        },
                    )
                },
            ],
            cards: vec![
                CardView {
                    title: "Pads".into(),
                    column: 0,
                    fill: true,
                    rows: vec![0, 1, 3],
                    ..CardView::default()
                },
                CardView {
                    title: "Memory".into(),
                    column: 1,
                    rows: vec![2, 4],
                    split: Some(Split {
                        title: "Together".into(),
                        total: "153 MB".into(),
                        parts: vec![("Loops".into(), 0.8), ("Takes".into(), 0.2)],
                    }),
                    ..CardView::default()
                },
                CardView {
                    title: "More".into(),
                    column: 1,
                    rows: vec![5, 6],
                    ..CardView::default()
                },
            ],
            ..PrefsView::default()
        }
    }

    /// Prefs' slot on the MPC's panel (the gui's `layouts_fit_the_screen`
    /// checks it is).
    pub const R: Rect = Rect::new(12, 84, 1256, 666);

    /// Every tab the app draws fits the page: each column's cards end at
    /// the foot and none past it, every row inside its card, with System
    /// info's rows as long as on the MPC.
    #[test]
    fn every_tab_of_the_app_fits_the_page() {
        use crate::app::Effect;
        let mut app = crate::app::App::new(44_100, false);
        let mut fx = app.open_tab(crate::app::View::Prefs);
        let info = crate::sysinfo::sample();
        for tab in 0..crate::app::PrefsTab::ALL.len() {
            fx.extend(app.pick_prefs_tab(tab));
            let Some(Effect::Menu(m)) = fx.iter().rev().find(|e| matches!(e, Effect::Menu(_)))
            else {
                panic!("no menu: {fx:?}");
            };
            let view = *m.prefs.clone().expect("Prefs' view");
            assert_eq!(view.tab, tab);
            let p = PrefsPage {
                view,
                info: info
                    .software
                    .iter()
                    .chain(&info.hardware)
                    .cloned()
                    .collect(),
                ..PrefsPage::default()
            };
            let g = p.geo(R);
            let foot = g.content.bottom();
            for col in 0..2u8 {
                let bottoms: Vec<i32> = (0..p.view.cards.len())
                    .filter(|&k| p.view.cards[k].column == col)
                    .map(|k| g.cards[k].bottom())
                    .collect();
                if let Some(&last) = bottoms.iter().max() {
                    assert_eq!(last, foot, "tab {tab}: column {col} ends at the foot");
                }
            }
            for (k, card) in p.view.cards.iter().enumerate() {
                for &i in &card.rows {
                    let a = g.rows[i].area;
                    assert_eq!(a.intersect(g.cards[k]), a, "tab {tab}: row {i} in card {k}");
                }
            }
        }
    }

    #[test]
    fn the_cards_fill_both_columns_to_the_foot_without_overlapping() {
        let p = PrefsPage {
            view: sample(),
            ..PrefsPage::default()
        };
        let g = p.geo(R);
        let foot = g.content.bottom();
        for col in 0..2u8 {
            let last = (0..p.view.cards.len())
                .filter(|&k| p.view.cards[k].column == col)
                .map(|k| g.cards[k])
                .max_by_key(|r| r.y)
                .unwrap();
            assert_eq!(last.bottom(), foot, "column {col} reaches the foot");
        }
        for (a, ra) in g.cards.iter().enumerate() {
            for (b, rb) in g.cards.iter().enumerate() {
                if a != b {
                    assert!(ra.intersect(*rb).is_empty(), "cards {a} and {b} overlap");
                }
            }
        }
        // Every row sits in its card, and rows don't overlap.
        for (k, card) in p.view.cards.iter().enumerate() {
            for &i in &card.rows {
                let a = g.rows[i].area;
                assert_eq!(a.intersect(g.cards[k]), a, "row {i} inside card {k}");
            }
            for &i in &card.rows {
                for &j in &card.rows {
                    if i != j {
                        assert!(g.rows[i].area.intersect(g.rows[j].area).is_empty());
                    }
                }
            }
        }
    }

    #[test]
    fn every_target_answers_with_its_event() {
        let mut p = PrefsPage {
            view: PrefsView {
                reset: Some("Reset Playing".into()),
                ..sample()
            },
            ..PrefsPage::default()
        };
        assert!(p.targets(R).iter().any(|t| t.id == "reset"));
        for t in p.targets(R) {
            let (x, y) = t.centre();
            let down = p.touch(1, Touch::Down, x, y, R);
            let up = p.touch(1, Touch::Up, x, y, R);
            if t.id == "reset" {
                assert_eq!(down, None, "Reset waits for the lift");
            }
            let ev = down.or(up);
            match t.id {
                "knob" => assert!(ev.is_none(), "a knob waits for the finger's move"),
                _ if !t.enabled => assert!(ev.is_none(), "{t:?} is already so"),
                "back" => assert_eq!(ev, Some(EventKind::Tab(BACK))),
                "reset" => assert_eq!(ev, Some(EventKind::Tab(RESET))),
                "tab" => assert_eq!(ev, Some(EventKind::Tab(t.index))),
                "choice" | "tile" => assert_eq!(
                    ev,
                    Some(EventKind::Cell {
                        row: t.index / 100,
                        col: t.index % 100
                    })
                ),
                "steps" => assert!(matches!(ev, Some(EventKind::Cell { row: 2, .. }))),
                "slider" => assert!(matches!(ev, Some(EventKind::Values { index: 3, .. }))),
                "switch" => assert_eq!(ev, Some(EventKind::Step { index: 1, delta: 1 })),
                "button" => assert_eq!(
                    ev,
                    Some(EventKind::Step {
                        index: t.index,
                        delta: 1
                    })
                ),
                other => panic!("no check for {other}"),
            }
        }
        // A touch outside every control answers nothing.
        assert_eq!(p.touch(1, Touch::Down, 600, 700, R), None);
    }

    /// Reset acts when the finger lifts on it: one that slides off, or is
    /// cancelled, changes nothing.
    #[test]
    fn reset_acts_on_the_lift_on_it() {
        let mut p = PrefsPage {
            view: PrefsView {
                reset: Some("Reset Playing".into()),
                ..sample()
            },
            ..PrefsPage::default()
        };
        let t = p
            .targets(R)
            .into_iter()
            .find(|t| t.id == "reset")
            .expect("Reset");
        let (x, y) = t.centre();
        assert!(t.rect.h >= 64, "Normal size: {:?}", t.rect);
        assert_eq!(p.touch(1, Touch::Down, x, y, R), None);
        assert_eq!(p.touch(1, Touch::Up, x, y, R), Some(EventKind::Tab(RESET)));
        assert_eq!(p.touch(2, Touch::Down, x, y, R), None);
        assert_eq!(p.touch(2, Touch::Up, x + 400, y, R), None, "slid off");
        assert_eq!(p.touch(3, Touch::Down, x, y, R), None);
        assert_eq!(p.touch(3, Touch::Cancel, x, y, R), None, "cancelled");
        // Another finger's lift doesn't press it.
        assert_eq!(p.touch(4, Touch::Down, x, y, R), None);
        assert_eq!(p.touch(5, Touch::Up, x, y, R), None);
    }

    /// Customise's controls: a banner across both columns, dials, swatches,
    /// looks and ‹ value ›, each touch answering with its event.
    #[test]
    fn customise_controls_answer_with_their_events() {
        let row = |name: &str, control: Control| RowView {
            name: name.into(),
            control: Some(control),
            ..RowView::default()
        };
        let dial = |name: &str| RowView {
            share: 1,
            ..row(
                name,
                Control::Dial {
                    pos: 0.5,
                    dot: None,
                },
            )
        };
        let colours = |n: usize| -> Vec<(String, Color)> {
            (0..n)
                .map(|k| (format!("C{k}"), Color::rgb(0x305AC8 + 0x140000 * k as u32)))
                .collect()
        };
        let view = PrefsView {
            tabs: sample().tabs,
            tab: 2,
            rows: vec![
                RowView {
                    share: 6,
                    desc: "A copy of Default".into(),
                    ..row("My theme", Control::Button("Rename".into()))
                },
                RowView {
                    bare: true,
                    share: 2,
                    faded: true,
                    ..row("", Control::Button("Undo changes".into()))
                },
                RowView {
                    bare: true,
                    share: 2,
                    strong: true,
                    ..row("", Control::Button("Save theme".into()))
                },
                dial("Accent"),
                dial("Second accent"),
                dial("Lightness"),
                dial("Warmth"),
                row(
                    "Every colour",
                    Control::Swatches {
                        colours: colours(9),
                        at: Some(4),
                        numbered: false,
                    },
                ),
                RowView {
                    bare: true,
                    ..row(
                        "Corners",
                        Control::Looks {
                            options: [
                                mpfree_ui::Corners::Round,
                                mpfree_ui::Corners::Chamfer,
                                mpfree_ui::Corners::Square,
                            ]
                            .iter()
                            .map(|&c| (format!("{c:?}"), Look::Corners(c)))
                            .collect(),
                            at: 0,
                        },
                    )
                },
                row("Set", Control::Stepper("Rainbow".into())),
            ],
            cards: vec![
                CardView {
                    column: 2,
                    rows: vec![0, 1, 2],
                    ..CardView::default()
                },
                CardView {
                    title: "Colours".into(),
                    column: 0,
                    rows: vec![3, 4, 5, 6, 7],
                    ..CardView::default()
                },
                CardView {
                    title: "Shape".into(),
                    column: 1,
                    rows: vec![8, 9],
                    ..CardView::default()
                },
            ],
            ..PrefsView::default()
        };
        let mut p = PrefsPage {
            view,
            ..PrefsPage::default()
        };
        let g = p.geo(R);
        // The banner spans both columns, its three on one line.
        let (name, undo, save) = (g.rows[0].area, g.rows[1].area, g.rows[2].area);
        assert!(
            name.y == undo.y && undo.y == save.y,
            "{name:?} {undo:?} {save:?}"
        );
        assert!(name.x < R.x + 260 && save.right() > R.right() - 30);
        assert!(name.w > 2 * undo.w, "the name has room for its line");
        // The four dials share a line.
        let ys: Vec<i32> = (3..7).map(|i| g.rows[i].area.y).collect();
        assert!(ys.iter().all(|&y| y == ys[0]), "{ys:?}");
        let mut counts = std::collections::BTreeMap::new();
        for t in p.targets(R) {
            *counts.entry(t.id).or_insert(0) += 1;
            let (x, y) = t.centre();
            let ev = p.touch(1, Touch::Down, x, y, R);
            p.touch(1, Touch::Up, x, y, R);
            match t.id {
                "back" | "tab" | "row" => {}
                "dial" => assert!(ev.is_none(), "a dial waits for the finger's move"),
                _ if !t.enabled => assert!(ev.is_none(), "{t:?} is already so"),
                "button" => assert_eq!(
                    ev,
                    Some(EventKind::Step {
                        index: t.index,
                        delta: 1
                    })
                ),
                "swatch" | "look" => assert_eq!(
                    ev,
                    Some(EventKind::Cell {
                        row: t.index / 100,
                        col: t.index % 100
                    })
                ),
                "stepper" => assert_eq!(
                    ev,
                    Some(EventKind::Step {
                        index: t.index / 100,
                        delta: if t.index % 100 == 0 { -1 } else { 1 },
                    })
                ),
                other => panic!("no check for {other}"),
            }
        }
        let n = |id| counts.get(id).copied().unwrap_or(0);
        assert_eq!(
            (n("button"), n("dial"), n("swatch"), n("look"), n("stepper")),
            (3, 4, 9, 3, 2)
        );
        // A dial turns with the finger's move up, as a knob does.
        let r = g.rows[3].control;
        let c = (r.x + r.w / 2, r.y + r.h / 2);
        p.touch(1, Touch::Down, c.0, c.1, R);
        let ev = p.touch(1, Touch::Move, c.0, c.1 - 30, R);
        assert!(
            matches!(ev, Some(EventKind::Values { index: 3, .. })),
            "{ev:?}"
        );
        p.touch(1, Touch::Up, c.0, c.1 - 30, R);
    }

    /// Hold and turn: a finger on a setting puts it under the wheel, which
    /// steps it; a row that only opens something, or the gallery, doesn't.
    #[test]
    fn a_finger_resting_on_a_setting_puts_it_under_the_wheel() {
        let mut p = PrefsPage {
            view: sample(),
            ..PrefsPage::default()
        };
        let g = p.geo(R);
        let at = |i: usize| (g.rows[i].area.x + 4, g.rows[i].area.y + 4);
        let (x, y) = at(3);
        let t = p.held(x, y, R).expect("the slider row");
        assert_eq!(t.index, 3);
        assert_eq!(
            p.held_step(&t, -2, false, R),
            Some(EventKind::Step {
                index: 3,
                delta: -2
            })
        );
        let (x, y) = at(5);
        assert!(p.held(x, y, R).is_none(), "Guide only opens something");
        p.view.gallery = Some(Gallery::default());
        let (x, y) = at(3);
        assert!(p.held(x, y, R).is_none());
    }

    #[test]
    fn a_knob_turns_with_the_fingers_move_up() {
        let mut p = PrefsPage {
            view: sample(),
            ..PrefsPage::default()
        };
        let knob = p.targets(R).into_iter().find(|t| t.id == "knob").unwrap();
        let (x, y) = knob.centre();
        assert_eq!(p.touch(7, Touch::Down, x, y, R), None);
        let ev = p.touch(7, Touch::Move, x, y - 100, R);
        assert_eq!(
            ev,
            Some(EventKind::Values {
                index: 4,
                set: vec![(0, 800)]
            })
        );
        p.touch(7, Touch::Up, x, y - 100, R);
        assert_eq!(p.drag, None);
    }

    #[test]
    fn the_gallery_tries_another_theme_and_asks_to_keep_it() {
        let tiles: Vec<ThemeTile> = mpfree_ui::theme::BUILTIN
            .iter()
            .map(|k| ThemeTile {
                key: k.to_string(),
                label: k.to_string(),
                problems: 0,
            })
            .collect();
        let mut p = PrefsPage {
            view: PrefsView {
                tabs: sample().tabs,
                tab: 2,
                rows: vec![RowView::default()],
                gallery: Some(Gallery {
                    tiles,
                    at: 0,
                    trial: Some(("Keep Slate?".into(), "Back to Default in 9 s".into())),
                    about: Vec::new(),
                    new_theme: true,
                    customise: false,
                }),
                ..PrefsView::default()
            },
            ..PrefsPage::default()
        };
        let ts = p.targets(R);
        assert_eq!(ts.iter().filter(|t| t.id == "theme").count(), 7);
        for t in ts {
            let (x, y) = t.centre();
            let ev = p.touch(1, Touch::Down, x, y, R);
            match (t.id, t.index) {
                ("theme", 0) => assert_eq!(ev, None, "the theme in use"),
                ("theme", k) => assert_eq!(ev, Some(EventKind::Cell { row: 0, col: k })),
                ("keep", 0) => assert_eq!(ev, Some(EventKind::Tab(KEEP))),
                ("new", _) => assert_eq!(ev, Some(EventKind::Tab(NEW_THEME))),
                ("keep", 1) => assert_eq!(ev, Some(EventKind::Tab(GO_BACK))),
                ("tab", 2) => assert_eq!(ev, None),
                ("tab", i) => assert_eq!(ev, Some(EventKind::Tab(i))),
                ("back", _) => assert_eq!(ev, Some(EventKind::Tab(BACK))),
                other => panic!("no check for {other:?}"),
            }
        }
    }

    #[test]
    fn it_draws_every_control_in_every_built_in_theme() {
        for name in mpfree_ui::theme::BUILTIN {
            let theme = Theme::builtin(name).unwrap();
            let mut canvas = mpfree_ui::SoftCanvas::new(1280, 800);
            let mut fonts = mpfree_ui::Fonts::embedded();
            let mut cx = PaintCx {
                canvas: &mut canvas,
                theme: &theme,
                fonts: &mut fonts,
            };
            let p = PrefsPage {
                view: PrefsView {
                    selected: 2,
                    editing: true,
                    ..sample()
                },
                info: vec![("MPFree".into(), "0.1.0".into())],
                ..PrefsPage::default()
            };
            p.paint(&mut cx, R);
        }
    }
}
