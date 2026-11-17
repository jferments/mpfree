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
//! - A tap on an empty square adds that route and picks it; a tap on a
//!   set one picks it. The card under the grid shows the picked square:
//!   its On and Off, and for Tracks by channel each route with its channel
//!   and track. The wheel moves across the squares, - and + take a route
//!   away or add it, and the Q-Links turn the picked route's channel and
//!   track.
//! - Not yet: thru from a port to an output, the pads to an output
//!   (drum pads out), and each route's blocks. Their squares say so.

use std::cell::Cell;

use mpfree_midi::cv::CvAssign;
use mpfree_midi::setup::{PortInput, TrackOut, TrackRoute};
use mpfree_mixer_ui::{IoInput, IoKnob, IoReply, IoSection};
use mpfree_ui::{Align, Color, FontId, PaintCx, Rect, Target, Touch};

use crate::{IoModel, Shared};

const HEAD_H: i32 = 30;
const GAP: i32 = 10;
const NAME_W: i32 = 150;
const COL_HEAD_H: i32 = 42;
const ROW_H: i32 = 46;
const COL_MIN: i32 = 96;
const PANEL_H: i32 = 196;
const BUTTON_W: i32 = 132;
const BUTTON_H: i32 = 36;
const LINE_H: i32 = 20;
const STEP_W: i32 = 40;
const ROUTE_H: i32 = 40;
/// Route lines the card shows at once.
const ROUTE_LINES: usize = 3;

pub(crate) const HINT: &str = "Tap a square: add or pick a route  ·  wheel: move  ·  - and +: take away or add  ·  Q-Links: channel, track";

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
    /// Not built yet.
    Later,
}

#[derive(Clone, Debug, PartialEq)]
struct Square {
    kind: Kind,
    text: String,
    sub: String,
    /// The route waits for its device, or won't send now.
    waits: bool,
}

impl Square {
    fn set(text: impl Into<String>, sub: impl Into<String>) -> Square {
        Square {
            kind: Kind::Set,
            text: text.into(),
            sub: sub.into(),
            waits: false,
        }
    }

    fn empty() -> Square {
        Square {
            kind: Kind::Empty,
            text: "+".to_string(),
            sub: String::new(),
            waits: false,
        }
    }

    fn none() -> Square {
        Square {
            kind: Kind::None,
            text: "—".to_string(),
            sub: String::new(),
            waits: false,
        }
    }

    fn later() -> Square {
        Square {
            kind: Kind::Later,
            text: "later".to_string(),
            sub: String::new(),
            waits: false,
        }
    }

    fn waits(mut self, w: bool) -> Square {
        self.waits = w;
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
            let routed = !routes_from(m, name).is_empty();
            Square::set(
                if routed { "the rest" } else { "all channels" },
                if *plugged { "plays it" } else { "waits for it" },
            )
            .waits(!plugged)
        }
        (Src::Port { name, plugged }, Dest::ByChannel) => {
            let rs = routes_from(m, name);
            let sub = if *plugged { "" } else { "waits for it" };
            match rs.as_slice() {
                [] => Square::empty(),
                [i] => {
                    let r = &m.setup.routes[*i];
                    Square::set(
                        track_name(m, r.track),
                        if *plugged {
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
                        if *plugged {
                            chans.join(", ")
                        } else {
                            sub.to_string()
                        },
                    )
                    .waits(!plugged)
                }
            }
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
        (Src::Port { .. }, Dest::Out { .. }) => Square::later(),
        (Src::Port { .. }, Dest::Cv) => Square::none(),
        (Src::Pads, Dest::Armed) => Square::set("always", "built in"),
        (Src::Pads, Dest::Out { .. }) => Square::later(),
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
        if l.main.contains(x, y) && has_main(src, dest) {
            return Some(Hit::Main);
        }
        if l.remove.contains(x, y) && *dest == Dest::ByChannel {
            return Some(Hit::Remove);
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
                                "{name} would neither play the armed track nor be heard by Learn: \
                                 turn the other on first"
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
                            return Err(format!("Every channel of {name} has a route"));
                        };
                        let taken = |t: u8| mine.iter().any(|&i| m.setup.routes[i].track == t);
                        let track = (0..tracks as u8).find(|&t| !taken(t)).unwrap_or(0);
                        m.setup.routes.push(TrackRoute {
                            port: name.clone(),
                            channel: Some(channel),
                            track,
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
                            return Err(format!("{name} has no route to take away"));
                        };
                        m.setup.routes.remove(i);
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
                            "It {} on {name} already",
                            if on { "plays" } else { "doesn't play" }
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
                self.note = Some(why_not_here(&src, &dest));
                false
            }
        };
        if changed {
            IoReply::Redraw
        } else {
            IoReply::Handled
        }
    }

    /// Steps the picked route's channel (all, then 1 to 16).
    fn route_channel(&mut self, k: usize, d: i32) -> IoReply {
        let Some((Src::Port { name, .. }, Dest::ByChannel)) = self.picked_now() else {
            return IoReply::Handled;
        };
        self.route = k;
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
            let o = m
                .setup
                .tracks
                .get_mut(usize::from(t))
                .and_then(|o| o.as_mut())
                .filter(|o| o.port == name)
                .ok_or_else(|| format!("It doesn't play on {name}: + or a tap sets it"))?;
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
                    Some((sq, s, d)) if matches!(sq.kind, Kind::None | Kind::Later) => {
                        self.note = Some(why_not_here(&s, &d));
                        IoReply::Redraw
                    }
                    _ => IoReply::Redraw,
                }
            }
            Hit::Main => {
                let on = {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    match self.picked(&m) {
                        Some((s, Dest::ByChannel)) => {
                            let _ = s;
                            true
                        }
                        Some((s, d)) => square(&m, &s, &d).kind != Kind::Set,
                        None => true,
                    }
                };
                self.switch(on).max(IoReply::Redraw)
            }
            Hit::Remove => self.switch(false).max(IoReply::Redraw),
            Hit::Route(k) => {
                self.route = k;
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
        .any(|(j, o)| j != i && *o == r)
    {
        return Err(format!(
            "{} → {} is a route already",
            r.source(),
            track_name(m, r.track)
        ));
    }
    m.setup.routes[i] = r;
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
    matches!(
        (src, dest),
        (
            Src::Port { .. },
            Dest::Armed | Dest::ByChannel | Dest::Learn
        ) | (Src::Track(_), Dest::Out { .. })
    )
}

/// Why nothing can be set on a square, in words.
fn why_not_here(src: &Src, dest: &Dest) -> String {
    match (src, dest) {
        (Src::Port { name, .. }, Dest::Out { name: out, .. }) => {
            format!("Thru from {name} to {out} comes later")
        }
        (Src::Port { .. }, Dest::Cv) => {
            "CV outputs follow a track: pick its job and track in the CV section".to_string()
        }
        (Src::Pads, Dest::Armed) => "The pads always play the armed track".to_string(),
        (Src::Pads, Dest::Out { .. }) => {
            "The pads to an output (drum pads out) comes later".to_string()
        }
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
                    name.clone()
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

fn dest_text(d: &Dest) -> (String, String) {
    match d {
        Dest::Armed => ("Armed track".to_string(), "on screen".to_string()),
        Dest::ByChannel => ("Tracks".to_string(), "by channel".to_string()),
        Dest::Learn => ("Learn".to_string(), "moves values".to_string()),
        Dest::Out { name, plugged } if name == mpfree_midi::setup::DIN_NAME => (
            "MIDI out".to_string(),
            if *plugged { "5-pin" } else { "unplugged" }.to_string(),
        ),
        Dest::Out { name, plugged } => (
            name.clone(),
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
            cx.canvas.fill_round_rect(r, 8, t.raised);
            let bar = if inward { t.good } else { t.accent };
            let bar = if sq.waits { t.text_faint } else { bar };
            cx.canvas
                .fill_round_rect(Rect::new(r.x, r.y + 4, 3, r.h - 8), 1, bar);
        }
        Kind::Empty => {
            cx.canvas.stroke_round_rect(r, 8, 1, t.outline);
        }
        Kind::None | Kind::Later => {}
    }
    if picked {
        cx.canvas.stroke_round_rect(r, 8, 2, t.accent);
    }
    let ink = match sq.kind {
        Kind::Set if sq.waits => t.text_dim,
        Kind::Set => t.text,
        Kind::Empty => t.text_dim,
        Kind::None | Kind::Later => t.text_faint,
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
    cx.canvas.fill_round_rect(r, 10, fill);
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
        cx.canvas.fill_round_rect(r, 8, t.raised);
        cx.text(FontId::Bold, t.size_body, r, Align::Center, t.text, s);
    }
    let mid = Rect::new(
        minus.right() + 4,
        minus.y,
        plus.x - minus.right() - 8,
        minus.h,
    );
    cx.canvas.fill_round_rect(mid, 8, t.bg);
    if picked {
        cx.canvas.stroke_round_rect(mid, 8, 2, t.accent);
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
        cx.canvas.fill_round_rect(l.panel, 10, t.surface);
        let (s, _) = src_text(m, src);
        let (d, _) = dest_text(dest);
        let title = format!("{} → {}", s, d).to_uppercase();
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
                            crate::control_text(&l.source, l.channel, name)
                        ));
                    }
                }
                say.push("Kept for every project (your rig).".to_string());
                if !plugged {
                    say.push(format!("{name} isn't plugged in: kept for when it is."));
                }
                lines(cx, l.lines, &say, t.text_dim);
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
                        cx.canvas.fill_round_rect(line, 8, t.raised);
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
                    let say = Rect::new(
                        steps[3].right() + 12,
                        line.y,
                        line.right() - steps[3].right() - 16,
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
                lines(cx, l.lines, &[why_not_here(src, dest)], t.text_dim);
            }
        }
    }
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
        cx.canvas.fill_round_rect(l.grid, 10, t.surface);
        for (c, d) in g.cols.iter().enumerate() {
            let x = l.body.x + NAME_W + c as i32 * l.col_w;
            let (name, sub) = dest_text(d);
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
                let (dn, _) = dest_text(d);
                out.push(Target::tap(
                    "routing-square",
                    row * g.cols.len() + col,
                    format!("{sn} → {dn}: {}", sq.text),
                    sr,
                ));
            }
        }
        if let (Some(s), Some(d)) = (g.rows.get(self.pick.0), g.cols.get(self.pick.1)) {
            if has_main(s, d) {
                out.push(Target::tap("routing-main", 0, "On, Off or + Route", l.main));
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
                IoReply::Redraw
            }
            IoInput::Page(d) => {
                let to = (self.pick.0 as i32 + d * 4).clamp(0, rows as i32 - 1) as usize;
                if to == self.pick.0 {
                    self.note = Some("No more rows that way".to_string());
                    return IoReply::Handled;
                }
                self.pick.0 = to;
                IoReply::Redraw
            }
            IoInput::Step(d) if d != 0 => self.switch(d > 0),
            IoInput::Step(_) => IoReply::Handled,
        }
    }

    fn tick(&mut self, _now: std::time::Duration) -> IoReply {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        if m.gen != self.gen {
            self.gen = m.gen;
            return IoReply::Redraw;
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
        match (self.picked_now(), knob) {
            (Some((Src::Port { .. }, Dest::ByChannel)), 0) => {
                self.route_channel(self.route, detents)
            }
            (Some((Src::Port { .. }, Dest::ByChannel)), 1) => {
                self.route_track(self.route, detents.signum())
            }
            (Some((Src::Track(_), Dest::Out { .. })), 0) => self.track_channel(detents),
            _ => IoReply::Ignored,
        }
    }
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
                    track: 0
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
    fn squares_that_arent_built_say_so() {
        let m = model();
        let mut s = Routing::new(m.clone());
        let _ = paint(&s);
        let (x, y) = centre(square_at(&s, "MIDI in", "MIDI out"));
        tap(&mut s, x, y);
        assert_eq!(s.take_note().unwrap(), "Thru from MIDI to MIDI comes later");
        let (x, y) = centre(square_at(&s, "The pads", "CV out"));
        tap(&mut s, x, y);
        assert!(s.take_note().is_some());
    }
}
