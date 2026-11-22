//! The Routing section (board 5 of the MIDI mockups, approved by the owner
//! on 16 Nov 2026; built without its blocks, which come with the MIDI
//! effects): where each source's MIDI goes, on one grid.
//!
//! - **Rows** are sources: each port with an input (plugged in, or named
//!   by the project and waiting for it), the MPC's own pads, and the
//!   tracks.
//! - **Columns** are where it goes: the **Armed track** (whichever track
//!   is armed), **Tracks by channel** (a channel of the port plays one
//!   track, whichever is armed: [`TrackRoute`]), **Learn**, each **MIDI
//!   out** port, and **CV out**.
//! - A port's Armed track and Learn squares are its Input (Plays tracks,
//!   Learn only, Both: the rig's, the same setting as on its row and its
//!   page). A track's square under a port is its Plays on (the project's).
//!   Tracks by channel are the project's routes.
//! - A port's square under a MIDI out port is **thru** ([`ThruRoute`]):
//!   its messages (all channels, or one) go straight out of that port, as
//!   they came, and play nothing inside MPFree. The project's, like the
//!   routes. A USB device's own output is "—": its messages never go back
//!   to it (the 5-pin jacks are two pieces of gear, so MIDI in to MIDI out
//!   is fine).
//! - A tap on an empty square adds that route and picks it; a tap on a
//!   set one picks it. The card under the grid shows the picked square:
//!   its On and Off, for Tracks by channel each route with its channel
//!   and track, and for thru each channel it sends. The wheel moves across
//!   the squares, - and + take a route away or add it, and the Q-Links
//!   turn the picked route's channel and track.
//! - **Blocks** (board 5): a port's Armed track square, and each route
//!   and thru (its line's Blocks button), has up to four blocks that
//!   shape what it takes, in order: Filter, Channel, Split, Transpose,
//!   Velocity ([`mpfree_midi::blocks`]). The card shows them as tiles
//!   with + Block; a tap picks one for the Q-Links, its × takes it away,
//!   a › between two swaps them. The card's last line says what last
//!   went through and what it became. Saved with the project.
//! - The pads' squares under an output say whether the armed track plays
//!   its kit on that port (drum pads out, by the gear's profile).

use std::cell::Cell;
use std::time::Duration;

use mpfree_midi::blocks::{Block, BlockKind, Blocks, Curve, Kind as MsgKind, Through, MAX_BLOCKS};
use mpfree_midi::cv::CvAssign;
use mpfree_midi::monitor::{Dir, To, What};
use mpfree_midi::setup::{thru_allowed, PortInput, ThruRoute, TrackOut, TrackRoute};
use mpfree_midi::Msg;
use mpfree_mixer_ui::{IoInput, IoKnob, IoReply, IoSection};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Target, Touch};

use crate::{IoModel, Shared};

const HEAD_H: i32 = 30;
const GAP: i32 = 10;
const NAME_W: i32 = 150;
const COL_HEAD_H: i32 = 42;
const ROW_H: i32 = 46;
const COL_MIN: i32 = 96;
const PANEL_H: i32 = 214;
const BUTTON_W: i32 = 132;
const BUTTON_H: i32 = 36;
const LINE_H: i32 = 20;
const STEP_W: i32 = 40;
const ROUTE_H: i32 = 40;
/// Route lines the card shows at once.
const ROUTE_LINES: usize = 3;
/// A block's tile.
const TILE_H: i32 = 106;
const TILE_MAX_W: i32 = 150;
/// The › between two tiles.
const ARROW_W: i32 = 22;
const BLOCKS_W: i32 = 112;
const THROUGH_H: i32 = 28;
/// How far back the card's last line looks in the monitor's list.
const THROUGH_LOOK: usize = 300;

pub(crate) const HINT: &str = "Tap a square: add or pick a route  ·  tap a block: the Q-Links turn it  ·  wheel: move  ·  - and +: take away or add";

/// A source row.
#[derive(Clone, Debug, PartialEq)]
enum Src {
    Port { name: String, plugged: bool },
    Pads,
    Track(u8),
}

/// A destination column.
#[derive(Clone, Debug, PartialEq)]
enum Dest {
    Armed,
    ByChannel,
    Learn,
    Out { name: String, plugged: bool },
    Cv,
}

/// How a square is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A route that is on.
    Set,
    /// Nothing yet: a tap adds it.
    Empty,
    /// Nothing can go there.
    None,
}

#[derive(Clone, Debug, PartialEq)]
struct Square {
    kind: Kind,
    text: String,
    sub: String,
    /// The route waits for its device, or won't send now.
    waits: bool,
    /// Its routes' blocks, all told (a badge in its corner).
    blocks: usize,
}

impl Square {
    fn set(text: impl Into<String>, sub: impl Into<String>) -> Square {
        Square {
            kind: Kind::Set,
            text: text.into(),
            sub: sub.into(),
            waits: false,
            blocks: 0,
        }
    }

    fn empty() -> Square {
        Square {
            kind: Kind::Empty,
            text: "+".to_string(),
            sub: String::new(),
            waits: false,
            blocks: 0,
        }
    }

    fn none() -> Square {
        Square {
            kind: Kind::None,
            text: "—".to_string(),
            sub: String::new(),
            waits: false,
            blocks: 0,
        }
    }

    fn waits(mut self, w: bool) -> Square {
        self.waits = w;
        self
    }

    fn blocks(mut self, n: usize) -> Square {
        self.blocks = n;
        self
    }
}

/// What the grid shows, from the model.
struct Grid {
    rows: Vec<Src>,
    cols: Vec<Dest>,
    /// Output ports left out for want of room.
    outs_hidden: usize,
}

/// The tracks a route or a row names, by their names on the screen.
fn track_name(m: &IoModel, t: u8) -> String {
    m.live
        .tracks
        .get(usize::from(t))
        .cloned()
        .unwrap_or_else(|| format!("Track {}", u16::from(t) + 1))
}

fn track_count(m: &IoModel) -> usize {
    m.live.tracks.len().max(1)
}

fn grid(m: &IoModel, room: usize) -> Grid {
    let ports = crate::midi::ports(m);
    let mut rows: Vec<Src> = ports
        .iter()
        .filter(|p| p.1.is_none_or(|l| l.has_in))
        .map(|p| Src::Port {
            name: p.0.clone(),
            plugged: p.1.is_some(),
        })
        .collect();
    rows.push(Src::Pads);
    rows.extend((0..track_count(m)).map(|t| Src::Track(t as u8)));
    let outs: Vec<Dest> = ports
        .iter()
        .filter(|p| p.1.is_none_or(|l| l.has_out))
        .map(|p| Dest::Out {
            name: p.0.clone(),
            plugged: p.1.is_some(),
        })
        .collect();
    let shown = outs.len().min(room.saturating_sub(4).max(1));
    let outs_hidden = outs.len() - shown;
    let mut cols = vec![Dest::Armed, Dest::ByChannel, Dest::Learn];
    cols.extend(outs.into_iter().take(shown));
    cols.push(Dest::Cv);
    Grid {
        rows,
        cols,
        outs_hidden,
    }
}

/// The project's routes from `port`, as indexes into `setup.routes`.
fn routes_from(m: &IoModel, port: &str) -> Vec<usize> {
    m.setup
        .routes
        .iter()
        .enumerate()
        .filter(|(_, r)| r.port == port)
        .map(|(i, _)| i)
        .collect()
}

/// The project's thru from `port` to the output `out`, as indexes into
/// `setup.thru`.
fn thru_to(m: &IoModel, port: &str, out: &str) -> Vec<usize> {
    m.setup
        .thru
        .iter()
        .enumerate()
        .filter(|(_, r)| r.from == port && r.to == out)
        .map(|(i, _)| i)
        .collect()
}

/// Why a thru sends nothing now, if it doesn't: its ports aren't both
/// plugged in, or MIDI out is off.
fn thru_waits(m: &IoModel, port_plugged: bool, out_plugged: bool) -> Option<&'static str> {
    if !port_plugged || !out_plugged {
        Some("waits for it")
    } else if !m.live.built.notes_out {
        Some("MIDI out is off")
    } else {
        None
    }
}

fn channel_text(c: Option<u8>) -> String {
    match c {
        Some(c) => format!("ch {}", c + 1),
        None => "all ch".to_string(),
    }
}

fn plays_armed(input: PortInput) -> bool {
    input != PortInput::LearnOnly
}

fn learns(input: PortInput) -> bool {
    input != PortInput::PlaysTracks
}

/// The CV outputs that follow track `t`, and their jobs.
fn cv_of(m: &IoModel, t: u8) -> Vec<(usize, &'static str)> {
    m.setup
        .cv
        .iter()
        .enumerate()
        .filter_map(|(i, a)| {
            let (track, job) = match *a {
                CvAssign::Pitch { track, .. } => (track, "pitch"),
                CvAssign::Gate { track, .. } => (track, "gate"),
                CvAssign::Trigger { track, .. } => (track, "trigger"),
                CvAssign::Velocity { track, .. } => (track, "velocity"),
                CvAssign::Pressure { track, .. } => (track, "pressure"),
                CvAssign::Cc { track, .. } => (track, "CC"),
                _ => return None,
            };
            (track == t).then_some((i, job))
        })
        .collect()
}

/// "1 and 2", "1, 2 and 5".
fn and_list(v: &[String]) -> String {
    match v {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn square(m: &IoModel, src: &Src, dest: &Dest) -> Square {
    match (src, dest) {
        (Src::Port { name, plugged }, Dest::Armed) => {
            if !plays_armed(m.setup.port(name).input) {
                return Square::empty();
            }
            // Routes and thru take their channels; the armed track gets
            // the rest.
            let routed =
                !routes_from(m, name).is_empty() || m.setup.thru.iter().any(|r| &r.from == name);
            Square::set(
                if routed { "the rest" } else { "all channels" },
                if *plugged { "plays it" } else { "waits for it" },
            )
            .waits(!plugged)
            .blocks(m.setup.port(name).blocks.len())
        }
        (Src::Port { name, plugged }, Dest::ByChannel) => {
            let rs = routes_from(m, name);
            let n: usize = rs.iter().map(|&i| m.setup.routes[i].blocks.len()).sum();
            square_by_channel(m, &rs, *plugged).blocks(n)
        }
        (Src::Port { name, plugged }, Dest::Learn) => {
            if !learns(m.setup.port(name).input) {
                return Square::empty();
            }
            let learned: Vec<String> = m
                .setup
                .learn
                .iter()
                .filter(|l| l.port.as_deref() == Some(name.as_str()) || l.port.is_none())
                .map(|l| match l.source {
                    mpfree_midi::learn::Source::Cc(n) => format!("CC {n}"),
                    mpfree_midi::learn::Source::Note(n) => crate::note_name(n),
                    mpfree_midi::learn::Source::PitchBend => "bend".to_string(),
                    mpfree_midi::learn::Source::ChanPressure => "pressure".to_string(),
                })
                .collect();
            let text = match learned.as_slice() {
                [] => "listens".to_string(),
                [one] => one.clone(),
                many => format!("{} controls", many.len()),
            };
            Square::set(
                text,
                match (learned.is_empty(), plugged) {
                    (_, false) => "waits for it",
                    (true, true) => "nothing yet",
                    (false, true) => "moves values",
                },
            )
            .waits(!plugged)
        }
        (
            Src::Port { name, plugged },
            Dest::Out {
                name: out,
                plugged: out_plugged,
            },
        ) => {
            if !thru_allowed(name, out) {
                return Square::none();
            }
            let rs = thru_to(m, name, out);
            if rs.is_empty() {
                return Square::empty();
            }
            let chans: Vec<Option<u8>> = rs.iter().map(|&i| m.setup.thru[i].channel).collect();
            let waits = thru_waits(m, *plugged, *out_plugged);
            let sub = match (waits, chans.as_slice()) {
                (Some(w), _) => w.to_string(),
                (None, [None]) => "all channels".to_string(),
                (None, [Some(c)]) => format!("ch {} only", c + 1),
                (None, many) => many
                    .iter()
                    .map(|&c| channel_text(c))
                    .collect::<Vec<_>>()
                    .join(", "),
            };
            let n = rs.iter().map(|&i| m.setup.thru[i].blocks.len()).sum();
            Square::set("thru", sub).waits(waits.is_some()).blocks(n)
        }
        (Src::Port { .. }, Dest::Cv) => Square::none(),
        (Src::Pads, Dest::Armed) => Square::set("always", "built in"),
        (Src::Pads, _) => Square::none(),
        (Src::Track(t), Dest::Out { name, plugged }) => {
            let Some(o) = m.setup.tracks.get(usize::from(*t)).and_then(|o| o.as_ref()) else {
                return Square::empty();
            };
            if &o.port != name {
                return Square::empty();
            }
            let live = m.live.port(name);
            let (sub, waits) = if !plugged {
                ("waits for it", true)
            } else if !m.live.built.notes_out {
                ("MIDI out is off", true)
            } else if !live.is_some_and(|l| l.sending) {
                ("not sending", true)
            } else {
                ("plays on it", false)
            };
            Square::set(channel_text(Some(o.channel)), sub).waits(waits)
        }
        (Src::Track(t), Dest::Cv) => {
            let cv = cv_of(m, *t);
            if cv.is_empty() {
                return Square::empty();
            }
            let outs: Vec<String> = cv.iter().map(|(i, _)| (i + 1).to_string()).collect();
            let mut jobs: Vec<&str> = cv.iter().map(|c| c.1).collect();
            jobs.dedup();
            Square::set(format!("CV {}", and_list(&outs)), jobs.join(", "))
                .waits(usize::from(m.live.built.cv_outs) <= cv[0].0)
        }
        (Src::Track(_), _) => Square::none(),
    }
}

/// The Tracks by channel square of a port with the routes `rs`.
fn square_by_channel(m: &IoModel, rs: &[usize], plugged: bool) -> Square {
    let sub = if plugged { "" } else { "waits for it" };

    match rs {
        [] => Square::empty(),
        [i] => {
            let r = &m.setup.routes[*i];
            Square::set(
                track_name(m, r.track),
                if plugged {
                    channel_text(r.channel)
                } else {
                    sub.to_string()
                },
            )
            .waits(!plugged)
        }
        many => {
            let chans: Vec<String> = many
                .iter()
                .map(|&i| channel_text(m.setup.routes[i].channel))
                .collect();
            Square::set(
                format!("{} routes", many.len()),
                if plugged {
                    chans.join(", ")
                } else {
                    sub.to_string()
                },
            )
            .waits(!plugged)
        }
    }
}

/// What a tap or a key hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Square(usize, usize),
    /// The card's main button: On, Off or + Route.
    Main,
    /// The card's Remove route.
    Remove,
    Route(usize),
    RouteChannel(usize, i32),
    RouteTrack(usize, i32),
    Channel(i32),
    /// A route line's Blocks: the card shows that route's blocks.
    Open(usize),
    /// The blocks view's ‹ Routes: back to the route lines.
    Back,
    Tile(usize),
    /// The picked tile's ×.
    DropBlock(usize),
    /// The › after tile `i`: it swaps with the next.
    Swap(usize),
    AddBlock,
    /// A kind in + Block's list.
    NewBlock(usize),
    CancelAdd,
}

/// Whose blocks the card shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Whose {
    /// A port's, on its way to the armed track.
    Port(String),
    /// A route's, by its place in `setup.routes`.
    Route(usize),
    /// A thru's, by its place in `setup.thru`.
    Thru(usize),
}

fn blocks_of(m: &IoModel, w: &Whose) -> Blocks {
    match w {
        Whose::Port(n) => m.setup.port(n).blocks,
        Whose::Route(i) => m.setup.routes.get(*i).map_or(Blocks::NONE, |r| r.blocks),
        Whose::Thru(i) => m.setup.thru.get(*i).map_or(Blocks::NONE, |r| r.blocks),
    }
}

fn set_blocks(m: &mut IoModel, w: &Whose, b: Blocks) -> Result<(), String> {
    match w {
        Whose::Port(n) => m.setup.set_port(n, |p| p.blocks = b),
        Whose::Route(i) => m.setup.routes.get_mut(*i).ok_or("That route went")?.blocks = b,
        Whose::Thru(i) => m.setup.thru.get_mut(*i).ok_or("That thru went")?.blocks = b,
    }
    Ok(())
}

/// The blocks view's places: the tiles' row, each tile `w` wide with a ›
/// after it, and the last line.
struct Tiles {
    row: Rect,
    w: i32,
    through: Rect,
}

fn tiles(l: &Layout) -> Tiles {
    let row = Rect::new(l.lines.x, l.lines.y, l.lines.w, TILE_H);
    let n = MAX_BLOCKS as i32 + 1;
    let w = ((row.w - (n - 1) * ARROW_W) / n).min(TILE_MAX_W);
    Tiles {
        row,
        w,
        through: Rect::new(l.lines.x, row.bottom() + 10, l.lines.w, THROUGH_H),
    }
}

impl Tiles {
    fn tile(&self, i: usize) -> Rect {
        Rect::new(
            self.row.x + i as i32 * (self.w + ARROW_W),
            self.row.y,
            self.w,
            TILE_H,
        )
    }

    /// The › after tile `i`.
    fn arrow(&self, i: usize) -> Rect {
        let t = self.tile(i);
        Rect::new(t.right(), t.y, ARROW_W, TILE_H)
    }

    /// Tile `i`'s ×, when it is picked.
    fn cross(&self, i: usize) -> Rect {
        let t = self.tile(i);
        Rect::new(t.right() - 34, t.y + 2, 32, 32)
    }

    /// + Block's kinds, then Cancel.
    fn kinds(&self) -> [Rect; 6] {
        let w = (self.row.w - 5 * 8) / 6;
        std::array::from_fn(|i| {
            Rect::new(
                self.row.x + i as i32 * (w + 8),
                self.row.y + 30,
                w,
                BUTTON_H + 8,
            )
        })
    }
}

/// A route line's Blocks button.
fn blocks_button(line: Rect) -> Rect {
    Rect::new(
        line.right() - BLOCKS_W - 4,
        line.y + 2,
        BLOCKS_W,
        line.h - 4,
    )
}

struct Layout {
    grid: Rect,
    /// The grid's squares area, under the column heads.
    body: Rect,
    col_w: i32,
    panel: Rect,
    main: Rect,
    remove: Rect,
    /// The card's lines.
    lines: Rect,
}

fn layout(r: Rect, cols: usize) -> Layout {
    let top = Rect::new(r.x, r.y + HEAD_H, r.w, r.h - HEAD_H);
    let (panel, grid) = top.take_bottom(PANEL_H);
    let grid = Rect::new(grid.x, grid.y, grid.w, (grid.h - GAP).max(0));
    let inner = grid.inset(10);
    let col_w = ((inner.w - NAME_W) / cols.max(1) as i32).max(1);
    let body = Rect::new(
        inner.x,
        inner.y + COL_HEAD_H,
        inner.w,
        (inner.h - COL_HEAD_H).max(0),
    );
    let head = Rect::new(panel.x + 14, panel.y + 8, panel.w - 28, BUTTON_H);
    let remove = Rect::new(head.right() - BUTTON_W, head.y, BUTTON_W, BUTTON_H);
    let main = Rect::new(remove.x - GAP - BUTTON_W, head.y, BUTTON_W, BUTTON_H);
    let lines = Rect::new(
        panel.x + 14,
        head.bottom() + 8,
        panel.w - 28,
        (panel.bottom() - head.bottom() - 16).max(0),
    );
    Layout {
        grid,
        body,
        col_w,
        panel,
        main,
        remove,
        lines,
    }
}

/// How many columns fit `r`.
fn room(r: Rect) -> usize {
    ((r.w - 20 - NAME_W) / COL_MIN).max(5) as usize
}

pub(crate) struct Routing {
    model: Shared,
    /// The square picked: row and column.
    pick: (usize, usize),
    /// The route picked on a Tracks by channel card, among the port's.
    route: usize,
    /// A Tracks by channel or thru card shows the picked route's blocks.
    open: bool,
    /// The block the Q-Links turn.
    block: Option<usize>,
    /// + Block was tapped: the card lists the kinds.
    adding: bool,
    /// The time, for the card's last line.
    now: Duration,
    /// The card's last line as [`Routing::through_now`] read it last.
    through: Option<(u64, String)>,
    /// The first row shown.
    top: Cell<usize>,
    down: Option<(u64, Option<Hit>)>,
    note: Option<String>,
    gen: u64,
}

impl Routing {
    pub fn new(model: Shared) -> Routing {
        Routing {
            model,
            pick: (0, 0),
            route: 0,
            open: false,
            block: None,
            adding: false,
            now: Duration::ZERO,
            through: None,
            top: Cell::new(0),
            down: None,
            note: None,
            gen: u64::MAX,
        }
    }

    fn rows_shown(body: Rect) -> usize {
        (body.h / ROW_H).max(1) as usize
    }

    /// The first row shown, moved so the picked row is in view.
    fn top_for(&self, rows: usize, shown: usize) -> usize {
        let mut top = self.top.get();
        if self.pick.0 < top {
            top = self.pick.0;
        } else if self.pick.0 >= top + shown {
            top = self.pick.0 + 1 - shown;
        }
        top = top.min(rows.saturating_sub(shown));
        self.top.set(top);
        top
    }

    fn square_rect(&self, l: &Layout, top: usize, row: usize, col: usize) -> Option<Rect> {
        let shown = Self::rows_shown(l.body);
        if row < top || row >= top + shown {
            return None;
        }
        let y = l.body.y + (row - top) as i32 * ROW_H;
        let x = l.body.x + NAME_W + col as i32 * l.col_w;
        Some(Rect::new(x + 4, y + 3, l.col_w - 8, ROW_H - 6))
    }

    /// The route lines on a Tracks by channel card: the line, its channel
    /// stepper (-, +), its track stepper (-, +).
    fn route_rects(l: &Layout, i: usize) -> (Rect, [Rect; 4]) {
        let y = l.lines.y + i as i32 * ROUTE_H;
        let line = Rect::new(l.lines.x, y, l.lines.w, ROUTE_H - 4);
        let ch = Rect::new(line.x + 8, line.y, 70 + 2 * STEP_W, line.h);
        let tr = Rect::new(ch.right() + 40, line.y, 200 + 2 * STEP_W, line.h);
        (
            line,
            [
                Rect::new(ch.x, ch.y + 2, STEP_W, ch.h - 4),
                Rect::new(ch.right() - STEP_W, ch.y + 2, STEP_W, ch.h - 4),
                Rect::new(tr.x, tr.y + 2, STEP_W, tr.h - 4),
                Rect::new(tr.right() - STEP_W, tr.y + 2, STEP_W, tr.h - 4),
            ],
        )
    }

    /// The channel stepper on a track's output card (-, +).
    fn channel_rects(l: &Layout) -> [Rect; 2] {
        let y = l.lines.y + 2 * LINE_H + 8;
        let x = l.lines.x;
        [
            Rect::new(x, y, STEP_W, 34),
            Rect::new(x + STEP_W + 90, y, STEP_W, 34),
        ]
    }

    fn hit(&self, x: i32, y: i32, r: Rect) -> Option<Hit> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let g = grid(&m, room(r));
        let l = layout(r, g.cols.len());
        let top = self.top_for(g.rows.len(), Self::rows_shown(l.body));
        if l.body.contains(x, y) && x >= l.body.x + NAME_W {
            let row = top + ((y - l.body.y) / ROW_H) as usize;
            let col = ((x - l.body.x - NAME_W) / l.col_w) as usize;
            if row < g.rows.len() && col < g.cols.len() {
                return Some(Hit::Square(row, col));
            }
            return None;
        }
        if !l.panel.contains(x, y) {
            return None;
        }
        let (src, dest) = (g.rows.get(self.pick.0)?, g.cols.get(self.pick.1)?);
        let whose = self.whose_of(&m, src, dest);
        if l.main.contains(x, y) && has_main(src, dest) {
            return Some(if whose.is_some() && has_lines(src, dest) {
                Hit::Back
            } else {
                Hit::Main
            });
        }
        if l.remove.contains(x, y) && has_lines(src, dest) {
            return Some(Hit::Remove);
        }
        if let Some(w) = whose {
            return self.hit_tiles(&l, blocks_of(&m, &w).len(), x, y);
        }
        match (src, dest) {
            (Src::Port { name, .. }, Dest::ByChannel) => {
                let n = routes_from(&m, name).len();
                let first = self.first_route_shown(n);
                for i in 0..n.saturating_sub(first).min(ROUTE_LINES) {
                    let (line, steps) = Self::route_rects(&l, i);
                    let k = first + i;
                    if steps[0].contains(x, y) {
                        return Some(Hit::RouteChannel(k, -1));
                    }
                    if steps[1].contains(x, y) {
                        return Some(Hit::RouteChannel(k, 1));
                    }
                    if steps[2].contains(x, y) {
                        return Some(Hit::RouteTrack(k, -1));
                    }
                    if steps[3].contains(x, y) {
                        return Some(Hit::RouteTrack(k, 1));
                    }
                    if blocks_button(line).contains(x, y) {
                        return Some(Hit::Open(k));
                    }
                    if line.contains(x, y) {
                        return Some(Hit::Route(k));
                    }
                }
                None
            }
            (Src::Port { name, .. }, Dest::Out { name: out, .. }) => {
                let n = thru_to(&m, name, out).len();
                let first = self.first_route_shown(n);
                for i in 0..n.saturating_sub(first).min(ROUTE_LINES) {
                    let (line, steps) = Self::route_rects(&l, i);
                    let k = first + i;
                    if steps[0].contains(x, y) {
                        return Some(Hit::RouteChannel(k, -1));
                    }
                    if steps[1].contains(x, y) {
                        return Some(Hit::RouteChannel(k, 1));
                    }
                    if blocks_button(line).contains(x, y) {
                        return Some(Hit::Open(k));
                    }
                    if line.contains(x, y) {
                        return Some(Hit::Route(k));
                    }
                }
                None
            }
            (Src::Track(t), Dest::Out { name, .. }) if plays_on(&m, *t, name).is_some() => {
                let [down, up] = Self::channel_rects(&l);
                if down.contains(x, y) {
                    Some(Hit::Channel(-1))
                } else if up.contains(x, y) {
                    Some(Hit::Channel(1))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// The blocks view's targets: each tile, the picked one's ×, the ›
    /// between two, + Block (or its kinds and Cancel).
    fn tile_targets(&self, m: &IoModel, l: &Layout, w: &Whose, out: &mut Vec<Target>) {
        let t = tiles(l);
        if self.adding {
            let k = t.kinds();
            for (i, kind) in BlockKind::ALL.iter().enumerate() {
                out.push(Target::tap("routing-new-block", i, kind.name(), k[i]));
            }
            out.push(Target::tap(
                "routing-cancel-block",
                0,
                "Cancel",
                k[BlockKind::ALL.len()],
            ));
            return;
        }
        let blocks = blocks_of(m, w);
        let n = blocks.len();
        for (i, b) in blocks.iter().enumerate() {
            let (value, _) = tile_text(b);
            out.push(Target::tap(
                "routing-tile",
                i,
                format!("{}: {value}", b.kind().name()),
                t.tile(i),
            ));
            if self.block == Some(i) {
                out.push(Target::tap(
                    "routing-drop-block",
                    i,
                    "Take the block away",
                    t.cross(i),
                ));
            }
            if i + 1 < n {
                out.push(Target::tap("routing-swap", i, "Swap", t.arrow(i)));
            }
        }
        if n < MAX_BLOCKS {
            out.push(Target::tap("routing-add-block", 0, "+ Block", t.tile(n)));
        }
    }

    /// What a tap at `x`, `y` hits among `n` blocks' tiles (or + Block's
    /// kinds).
    fn hit_tiles(&self, l: &Layout, n: usize, x: i32, y: i32) -> Option<Hit> {
        let t = tiles(l);
        if self.adding {
            let k = t.kinds();
            if let Some(i) = k[..BlockKind::ALL.len()]
                .iter()
                .position(|r| r.contains(x, y))
            {
                return Some(Hit::NewBlock(i));
            }
            return k[BlockKind::ALL.len()]
                .contains(x, y)
                .then_some(Hit::CancelAdd);
        }
        for i in 0..n {
            if self.block == Some(i) && t.cross(i).contains(x, y) {
                return Some(Hit::DropBlock(i));
            }
            if t.tile(i).contains(x, y) {
                return Some(Hit::Tile(i));
            }
            if i + 1 < n && t.arrow(i).contains(x, y) {
                return Some(Hit::Swap(i));
            }
        }
        (n < MAX_BLOCKS && t.tile(n).contains(x, y)).then_some(Hit::AddBlock)
    }

    /// Whose blocks the card shows for the square `src`, `dest`: a port's
    /// on its Armed track square (when it plays the armed track), the
    /// picked route's or thru's once its Blocks was tapped.
    fn whose_of(&self, m: &IoModel, src: &Src, dest: &Dest) -> Option<Whose> {
        match (src, dest) {
            (Src::Port { name, .. }, Dest::Armed) => {
                plays_armed(m.setup.port(name).input).then(|| Whose::Port(name.clone()))
            }
            (Src::Port { name, .. }, Dest::ByChannel) if self.open => {
                let mine = routes_from(m, name);
                mine.get(self.route.min(mine.len().saturating_sub(1)))
                    .map(|&i| Whose::Route(i))
            }
            (Src::Port { name, .. }, Dest::Out { name: out, .. })
                if self.open && thru_allowed(name, out) =>
            {
                let mine = thru_to(m, name, out);
                mine.get(self.route.min(mine.len().saturating_sub(1)))
                    .map(|&i| Whose::Thru(i))
            }
            _ => None,
        }
    }

    fn whose(&self, m: &IoModel) -> Option<Whose> {
        let (s, d) = self.picked(m)?;
        self.whose_of(m, &s, &d)
    }

    /// The card's last line as it reads now, while a route's blocks show:
    /// its message's time and how long ago it came.
    fn through_now(&self, m: &IoModel) -> Option<(u64, String)> {
        let (s, d) = self.picked(m)?;
        let w = self.whose_of(m, &s, &d)?;
        let Src::Port { name, .. } = &s else {
            return None;
        };
        let said = last_through(m, &w, name, blocks_of(m, &w))?;
        let now = m.monitor.now(self.now.as_secs_f64());
        Some((said.t.to_bits(), crate::now::ago(now - said.t)))
    }

    /// A new square or route picked: the card starts on its route lines.
    fn repick(&mut self) {
        self.open = false;
        self.block = None;
        self.adding = false;
    }

    /// Changes the blocks the card shows; true when they changed.
    fn edit_blocks(&mut self, f: impl FnOnce(&mut Blocks) -> Result<(), String>) -> bool {
        let Some(w) = ({
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            self.whose(&m)
        }) else {
            self.note = Some("Pick a route first".to_string());
            return false;
        };
        self.edit(|m| {
            let mut b = blocks_of(m, &w);
            f(&mut b)?;
            set_blocks(m, &w, b)
        })
    }

    fn first_route_shown(&self, n: usize) -> usize {
        if n <= ROUTE_LINES {
            0
        } else {
            (self.route + 1)
                .saturating_sub(ROUTE_LINES)
                .min(n - ROUTE_LINES)
        }
    }

    /// The picked square's source and destination now.
    fn picked(&self, m: &IoModel) -> Option<(Src, Dest)> {
        let g = grid(m, usize::MAX);
        Some((
            g.rows.get(self.pick.0)?.clone(),
            g.cols.get(self.pick.1)?.clone(),
        ))
    }

    /// Edits the setup; true when it changed.
    fn edit(&mut self, f: impl FnOnce(&mut IoModel) -> Result<(), String>) -> bool {
        let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let before = m.setup.clone();
        if let Err(e) = f(&mut m) {
            drop(m);
            self.note = Some(e);
            return false;
        }
        let changed = m.setup != before;
        if changed {
            m.gen += 1;
            self.gen = m.gen;
        }
        changed
    }

    /// Turns the picked square's route on (`on`) or off, or adds a route
    /// to Tracks by channel and takes the picked one away.
    fn switch(&mut self, on: bool) -> IoReply {
        let picked = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            self.picked(&m)
        };
        let Some((src, dest)) = picked else {
            return IoReply::Handled;
        };
        let changed = match (&src, &dest) {
            (Src::Port { name, .. }, Dest::Armed | Dest::Learn) => {
                let armed = dest == Dest::Armed;
                let name = name.clone();
                self.edit(|m| {
                    let now = m.setup.port(&name).input;
                    let (plays, learn) = if armed {
                        (on, learns(now))
                    } else {
                        (plays_armed(now), on)
                    };
                    let to = match (plays, learn) {
                        (true, true) => PortInput::Both,
                        (true, false) => PortInput::PlaysTracks,
                        (false, true) => PortInput::LearnOnly,
                        (false, false) => {
                            return Err(format!(
                                "{} would neither play the armed track nor be heard by Learn: \
                                 turn the other on first",
                                m.setup.shown(&name)
                            ))
                        }
                    };
                    if to == now {
                        return Err(format!(
                            "{} is {} already",
                            if armed { "Armed track" } else { "Learn" },
                            if on { "on" } else { "off" }
                        ));
                    }
                    m.setup.set_port(&name, |p| p.input = to);
                    Ok(())
                })
            }
            (Src::Port { name, .. }, Dest::ByChannel) => {
                let name = name.clone();
                if on {
                    let mut added = None;
                    let c = self.edit(|m| {
                        let tracks = track_count(m);
                        let mine = routes_from(m, &name);
                        if m.setup.routes.len() >= mpfree_midi::setup::MAX_TRACK_ROUTES {
                            return Err(format!(
                                "A project keeps {} routes at most: take one away first",
                                mpfree_midi::setup::MAX_TRACK_ROUTES
                            ));
                        }
                        // The first channel this port doesn't route yet,
                        // to the first track that has no route from it.
                        let used =
                            |c: u8| mine.iter().any(|&i| m.setup.routes[i].channel == Some(c));
                        let Some(channel) = (0..16u8).find(|&c| !used(c)) else {
                            return Err(format!(
                                "Every channel of {} has a route",
                                m.setup.shown(&name)
                            ));
                        };
                        let taken = |t: u8| mine.iter().any(|&i| m.setup.routes[i].track == t);
                        let track = (0..tracks as u8).find(|&t| !taken(t)).unwrap_or(0);
                        m.setup.routes.push(TrackRoute {
                            port: name.clone(),
                            channel: Some(channel),
                            track,
                            blocks: Blocks::NONE,
                        });
                        added = Some(mine.len());
                        Ok(())
                    });
                    if let Some(k) = added {
                        self.route = k;
                    }
                    c
                } else {
                    let k = self.route;
                    let c = self.edit(|m| {
                        let mine = routes_from(m, &name);
                        let Some(&i) = mine.get(k) else {
                            return Err(format!(
                                "{} has no route to take away",
                                m.setup.shown(&name)
                            ));
                        };
                        m.setup.routes.remove(i);
                        Ok(())
                    });
                    self.route = self.route.saturating_sub(1);
                    c
                }
            }
            (Src::Port { name, .. }, Dest::Out { name: out, .. }) if thru_allowed(name, out) => {
                let (name, out) = (name.clone(), out.clone());
                if on {
                    let mut added = None;
                    let c = self.edit(|m| {
                        let mine = thru_to(m, &name, &out);
                        if m.setup.thru.len() >= mpfree_midi::setup::MAX_THRU_ROUTES {
                            return Err(format!(
                                "A project keeps {} thru at most: take one away first",
                                mpfree_midi::setup::MAX_THRU_ROUTES
                            ));
                        }
                        let chans: Vec<Option<u8>> =
                            mine.iter().map(|&i| m.setup.thru[i].channel).collect();
                        // All channels first; then the first channel not
                        // sent yet, when one is picked.
                        let channel = if chans.is_empty() {
                            None
                        } else if chans.contains(&None) {
                            return Err(format!(
                                "Every channel of {} goes thru to {} already: \
                                 pick one with - and + first",
                                m.setup.shown(&name),
                                out_said(m, &out)
                            ));
                        } else {
                            let Some(c) = (0..16u8).find(|c| !chans.contains(&Some(*c))) else {
                                return Err(format!(
                                    "Every channel of {} goes thru already",
                                    m.setup.shown(&name)
                                ));
                            };
                            Some(c)
                        };
                        m.setup.thru.push(ThruRoute {
                            from: name.clone(),
                            channel,
                            to: out.clone(),
                            blocks: Blocks::NONE,
                        });
                        added = Some(mine.len());
                        Ok(())
                    });
                    if let Some(k) = added {
                        self.route = k;
                    }
                    c
                } else {
                    let k = self.route;
                    let c = self.edit(|m| {
                        let mine = thru_to(m, &name, &out);
                        let Some(&i) = mine.get(k.min(mine.len().saturating_sub(1))) else {
                            return Err(format!(
                                "{} has no thru to {} to take away",
                                m.setup.shown(&name),
                                out_said(m, &out)
                            ));
                        };
                        m.setup.thru.remove(i);
                        Ok(())
                    });
                    self.route = self.route.saturating_sub(1);
                    c
                }
            }
            (Src::Track(t), Dest::Out { name, .. }) => {
                let (t, name) = (*t, name.clone());
                self.edit(|m| {
                    let slot = m
                        .setup
                        .tracks
                        .get_mut(usize::from(t))
                        .ok_or("MIDI out takes tracks 1 to 16")?;
                    let here = slot.as_ref().is_some_and(|o| o.port == name);
                    if on == here {
                        return Err(format!(
                            "It {} on {} already",
                            if on { "plays" } else { "doesn't play" },
                            out_said(m, &name)
                        ));
                    }
                    *slot = if on {
                        let ch = slot.as_ref().map_or(0, |o| o.channel);
                        Some(TrackOut::new(&name, ch))
                    } else {
                        None
                    };
                    Ok(())
                })
            }
            _ => {
                let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                self.note = Some(why_not_here(&m, &src, &dest));
                false
            }
        };
        if changed {
            IoReply::Redraw
        } else {
            IoReply::Handled
        }
    }

    /// Steps the picked route's or thru's channel (all, then 1 to 16).
    fn route_channel(&mut self, k: usize, d: i32) -> IoReply {
        let (name, out) = match self.picked_now() {
            Some((Src::Port { name, .. }, Dest::ByChannel)) => (name, None),
            Some((Src::Port { name, .. }, Dest::Out { name: out, .. })) => (name, Some(out)),
            _ => return IoReply::Handled,
        };
        self.route = k;
        if let Some(out) = out {
            let changed = self.edit(|m| {
                let i = *thru_to(m, &name, &out).get(k).ok_or("No thru picked")?;
                let r = m.setup.thru[i].clone();
                let at = r.channel.map_or(0, |c| i32::from(c) + 1);
                let to = at + d.signum();
                if !(0..=16).contains(&to) {
                    return Err(format!(
                        "The thru's channel is at its {}: {} it",
                        if d > 0 { "last, 16" } else { "first, all" },
                        if d > 0 { "- lowers" } else { "+ raises" }
                    ));
                }
                let channel = (to > 0).then(|| (to - 1) as u8);
                set_thru(m, i, ThruRoute { channel, ..r })
            });
            return if changed {
                IoReply::Redraw
            } else {
                IoReply::Handled
            };
        }
        let changed = self.edit(|m| {
            let i = *routes_from(m, &name).get(k).ok_or("No route picked")?;
            let r = m.setup.routes[i].clone();
            // All channels first, then 1 to 16.
            let at = r.channel.map_or(0, |c| i32::from(c) + 1);
            let to = at + d.signum();
            if !(0..=16).contains(&to) {
                return Err(format!(
                    "The route's channel is at its {}: {} it",
                    if d > 0 { "last, 16" } else { "first, all" },
                    if d > 0 { "- lowers" } else { "+ raises" }
                ));
            }
            let channel = (to > 0).then(|| (to - 1) as u8);
            set_route(m, i, TrackRoute { channel, ..r })
        });
        if changed {
            IoReply::Redraw
        } else {
            IoReply::Handled
        }
    }

    /// Steps the picked route's track.
    fn route_track(&mut self, k: usize, d: i32) -> IoReply {
        let Some((Src::Port { name, .. }, Dest::ByChannel)) = self.picked_now() else {
            return IoReply::Handled;
        };
        self.route = k;
        let changed = self.edit(|m| {
            let i = *routes_from(m, &name).get(k).ok_or("No route picked")?;
            let r = m.setup.routes[i].clone();
            let to = i32::from(r.track) + d;
            let last = track_count(m) as i32 - 1;
            if !(0..=last).contains(&to) {
                return Err(format!(
                    "The route plays the {} track: {} picks another",
                    if d > 0 { "last" } else { "first" },
                    if d > 0 { "-" } else { "+" }
                ));
            }
            set_route(
                m,
                i,
                TrackRoute {
                    track: to as u8,
                    ..r
                },
            )
        });
        if changed {
            IoReply::Redraw
        } else {
            IoReply::Handled
        }
    }

    /// Steps a track's output channel on the picked square.
    fn track_channel(&mut self, d: i32) -> IoReply {
        let Some((Src::Track(t), Dest::Out { name, .. })) = self.picked_now() else {
            return IoReply::Handled;
        };
        let changed = self.edit(|m| {
            let said = out_said(m, &name);
            let o = m
                .setup
                .tracks
                .get_mut(usize::from(t))
                .and_then(|o| o.as_mut())
                .filter(|o| o.port == name)
                .ok_or_else(|| format!("It doesn't play on {said}: + or a tap sets it"))?;
            let to = i32::from(o.channel) + d.signum();
            if !(0..16).contains(&to) {
                return Err(format!(
                    "The channel is at its {}: {} it",
                    if d > 0 { "last, 16" } else { "first, 1" },
                    if d > 0 { "- lowers" } else { "+ raises" }
                ));
            }
            o.channel = to as u8;
            Ok(())
        });
        if changed {
            IoReply::Redraw
        } else {
            IoReply::Handled
        }
    }

    fn picked_now(&self) -> Option<(Src, Dest)> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        self.picked(&m)
    }

    fn tap(&mut self, h: Hit) -> IoReply {
        match h {
            Hit::Square(row, col) => {
                let was = self.pick;
                self.pick = (row, col);
                if was != self.pick {
                    self.route = 0;
                    self.repick();
                }
                let sq = {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    let g = grid(&m, usize::MAX);
                    match (g.rows.get(row), g.cols.get(col)) {
                        (Some(s), Some(d)) => Some((square(&m, s, d), s.clone(), d.clone())),
                        _ => None,
                    }
                };
                match sq {
                    Some((sq, _, _)) if sq.kind == Kind::Empty => {
                        self.switch(true).max(IoReply::Redraw)
                    }
                    Some((sq, s, d)) if matches!(sq.kind, Kind::None) => {
                        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                        self.note = Some(why_not_here(&m, &s, &d));
                        IoReply::Redraw
                    }
                    _ => IoReply::Redraw,
                }
            }
            Hit::Main => {
                let on = {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    match self.picked(&m) {
                        Some((s, d)) if has_lines(&s, &d) => true,
                        Some((s, d)) => square(&m, &s, &d).kind != Kind::Set,
                        None => true,
                    }
                };
                self.switch(on).max(IoReply::Redraw)
            }
            Hit::Remove => {
                let r = self.switch(false);
                self.repick();
                r.max(IoReply::Redraw)
            }
            Hit::Route(k) => {
                self.route = k;
                IoReply::Redraw
            }
            Hit::Open(k) => {
                self.route = k;
                self.repick();
                self.open = true;
                IoReply::Redraw
            }
            Hit::Back => {
                self.repick();
                IoReply::Redraw
            }
            Hit::Tile(i) => {
                self.block = Some(i);
                self.adding = false;
                IoReply::Redraw
            }
            Hit::DropBlock(i) => {
                self.edit_blocks(|b| {
                    b.remove(i);
                    Ok(())
                });
                self.block = None;
                IoReply::Redraw
            }
            Hit::Swap(i) => {
                if self.edit_blocks(|b| {
                    b.move_later(i);
                    Ok(())
                }) {
                    self.block = match self.block {
                        Some(k) if k == i => Some(i + 1),
                        Some(k) if k == i + 1 => Some(i),
                        other => other,
                    };
                }
                IoReply::Redraw
            }
            Hit::AddBlock => {
                self.adding = true;
                self.block = None;
                IoReply::Redraw
            }
            Hit::NewBlock(k) => {
                let mut at = None;
                self.edit_blocks(|b| {
                    if !b.push(BlockKind::ALL[k].new_block()) {
                        return Err(format!("A route has {MAX_BLOCKS} blocks at most"));
                    }
                    at = Some(b.len() - 1);
                    Ok(())
                });
                self.adding = false;
                self.block = at;
                IoReply::Redraw
            }
            Hit::CancelAdd => {
                self.adding = false;
                IoReply::Redraw
            }
            Hit::RouteChannel(k, d) => self.route_channel(k, d).max(IoReply::Redraw),
            Hit::RouteTrack(k, d) => self.route_track(k, d).max(IoReply::Redraw),
            Hit::Channel(d) => self.track_channel(d).max(IoReply::Redraw),
        }
    }
}

/// A route set again, refused if the port has that route already.
fn set_route(m: &mut IoModel, i: usize, r: TrackRoute) -> Result<(), String> {
    if m.setup
        .routes
        .iter()
        .enumerate()
        .any(|(j, o)| j != i && o.same_route(&r))
    {
        return Err(format!(
            "{} → {} is a route already",
            r.source_in(&m.setup),
            track_name(m, r.track)
        ));
    }
    m.setup.routes[i] = r;
    Ok(())
}

/// A thru set again, refused if the port sends that channel there
/// already.
fn set_thru(m: &mut IoModel, i: usize, r: ThruRoute) -> Result<(), String> {
    if m.setup
        .thru
        .iter()
        .enumerate()
        .any(|(j, o)| j != i && o.same_route(&r))
    {
        return Err(format!(
            "{} goes thru to {} already",
            r.source_in(&m.setup),
            out_said(m, &r.to)
        ));
    }
    m.setup.thru[i] = r;
    Ok(())
}

/// The track's output channel when it plays on `port`.
fn plays_on(m: &IoModel, t: u8, port: &str) -> Option<u8> {
    m.setup
        .tracks
        .get(usize::from(t))?
        .as_ref()
        .filter(|o| o.port == port)
        .map(|o| o.channel)
}

/// Whether the picked square's card has an On, Off or + Route button.
fn has_main(src: &Src, dest: &Dest) -> bool {
    has_lines(src, dest)
        || matches!(
            (src, dest),
            (Src::Port { .. }, Dest::Armed | Dest::Learn) | (Src::Track(_), Dest::Out { .. })
        )
}

/// Whether the picked square's card lists routes, with + Route and Remove
/// route: Tracks by channel, and thru.
fn has_lines(src: &Src, dest: &Dest) -> bool {
    match (src, dest) {
        (Src::Port { .. }, Dest::ByChannel) => true,
        (Src::Port { name, .. }, Dest::Out { name: out, .. }) => thru_allowed(name, out),
        _ => false,
    }
}

/// Why nothing can be set on a square, in words.
fn why_not_here(m: &IoModel, src: &Src, dest: &Dest) -> String {
    match (src, dest) {
        (Src::Port { name, .. }, Dest::Out { .. }) => {
            format!(
                "{}'s messages never go back to it: thru goes to another output",
                m.setup.shown(name)
            )
        }
        (Src::Port { .. }, Dest::Cv) => {
            "CV outputs follow a track: pick its job and track in the CV section".to_string()
        }
        (Src::Pads, Dest::Armed) => "The pads always play the armed track".to_string(),
        (Src::Pads, Dest::Out { name, .. }) => format!(
            "The pads play the armed track: when it plays on {} (a track's row), \
             so do they, a kit's pads by the gear's profile",
            out_said(m, name)
        ),
        (Src::Pads, _) => "The pads play the armed track only".to_string(),
        (Src::Track(_), Dest::Cv) => {
            "Pick a CV output's job and this track in the CV section".to_string()
        }
        (Src::Track(_), _) => "A track's notes go out: to MIDI out or CV".to_string(),
        _ => "Nothing to set here".to_string(),
    }
}

fn src_text(m: &IoModel, s: &Src) -> (String, String) {
    match s {
        Src::Port { name, plugged } => {
            let kind = m
                .live
                .port(name)
                .map(|l| l.kind.clone())
                .unwrap_or_else(|| "USB".to_string());
            (
                if name == mpfree_midi::setup::DIN_NAME {
                    "MIDI in".to_string()
                } else {
                    m.setup.shown(name).to_string()
                },
                if *plugged {
                    kind
                } else {
                    "USB · unplugged".to_string()
                },
            )
        }
        Src::Pads => ("The pads".to_string(), "MPC One".to_string()),
        Src::Track(t) => (
            track_name(m, *t),
            format!("track {} · its notes", u16::from(*t) + 1),
        ),
    }
}

/// An output port in words: "MIDI out" for the 5-pin jacks, else its
/// name on screen.
fn out_said(m: &IoModel, name: &str) -> String {
    if name == mpfree_midi::setup::DIN_NAME {
        "MIDI out".to_string()
    } else {
        m.setup.shown(name).to_string()
    }
}

fn dest_text(m: &IoModel, d: &Dest) -> (String, String) {
    match d {
        Dest::Armed => ("Armed track".to_string(), "on screen".to_string()),
        Dest::ByChannel => ("Tracks".to_string(), "by channel".to_string()),
        Dest::Learn => ("Learn".to_string(), "moves values".to_string()),
        Dest::Out { name, plugged } if name == mpfree_midi::setup::DIN_NAME => (
            "MIDI out".to_string(),
            if *plugged { "5-pin" } else { "unplugged" }.to_string(),
        ),
        Dest::Out { name, plugged } => (
            m.setup.shown(name).to_string(),
            if *plugged { "USB" } else { "USB · unplugged" }.to_string(),
        ),
        Dest::Cv => ("CV out".to_string(), "8 jacks".to_string()),
    }
}

/// Inward (plays in MPFree) or outward (sends out).
fn inward(d: &Dest) -> bool {
    matches!(d, Dest::Armed | Dest::ByChannel | Dest::Learn)
}

fn paint_square(cx: &mut PaintCx, r: Rect, sq: &Square, inward: bool, picked: bool) {
    let t = cx.theme.clone();
    match sq.kind {
        Kind::Set => {
            cx.canvas.fill_round_rect(r, t.round(8), t.raised);
            let bar = if inward { t.good } else { t.accent };
            let bar = if sq.waits { t.text_faint } else { bar };
            cx.canvas
                .fill_round_rect(Rect::new(r.x, r.y + 4, 3, r.h - 8), 1, bar);
        }
        Kind::Empty => {
            cx.canvas.stroke_round_rect(r, t.round(8), 1, t.outline);
        }
        Kind::None => {}
    }
    if picked {
        cx.canvas.stroke_round_rect(r, t.round(8), 2, t.accent);
    }
    if sq.blocks > 0 {
        let b = Rect::new(r.right() - 24, r.bottom() - 20, 20, 16);
        cx.canvas.fill_round_rect(b, 8, t.accent2);
        cx.text(
            FontId::Bold,
            t.size_small * 0.75,
            b,
            Align::Center,
            t.ink_on(t.accent2),
            &sq.blocks.to_string(),
        );
    }
    let ink = match sq.kind {
        Kind::Set if sq.waits => t.text_dim,
        Kind::Set => t.text,
        Kind::Empty => t.text_dim,
        Kind::None => t.text_faint,
    };
    if sq.sub.is_empty() || sq.kind != Kind::Set {
        cx.text(FontId::Sans, t.size_small, r, Align::Center, ink, &sq.text);
        return;
    }
    let inner = Rect::new(r.x + 10, r.y + 3, r.w - 14, r.h - 6);
    let (a, b) = inner.take_top(inner.h / 2);
    cx.text(FontId::Sans, t.size_small, a, Align::Left, ink, &sq.text);
    let sub_ink = if sq.waits { t.warn } else { t.text_dim };
    cx.text(
        FontId::Sans,
        t.size_small * 0.85,
        b,
        Align::Left,
        sub_ink,
        &sq.sub,
    );
}

fn button(cx: &mut PaintCx, r: Rect, text: &str, lit: bool) {
    let t = cx.theme.clone();
    let (fill, ink) = if lit {
        (t.accent, t.on_accent)
    } else {
        (t.raised, t.text)
    };
    cx.canvas.fill_round_rect(r, t.round(10), fill);
    cx.text(FontId::Bold, t.size_small, r, Align::Center, ink, text);
}

fn lines(cx: &mut PaintCx, r: Rect, text: &[String], ink: Color) {
    let t = cx.theme.clone();
    for (i, l) in text.iter().enumerate() {
        let lr = Rect::new(r.x, r.y + i as i32 * LINE_H, r.w, LINE_H);
        if lr.bottom() > r.bottom() {
            break;
        }
        cx.text(FontId::Sans, t.size_small, lr, Align::Left, ink, l);
    }
}

fn stepper(cx: &mut PaintCx, minus: Rect, plus: Rect, value: &str, picked: bool) {
    let t = cx.theme.clone();
    for (r, s) in [(minus, "−"), (plus, "+")] {
        cx.canvas.fill_round_rect(r, t.round(8), t.raised);
        cx.text(FontId::Bold, t.size_body, r, Align::Center, t.text, s);
    }
    let mid = Rect::new(
        minus.right() + 4,
        minus.y,
        plus.x - minus.right() - 8,
        minus.h,
    );
    cx.canvas.fill_round_rect(mid, t.round(8), t.bg);
    if picked {
        cx.canvas.stroke_round_rect(mid, t.round(8), 2, t.accent);
    }
    cx.text(
        FontId::Sans,
        t.size_small,
        mid,
        Align::Center,
        t.text,
        value,
    );
}

impl Routing {
    /// The card under the grid for the picked square.
    fn paint_card(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, src: &Src, dest: &Dest) {
        let t = cx.theme.clone();
        cx.canvas.fill_round_rect(l.panel, t.round(10), t.surface);
        let (s, _) = src_text(m, src);
        let (d, _) = dest_text(m, dest);
        let whose = self.whose_of(m, src, dest);
        let title = match &whose {
            Some(Whose::Route(i)) => {
                let r = &m.setup.routes[*i];
                format!(
                    "{s} · {} → {} · its blocks",
                    channel_text(r.channel),
                    track_name(m, r.track)
                )
            }
            Some(Whose::Thru(i)) => {
                let r = &m.setup.thru[*i];
                format!("{s} · {} → {d} · its blocks", channel_text(r.channel))
            }
            Some(Whose::Port(_)) => format!("{s} → {d} · its blocks, in order"),
            None => format!("{s} → {d}"),
        }
        .to_uppercase();
        let head = Rect::new(
            l.panel.x + 14,
            l.panel.y + 8,
            l.main.x - l.panel.x - 24,
            BUTTON_H,
        );
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            &title,
        );
        let sq = square(m, src, dest);
        match (src, dest) {
            (Src::Port { name, plugged }, Dest::Armed) if whose.is_some() => {
                button(cx, l.main, "On", true);
                let mut say = vec![
                    "Its notes play the armed track, on every channel no route takes.".to_string(),
                    "On and Off: its Input, your rig's. Its blocks: saved with the project."
                        .to_string(),
                ];
                if !plugged {
                    say.push(format!("{name} isn't plugged in: kept for when it is."));
                }
                if let Some(w) = &whose {
                    self.paint_blocks(cx, m, l, w, name, &say);
                }
            }
            (Src::Port { name, plugged }, Dest::Armed | Dest::Learn) => {
                let on = sq.kind == Kind::Set;
                button(cx, l.main, if on { "On" } else { "Off" }, on);
                let armed = *dest == Dest::Armed;
                let mut say: Vec<String> = Vec::new();
                if armed {
                    say.push(if on {
                        "Its notes play the armed track, on every channel no route takes."
                            .to_string()
                    } else {
                        "Off: its notes play only the tracks its routes name.".to_string()
                    });
                    say.push(
                        "The same as its Input (Plays tracks, Both) on the MIDI section."
                            .to_string(),
                    );
                } else {
                    say.push(if on {
                        "Learn hears its controls: arm Learn on a value, then turn one.".to_string()
                    } else {
                        "Off: Learn doesn't hear it, and controls learned from it rest.".to_string()
                    });
                    for l in m
                        .setup
                        .learn
                        .iter()
                        .filter(|l| l.port.as_deref() == Some(name.as_str()))
                        .take(2)
                    {
                        say.push(format!(
                            "Learned: {}",
                            crate::control_text(&l.source, l.channel, m.setup.shown(name))
                        ));
                    }
                }
                say.push("Kept for every project (your rig).".to_string());
                if !plugged {
                    say.push(format!("{name} isn't plugged in: kept for when it is."));
                }
                lines(cx, l.lines, &say, t.text_dim);
            }
            (Src::Port { name, .. }, Dest::ByChannel | Dest::Out { .. }) if whose.is_some() => {
                button(cx, l.main, "‹ Routes", false);
                button(cx, l.remove, "Remove route", false);
                if let Some(w) = &whose {
                    self.paint_blocks(cx, m, l, w, name, &[]);
                }
            }
            (Src::Port { name, plugged }, Dest::ByChannel) => {
                button(cx, l.main, "+ Route", false);
                button(cx, l.remove, "Remove route", false);
                let mine = routes_from(m, name);
                if mine.is_empty() {
                    let say = [
                        "Add a route: a channel of this port plays one track, whichever"
                            .to_string(),
                        "track is armed. That channel no longer plays the armed track.".to_string(),
                        "Saved with the project.".to_string(),
                    ];
                    lines(cx, l.lines, &say, t.text_dim);
                    return;
                }
                let first = self.first_route_shown(mine.len());
                for (k, &i) in mine.iter().enumerate().skip(first).take(ROUTE_LINES) {
                    let r = &m.setup.routes[i];
                    let (line, steps) = Self::route_rects(l, k - first);
                    let picked = k == self.route.min(mine.len() - 1);
                    if picked {
                        cx.canvas.fill_round_rect(line, t.round(8), t.raised);
                    }
                    stepper(cx, steps[0], steps[1], &channel_text(r.channel), picked);
                    let arrow = Rect::new(steps[1].right() + 4, line.y, 32, line.h);
                    cx.text(
                        FontId::Sans,
                        t.size_body,
                        arrow,
                        Align::Center,
                        t.text_dim,
                        "→",
                    );
                    stepper(cx, steps[2], steps[3], &track_name(m, r.track), picked);
                    let bb = blocks_button(line);
                    blocks_chip(cx, bb, r.blocks.len());
                    let say = Rect::new(
                        steps[3].right() + 12,
                        line.y,
                        bb.x - steps[3].right() - 16,
                        line.h,
                    );
                    let what = if *plugged { "plays it" } else { "waits for it" };
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        say,
                        Align::Left,
                        t.text_dim,
                        what,
                    );
                }
                if mine.len() > ROUTE_LINES {
                    let more = Rect::new(l.lines.x, l.lines.bottom() - LINE_H, l.lines.w, LINE_H);
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        more,
                        Align::Right,
                        t.text_faint,
                        &format!(
                            "{} routes · the wheel on the card's Q-Links moves",
                            mine.len()
                        ),
                    );
                }
            }
            (
                Src::Port { name, plugged },
                Dest::Out {
                    name: out,
                    plugged: out_plugged,
                },
            ) if thru_allowed(name, out) => {
                button(cx, l.main, "+ Route", false);
                button(cx, l.remove, "Remove route", false);
                let mine = thru_to(m, name, out);
                if mine.is_empty() {
                    let say = [
                        format!(
                            "Add thru: what comes in on {s} goes straight out of {d}, as it came."
                        ),
                        "Its channels no longer play inside MPFree. Never clock or SysEx."
                            .to_string(),
                        "Saved with the project. It sends while Prefs' Send MIDI and CV out is on."
                            .to_string(),
                    ];
                    lines(cx, l.lines, &say, t.text_dim);
                    return;
                }
                let waits = thru_waits(m, *plugged, *out_plugged);
                let first = self.first_route_shown(mine.len());
                for (k, &i) in mine.iter().enumerate().skip(first).take(ROUTE_LINES) {
                    let r = &m.setup.thru[i];
                    let (line, steps) = Self::route_rects(l, k - first);
                    let picked = k == self.route.min(mine.len() - 1);
                    if picked {
                        cx.canvas.fill_round_rect(line, t.round(8), t.raised);
                    }
                    stepper(cx, steps[0], steps[1], &channel_text(r.channel), picked);
                    let bb = blocks_button(line);
                    blocks_chip(cx, bb, r.blocks.len());
                    let say = Rect::new(
                        steps[1].right() + 12,
                        line.y,
                        bb.x - steps[1].right() - 16,
                        line.h,
                    );
                    let how = if r.blocks.is_empty() {
                        "as it came"
                    } else {
                        "through its blocks"
                    };
                    let (ink, what) = match waits {
                        Some(w) => (t.warn, format!("→ {d}: sends nothing now, {w}")),
                        None => (t.text_dim, format!("→ {d}: sends it {how}")),
                    };
                    cx.text(FontId::Sans, t.size_small, say, Align::Left, ink, &what);
                }
                if mine.len() > ROUTE_LINES {
                    let more = Rect::new(l.lines.x, l.lines.bottom() - LINE_H, l.lines.w, LINE_H);
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        more,
                        Align::Right,
                        t.text_faint,
                        &format!("{} channels · Q-Link 1 turns the picked one", mine.len()),
                    );
                }
            }
            (Src::Track(tr), Dest::Out { name, .. }) => {
                let ch = plays_on(m, *tr, name);
                button(
                    cx,
                    l.main,
                    if ch.is_some() { "On" } else { "Off" },
                    ch.is_some(),
                );
                let mut say = vec![if ch.is_some() {
                    format!(
                        "{} plays out of {} instead of inside MPFree.",
                        track_name(m, *tr),
                        d
                    )
                } else {
                    format!(
                        "On: {} plays out of {} instead of inside MPFree.",
                        track_name(m, *tr),
                        d
                    )
                }];
                say.push(
                    "Its Plays on, the same setting as on the MIDI section; saved with the project."
                        .to_string(),
                );
                lines(cx, l.lines, &say, t.text_dim);
                if let Some(c) = ch {
                    let [down, up] = Self::channel_rects(l);
                    stepper(cx, down, up, &format!("ch {}", c + 1), true);
                    let why = Rect::new(
                        up.right() + 16,
                        up.y,
                        l.lines.right() - up.right() - 16,
                        up.h,
                    );
                    let (ink, words) = if sq.waits {
                        (t.warn, format!("Sends nothing now: {}", sq.sub))
                    } else {
                        (t.good, "Sending".to_string())
                    };
                    cx.text(FontId::Sans, t.size_small, why, Align::Left, ink, &words);
                }
            }
            (Src::Track(tr), Dest::Cv) => {
                let cv = cv_of(m, *tr);
                let mut say: Vec<String> = cv
                    .iter()
                    .map(|(i, job)| format!("CV {} follows its {job}", i + 1))
                    .collect();
                if say.is_empty() {
                    say.push("No CV output follows this track.".to_string());
                }
                say.push("Set in the CV section: each output's job and track.".to_string());
                lines(cx, l.lines, &say, t.text_dim);
            }
            _ => {
                lines(cx, l.lines, &[why_not_here(m, src, dest)], t.text_dim);
            }
        }
    }
}

impl Routing {
    /// The blocks view: `w`'s blocks as tiles with + Block (or + Block's
    /// kinds), `say` beside + Block while there are none, and the last
    /// line: what last came in on `port` through them.
    fn paint_blocks(
        &self,
        cx: &mut PaintCx,
        m: &IoModel,
        l: &Layout,
        w: &Whose,
        port: &str,
        say: &[String],
    ) {
        let t = cx.theme.clone();
        let tl = tiles(l);
        let blocks = blocks_of(m, w);
        let n = blocks.len();
        if self.adding {
            cx.text(
                FontId::Bold,
                t.size_small,
                Rect::new(tl.row.x, tl.row.y, tl.row.w, 22),
                Align::Left,
                t.text_dim,
                "ADD A BLOCK: WHAT IT DOES TO WHAT THE ROUTE TAKES",
            );
            let k = tl.kinds();
            for (i, kind) in BlockKind::ALL.iter().enumerate() {
                button(cx, k[i], kind.name(), false);
            }
            button(cx, k[BlockKind::ALL.len()], "Cancel", false);
            let help = Rect::new(tl.row.x, k[0].bottom() + 6, tl.row.w, LINE_H);
            cx.text(
                FontId::Sans,
                t.size_small * 0.9,
                help,
                Align::Left,
                t.text_faint,
                "Filter drops kinds · Channel sends on one · Split takes a range of keys",
            );
        } else {
            for (i, b) in blocks.iter().enumerate() {
                paint_tile(cx, tl.tile(i), b, self.block == Some(i));
                if self.block == Some(i) {
                    cx.text(
                        FontId::Sans,
                        t.size_body,
                        tl.cross(i),
                        Align::Center,
                        t.text_dim,
                        "×",
                    );
                }
                let ink = if i + 1 < n { t.text_dim } else { t.text_faint };
                if i + 1 < n || n < MAX_BLOCKS {
                    cx.text(
                        FontId::Sans,
                        t.size_body,
                        tl.arrow(i),
                        Align::Center,
                        ink,
                        "›",
                    );
                }
            }
            if n < MAX_BLOCKS {
                let r = tl.tile(n);
                cx.canvas.stroke_round_rect(r, t.round(10), 1, t.outline);
                let head = Rect::new(r.x, r.y + 14, r.w, 24);
                cx.text(
                    FontId::Bold,
                    t.size_body,
                    head,
                    Align::Center,
                    t.text,
                    "+ Block",
                );
                let names: Vec<&str> = BlockKind::ALL.iter().map(|k| k.name()).collect();
                let words = Rect::new(r.x + 10, r.y + 44, r.w - 20, r.h - 50);
                wrap(
                    cx,
                    t.size_small * 0.85,
                    words,
                    Align::Center,
                    t.text_dim,
                    &names.join(", "),
                );
                if n == 0 {
                    let right = Rect::new(
                        r.right() + 16,
                        r.y + 8,
                        tl.row.right() - r.right() - 16,
                        r.h - 8,
                    );
                    lines(cx, right, say, t.text_dim);
                }
            }
        }
        // What last went through, and what it became.
        cx.canvas.fill_round_rect(tl.through, t.round(6), t.bg);
        let inner = Rect::new(
            tl.through.x + 10,
            tl.through.y,
            tl.through.w - 20,
            tl.through.h,
        );
        let now = m.monitor.now(self.now.as_secs_f64());
        let said = last_through(m, w, port, blocks);
        let px = t.size_small * 0.9;
        let Some(said) = said else {
            cx.text(
                FontId::Sans,
                px,
                inner,
                Align::Left,
                t.text_faint,
                &format!(
                    "Nothing through yet: play something on {}",
                    m.setup.shown(port)
                ),
            );
            return;
        };
        let mut x = inner.x;
        let mut put = |cx: &mut PaintCx, text: &str, ink: Color, font: FontId| {
            let r = Rect::new(x, inner.y, (inner.right() - x).max(0), inner.h);
            cx.text(font, px, r, Align::Left, ink, text);
            x += cx.measure(font, px, text) + 10;
        };
        put(cx, "Last through", t.text_dim, FontId::Sans);
        put(cx, &msg_words(&said.came), t.text, FontId::Bold);
        put(cx, "→", t.text_faint, FontId::Sans);
        match said.went {
            Through::Out(out) => {
                put(cx, &out_words(&said.came, &out), t.good, FontId::Bold);
                put(
                    cx,
                    &format!("{} · {}", said.where_to, crate::now::ago(now - said.t)),
                    t.text_dim,
                    FontId::Sans,
                );
            }
            Through::Dropped => put(
                cx,
                &format!("dropped · {}", crate::now::ago(now - said.t)),
                t.warn,
                FontId::Sans,
            ),
            Through::Free => put(
                cx,
                &format!(
                    "outside the split: left for others · {}",
                    crate::now::ago(now - said.t)
                ),
                t.text_dim,
                FontId::Sans,
            ),
        }
    }
}

/// A route line's Blocks button: how many it has.
fn blocks_chip(cx: &mut PaintCx, r: Rect, n: usize) {
    let text = match n {
        0 => "Blocks ›".to_string(),
        1 => "1 block ›".to_string(),
        n => format!("{n} blocks ›"),
    };
    let t = cx.theme.clone();
    cx.canvas.stroke_round_rect(r, t.round(10), 1, t.outline);
    cx.text(FontId::Bold, t.size_small, r, Align::Center, t.text, &text);
}

/// A block's tile: its kind, its value and a line under it.
fn paint_tile(cx: &mut PaintCx, r: Rect, b: &Block, picked: bool) {
    let t = cx.theme.clone();
    cx.canvas.fill_round_rect(r, t.round(10), t.raised);
    if picked {
        cx.canvas.stroke_round_rect(r, t.round(10), 2, t.accent);
    }
    let inner = Rect::new(r.x + 12, r.y + 8, r.w - 24, r.h - 16);
    let name = b.kind().name().to_uppercase();
    cx.text(
        FontId::Bold,
        t.size_small * 0.85,
        Rect::new(inner.x, inner.y, inner.w - 20, 18),
        Align::Left,
        t.text_dim,
        &name,
    );
    let (value, under) = tile_text(b);
    let v = Rect::new(inner.x, inner.y + 22, inner.w, 30);
    match *b {
        Block::Velocity {
            curve, fixed: None, ..
        } => {
            // The curve, from soft to hard.
            cx.canvas.fill_round_rect(v, 4, t.bg);
            let g = v.inset(4);
            let pts: Vec<(f32, f32)> = (0..=16)
                .map(|k| {
                    let x = k as f32 / 16.0;
                    let y = match curve {
                        Curve::Soft => x.sqrt(),
                        Curve::Linear => x,
                        Curve::Hard => x * x,
                    };
                    (
                        g.x as f32 + x * g.w as f32,
                        g.bottom() as f32 - y * g.h as f32,
                    )
                })
                .collect();
            let mut sh = mpfree_ui::Shape::new(v);
            sh.polyline(&pts, 2.0);
            sh.draw(cx.canvas, t.accent);
        }
        _ => {
            fit(cx, v, &value, t.text);
        }
    }
    let u = Rect::new(inner.x, inner.y + 56, inner.w, inner.h - 56);
    let under = match *b {
        Block::Velocity { fixed: None, .. } => value,
        _ => under,
    };
    wrap(cx, t.size_small * 0.85, u, Align::Left, t.text_dim, &under);
}

/// `text` on one line in `r`, as large as fits.
fn fit(cx: &mut PaintCx, r: Rect, text: &str, ink: Color) {
    let t = cx.theme.clone();
    let mut px = t.size_body * 1.1;
    while px > t.size_small * 0.8 && cx.measure(FontId::Bold, px, text) > r.w {
        px -= 1.0;
    }
    cx.text(FontId::Bold, px, r, Align::Left, ink, text);
}

/// `text` in `r`, broken between words, a line of 16 at a time.
fn wrap(cx: &mut PaintCx, px: f32, r: Rect, align: Align, ink: Color, text: &str) {
    const H: i32 = 16;
    let mut y = r.y;
    let mut line = String::new();
    let mut put = |cx: &mut PaintCx, line: &str| {
        if y + H <= r.bottom() {
            cx.text(
                FontId::Sans,
                px,
                Rect::new(r.x, y, r.w, H),
                align,
                ink,
                line,
            );
        }
        y += H;
    };
    for w in text.split(' ') {
        let longer = if line.is_empty() {
            w.to_string()
        } else {
            format!("{line} {w}")
        };
        if !line.is_empty() && cx.measure(FontId::Sans, px, &longer) > r.w {
            put(cx, &line);
            line = w.to_string();
        } else {
            line = longer;
        }
    }
    if !line.is_empty() {
        put(cx, &line);
    }
}

/// A kind's short word on a tile.
fn short(k: MsgKind) -> &'static str {
    match k {
        MsgKind::Notes => "notes",
        MsgKind::Pressure => "pressure",
        MsgKind::Bend => "bend",
        MsgKind::Controls => "CC",
        MsgKind::Program => "program",
    }
}

/// What a block's tile says: its value, and the line under it.
fn tile_text(b: &Block) -> (String, String) {
    match *b {
        Block::Filter(drop) => {
            let kept: Vec<&str> = MsgKind::ALL
                .iter()
                .filter(|k| !drop.has(**k))
                .map(|k| short(*k))
                .collect();
            let dropped: Vec<&str> = drop.iter().map(short).collect();
            let value = match (kept.as_slice(), dropped.as_slice()) {
                ([], _) => "Nothing".to_string(),
                (_, []) => "Everything".to_string(),
                (_, [one]) => format!("No {one}"),
                ([one], _) => cap(&format!("{one} only")),
                (k, _) => format!("{} of {} kinds", k.len(), MsgKind::ALL.len()),
            };
            let under = match (kept.len(), dropped.len()) {
                (_, 0) => "drops nothing yet".to_string(),
                (0, _) => "drops everything".to_string(),
                (k, d) if d > 2 && k <= 2 => format!("keeps {}", kept.join(", ")),
                _ => format!("drops {}", dropped.join(", ")),
            };
            (value, under)
        }
        Block::Channel(c) => (format!("ch {}", c + 1), "everything leaves on it".into()),
        Block::Split { low, high } => (
            format!("{} to {}", crate::note_name(low), crate::note_name(high)),
            "other keys go on".into(),
        ),
        Block::Transpose(t) => (
            if t == 0 {
                "0".to_string()
            } else {
                format!("{t:+}")
            },
            match t {
                0 => "as played".into(),
                12 => "one octave up".into(),
                -12 => "one octave down".into(),
                t if t % 12 == 0 && t > 0 => format!("{} octaves up", t / 12),
                t if t % 12 == 0 => format!("{} octaves down", -t / 12),
                1 => "a semitone up".into(),
                -1 => "a semitone down".into(),
                t if t > 0 => format!("{t} semitones up"),
                t => format!("{} semitones down", -t),
            },
        ),
        Block::Velocity {
            curve,
            low,
            high,
            fixed,
        } => match fixed {
            Some(f) => (format!("All at {f}"), "every note the same".into()),
            None => (format!("{} · {low} to {high}", curve.word()), String::new()),
        },
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// A message in a few words for the card's last line.
fn msg_words(m: &Msg) -> String {
    let n = crate::note_name;
    match *m {
        Msg::NoteOn { note, vel, .. } => format!("Note on {} vel {vel}", n(note)),
        Msg::NoteOff { note, .. } => format!("Note off {}", n(note)),
        Msg::PolyPressure { note, value, .. } => format!("Pressure {} {value}", n(note)),
        Msg::Cc { cc, value, .. } => format!("CC {cc} = {value}"),
        Msg::Program { program, .. } => format!("Program {}", u16::from(program) + 1),
        Msg::ChanPressure { value, .. } => format!("Pressure {value}"),
        Msg::PitchBend { value, .. } => format!("Bend {value:+}"),
    }
}

/// What `came` became, said shortly: a note's new name and velocity, and
/// its channel when that changed.
fn out_words(came: &Msg, out: &Msg) -> String {
    let n = crate::note_name;
    let mut s = match *out {
        Msg::NoteOn { note, vel, .. } => format!("{} vel {vel}", n(note)),
        Msg::NoteOff { note, .. } => format!("{} off", n(note)),
        _ => msg_words(out),
    };
    if came.channel() != out.channel() {
        s.push_str(&format!(" · ch {}", out.channel() + 1));
    }
    s
}

/// The card's last line: a message that came in on `port` for `w`, what
/// `blocks` made of it, where it went, and when (monitor seconds).
struct Said {
    came: Msg,
    went: Through,
    where_to: String,
    t: f64,
}

fn last_through(m: &IoModel, w: &Whose, port: &str, blocks: Blocks) -> Option<Said> {
    let id = m.monitor.ports().iter().find(|p| p.name == port)?.id;
    let (channel, where_to) = match w {
        Whose::Port(_) => (None, "played the armed track".to_string()),
        Whose::Route(i) => {
            let r = m.setup.routes.get(*i)?;
            (r.channel, format!("played {}", track_name(m, r.track)))
        }
        Whose::Thru(i) => {
            let r = m.setup.thru.get(*i)?;
            (r.channel, format!("sent to {}", out_said(m, &r.to)))
        }
    };
    let to_id = match w {
        Whose::Thru(i) => m
            .monitor
            .ports()
            .iter()
            .find(|p| p.name == m.setup.thru[*i].to)
            .map(|p| p.id),
        _ => None,
    };
    for (_, s) in m.monitor.newest().take(THROUGH_LOOK) {
        if s.port != id || s.dir != Dir::In {
            continue;
        }
        let What::Channel { msg, .. } = s.what else {
            continue;
        };
        if channel.is_some_and(|c| c != msg.channel()) {
            continue;
        }
        let ours = match (w, s.to) {
            (_, To::Nothing) => true,
            (Whose::Port(_), To::Track { .. } | To::NoPad { .. }) => true,
            (Whose::Route(i), To::Routed { track }) => m.setup.routes[*i].track == track,
            (Whose::Thru(_), To::Thru { to }) => Some(to) == to_id,
            _ => false,
        };
        if !ours {
            continue;
        }
        return Some(Said {
            came: msg,
            went: blocks.run(msg),
            where_to: where_to.clone(),
            t: s.t,
        });
    }
    None
}

impl IoSection for Routing {
    fn label(&self) -> &str {
        crate::ROUTING
    }

    fn hint(&self) -> Option<&str> {
        Some(HINT)
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let g = grid(&m, room(r));
        let l = layout(r, g.cols.len());
        // The heading and the key.
        let head = Rect::new(r.x + 4, r.y, r.w - 8, HEAD_H - 6);
        cx.text(
            FontId::Bold,
            t.size_small,
            head,
            Align::Left,
            t.text_dim,
            "WHERE EACH SOURCE'S MIDI GOES · SAVED WITH THE PROJECT",
        );
        let key = Rect::new(head.right() - 260, head.y, 260, head.h);
        let (a, b) = key.take_left(130);
        cx.canvas
            .fill_round_rect(Rect::new(a.x, a.y + 6, 3, a.h - 12), 1, t.good);
        cx.text(
            FontId::Sans,
            t.size_small * 0.9,
            Rect::new(a.x + 8, a.y, a.w - 8, a.h),
            Align::Left,
            t.text_dim,
            "plays in MPFree",
        );
        cx.canvas
            .fill_round_rect(Rect::new(b.x, b.y + 6, 3, b.h - 12), 1, t.accent);
        cx.text(
            FontId::Sans,
            t.size_small * 0.9,
            Rect::new(b.x + 8, b.y, b.w - 8, b.h),
            Align::Left,
            t.text_dim,
            "sends out",
        );

        // The grid.
        cx.canvas.fill_round_rect(l.grid, t.round(10), t.surface);
        for (c, d) in g.cols.iter().enumerate() {
            let x = l.body.x + NAME_W + c as i32 * l.col_w;
            let (name, sub) = dest_text(&m, d);
            let a = Rect::new(x + 6, l.body.y - COL_HEAD_H, l.col_w - 8, COL_HEAD_H / 2);
            let b = Rect::new(x + 6, a.bottom(), l.col_w - 8, COL_HEAD_H / 2 - 4);
            let plugged = !matches!(d, Dest::Out { plugged: false, .. });
            cx.text(
                FontId::Bold,
                t.size_small,
                a,
                Align::Left,
                if plugged { t.text } else { t.text_faint },
                &name,
            );
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                b,
                Align::Left,
                t.text_dim,
                &sub,
            );
        }
        let shown = Self::rows_shown(l.body);
        let top = self.top_for(g.rows.len(), shown);
        for (row, s) in g.rows.iter().enumerate().skip(top).take(shown) {
            let y = l.body.y + (row - top) as i32 * ROW_H;
            let (name, sub) = src_text(&m, s);
            let faint = matches!(s, Src::Port { plugged: false, .. });
            let a = Rect::new(l.body.x + 4, y + 4, NAME_W - 8, ROW_H / 2 - 4);
            let b = Rect::new(l.body.x + 4, a.bottom(), NAME_W - 8, ROW_H / 2 - 4);
            cx.text(
                FontId::Sans,
                t.size_body,
                a,
                Align::Left,
                if faint { t.text_faint } else { t.text },
                &name,
            );
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                b,
                Align::Left,
                t.text_dim,
                &sub,
            );
            for (col, d) in g.cols.iter().enumerate() {
                if let Some(sr) = self.square_rect(&l, top, row, col) {
                    let sq = square(&m, s, d);
                    paint_square(cx, sr, &sq, inward(d), self.pick == (row, col));
                }
            }
        }
        if top + shown < g.rows.len() || top > 0 {
            let more = Rect::new(l.grid.x + 10, l.grid.bottom() - 18, NAME_W, 16);
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                more,
                Align::Left,
                t.text_faint,
                &format!(
                    "rows {}–{} of {}",
                    top + 1,
                    (top + shown).min(g.rows.len()),
                    g.rows.len()
                ),
            );
        }
        if g.outs_hidden > 0 {
            let more = Rect::new(l.grid.right() - 300, l.grid.bottom() - 18, 290, 16);
            cx.text(
                FontId::Sans,
                t.size_small * 0.85,
                more,
                Align::Right,
                t.text_faint,
                &format!("{} more outputs: on the MIDI section", g.outs_hidden),
            );
        }
        // The picked square's card.
        if let (Some(s), Some(d)) = (g.rows.get(self.pick.0), g.cols.get(self.pick.1)) {
            self.paint_card(cx, &m, &l, s, d);
        }
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let g = grid(&m, room(r));
        let l = layout(r, g.cols.len());
        let top = self.top_for(g.rows.len(), Self::rows_shown(l.body));
        let mut out = Vec::new();
        for (row, s) in g.rows.iter().enumerate() {
            for (col, d) in g.cols.iter().enumerate() {
                let Some(sr) = self.square_rect(&l, top, row, col) else {
                    continue;
                };
                let sq = square(&m, s, d);
                let (sn, _) = src_text(&m, s);
                let (dn, _) = dest_text(&m, d);
                out.push(Target::tap(
                    "routing-square",
                    row * g.cols.len() + col,
                    format!("{sn} → {dn}: {}", sq.text),
                    sr,
                ));
            }
        }
        if let (Some(s), Some(d)) = (g.rows.get(self.pick.0), g.cols.get(self.pick.1)) {
            let whose = self.whose_of(&m, s, d);
            if has_main(s, d) {
                let label = if whose.is_some() && has_lines(s, d) {
                    "‹ Routes"
                } else {
                    "On, Off or + Route"
                };
                out.push(Target::tap("routing-main", 0, label, l.main));
            }
            if let Some(w) = whose {
                if has_lines(s, d) {
                    out.push(Target::tap("routing-remove", 0, "Remove route", l.remove));
                }
                self.tile_targets(&m, &l, &w, &mut out);
                return out;
            }
            match (s, d) {
                (Src::Port { name, .. }, Dest::ByChannel) => {
                    out.push(
                        Target::tap("routing-remove", 0, "Remove route", l.remove)
                            .disabled_if(routes_from(&m, name).is_empty()),
                    );
                    let n = routes_from(&m, name).len();
                    let first = self.first_route_shown(n);
                    for i in 0..n.saturating_sub(first).min(ROUTE_LINES) {
                        let (_, steps) = Self::route_rects(&l, i);
                        for (k, (id, label)) in [
                            ("routing-channel", "Channel −"),
                            ("routing-channel", "Channel +"),
                            ("routing-track", "Track −"),
                            ("routing-track", "Track +"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            out.push(Target::tap(id, (first + i) * 2 + k % 2, label, steps[k]));
                        }
                        let (line, _) = Self::route_rects(&l, i);
                        out.push(Target::tap(
                            "routing-blocks",
                            first + i,
                            "Blocks",
                            blocks_button(line),
                        ));
                    }
                }
                (Src::Port { name, .. }, Dest::Out { name: to, .. }) if thru_allowed(name, to) => {
                    let n = thru_to(&m, name, to).len();
                    out.push(
                        Target::tap("routing-remove", 0, "Remove route", l.remove)
                            .disabled_if(n == 0),
                    );
                    let first = self.first_route_shown(n);
                    for i in 0..n.saturating_sub(first).min(ROUTE_LINES) {
                        let (_, steps) = Self::route_rects(&l, i);
                        for (k, label) in ["Channel −", "Channel +"].into_iter().enumerate() {
                            out.push(Target::tap(
                                "routing-channel",
                                (first + i) * 2 + k,
                                label,
                                steps[k],
                            ));
                        }
                        let (line, _) = Self::route_rects(&l, i);
                        out.push(Target::tap(
                            "routing-blocks",
                            first + i,
                            "Blocks",
                            blocks_button(line),
                        ));
                    }
                }
                (Src::Track(tr), Dest::Out { name, .. }) if plays_on(&m, *tr, name).is_some() => {
                    let [down, up] = Self::channel_rects(&l);
                    out.push(Target::tap("routing-out-channel", 0, "Channel −", down));
                    out.push(Target::tap("routing-out-channel", 1, "Channel +", up));
                }
                _ => {}
            }
        }
        out
    }

    fn touch(&mut self, finger: u64, phase: Touch, x: i32, y: i32, r: Rect) -> IoReply {
        match phase {
            Touch::Down => {
                let h = self.hit(x, y, r);
                self.down = Some((finger, h));
                IoReply::Handled
            }
            Touch::Move => IoReply::Handled,
            Touch::Cancel => {
                self.down = None;
                IoReply::Handled
            }
            Touch::Up => {
                let Some((f, Some(h))) = self.down.take() else {
                    return IoReply::Handled;
                };
                if f != finger || self.hit(x, y, r) != Some(h) {
                    return IoReply::Handled;
                }
                self.tap(h)
            }
        }
    }

    fn input(&mut self, ev: IoInput) -> IoReply {
        let (rows, cols) = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            let g = grid(&m, usize::MAX);
            (g.rows.len(), g.cols.len())
        };
        match ev {
            IoInput::Wheel(d) => {
                let at = (self.pick.0 * cols + self.pick.1) as i32 + d;
                let last = (rows * cols) as i32 - 1;
                if !(0..=last).contains(&at) {
                    self.note = Some(
                        if d > 0 {
                            "The last square is picked"
                        } else {
                            "The first square is picked"
                        }
                        .to_string(),
                    );
                    return IoReply::Handled;
                }
                self.pick = (at as usize / cols, at as usize % cols);
                self.route = 0;
                self.repick();
                IoReply::Redraw
            }
            IoInput::Page(d) => {
                let to = (self.pick.0 as i32 + d * 4).clamp(0, rows as i32 - 1) as usize;
                if to == self.pick.0 {
                    self.note = Some("No more rows that way".to_string());
                    return IoReply::Handled;
                }
                self.pick.0 = to;
                self.route = 0;
                self.repick();
                IoReply::Redraw
            }
            // - takes the picked block away.
            IoInput::Step(d) if d < 0 && self.block.is_some() => {
                let i = self.block.unwrap_or(0);
                self.tap(Hit::DropBlock(i))
            }
            IoInput::Step(d) if d != 0 => self.switch(d > 0),
            IoInput::Step(_) => IoReply::Handled,
        }
    }

    fn tick(&mut self, now: Duration) -> IoReply {
        let was = self.now;
        self.now = now;
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if m.gen != self.gen {
            self.gen = m.gen;
            return IoReply::Redraw;
        }
        // The card's last line, while a route's blocks show: looked at
        // about four times a second, drawn again when it reads otherwise
        // (a new message, or a second more ago).
        if (now.as_millis() / 250) != (was.as_millis() / 250) {
            let line = self.through_now(&m);
            if line != self.through {
                self.through = line;
                return IoReply::Redraw;
            }
        }
        IoReply::Ignored
    }

    fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    fn qlinks(&self) -> Option<[IoKnob; 4]> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let off = || IoKnob {
            label: String::new(),
            text: String::new(),
            pos: 0.0,
            enabled: false,
        };
        let mut knobs = [off(), off(), off(), off()];
        // A picked block: the Q-Links turn it.
        if let (Some(w), Some(i)) = (self.whose(&m), self.block) {
            if let Some(b) = blocks_of(&m, &w).get(i) {
                for (k, (label, text, pos)) in block_knobs(&b).into_iter().enumerate() {
                    knobs[k] = IoKnob {
                        label: label.to_string(),
                        text,
                        pos,
                        enabled: true,
                    };
                }
                return Some(knobs);
            }
        }
        match self.picked(&m) {
            Some((Src::Port { name, .. }, Dest::ByChannel)) => {
                let mine = routes_from(&m, &name);
                if let Some(&i) = mine.get(self.route.min(mine.len().saturating_sub(1))) {
                    let r = &m.setup.routes[i];
                    let at = r.channel.map_or(0, |c| usize::from(c) + 1);
                    knobs[0] = IoKnob {
                        label: "Channel".to_string(),
                        text: channel_text(r.channel),
                        pos: at as f32 / 16.0,
                        enabled: true,
                    };
                    let n = track_count(&m);
                    knobs[1] = IoKnob {
                        label: "Track".to_string(),
                        text: track_name(&m, r.track),
                        pos: f32::from(r.track) / (n.max(2) - 1) as f32,
                        enabled: true,
                    };
                }
            }
            Some((Src::Port { name, .. }, Dest::Out { name: out, .. })) => {
                let mine = thru_to(&m, &name, &out);
                if let Some(&i) = mine.get(self.route.min(mine.len().saturating_sub(1))) {
                    let r = &m.setup.thru[i];
                    knobs[0] = IoKnob {
                        label: "Channel".to_string(),
                        text: channel_text(r.channel),
                        pos: r.channel.map_or(0, |c| usize::from(c) + 1) as f32 / 16.0,
                        enabled: true,
                    };
                }
            }
            Some((Src::Track(t), Dest::Out { name, .. })) => {
                if let Some(c) = plays_on(&m, t, &name) {
                    knobs[0] = IoKnob {
                        label: "Channel".to_string(),
                        text: format!("ch {}", c + 1),
                        pos: f32::from(c) / 15.0,
                        enabled: true,
                    };
                }
            }
            _ => {}
        }
        Some(knobs)
    }

    fn qlink_turn(&mut self, knob: usize, detents: i32, _fine: bool) -> IoReply {
        if detents == 0 {
            return IoReply::Ignored;
        }
        if let Some(i) = self.block {
            let turned = self.edit_blocks(|b| {
                let x = b.get(i).ok_or("That block went")?;
                let to = turn_block(x, knob, detents).ok_or("That Q-Link turns nothing here")?;
                b.set(i, to);
                Ok(())
            });
            return if turned {
                IoReply::Redraw
            } else {
                IoReply::Handled
            };
        }
        match (self.picked_now(), knob) {
            (Some((Src::Port { .. }, Dest::ByChannel)), 0) => {
                self.route_channel(self.route, detents)
            }
            (Some((Src::Port { .. }, Dest::ByChannel)), 1) => {
                self.route_track(self.route, detents.signum())
            }
            (Some((Src::Port { .. }, Dest::Out { .. })), 0) => {
                self.route_channel(self.route, detents)
            }
            (Some((Src::Track(_), Dest::Out { .. })), 0) => self.track_channel(detents),
            _ => IoReply::Ignored,
        }
    }
}

/// The Q-Links of a picked block: label, value and where it sits (0 to
/// 1), in Q-Link order.
fn block_knobs(b: &Block) -> Vec<(&'static str, String, f32)> {
    let n = crate::note_name;
    let pass = |drop: mpfree_midi::blocks::Kinds, ks: &[MsgKind]| {
        let dropped = ks.iter().all(|k| drop.has(*k));
        (
            if dropped { "drops" } else { "lets by" }.to_string(),
            if dropped { 1.0 } else { 0.0 },
        )
    };
    match *b {
        Block::Filter(drop) => FILTER_KNOBS
            .iter()
            .map(|(label, ks)| {
                let (text, pos) = pass(drop, ks);
                (*label, text, pos)
            })
            .collect(),
        Block::Channel(c) => vec![("Channel", format!("ch {}", c + 1), f32::from(c) / 15.0)],
        Block::Split { low, high } => vec![
            ("Lowest", n(low), f32::from(low) / 127.0),
            ("Highest", n(high), f32::from(high) / 127.0),
        ],
        Block::Transpose(t) => {
            let pos = (f32::from(t) + 48.0) / 96.0;
            vec![
                ("Semitones", format!("{t:+}"), pos),
                ("Octaves", format!("{:+}", t / 12), pos),
            ]
        }
        Block::Velocity {
            curve,
            low,
            high,
            fixed,
        } => vec![
            (
                "Curve",
                curve.word().to_string(),
                Curve::ALL.iter().position(|c| *c == curve).unwrap_or(1) as f32 / 2.0,
            ),
            ("Lowest", low.to_string(), f32::from(low) / 127.0),
            ("Highest", high.to_string(), f32::from(high) / 127.0),
            (
                "Fixed",
                fixed.map_or("off".to_string(), |f| f.to_string()),
                f32::from(fixed.unwrap_or(0)) / 127.0,
            ),
        ],
    }
}

/// A Filter's four Q-Links: the kinds each lets by or drops.
const FILTER_KNOBS: [(&str, &[MsgKind]); 4] = [
    ("Notes", &[MsgKind::Notes]),
    ("Bend, pressure", &[MsgKind::Bend, MsgKind::Pressure]),
    ("Controls", &[MsgKind::Controls]),
    ("Program", &[MsgKind::Program]),
];

/// `b` with Q-Link `knob` turned `d` detents; None when that Q-Link turns
/// nothing on it.
fn turn_block(b: Block, knob: usize, d: i32) -> Option<Block> {
    let step = |v: u8, lo: i32, hi: i32| (i32::from(v) + d).clamp(lo, hi) as u8;
    Some(match (b, knob) {
        (Block::Filter(drop), k) => {
            let (_, ks) = FILTER_KNOBS.get(k)?;
            let mut drop = drop;
            for kind in ks.iter() {
                drop = drop.with(*kind, d > 0);
            }
            Block::Filter(drop)
        }
        (Block::Channel(c), 0) => Block::Channel(step(c, 0, 15)),
        (Block::Split { low, high }, 0) => Block::Split {
            low: step(low, 0, i32::from(high)),
            high,
        },
        (Block::Split { low, high }, 1) => Block::Split {
            low,
            high: step(high, i32::from(low), 127),
        },
        (Block::Transpose(t), 0 | 1) => {
            let by = if knob == 1 { 12 * d } else { d };
            let m = i32::from(mpfree_midi::blocks::MAX_TRANSPOSE);
            Block::Transpose((i32::from(t) + by).clamp(-m, m) as i8)
        }
        (
            Block::Velocity {
                curve,
                low,
                high,
                fixed,
            },
            k,
        ) => {
            let (low, high, fixed, curve) = match k {
                0 => {
                    let at = Curve::ALL.iter().position(|c| *c == curve).unwrap_or(1) as i32;
                    let to = (at + d.signum()).clamp(0, 2) as usize;
                    (low, high, fixed, Curve::ALL[to])
                }
                1 => (step(low, 1, i32::from(high)), high, fixed, curve),
                2 => (low, step(high, i32::from(low), 127), fixed, curve),
                3 => {
                    // 0 is off.
                    let v = step(fixed.unwrap_or(0), 0, 127);
                    (low, high, (v > 0).then_some(v), curve)
                }
                _ => return None,
            };
            Block::Velocity {
                curve,
                low,
                high,
                fixed,
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, AREA};
    use mpfree_midi::setup::DIN_NAME;
    use std::sync::{Arc, Mutex};

    fn model() -> Shared {
        Arc::new(Mutex::new(crate::sample::rig()))
    }

    fn centre(r: Rect) -> (i32, i32) {
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn tap(s: &mut Routing, x: i32, y: i32) -> IoReply {
        s.touch(1, Touch::Down, x, y, AREA);
        s.touch(1, Touch::Up, x, y, AREA)
    }

    /// The square for `src` (a row's name) and `dest` (a column's name).
    fn square_at(s: &Routing, src: &str, dest: &str) -> Rect {
        let ts = s.targets(AREA);
        ts.iter()
            .find(|t| t.id == "routing-square" && t.label.starts_with(&format!("{src} → {dest}:")))
            .unwrap_or_else(|| {
                panic!(
                    "no square {src} → {dest} in {:?}",
                    ts.iter().map(|t| &t.label).collect::<Vec<_>>()
                )
            })
            .rect
    }

    fn target(s: &Routing, id: &str, index: usize) -> Rect {
        s.targets(AREA)
            .into_iter()
            .find(|t| t.id == id && t.index == index)
            .unwrap_or_else(|| panic!("no {id} {index}"))
            .rect
    }

    #[test]
    fn every_control_fits_the_section() {
        let m = model();
        let s = Routing::new(m.clone());
        let _ = paint(&s);
        let ts = s.targets(AREA);
        assert!(ts.len() > 20, "{} targets", ts.len());
        for t in ts {
            let r = t.rect;
            assert!(
                r.x >= AREA.x
                    && r.y >= AREA.y
                    && r.right() <= AREA.right()
                    && r.bottom() <= AREA.bottom(),
                "{} \"{}\" at {r:?} is outside {AREA:?}",
                t.id,
                t.label
            );
        }
    }

    /// The Armed track card shows the port's blocks: + Block adds one of
    /// a kind, a tap picks it for the Q-Links, › swaps two, × and -
    /// take one away; a route's line opens its own blocks.
    #[test]
    fn blocks_are_added_turned_swapped_and_taken_away_on_the_card() {
        let m = model();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let blocks = || m.lock().unwrap().setup.port(DIN_NAME).blocks.to_text();
        let (x, y) = centre(square_at(&s, "MIDI in", "Armed track"));
        tap(&mut s, x, y);
        let press = |s: &mut Routing, id: &str, i: usize| {
            let r = target(s, id, i);
            tap(s, r.x + r.w / 2, r.y + r.h / 2);
            let _ = paint(s);
        };
        press(&mut s, "routing-add-block", 0);
        let transpose = BlockKind::ALL
            .iter()
            .position(|k| *k == BlockKind::Transpose)
            .unwrap();
        press(&mut s, "routing-new-block", transpose);
        assert_eq!(blocks(), "transpose 0");
        // Picked: Q-Link 2 turns octaves, Q-Link 1 semitones.
        let q = s.qlinks().unwrap();
        assert_eq!(
            (q[0].label.as_str(), q[1].label.as_str()),
            ("Semitones", "Octaves")
        );
        assert_eq!(s.qlink_turn(1, -1, false), IoReply::Redraw);
        assert_eq!(s.qlink_turn(0, 2, false), IoReply::Redraw);
        assert_eq!(blocks(), "transpose -10");
        press(&mut s, "routing-add-block", 0);
        press(&mut s, "routing-new-block", 4);
        assert_eq!(
            s.qlink_turn(0, -1, false),
            IoReply::Redraw,
            "the curve: soft"
        );
        assert_eq!(s.qlink_turn(3, 90, false), IoReply::Redraw, "fixed at 90");
        assert_eq!(blocks(), "transpose -10; velocity fixed 90");
        assert_eq!(s.qlink_turn(3, -90, false), IoReply::Redraw);
        assert_eq!(blocks(), "transpose -10; velocity soft 1 127");
        press(&mut s, "routing-swap", 0);
        assert_eq!(blocks(), "velocity soft 1 127; transpose -10");
        assert!(s
            .targets(AREA)
            .iter()
            .any(|t| t.id == "routing-tile" && t.label == "Transpose: -10"));
        // The picked block followed its swap; × takes it away.
        press(&mut s, "routing-drop-block", 0);
        assert_eq!(blocks(), "transpose -10");
        press(&mut s, "routing-tile", 0);
        assert_eq!(s.input(IoInput::Step(-1)), IoReply::Redraw);
        assert_eq!(blocks(), "");
        // A Filter's Q-Links: what each kind does.
        press(&mut s, "routing-add-block", 0);
        press(&mut s, "routing-new-block", 0);
        assert_eq!(blocks(), "filter program");
        s.qlink_turn(1, 1, false);
        assert_eq!(blocks(), "filter pressure bend program");
        let q = s.qlinks().unwrap();
        assert_eq!(q[3].text, "drops");
        assert_eq!(q[0].text, "lets by");
        // A route's line opens its blocks, ‹ Routes closes them.
        let (x, y) = centre(square_at(&s, "MIDI in", "Tracks"));
        tap(&mut s, x, y);
        let _ = paint(&s);
        assert_eq!(m.lock().unwrap().setup.routes.len(), 1);
        press(&mut s, "routing-blocks", 0);
        press(&mut s, "routing-add-block", 0);
        press(&mut s, "routing-new-block", 1);
        s.qlink_turn(0, 9, false);
        assert_eq!(
            m.lock().unwrap().setup.routes[0].blocks.to_text(),
            "channel 10"
        );
        assert!(target(&s, "routing-main", 0).w > 0);
        assert!(s.targets(AREA).iter().all(|t| t.id != "routing-channel"));
        press(&mut s, "routing-main", 0);
        assert!(s.targets(AREA).iter().any(|t| t.id == "routing-channel"));
        // Every control of the blocks view fits, at every width.
        press(&mut s, "routing-blocks", 0);
        for area in [
            AREA,
            Rect::new(194, 136, 772, 614),
            Rect::new(184, 126, 912, 640),
        ] {
            for t in s.targets(area) {
                let r = t.rect;
                assert!(
                    r.x >= area.x && r.right() <= area.right() && r.bottom() <= area.bottom(),
                    "{} at {r:?} is outside {area:?}",
                    t.id
                );
            }
        }
    }

    /// The card's last line: the newest message on the port that went the
    /// route's way, and what its blocks made of it.
    #[test]
    fn the_last_line_says_what_went_through_and_what_it_became() {
        use mpfree_midi::monitor::Feed;
        let m = model();
        let mut v = m.lock().unwrap();
        let mut p = mpfree_midi::port::Ports::new();
        p.set_din(true);
        v.monitor.set_ports(p.all());
        let blocks = Blocks::parse("transpose -12; velocity fixed 104; channel 2").unwrap();
        v.setup.set_port(DIN_NAME, |p| p.blocks = blocks);
        let whose = Whose::Port(DIN_NAME.to_string());
        assert!(
            last_through(&v, &whose, DIN_NAME, blocks).is_none(),
            "nothing yet"
        );
        let mut f = Feed::new();
        f.bytes(mpfree_midi::DIN, Dir::In, 2.0, &[0x90, 60, 96]);
        f.to(To::Track {
            track: 1,
            pad: None,
        });
        f.bytes(mpfree_midi::DIN, Dir::In, 2.5, &[0xB0, 1, 9]);
        f.to(To::Learned(mpfree_param::ParamAddr::new(
            mpfree_param::Owner::Transport,
            0,
        )));
        f.drain_into(&mut v.monitor, 3.0);
        let said = last_through(&v, &whose, DIN_NAME, blocks).expect("the note");
        assert_eq!(msg_words(&said.came), "Note on C4 vel 96");
        let Through::Out(out) = said.went else {
            panic!("{:?}", said.went)
        };
        assert_eq!(out_words(&said.came, &out), "C3 vel 104 · ch 2");
        assert_eq!(said.where_to, "played the armed track");
        // A Filter that drops notes: the line says so.
        let drop = Blocks::parse("filter notes").unwrap();
        let said = last_through(&v, &whose, DIN_NAME, drop).unwrap();
        assert_eq!(said.went, Through::Dropped);
    }

    /// Each tile's words fit it, at the narrowest width; the line under
    /// the value takes two lines at most.
    #[test]
    fn a_tiles_words_fit_it() {
        let mut fonts = mpfree_ui::Fonts::embedded();
        let theme = mpfree_ui::Theme::default();
        let l = layout(AREA, 6);
        let tl = tiles(&l);
        let inner = tl.w - 24;
        let mut all: Vec<Block> = vec![
            Block::Filter(mpfree_midi::blocks::Kinds::NONE.with(MsgKind::Program, true)),
            Block::Filter(
                mpfree_midi::blocks::Kinds::NONE
                    .with(MsgKind::Program, true)
                    .with(MsgKind::Pressure, true),
            ),
            Block::Filter(
                mpfree_midi::blocks::Kinds::NONE
                    .with(MsgKind::Notes, true)
                    .with(MsgKind::Pressure, true)
                    .with(MsgKind::Bend, true),
            ),
            Block::Channel(15),
            Block::Split { low: 1, high: 127 },
            Block::Split { low: 61, high: 66 },
            Block::Transpose(-47),
            Block::Transpose(-36),
            Block::Velocity {
                curve: Curve::Linear,
                low: 100,
                high: 127,
                fixed: None,
            },
            Block::Velocity {
                curve: Curve::Hard,
                low: 1,
                high: 127,
                fixed: Some(127),
            },
        ];
        all.extend(BlockKind::ALL.iter().map(|k| k.new_block()));
        for b in all {
            let (value, under) = tile_text(&b);
            let px = theme.size_small * 0.8;
            let vw = fonts.measure(FontId::Bold, px, &value);
            assert!(vw <= inner, "{value:?} is {vw} wide, the tile {inner}");
            let words: Vec<&str> = under.split(' ').collect();
            let mut lines = 1;
            let mut line = String::new();
            for w in words {
                let longer = if line.is_empty() {
                    w.to_string()
                } else {
                    format!("{line} {w}")
                };
                if !line.is_empty()
                    && fonts.measure(FontId::Sans, theme.size_small * 0.85, &longer) > inner
                {
                    lines += 1;
                    line = w.to_string();
                } else {
                    line = longer;
                }
            }
            assert!(lines <= 2, "{under:?} takes {lines} lines");
        }
    }

    #[test]
    fn a_tap_on_tracks_by_channel_adds_a_route_and_the_card_edits_it() {
        let m = model();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let before = m.lock().unwrap().gen;
        let (x, y) = centre(square_at(&s, "MIDI in", "Tracks"));
        assert_eq!(tap(&mut s, x, y), IoReply::Redraw);
        {
            let v = m.lock().unwrap();
            assert!(v.gen > before);
            assert_eq!(
                v.setup.routes,
                vec![TrackRoute {
                    port: DIN_NAME.to_string(),
                    channel: Some(0),
                    track: 0,
                    blocks: mpfree_midi::blocks::Blocks::NONE,
                }]
            );
        }
        // Channel up to 10, track up to 2, on the card.
        let up = target(&s, "routing-channel", 1);
        for _ in 0..9 {
            tap(&mut s, up.x + 4, up.y + 4);
        }
        let tr = target(&s, "routing-track", 1);
        tap(&mut s, tr.x + 4, tr.y + 4);
        {
            let v = m.lock().unwrap();
            assert_eq!(v.setup.routes[0].channel, Some(9));
            assert_eq!(v.setup.routes[0].track, 1);
        }
        let _ = paint(&s);
        assert!(s
            .targets(AREA)
            .iter()
            .any(|t| t.label == "MIDI in → Tracks: Bass"));
        // The Q-Links turn the same route; + adds another, - takes the
        // picked one away.
        let q = s.qlinks().unwrap();
        assert_eq!(
            (q[0].label.as_str(), q[0].text.as_str()),
            ("Channel", "ch 10")
        );
        assert_eq!(s.input(IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(m.lock().unwrap().setup.routes.len(), 2);
        assert_eq!(s.input(IoInput::Step(-1)), IoReply::Redraw);
        assert_eq!(m.lock().unwrap().setup.routes.len(), 1);
        // Down past all channels says why.
        for _ in 0..20 {
            s.qlink_turn(0, -1, false);
        }
        assert_eq!(m.lock().unwrap().setup.routes[0].channel, None);
        assert!(s.take_note().unwrap().contains("first, all"));
    }

    #[test]
    fn the_armed_track_and_learn_squares_are_the_ports_input_and_never_both_off() {
        let m = model();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let input = || m.lock().unwrap().setup.port(DIN_NAME).input;
        assert_eq!(input(), PortInput::Both);
        let (x, y) = centre(square_at(&s, "MIDI in", "Armed track"));
        tap(&mut s, x, y);
        let main = target(&s, "routing-main", 0);
        tap(&mut s, main.x + 4, main.y + 4);
        assert_eq!(input(), PortInput::LearnOnly);
        let (x, y) = centre(square_at(&s, "MIDI in", "Learn"));
        tap(&mut s, x, y);
        assert_eq!(s.input(IoInput::Step(-1)), IoReply::Handled);
        assert_eq!(input(), PortInput::LearnOnly, "never neither");
        assert!(s.take_note().unwrap().contains("turn the other on first"));
        // A tap on the empty Armed track square turns it back on.
        let (x, y) = centre(square_at(&s, "MIDI in", "Armed track"));
        tap(&mut s, x, y);
        assert_eq!(input(), PortInput::Both);
    }

    #[test]
    fn a_track_square_under_a_port_is_its_plays_on() {
        let m = model();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let name = track_name(&m.lock().unwrap(), 5);
        // Rows past the first screen are reached by the wheel.
        for _ in 0..200 {
            if s.targets(AREA)
                .iter()
                .any(|t| t.label.starts_with(&format!("{name} → MIDI out:")))
            {
                break;
            }
            s.input(IoInput::Page(1));
            let _ = paint(&s);
        }
        let (x, y) = centre(square_at(&s, &name, "MIDI out"));
        tap(&mut s, x, y);
        assert_eq!(
            m.lock().unwrap().setup.tracks[5],
            Some(TrackOut::new(DIN_NAME, 0))
        );
        let up = target(&s, "routing-out-channel", 1);
        tap(&mut s, up.x + 4, up.y + 4);
        assert_eq!(
            m.lock().unwrap().setup.tracks[5].as_ref().unwrap().channel,
            1
        );
        let main = target(&s, "routing-main", 0);
        tap(&mut s, main.x + 4, main.y + 4);
        assert_eq!(m.lock().unwrap().setup.tracks[5], None);
    }

    #[test]
    fn squares_where_nothing_goes_say_why() {
        let m = model();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let (x, y) = centre(square_at(&s, "The pads", "MIDI out"));
        tap(&mut s, x, y);
        assert!(s.take_note().unwrap().contains("by the gear's profile"));
        let (x, y) = centre(square_at(&s, "The pads", "CV out"));
        tap(&mut s, x, y);
        assert!(s.take_note().is_some());
    }

    /// A tap on a port's square under an output sends all its channels
    /// there; the card narrows it to a channel and adds more, and a USB
    /// device's own output is never offered.
    #[test]
    fn a_named_ports_row_says_its_name_and_its_routes_keep_its_own() {
        let m = model();
        m.lock()
            .unwrap()
            .setup
            .set_shown("Keystep 37", "Lead keys", [])
            .unwrap();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let (x, y) = centre(square_at(&s, "Lead keys", "MIDI out"));
        assert_eq!(tap(&mut s, x, y), IoReply::Redraw);
        let thru = m.lock().unwrap().setup.thru.clone();
        assert_eq!(thru[0].from, "Keystep 37", "the setup names it by its own");
        let _ = paint(&s);
        assert!(s
            .targets(AREA)
            .iter()
            .any(|t| t.label == "Lead keys → MIDI out: thru"));
    }

    #[test]
    fn a_tap_under_an_output_sends_the_port_thru_and_the_card_picks_its_channels() {
        let m = model();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let (x, y) = centre(square_at(&s, "Keystep 37", "MIDI out"));
        assert_eq!(tap(&mut s, x, y), IoReply::Redraw);
        let thru = |m: &Shared| m.lock().unwrap().setup.thru.clone();
        assert_eq!(
            thru(&m),
            vec![ThruRoute {
                from: "Keystep 37".to_string(),
                channel: None,
                to: DIN_NAME.to_string(),
                blocks: mpfree_midi::blocks::Blocks::NONE,
            }]
        );
        // MIDI out is off in the sample rig: the square says so.
        let _ = paint(&s);
        assert!(s
            .targets(AREA)
            .iter()
            .any(|t| t.label == "Keystep 37 → MIDI out: thru"));
        assert_eq!(
            square(
                &m.lock().unwrap(),
                &Src::Port {
                    name: "Keystep 37".into(),
                    plugged: true
                },
                &Dest::Out {
                    name: DIN_NAME.into(),
                    plugged: true
                }
            )
            .sub,
            "MIDI out is off"
        );
        // + on the card while every channel goes is refused; channel 10
        // only, then + adds channel 1.
        assert_eq!(s.input(IoInput::Step(1)), IoReply::Handled);
        assert!(s.take_note().unwrap().contains("already"));
        let up = target(&s, "routing-channel", 1);
        for _ in 0..10 {
            tap(&mut s, up.x + 4, up.y + 4);
        }
        assert_eq!(thru(&m)[0].channel, Some(9));
        assert_eq!(s.input(IoInput::Step(1)), IoReply::Redraw);
        assert_eq!(thru(&m)[1].channel, Some(0));
        m.lock().unwrap().live.built.notes_out = true;
        let sq = square(
            &m.lock().unwrap(),
            &Src::Port {
                name: "Keystep 37".into(),
                plugged: true,
            },
            &Dest::Out {
                name: DIN_NAME.into(),
                plugged: true,
            },
        );
        assert_eq!((sq.text.as_str(), sq.sub.as_str()), ("thru", "ch 10, ch 1"));
        // The Q-Link turns the picked one; the same channel twice is
        // refused; Remove route takes the picked one away.
        assert_eq!(s.qlinks().unwrap()[0].text, "ch 1");
        for _ in 0..9 {
            s.qlink_turn(0, 1, false);
        }
        assert!(s.take_note().unwrap().contains("already"));
        assert_eq!(thru(&m)[1].channel, Some(8));
        let rm = target(&s, "routing-remove", 0);
        tap(&mut s, rm.x + 4, rm.y + 4);
        assert_eq!(thru(&m).len(), 1);
        // A device's own output: nothing can go there (its column is past
        // the grid's room here, as on the MPC's screen).
        let keys = Src::Port {
            name: "Keystep 37".into(),
            plugged: true,
        };
        let own = Dest::Out {
            name: "Keystep 37".into(),
            plugged: true,
        };
        assert_eq!(square(&m.lock().unwrap(), &keys, &own).kind, Kind::None);
        assert!(why_not_here(&IoModel::default(), &keys, &own).contains("never go back to it"));
        // MIDI in to MIDI out is two pieces of gear: allowed.
        let (x, y) = centre(square_at(&s, "MIDI in", "MIDI out"));
        tap(&mut s, x, y);
        assert_eq!(thru(&m).len(), 2);
    }
}
