//! The Monitor section: every message in and out of the MIDI ports, as it
//! happens ([`mpfree_midi::monitor`] is its model).
//!
//! - **Chips** along the top pick what is listed (Notes, CC, Program,
//!   Bend and pressure, Clock, SysEx) and which way (In, Out). Hold stops
//!   the list where it is; Clear empties it.
//! - **Lights:** a row per port and a light per channel, the upper half in
//!   and the lower half out, lit for a quarter second after a message, and
//!   the tempo of a clock coming in or going out. A tap on a light lists
//!   only that port, way and channel; a tap on a port's name, only that
//!   port and way. The same tap again lists everything.
//! - **The list,** newest first: when, the port and way, the channel, the
//!   message in words and its bytes. Clock, active sensing and time code
//!   are counted, not listed: a line when each starts and stops. The data
//!   wheel and the page arrows scroll back, and the list holds while it is
//!   scrolled back; scrolled to the top again, it follows.

use std::cell::Cell;
use std::time::Duration;

use mpfree_midi::monitor::{Dir, Filter, Kind, Monitor, Seen, Steady, What};
use mpfree_midi::{Msg, PortId};
use mpfree_mixer_ui::{IoInput, IoReply, IoSection};
use mpfree_ui::{Align, FontId, PaintCx, Rect, Target, Touch};

use crate::form::{BANNER_H, GAP};
use crate::{note_name, IoModel, Shared};

/// The chips' labels, in [`Kind::ALL`] order.
const KIND_CHIPS: [(&str, i32); 6] = [
    ("Notes", 78),
    ("CC", 56),
    ("Program", 96),
    ("Bend & pressure", 158),
    ("Clock", 74),
    ("SysEx", 78),
];
const DIR_CHIPS: [(&str, i32, Dir); 2] = [("In", 52, Dir::In), ("Out", 60, Dir::Out)];
const BUTTON_W: i32 = 84;
const CHIP_H: i32 = 44;
const CHIP_GAP: i32 = 6;

const GRID_HEAD_H: i32 = 22;
const PORT_H: i32 = 40;
const PORT_GAP: i32 = 6;
const NAME_W: i32 = 170;
const CLOCK_W: i32 = 124;

const LIST_HEAD_H: i32 = 30;
pub(crate) const LINE_H: i32 = 30;
const TIME_W: i32 = 104;
const PORT_COL_W: i32 = 200;
const CH_W: i32 = 70;
const BYTES_W: i32 = 120;

/// The hint line while the Monitor shows.
pub(crate) const HINT: &str = "Tap a light: only that port and channel  ·  wheel: scroll back  ·  Hold stops the list  ·  the I/O icon or TRACK MIX closes";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hit {
    Kind(Kind),
    Dir(Dir),
    Hold,
    Clear,
    Port(PortId, Dir),
    Light(PortId, Dir, u8),
    List,
}

struct Layout {
    banner: Option<Rect>,
    chips: Vec<(Hit, Rect)>,
    grid_head: Rect,
    /// Each port shown and its row (the in half on top).
    ports: Vec<(PortId, Rect)>,
    list_head: Rect,
    list: Rect,
}

fn halves(row: Rect) -> [(Dir, Rect); 2] {
    let h = (row.h - 4) / 2;
    [
        (Dir::In, Rect::new(row.x, row.y, row.w, h)),
        (Dir::Out, Rect::new(row.x, row.y + h + 4, row.w, h)),
    ]
}

/// The channel lights' areas in a port's row: 16, then the clock's.
fn cells(row: Rect) -> ([Rect; 16], Rect) {
    let x0 = row.x + NAME_W;
    let clock = Rect::new(row.x + row.w - CLOCK_W, row.y, CLOCK_W, row.h);
    let w = (clock.x - 12 - x0) / 16;
    (
        std::array::from_fn(|c| Rect::new(x0 + c as i32 * w, row.y, w - 4, row.h)),
        clock,
    )
}

/// `t` seconds on the app's clock as minutes, seconds and milliseconds.
fn clock_text(t: f64) -> String {
    let t = t.max(0.0);
    let m = (t / 60.0) as u64;
    format!("{m}:{:06.3}", t - m as f64 * 60.0)
}

fn way(d: Dir) -> &'static str {
    match d {
        Dir::In => "in",
        Dir::Out => "out",
    }
}

/// A port's name and way, as the list says it ("Keystep 37 in").
fn port_text(m: &Monitor, port: PortId, d: Dir) -> String {
    match m.port_name(port) {
        Some(n) => format!("{n} {}", way(d)),
        None => format!("Port {} {}", port.0 + 1, way(d)),
    }
}

/// The controllers most gear agrees on.
fn cc_name(cc: u8) -> Option<&'static str> {
    Some(match cc {
        0 => "Bank select",
        1 => "Mod wheel",
        2 => "Breath",
        4 => "Foot",
        5 => "Glide time",
        6 => "Data entry",
        7 => "Volume",
        8 => "Balance",
        10 => "Pan",
        11 => "Expression",
        32 => "Bank select fine",
        64 => "Sustain",
        65 => "Glide",
        66 => "Sostenuto",
        67 => "Soft pedal",
        71 => "Resonance",
        74 => "Cutoff",
        120 => "All sound off",
        121 => "Reset controllers",
        122 => "Local control",
        123 => "All notes off",
        124 => "Omni off",
        125 => "Omni on",
        126 => "Mono",
        127 => "Poly",
        _ => return None,
    })
}

/// A SysEx maker's name, for the ones seen on stages most.
fn maker_name(id: [u8; 3]) -> Option<&'static str> {
    Some(match id {
        [0x01, ..] => "Sequential",
        [0x04, ..] => "Moog",
        [0x41, ..] => "Roland",
        [0x42, ..] => "Korg",
        [0x43, ..] => "Yamaha",
        [0x47, ..] => "Akai",
        [0x7D, ..] => "non-commercial",
        [0x7E, ..] => "universal",
        [0x7F, ..] => "universal real-time",
        [0, 0x20, 0x29] => "Novation",
        [0, 0x20, 0x32] => "Behringer",
        [0, 0x20, 0x3C] => "Elektron",
        [0, 0x20, 0x6B] => "Arturia",
        _ => return None,
    })
}

fn steady_name(s: Steady) -> &'static str {
    match s {
        Steady::Clock => "Clock",
        Steady::Sensing => "Active sensing",
        Steady::TimeCode => "Time code",
    }
}

/// A message in words, without its port or channel ("Note on · C3 ·
/// vel 100"). Notes are named as everywhere in the app (48 is C3);
/// programs and songs count from 1.
pub fn words(s: &Seen) -> String {
    match s.what {
        What::Channel { msg, .. } => match msg {
            Msg::NoteOn { note, vel, .. } => {
                format!("Note on · {} · vel {vel}", note_name(note))
            }
            Msg::NoteOff { note, .. } => format!("Note off · {}", note_name(note)),
            Msg::PolyPressure { note, value, .. } => {
                format!("Pressure · {} · {value}", note_name(note))
            }
            Msg::Cc { cc, value, .. } => match cc_name(cc) {
                Some(n) => format!("CC {cc} {n} · {value}"),
                None => format!("CC {cc} · {value}"),
            },
            Msg::Program { program, .. } => format!("Program {}", u16::from(program) + 1),
            Msg::ChanPressure { value, .. } => format!("Channel pressure · {value}"),
            Msg::PitchBend { value, .. } => format!("Pitch bend · {value:+}"),
        },
        What::Start => "Start".into(),
        What::Continue => "Continue".into(),
        What::Stop => "Stop".into(),
        What::Reset => "System reset".into(),
        What::SongPosition(p) => format!(
            "Song position · bar {}.{}.{}",
            p / 16 + 1,
            p / 4 % 4 + 1,
            p % 4 + 1
        ),
        What::SongSelect(n) => format!("Song select · {}", u16::from(n) + 1),
        What::TuneRequest => "Tune request".into(),
        What::SysEx { len, maker } => match maker_name(maker) {
            Some(n) => format!("SysEx · {len} bytes · {n}"),
            None if maker[0] == 0 => format!(
                "SysEx · {len} bytes · maker 00 {:02X} {:02X}",
                maker[1], maker[2]
            ),
            None => format!("SysEx · {len} bytes · maker {:02X}", maker[0]),
        },
        What::Arriving(k) => format!(
            "{} {} (counted, not listed)",
            steady_name(k),
            match s.dir {
                Dir::In => "arriving",
                Dir::Out => "going out",
            }
        ),
        What::Stopped(k, n) => {
            let what = match k {
                Steady::Clock => "ticks",
                Steady::Sensing => "messages",
                Steady::TimeCode => "quarter frames",
            };
            format!("{} stopped · {n} {what}", steady_name(k))
        }
        What::Undefined(b) => format!("Undefined {b:02X}"),
    }
}

/// The bytes a line shows: a channel message's own, a SysEx's start.
fn bytes_text(w: &What) -> String {
    let (b, n) = w.bytes();
    let mut s = b[..n]
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ");
    if matches!(w, What::SysEx { .. }) {
        s.push_str(" … F7");
    }
    s
}

pub(crate) struct MonitorSection {
    model: Shared,
    filter: Filter,
    /// The newest message listed while the list holds; None: it follows.
    anchor: Option<u64>,
    /// Hold was tapped (scrolled back to the top, it still holds).
    held: bool,
    now: Duration,
    live: u64,
    down: Option<(u64, Option<Hit>)>,
    /// Lines that fit, from the last paint.
    fit: Cell<usize>,
}

impl MonitorSection {
    pub fn new(model: Shared) -> MonitorSection {
        MonitorSection {
            model,
            filter: Filter::ALL,
            anchor: None,
            held: false,
            now: Duration::ZERO,
            live: u64::MAX,
            down: None,
            fit: Cell::new(10),
        }
    }

    fn layout(&self, m: &IoModel, r: Rect) -> Layout {
        let mut y = r.y;
        let banner = (!m.live.built.monitor).then(|| {
            let b = Rect::new(r.x, y, r.w, BANNER_H);
            y += BANNER_H + GAP;
            b
        });
        let mut chips = Vec::new();
        let mut x = r.x;
        for (k, (_, w)) in Kind::ALL.iter().zip(KIND_CHIPS) {
            chips.push((Hit::Kind(*k), Rect::new(x, y, w, CHIP_H)));
            x += w + CHIP_GAP;
        }
        // The ways, Hold and Clear follow on the same line when they fit
        // (a full-width section), otherwise on a line of their own (beside
        // the mixer's master strip).
        let dirs_w: i32 = DIR_CHIPS.iter().map(|c| c.1 + CHIP_GAP).sum();
        let buttons_w = 2 * BUTTON_W + CHIP_GAP;
        if x + 14 + dirs_w + buttons_w > r.x + r.w {
            y += CHIP_H + 8;
            x = r.x;
        } else {
            x += 14;
        }
        for (_, w, d) in DIR_CHIPS {
            chips.push((Hit::Dir(d), Rect::new(x, y, w, CHIP_H)));
            x += w + CHIP_GAP;
        }
        let right = r.x + r.w;
        chips.push((Hit::Clear, Rect::new(right - BUTTON_W, y, BUTTON_W, CHIP_H)));
        chips.push((
            Hit::Hold,
            Rect::new(right - 2 * BUTTON_W - CHIP_GAP, y, BUTTON_W, CHIP_H),
        ));
        y += CHIP_H + 12;
        let grid_head = Rect::new(r.x, y, r.w, GRID_HEAD_H);
        y += GRID_HEAD_H;
        let mut ports = Vec::new();
        for p in m.monitor.shown_ports() {
            ports.push((p.id, Rect::new(r.x, y, r.w, PORT_H)));
            y += PORT_H + PORT_GAP;
        }
        y += 8;
        let list_head = Rect::new(r.x, y, r.w, LIST_HEAD_H);
        y += LIST_HEAD_H;
        let list = Rect::new(r.x, y, r.w, (r.y + r.h - y).max(0));
        Layout {
            banner,
            chips,
            grid_head,
            ports,
            list_head,
            list,
        }
    }

    fn hit(&self, x: i32, y: i32, r: Rect) -> Option<Hit> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = self.layout(&m, r);
        if let Some((h, _)) = l.chips.iter().find(|c| c.1.contains(x, y)) {
            return Some(*h);
        }
        for &(port, row) in &l.ports {
            if !row.contains(x, y) {
                continue;
            }
            for (d, half) in halves(row) {
                // Each half's whole height, the gap shared out.
                let band = match d {
                    Dir::In => Rect::new(half.x, row.y, half.w, row.h / 2),
                    Dir::Out => Rect::new(half.x, row.y + row.h / 2, half.w, row.h - row.h / 2),
                };
                if !band.contains(x, y) {
                    continue;
                }
                if x < half.x + NAME_W {
                    return Some(Hit::Port(port, d));
                }
                let (lights, _) = cells(half);
                return lights
                    .iter()
                    .position(|c| x >= c.x && x < c.x + c.w + 4)
                    .map(|c| Hit::Light(port, d, c as u8));
            }
        }
        l.list.contains(x, y).then_some(Hit::List)
    }

    /// The messages listed now, newest first, with how many newer ones
    /// are held back.
    fn lines(&self, m: &Monitor, most: usize) -> (Vec<(u64, Seen)>, usize) {
        let mut newer = 0;
        let mut out = Vec::new();
        for (n, s) in m.newest().filter(|(_, s)| self.filter.matches(s)) {
            if self.anchor.is_some_and(|a| n > a) {
                newer += 1;
                continue;
            }
            if out.len() < most {
                out.push((n, *s));
            } else {
                break;
            }
        }
        (out, newer)
    }

    /// Scrolls the list by `d` lines (positive: back to older ones).
    fn scroll(&mut self, d: i32) -> IoReply {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let list: Vec<u64> = m
            .monitor
            .newest()
            .filter(|(_, s)| self.filter.matches(s))
            .map(|(n, _)| n)
            .collect();
        drop(m);
        if list.is_empty() {
            return IoReply::Handled;
        }
        let at = match self.anchor {
            None => 0,
            Some(a) => list.iter().position(|&n| n <= a).unwrap_or(list.len() - 1),
        };
        let last = list.len().saturating_sub(self.fit.get().max(1));
        let to = (at as i64 + d as i64).clamp(0, last as i64) as usize;
        let anchor = if to == 0 && !self.held {
            None
        } else {
            Some(list[to])
        };
        if anchor == self.anchor {
            return IoReply::Handled;
        }
        self.anchor = anchor;
        IoReply::Redraw
    }

    fn tap(&mut self, h: Hit) -> IoReply {
        let f = &mut self.filter;
        match h {
            Hit::Kind(k) => f.kinds ^= k.bit(),
            Hit::Dir(d) => f.dirs ^= Filter::dir_bit(d),
            Hit::Hold => {
                if self.anchor.is_some() {
                    self.anchor = None;
                    self.held = false;
                } else {
                    let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                    self.anchor = Some(m.monitor.total().saturating_sub(1));
                    self.held = true;
                }
            }
            Hit::Clear => {
                let mut m = self.model.lock().unwrap_or_else(|e| e.into_inner());
                m.monitor.clear();
                self.anchor = None;
                self.held = false;
            }
            Hit::Port(p, d) => {
                let only = Filter::dir_bit(d);
                if f.port == Some(p) && f.channel.is_none() && f.dirs == only {
                    (f.port, f.dirs) = (None, 0b11);
                } else {
                    (f.port, f.channel, f.dirs) = (Some(p), None, only);
                }
            }
            Hit::Light(p, d, c) => {
                let only = Filter::dir_bit(d);
                if f.port == Some(p) && f.channel == Some(c) && f.dirs == only {
                    (f.port, f.channel, f.dirs) = (None, None, 0b11);
                } else {
                    (f.port, f.channel, f.dirs) = (Some(p), Some(c), only);
                }
            }
            Hit::List => return IoReply::Handled,
        }
        IoReply::Redraw
    }

    /// A number that changes when anything the screen shows changes.
    fn live_now(&self, m: &IoModel, now: f64) -> u64 {
        let mon = &m.monitor;
        let mut h = mon.total() ^ (mon.len() as u64) << 32 ^ mon.lost << 48;
        for p in mon.shown_ports() {
            for d in [Dir::In, Dir::Out] {
                let mut bits = 0u64;
                for c in 0..16u8 {
                    bits |= u64::from(mon.lit(p.id, d, c, now)) << c;
                }
                bits |= u64::from(mon.sys_lit(p.id, d, now)) << 16;
                let bpm = mon
                    .bpm(p.id, d)
                    .map_or(0, |b| (b * 10.0).round() as u64 + 1);
                h = h.rotate_left(13) ^ bits ^ bpm << 17;
            }
        }
        h ^ u64::from(m.live.built.monitor)
    }

    fn chip(cx: &mut PaintCx, r: Rect, text: &str, on: bool) {
        let t = cx.theme.clone();
        let (fill, ink) = if on {
            (t.accent, t.on_accent)
        } else {
            (t.surface, t.text_dim)
        };
        cx.canvas.fill_round_rect(r, 10, fill);
        cx.text(FontId::Bold, t.size_small, r, Align::Center, ink, text);
    }

    fn paint_lights(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout, now: f64) {
        let t = cx.theme.clone();
        let mon = &m.monitor;
        if let Some(&(_, first)) = l.ports.first() {
            let (lights, clock) = cells(first);
            for (c, cr) in lights.iter().enumerate() {
                let hr = Rect::new(cr.x, l.grid_head.y, cr.w, l.grid_head.h);
                let on = self.filter.channel == Some(c as u8);
                cx.text(
                    if on { FontId::Bold } else { FontId::Sans },
                    t.size_small,
                    hr,
                    Align::Center,
                    if on { t.text } else { t.text_faint },
                    &(c + 1).to_string(),
                );
            }
            let hr = Rect::new(clock.x, l.grid_head.y, clock.w, l.grid_head.h);
            cx.text(
                FontId::Sans,
                t.size_small,
                hr.inset_xy(8, 0),
                Align::Left,
                t.text_faint,
                "Clock",
            );
        }
        for &(port, row) in &l.ports {
            let name = mon.port_name(port).unwrap_or("Port");
            let picked = self.filter.port == Some(port);
            cx.text(
                if picked { FontId::Bold } else { FontId::Sans },
                t.size_body,
                Rect::new(row.x + 4, row.y, NAME_W - 48, row.h),
                Align::Left,
                if picked { t.text } else { t.text_dim },
                name,
            );
            for (d, half) in halves(row) {
                let mine = picked && self.filter.shows_dir(d) && self.filter.dirs != 0b11;
                cx.text(
                    if mine { FontId::Bold } else { FontId::Sans },
                    t.size_small,
                    Rect::new(row.x + NAME_W - 44, half.y, 36, half.h),
                    Align::Right,
                    if mine { t.text } else { t.text_faint },
                    way(d),
                );
                let lit_ink = match d {
                    Dir::In => t.good,
                    Dir::Out => t.accent,
                };
                let (lights, clock) = cells(half);
                for (c, cr) in lights.iter().enumerate() {
                    let lit = mon.lit(port, d, c as u8, now);
                    cx.canvas
                        .fill_round_rect(*cr, 4, if lit { lit_ink } else { t.surface });
                    if mine && self.filter.channel == Some(c as u8) {
                        cx.canvas.stroke_round_rect(*cr, 4, 2, t.text);
                    }
                }
                cx.canvas.fill_round_rect(clock, 4, t.surface);
                let dot = Rect::new(clock.x + 8, clock.y + clock.h / 2 - 4, 8, 8);
                let sys = mon.sys_lit(port, d, now);
                cx.canvas
                    .fill_round_rect(dot, 4, if sys { lit_ink } else { t.outline });
                if let Some(bpm) = mon.bpm(port, d) {
                    cx.text(
                        FontId::Sans,
                        t.size_small,
                        Rect::new(clock.x + 22, clock.y, clock.w - 26, clock.h),
                        Align::Left,
                        t.text,
                        &format!("{bpm:.1} BPM"),
                    );
                }
            }
        }
    }

    fn filter_text(&self, m: &Monitor) -> String {
        let f = &self.filter;
        let mut parts = Vec::new();
        if let Some(p) = f.port {
            let d = match f.dirs {
                1 => Some(Dir::In),
                2 => Some(Dir::Out),
                _ => None,
            };
            parts.push(match d {
                Some(d) => port_text(m, p, d),
                None => m.port_name(p).unwrap_or("Port").to_string(),
            });
        } else if f.dirs != 0b11 {
            parts.push(
                match f.dirs {
                    1 => "In only",
                    2 => "Out only",
                    _ => "Neither way",
                }
                .to_string(),
            );
        }
        if let Some(c) = f.channel {
            parts.push(format!("channel {}", c + 1));
        }
        let hidden: Vec<&str> = Kind::ALL
            .iter()
            .zip(KIND_CHIPS)
            .filter(|(k, _)| !f.shows_kind(**k))
            .map(|(_, (name, _))| name)
            .collect();
        if !hidden.is_empty() {
            parts.push(format!("no {}", hidden.join(", ")));
        }
        if parts.is_empty() {
            "Everything".to_string()
        } else {
            parts.join(" · ")
        }
    }

    fn paint_list(&self, cx: &mut PaintCx, m: &IoModel, l: &Layout) {
        let t = cx.theme.clone();
        let mon = &m.monitor;
        let fit = (l.list.h / LINE_H).max(1) as usize;
        self.fit.set(fit);
        let (lines, newer) = self.lines(mon, fit);
        cx.text(
            FontId::Bold,
            t.size_small,
            l.list_head.inset_xy(4, 0),
            Align::Left,
            t.text_dim,
            &format!("Messages: {}", self.filter_text(mon)),
        );
        let right = if self.anchor.is_some() {
            if newer > 0 {
                format!("Holding · {newer} newer")
            } else {
                "Holding".to_string()
            }
        } else if mon.lost > 0 {
            format!("{} too many at once, not listed", mon.lost)
        } else {
            String::new()
        };
        cx.text(
            FontId::Sans,
            t.size_small,
            l.list_head.inset_xy(4, 0),
            Align::Right,
            if self.anchor.is_some() {
                t.warn
            } else {
                t.text_faint
            },
            &right,
        );
        if lines.is_empty() {
            let why = if mon.is_empty() {
                "Nothing yet: play a keyboard plugged into MIDI In or USB"
            } else {
                "Nothing listed matches the chips and lights picked"
            };
            cx.text(
                FontId::Sans,
                t.size_body,
                Rect::new(l.list.x, l.list.y + 24, l.list.w, 40),
                Align::Center,
                t.text_faint,
                why,
            );
            return;
        }
        for (i, (_, s)) in lines.iter().enumerate() {
            let y = l.list.y + i as i32 * LINE_H;
            let row = Rect::new(l.list.x, y, l.list.w, LINE_H - 2);
            if i % 2 == 0 {
                cx.canvas.fill_round_rect(row, 6, t.surface);
            }
            let x = row.x + 8;
            let x = col(
                cx,
                x,
                TIME_W,
                row,
                FontId::Mono,
                t.text_faint,
                &clock_text(s.t),
            );
            let ink = match s.dir {
                Dir::In => t.text,
                Dir::Out => t.accent,
            };
            let port = port_text(mon, s.port, s.dir);
            let x = col(cx, x, PORT_COL_W, row, FontId::Sans, ink, &port);
            let ch = s
                .what
                .channel()
                .map_or(String::new(), |c| format!("Ch {}", c + 1));
            let x = col(cx, x, CH_W, row, FontId::Sans, t.text_dim, &ch);
            let words_w = row.x + row.w - BYTES_W - x;
            let kind_ink = match s.what {
                What::Arriving(_) | What::Stopped(..) => t.text_dim,
                _ => t.text,
            };
            col(cx, x, words_w, row, FontId::Sans, kind_ink, &words(s));
            cx.text(
                FontId::Mono,
                t.size_small,
                Rect::new(row.x + row.w - BYTES_W, row.y, BYTES_W - 8, row.h),
                Align::Right,
                t.text_faint,
                &bytes_text(&s.what),
            );
        }
    }
}

/// Draws `text` in a column `w` wide from `x` in `row`; returns where the
/// next column starts.
fn col(
    cx: &mut PaintCx,
    x: i32,
    w: i32,
    row: Rect,
    font: FontId,
    ink: mpfree_ui::Color,
    text: &str,
) -> i32 {
    let size = cx.theme.size_small;
    cx.text(
        font,
        size,
        Rect::new(x, row.y, w - 8, row.h),
        Align::Left,
        ink,
        text,
    );
    x + w
}

impl IoSection for MonitorSection {
    fn label(&self) -> &str {
        crate::MONITOR
    }

    fn hint(&self) -> Option<&str> {
        Some(HINT)
    }

    fn targets(&self, r: Rect) -> Vec<Target> {
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = self.layout(&m, r);
        let mut out = Vec::new();
        for &(h, cr) in &l.chips {
            let t = match h {
                Hit::Kind(k) => {
                    let i = Kind::ALL.iter().position(|&a| a == k).unwrap_or(0);
                    Target::tap("monitor-kind", i, KIND_CHIPS[i].0, cr)
                }
                Hit::Dir(d) => {
                    let i = DIR_CHIPS.iter().position(|c| c.2 == d).unwrap_or(0);
                    Target::tap("monitor-way", i, DIR_CHIPS[i].0, cr)
                }
                Hit::Hold => Target::tap("monitor-hold", 0, "Hold", cr),
                Hit::Clear => Target::tap("monitor-clear", 0, "Clear", cr),
                _ => continue,
            };
            out.push(t);
        }
        for (k, &(port, row)) in l.ports.iter().enumerate() {
            let name = m.monitor.port_name(port).unwrap_or("?").to_string();
            for (d, half) in halves(row) {
                let way = if d == Dir::In { "in" } else { "out" };
                let j = k * 2 + usize::from(d == Dir::Out);
                out.push(Target::tap(
                    "monitor-port",
                    j,
                    format!("{name} {way}"),
                    Rect::new(half.x, half.y, NAME_W - 8, half.h),
                ));
                let (lights, _) = cells(half);
                for (c, lr) in lights.iter().enumerate() {
                    out.push(Target::tap(
                        "monitor-light",
                        j * 16 + c,
                        format!("{name} {way} ch {}", c + 1),
                        *lr,
                    ));
                }
            }
        }
        out
    }

    fn paint(&self, cx: &mut PaintCx, r: Rect) {
        let t = cx.theme.clone();
        cx.canvas.fill_rect(r, t.bg);
        let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
        let l = self.layout(&m, r);
        if let Some(b) = l.banner {
            cx.canvas.fill_round_rect(b, 10, t.surface);
            cx.canvas
                .fill_round_rect(Rect::new(b.x, b.y + 8, 4, b.h - 16), 2, t.warn);
            cx.text(
                FontId::Sans,
                t.size_small,
                b.inset_xy(16, 0),
                Align::Left,
                t.text_dim,
                "The monitor isn't connected to the ports in this build yet: nothing will be listed.",
            );
        }
        for &(h, cr) in &l.chips {
            match h {
                Hit::Kind(k) => {
                    let i = Kind::ALL.iter().position(|x| *x == k).unwrap_or(0);
                    Self::chip(cx, cr, KIND_CHIPS[i].0, self.filter.shows_kind(k));
                }
                Hit::Dir(d) => {
                    let name = DIR_CHIPS.iter().find(|c| c.2 == d).map_or("", |c| c.0);
                    Self::chip(cx, cr, name, self.filter.shows_dir(d));
                }
                Hit::Hold => {
                    let on = self.anchor.is_some();
                    let (fill, ink) = if on {
                        (t.warn, t.bg)
                    } else {
                        (t.raised, t.text)
                    };
                    cx.canvas.fill_round_rect(cr, 10, fill);
                    cx.text(FontId::Bold, t.size_small, cr, Align::Center, ink, "Hold");
                }
                Hit::Clear => {
                    cx.canvas.fill_round_rect(cr, 10, t.raised);
                    cx.text(
                        FontId::Bold,
                        t.size_small,
                        cr,
                        Align::Center,
                        t.text,
                        "Clear",
                    );
                }
                _ => {}
            }
        }
        let now = self.now.as_secs_f64();
        self.paint_lights(cx, &m, &l, now);
        self.paint_list(cx, &m, &l);
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
        match ev {
            IoInput::Wheel(d) => self.scroll(d),
            IoInput::Page(d) => self.scroll(d * self.fit.get() as i32),
            IoInput::Step(_) => IoReply::Ignored,
        }
    }

    fn tick(&mut self, now: Duration) -> IoReply {
        self.now = now;
        let live = {
            let m = self.model.lock().unwrap_or_else(|e| e.into_inner());
            self.live_now(&m, now.as_secs_f64())
        };
        if live != self.live {
            self.live = live;
            IoReply::Redraw
        } else {
            IoReply::Ignored
        }
    }

    fn shown(&mut self, on: bool) {
        if !on {
            self.down = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::form::testing::{paint, tap, AREA};
    use mpfree_midi::monitor::Feed;
    use mpfree_midi::port::{PortInfo, PortKind};
    use mpfree_midi::DIN;
    use std::sync::{Arc, Mutex};

    const KEYS: PortId = PortId(1);

    fn ports() -> Vec<PortInfo> {
        let mut p = mpfree_midi::port::Ports::new();
        p.set_din(true);
        p.plugged("Keystep 37", true, true);
        p.all().to_vec()
    }

    /// A rig with the monitor fed: a chord on the keyboard, a CC out of
    /// the 5-pin jacks, clock arriving there.
    fn model() -> Shared {
        let mut m = IoModel::default();
        m.live.built.monitor = true;
        m.monitor.set_ports(&ports());
        let mut f = Feed::new();
        f.bytes(KEYS, Dir::In, 1.0, &[0x90, 48, 100, 52, 90, 55, 80]);
        for i in 0..60 {
            f.bytes(DIN, Dir::In, 1.0 + i as f64 * 0.02, &[0xF8]);
            if i == 25 {
                f.bytes(DIN, Dir::Out, 1.5, &[0xB1, 74, 64]);
            }
        }
        f.bytes(KEYS, Dir::In, 2.0, &[0x80, 48, 0]);
        f.drain_into(&mut m.monitor, 2.1);
        Arc::new(Mutex::new(m))
    }

    fn shown(s: &MonitorSection) -> Vec<String> {
        let m = s.model.lock().unwrap();
        s.lines(&m.monitor, 100)
            .0
            .iter()
            .map(|(_, l)| format!("{} | {}", port_text(&m.monitor, l.port, l.dir), words(l)))
            .collect()
    }

    fn layout(s: &MonitorSection) -> Layout {
        let m = s.model.lock().unwrap();
        s.layout(&m, AREA)
    }

    fn chip(s: &MonitorSection, h: Hit) -> (i32, i32) {
        let l = layout(s);
        let r = l.chips.iter().find(|c| c.0 == h).expect("a chip").1;
        (r.x + r.w / 2, r.y + r.h / 2)
    }

    fn light(s: &MonitorSection, port: PortId, d: Dir, c: usize) -> (i32, i32) {
        let l = layout(s);
        let row = l.ports.iter().find(|p| p.0 == port).expect("a port").1;
        let half = halves(row).into_iter().find(|h| h.0 == d).unwrap().1;
        let cr = cells(half).0[c];
        (cr.x + cr.w / 2, cr.y + cr.h / 2)
    }

    #[test]
    fn messages_are_listed_newest_first_in_words() {
        let s = MonitorSection::new(model());
        assert_eq!(
            shown(&s),
            [
                "Keystep 37 in | Note off · C3",
                "MIDI out | CC 74 Cutoff · 64",
                "MIDI in | Clock arriving (counted, not listed)",
                "Keystep 37 in | Note on · G3 · vel 80",
                "Keystep 37 in | Note on · E3 · vel 90",
                "Keystep 37 in | Note on · C3 · vel 100",
            ]
        );
        let m = s.model.lock().unwrap();
        let (top, _) = m.monitor.newest().next().unwrap();
        assert_eq!(top, 5);
        let note_off = m.monitor.newest().next().unwrap().1;
        assert_eq!(bytes_text(&note_off.what), "80 30 00");
        assert_eq!(clock_text(62.5), "1:02.500");
    }

    #[test]
    fn chips_and_lights_pick_what_is_listed() {
        let mut s = MonitorSection::new(model());
        let (x, y) = chip(&s, Hit::Kind(Kind::Notes));
        assert_eq!(tap(&mut s, x, y), IoReply::Redraw);
        assert_eq!(shown(&s).len(), 2, "no notes: {:?}", shown(&s));
        tap(&mut s, x, y);
        let (x, y) = chip(&s, Hit::Dir(Dir::In));
        tap(&mut s, x, y);
        assert_eq!(shown(&s), ["MIDI out | CC 74 Cutoff · 64"]);
        tap(&mut s, x, y);
        // The keyboard's channel 1 light, in: only its notes.
        let (x, y) = light(&s, KEYS, Dir::In, 0);
        tap(&mut s, x, y);
        assert_eq!(s.filter.port, Some(KEYS));
        assert_eq!(s.filter.channel, Some(0));
        assert_eq!(shown(&s).len(), 4);
        assert!(shown(&s)
            .iter()
            .all(|l| l.starts_with("Keystep 37 in | Note")));
        // The same light again: everything.
        tap(&mut s, x, y);
        assert_eq!(s.filter, Filter::ALL);
        // The 5-pin jacks' name, out half: only what went out of them.
        let l = layout(&s);
        let row = l.ports.iter().find(|p| p.0 == DIN).unwrap().1;
        tap(&mut s, row.x + 20, row.y + row.h - 6);
        assert_eq!(shown(&s), ["MIDI out | CC 74 Cutoff · 64"]);
    }

    #[test]
    fn the_list_holds_while_scrolled_back_and_follows_at_the_top() {
        let mut s = MonitorSection::new(model());
        let feed = |s: &MonitorSection, t: f64, bytes: &[u8]| {
            let mut m = s.model.lock().unwrap();
            let mut f = Feed::new();
            f.bytes(KEYS, Dir::In, t, bytes);
            f.drain_into(&mut m.monitor, t);
        };
        for v in 0..30u8 {
            feed(&s, 2.2 + f64::from(v) * 0.01, &[0xB0, 1, v]);
        }
        paint(&s);
        let fit = s.fit.get();
        assert!(fit >= 8, "{fit} lines fit");
        assert_eq!(s.input(IoInput::Wheel(2)), IoReply::Redraw);
        assert_eq!(shown(&s)[0], "Keystep 37 in | CC 1 Mod wheel · 27");
        // More arrive: the list stays where it was.
        feed(&s, 3.0, &[0x90, 60, 1, 62, 1]);
        assert_eq!(shown(&s)[0], "Keystep 37 in | CC 1 Mod wheel · 27");
        let (_, newer) = s.lines(&s.model.lock().unwrap().monitor, 10);
        assert_eq!(newer, 4);
        // Not past the oldest page.
        s.input(IoInput::Page(10));
        assert_eq!(shown(&s).len(), fit);
        // Back to the top: it follows again.
        s.input(IoInput::Wheel(-100));
        assert_eq!(s.anchor, None);
        assert_eq!(shown(&s)[0], "Keystep 37 in | Note on · D4 · vel 1");
        // Hold: new ones wait, Hold again shows them.
        let (x, y) = chip(&s, Hit::Hold);
        tap(&mut s, x, y);
        assert!(s.anchor.is_some());
        feed(&s, 4.0, &[0xC0, 9]);
        assert_eq!(shown(&s)[0], "Keystep 37 in | Note on · D4 · vel 1");
        tap(&mut s, x, y);
        assert_eq!(shown(&s)[0], "Keystep 37 in | Program 10");
        // Clear empties it.
        let (x, y) = chip(&s, Hit::Clear);
        tap(&mut s, x, y);
        assert!(shown(&s).is_empty());
    }

    #[test]
    fn lights_show_each_channel_each_way_and_the_tick_redraws_them() {
        let mut s = MonitorSection::new(model());
        let t = mpfree_ui::Theme::dark();
        // The 5-pin jacks' channel 2 went out at 1.5 s.
        assert_eq!(s.tick(Duration::from_millis(1600)), IoReply::Redraw);
        assert_eq!(s.tick(Duration::from_millis(1600)), IoReply::Ignored);
        let c = paint(&s);
        let (x, y) = light(&s, DIN, Dir::Out, 1);
        assert_eq!(c.pixel(x, y), t.accent, "channel 2 out, lit");
        let (x, y) = light(&s, DIN, Dir::In, 1);
        assert_eq!(c.pixel(x, y), t.surface, "channel 2 in, dark");
        // The keyboard's note-off at 2 s.
        assert_eq!(s.tick(Duration::from_millis(2100)), IoReply::Redraw);
        let c = paint(&s);
        let (x, y) = light(&s, KEYS, Dir::In, 0);
        assert_eq!(c.pixel(x, y), t.good, "channel 1 in, lit");
        let (x, y) = light(&s, KEYS, Dir::In, 1);
        assert_eq!(c.pixel(x, y), t.surface, "channel 2, dark");
        let (x, y) = light(&s, DIN, Dir::Out, 1);
        assert_eq!(c.pixel(x, y), t.surface, "gone dark");
        // Its clock's tempo: 125 BPM.
        let m = s.model.lock().unwrap();
        let bpm = m.monitor.bpm(DIN, Dir::In).unwrap();
        assert!((bpm - 125.0).abs() < 1.0, "{bpm}");
    }

    #[test]
    fn an_unfed_monitor_says_so_and_unplugged_ports_go() {
        let s = MonitorSection::new(Arc::new(Mutex::new(IoModel::default())));
        let l = layout(&s);
        assert!(l.banner.is_some());
        assert_eq!(l.ports.len(), 1, "the 5-pin jacks");
        let s = MonitorSection::new(model());
        assert!(layout(&s).banner.is_none());
        assert_eq!(layout(&s).ports.len(), 2);
        let mut p = ports();
        p[1].connected = false;
        assert_eq!(p[1].kind, PortKind::UsbHost);
        s.model.lock().unwrap().monitor.set_ports(&p);
        assert_eq!(layout(&s).ports.len(), 1);
        // Its messages stay listed by name.
        assert_eq!(shown(&s)[0], "Keystep 37 in | Note off · C3");
    }

    #[test]
    fn words_cover_every_kind() {
        let at = |what| Seen {
            t: 0.0,
            port: DIN,
            dir: Dir::In,
            what,
        };
        let cases = [
            (
                What::SongPosition(16 * 9 + 4 * 2 + 1),
                "Song position · bar 10.3.2",
            ),
            (What::SongSelect(0), "Song select · 1"),
            (
                What::SysEx {
                    len: 12,
                    maker: [0x47, 0, 0],
                },
                "SysEx · 12 bytes · Akai",
            ),
            (
                What::SysEx {
                    len: 9,
                    maker: [0, 0x21, 0x09],
                },
                "SysEx · 9 bytes · maker 00 21 09",
            ),
            (What::Stopped(Steady::Clock, 96), "Clock stopped · 96 ticks"),
            (What::Undefined(0xFD), "Undefined FD"),
            (What::Reset, "System reset"),
        ];
        for (w, text) in cases {
            assert_eq!(words(&at(w)), text);
        }
        let bend = What::Channel {
            msg: Msg::PitchBend { ch: 0, value: -512 },
            bytes: [0xE0, 0, 0x3C],
            len: 3,
        };
        assert_eq!(words(&at(bend)), "Pitch bend · -512");
        let out = Seen {
            dir: Dir::Out,
            ..at(What::Arriving(Steady::Clock))
        };
        assert_eq!(words(&out), "Clock going out (counted, not listed)");
        assert_eq!(
            bytes_text(&What::SysEx {
                len: 12,
                maker: [0x47, 0, 0]
            }),
            "F0 47 … F7"
        );
    }
}
